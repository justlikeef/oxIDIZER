use std::ffi::{c_char, c_void, CString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use axum::http::HeaderMap;
use std::net::SocketAddr;
use bumpalo::Bump;

// Re-export generic types from ox_pipeline_plugin
pub use ox_pipeline_plugin::{
    LogLevel, FlowControl, ModuleStatus, ReturnParameters, HandlerResult,
    LogCallback, AllocStrFn, AllocFn, GetStateFn, SetStateFn, GetConfigFn, CoreHostApi
};

// --- Pipeline State ---
// This struct is owned by the host (ox_webservice) and contains all request-specific data.
#[repr(C)]
pub struct PipelineState {
    pub arena: Bump,
    pub protocol: String,
    pub request_method: String,
    pub request_path: String,
    pub request_query: String,
    pub request_headers: HeaderMap,
    pub request_body: Vec<u8>,
    pub source_ip: SocketAddr,
    pub status_code: u16,
    pub response_headers: HeaderMap,
    pub response_body: Vec<u8>,
    pub module_context: ModuleContext,
    pub pipeline_ptr: *const c_void,
    // New Fields
    pub flags: std::collections::HashSet<String>,
    pub execution_history: Vec<ModuleExecutionRecord>,
    // Route Capture (for path rewriting)
    pub route_capture: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ModuleExecutionRecord {
    pub module_name: String,
    pub status: ModuleStatus,
    pub flow_control: FlowControl,
    #[serde(skip)]
    #[serde(default = "std::ptr::null_mut")]
    pub return_data: *mut c_void, 
}

unsafe impl Send for ModuleExecutionRecord {}
unsafe impl Sync for ModuleExecutionRecord {}

unsafe impl Send for PipelineState {}
unsafe impl Sync for PipelineState {}

impl PipelineState {
    pub fn add_flag(&mut self, flag: &str) {
        self.flags.insert(flag.to_string());
    }

    pub fn remove_flag(&mut self, flag: &str) {
        self.flags.remove(flag);
    }

    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.contains(flag)
    }
}

// Define the C-compatible function signature for handlers
pub type WebServiceHandler = unsafe extern "C" fn(
    instance_ptr: *mut c_void, 
    pipeline_state_ptr: *mut PipelineState, 
    // We pass CoreHostApi instead of full V1 in the new system?
    // Maintaining signature for now but types are from ox_plugin
    log_callback: LogCallback,
    alloc_fn: AllocFn,
    arena: *const c_void,
) -> HandlerResult;

#[repr(C)]
pub struct ModuleInterface {
    pub instance_ptr: *mut c_void,
    pub handler_fn: WebServiceHandler,
    pub log_callback: LogCallback,
    pub get_config: GetConfigFn,
}

// Log implementation...
struct ModuleLogger {
    callback: LogCallback,
    module_name: CString,
}

impl log::Log for ModuleLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Trace
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            if let Ok(c_msg) = CString::new(format!("{}", record.args())) {
                let level = match record.level() {
                    log::Level::Error => LogLevel::Error,
                    log::Level::Warn => LogLevel::Warn,
                    log::Level::Info => LogLevel::Info,
                    log::Level::Debug => LogLevel::Debug,
                    log::Level::Trace => LogLevel::Trace,
                };
                unsafe {
                    (self.callback)(level, self.module_name.as_ptr(), c_msg.as_ptr());
                }
            }
        }
    }

    fn flush(&self) {}
}

pub fn init_logging(callback: LogCallback, module_name: &str) -> std::result::Result<(), log::SetLoggerError> {
    let logger = Box::new(ModuleLogger {
        callback,
        module_name: CString::new(module_name).unwrap(),
    });
    log::set_boxed_logger(logger)?;
    log::set_max_level(log::LevelFilter::Trace);
    Ok(())
}


// Define the shared Module Context type
pub type ModuleContext = Arc<RwLock<HashMap<String, Value>>>;


// We will keep WebServiceApiV1 but usage should migrate to CoreHostApi
// For partial compatibility, we ensure CoreHostApi is a prefix or we just don't use WebServiceApiV1 in new modules.

// WebServiceApiV1 is now fully generic.
pub type WebServiceApiV1 = CoreHostApi;

// Module Initialization now takes CoreHostApi
pub type InitializeModuleFn = unsafe extern "C" fn(
    module_params_json_ptr: *const c_char,
    module_id: *const c_char,
    api: *const CoreHostApi
) -> *mut ModuleInterface;

// Config structs (UriMatcher, ModuleConfig, Phase etc.)
// Phase is specific to this host logic, so it stays here.

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UriMatcher {
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(alias = "url")]
    pub path: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub query: Option<HashMap<String, String>>,
    #[serde(default)]
    pub priority: u16,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub status_code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleConfig {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, alias = "uris")]
    pub routes: Option<Vec<UriMatcher>>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub query: Option<HashMap<String, String>>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(flatten)]
    pub extra_params: HashMap<String, Value>,
}

impl Default for ModuleConfig {
    fn default() -> Self {
        ModuleConfig {
            id: None,
            name: String::new(),
            routes: None,
            headers: None,
            query: None,
            path: None,
            params: None,
            extra_params: HashMap::new(),
        }
    }
}




