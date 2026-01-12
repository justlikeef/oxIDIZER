use ox_webservice_api::{
    PipelineState, FlowControl, ModuleStatus, ReturnParameters, HandlerResult,
    LogLevel, AllocStrFn, LogCallback
};
use std::ffi::{c_void, CString};
use std::os::raw::c_char;
use axum::http::header;

#[no_mangle]
pub unsafe extern "C" fn initialize_module(
    _params_json: *const c_char,
    _module_id: *const c_char,
    _api: *const ox_webservice_api::CoreHostApi
) -> *mut ox_webservice_api::ModuleInterface {
    let interface = Box::new(ox_webservice_api::ModuleInterface {
        instance_ptr: std::ptr::null_mut(),
        handler_fn: process_request,
        log_callback: dummy_log_callback,
        get_config: dummy_get_config,
    });
    Box::into_raw(interface)
}

#[no_mangle]
pub unsafe extern "C" fn dummy_log_callback(
    _level: LogLevel, 
    _module: *const c_char, 
    _message: *const c_char
) {
    // No-op
}

#[no_mangle]
pub unsafe extern "C" fn dummy_get_config(
    _state: *mut c_void, 
    _arena: *const c_void, 
    _alloc_fn: AllocStrFn
) -> *mut c_char {
    std::ptr::null_mut()
}

fn log(cb: LogCallback, level: LogLevel, msg: &str) {
    if let (Ok(c_mod), Ok(c_msg)) = (CString::new("vary_header"), CString::new(msg)) {
        unsafe { cb(level, c_mod.as_ptr(), c_msg.as_ptr()) };
    }
}

#[no_mangle]
pub extern "C" fn process_request(
    _instance_ptr: *mut c_void, 
    pipeline_state_ptr: *mut PipelineState, 
    log_callback: LogCallback,
    _alloc_fn: ox_webservice_api::AllocFn,
    _arena_ptr: *const c_void
) -> HandlerResult {
    let mut status = ModuleStatus::Unmodified;

    if pipeline_state_ptr.is_null() {
        return HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
        };
    }

    let pipeline_state = unsafe { &mut *pipeline_state_ptr };

    log(log_callback, LogLevel::Debug, "ox_webservice_vary_header: Processing request (Unconditional)");

    let has_accept_vary = pipeline_state.response_headers.get_all(header::VARY).iter().any(|val| {
        if let Ok(v_str) = val.to_str() {
            v_str.to_lowercase().contains("accept")
        } else {
            false
        }
    });

    if !has_accept_vary {
        if let Ok(val) = axum::http::HeaderValue::from_str("Accept") {
            pipeline_state.response_headers.append(header::VARY, val);
            status = ModuleStatus::Modified;
            log(log_callback, LogLevel::Debug, "ox_webservice_vary_header: Added 'Vary: Accept' header to response");
        }
    }



    HandlerResult {
        status,
        flow_control: FlowControl::Continue,
        return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
    }
}
