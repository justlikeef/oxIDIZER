use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use std::collections::HashMap;
use ox_type_converter::ValueType;
use std::ffi::{c_void, CString, CStr};
use libc::c_char;
use std::sync::{Arc, Mutex};
use rusqlite::{params, Connection, types::ToSql};
use serde_json;

pub struct SqlitePersistenceDriver {
    conn: Mutex<Option<Connection>>,
    connection_string: Mutex<String>,
}

impl PersistenceDriver for SqlitePersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        let mut guard = self.conn.lock().map_err(|e| e.to_string())?;
        let conn = guard.as_mut().ok_or("SQLite connection not initialized")?;

        use ox_persistence_driver_sql::{SqlBuilder, SqlDialect};
        let builder = SqlBuilder::new(SqlDialect::Sqlite);
        let keys: Vec<String> = serializable_map.keys().cloned().collect();
        // SQLite usually requires table creation. For now assumes table exists logic handled elsewhere or via raw SQL if needed.
        // But let's assume standard INSERT.
        let query = builder.build_insert(location, &keys);

        let mut params_vec: Vec<Box<dyn ToSql>> = Vec::new();
        
        for k in &keys {
            let (v, t, _) = &serializable_map[k];
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
                     // SQLite uses 0/1 for bool, but rusqlite handles bool mapping usually?
                     // Verify rusqlite ToSql implementation. It maps bool to integer.
                     if let Ok(b) = v.parse::<bool>() {
                        params_vec.push(Box::new(b));
                     } else {
                        params_vec.push(Box::new(v.clone()));
                     }
                },
                _ => params_vec.push(Box::new(v.clone())),
            }
        }
        
        let params_refs: Vec<&dyn ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        conn.execute(&query, params_refs.as_slice())
            .map_err(|e| e.to_string())?;
            
        Ok(())
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let mut guard = self.conn.lock().map_err(|e| e.to_string())?;
        let conn = guard.as_mut().ok_or("SQLite connection not initialized")?;

        use ox_persistence_driver_sql::{SqlBuilder, SqlDialect};
        let builder = SqlBuilder::new(SqlDialect::Sqlite);
        let query = builder.build_select_by_id(location);
        
        // Prepare statement
        let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
        
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let mut map = HashMap::new();
            
            // Iterate columns. Rusqlite doesn't easily let us iterate all columns generically without knowing count/names upfront 
            // unless we use `column_count()` and `column_name()`.
            let col_count = row.as_ref().column_count();
            for i in 0..col_count {
                let name = row.as_ref().column_name(i).unwrap_or("unknown").to_string();
                
                // Try to get as generic Value or String
                // rusqlite::types::ValueRef
                let val_ref = row.get_ref(i).map_err(|e| e.to_string())?;
                
                let (val_str, v_type) = match val_ref {
                    rusqlite::types::ValueRef::Null => ("".to_string(), ValueType::String),
                    rusqlite::types::ValueRef::Integer(i) => (i.to_string(), ValueType::Integer),
                    rusqlite::types::ValueRef::Real(f) => (f.to_string(), ValueType::Float),
                    rusqlite::types::ValueRef::Text(t) => (String::from_utf8_lossy(t).to_string(), ValueType::String),
                    rusqlite::types::ValueRef::Blob(_) => ("<blob>".to_string(), ValueType::Binary),
                };
                
                map.insert(name, (val_str, v_type, HashMap::new()));
            }
            Ok(map)
        } else {
             Err(format!("Object with id {} not found", id))
        }
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
         let mut guard = self.conn.lock().map_err(|e| e.to_string())?;
        let conn = guard.as_mut().ok_or("SQLite connection not initialized")?;

        use ox_persistence_driver_sql::{SqlBuilder, SqlDialect};
        let builder = SqlBuilder::new(SqlDialect::Sqlite);
        let keys: Vec<String> = filter.keys().cloned().collect();
        let query = builder.build_fetch(location, &keys);
        
        let mut params_vec: Vec<Box<dyn ToSql>> = Vec::new();
        
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
                _ => params_vec.push(Box::new(v.clone())),
            }
        }
         let params_refs: Vec<&dyn ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
         
         let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
         let mut rows = stmt.query(params_refs.as_slice()).map_err(|e| e.to_string())?;
         
        let mut ids = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            // Assume ID is first column or named "id"
            // Let's try to get "id"
            let id_val: rusqlite::Result<String> = row.get("id").or_else(|_| {
                // validation fallback to index 0 if not named id?
                // But build_fetch typically selects ID.
                // Or integer id
                row.get::<_, i64>("id").map(|i| i.to_string())
            });
            
            if let Ok(s) = id_val {
                ids.push(s);
            }
        }
        Ok(ids)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
         // No-op
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        // Could run CREATE TABLE IF NOT EXISTS here if location is known
        Ok(())
    }

    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        // List tables?
        let mut guard = self.conn.lock().map_err(|e| e.to_string())?;
        let conn = guard.as_mut().ok_or("SQLite connection not initialized")?;
        
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |row| row.get(0)).map_err(|e| e.to_string())?;
        
        let mut tables = Vec::new();
        for name in rows {
            if let Ok(n) = name {
                tables.push(n);
            }
        }
        Ok(tables)
    }
    
    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, dataset_name: &str) -> Result<DataSet, String> {
        Ok(DataSet { name: dataset_name.to_string(), columns: Vec::new() })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![ConnectionParameter {
            name: "filename".to_string(),
            description: "Path to SQLite database file".to_string(),
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
    
    let db_path = config.get("filename").cloned().unwrap_or_else(|| ":memory:".to_string());
    
    // In SQLite, connection is to a file.
    let conn = Connection::open(&db_path).ok();

    let driver = Box::new(SqlitePersistenceDriver { 
        conn: Mutex::new(conn),
        connection_string: Mutex::new(db_path)
    });
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut SqlitePersistenceDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void, 
    data_json: *const c_char, 
    location: *const c_char
) -> i32 {
    let driver = &*(ctx as *mut SqlitePersistenceDriver);
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
    let driver = &*(ctx as *mut SqlitePersistenceDriver);
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
    let driver = &*(ctx as *mut SqlitePersistenceDriver);
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
            human_name: "SQLite Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_driver_sqlite".to_string(),
        description: "A SQLite persistence driver.".to_string(),
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
