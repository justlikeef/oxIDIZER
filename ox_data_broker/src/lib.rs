use ox_webservice_api::{WebServiceApiV1, ModuleInterface, PipelineState, HandlerResult, LogCallback, AllocFn, LogLevel};
use ox_persistence::{get_registered_drivers};
use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use serde_json;

// Handler to return a list of registered drivers (FFI compatible)
#[no_mangle]
pub unsafe extern "C" fn list_drivers_ffi_handler(_instance_ptr: *mut c_void, pipeline_state_ptr: *mut PipelineState, log_callback: LogCallback, _alloc_fn: AllocFn, _arena: *const c_void) -> HandlerResult {
    let drivers = get_registered_drivers();
    let json_string = serde_json::to_string(&drivers).expect("Failed to serialize DriverMetadata");
    
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.response_body = json_string.into_bytes();
    pipeline_state.status_code = 200;

    let module = CString::new("ox_data_broker").unwrap();
    let message = CString::new("Successfully listed drivers").unwrap();
    log_callback(LogLevel::Info, module.as_ptr(), message.as_ptr());

    HandlerResult::ModifiedContinue
}

// This function will be called by ox_webservice to initialize the module
#[no_mangle]
pub unsafe extern "C" fn initialize_module(_module_params_json_ptr: *const c_char, api: *const WebServiceApiV1) -> *mut ModuleInterface {
    let module_interface = Box::new(ModuleInterface {
        instance_ptr: std::ptr::null_mut(),
        handler_fn: list_drivers_ffi_handler,
        log_callback: (*api).log_callback,
    });
    Box::into_raw(module_interface)
}