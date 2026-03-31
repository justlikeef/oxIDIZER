
use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::ffi::{CString, CStr};
use libc::{c_char, c_void};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE,
    OX_LOG_INFO, OX_LOG_ERROR,
};
use lazy_static::lazy_static;
use prost::Message;

use ox_persistence::DriversList;

use ox_fileproc::process_file;

/// Prost-compatible mirror of DataSource for binary encoding.
/// The `config` field (serde_json::Value) is serialised to JSON and stored in `config_json`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct DataSourceProto {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub driver_id: String,
    /// JSON-encoded representation of the driver config map.
    #[prost(string, tag = "4")]
    pub config_json: String,
}

/// Prost-compatible list wrapper.
#[derive(Clone, PartialEq, prost::Message)]
pub struct DataSourcesListProto {
    #[prost(message, repeated, tag = "1")]
    pub data_sources: Vec<DataSourceProto>,
}

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

    pub fn execute_driver_action(&self, driver_id: &str, action: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let drivers_list = self.load_configured_drivers()?;
        let driver_conf = drivers_list.drivers.iter().find(|d| d.id == driver_id)
            .ok_or_else(|| format!("Driver '{}' not found", driver_id))?;
        
        let lib_path = if !driver_conf.library_path.is_empty() {
             format!("{}/lib{}.so", driver_conf.library_path, driver_conf.name)
        } else {
             format!("{}/lib{}.so", self.config.driver_root, driver_conf.name)
        };

        unsafe {
             let lib = libloading::Library::new(&lib_path).map_err(|e| format!("Failed to load library '{}': {}", lib_path, e))?;
             
             // First init driver (stateless for action?) 
             // Ideally actions like 'discover' are stateless or static.
             // But valid FFI requires an instance context?
             // ox_driver_call_action takes context. Setup needs to initialize dummy driver.
             // We init with empty config just to get a handle.
             let init_driver: libloading::Symbol<unsafe extern "C" fn(*const libc::c_char) -> *mut libc::c_void> =
                lib.get(b"ox_driver_init").expect("Missing ox_driver_init");
             let destroy_driver: libloading::Symbol<unsafe extern "C" fn(*mut libc::c_void)> =
                lib.get(b"ox_driver_destroy").expect("Missing ox_driver_destroy");
             
             let config_json = CString::new("{}").unwrap();
             let ctx = init_driver(config_json.as_ptr());
             
             let call_action: libloading::Symbol<unsafe extern "C" fn(*mut libc::c_void, *const libc::c_char, *const libc::c_char) -> ox_persistence::OxBuffer> =
                 lib.get(b"ox_driver_call_action").map_err(|_| "Driver does not support actions (missing symbols)".to_string())?;
             
             let action_c = CString::new(action).unwrap();
             let params_json = CString::new(params.to_string()).unwrap();
             
             let buf = call_action(ctx, action_c.as_ptr(), params_json.as_ptr());
             let result_json = buf.to_string();
             ox_persistence::free_ox_buffer(buf);
             
             destroy_driver(ctx);
             
             serde_json::from_str(&result_json).map_err(|e| e.to_string())
        }
    }
    
    pub fn list_driver_datasets(&self, driver_id: &str, connection_info: &serde_json::Value) -> Result<Vec<String>, String> {
        let drivers_list = self.load_configured_drivers()?;
        let driver_conf = drivers_list.drivers.iter().find(|d| d.id == driver_id)
            .ok_or_else(|| format!("Driver '{}' not found", driver_id))?;
            
        let lib_path = if !driver_conf.library_path.is_empty() {
             format!("{}/lib{}.so", driver_conf.library_path, driver_conf.name)
        } else {
             format!("{}/lib{}.so", self.config.driver_root, driver_conf.name)
        };

        unsafe {
             let lib = libloading::Library::new(&lib_path).map_err(|e| format!("Failed to load library '{}': {}", lib_path, e))?;
             
             let init_driver: libloading::Symbol<unsafe extern "C" fn(*const libc::c_char) -> *mut libc::c_void> =
                lib.get(b"ox_driver_init").expect("Missing ox_driver_init");
             let destroy_driver: libloading::Symbol<unsafe extern "C" fn(*mut libc::c_void)> =
                lib.get(b"ox_driver_destroy").expect("Missing ox_driver_destroy");
             
             let config_json = CString::new("{}").unwrap();
             let ctx = init_driver(config_json.as_ptr());

             let list_datasets: libloading::Symbol<unsafe extern "C" fn(*mut libc::c_void, *const libc::c_char) -> ox_persistence::OxBuffer> =
                 lib.get(b"ox_driver_list_datasets").map_err(|_| "Driver does not support listing datasets".to_string())?;
                 
             let info_c = CString::new(connection_info.to_string()).unwrap();
             
             let buf = list_datasets(ctx, info_c.as_ptr());
             let result_json = buf.to_string();
             ox_persistence::free_ox_buffer(buf);
             
             destroy_driver(ctx);
             
             serde_json::from_str(&result_json).map_err(|e| format!("Error parsing driver response: {} | JSON: {}", e, result_json))
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
    api: CoreHostApi,
}

lazy_static! {
    static ref DATA_SOURCE_MANAGER_INSTANCE: Mutex<Option<Arc<DataSourceManager>>> = Mutex::new(None);
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let p = (api.get_field)(task_ctx, c_key.as_ptr());
    if p.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

fn get_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> Option<Vec<u8>> {
    let c_key = CString::new(key).unwrap();
    let mut len: usize = 0;
    let ptr = (api.get_field_bytes)(task_ctx, c_key.as_ptr(), &mut len as *mut usize);
    if ptr.is_null() || len == 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

fn set_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
    let c_key = CString::new(key).unwrap();
    (api.set_field_bytes)(task_ctx, c_key.as_ptr(), data.as_ptr(), data.len());
}

fn log_msg(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api = unsafe { *api_ptr };

    let params_str = if !plugin_config_ctx.is_null() {
        unsafe { CStr::from_ptr(plugin_config_ctx).to_string_lossy().to_string() }
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

    let ctx = Box::new(ModuleContext { manager, api });
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    if plugin_config_ctx.is_null() {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }
    let context = unsafe { &*(plugin_config_ctx as *mut ModuleContext) };
    let api = &context.api;

    // --- Conflict Management ---
    let is_modified = get_field(api, task_ctx, "flow.modified") == "true";

    if is_modified {
        let action = context.manager.config.on_content_conflict.unwrap_or(ContentConflictAction::Skip);
        log_msg(api, task_ctx, OX_LOG_INFO, &format!("Conflict check: is_modified=true, action={:?}", action));

        match action {
            ContentConflictAction::Skip => {
                return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
            },
            ContentConflictAction::Error => {
                set_field(api, task_ctx, "response.status", "500");
                set_field(api, task_ctx, "response.body", "Conflict: Flow content already modified");
                return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
            },
            _ => {}
        }
    }

    // --- Path Resolution ---
    let capture = get_field(api, task_ctx, "request.capture");
    let (path_str, using_capture) = if !capture.is_empty() {
        (capture, true)
    } else {
        let resource = get_field(api, task_ctx, "request.resource");
        let full = if !resource.is_empty() {
            resource
        } else {
            let p = get_field(api, task_ctx, "request.path");
            if !p.is_empty() { p } else { "/".to_string() }
        };
        (full, false)
    };

    let clean_path = path_str.trim_matches('/').to_string();

    let method = {
        let verb = get_field(api, task_ctx, "request.verb");
        if !verb.is_empty() {
            verb
        } else {
            let m = get_field(api, task_ctx, "request.method");
            if !m.is_empty() { m.to_lowercase() } else { "get".to_string() }
        }
    };

    let send_json_error = move |error_msg: String, status_code: i32| {
        let json_error = serde_json::json!({ "error": error_msg });
        set_field(api, task_ctx, "response.body", &json_error.to_string());
        set_field(api, task_ctx, "response.status", &status_code.to_string());
        set_field(api, task_ctx, "response.header.Content-Type", "application/json");
    };

    let send_json_success = move |body: String| {
        set_field(api, task_ctx, "response.body", &body);
        set_field(api, task_ctx, "response.status", "200");
        set_field(api, task_ctx, "response.header.Content-Type", "application/json");
    };

    // --- Routing Logic ---

    // 1. List Data Sources
    let is_root = clean_path.is_empty() || (!using_capture && clean_path == "data_sources");

    if method == "get" && is_root {
        match context.manager.load_data_sources() {
            Ok(list) => {
                // Encode domain data as protobuf and store in binary task-state field.
                let proto_list = DataSourcesListProto {
                    data_sources: list.data_sources.iter().map(|ds| DataSourceProto {
                        id: ds.id.clone(),
                        name: ds.name.clone(),
                        driver_id: ds.driver_id.clone(),
                        config_json: serde_json::to_string(&ds.config).unwrap_or_default(),
                    }).collect(),
                };
                let mut proto_bytes = Vec::new();
                if proto_list.encode(&mut proto_bytes).is_ok() {
                    set_field_bytes_data(api, task_ctx, "data.datasources_proto", &proto_bytes);
                }
                // HTTP response body stays as JSON for compatibility.
                send_json_success(serde_json::to_string(&list).unwrap_or_default());
            },
            Err(e) => send_json_error(e, 500),
        }
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    // 2. Add Data Source
    if (method == "create" || method == "post") && is_root {
        let body = {
            let payload = get_field(api, task_ctx, "request.payload");
            let body_field = get_field(api, task_ctx, "request.body");
            if !payload.is_empty() {
                payload
            } else if !body_field.is_empty() {
                body_field
            } else {
                let body_path = get_field(api, task_ctx, "request.body_path");
                if !body_path.is_empty() {
                    std::fs::read_to_string(&body_path).unwrap_or_else(|_| "{}".to_string())
                } else {
                    send_json_error("Missing body".to_string(), 400);
                    return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
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
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    // 3. Delete Data Source
    if method == "delete" {
        let id_opt: Option<&str> = if using_capture {
            if !clean_path.is_empty() { Some(&clean_path) } else { None }
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
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }
    }

    // 3.5. Execute Action
    if method == "post" && (path_str.contains("/action/") || path_str.contains("/list_datasets/")) {
        let parts: Vec<&str> = clean_path.split('/').collect();

        if parts.len() >= 3 && parts[0] == "action" {
            let driver_id = parts[1];
            let action_name = parts[2];
            let body = {
                let payload = get_field(api, task_ctx, "request.payload");
                if !payload.is_empty() { payload } else {
                    let p = get_field(api, task_ctx, "request.body_path");
                    if !p.is_empty() { std::fs::read_to_string(&p).unwrap_or("{}".to_string()) } else { "{}".to_string() }
                }
            };
            let params: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::json!({}));

            match context.manager.execute_driver_action(driver_id, action_name, &params) {
                Ok(val) => send_json_success(val.to_string()),
                Err(e) => send_json_error(e, 500),
            }
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        if parts.len() >= 2 && parts[0] == "list_datasets" {
            let driver_id = parts[1];
            let body = {
                let payload = get_field(api, task_ctx, "request.payload");
                if !payload.is_empty() { payload } else {
                    let p = get_field(api, task_ctx, "request.body_path");
                    if !p.is_empty() { std::fs::read_to_string(&p).unwrap_or("{}".to_string()) } else { "{}".to_string() }
                }
            };
            let params: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::json!({}));

            match context.manager.list_driver_datasets(driver_id, &params) {
                Ok(val) => send_json_success(serde_json::json!(val).to_string()),
                Err(e) => send_json_error(e, 500),
            }
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }
    }

    // 4. Render Creation Form
    if method == "get" && (clean_path == "new/form" || path_str.ends_with("/data_sources/new/form")) {
        let driver_id = get_field(api, task_ctx, "request.query.driver");

        let ds_id = {
            let id = get_field(api, task_ctx, "request.query.id");
            if !id.is_empty() { Some(id) } else { None }
        };

        let mut existing_ds: Option<DataSource> = None;
        let mut driver_id_to_use = driver_id.clone();

        if let Some(ref id) = ds_id {
            match context.manager.get_data_source(id) {
                Ok(Some(ds)) => {
                    driver_id_to_use = ds.driver_id.clone();
                    existing_ds = Some(ds);
                },
                Ok(None) => {
                    send_json_error(format!("Data source '{}' not found", id), 404);
                    return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
                },
                Err(e) => {
                    send_json_error(format!("Error loading data source: {}", e), 500);
                    return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
                }
            }
        }

        if driver_id_to_use.is_empty() {
            send_json_error("Missing 'driver' query parameter and no 'id' provided for lookup.".to_string(), 400);
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }

        let drivers_list = match context.manager.load_configured_drivers() {
            Ok(l) => l,
            Err(e) => {
                send_json_error(format!("Failed to load drivers config: {}", e), 500);
                return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
            }
        };

        let driver_opt = drivers_list.drivers.iter().find(|d| d.id == driver_id_to_use);
        if driver_opt.is_none() {
            send_json_error(format!("Driver '{}' not found in configuration.", driver_id_to_use), 404);
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }
        let driver_conf = driver_opt.unwrap();

        let lib_path = if !driver_conf.library_path.is_empty() {
            format!("{}/lib{}.so", driver_conf.library_path, driver_conf.name)
        } else {
            format!("{}/lib{}.so", context.manager.config.driver_root, driver_conf.name)
        };

        let schema_yaml = match context.manager.get_driver_schema(&lib_path) {
            Ok(s) => s,
            Err(e) => {
                send_json_error(format!("Failed to load schema from driver: {}", e), 500);
                return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
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

        let yaml_val: serde_yaml::Value = serde_yaml::from_str(&schema_yaml)
            .map_err(|e| format!("YAML Parse Error: {}", e))
            .unwrap_or(serde_yaml::Value::Null);

        let json_val = serde_json::to_value(yaml_val).unwrap_or(serde_json::Value::Null);
        let form_def_json = serde_json::to_string(&json_val).unwrap_or("{}".to_string());

        let render_res = ox_forms_api::render_form(&form_def_json, &serde_json::Value::Object(props.into_iter().collect()));

        match render_res {
            Ok(html) => {
                set_field(api, task_ctx, "response.body", &html);
                set_field(api, task_ctx, "response.status", "200");
                set_field(api, task_ctx, "response.header.Content-Type", "text/html");
            },
            Err(e) => {
                let err_msg = format!("Form Render Error: {}", e);
                log_msg(api, task_ctx, OX_LOG_ERROR, &err_msg);
                send_json_error(err_msg, 500);
            },
        }
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = unsafe { Box::from_raw(plugin_config_ctx as *mut ModuleContext) };
    }
}
