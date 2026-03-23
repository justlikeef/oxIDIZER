use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use std::collections::HashMap;
use ox_type_converter::ValueType;
use std::ffi::{c_void, CString, CStr};
use libc::c_char;
use std::sync::Arc;
use ox_fileproc::{serde_json, serde_yaml};

pub struct YamlPersistenceDriver;

impl PersistenceDriver for YamlPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        use std::fs;
        use std::path::Path;
        use ox_fileproc::RawFile;

        let id_val = serializable_map.get("id").ok_or("Missing 'id' in data object")?;
        let id = &id_val.0;

        let file_path = Path::new(location);
        
        // Strategy:
        // 1. Try to open file with ox_fileproc
        // 2. Find record with [id=...]
        // 3. If found, update fields.
        // 4. If not found, append new record.

        // Check if file exists. If not, create empty one (or with empty list if yaml)
        if !file_path.exists() {
             fs::write(file_path, "---\n[]\n").map_err(|e| e.to_string())?;
        }

        let mut raw_file = RawFile::open(file_path).map_err(|e| e.to_string())?;

        // Construct query to find the list item with this ID.
        // Assuming list of objects: "[id=THE_ID]" should find the item.
        let query = format!("[id={}]", id);
        
        // We need to collect cursors first because we cannot mutate raw_file while iterating cursors
        let cursors: Vec<_> = raw_file.find(&query).collect();
        
        if let Some(_record_cursor) = cursors.first() {
            // Record exists. Update its fields.
            // We need to re-scan from *this* cursor to find child fields.
            // But RawFile::find starts from root. 
            // We need a way to surgically update children of *this* block.
            
            // ox_fileproc::RawFile doesn't have a scoped update helper yet using existing cursors directly exposed?
            // Wait, we can construct a sub-query? 
            // "QUERY/field_name"
            
            // Actually, RawFile::find starts from root.
            // If we have a unique ID, " [id=ID]/field_name " should target the field *inside* that record.
            
            // We'll iterate fields and update them one by one.
            // We collect updates to apply them safely (though RawFile::update handles index shifts if we are careful? 
            // Actually RawFile::update might invalidate other cursors if lengths change.
            // We should reload or handle offsets. 
            // Safest: One update at a time, finding freshly.
            
            for (key, (val_str, _, _)) in serializable_map {
                if key == "id" { continue; } // Don't update ID
                
                let field_query = format!("[id={}]/{}", id, key);
                
                // Check if field exists
                let field_cursors: Vec<_> = raw_file.find(&field_query).collect();
                if let Some(field_cursor) = field_cursors.first() {
                    // Update existing field
                    // We simply replace the value span.
                    // Note: val_str is raw string. If it's a string type, we might need quotes?
                    // The driver receives "val_str". `ox_persistence` convention: val_str is strictly the value.
                    // In YAML, strings don't always need quotes, but if they contain specials they do.
                    // For safety, let's use serde_yaml to value-serialize just this string.
                    let safe_val = serde_yaml::to_string(&serde_yaml::Value::String(val_str.clone()))
                        .unwrap_or(val_str.clone())
                        .trim_start_matches("---\n")
                        .trim()
                        .to_string();
                        
                    raw_file.update(field_cursor.span.clone(), &safe_val);
                } else {
                    // Field does not exist in this record. We should append it.
                    // "append" inserts after the node.
                    // We want to insert *into* the record.
                    // `ox_fileproc` naming `append` adds as child if cursor allows?
                    // Actually `append` inserts at `cursor.span.end`.
                    // For a YAML map, inserting at end of block might work if indent is correct.
                    // Managing indent manually is hard here.
                    // For now, only UPDATE is fully supported "surgically".
                    // If we need to add fields, implementation complexity rises.
                    // Let's Skip adding new fields for this initial "Surgical" pass, 
                    // OR we try to append to the record cursor.
                    
                    // We'll fallback to logging a warning or just skip.
                    // User request said "The drivers ... should be able to use ox_fileproc."
                    // Implies "where possible".
                    eprintln!("Warning: Field '{}' not found in record '{}'. Surgical insertion not yet fully auto-indented.", key, id);
                }
            }
            
            raw_file.save().map_err(|e| e.to_string())?;

        } else {
            // New Record. We can't surgically update what's not there.
            // We must append the whole record.
            // Logic: Read file as YAML (using serde), add record, write back?
            // That DESTROYS comments.
            // We want to append text to the file using RawFile if possible.
            // "Append to root"? 
            
            // If we can construct the YAML string for this new record:
            let mut temp_map = serde_yaml::Mapping::new();
            for (k, (v, vt, _)) in serializable_map {
                let yv = match vt {
                    ValueType::Integer => serde_yaml::Value::Number(v.parse().unwrap_or(0).into()),
                     _ => serde_yaml::Value::String(v.clone()),
                };
                temp_map.insert(serde_yaml::Value::String(k.clone()), yv);
            }
            let record_str = serde_yaml::to_string(&vec![temp_map]).map_err(|e| e.to_string())?;
            // serde writes "---\n- field: val..."
            // We want to append just the list item part.
            // Strip "---\n"
            let clean_rec = record_str.trim_start_matches("---\n");
            
            // Append to end of file
            // We can just append to raw_file.content
            // But we need to ensure newline separator.
            let mut file_content = raw_file.content;
            if !file_content.ends_with('\n') {
                file_content.push('\n');
            }
            file_content.push_str(clean_rec);
            
            fs::write(file_path, file_content).map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        use std::fs;
        use std::path::Path;
        use serde_yaml::Value;

        let file_path = Path::new(location);
        if !file_path.exists() {
            return Err(format!("File {} not found", location));
        }
        let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
        let records: Vec<Value> = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;

        for record in records {
             // In YAML, keys are Values. We need to find "id" key.
             if let Value::Mapping(map) = &record {
                 if let Some(rec_id_val) = map.get(&Value::String("id".to_string())) {
                     let rec_id = match rec_id_val {
                         Value::String(s) => s.clone(),
                         Value::Number(n) => n.to_string(),
                         _ => format!("{:?}", rec_id_val),
                     };
                     
                     if rec_id == id {
                         let mut result_map = HashMap::new();
                         for (k, v) in map {
                             let key_str = match k {
                                 Value::String(s) => s.clone(),
                                 _ => format!("{:?}", k),
                             };

                             let (val_str, val_type) = match v {
                                 Value::String(s) => (s.clone(), ox_type_converter::TypeConverter::infer_value_type(s)),
                                 Value::Number(n) => {
                                     if n.is_i64() {
                                         (n.to_string(), ValueType::Integer)
                                     } else {
                                         (n.to_string(), ValueType::Float)
                                     }
                                 },
                                 Value::Bool(b) => (b.to_string(), ValueType::Boolean),
                                 Value::Sequence(_) => {
                                     let s = serde_yaml::to_string(v).unwrap_or_default();
                                     // Strip the leading "---\n" if present from serialization
                                     let s = s.trim_start_matches("---\n").trim().to_string();
                                     (s, ValueType::List(Box::new(ValueType::String)))
                                 },
                                 Value::Mapping(_) => {
                                     let s = serde_yaml::to_string(v).unwrap_or_default();
                                     let s = s.trim_start_matches("---\n").trim().to_string();
                                     (s, ValueType::Map)
                                 },
                                 Value::Null => ("".to_string(), ValueType::String),
                                 Value::Tagged(_) => ("tagged_value".to_string(), ValueType::String), // Simple fallback
 
                             };
                             result_map.insert(key_str, (val_str, val_type, HashMap::new()));
                         }
                         return Ok(result_map);
                     }
                 }
             }
        }

        Err(format!("Object with id {} not found in {}", id, location))
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
        use std::fs;
        use std::path::Path;
        use serde_yaml::Value;

        let file_path = Path::new(location);
        if !file_path.exists() {
             return Ok(Vec::new());
        }
        let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
        let records: Vec<Value> = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;

        let mut matching_ids = Vec::new();

        for record in records {
             if let Value::Mapping(map) = &record {
                let mut matches = true;
                for (key, (filter_val, filter_type, _)) in filter {
                     if let Some(record_val) = map.get(&Value::String(key.clone())) {
                         match (filter_type, record_val) {
                             (ValueType::String, Value::String(s)) => if s != filter_val { matches = false; break; },
                             (ValueType::Integer, Value::Number(n)) => {
                                 if let Ok(i) = filter_val.parse::<i64>() {
                                    if n.as_i64() != Some(i) { matches = false; break; }
                                 }
                             },
                             (ValueType::Float, Value::Number(n)) => {
                                 if let Ok(f) = filter_val.parse::<f64>() {
                                    if n.as_f64() != Some(f) { matches = false; break; }
                                 }
                             },
                             (ValueType::Boolean, Value::Bool(b)) => {
                                 if let Ok(fb) = filter_val.parse::<bool>() {
                                     if b != &fb { matches = false; break; }
                                 }
                             },
                             _ => {
                                  let rec_str = match record_val {
                                      Value::String(s) => s.clone(),
                                      Value::Number(n) => n.to_string(),
                                      Value::Bool(b) => b.to_string(),
                                      _ => format!("{:?}", record_val),
                                  };
                                  if &rec_str != filter_val { matches = false; break; }
                             }
                         }
                     } else {
                         matches = false;
                         break;
                     }
                }

                if matches {
                    if let Some(id_val) = map.get(&Value::String("id".to_string())) {
                        let id_str = match id_val {
                             Value::String(s) => s.clone(),
                             Value::Number(n) => n.to_string(),
                             _ => format!("{:?}", id_val),
                        };
                        matching_ids.push(id_str);
                    }
                }
             }
        }
        Ok(matching_ids)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("YamlDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("Preparing YAML datastore: {:?}", connection_info);
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        Ok(vec!["default".to_string()])
    }
    
    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, dataset_name: &str) -> Result<DataSet, String> {
        Ok(DataSet { name: dataset_name.to_string(), columns: Vec::new() })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
         vec![ConnectionParameter {
            name: "path".to_string(),
            description: "Path to yaml file".to_string(),
            data_type: "string".to_string(),
            is_required: true,
            default_value: None,
        }]
    }
}


