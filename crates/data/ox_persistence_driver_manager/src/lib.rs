use std::path::Path;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use std::sync::Arc;

use std::ffi::{c_char, c_void, CStr, CString};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_INFO, OX_LOG_ERROR, OX_LOG_DEBUG,
};
use serde_json::Value;
use ox_fileproc::{process_file, RawFile};
use ox_persistence::DriversList;
use ox_data_error::OxDataError;

const MODULE_NAME: &str = "ox_persistence_driver_manager";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[allow(non_camel_case_types)]
pub enum ContentConflictAction { overwrite, append, skip, error }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DriverManagerConfig {
    pub drivers_file: String,
    pub driver_root: String,
    #[serde(default)]
    pub on_content_conflict: Option<ContentConflictAction>,
}

/// Proto-compatible mirror of DriverManagerConfig (conflict action serialized as string)
#[derive(prost::Message, Clone)]
pub struct DriverManagerConfigProto {
    #[prost(string, tag = "1")]
    pub drivers_file: String,
    #[prost(string, tag = "2")]
    pub driver_root: String,
    #[prost(string, optional, tag = "3")]
    pub on_content_conflict: Option<String>,
}

pub struct DriverManager {
    config: DriverManagerConfig,
}

impl DriverManager {
    pub fn list_available_driver_files(&self) -> Result<Vec<String>, OxDataError> {
        let root = Path::new(&self.config.driver_root);
        if !root.exists() { return Err(OxDataError::InternalError(format!("Driver root does not exist: {}", self.config.driver_root))); }
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

    pub fn load_configured_drivers(&self) -> Result<DriversList, OxDataError> {
        let path = Path::new(&self.config.drivers_file);
        if !path.exists() { return Ok(DriversList { drivers: Vec::new() }); }
        let val = process_file(path, 5).map_err(|e| OxDataError::InternalError(e.to_string()))?;
        serde_json::from_value(val).map_err(|e| OxDataError::InternalError(e.to_string()))
    }

    pub fn toggle_driver_status(&self, id: &str) -> Result<String, OxDataError> {
        let mut raw = RawFile::open(&self.config.drivers_file).map_err(|e| OxDataError::InternalError(e.to_string()))?;
        let query = format!("drivers[id=\"{}\"]/state", id);
        let span_and_val = raw.find(&query).next().map(|c| {
            let val = c.value().trim();
            (c.span.clone(), val.starts_with('"'), val.trim_matches('"').to_string())
        });

        if let Some((span, is_quoted, current_val)) = span_and_val {
            let new_val = if current_val == "enabled" { "disabled" } else { "enabled" };
            let replacement = if is_quoted { format!("\"{}\"", new_val) } else { new_val.to_string() };
            raw.update(span, &replacement);
            raw.save().map_err(|e| OxDataError::InternalError(e.to_string()))?;
            Ok(new_val.to_string())
        } else {
            Err(OxDataError::InternalError(format!("Driver '{}' not found", id)))
        }
    }

    pub fn get_driver_metadata(&self, library_path: &str) -> Result<String, OxDataError> {
        unsafe {
            let lib = libloading::Library::new(library_path).map_err(|e| OxDataError::DriverError(e.to_string()))?;
            let get_meta: libloading::Symbol<unsafe extern "C" fn() -> *mut libc::c_char> =
                lib.get(b"ox_driver_get_driver_metadata").map_err(|_| OxDataError::DriverError("Missing symbol".to_string()))?;
            let ptr = get_meta();
            if ptr.is_null() { return Err(OxDataError::DriverError("null metadata".to_string())); }
            Ok(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        }
    }

    pub fn get_driver_schema(&self, library_path: &str) -> Result<String, OxDataError> {
        unsafe {
            let lib = libloading::Library::new(library_path).map_err(|e| OxDataError::DriverError(e.to_string()))?;
            let get_schema: libloading::Symbol<unsafe extern "C" fn() -> *mut libc::c_char> =
                lib.get(b"ox_driver_get_config_schema").map_err(|_| OxDataError::DriverError("Missing symbol".to_string()))?;
            let ptr = get_schema();
            if ptr.is_null() { return Err(OxDataError::DriverError("null schema".to_string())); }
            Ok(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        }
    }
}

pub struct ModuleContext {
    manager: Arc<DriverManager>,
    api: CoreHostApi,
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

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

fn get_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> Option<Vec<u8>> {
    let c_key = CString::new(key).unwrap();
    let mut len: usize = 0;
    let ptr = (api.get_field_bytes)(task_ctx, c_key.as_ptr(), &mut len as *mut usize);
    if ptr.is_null() || len == 0 { return None; }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

fn set_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
    let c_key = CString::new(key).unwrap();
    (api.set_field_bytes)(task_ctx, c_key.as_ptr(), data.as_ptr(), data.len());
}

fn json_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };

    let params_str = if !plugin_config_ctx.is_null() {
        unsafe { CStr::from_ptr(plugin_config_ctx).to_string_lossy().to_string() }
    } else { "{}".to_string() };

    let params: Value = serde_json::from_str(&params_str).unwrap_or(Value::Null);
    let config = DriverManagerConfig {
        drivers_file: params.get("drivers_file").and_then(|v| v.as_str()).unwrap_or("conf/drivers.yaml").to_string(),
        driver_root: params.get("driver_root").and_then(|v| v.as_str()).unwrap_or("conf/drivers").to_string(),
        on_content_conflict: None,
    };

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, &format!("{} initialized", MODULE_NAME));

    let ctx = Box::new(ModuleContext {
        manager: Arc::new(DriverManager { config }),
        api,
    });
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
    let manager = &context.manager;

    let method = get_field(api, task_ctx, "request.method").to_lowercase();
    let path = { let p1 = get_field(api, task_ctx, "request.path"); if p1.is_empty() { get_field(api, task_ctx, "request.resource") } else { p1 } };

    log(api, task_ctx, OX_LOG_DEBUG, &format!("DriverManager: {} {}", method, path));

    if method == "post" || method == "create" {
        // Toggle driver status
        let trimmed = path.trim_end_matches('/');
        let id = trimmed.split('/').last().unwrap_or("");
        if !id.is_empty() && id != "available" {
            match manager.toggle_driver_status(id) {
                Ok(new_status) => {
                    let body = serde_json::json!({"status": new_status, "id": id}).to_string();
                    json_response(api, task_ctx, 200, &body);
                }
                Err(e) => {
                    log(api, task_ctx, OX_LOG_ERROR, &format!("Toggle failed: {}", e));
                    json_response(api, task_ctx, 500, &serde_json::json!({"error": e.to_string()}).to_string());
                }
            }
        } else {
            json_response(api, task_ctx, 400, &serde_json::json!({"error": "Invalid driver ID"}).to_string());
        }
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    if method == "get" && path.ends_with("/available") {
        match manager.list_available_driver_files() {
            Ok(files) => json_response(api, task_ctx, 200, &serde_json::to_string(&files).unwrap_or_default()),
            Err(e) => json_response(api, task_ctx, 500, &serde_json::json!({"error": e.to_string()}).to_string()),
        }
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    if method == "get" && path.ends_with("/schema") {
        let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
        if segments.len() >= 2 {
            let id = segments[segments.len() - 2];
            match manager.load_configured_drivers() {
                Ok(list) => {
                    if let Some(driver) = list.drivers.iter().find(|d| d.id == id) {
                        let lib_path = format!("{}/lib{}.so", if driver.library_path.is_empty() { &manager.config.driver_root } else { &driver.library_path }, driver.name);
                        match manager.get_driver_schema(&lib_path) {
                            Ok(schema) => json_response(api, task_ctx, 200, &schema),
                            Err(e) => json_response(api, task_ctx, 500, &serde_json::json!({"error": e.to_string()}).to_string()),
                        }
                    } else {
                        json_response(api, task_ctx, 404, &serde_json::json!({"error": "Driver not found"}).to_string());
                    }
                }
                Err(e) => json_response(api, task_ctx, 500, &serde_json::json!({"error": e.to_string()}).to_string()),
            }
        }
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    if method == "get" {
        match manager.load_configured_drivers() {
            Ok(list) => {
                let query_str = get_field(api, task_ctx, "request.query");
                let state_filter: Option<String> = query_str.split('&')
                    .find(|p| p.starts_with("state="))
                    .map(|p| p["state=".len()..].to_string());
                let drivers = if let Some(ref state) = state_filter {
                    list.drivers.into_iter().filter(|d| &d.state == state).collect()
                } else {
                    list.drivers
                };
                let json = serde_json::json!({"drivers": drivers}).to_string();
                json_response(api, task_ctx, 200, &json);
            }
            Err(e) => json_response(api, task_ctx, 500, &serde_json::json!({"error": e.to_string()}).to_string()),
        }
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    json_response(api, task_ctx, 405, &serde_json::json!({"error": format!("Method {} not allowed", method)}).to_string());
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
