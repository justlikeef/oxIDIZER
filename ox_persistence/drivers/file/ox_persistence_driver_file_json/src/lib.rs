use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use std::collections::HashMap;
use ox_type_converter::ValueType;
use std::ffi::{c_void, CString, CStr};
use libc::c_char;
use std::sync::Arc;

pub struct JsonPersistenceDriver;

impl JsonPersistenceDriver {
    pub fn register() {
         let mut compatible_modules = HashMap::new();
        compatible_modules.insert(
            "ox_data_broker_server".to_string(),
            ModuleCompatibility {
                human_name: "JSON Persistence Driver".to_string(),
                crate_type: "Data Source Driver".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        );

        let metadata = DriverMetadata {
            name: "ox_persistence_driver_json".to_string(),
            description: "A JSON persistence driver.".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            compatible_modules,
        };

        ox_persistence::register_persistence_driver(Arc::new(JsonPersistenceDriver), metadata);
    }
}

impl PersistenceDriver for JsonPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        use std::fs;
        use std::path::Path;
        use serde_json::{Value, Map, Number};

        let file_path = Path::new(location);
        let mut records: Vec<Value> = if file_path.exists() {
            let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
            if content.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&content).map_err(|e| e.to_string())?
            }
        } else {
            Vec::new()
        };

        let mut new_record = Map::new();
        for (key, (val_str, val_type, _)) in serializable_map {
            let json_val = match val_type {
                ValueType::Integer => {
                    let num = val_str.parse::<i64>().map_err(|e| e.to_string())?;
                    Value::Number(Number::from(num))
                },
                ValueType::Float => {
                    let num = val_str.parse::<f64>().map_err(|e| e.to_string())?;
                    Number::from_f64(num).map(Value::Number).unwrap_or(Value::Null)
                },
                ValueType::Boolean => {
                    let b = val_str.parse::<bool>().map_err(|e| e.to_string())?;
                    Value::Bool(b)
                },
                ValueType::List(_) | ValueType::Map => {
                     serde_json::from_str(val_str).unwrap_or(Value::String(val_str.clone()))
                },
                _ => Value::String(val_str.clone()),
            };
            new_record.insert(key.clone(), json_val);
        }

        records.push(Value::Object(new_record));

        let new_content = serde_json::to_string_pretty(&records).map_err(|e| e.to_string())?;
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
        use serde_json::Value;

        let file_path = Path::new(location);
        if !file_path.exists() {
            return Err(format!("File {} not found", location));
        }
        let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
        let records: Vec<Value> = serde_json::from_str(&content).map_err(|e| e.to_string())?;

        for record in records {
             if let Some(rec_id_val) = record.get("id") {
                 let rec_id = match rec_id_val {
                     Value::String(s) => s.clone(),
                     Value::Number(n) => n.to_string(),
                     _ => rec_id_val.to_string(),
                 };
                 
                 if rec_id == id {
                     // Convert Object back to Datastore Map
                     if let Value::Object(map) = record {
                         let mut result_map = HashMap::new();
                         for (k, v) in map {
                             let (val_str, val_type) = match v {
                                 Value::String(s) => {
                     // Try to infer specific type (e.g. DateTime) from string
                     let inferred = ox_type_converter::TypeConverter::infer_value_type(&s);
                     (s.clone(), inferred)
                },                 Value::Number(n) => {
                                     if n.is_i64() {
                                         (n.to_string(), ValueType::Integer)
                                     } else {
                                         (n.to_string(), ValueType::Float)
                                     }
                                 },
                                 Value::Bool(b) => (b.to_string(), ValueType::Boolean),
                                 Value::Array(_) => (v.to_string(), ValueType::List(Box::new(ValueType::String))), // Basic inference
                                 Value::Object(_) => (v.to_string(), ValueType::Map),
                                 Value::Null => ("".to_string(), ValueType::String),
                             };
                             result_map.insert(k, (val_str, val_type, HashMap::new()));
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
        use serde_json::Value;

        let file_path = Path::new(location);
        if !file_path.exists() {
             return Ok(Vec::new());
        }
        let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
        let records: Vec<Value> = serde_json::from_str(&content).map_err(|e| e.to_string())?;

        let mut matching_ids = Vec::new();

        for record in records {
            let mut matches = true;
             for (key, (filter_val, filter_type, _)) in filter {
                 if let Some(record_val) = record.get(key) {
                     // Compare types
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
                         // Fallback string comparison
                         _ => {
                             let record_str = match record_val {
                                 Value::String(s) => s.clone(),
                                 Value::Number(n) => n.to_string(),
                                 Value::Bool(b) => b.to_string(),
                                 _ => record_val.to_string(),
                             };
                             if &record_str != filter_val { matches = false; break; }
                         }
                     }
                 } else {
                     matches = false;
                     break;
                 }
            }

            if matches {
                if let Some(id_val) = record.get("id") {
                    let id_str = match id_val {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        _ => id_val.to_string(),
                    };
                    matching_ids.push(id_str);
                }
            }
        }
        Ok(matching_ids)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("JsonDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("Preparing JSON datastore: {:?}", connection_info);
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
            description: "Path to json file".to_string(),
            data_type: "string".to_string(),
            is_required: true,
            default_value: None,
        }]
    }
}

// Stub FFI functions need to be updated to call trait methods


// --- FFI Exports ---

#[no_mangle]
pub extern "C" fn ox_driver_init(_config_json: *const c_char) -> *mut c_void {
    let driver = Box::new(JsonPersistenceDriver);
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut JsonPersistenceDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void, 
    data_json: *const c_char, 
    location: *const c_char
) -> i32 {
    let driver = &*(ctx as *mut JsonPersistenceDriver);
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
    let driver = &*(ctx as *mut JsonPersistenceDriver);
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
    let driver = &*(ctx as *mut JsonPersistenceDriver);
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
            human_name: "JSON Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_driver_json".to_string(),
        description: "A JSON persistence driver.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}



#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}

mod tests;