use axum::Json;
use ox_persistence::{get_registered_drivers, DriverMetadata};
use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use serde_json;

// Define the C-compatible function signature for handlers
pub type WebServiceHandler = unsafe extern "C" fn(*mut c_char) -> *mut c_char;

// This struct will be returned by dynamically loaded modules
// It contains a list of endpoints, each with a URL path and a raw function pointer to its handler
pub struct ModuleEndpoints {
    pub endpoints: Vec<(String, WebServiceHandler)>,
}

// Handler to return a list of registered drivers (FFI compatible)
#[no_mangle]
pub extern "C" fn list_drivers_ffi_handler(_request_ptr: *mut c_char) -> *mut c_char {
    let drivers = get_registered_drivers();
    let json_string = serde_json::to_string(&drivers).expect("Failed to serialize DriverMetadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

// This function will be called by ox_webservice to initialize the module
#[no_mangle]
pub extern "C" fn initialize_module() -> *mut c_void {
    let endpoints = vec![
        ("drivers/list".to_string(), list_drivers_ffi_handler as WebServiceHandler),
    ];
    let boxed_endpoints = Box::new(ModuleEndpoints { endpoints });
    Box::into_raw(boxed_endpoints) as *mut c_void
}