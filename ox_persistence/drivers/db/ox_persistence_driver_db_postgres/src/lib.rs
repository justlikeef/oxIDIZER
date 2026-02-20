use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use std::collections::HashMap;
use ox_type_converter::ValueType;
use std::ffi::{c_void, CString, CStr};
use libc::c_char;
use std::sync::Arc;
use chrono;
use ox_fileproc::serde_json;


use std::sync::Mutex;

pub struct PostgresPersistenceDriver {
    client: Mutex<Option<postgres::Client>>,
}

impl PersistenceDriver for PostgresPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        let mut guard = self.client.lock().map_err(|e| e.to_string())?;
        let client = guard.as_mut().ok_or("Postgres client not initialized")?;

        use ox_persistence_driver_sql::{SqlBuilder, SqlDialect};
        let builder = SqlBuilder::new(SqlDialect::Postgres);
        let keys: Vec<String> = serializable_map.keys().cloned().collect();
        let query = builder.build_insert(location, &keys);

        let mut params_owned: Vec<Box<dyn postgres::types::ToSql + Sync + Send>> = Vec::new();

        for k in &keys {
            let (v, t, _) = &serializable_map[k];
            match t {
                ValueType::Integer => {
                    if let Ok(i) = v.parse::<i64>() {
                        params_owned.push(Box::new(i));
                    } else {
                        params_owned.push(Box::new(v.clone()));
                    }
                },
                ValueType::Float => {
                    if let Ok(f) = v.parse::<f64>() {
                        params_owned.push(Box::new(f));
                    } else {
                        params_owned.push(Box::new(v.clone()));
                    }
                },
                ValueType::Boolean => {
                    if let Ok(b) = v.parse::<bool>() {
                        params_owned.push(Box::new(b));
                    } else {
                        params_owned.push(Box::new(v.clone()));
                    }
                },
                ValueType::DateTime => {
                     if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(v) {
                         params_owned.push(Box::new(dt));
                    } else {
                         params_owned.push(Box::new(v.clone()));
                    }
                },
                _ => params_owned.push(Box::new(v.clone())),
            }
        }
        
        let params_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = params_owned.iter()
            .map(|s| s.as_ref() as &(dyn postgres::types::ToSql + Sync))
            .collect();

        client.execute(&query, &params_refs).map_err(|e| e.to_string())?;
        
        Ok(())
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let mut guard = self.client.lock().map_err(|e| e.to_string())?;
        let client = guard.as_mut().ok_or("Postgres client not initialized")?;

        use ox_persistence_driver_sql::{SqlBuilder, SqlDialect};
        let builder = SqlBuilder::new(SqlDialect::Postgres);
        let query = builder.build_select_by_id(location);
        
        let rows = client.query(&query, &[&id]).map_err(|e| e.to_string())?;
        
        if let Some(row) = rows.get(0) {
            let mut map = HashMap::new();
            for col in row.columns() {
                 // Try to guess type or convert to string
                 // Postgres row.get panics if type mismatches unless we use explicit type or try_get (which needs Type)
                 // Or we can get as String if FromSql<String> is implemented for the column type.
                 // Ideally we inspect col.type_()
                 
                 let val_str: String = if let Ok(s) = row.try_get::<_, String>(col.name()) {
                     s
                 } else if let Ok(i) = row.try_get::<_, i64>(col.name()) {
                     i.to_string()
                 } else if let Ok(f) = row.try_get::<_, f64>(col.name()) {
                     f.to_string()
                 } else if let Ok(b) = row.try_get::<_, bool>(col.name()) {
                     b.to_string()
                 } else {
                     "".to_string()
                 };
                 
                 map.insert(col.name().to_string(), (val_str, ValueType::String, HashMap::new()));
            }
            Ok(map)
        } else {
            Err(format!("Object with id {} not found", id))
        }
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
        let mut guard = self.client.lock().map_err(|e| e.to_string())?;
        let client = guard.as_mut().ok_or("Postgres client not initialized")?;

        use ox_persistence_driver_sql::{SqlBuilder, SqlDialect};
        let builder = SqlBuilder::new(SqlDialect::Postgres);
        let keys: Vec<String> = filter.keys().cloned().collect();
        let query = builder.build_fetch(location, &keys);
        
        let mut params_vec: Vec<Box<dyn postgres::types::ToSql + Sync + Send>> = Vec::new();

        for k in &keys {
            let (v, t, _) = &filter[k];
            match t {
                ValueType::Integer => {
                    if let Ok(i) = v.parse::<i64>() {
                        params_vec.push(Box::new(i));
                    } else {
                        params_vec.push(Box::new(v.clone()));
                    }
                },
                ValueType::Float => {
                    if let Ok(f) = v.parse::<f64>() {
                        params_vec.push(Box::new(f));
                    } else {
                        params_vec.push(Box::new(v.clone()));
                    }
                },
                ValueType::Boolean => {
                    if let Ok(b) = v.parse::<bool>() {
                        params_vec.push(Box::new(b));
                    } else {
                        params_vec.push(Box::new(v.clone()));
                    }
                },
                ValueType::DateTime => {
                     if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(v) {
                         params_vec.push(Box::new(dt));
                    } else {
                         params_vec.push(Box::new(v.clone()));
                    }
                },
                _ => params_vec.push(Box::new(v.clone())),
            }

        }

        let params_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = params_vec.iter()
            .map(|s| s.as_ref() as &(dyn postgres::types::ToSql + Sync))
            .collect();

        let rows = client.query(&query, &params_refs).map_err(|e| e.to_string())?;
        
        let mut ids = Vec::new();
        for row in rows {
            // ID is likely primary key, could be integer or string.
            let id: String = if let Ok(s) = row.try_get("id") {
                s
            } else if let Ok(i) = row.try_get::<_, i64>("id") {
                i.to_string()
            } else {
                "".to_string()
            };
            ids.push(id);
        }
        
        Ok(ids)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
         println!("PostgresDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("Preparing Postgres datastore: {:?}", connection_info);
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
            name: "connection_string".to_string(),
            description: "Postgres Connection String".to_string(),
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
    
    let client = if let Some(conn_str) = config.get("connection_string") {
        postgres::Client::connect(conn_str, postgres::NoTls).ok()
    } else {
        None
    };

    let driver = Box::new(PostgresPersistenceDriver { client: Mutex::new(client) });
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut PostgresPersistenceDriver);
    }
}


#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void, 
    data_json: *const c_char, 
    location: *const c_char
) -> i32 {
    let driver = &*(ctx as *mut PostgresPersistenceDriver);
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
    let driver = &*(ctx as *mut PostgresPersistenceDriver);
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
    let driver = &*(ctx as *mut PostgresPersistenceDriver);
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
            human_name: "PostgreSQL Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    // Parse friendly_name from schema using ox_fileproc
    let schema = include_str!("../ox_persistence_driver_db_postgres_config_schema.yaml");
    let friendly_name = if let Ok(serde_json::Value::Object(map)) = ox_fileproc::processor::parse_content(schema, "yaml") {
        map.get("friendly_name")
           .and_then(|v| v.as_str())
           .or_else(|| map.get("name").and_then(|v| v.as_str()))
           .map(|s| s.to_string())
           .unwrap_or("PostgreSQL".to_string())
    } else {
        "PostgreSQL".to_string()
    };

    let metadata = DriverMetadata {
        name: "ox_persistence_driver_postgresql".to_string(),
        friendly_name: Some(friendly_name),
        description: "A PostgreSQL persistence driver.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = include_str!("../ox_persistence_driver_db_postgres_config_schema.yaml");
    CString::new(schema).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}