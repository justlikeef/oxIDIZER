use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_INFO,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};

const MODULE_NAME: &str = "ox_webservice_errorhandler_json";

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Append,
    Replace,
    Ignore,
}

fn default_on_success() -> Action { Action::Ignore }
fn default_on_error() -> Action { Action::Append }

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_on_success")]
    pub on_success: Action,
    #[serde(default = "default_on_error")]
    pub on_error: Action,
}

pub struct ModuleContext {
    config: Config,
    api: CoreHostApi,
}

/// Protobuf-compatible mirror of Config.
/// on_success/on_error are encoded as i32 (0=ignore, 1=append, 2=replace).
#[derive(prost::Message)]
pub struct ConfigProto {
    #[prost(int32, tag = "1")]
    pub on_success: i32,
    #[prost(int32, tag = "2")]
    pub on_error: i32,
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

#[allow(dead_code)]
fn get_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> Option<Vec<u8>> {
    let c_key = CString::new(key).unwrap();
    let mut len: usize = 0;
    let ptr = (api.get_field_bytes)(task_ctx, c_key.as_ptr(), &mut len as *mut usize);
    if ptr.is_null() || len == 0 { return None; }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

#[allow(dead_code)]
fn set_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
    let c_key = CString::new(key).unwrap();
    (api.set_field_bytes)(task_ctx, c_key.as_ptr(), data.as_ptr(), data.len());
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
    let config: Config = serde_json::from_value(params).unwrap_or(Config {
        on_success: Action::Ignore,
        on_error: Action::Append,
    });

    if let Ok(c) = CString::new(format!("{} initialized", MODULE_NAME)) {
        (api.log)(std::ptr::null_mut(), OX_LOG_INFO, c.as_ptr());
    }

    let ctx = Box::new(ModuleContext { config, api });
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
    let status: u16 = status_str.parse().unwrap_or(200);
    let is_error = status >= 400;

    let action = if is_error { context.config.on_error } else { context.config.on_success };
    if action == Action::Ignore {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    // If a plugin already set a JSON body, pass it through unchanged.
    let existing_ct = get_field(api, task_ctx, "response.header.Content-Type");
    let existing_body = get_field(api, task_ctx, "response.body");
    if action != Action::Append
        && existing_ct.contains("application/json")
        && !existing_body.is_empty()
    {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    let status_text = axum::http::StatusCode::from_u16(status)
        .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        .canonical_reason().unwrap_or("Unknown Error");

    let mut final_json = serde_json::Map::new();
    final_json.insert("status".to_string(), serde_json::json!(status));
    final_json.insert("message".to_string(), Value::String(status_text.to_string()));

    if action == Action::Append {
        let existing_body = get_field(api, task_ctx, "response.body");
        if let Ok(existing_val) = serde_json::from_str::<Value>(&existing_body) {
            if let Some(obj) = existing_val.as_object() {
                for (k, v) in obj { final_json.insert(k.clone(), v.clone()); }
            } else if !existing_body.is_empty() {
                final_json.insert("data".to_string(), existing_val);
            }
        } else if !existing_body.is_empty() {
            final_json.insert("content".to_string(), Value::String(existing_body));
        }
    }

    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
    let serialized = serde_json::to_string(&final_json).unwrap_or("{}".to_string());
    set_field(api, task_ctx, "response.body", &serialized);

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
        let _ = Box::from_raw(plugin_config_ctx as *mut ModuleContext);
    }
}
