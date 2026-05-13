use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_DEBUG, OX_LOG_ERROR, OX_LOG_INFO, OX_LOG_WARN
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use tera::{Context, Tera};

const MODULE_NAME: &str = "ox_webservice_errorhandler_jinja2";

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct ErrorHandlerConfig {
    pub content_root: PathBuf,
    pub debug_force_status: Option<u16>,
}

pub struct ModuleContext {
    content_root: PathBuf,
    debug_force_status: Option<u16>,
    api: CoreHostApi,
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let res_ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if res_ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(res_ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
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

    let config_file = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "Missing config_file parameter");
            return std::ptr::null_mut();
        }
    };

    let config: ErrorHandlerConfig = match ox_fileproc::process_file(&PathBuf::from(&config_file), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to deserialize ErrorHandlerConfig: {}", e)); return std::ptr::null_mut(); }
        },
        Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to process config file: {}", e)); return std::ptr::null_mut(); }
    };

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, &format!("{} initialized with content_root: {:?}", MODULE_NAME, config.content_root));

    let ctx = Box::new(ModuleContext {
        content_root: config.content_root,
        debug_force_status: config.debug_force_status,
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

    let status_str = get_field(api, task_ctx, "response.status");
    let mut status_code: u16 = status_str.parse().unwrap_or(200);

    if let Some(forced) = context.debug_force_status {
        status_code = forced;
        set_field(api, task_ctx, "response.status", &forced.to_string());
    }

    if status_code < 400 {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    log(api, task_ctx, OX_LOG_DEBUG, &format!("Handling error with status: {}", status_code));

    let mut context_map = serde_json::Map::new();

    let method = get_field(api, task_ctx, "request.method");
    if !method.is_empty() { context_map.insert("request_method".to_string(), Value::String(method)); }

    let path = get_field(api, task_ctx, "request.path");
    if !path.is_empty() { context_map.insert("request_path".to_string(), Value::String(path)); }

    let query = get_field(api, task_ctx, "request.query");
    if !query.is_empty() { context_map.insert("request_query".to_string(), Value::String(query)); }

    let source_ip = get_field(api, task_ctx, "request.source_ip");
    if !source_ip.is_empty() { context_map.insert("source_ip".to_string(), Value::String(source_ip)); }

    let status_text = axum::http::StatusCode::from_u16(status_code)
        .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        .canonical_reason().unwrap_or("Unknown Error");
    context_map.insert("status_code".to_string(), serde_json::json!(status_code));
    context_map.insert("status_text".to_string(), Value::String(status_text.to_string()));
    context_map.insert("message".to_string(), Value::String("An error occurred.".to_string()));
    context_map.insert("module_name".to_string(), Value::String("Unknown".to_string()));
    context_map.insert("module_context".to_string(), serde_json::Value::Object(serde_json::Map::new()));

    let render_context = Context::from_value(Value::Object(context_map)).unwrap_or_else(|_| Context::new());

    let status_template_path = context.content_root.join(format!("{}.jinja2", status_code));
    let index_template_path = context.content_root.join("index.jinja2");

    let template_to_use = if status_template_path.exists() {
        Some(status_template_path)
    } else if index_template_path.exists() {
        Some(index_template_path)
    } else { None };

    let response_body = match template_to_use {
        Some(path) => {
            log(api, task_ctx, OX_LOG_DEBUG, &format!("Rendering error template: {:?}", path));
            match std::fs::read_to_string(&path) {
                Ok(tmpl) => Tera::one_off(&tmpl, &render_context, false).unwrap_or_else(|e| {
                    log(api, task_ctx, OX_LOG_ERROR, &format!("Template render error: {}", e));
                    "500 Internal Server Error".to_string()
                }),
                Err(e) => {
                    log(api, task_ctx, OX_LOG_ERROR, &format!("Failed to read template: {}", e));
                    "500 Internal Server Error".to_string()
                }
            }
        }
        None => {
            log(api, task_ctx, OX_LOG_WARN, &format!("No specific error template found for status {}", status_code));
            format!("{} {}", status_code, status_text)
        }
    };

    let accept = get_field(api, task_ctx, "request.header.Accept");
    let existing_ct = get_field(api, task_ctx, "response.header.Content-Type");
    let existing_body = get_field(api, task_ctx, "response.body");

    // If a plugin already set a JSON body, pass it through unchanged — don't overwrite
    // a structured API error response with a rendered template.
    if existing_ct.contains("application/json") && !existing_body.is_empty() {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    let serve_json = accept.contains("application/json") || existing_ct.contains("application/json");

    if serve_json {
        let mut json_response = serde_json::Map::new();
        json_response.insert("status".to_string(), serde_json::json!(status_code));
        json_response.insert("status_text".to_string(), Value::String(status_text.to_string()));
        json_response.insert("message".to_string(), Value::String(response_body));
        let serialized = serde_json::to_string(&json_response).unwrap_or("{}".to_string());
        set_field(api, task_ctx, "response.header.Content-Type", "application/json");
        set_field(api, task_ctx, "response.body", &serialized);
    } else {
        set_field(api, task_ctx, "response.header.Content-Type", "text/html");
        set_field(api, task_ctx, "response.body", &response_body);
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