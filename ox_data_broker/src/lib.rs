use ox_webservice_api::{ModuleEndpoint, ModuleEndpoints, WebServiceHandler, InitializationData, SendableWebServiceHandler};
use ox_persistence::{get_registered_drivers, DriverMetadata};
use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use serde_json;

// Handler to return a list of registered drivers (FFI compatible)
#[no_mangle]
pub extern "C" fn list_drivers_ffi_handler(_request_ptr: *mut c_char) -> *mut c_char {
    let drivers = get_registered_drivers();
    let json_string = serde_json::to_string(&drivers).expect("Failed to serialize DriverMetadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

// This function will be called by ox_webservice to initialize the module
#[no_mangle]
pub extern "C" fn initialize_module(init_data_ptr: *mut c_char) -> *mut c_void {
    let init_data_str = unsafe { CStr::from_ptr(init_data_ptr).to_str().unwrap() };
    let init_data: InitializationData = serde_json::from_str(init_data_str).unwrap();

    if let Some(config_file) = init_data.context.module_config.as_ref().and_then(|mc| mc.params.as_ref()).and_then(|p| p.get("config_file")).and_then(|v| v.as_str()) {
        println!("ox_data_broker: Received config file path: {}", config_file);
        // Here you would typically load and use the configuration file
    }

    let endpoints = vec![
        ModuleEndpoint {
            path: "drivers/list".to_string(),
            handler: SendableWebServiceHandler(list_drivers_ffi_handler as WebServiceHandler),
            priority: 0,
        },
    ];
    let boxed_endpoints = Box::new(ModuleEndpoints { endpoints });
    Box::into_raw(boxed_endpoints) as *mut c_void
}