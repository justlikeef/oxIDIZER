//! ox_workflow_abi cdylib wrapper for ox_cc_admin_plugin.
//!
//! Plugin config: path to the YAML config file (passed as plugin_config_ctx string).
//!
//! Routes (all under /admin/api/):
//!   GET    /admin/api/clients
//!   POST   /admin/api/templates
//!   GET    /admin/api/templates
//!   GET    /admin/api/templates/{template_id}
//!   GET    /admin/api/pending
//!   GET    /admin/api/pending/{template_id}
//!   POST   /admin/api/pending/{template_id}/approve
//!   POST   /admin/api/pending/{template_id}/reject
//!   GET    /admin/api/approved
//!   POST   /admin/api/approved/{template_id}/deploy
//!   GET    /admin/api/audit
//!   GET    /admin/api/manifest-clients
//!   GET    /admin/api/manifest-clients/{client_id}/status
//!   GET    /admin/api/manifest-clients/{client_id}/history
//!   GET    /admin/api/reports/{client_id}
//!   PATCH  /admin/api/manifest-clients/{client_id}/expire
//!   POST   /admin/api/sessions
//!   GET    /admin/api/sessions
//!   GET    /admin/api/sessions/pending
//!   POST   /admin/api/sessions/{id}/approve
//!   POST   /admin/api/sessions/{id}/reject
//!   DELETE /admin/api/sessions/{id}
//!   POST   /admin/api/sessions/{id}/submit

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

use crate::config::AdminPluginConfig;
use crate::db::AdminDb;
use crate::handlers;
use crate::http_client::AdminHttpClient;

#[cfg(not(test))]
struct PluginState {
    api: CoreHostApi,
    config: AdminPluginConfig,
    http_client: AdminHttpClient,
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
        log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cc_admin_plugin: missing config");
        return std::ptr::null_mut();
    } else {
        let raw = unsafe { CStr::from_ptr(config_ptr).to_string_lossy() };
        // Try to parse as JSON module config and extract config_file
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(path) = v.get("config_file").and_then(|v| v.as_str()) {
                path.to_string()
            } else {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    "ox_cc_admin_plugin: missing config_file in module config");
                return std::ptr::null_mut();
            }
        } else {
            // Not JSON — treat the raw string as a direct file path (tests / CLI)
            raw.into_owned()
        }
    };

    let config = match AdminPluginConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_cc_admin_plugin: config error: {}", e));
            return std::ptr::null_mut();
        }
    };

    let http_client = match AdminHttpClient::new(&config) {
        Ok(c) => c,
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_cc_admin_plugin: http client init error: {}", e));
            return std::ptr::null_mut();
        }
    };

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, "ox_cc_admin_plugin: initialized");
    Box::into_raw(Box::new(PluginState { api, config, http_client })) as *mut c_void
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
            log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cc_admin_plugin: panic in handler");
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

    let db = match AdminDb::open(&state.config.db_path, &state.config.db_encryption_key) {
        Ok(d) => d,
        Err(e) => {
            log(api, task_ctx, OX_LOG_ERROR,
                &format!("ox_cc_admin_plugin: db error: {}", e));
            respond(api, task_ctx, 503, r#"{"error":"service unavailable"}"#);
            return cont;
        }
    };

    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let client = &state.http_client as &dyn crate::http_client::HttpClient;

    let response = match (method.as_str(), segs.as_slice()) {
        ("GET", ["admin", "api", "clients"]) => handlers::list_clients(client, &state.config),
        ("POST", ["admin", "api", "templates"]) => {
            handlers::create_template(&db, client, &state.config, &body)
        }
        ("GET", ["admin", "api", "templates"]) => handlers::list_templates(&db),
        ("GET", ["admin", "api", "templates", template_id]) => {
            handlers::get_template(&db, template_id)
        }
        ("GET", ["admin", "api", "pending"]) => handlers::list_pending(client, &state.config),
        ("GET", ["admin", "api", "pending", template_id]) => {
            handlers::get_pending(client, &state.config, template_id)
        }
        ("POST", ["admin", "api", "pending", template_id, "approve"]) => {
            handlers::approve(client, &state.config, template_id, &body)
        }
        ("POST", ["admin", "api", "pending", template_id, "reject"]) => {
            handlers::reject(client, &state.config, template_id, &body)
        }
        ("GET", ["admin", "api", "approved"]) => handlers::list_approved(client, &state.config),
        ("POST", ["admin", "api", "approved", template_id, "deploy"]) => {
            handlers::deploy(&db, client, &state.config, template_id)
        }
        ("GET", ["admin", "api", "audit"]) => handlers::query_audit(client, &state.config),
        ("GET", ["admin", "api", "manifest-clients"]) => {
            handlers::manifest_clients(client, &state.config)
        }
        ("GET", ["admin", "api", "manifest-clients", client_id, "status"]) => {
            handlers::manifest_client_status(client, &state.config, client_id)
        }
        ("GET", ["admin", "api", "manifest-clients", client_id, "history"]) => {
            handlers::manifest_client_history(client, &state.config, client_id)
        }
        ("GET", ["admin", "api", "reports", client_id]) => {
            handlers::manifest_reports(client, &state.config, client_id)
        }
        ("PATCH", ["admin", "api", "manifest-clients", client_id, "expire"]) => {
            handlers::manifest_expire(client, &state.config, client_id)
        }
        ("POST", ["admin", "api", "sessions"]) => {
            handlers::open_session(&db, client, &state.config, &body)
        }
        ("GET", ["admin", "api", "sessions"]) => handlers::list_sessions(&db),
        ("GET", ["admin", "api", "sessions", "pending"]) => {
            handlers::list_pending_admin_sessions(&db)
        }
        ("POST", ["admin", "api", "sessions", session_id, "approve"]) => {
            handlers::approve_session(&db, client, &state.config, session_id, &body)
        }
        ("POST", ["admin", "api", "sessions", session_id, "reject"]) => {
            handlers::reject_session(&db, client, &state.config, session_id, &body)
        }
        ("DELETE", ["admin", "api", "sessions", session_id]) => {
            handlers::close_session(&db, client, &state.config, session_id)
        }
        ("POST", ["admin", "api", "sessions", session_id, "submit"]) => {
            handlers::submit_session_manifest(&db, client, &state.config, session_id, &body)
        }
        _ => {
            log(api, task_ctx, OX_LOG_INFO,
                &format!("ox_cc_admin_plugin: no route for {} {}", method, path));
            return cont;
        }
    };

    respond(api, task_ctx, response.status, &response.body);
    cont
}
