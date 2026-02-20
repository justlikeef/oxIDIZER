use ox_webservice_api::{
    WebServiceApiV1, ModuleInterface, PipelineState, HandlerResult,
    LogCallback, AllocFn, AllocStrFn, LogLevel,
    ModuleStatus, FlowControl, ReturnParameters,
};
use ox_persistence::{OxBuffer, DriverMetadata};
use libc::{c_char, c_void, c_int, size_t};
use std::ffi::{CStr, CString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use libloading::{Library, Symbol};
use std::path::Path;

// --- FFI Signature Definitions ---
type InitFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type DestroyFn = unsafe extern "C" fn(*mut c_void);
type PersistFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type RestoreFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> OxBuffer;
type FetchFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> OxBuffer;
type FreeBufferFn = unsafe extern "C" fn(OxBuffer);
type GetMetadataFn = unsafe extern "C" fn() -> *mut c_char;

struct LoadedDriver {
    library: Library,
    context: *mut c_void, // The driver instance created by init
    // Functions
    destroy_fn: DestroyFn,
    persist_fn: PersistFn,
    restore_fn: RestoreFn,
    fetch_fn: FetchFn,
    free_buffer_fn: FreeBufferFn,
    metadata: DriverMetadata,
}

// Make LoadedDriver Send+Sync so it can be in a global Mutex
unsafe impl Send for LoadedDriver {}
unsafe impl Sync for LoadedDriver {}

impl LoadedDriver {
    unsafe fn load(path: &str) -> Result<Self, String> {
        let lib = Library::new(path).map_err(|e| format!("Failed to load library: {}", e))?;

        // We use .get() to find symbols, but we copy the function pointer value out immediately.
        // Symbols from libloading implement Deref to the underlying type.
        // Since our types are `unsafe extern "C" fn...` which are Copy, we can just dereference and copy.
        let init: InitFn = *lib.get(b"ox_driver_init").map_err(|e| e.to_string())?;
        let destroy: DestroyFn = *lib.get(b"ox_driver_destroy").map_err(|e| e.to_string())?;
        let persist: PersistFn = *lib.get(b"ox_driver_persist").map_err(|e| e.to_string())?;
        let restore: RestoreFn = *lib.get(b"ox_driver_restore").map_err(|e| e.to_string())?;
        let fetch: FetchFn = *lib.get(b"ox_driver_fetch").map_err(|e| e.to_string())?;
        let free_buf: FreeBufferFn = *lib.get(b"ox_driver_free_buffer").map_err(|e| e.to_string())?;
        let get_meta: GetMetadataFn = *lib.get(b"ox_driver_get_driver_metadata").map_err(|e| e.to_string())?;

        // Initialize
        let config = CString::new("{}").unwrap();
        let ctx = init(config.as_ptr());

        // Get Metadata
        let meta_ptr = get_meta();
        let meta_str = CStr::from_ptr(meta_ptr).to_string_lossy();
        let metadata: DriverMetadata = serde_json::from_str(&meta_str).map_err(|e| e.to_string())?;
        
        Ok(LoadedDriver {
            library: lib,
            context: ctx,
            destroy_fn: destroy,
            persist_fn: persist,
            restore_fn: restore,
            fetch_fn: fetch,
            free_buffer_fn: free_buf,
            metadata,
        })
    }
}

impl Drop for LoadedDriver {
    fn drop(&mut self) {
        unsafe {
            (self.destroy_fn)(self.context);
        }
    }
}

struct DriverManager {
    drivers: HashMap<String, LoadedDriver>,
}

lazy_static::lazy_static! {
    static ref DRIVER_MANAGER: Mutex<DriverManager> = Mutex::new(DriverManager {
        drivers: HashMap::new(),
    });
}

// Helper to log
unsafe fn log(cb: LogCallback, level: LogLevel, msg: &str) {
    let m = CString::new("ox_data_broker").unwrap();
    let msg_c = CString::new(msg).unwrap();
    cb(level, m.as_ptr(), msg_c.as_ptr());
}

// Main Handler
#[no_mangle]
pub unsafe extern "C" fn broker_handler(_instance_ptr: *mut c_void, pipeline_state_ptr: *mut PipelineState, log_callback: LogCallback, _alloc_fn: AllocFn, _arena: *const c_void) -> HandlerResult {
    let pipeline_state = &mut *pipeline_state_ptr;
    let request_path = &pipeline_state.request_path;
    let method = &pipeline_state.request_method;
    
    // Simple Routing
    if request_path == "/drivers" && method == "GET" {
        let manager = DRIVER_MANAGER.lock().unwrap();
        let meta_list: Vec<&DriverMetadata> = manager.drivers.values().map(|d| &d.metadata).collect();
        let json = serde_json::to_string(&meta_list).unwrap_or_default();
        pipeline_state.response_body = json.into_bytes();
        pipeline_state.status_code = 200;
        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        };
    }

    if request_path == "/drivers/reload" && method == "POST" {
        use ox_fileproc::process_file;
        use ox_persistence::DriversList;

        let mut manager = DRIVER_MANAGER.lock().unwrap();
        let drivers_path = Path::new("conf/drivers.yaml"); // Assuming running from root
        
        let mut loaded_count = 0;
        let mut errors = Vec::new();

        if drivers_path.exists() {
             match process_file(drivers_path, 5) {
                 Ok(val) => {
                     // Deserialize
                     if let Ok(list) = serde_json::from_value::<DriversList>(val) {
                         for driver_conf in list.drivers {
                             if driver_conf.state == "enabled" {
                                 // Determine path: Explicit library_path OR inferred from name
                                 let dir_path = if !driver_conf.library_path.is_empty() {
                                     Path::new(&driver_conf.library_path).to_path_buf()
                                 } else {
                                     Path::new("target/debug").to_path_buf()
                                 };

                                 #[cfg(target_os = "windows")]
                                 let filename = format!("{}.dll", driver_conf.name);
                                 #[cfg(target_os = "macos")]
                                 let filename = format!("lib{}.dylib", driver_conf.name);
                                 #[cfg(target_os = "linux")]
                                 let filename = format!("lib{}.so", driver_conf.name);
                                 #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
                                 let filename = format!("lib{}.so", driver_conf.name); // Default fallback

                                 let lib_path = dir_path.join(filename);
                                 let lib_path_str = lib_path.to_string_lossy().to_string();

                                 match LoadedDriver::load(&lib_path_str) {
                                     Ok(loaded) => {
                                         let name = loaded.metadata.name.clone();
                                         manager.drivers.insert(name.clone(), loaded);
                                         log(log_callback, LogLevel::Info, &format!("Loaded driver: {}", name));
                                         loaded_count += 1;
                                     },
                                     Err(e) => {
                                         let msg = format!("Failed to load driver '{}' from '{}': {}", driver_conf.name, lib_path_str, e);
                                         log(log_callback, LogLevel::Error, &msg);
                                         errors.push(msg);
                                     }
                                 }
                             }
                         }
                     } else {
                         errors.push("Failed to deserialize drivers list".to_string());
                     }
                 },
                 Err(e) => errors.push(format!("Failed to process drivers config file: {}", e))
             }
        } else {
            errors.push("drivers.yaml not found".to_string());
        }

        let msg = if errors.is_empty() {
            format!("Reloaded {} drivers successfully.", loaded_count)
        } else {
             format!("Reloaded {} drivers. Errors: {:?}", loaded_count, errors)
        };
        
        pipeline_state.status_code = if errors.is_empty() { 200 } else { 500 };
        pipeline_state.response_body = msg.into_bytes();

        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        };
    }

    // Generic Operation Request: /data/{driver_name}/{operation}
    // E.g. /data/ox_persistence_datastore_flatfile/persist
    if request_path.starts_with("/data/") {
        let parts: Vec<&str> = request_path.split('/').collect();
        if parts.len() < 4 {
            pipeline_state.status_code = 400;
            return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            };
        }
        let driver_name = parts[2];
        let operation = parts[3];

        let manager = DRIVER_MANAGER.lock().unwrap();
        if let Some(driver) = manager.drivers.get(driver_name) {
            
            // Extract body for POST
            let body = String::from_utf8_lossy(&pipeline_state.request_body).into_owned();

            // Hardcoded "location" for now, or extract from headers/query?
            // Let's assume passed in header "X-Location" or defaulting
            let location = CString::new("data.csv").unwrap(); // Default for testing

            match (method.as_ref(), operation) {
                ("POST", "persist") => {
                    let data_c = CString::new(body).unwrap_or_default();
                    let result = (driver.persist_fn)(driver.context, data_c.as_ptr(), location.as_ptr());
                    if result == 0 {
                        pipeline_state.status_code = 200;
                        pipeline_state.response_body = "Persisted".as_bytes().to_vec();
                    } else {
                        pipeline_state.status_code = 500;
                        pipeline_state.response_body = "Persistence failed".as_bytes().to_vec();
                    }
                },
                ("GET", "restore") => {
                    // Assume ID is in query string or body? Let's use body for simplicity or header
                    // Using body for ID for now
                    let id_c = CString::new(body).unwrap_or_default();
                    let buf = (driver.restore_fn)(driver.context, location.as_ptr(), id_c.as_ptr());
                    let json = buf.to_string();
                    (driver.free_buffer_fn)(buf);
                    pipeline_state.response_body = json.into_bytes();
                    pipeline_state.status_code = 200;
                },
                ("POST", "fetch") => {
                    let filter_c = CString::new(body).unwrap_or_default();
                    let buf = (driver.fetch_fn)(driver.context, filter_c.as_ptr(), location.as_ptr());
                    let json = buf.to_string();
                    (driver.free_buffer_fn)(buf);
                    pipeline_state.response_body = json.into_bytes();
                    pipeline_state.status_code = 200;
                },
                _ => {
                    pipeline_state.status_code = 405; // Method not allowed
                }
            }

        } else {
            pipeline_state.status_code = 404;
            pipeline_state.response_body = format!("Driver {} not found", driver_name).into_bytes();
        }
        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        };
    }

    // Default 404
    pipeline_state.status_code = 404;
    HandlerResult {
        status: ModuleStatus::Modified,
        flow_control: FlowControl::Continue,
        return_parameters: ReturnParameters {
            return_data: std::ptr::null_mut(),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn initialize_module(_module_params_json_ptr: *const c_char, _module_id: *const c_char, api: *const WebServiceApiV1) -> *mut ModuleInterface {
    let module_interface = Box::new(ModuleInterface {
        instance_ptr: std::ptr::null_mut(),
        handler_fn: broker_handler,
        log_callback: (*api).log_callback,
        get_config: get_config_c,
    });
    // On init, we could try to auto-load drivers, but explicit reload endpoint is safer for now.
    Box::into_raw(module_interface)
}

unsafe extern "C" fn get_config_c(
    _instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    let json = "null";
    alloc_fn(arena, CString::new(json).unwrap().as_ptr())
}