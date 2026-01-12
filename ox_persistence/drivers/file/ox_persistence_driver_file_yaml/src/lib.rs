use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use std::collections::HashMap;
use ox_type_converter::ValueType;
use std::ffi::{c_void, CString, CStr};
use libc::c_char;
use std::sync::Arc;

pub struct YamlPersistenceDriver;

impl PersistenceDriver for YamlPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        use std::fs;
        use std::path::Path;
        use serde_yaml::{Value, Mapping, Number};

        let file_path = Path::new(location);
        let mut records: Vec<Value> = if file_path.exists() {
            let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
            if content.trim().is_empty() {
                Vec::new()
            } else {
                serde_yaml::from_str(&content).map_err(|e| e.to_string())?
            }
        } else {
            Vec::new()
        };

        let mut new_record = Mapping::new();
        for (key, (val_str, val_type, _)) in serializable_map {
            let yaml_val = match val_type {
                ValueType::Integer => {
                    let num = val_str.parse::<i64>().map_err(|e| e.to_string())?;
                    Value::Number(Number::from(num))
                },
                ValueType::Float => {
                    let num = val_str.parse::<f64>().map_err(|e| e.to_string())?;
                    Value::Number(Number::from(num))
                },
                ValueType::Boolean => {
                    let b = val_str.parse::<bool>().map_err(|e| e.to_string())?;
                    Value::Bool(b)
                },
                ValueType::List(_) | ValueType::Map => {
                     serde_yaml::from_str(val_str).unwrap_or(Value::String(val_str.clone()))
                },
                _ => Value::String(val_str.clone()),
            };
            new_record.insert(Value::String(key.clone()), yaml_val);
        }

        records.push(Value::Mapping(new_record));

        let new_content = serde_yaml::to_string(&records).map_err(|e| e.to_string())?;
        fs::write(file_path, new_content).map_err(|e| e.to_string())?;

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

    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
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

    let metadata = DriverMetadata {
        name: "ox_persistence_driver_yaml".to_string(),
        description: "A YAML persistence driver.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
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

