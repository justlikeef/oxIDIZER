use std::ffi::{CStr, CString};
use libc::{c_char, c_void};
use serde::Serialize;


use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_INFO,
};

const MODULE_NAME: &str = "ox_webservice_ping";

#[derive(Serialize, prost::Message, Clone)]
struct PingResponse {
    #[prost(string, tag = "1")]
    response: String,
}

pub struct ModuleContext {
    api: CoreHostApi,
    #[allow(dead_code)]
    module_id: String,
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let res_ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if res_ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(res_ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

fn log_msg(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c_msg) = CString::new(msg) {
        (api.log)(task_ctx, level, c_msg.as_ptr());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    _plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api = unsafe { *api_ptr };

    let ctx = Box::new(ModuleContext {
        api,
        module_id: MODULE_NAME.to_string(),
    });

    if let Ok(c_msg) = CString::new("ox_webservice_ping initialized") {
        (api.log)(std::ptr::null_mut(), OX_LOG_INFO, c_msg.as_ptr());
    }

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

    let accept = get_field(api, task_ctx, "request.header.accept");
    let query = get_field(api, task_ctx, "request.query");

    let format = if accept.contains("application/json")
        || query.split('&').any(|p| p == "format=json")
    {
        "json"
    } else {
        "html"
    };

    let (body_content, content_type) = if format == "html" {
        ("<html><body><h1>response: pong</h1></body></html>".to_string(), "text/html")
    } else {
        let response = PingResponse {
            response: "pong".to_string(),
        };
        (serde_json::to_string(&response).unwrap_or(r#"{"response":"pong"}"#.to_string()), "application/json")
    };

    log_msg(api, task_ctx, OX_LOG_INFO, &format!("Handling ping request (format: {})", format));

    set_field(api, task_ctx, "response.body", &body_content);
    set_field(api, task_ctx, "response.status", "200");
    set_field(api, task_ctx, "response.header.Content-Type", content_type);

    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = Box::from_raw(plugin_config_ctx as *mut ModuleContext);
    }
}
