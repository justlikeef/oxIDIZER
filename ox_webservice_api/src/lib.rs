use libc::{c_char, c_void};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use axum::http::HeaderMap;
use std::net::SocketAddr;
use bumpalo::Bump;

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
}

// Define the C-compatible function signature for handlers
pub type WebServiceHandler = unsafe extern "C" fn(
    instance_ptr: *mut c_void, 
    pipeline_state_ptr: *mut PipelineState, 
    log_callback: LogCallback,
    alloc_fn: AllocFn,
    arena: *const c_void,
) -> HandlerResult;

#[repr(C)]
pub struct ModuleInterface {
    pub instance_ptr: *mut c_void,
    pub handler_fn: WebServiceHandler,
    pub log_callback: LogCallback,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl From<LogLevel> for log::Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Error => log::Level::Error,
            LogLevel::Warn => log::Level::Warn,
            LogLevel::Info => log::Level::Info,
            LogLevel::Debug => log::Level::Debug,
            LogLevel::Trace => log::Level::Trace,
        }
    }
}

pub type LogCallback = unsafe extern "C" fn(level: LogLevel, module: *const c_char, message: *const c_char);




// Define the shared Module Context type
pub type ModuleContext = Arc<RwLock<HashMap<String, Value>>>;

pub type AllocStrFn = unsafe extern "C" fn(arena: *const c_void, s: *const c_char) -> *mut c_char;
pub type AllocFn = unsafe extern "C" fn(arena: *mut c_void, size: usize, align: usize) -> *mut c_void;

#[repr(C)]
pub struct WebServiceApiV1 {
    pub log_callback: LogCallback,
    pub alloc_str: AllocStrFn,
    pub alloc_raw: AllocFn,
    
    // Module Context
    pub get_module_context_value: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,
    pub set_module_context_value: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, value_json: *const c_char),

    // Request Getters
    pub get_request_method: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,
    pub get_request_path: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,
    pub get_request_query: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,
    pub get_request_header: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,
    pub get_request_headers: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,
    pub get_request_body: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,
    pub get_source_ip: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,

    // Request Setters
    pub set_request_path: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, path: *const c_char),
    pub set_request_header: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, value: *const c_char),
    pub set_source_ip: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, ip: *const c_char),

    // Response Getters
    pub get_response_status: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState) -> u16,
    pub get_response_header: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,
    pub get_response_body: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,

    // Response Setters
    pub set_response_status: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, status_code: u16),
    pub set_response_header: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, value: *const c_char),
    pub set_response_body: unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, body: *const u8, body_len: usize),

    // Server Metrics
    pub get_server_metrics: unsafe extern "C" fn(arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char,
}

pub type InitializeModuleFn = unsafe extern "C" fn(
    module_params_json_ptr: *const c_char,
    api: *const WebServiceApiV1
) -> *mut ModuleInterface;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UriMatcher {
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleConfig {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub uris: Option<Vec<UriMatcher>>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub error_path: Option<String>,
    #[serde(default = "default_phase")]
    pub phase: Phase,
    #[serde(default)]
    pub priority: u16,
}

impl Default for ModuleConfig {
    fn default() -> Self {
        ModuleConfig {
            id: None,
            name: String::new(),
            params: None,
            uris: None,
            path: None,
            error_path: None,
            phase: default_phase(),
            priority: 0,
        }
    }
}

fn default_phase() -> Phase {
    Phase::Content
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Phase {
    PreEarlyRequest,
    EarlyRequest,
    PostEarlyRequest,
    PreAuthentication,
    Authentication,
    PostAuthentication,
    PreAuthorization,
    Authorization,
    PostAuthorization,
    PreContent,
    Content,
    PostContent,
    PreAccounting,
    Accounting,
    PostAccounting,
    PreErrorHandling,
    ErrorHandling,
    PostErrorHandling,
    PreLateRequest,
    LateRequest,
    PostLateRequest,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerResult {
    UnmodifiedContinue,
    ModifiedContinue,
    UnmodifiedNextPhase,
    ModifiedNextPhase,
    UnmodifiedJumpToError,
    ModifiedJumpToError,
    HaltProcessing,
}
