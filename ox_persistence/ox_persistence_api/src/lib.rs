use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use std::collections::HashMap;
use ox_type_converter::ValueType;
use std::ffi::{c_void, CString, CStr};
use libc::c_char;
use std::sync::Arc;
use reqwest::blocking; // Added for reqwest blocking client
use serde_json; // Added for parsing config JSON

pub struct OxPersistenceApiDriver {
    api_endpoint: String,
    api_key: String,
}

impl PersistenceDriver for OxPersistenceApiDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        let client = reqwest::blocking::Client::new();
        // Assume location is the API endpoint. If not, use self.api_endpoint which is loaded from config on init.
        // Actually, PersistenceDriver `persist` location argument implies specific location (e.g. file, table).
        // For API, it might be the resource path, e.g. "/users".
        // Combine with base URL.
        
        // We'll trust "location" is the full URL or handle base URL logic if we stored it.
        // The struct has `api_endpoint` field.
        let url = if location.starts_with("http") {
            location.to_string()
        } else {
             format!("{}/{}", self.api_endpoint.trim_end_matches('/'), location.trim_start_matches('/'))
        };

        // We need to convert the map to a JSON structure the API expects.
        // Sending the raw serializable_map (with types and metadata) might be too verbose for standard APIs.
        // But for "persistence", preserving types is good. Let's send it as is.
        
        let res = client.post(&url)
            .header("Authorization", &self.api_key)
            .json(serializable_map)
            .send()
            .map_err(|e| e.to_string())?;

        if res.status().is_success() {
            Ok(())
        } else {
            Err(format!("API Request failed: {}", res.status()))
        }
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let client = reqwest::blocking::Client::new();
        let url = if location.starts_with("http") {
             location.to_string()
        } else {
             format!("{}/{}/{}", self.api_endpoint.trim_end_matches('/'), location.trim_start_matches('/'), id)
        };

        let res = client.get(&url)
            .header("Authorization", &self.api_key)
            .send()
            .map_err(|e| e.to_string())?;

        if res.status().is_success() {
             let map = res.json::<HashMap<String, (String, ValueType, HashMap<String, String>)>>()
                .map_err(|e| e.to_string())?;
             Ok(map)
        } else {
             Err(format!("API Request failed: {}", res.status()))
        }
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
        let client = reqwest::blocking::Client::new();
        let url = if location.starts_with("http") {
             location.to_string()
        } else {
             format!("{}/{}", self.api_endpoint.trim_end_matches('/'), location.trim_start_matches('/'))
        };
        
        // Send filter as query params or body? POST with body is safer for complex filters.
        let res = client.post(&url)
            .header("Authorization", &self.api_key)
            .header("X-Action", "fetch") 
            .json(filter)
            .send()
            .map_err(|e| e.to_string())?;

        if res.status().is_success() {
             let ids = res.json::<Vec<String>>().map_err(|e| e.to_string())?;
             Ok(ids)
        } else {
             Err(format!("API Request failed: {}", res.status()))
        }
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("ApiDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), String> {
        Err("Not implemented".to_string())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        Err("Not implemented".to_string())
    }
    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, _dataset_name: &str) -> Result<DataSet, String> {
        Err("Not implemented".to_string())
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "api_endpoint".to_string(),
                description: "The API endpoint URL for persistence operations.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "api_key".to_string(),
                description: "API key for authentication (if required).".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
        ]
    }
}

// --- FFI Exports ---

#[no_mangle]
pub extern "C" fn ox_driver_init(config_json: *const c_char) -> *mut c_void {
    let config_str = unsafe { CStr::from_ptr(config_json).to_string_lossy() };
    let config: HashMap<String, String> = serde_json::from_str(&config_str).unwrap_or_default();
    
    let api_endpoint = config.get("api_endpoint").cloned().unwrap_or_default();
    let api_key = config.get("api_key").cloned().unwrap_or_default();

    let driver = Box::new(OxPersistenceApiDriver { api_endpoint, api_key });
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut OxPersistenceApiDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void, 
    data_json: *const c_char, 
    location: *const c_char
) -> i32 {
    let driver = &*(ctx as *mut OxPersistenceApiDriver);
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
    let driver = &*(ctx as *mut OxPersistenceApiDriver);
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
    let driver = &*(ctx as *mut OxPersistenceApiDriver);
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
            human_name: "API Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_api".to_string(),
        description: "A persistence driver that delegates to an HTTP API.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = include_str!("../config_schema.yaml");
    CString::new(schema).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}
