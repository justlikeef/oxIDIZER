use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use std::collections::HashMap;
use ox_type_converter::ValueType;
use std::ffi::{c_void, CString, CStr};
use libc::c_char;
use std::sync::Arc;
use chrono;


use tokio::runtime::Runtime;
use tokio_util::compat::TokioAsyncWriteCompatExt;
use tiberius::{Client, Config, AuthMethod};
use futures::StreamExt;
use std::net::ToSocketAddrs;

pub struct MssqlPersistenceDriver {
    runtime: Runtime,
    connection_string: Option<String>,
}

impl PersistenceDriver for MssqlPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        let conn_str = self.connection_string.as_ref().ok_or("Connection string not set")?;
        // Parse simple connection string: server=host;user=u;password=p;port=1433
        // For this demo, let's assume we parse it or use a config map if passed.
        // But here we only have the string.
        
        let rt = &self.runtime;
        
        rt.block_on(async {
            let config = Config::from_ado_string(conn_str).map_err(|e| e.to_string())?;
            let tcp = tokio::net::TcpStream::connect(config.get_addr()).await.map_err(|e| e.to_string())?;
            let tcp = tcp.compat_write();
            let mut client = Client::connect(config, tcp).await.map_err(|e| e.to_string())?;

            use ox_persistence_driver_sql::{SqlBuilder, SqlDialect};
            let builder = SqlBuilder::new(SqlDialect::Mssql);
            let keys: Vec<String> = serializable_map.keys().cloned().collect();
            let query = builder.build_insert(location, &keys);

            let mut query_obj = tiberius::Query::new(query);
            for k in &keys {
                let (v, t, _) = &serializable_map[k];
                match t {
                    ValueType::Integer => {
                        if let Ok(i) = v.parse::<i64>() {
                             query_obj.bind(i);
                        } else {
                             query_obj.bind(v.as_str());
                        }
                    },
                    ValueType::Float => {
                        if let Ok(f) = v.parse::<f64>() {
                             query_obj.bind(f);
                        } else {
                             query_obj.bind(v.as_str());
                        }
                    },
                    ValueType::Boolean => {
                         if let Ok(b) = v.parse::<bool>() {
                             query_obj.bind(b);
                         } else {
                             query_obj.bind(v.as_str());
                         }
                    },
                    ValueType::DateTime => {
                         if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(v) {
                             query_obj.bind(dt);
                         } else {
                             query_obj.bind(v.as_str());
                         }
                    },
                    _ => query_obj.bind(v.as_str()),
                }
            }

            query_obj.execute(&mut client).await.map_err(|e| e.to_string())?;
            
            Ok::<(), String>(())
        }).map_err(|e| e.to_string())?;

        Ok(())
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let conn_str = self.connection_string.as_ref().ok_or("Connection string not set")?;
        let rt = &self.runtime;

        rt.block_on(async {
            let config = Config::from_ado_string(conn_str).map_err(|e| e.to_string())?;
            let tcp = tokio::net::TcpStream::connect(config.get_addr()).await.map_err(|e| e.to_string())?;
            let tcp = tcp.compat_write();
            let mut client = Client::connect(config, tcp).await.map_err(|e| e.to_string())?;

            let query_str = format!("SELECT * FROM [{}] WHERE id = @P1", location);
            let mut query = tiberius::Query::new(query_str);
            query.bind(id);

            let result = query.query(&mut client).await.map_err(|e| e.to_string())?;
            let row = result.into_row().await.map_err(|e| e.to_string())?;

            if let Some(r) = row {
                let mut map = HashMap::new();
                for col in r.columns() {
                    let name = col.name().to_string();
                    // Tiberius row.get needs type. We can use try_get string.
                    let val: Option<&str> = r.try_get(name.as_str()).map_err(|e| e.to_string())?;
                    let val_str = val.unwrap_or_default().to_string();
                    map.insert(name, (val_str, ValueType::from("string"), HashMap::new()));
                }
                Ok(map)
            } else {
                 Err(format!("Object with id {} not found", id))
            }
        }).map_err(|e| e.to_string())
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
         let conn_str = self.connection_string.as_ref().ok_or("Connection string not set")?;
        let rt = &self.runtime;

        rt.block_on(async {
            let config = Config::from_ado_string(conn_str).map_err(|e| e.to_string())?;
            let tcp = tokio::net::TcpStream::connect(config.get_addr()).await.map_err(|e| e.to_string())?;
            let tcp = tcp.compat_write();
            let mut client = Client::connect(config, tcp).await.map_err(|e| e.to_string())?;

            let mut query_str = format!("SELECT id FROM [{}] WHERE 1=1", location);
            let mut idx = 1;

             // We need to bind dynamically, but tiberius Query bindings are appended.
            for k in filter.keys() {
                query_str.push_str(&format!(" AND [{}] = @P{}", k, idx));
                idx += 1;
            }

            let mut query = tiberius::Query::new(query_str);
            for (_, (v, t, _)) in filter {
                match t {
                    ValueType::Integer => {
                        if let Ok(i) = v.parse::<i64>() {
                             query.bind(i);
                        } else {
                             query.bind(v.as_str());
                        }
                    },
                    ValueType::Float => {
                        if let Ok(f) = v.parse::<f64>() {
                             query.bind(f);
                        } else {
                             query.bind(v.as_str());
                        }
                    },
                    ValueType::Boolean => {
                         if let Ok(b) = v.parse::<bool>() {
                             query.bind(b);
                         } else {
                             query.bind(v.as_str());
                         }
                    },
                    ValueType::DateTime => {
                         if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(v) {
                             query.bind(dt);
                         } else {
                             query.bind(v.as_str());
                         }
                    },
                    _ => query.bind(v.as_str()),
                }
            }

            let mut stream = query.query(&mut client).await.map_err(|e| e.to_string())?;
            let mut ids = Vec::new();
            
            while let Some(item) = stream.next().await {
                let item = item.map_err(|e| e.to_string())?;
                if let tiberius::QueryItem::Row(row) = item {
                    let id: Option<&str> = row.try_get("id").map_err(|e| e.to_string())?;
                    if let Some(id_val) = id {
                        ids.push(id_val.to_string());
                    }
                }
            }
            Ok::<Vec<String>, String>(ids)
        }).map_err(|e: String| e)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
         println!("MssqlDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("Preparing MSSQL datastore: {:?}", connection_info);
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
            description: "MSSQL Connection String".to_string(),
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
    
    let connection_string = config.get("connection_string").cloned();
    let runtime = Runtime::new().unwrap();

    let driver = Box::new(MssqlPersistenceDriver { runtime, connection_string });
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut MssqlPersistenceDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void, 
    data_json: *const c_char, 
    location: *const c_char
) -> i32 {
    let driver = &*(ctx as *mut MssqlPersistenceDriver);
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
    let driver = &*(ctx as *mut MssqlPersistenceDriver);
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
    let driver = &*(ctx as *mut MssqlPersistenceDriver);
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
            human_name: "MSSQL Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_driver_mssql".to_string(),
        description: "A MSSQL persistence driver.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = include_str!("../ox_persistence_driver_db_mssql_config_schema.yaml");
    CString::new(schema).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}