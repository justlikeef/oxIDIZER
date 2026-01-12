
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

use ox_forms::schema::{FormDefinition, ModuleSchema};
use ox_persistence::{ConfiguredDriver, DriversList};

use ox_fileproc::{process_file, RawFile};

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
    #[serde(default = "default_drivers_file")]
    pub drivers_file: String,
    #[serde(default = "default_driver_root")]
    pub driver_root: String,
}

fn default_drivers_file() -> String { "/var/repos/oxIDIZER/conf/drivers.yaml".to_string() }
fn default_driver_root() -> String { "/var/repos/oxIDIZER/conf/drivers".to_string() }

pub struct DataSourceManager {
    config: DataSourceManagerConfig,
}

impl DataSourceManager {
    pub fn new(config: DataSourceManagerConfig) -> Self {
        DataSourceManager { config }
    }

    pub fn load_configured_drivers(&self) -> Result<DriversList, String> {
        let path = Path::new(&self.config.drivers_file);
        if !path.exists() {
            return Ok(DriversList { drivers: Vec::new() });
        }
         // Max depth 5 for recursion
        let val = process_file(path, 5).map_err(|e| e.to_string())?;
        serde_json::from_value(val).map_err(|e| e.to_string())
    }
    
    pub fn get_driver_schema(&self, library_path: &str) -> Result<String, String> {
         unsafe {
            let lib = libloading::Library::new(library_path).map_err(|e| format!("Failed to load library '{}': {}", library_path, e))?;
            
            let get_schema: libloading::Symbol<unsafe extern "C" fn() -> *mut libc::c_char> = 
                lib.get(b"ox_driver_get_config_schema").map_err(|_| "Missing symbol: ox_driver_get_config_schema".to_string())?;
            
            let ptr = get_schema();
            if ptr.is_null() {
                return Err("ox_driver_get_config_schema returned null".to_string());
            }
            
            let c_str = std::ffi::CStr::from_ptr(ptr);
             let schema_str = c_str.to_string_lossy().into_owned();
             
             Ok(schema_str)
        }
    }

    pub fn get_data_source(&self, id: &str) -> Result<Option<DataSource>, String> {
         let dir_path = Path::new(&self.config.data_sources_dir);
         let file_path = dir_path.join(format!("{}.yaml", id)); // Assume yaml for now
         let file_path_json = dir_path.join(format!("{}.json", id));
         
         let path = if file_path.exists() {
             file_path
         } else if file_path_json.exists() {
             file_path_json
         } else {
             return Ok(None);
         };

         let val = process_file(&path, 5).map_err(|e| e.to_string())?;
         // Try single
         if let Ok(ds) = serde_json::from_value::<DataSource>(val.clone()) {
              if ds.id == id { return Ok(Some(ds)); }
         } 
         // Try list
         if let Ok(list) = serde_json::from_value::<DataSourcesList>(val) {
              return Ok(list.data_sources.into_iter().find(|ds| ds.id == id));
         }
         Ok(None)
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

    let drivers_file = params.get("drivers_file")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(default_drivers_file);

    let driver_root = params.get("driver_root")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(default_driver_root);

    let config = DataSourceManagerConfig {
        data_sources_dir,
        on_content_conflict,
        drivers_file,
        driver_root,
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

    // 4. Render Creation Form
    // Path: /data_sources/new/form
    if method == "get" && (clean_path == "new/form" || path_str.ends_with("/data_sources/new/form")) {
        let driver_id = ctx.get("request.query.driver")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        
        let ds_id = ctx.get("request.query.id")
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let mut existing_ds: Option<DataSource> = None;
        let mut driver_id_to_use = driver_id.clone();
        
        // If ID provided, load existing DS to get driver_id and values
        if let Some(ref id) = ds_id {
             match context.manager.get_data_source(id) {
                 Ok(Some(ds)) => {
                     driver_id_to_use = ds.driver_id.clone();
                     existing_ds = Some(ds);
                 },
                 Ok(None) => {
                      send_json_error(format!("Data source '{}' not found", id), 404);
                      return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
                 },
                 Err(e) => {
                      send_json_error(format!("Error loading data source: {}", e), 500);
                      return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
                 }
             }
        }

        if driver_id_to_use.is_empty() {
             send_json_error("Missing 'driver' query parameter and no 'id' provided for lookup.".to_string(), 400);
             return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
        }

        // Load Drivers Config to find library
        let drivers_list = match context.manager.load_configured_drivers() {
             Ok(l) => l,
             Err(e) => {
                  send_json_error(format!("Failed to load drivers config: {}", e), 500);
                  return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
             }
        };

        let driver_opt = drivers_list.drivers.iter().find(|d| d.id == driver_id_to_use);
        if driver_opt.is_none() {
             send_json_error(format!("Driver '{}' not found in configuration.", driver_id_to_use), 404);
             return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
        }
        let driver_conf = driver_opt.unwrap();
        
        let lib_path = if !driver_conf.library_path.is_empty() {
             format!("{}/lib{}.so", driver_conf.library_path, driver_conf.name)
        } else {
             format!("{}/lib{}.so", context.manager.config.driver_root, driver_conf.name)
        };

        // Load Schema
        let schema_yaml = match context.manager.get_driver_schema(&lib_path) {
             Ok(s) => s,
             Err(e) => {
                  send_json_error(format!("Failed to load schema from driver: {}", e), 500);
                  return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
             }
        };

        let mut props: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
        if let Some(ds) = existing_ds {
            if let Some(obj) = ds.config.as_object() {
                for (k, v) in obj {
                    props.insert(k.clone(), v.clone());
                }
            }
        }

        // Parse Form Definition or Module Schema
        let render_res = if let Ok(module) = serde_yaml::from_str::<ModuleSchema>(&schema_yaml) {
            // Determine which form to render. 
            // We search for a "main" form or use the first one.
            let form_id = module.forms.iter().find(|f| f.id.contains("main")).map(|f| f.id.as_str()).unwrap_or(module.forms[0].id.as_str());
            ox_forms::render_standard_module(&module, form_id, &props)
        } else {
            // Fallback to single form
            serde_yaml::from_str::<FormDefinition>(&schema_yaml)
                .map_err(|e| anyhow::anyhow!("Schema Parse Error: {}", e))
                .and_then(|form_def| ox_forms::render_standard_form(&form_def, &props))
        };

        match render_res {
            Ok(html) => {
                let _ = ctx.set("response.body", serde_json::Value::String(html));
                let _ = ctx.set("response.status", serde_json::json!(200));
                let _ = ctx.set("response.header.Content-Type", serde_json::Value::String("text/html".to_string()));
            },
            Err(e) => {
                let err_msg = format!("Form Render Error: {}", e);
                if let Ok(c_msg) = std::ffi::CString::new(err_msg.clone()) {
                    let module_name = std::ffi::CString::new(context.module_id.clone()).unwrap_or(std::ffi::CString::new("ox_persistence_datasource_manager").unwrap());
                    unsafe { (context.api.log_callback)(LogLevel::Error, module_name.as_ptr(), c_msg.as_ptr()); }
                }
                send_json_error(err_msg, 500);
            },
        }
        return HandlerResult { status: ModuleStatus::Modified, flow_control: FlowControl::Continue, return_parameters: ReturnParameters { return_data: std::ptr::null_mut() } };
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
