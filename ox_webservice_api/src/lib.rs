use libc::{c_char, c_void};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

// Define the C-compatible function signature for handlers
pub type WebServiceHandler = unsafe extern "C" fn(*mut c_char) -> *mut c_char;

#[derive(Debug, Serialize, Clone)]
pub struct ModuleEndpoint {
    pub path: String,
    #[serde(skip)]
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
#[derive(Debug, Serialize, Deserialize, Clone)]
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
    pub bound_ip: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InitializationData {
    pub context: WebServiceContext,
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub endpoints: Vec<String>,
}

impl<'de> Deserialize<'de> for ModuleEndpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ModuleEndpointHelper {
            path: String,
            priority: u16,
        }

        let helper = ModuleEndpointHelper::deserialize(deserializer)?;
        Ok(ModuleEndpoint {
            path: helper.path,
            handler: dummy_handler, // Assign a dummy handler
            priority: helper.priority,
        })
    }
}

unsafe extern "C" fn dummy_handler(_request_ptr: *mut c_char) -> *mut c_char {
    // Return a null pointer or an empty string
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn render_template_ffi(template_name_ptr: *mut c_char, data_ptr: *mut c_char) -> *mut c_char {
    // This will be implemented in ox_webservice/src/main.rs
    std::ptr::null_mut()
}