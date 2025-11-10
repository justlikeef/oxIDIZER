use axum::{
    http::StatusCode,
};
use std::ffi::{CStr, CString, c_char, c_void};
use sysinfo::{System};
use log::{info, debug};

use ox_webservice::{ModuleEndpoints, ModuleEndpoint, SendableWebServiceHandler, InitializationData};

#[no_mangle]
pub extern "C" fn initialize_module(init_data_ptr: *mut c_char) -> *mut c_void {
    let init_data_str = unsafe { CStr::from_ptr(init_data_ptr).to_str().unwrap() };
    let _init_data: InitializationData = serde_json::from_str(init_data_str).unwrap();

    info!("ox_server_info: Initializing module...");

    let mut endpoints = Vec::new();

    // The user wants to register /info/
    endpoints.push(ModuleEndpoint {
        path: "info/".to_string(),
        handler: SendableWebServiceHandler(server_info_handler_internal),
        priority: 10, // Default priority
    });

    let boxed_endpoints = Box::new(ModuleEndpoints { endpoints });
    Box::into_raw(boxed_endpoints) as *mut c_void
}

extern "C" fn server_info_handler_internal(request_ptr: *mut c_char) -> *mut c_char {
    debug!("server_info_handler_internal called");
    let request_str = unsafe { CStr::from_ptr(request_ptr).to_str().unwrap() };
    let request: serde_json::Value = serde_json::from_str(request_str).unwrap();
    let path = request.get("path").and_then(|v| v.as_str()).unwrap_or("");

    let mut sys = System::new_all();
    sys.refresh_all();

    let total_memory_gb = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
    let available_memory_gb = sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0;

    let total_disk_gb = 0.0;
    let available_disk_gb = 0.0;

    let cpu_usage = 0.0;

    let response_json = serde_json::json!({
        "status": 200,
        "body": {
            "message": "Server Info",
            "path": path,
            "os_info": format!("{} {}", System::name().unwrap_or_else(|| "Unknown".to_string()), System::os_version().unwrap_or_else(|| "Unknown".to_string())),
            "hostname": System::host_name().unwrap_or_else(|| "Unknown".to_string()),
            "total_memory_gb": format!("{:.2}", total_memory_gb),
            "available_memory_gb": format!("{:.2}", available_memory_gb),
            "total_disk_gb": format!("{:.2}", total_disk_gb),
            "available_disk_gb": format!("{:.2}", available_disk_gb),
            "cpu_usage": format!("{:.2}", cpu_usage),
            "version": env!("CARGO_PKG_VERSION"),
            "build_date": env!("VERGEN_BUILD_TIMESTAMP"),
        },
        "headers": {
            "Content-Type": "application/json"
        }
    });

    CString::new(response_json.to_string()).unwrap().into_raw()
}