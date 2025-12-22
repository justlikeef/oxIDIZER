
use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use std::sync::{Arc, Mutex};
use std::ffi::{CString, CStr};
use libc::{c_char, c_void};
use ox_webservice_api::{
    WebServiceApiV1, ModuleInterface, PipelineState, HandlerResult,
    LogCallback, AllocFn, AllocStrFn,
    ModuleStatus, FlowControl, ReturnParameters, LogLevel,
};
use lazy_static::lazy_static;

use ox_fileproc::{process_file, RawFile};




#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DriverManagerConfig {
    pub drivers_file: String,
    pub driver_root: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfiguredDriver {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub library_path: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DriversList {
    #[serde(default)]
    pub drivers: Vec<ConfiguredDriver>,
}

pub struct DriverManager {
    config: DriverManagerConfig,
}

impl DriverManager {
    pub fn new(config: DriverManagerConfig) -> Self {
        DriverManager { config }
    }

    pub fn list_available_driver_files(&self) -> Result<Vec<String>, String> {
        let root = Path::new(&self.config.driver_root);
        if !root.exists() {
            return Err(format!("Driver root directory does not exist: {}", self.config.driver_root));
        }

        let mut files = Vec::new();
        for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    let ext_str = ext.to_string_lossy();
                    if ext_str == "so" || ext_str == "dll" || ext_str == "dylib" {
                        if let Ok(stripped) = path.strip_prefix(root) {
                            files.push(stripped.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        Ok(files)
    }

    pub fn load_configured_drivers(&self) -> Result<DriversList, String> {
        let path_str = &self.config.drivers_file;
        let path = Path::new(path_str);
        
        if !path.exists() {
             return Ok(DriversList { drivers: Vec::new() });
        }
        
        // Use ox_fileproc::process_file (supports JSON/YAML and includes)
        // Max depth 5 for recursion
        let val = process_file(path, 5).map_err(|e| e.to_string())?;
        
        // Convert Value to DriversList
        serde_json::from_value(val).map_err(|e| e.to_string())
    }

    pub fn save_configured_drivers(&self, list: &DriversList) -> Result<(), String> {
         let content = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
         if let Some(parent) = Path::new(&self.config.drivers_file).parent() {
             fs::create_dir_all(parent).map_err(|e| e.to_string())?;
         }
         fs::write(&self.config.drivers_file, content).map_err(|e| e.to_string())
    }

    pub fn get_driver_metadata(&self, library_path: &str) -> Result<String, String> {
        unsafe {
            let lib = libloading::Library::new(library_path).map_err(|e| format!("Failed to load library '{}': {}", library_path, e))?;
            
            let get_metadata: libloading::Symbol<unsafe extern "C" fn() -> *mut libc::c_char> = 
                lib.get(b"ox_driver_get_driver_metadata").map_err(|_| "Missing symbol: ox_driver_get_driver_metadata".to_string())?;
            
            let ptr = get_metadata();
            if ptr.is_null() {
                return Err("ox_driver_get_driver_metadata returned null".to_string());
            }
            
            let c_str = std::ffi::CStr::from_ptr(ptr);
             let meta_str = c_str.to_string_lossy().into_owned();
             
             Ok(meta_str)
        }
    }

    pub fn toggle_driver_status(&self, id: &str) -> Result<String, String> {
         let mut raw = RawFile::open(&self.config.drivers_file).map_err(|e| e.to_string())?;
         
         // We must construct the filter syntax.
         // The ID in the file is quoted, so we must quote it in the query too.
         let query = format!("drivers[id=\"{}\"]/state", id);
         

         let span_and_quoted = raw.find(&query).next().map(|c| {
             let val = c.value().trim();
             (c.span.clone(), val.starts_with('"'), val.trim_matches('"').to_string())
         });
         
         if let Some((span, is_quoted, current_val)) = span_and_quoted {
              let new_status_val = if current_val == "enabled" { "disabled" } else { "enabled" };
              let replacement = if is_quoted { format!("\"{}\"", new_status_val) } else { new_status_val.to_string() };
              
              raw.update(span, &replacement);
              raw.save().map_err(|e| e.to_string())?;
              Ok(new_status_val.to_string())
         } else {
             Err(format!("Driver with ID '{}' not found or has no state field", id))
         }
    }
}

pub struct ModuleContext {
    manager: Arc<DriverManager>,
    api: &'static WebServiceApiV1,
}

lazy_static! {
    static ref DRIVER_MANAGER_INSTANCE: Mutex<Option<Arc<DriverManager>>> = Mutex::new(None);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    module_id: *const c_char,
    api_ptr: *const WebServiceApiV1,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api = unsafe { &*api_ptr };

    let module_id_str = if !module_id.is_null() {
        unsafe { CStr::from_ptr(module_id).to_string_lossy().to_string() }
    } else {
        "unknown_module".to_string()
    };

    let _ = ox_webservice_api::init_logging(api.log_callback, &module_id_str);

    let params_str = if !module_params_json_ptr.is_null() {
        unsafe { CStr::from_ptr(module_params_json_ptr).to_string_lossy().to_string() }
    } else {
        "{}".to_string()
    };
    
    let params: serde_json::Value = serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);

    let drivers_file = params.get("drivers_file").and_then(|v| v.as_str()).unwrap_or("/var/repos/oxIDIZER/conf/drivers.json").to_string();
    let driver_root = params.get("driver_root").and_then(|v| v.as_str()).unwrap_or("/var/repos/oxIDIZER/conf/drivers").to_string();

    let config = DriverManagerConfig {
        drivers_file,
        driver_root,
    };

    let manager = Arc::new(DriverManager::new(config));
    
    *DRIVER_MANAGER_INSTANCE.lock().unwrap() = Some(manager.clone());

    let ctx = Box::new(ModuleContext {
        manager,
        api,
    });

    let interface = Box::new(ModuleInterface {
        instance_ptr: Box::into_raw(ctx) as *mut c_void,
        handler_fn: process_request,
        log_callback: api.log_callback,
        get_config: get_config,
    });

    Box::into_raw(interface)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn process_request(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    _log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void,
) -> HandlerResult {
    if instance_ptr.is_null() {
        return HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }
    let context = unsafe { &*(instance_ptr as *mut ModuleContext) };
    let pipeline_state = unsafe { &mut *pipeline_state_ptr };

    let path = pipeline_state.request_path.clone();
    let method = pipeline_state.request_method.clone();

    // Handle Toggle Status Request
    if method == "POST" && path.starts_with("/drivers/") {
         if let Some(id) = path.strip_prefix("/drivers/") {
             if !id.is_empty() && id != "available" {
                 let manager = &context.manager;
                 match manager.toggle_driver_status(id) {
                     Ok(new_status) => {
                         pipeline_state.response_body = format!("{{\"status\": \"{}\"}}", new_status).into_bytes();
                         pipeline_state.status_code = 200;
                         let ct_k = CString::new("Content-Type").unwrap();
                         let ct_v = CString::new("application/json").unwrap();
                         unsafe { (context.api.set_response_header)(pipeline_state, ct_k.as_ptr(), ct_v.as_ptr()); }
                     },
                     Err(e) => {
                         let err = format!("Failed to toggle driver (file: {}): {}", manager.config.drivers_file, e);
                         let log_msg = CString::new(err.clone()).unwrap();
                         let module_name = CString::new("ox_persistence_datastore_drivermanager").unwrap();
                         unsafe { (context.api.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }

                         pipeline_state.response_body = err.into_bytes();
                         pipeline_state.status_code = 500;
                     }
                 }
                 return HandlerResult {
                    status: ModuleStatus::Modified,
                    flow_control: FlowControl::Continue,
                    return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
                };
             }
         }
    }

    if path == "/drivers/available" && method == "GET" {
        let manager = &context.manager;
        match manager.list_available_driver_files() {
            Ok(files) => {
                let json = serde_json::to_string(&files).unwrap_or_default();
                pipeline_state.response_body = json.into_bytes();
                pipeline_state.status_code = 200;
                
                let ct_k = CString::new("Content-Type").unwrap();
                let ct_v = CString::new("application/json").unwrap();
                unsafe { (context.api.set_response_header)(pipeline_state, ct_k.as_ptr(), ct_v.as_ptr()); }
            },
            Err(e) => {
                let err = format!("Failed to list available drivers: {}", e);
                pipeline_state.response_body = err.into_bytes();
                pipeline_state.status_code = 500;
            }
        }
        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }
    
    // Default: List drivers (JSON)
    if path == "/drivers" && method == "GET" {
         let manager = &context.manager; 
         
         match manager.load_configured_drivers() {
             Ok(list) => {
                let json = serde_json::to_string(&list).unwrap_or_default();
                pipeline_state.response_body = json.into_bytes();
                pipeline_state.status_code = 200;
                let ct_k = CString::new("Content-Type").unwrap();
                let ct_v = CString::new("application/json").unwrap();
                unsafe { (context.api.set_response_header)(pipeline_state, ct_k.as_ptr(), ct_v.as_ptr()); }
             },
             Err(e) => {
                let err = format!("Failed to load drivers: {}", e);
                pipeline_state.response_body = err.into_bytes();
                pipeline_state.status_code = 500;
             }
         }
         return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }

    // Default Fallback
    HandlerResult {
        status: ModuleStatus::Unmodified,
        flow_control: FlowControl::Continue,
        return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_config(
    instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    if instance_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let context = unsafe { &*(instance_ptr as *mut ModuleContext) };
    let manager = &context.manager;
    
    let json = serde_json::to_string(&manager.config).unwrap_or("{}".to_string());
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
}

#[cfg(test)]
mod functional_tests;
#[cfg(test)]
mod functional_tests_security;