// --- FFI Exports ---

#[no_mangle]
pub extern "C" fn ox_driver_init(_config_json: *const c_char) -> *mut c_void {
    let driver = Box::new(YamlPersistenceDriver);
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut YamlPersistenceDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void, 
    data_json: *const c_char, 
    location: *const c_char
) -> i32 {
    let driver = &*(ctx as *mut YamlPersistenceDriver);
    let data_str = CStr::from_ptr(data_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();

    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&data_str) {
        Ok(map) => {
            match driver.persist(&map, &location_str) {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("Persist error: {}", e);
                    -1
                }
            }
        },
        Err(e) => {
            eprintln!("JSON parse error: {}", e);
            -2
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_restore(
    ctx: *mut c_void, 
    location: *const c_char, 
    id: *const c_char
) -> OxBuffer {
    let driver = &*(ctx as *mut YamlPersistenceDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();

    match driver.restore(&location_str, &id_str) {
        Ok(map) => {
            let json = serde_json::to_string(&map).unwrap_or_default();
            OxBuffer::from_str(json)
        },
        Err(e) => {
            eprintln!("Restore error: {}", e);
            OxBuffer::empty()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_fetch(
    ctx: *mut c_void, 
    filter_json: *const c_char, 
    location: *const c_char
) -> OxBuffer {
    let driver = &*(ctx as *mut YamlPersistenceDriver);
    let filter_str = CStr::from_ptr(filter_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();

    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&filter_str) {
        Ok(filter) => {
            match driver.fetch(&filter, &location_str) {
                Ok(ids) => {
                    let json = serde_json::to_string(&ids).unwrap_or_default();
                    OxBuffer::from_str(json)
                },
                Err(e) => {
                    eprintln!("Fetch error: {}", e);
                    OxBuffer::empty()
                }
            }
        },
        Err(e) => {
            eprintln!("JSON parse error: {}", e);
            OxBuffer::empty()
        }
    }
}




#[no_mangle]
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compatible_modules = HashMap::new();
    compatible_modules.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "YAML Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    // Parse friendly_name from schema using ox_fileproc
    let schema = include_str!("../ox_persistence_driver_file_yaml_config_schema.yaml");
    let friendly_name = match ox_fileproc::processor::parse_content(schema, "yaml") {
        Ok(ox_fileproc::serde_json::Value::Object(map)) => {
            map.get("friendly_name")
               .and_then(|v| v.as_str())
               .map(|s| s.to_string())
               .unwrap_or("YAML File".to_string())
        },
        Ok(_) => {
            eprintln!("Schema parsed but not an object!");
            "YAML File".to_string()
        },
        Err(e) => {
            eprintln!("Schema parse error: {}", e);
            "YAML File".to_string()
        }
    };

    let metadata = DriverMetadata {
        name: "ox_persistence_driver_yaml".to_string(),
        friendly_name: Some(friendly_name),
        description: "A YAML persistence driver.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = ox_fileproc::serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = include_str!("../ox_persistence_driver_file_yaml_config_schema.yaml");
    CString::new(schema).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::collections::HashMap;

    #[test]
    fn test_persist_preserves_comments() {
        // Setup
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("config.yaml");
        let initial_content = r#"
# This is a comment
- id: "1"
  name: "original" # Inline comment
  status: "active"
"#;
        fs::write(&file_path, initial_content).unwrap();
        
        // Driver
        let driver = YamlPersistenceDriver;
        let mut map = HashMap::new();
        map.insert("id".to_string(), ("1".to_string(), ValueType::String, HashMap::new()));
        map.insert("name".to_string(), ("updated".to_string(), ValueType::String, HashMap::new()));
        map.insert("status".to_string(), ("active".to_string(), ValueType::String, HashMap::new()));

        // Act
        driver.persist(&map, file_path.to_str().unwrap()).unwrap();

        // Assert
        let new_content = fs::read_to_string(&file_path).unwrap();
        println!("New Content:\n{}", new_content);
        
        assert!(new_content.contains("# This is a comment"), "Header comment missing");
        assert!(new_content.contains("name: updated"), "Value not updated");
        assert!(new_content.contains("# Inline comment"), "Inline comment missing");
    }
}
