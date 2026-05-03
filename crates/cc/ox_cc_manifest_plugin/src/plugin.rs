//! ox_workflow_abi cdylib wrapper for ox_cc_manifest_plugin.
//!
//! Plugin config: path to the YAML config file (passed as plugin_config_ctx string).
//!
//! Routes:
//!   POST  /cc/manifest/{client_id}          — deploy signed envelope
//!   GET   /cc/manifest/{client_id}/latest   — client polls current envelope
//!   GET   /cc/manifest/{client_id}/history  — admin: list historical envelopes
//!   PATCH /cc/manifest/{client_id}/expire   — admin: expire current envelope
//!   GET   /cc/clients                       — admin: list all clients
//!   GET   /cc/clients/{client_id}/status    — admin: client status

#![cfg_attr(test, allow(unused_imports, dead_code))]

use std::ffi::{c_void, CStr, CString};
#[cfg(not(test))]
use std::ffi::c_char;
#[cfg(not(test))]
use std::panic;

use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
};
#[cfg(not(test))]
use ox_workflow_abi::OX_WORKFLOW_ABI_VERSION;

use crate::config::ManifestPluginConfig;
use crate::db::ManifestDb;
use crate::handlers;

#[cfg(not(test))]
struct PluginState {
    api: CoreHostApi,
    config: ManifestPluginConfig,
}

#[cfg(not(test))]
unsafe impl Send for PluginState {}
#[cfg(not(test))]
unsafe impl Sync for PluginState {}

#[cfg(not(test))]
fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let Ok(c_key) = CString::new(key) else { return String::new() };
    let ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

#[cfg(not(test))]
fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    if let (Ok(c_key), Ok(c_val)) = (CString::new(key), CString::new(value)) {
        (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
    }
}

#[cfg(not(test))]
#[allow(dead_code)]
fn get_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> Option<Vec<u8>> {
    let c_key = CString::new(key).unwrap();
    let mut len: usize = 0;
    let ptr = (api.get_field_bytes)(task_ctx, c_key.as_ptr(), &mut len as *mut usize);
    if ptr.is_null() || len == 0 { return None; }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

#[cfg(not(test))]
#[allow(dead_code)]
fn set_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
    let c_key = CString::new(key).unwrap();
    (api.set_field_bytes)(task_ctx, c_key.as_ptr(), data.as_ptr(), data.len());
}

#[cfg(not(test))]
fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c_msg) = CString::new(msg) {
        (api.log)(task_ctx, level, c_msg.as_ptr());
    }
}

#[cfg(not(test))]
fn respond(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

#[cfg(not(test))]
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

    let config_path = if config_ptr.is_null() {
        log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cc_manifest_plugin: missing config");
        return std::ptr::null_mut();
    } else {
        let raw = unsafe { CStr::from_ptr(config_ptr).to_string_lossy() };
        // Try to parse as JSON module config and extract config_file
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(path) = v.get("config_file").and_then(|v| v.as_str()) {
                path.to_string()
            } else {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    "ox_cc_manifest_plugin: missing config_file in module config");
                return std::ptr::null_mut();
            }
        } else {
            // Not JSON — treat the raw string as a direct file path (tests / CLI)
            raw.into_owned()
        }
    };

    let config = match ManifestPluginConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_cc_manifest_plugin: config error: {}", e));
            return std::ptr::null_mut();
        }
    };

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, "ox_cc_manifest_plugin: initialized");
    Box::into_raw(Box::new(PluginState { api, config })) as *mut c_void
}

#[cfg(not(test))]
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
            log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cc_manifest_plugin: panic in handler");
            respond(&state.api, task_ctx, 500, r#"{"error":"internal error"}"#);
            cont
        }
    }
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_plugin_ctx: *mut c_void, _task_ctx: *mut c_void) {}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)); }
    }
}

#[cfg(not(test))]
fn dispatch(state: &PluginState, task_ctx: *mut c_void) -> FlowControl {
    let api = &state.api;
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };

    let method = get_field(api, task_ctx, "request.method").to_uppercase();
    let path = get_field(api, task_ctx, "request.path");
    let body = get_field(api, task_ctx, "request.body");

    let db = match ManifestDb::open(&state.config.db_path, &state.config.db_encryption_key) {
        Ok(d) => d,
        Err(e) => {
            log(api, task_ctx, OX_LOG_ERROR,
                &format!("ox_cc_manifest_plugin: db error: {}", e));
            respond(api, task_ctx, 503, r#"{"error":"service unavailable"}"#);
            return cont;
        }
    };

    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    let response = match (method.as_str(), segs.as_slice()) {
        ("POST", ["cc", "manifest", client_id]) => {
            handlers::deploy_envelope(&db, client_id, &body)
        }
        ("POST", ["cc", "bootstrap"]) => {
            handlers::bootstrap_checkin(&db, &state.config, &body)
        }
        ("GET", ["cc", "manifest", client_id, "latest"]) => {
            handlers::get_latest(&db, client_id)
        }
        ("GET", ["cc", "manifest", client_id, "history"]) => {
            handlers::get_history(&db, client_id)
        }
        ("PATCH", ["cc", "manifest", client_id, "expire"]) => {
            handlers::expire_manifest(&db, client_id)
        }
        ("GET", ["cc", "clients"]) => handlers::list_clients(&db),
        ("GET", ["cc", "clients", "pending"]) => handlers::list_pending_clients(&db),
        ("POST", ["cc", "clients", client_id, "trust"]) => {
            handlers::trust_client(&db, client_id)
        }
        ("GET", ["cc", "clients", client_id, "status"]) => {
            handlers::get_client_status(&db, client_id)
        }
        _ => {
            log(api, task_ctx, OX_LOG_INFO,
                &format!("ox_cc_manifest_plugin: no route for {} {}", method, path));
            return cont;
        }
    };

    respond(api, task_ctx, response.status, &response.body);
    cont
}
