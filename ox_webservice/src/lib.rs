use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use serde::{Serialize, Deserialize};

// Define the C-compatible function signature for handlers
pub type WebServiceHandler = unsafe extern "C" fn(*mut c_char) -> *mut c_char;

#[derive(Debug, Serialize, Deserialize)]
pub struct ModuleEndpoint {
    pub path: String,
    #[serde(skip)] // Don't serialize the function pointer
    pub handler: WebServiceHandler,
    pub priority: u16,
}

// This struct will be returned by dynamically loaded modules
// It contains a list of endpoints, each with a URL path and a raw function pointer to its handler
#[derive(Debug, Serialize, Deserialize)]
pub struct ModuleEndpoints {
    pub endpoints: Vec<ModuleEndpoint>,
}

// C-compatible function to destroy ModuleEndpoints instance
#[no_mangle]
pub extern "C" fn destroy_module_endpoints(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        // Reconstruct the Box and let it drop
        let _ = Box::from_raw(ptr as *mut ModuleEndpoints);
    }
}

// Context struct to be passed to dynamically loaded modules
#[derive(Debug, Serialize, Deserialize)]
pub struct WebServiceContext {
    pub version: String,
    pub build_date: String,
    pub running_directory: String,
    pub config_file_location: String,
    pub loaded_modules: Vec<String>,
    pub hostname: String,
    pub os_info: String,
    pub total_memory_gb: f64,
    pub available_memory_gb: f64,
    pub total_disk_gb: f64,
    pub available_disk_gb: f64,
    pub server_port: u16,
}

// C-compatible function to destroy WebServiceContext instance
#[no_mangle]
pub extern "C" fn destroy_webservice_context(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        // Reconstruct the Box and let it drop
        let _ = Box::from_raw(ptr as *mut WebServiceContext);
    }
}