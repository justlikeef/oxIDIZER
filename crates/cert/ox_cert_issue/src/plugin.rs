use std::ffi::{c_char, c_void, CStr, CString};
use std::panic;
use std::path::Path;

use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
    OX_WORKFLOW_ABI_VERSION,
};

use crate::config::CertIssueConfig;
use crate::handlers::handle_issue;

#[allow(dead_code)]
struct PluginState {
    api: CoreHostApi,
    config: CertIssueConfig,
}

unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c_msg) = CString::new(msg) {
        (api.log)(task_ctx, level, c_msg.as_ptr());
    }
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let Ok(c_key) = CString::new(key) else { return String::new() };
    let ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    if let (Ok(c_key), Ok(c_val)) = (CString::new(key), CString::new(value)) {
        (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_init(
    config_ptr: *const c_char,
    api_ptr: *const CoreHostApi,
    abi_version: u32,
) -> *mut c_void {
    if abi_version != OX_WORKFLOW_ABI_VERSION || api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api = unsafe { *api_ptr };

    let params_str = if !config_ptr.is_null() {
        unsafe { CStr::from_ptr(config_ptr).to_string_lossy().to_string() }
    } else { String::new() };
    let params: serde_json::Value = serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);
    let config_path = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_issue: missing config_file param");
            return std::ptr::null_mut();
        }
    };
    let config: CertIssueConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_issue: config error: {}", e));
                return std::ptr::null_mut();
            }
        },
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_cert_issue: failed to load config: {}", e));
            return std::ptr::null_mut();
        }
    };

    log(&api, std::ptr::null_mut(), OX_LOG_INFO,
        &format!("ox_cert_issue: initialized for tenant '{}'", config.tenant_id));
    Box::into_raw(Box::new(PluginState { api, config })) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(
    plugin_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    if plugin_ctx.is_null() { return cont; }
    let state = unsafe { &*(plugin_ctx as *mut PluginState) };

    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| dispatch(state, task_ctx)));
    match result {
        Ok(fc) => fc,
        Err(_) => {
            log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_issue: panic in handler");
            set_response(&state.api, task_ctx, 500, r#"{"error":{"code":"INTERNAL_ERROR","message":"panic"}}"#);
            cont
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_plugin_ctx: *mut c_void, _task_ctx: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)); }
    }
}

fn dispatch(state: &PluginState, task_ctx: *mut c_void) -> FlowControl {
    let api = &state.api;
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };

    let method = get_field(api, task_ctx, "request.method").to_uppercase();
    let path   = get_field(api, task_ctx, "request.path");

    // Only handle POST /api/v1/certificates
    if method != "POST" || path.trim_end_matches('/') != "/api/v1/certificates" {
        return cont;
    }

    let body          = get_field(api, task_ctx, "request.body");
    let content_type  = get_field(api, task_ctx, "request.header.Content-Type");
    let ra_approved   = get_field(api, task_ctx, "cert.ra.approved") == "true";
    let enrichment    = get_field(api, task_ctx, "cert.webhook.enrichment");
    let enrichment    = if enrichment.is_empty() { None } else { Some(enrichment.as_str()) };

    match handle_issue(&state.config, &body, &content_type, ra_approved, enrichment) {
        Ok(outcome) => {
            set_response(api, task_ctx, outcome.http_status, &outcome.body_json);
            if let Some(serial) = &outcome.serial {
                set_field(api, task_ctx, "cert.issued.serial", serial);
            }
            if let Some(not_after) = &outcome.not_after_rfc3339 {
                set_field(api, task_ctx, "cert.issued.not_after", not_after);
            }
        }
        Err(e) => {
            set_response(api, task_ctx, e.http_status, &e.to_body(&state.config.tenant_id));
        }
    }

    cont
}

fn set_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}
