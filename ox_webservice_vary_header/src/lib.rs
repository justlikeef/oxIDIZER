use libc::{c_void, c_char};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_INFO, OX_LOG_DEBUG,
};
use std::ffi::{CStr, CString};

const MODULE_NAME: &str = "ox_webservice_vary_header";

pub struct ModuleContext {
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
    _plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };
    if let Ok(c) = CString::new(format!("{} initialized", MODULE_NAME)) {
        (api.log)(std::ptr::null_mut(), OX_LOG_INFO, c.as_ptr());
    }
    let ctx = Box::new(ModuleContext { api });
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

    // Get current Vary header value if any
    let current_vary = get_field(api, task_ctx, "response.header.Vary");
    
    if !current_vary.to_lowercase().contains("accept") {
        let new_vary = if current_vary.is_empty() {
            "Accept".to_string()
        } else {
            format!("{}, Accept", current_vary)
        };
        set_field(api, task_ctx, "response.header.Vary", &new_vary);
        log(api, task_ctx, OX_LOG_DEBUG, "Added 'Vary: Accept' header to response");
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
        let _ = Box::from_raw(plugin_config_ctx as *mut ModuleContext);
    }
}
