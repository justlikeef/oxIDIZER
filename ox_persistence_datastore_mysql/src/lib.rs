use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use std::collections::HashMap;
use ox_type_converter::ValueType;
use std::ffi::{c_void, CString, CStr};
use libc::c_char;
use std::sync::Arc;
use chrono::{Datelike, Timelike};


pub struct MysqlPersistenceDriver {
    pool: Option<mysql::Pool>,
}

impl PersistenceDriver for MysqlPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        let pool = self.pool.as_ref().ok_or("MySQL pool not initialized")?;
        let mut conn = pool.get_conn().map_err(|e| e.to_string())?;

        // Construct INSERT statement dynamically
        let keys: Vec<&String> = serializable_map.keys().collect();
        let cols = keys.iter().map(|k| format!("`{}`", k)).collect::<Vec<_>>().join(", ");
        let vals = keys.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let query = format!("INSERT INTO `{}` ({}) VALUES ({})", location, cols, vals);

        let params: Vec<mysql::Value> = keys.iter().map(|k| {
            let (v, t, _) = &serializable_map[*k];
            match t {
                ValueType::Integer => {
                    if let Ok(i) = v.parse::<i64>() {
                        mysql::Value::Int(i)
                    } else {
                        mysql::Value::from(v)
                    }
                },
                ValueType::Float => {
                    if let Ok(f) = v.parse::<f64>() {
                        mysql::Value::Double(f)
                    } else {
                        mysql::Value::from(v)
                    }
                },
                ValueType::Boolean => {
                    if let Ok(b) = v.parse::<bool>() {
                        mysql::Value::Int(if b { 1 } else { 0 })
                    } else {
                        mysql::Value::from(v)
                    }
                },
                ValueType::DateTime => {
                     // Parse ISO8601 string to NaiveDateTime or just pass string if MySQL handles it?
                     // MySQL driver handles NaiveDateTime.
                     // We need chrono::NaiveDateTime.
                     if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(v) {
                         mysql::Value::Date(
                             dt.year() as u16,
                             dt.month() as u8,
                             dt.day() as u8,
                             dt.hour() as u8,
                             dt.minute() as u8,
                             dt.second() as u8,
                             dt.timestamp_subsec_micros()
                         )
                     } else {
                         mysql::Value::from(v)
                     }
                },
                _ => mysql::Value::from(v),
            }
        }).collect();

        use mysql::prelude::Queryable;
        conn.exec_drop(query, params).map_err(|e| e.to_string())?;
        
        Ok(())
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let pool = self.pool.as_ref().ok_or("MySQL pool not initialized")?;
        let mut conn = pool.get_conn().map_err(|e| e.to_string())?;

        let query = format!("SELECT * FROM `{}` WHERE id = :id", location);
        
        use mysql::prelude::Queryable;
        use mysql::params;

        let res = conn.exec_iter(query, params!{ "id" => id }).map_err(|e| e.to_string())?;
        
        // We need to map columns back to HashMap.
        
        for row in res {
            let row = row.map_err(|e| e.to_string())?;
            let mut map = HashMap::new();
            
            for (i, col) in row.columns_ref().iter().enumerate() {
                // Try to get typed value or fallback to string
                let val_str: String = row.get(i).unwrap_or_default();
                let col_name = col.name_str().to_string();
                
                // Inference could be improved by checking ColumnType
                map.insert(col_name, (val_str, ValueType::String, HashMap::new()));
            }
            return Ok(map);
        }

        Err(format!("Object with id {} not found", id))
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
        let pool = self.pool.as_ref().ok_or("MySQL pool not initialized")?;
        let mut conn = pool.get_conn().map_err(|e| e.to_string())?;

        let mut query = format!("SELECT id FROM `{}` WHERE 1=1", location);
        let mut params_vec: Vec<mysql::Value> = Vec::new();

        for (k, (v, t, _)) in filter {
            query.push_str(&format!(" AND `{}` = ?", k));
             match t {
                ValueType::Integer => {
                    if let Ok(i) = v.parse::<i64>() {
                        params_vec.push(mysql::Value::Int(i));
                    } else {
                         params_vec.push(mysql::Value::from(v));
                    }
                },
                ValueType::Float => {
                    if let Ok(f) = v.parse::<f64>() {
                         params_vec.push(mysql::Value::Double(f));
                    } else {
                         params_vec.push(mysql::Value::from(v));
                    }
                },
                ValueType::Boolean => {
                    if let Ok(b) = v.parse::<bool>() {
                         params_vec.push(mysql::Value::Int(if b { 1 } else { 0 }));
                    } else {
                         params_vec.push(mysql::Value::from(v));
                    }
                },
                ValueType::DateTime => {
                     if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(v) {
                         params_vec.push(mysql::Value::Date(
                             dt.year() as u16,
                             dt.month() as u8,
                             dt.day() as u8,
                             dt.hour() as u8,
                             dt.minute() as u8,
                             dt.second() as u8,
                             dt.timestamp_subsec_micros()
                         ));
                    } else {
                         params_vec.push(mysql::Value::from(v));
                    }
                },
                _ => params_vec.push(mysql::Value::from(v)),
            }
        }

        use mysql::prelude::Queryable;
        let ids: Vec<String> = conn.exec(query, params_vec).map_err(|e| e.to_string())?;
        
        Ok(ids)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
         println!("MysqlDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("Preparing MySQL datastore: {:?}", connection_info);
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
            name: "dsn".to_string(),
            description: "MySQL Connection String".to_string(),
            data_type: "string".to_string(),
            is_required: true,
            default_value: None,
        }]
    }
}
 
// --- FFI Exports ---

#[no_mangle]
pub extern "C" fn ox_driver_init(config_json: *const c_char) -> *mut c_void {
    let config_str = unsafe { CStr::from_ptr(config_json).to_string_lossy() };
    let config: HashMap<String, String> = serde_json::from_str(&config_str).unwrap_or_default();
    
    let pool = if let Some(dsn) = config.get("dsn") {
        mysql::Pool::new(dsn.as_str()).ok()
    } else {
        None
    };

    let driver = Box::new(MysqlPersistenceDriver { pool });
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut MysqlPersistenceDriver);
    }
}


#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void, 
    data_json: *const c_char, 
    location: *const c_char
) -> i32 {
    let driver = &*(ctx as *mut MysqlPersistenceDriver);
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
    let driver = &*(ctx as *mut MysqlPersistenceDriver);
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
    let driver = &*(ctx as *mut MysqlPersistenceDriver);
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
pub extern "C" fn ox_driver_get_metadata() -> *mut c_char {
    let mut compatible_modules = HashMap::new();
    compatible_modules.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "MySQL Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_datastore_mysql".to_string(),
        description: "A MySQL persistence driver.".to_string(),
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