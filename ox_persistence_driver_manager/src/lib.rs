
use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use std::sync::{Arc, Mutex};
use std::ffi::{CString, CStr};
use libc::{c_char, c_void};
use ox_webservice_api::{
    ModuleInterface, PipelineState, HandlerResult,
    LogCallback, AllocFn, AllocStrFn,
    ModuleStatus, FlowControl, ReturnParameters, LogLevel, CoreHostApi,
};
use lazy_static::lazy_static;
use bumpalo::Bump;

use ox_fileproc::{process_file, RawFile};




#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DriverManagerConfig {
    pub drivers_file: String,
    pub driver_root: String,
}


use ox_persistence::{ConfiguredDriver, DriversList}; // Import from shared crate


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
    api: &'static CoreHostApi,
    module_id: String,
}

lazy_static! {
    static ref DRIVER_MANAGER_INSTANCE: Mutex<Option<Arc<DriverManager>>> = Mutex::new(None);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    module_id: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api = unsafe { &*api_ptr };

    let module_id_str = if !module_id.is_null() {
        unsafe { CStr::from_ptr(module_id).to_string_lossy().to_string() }
    } else {
        "ox_persistence_driver_manager".to_string()
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
        module_id: module_id_str,
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
    let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

    let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
        context.api, 
        pipeline_state_ptr as *mut c_void, 
        arena_ptr
    ) };



    let path = match ctx.get("http.request.path") {
        Some(v) => v.as_str().unwrap_or("/").to_string(),
        None => "/".to_string()
    };
    let method = match ctx.get("http.request.method") {
        Some(v) => v.as_str().unwrap_or("GET").to_string(),
        None => "GET".to_string()
    };

    // Helper closure for JSON error responses
    let send_json_error = |error_msg: String, status_code: i32| {
        let json_error = serde_json::json!({
            "error": error_msg
        });
        let _ = ctx.set("http.response.body", serde_json::Value::String(json_error.to_string()));
        let _ = ctx.set("http.response.status", serde_json::json!(status_code));
        let _ = ctx.set("http.response.header.Content-Type", serde_json::Value::String("application/json".to_string()));
    };

    // Helper closure for JSON success responses
    let send_json_success = |body: String| {
        let _ = ctx.set("http.response.body", serde_json::Value::String(body));
        let _ = ctx.set("http.response.status", serde_json::json!(200));
        let _ = ctx.set("http.response.header.Content-Type", serde_json::Value::String("application/json".to_string()));
    };

    // --- Route Dispatching ---

    // 1. POST (Toggle) - matches any path ending in a driver ID, e.g., /drivers/driver_id
    // We treat the *last segment* of the path as the ID.
    if method == "POST" {
        // Extract ID from the last path segment (ignoring trailing slash)
        let trimmed_path = path.trim_end_matches('/');
        let id = trimmed_path.split('/').last().unwrap_or("");
        
        if id.is_empty() || id == "available" { // "available" is reserved for GET
             send_json_error("Invalid driver ID provided.".to_string(), 400);
        } else {
             let manager = &context.manager;
             match manager.toggle_driver_status(id) {
                 Ok(new_status) => {
                     let response = serde_json::json!({
                         "status": new_status,
                         "id": id
                     });
                     send_json_success(response.to_string());
                 },
                 Err(e) => {
                     let err_msg = format!("Failed to toggle driver '{}': {}", id, e);
                     // Log error internally
                     if let Ok(c_msg) = CString::new(err_msg.clone()) {
                          let module_name = CString::new(context.module_id.clone()).unwrap_or(CString::new("ox_persistence_driver_manager").unwrap());
                          unsafe { (context.api.log_callback)(LogLevel::Error, module_name.as_ptr(), c_msg.as_ptr()); }
                     }
                     // Return JSON error to user
                     send_json_error(err_msg, 500);
                 }
             }
        }
        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue, // Continue as requested by user
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }

    // 2. GET /available - List available driver files from disk
    if method == "GET" && path.ends_with("/available") { // strict suffix match
        let manager = &context.manager;
        match manager.list_available_driver_files() {
            Ok(files) => {
                let json = serde_json::to_string(&files).unwrap_or_default();
                send_json_success(json);
            },
            Err(e) => {
                send_json_error(format!("Failed to list available drivers: {}", e), 500);
            }
        }
        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }

    // 3. GET (Default) - List configured drivers
    if method == "GET" {
         let manager = &context.manager;
         match manager.load_configured_drivers() {
             Ok(list) => {
                let json = serde_json::to_string(&list).unwrap_or_default();
                send_json_success(json);
             },
             Err(e) => {
                send_json_error(format!("Failed to load drivers config: {}", e), 500);
             }
         }
         return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
         };
    }

    // Capture Unhandled Methods (e.g., PUT, DELETE) if routed here
    send_json_error(format!("Method {} not allowed.", method), 405);
    
    HandlerResult {
        status: ModuleStatus::Modified,
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
