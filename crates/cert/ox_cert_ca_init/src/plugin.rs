use std::ffi::{c_void, CStr, CString};
use std::ffi::c_char;
use std::panic;
use std::path::Path;

use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
    OX_WORKFLOW_ABI_VERSION,
};

use crate::config::CaInitConfig;

#[allow(dead_code)]
struct PluginState {
    api: CoreHostApi,
}

unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c_msg) = CString::new(msg) {
        (api.log)(task_ctx, level, c_msg.as_ptr());
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
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_ca_init: missing config_file param");
            return std::ptr::null_mut();
        }
    };
    let config: CaInitConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_ca_init: config parse error: {}", e));
                return std::ptr::null_mut();
            }
        },
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_cert_ca_init: failed to load config: {}", e));
            return std::ptr::null_mut();
        }
    };

    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| crate::init::run(&config)));
    match result {
        Ok(Ok(())) => {
            log(&api, std::ptr::null_mut(), OX_LOG_INFO,
                &format!("ox_cert_ca_init: CA hierarchy ready for tenant '{}'", config.tenant_id));
            Box::into_raw(Box::new(PluginState { api })) as *mut c_void
        }
        Ok(Err(e)) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_cert_ca_init: init failed: {}", e));
            std::ptr::null_mut()
        }
        Err(_) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                "ox_cert_ca_init: panic during initialization");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(
    _plugin_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) -> FlowControl {
    // PreEarlyRequest init-only module — no request handling.
    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_plugin_ctx: *mut c_void, _task_ctx: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)); }
    }
}
