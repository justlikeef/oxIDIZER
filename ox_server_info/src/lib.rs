use axum::{response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString, c_char, c_void};
use sysinfo::{Disk, System};

use ox_webservice::{ModuleEndpoints, WebServiceHandler, WebServiceContext, ModuleEndpoint, InitializationData};

static mut MODULE_STATE: Option<WebServiceContext> = None;

#[derive(Debug, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub endpoints: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    pub build_date: String,
    pub running_directory: String,
    pub config_file_location: String,
    pub loaded_modules: Vec<ModuleInfo>,
    pub hostname: String,
    pub os_info: String,
    pub total_memory_gb: f64,
    pub available_memory_gb: f64,
    pub total_disk_gb: f64,
    pub available_disk_gb: f64,
    pub server_port: u16,
    pub bound_ip: String,
}

// This is the handler that will be called by the webservice
pub extern "C" fn server_info_handler_internal(
    request_ptr: *mut c_char,
) -> *mut c_char {
    let c_str = unsafe { CStr::from_ptr(request_ptr) };
    let request_json = c_str.to_str().expect("Failed to convert CStr to &str");
    let _request: serde_json::Value = if request_json.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(request_json).expect("Failed to deserialize request JSON")
    };

    let context = unsafe { MODULE_STATE.as_ref().unwrap() };

    let mut sys = System::new_all();
    sys.refresh_all();

    let total_memory_gb = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
    let available_memory_gb = sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0;

    let total_disk_gb = 0.0;
    let available_disk_gb = 0.0;
    // for disk in sys.disks() {
    //     total_disk_gb += disk.total_space() as f64 / 1024.0 / 1024.0 / 1024.0;
    //     available_disk_gb += disk.available_space() as f64 / 1024.0 / 1024.0 / 1024.0;
    // }

    let loaded_modules_info: Vec<ModuleInfo> = context
        .loaded_modules
        .iter()
        .map(|module_name| ModuleInfo {
            name: module_name.clone(),
            endpoints: vec![], // We don't have endpoint info here
        })
        .collect();

    let info = ServerInfo {
        version: context.version.clone(),
        build_date: context.build_date.clone(),
        running_directory: context.running_directory.clone(),
        config_file_location: context.config_file_location.clone(),
        loaded_modules: loaded_modules_info,
        hostname: context.hostname.clone(),
        os_info: context.os_info.clone(),
        total_memory_gb,
        available_memory_gb,
        total_disk_gb,
        available_disk_gb,
        server_port: context.server_port,
        bound_ip: context.bound_ip.clone(),
    };

    let response_json = serde_json::to_string(&info).expect("Failed to serialize ServerInfo");
    CString::new(response_json).unwrap().into_raw()
}

#[no_mangle]
pub extern "C" fn initialize_module(init_data_ptr: *mut c_char) -> *mut c_void {
    let init_data_str = unsafe { CStr::from_ptr(init_data_ptr).to_str().unwrap() };
    let init_data: InitializationData = serde_json::from_str(init_data_str).unwrap();

    unsafe {
        MODULE_STATE = Some(init_data.context);
    }

    let endpoints = vec![
        ModuleEndpoint {
            path: "".to_string(),
            handler: server_info_handler_internal,
            priority: 999,
        },
    ];
    let module_endpoints = ModuleEndpoints { endpoints };
    Box::into_raw(Box::new(module_endpoints)) as *mut c_void
}