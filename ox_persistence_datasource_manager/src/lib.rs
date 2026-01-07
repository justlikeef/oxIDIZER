
use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};
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

use ox_fileproc::process_file;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DataSource {
    pub id: String,
    pub name: String,
    pub driver_id: String,
    pub config: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DataSourcesList {
    #[serde(default)]
    pub data_sources: Vec<DataSource>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[allow(non_camel_case_types)]
pub enum ContentConflictAction {
    Overwrite,
    Append,
    Skip,
    Error,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DataSourceManagerConfig {
    pub data_sources_dir: String,
    #[serde(default)]
    pub on_content_conflict: Option<ContentConflictAction>,
}

pub struct DataSourceManager {
    config: DataSourceManagerConfig,
}

impl DataSourceManager {
    pub fn new(config: DataSourceManagerConfig) -> Self {
        DataSourceManager { config }
    }

    pub fn load_data_sources(&self) -> Result<DataSourcesList, String> {
        let mut data_sources = Vec::new();
        let dir_path = Path::new(&self.config.data_sources_dir);

        if dir_path.exists() {
            let entries = fs::read_dir(dir_path).map_err(|e| e.to_string())?;
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("yaml") || path.extension().and_then(|s| s.to_str()) == Some("json") {
                    let val = process_file(&path, 5).map_err(|e| format!("Error processing {:?}: {}", path, e))?;
                    if let Ok(ds) = serde_json::from_value::<DataSource>(val.clone()) {
                         data_sources.push(ds);
                    } else if let Ok(list) = serde_json::from_value::<DataSourcesList>(val) {
                         // Support multiple data sources in one file within the directory
                         data_sources.extend(list.data_sources);
                    }
                }
            }
        }
        
        Ok(DataSourcesList { data_sources })
    }

    pub fn save_data_source(&self, ds: &DataSource) -> Result<(), String> {
        let dir_path = Path::new(&self.config.data_sources_dir);
        if !dir_path.exists() {
            fs::create_dir_all(dir_path).map_err(|e| e.to_string())?;
        }
        
        let file_name = format!("{}.yaml", ds.id);
        let file_path = dir_path.join(file_name);
        
        let content = serde_json::to_string_pretty(ds).map_err(|e| e.to_string())?;
        fs::write(file_path, content).map_err(|e| e.to_string())
    }

    pub fn add_data_source(&self, ds: DataSource) -> Result<(), String> {
        // Just save the individual file
        self.save_data_source(&ds)
    }

    pub fn remove_data_source(&self, id: &str) -> Result<(), String> {
        let dir_path = Path::new(&self.config.data_sources_dir);
        let file_name = format!("{}.yaml", id);
        let file_path = dir_path.join(file_name);
        
        if file_path.exists() {
            fs::remove_file(file_path).map_err(|e| e.to_string())
        } else {
            Ok(()) // Already gone
        }
    }
}

pub struct ModuleContext {
    manager: Arc<DataSourceManager>,
    api: &'static CoreHostApi,
    module_id: String,
}

lazy_static! {
    static ref DATA_SOURCE_MANAGER_INSTANCE: Mutex<Option<Arc<DataSourceManager>>> = Mutex::new(None);
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
        "ox_persistence_datasource_manager".to_string()
    };

    let _ = ox_webservice_api::init_logging(api.log_callback, &module_id_str);

    let params_str = if !module_params_json_ptr.is_null() {
        unsafe { CStr::from_ptr(module_params_json_ptr).to_string_lossy().to_string() }
    } else {
        "{}".to_string()
    };
    
    let params: serde_json::Value = serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);

    let data_sources_dir = params.get("data_sources_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("/var/repos/oxIDIZER/ox_persistence/conf/datastores").to_string();

    let on_content_conflict = params.get("on_content_conflict")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "overwrite" => Some(ContentConflictAction::Overwrite),
            "append" => Some(ContentConflictAction::Append),
            "skip" => Some(ContentConflictAction::Skip),
            "error" => Some(ContentConflictAction::Error),
            _ => None,
        });

    let config = DataSourceManagerConfig {
        data_sources_dir,
        on_content_conflict,
    };

    let manager = Arc::new(DataSourceManager::new(config));
    
    *DATA_SOURCE_MANAGER_INSTANCE.lock().unwrap() = Some(manager.clone());

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

    // --- Conflict Management ---
    let is_modified = if let Some(val) = ctx.get("pipeline.modified") {
         val.as_str().unwrap_or("false") == "true"
    } else { false };

    if is_modified {
         let action = context.manager.config.on_content_conflict.unwrap_or(ContentConflictAction::Skip);
         if let Ok(c_msg) = CString::new(format!("Conflict check: is_modified=true, action={:?}", action)) {
              let module_name = CString::new(context.module_id.clone()).unwrap_or(CString::new("ox_persistence_datasource_manager").unwrap());
              unsafe { (context.api.log_callback)(LogLevel::Info, module_name.as_ptr(), c_msg.as_ptr()); }
         }

         match action {
             ContentConflictAction::Skip => {
                 return HandlerResult {
                    status: ModuleStatus::Unmodified,
                    flow_control: FlowControl::Continue,
                    return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
                 };
             },
             ContentConflictAction::Error => {
                 let _ = ctx.set("response.status", serde_json::json!(500));
                 let _ = ctx.set("response.body", serde_json::json!("Conflict: Pipeline content already modified"));
                 return HandlerResult {
                     status: ModuleStatus::Modified,
                     flow_control: FlowControl::Continue,
                     return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
                 };
             },
             _ => {} // overwrite/append: proceed as normal
         }
    }

    // --- Path Resolution ---
    // User Requirement: Module should not know about /data_sources section.
    // We prioritize 'request.capture' which the Router sets to the relative path.
    
    let (path_str, using_capture) = match ctx.get("request.capture") {
        Some(v) => (v.as_str().unwrap_or("").to_string(), true),
        None => {
            // Fallback to full path if Router didn't run or capture didn't match.
            // In this case, we might still be receiving the full path.
            let full = match ctx.get("request.resource") {
                Some(v) => v.as_str().unwrap_or("/").to_string(),
                None => match ctx.get("request.path") {
                    Some(v) => v.as_str().unwrap_or("/").to_string(),
                    None => "/".to_string()
                }
            };
            (full, false)
        }
    };
    
    // Normalize path: Remove leading/trailing slashes for easier matching of IDs
    let clean_path = path_str.trim_matches('/');

    let method = match ctx.get("request.verb") {
        Some(v) => v.as_str().unwrap_or("get").to_string(),
        None => match ctx.get("request.method") {
            Some(v) => v.as_str().unwrap_or("GET").to_string().to_lowercase(),
            None => "get".to_string()
        }
    };

    let send_json_error = |error_msg: String, status_code: i32| {
        let json_error = serde_json::json!({ "error": error_msg });
        let _ = ctx.set("response.body", serde_json::Value::String(json_error.to_string()));
        let _ = ctx.set("response.status", serde_json::json!(status_code));
        let _ = ctx.set("response.header.Content-Type", serde_json::Value::String("application/json".to_string()));
    };

    let send_json_success = |body: String| {
        let _ = ctx.set("response.body", serde_json::Value::String(body));
        let _ = ctx.set("response.status", serde_json::json!(200));
        let _ = ctx.set("response.header.Content-Type", serde_json::Value::String("application/json".to_string()));
    };

    // --- Routing Logic ---

    // 1. List Data Sources
    // MATCH: clean_path is empty (root)
    // Legacy fallback: full path is "/data_sources"
    let is_root = clean_path.is_empty() || (!using_capture && clean_path == "data_sources");

    if method == "get" && is_root {
        match context.manager.load_data_sources() {
            Ok(list) => send_json_success(serde_json::to_string(&list).unwrap_or_default()),
            Err(e) => send_json_error(e, 500),
        }
        return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
    }

    // 2. Add Data Source
    // MATCH: clean_path is empty (root)
    if method == "create" && is_root {
        let body = match ctx.get("request.payload") {
            Some(v) => v.as_str().unwrap_or("{}").to_string(),
            None => {
                match ctx.get("request.body_path") {
                    Some(path_val) => {
                        let path_str = path_val.as_str().unwrap_or("");
                        if !path_str.is_empty() {
                            std::fs::read_to_string(path_str).unwrap_or_else(|_| "{}".to_string())
                        } else { "{}".to_string() }
                    },
                    None => {
                        send_json_error("Missing body".to_string(), 400);
                        return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
                    }
                }
            }
        };
        match serde_json::from_str::<DataSource>(&body) {
            Ok(ds) => {
                match context.manager.add_data_source(ds) {
                    Ok(_) => send_json_success(serde_json::json!({"status": "created"}).to_string()),
                    Err(e) => send_json_error(e, 500),
                }
            },
            Err(e) => send_json_error(format!("Invalid JSON: {}", e), 400),
        }
        return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
    }

    // 3. Delete Data Source
    // MATCH: clean_path is NOT empty (it is the ID)
    // Legacy fallback: path starts with "data_sources/"
    if method == "delete" {
        let id_opt = if using_capture {
            if !clean_path.is_empty() { Some(clean_path) } else { None }
        } else {
             if path_str.starts_with("/data_sources/") {
                 Some(path_str.trim_start_matches("/data_sources/").trim_matches('/'))
             } else { None }
        };

        if let Some(id) = id_opt {
             match context.manager.remove_data_source(id) {
                Ok(_) => send_json_success(serde_json::json!({"status": "deleted"}).to_string()),
                Err(e) => send_json_error(e, 500),
            }
            return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
        }
    }

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
    if instance_ptr.is_null() { return std::ptr::null_mut(); }
    let context = unsafe { &*(instance_ptr as *mut ModuleContext) };
    let json = serde_json::to_string(&context.manager.config).unwrap_or("{}".to_string());
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
}
