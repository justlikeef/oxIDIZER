use libc::{c_char, c_void};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use axum::http::HeaderMap;
use std::net::SocketAddr;
use bumpalo::Bump;
use std::ffi::{CStr, CString};
use log::{error, warn, info, debug, trace};

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
pub enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

pub type LogCallback = unsafe extern "C" fn(level: LogLevel, message: *const c_char);

// Define the shared Module Context type
pub type ModuleContext = Arc<RwLock<HashMap<String, Value>>>;

// --- FFI Helper Function Pointer Types ---
pub type GetModuleContextValueFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type SetModuleContextValueFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, value_json: *const c_char);
pub type GetRequestMethodFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type GetRequestPathFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type GetRequestQueryFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type GetRequestHeaderFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type GetRequestHeadersFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type GetRequestBodyFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type GetSourceIpFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type SetRequestPathFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, path: *const c_char);
pub type SetRequestHeaderFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, value: *const c_char);
pub type SetSourceIpFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, ip: *const c_char);
pub type GetResponseStatusFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState) -> u16;
pub type GetResponseHeaderFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type SetResponseStatusFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, status_code: u16);
pub type SetResponseHeaderFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, key: *const c_char, value: *const c_char);
pub type SetResponseBodyFn = unsafe extern "C" fn(pipeline_state_ptr: *mut PipelineState, body: *const u8, body_len: usize);
pub type AllocStrFn = unsafe extern "C" fn(arena: *const c_void, s: *const c_char) -> *mut c_char;
pub type AllocFn = unsafe extern "C" fn(arena: *mut c_void, size: usize, align: usize) -> *mut c_void;


#[repr(C)]
#[derive(Clone, Copy)]
pub struct WebServiceApiV1 {
    pub log_callback: LogCallback,
    pub alloc_str: AllocStrFn,
    pub alloc_raw: AllocFn,
    
    // Module Context
    pub get_module_context_value: GetModuleContextValueFn,
    pub set_module_context_value: SetModuleContextValueFn,

    // Request Getters
    pub get_request_method: GetRequestMethodFn,
    pub get_request_path: GetRequestPathFn,
    pub get_request_query: GetRequestQueryFn,
    pub get_request_header: GetRequestHeaderFn,
    pub get_request_headers: GetRequestHeadersFn,
    pub get_request_body: GetRequestBodyFn,
    pub get_source_ip: GetSourceIpFn,

    // Request Setters
    pub set_request_path: SetRequestPathFn,
    pub set_request_header: SetRequestHeaderFn,
    pub set_source_ip: SetSourceIpFn,

    // Response Getters
    pub get_response_status: GetResponseStatusFn,
    pub get_response_header: GetResponseHeaderFn,

    // Response Setters
    pub set_response_status: SetResponseStatusFn,
    pub set_response_header: SetResponseHeaderFn,
    pub set_response_body: SetResponseBodyFn,
}

pub type InitializeModuleFn = unsafe extern "C" fn(
    module_params_json_ptr: *const c_char,
    api: *const WebServiceApiV1
) -> *mut ModuleInterface;


// Context struct to be passed to dynamically loaded modules
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WebServiceContext {
}

impl Default for WebServiceContext {
    fn default() -> Self {
        Self {
        }
    }
}

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
    pub error_path: Option<String>,
    #[serde(default = "default_phase")] // New field
    pub phase: Phase,
    #[serde(default)] // New field
    pub priority: u16,
}

impl Default for ModuleConfig {
    fn default() -> Self {
        ModuleConfig {
            id: None,
            name: String::new(),
            params: None,
            uris: None,
            error_path: None,
            phase: default_phase(), // Default to Content phase
            priority: 0, // Default priority
        }
    }
}

// New default function for phase
fn default_phase() -> Phase {
    Phase::Content
}



#[derive(Debug, Deserialize)]
pub struct InitializationData {
    pub config_path: String,
    pub context: WebServiceContext,
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
    UnmodifiedContinue, // Did nothing, continue in current phase
    ModifiedContinue,   // Modified context, continue in current phase
    UnmodifiedNextPhase,  // Did nothing, skip to next phase
    ModifiedNextPhase,    // Modified context, skip to next phase
    UnmodifiedJumpToError, // Did nothing, jump to PreErrorHandling phase
    ModifiedJumpToError,   // Modified context, jump to PreErrorHandling phase
    HaltProcessing,     // Fatal error, stop pipeline immediately
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn log_callback(level: LogLevel, message: *const c_char) {
    let message = unsafe { CStr::from_ptr(message).to_string_lossy() };
    match level {
        LogLevel::Error => error!("{}", message),
        LogLevel::Warn => warn!("{}", message),
        LogLevel::Info => info!("{}", message),
        LogLevel::Debug => debug!("{}", message),
        LogLevel::Trace => trace!("{}", message),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_module_context_value_c(pipeline_state_ptr: *mut PipelineState, key_ptr: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state_ptr };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    let module_context_read_guard = state.module_context.read().unwrap();
    match module_context_read_guard.get(key) {
        Some(value) => {
            let s = serde_json::to_string(value).unwrap();
            let c_str = CString::new(s).unwrap();
            unsafe { alloc_fn(arena, c_str.as_ptr()) }
        }
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_module_context_value_c(pipeline_state_ptr: *mut PipelineState, key_ptr: *const c_char, value_json_ptr: *const c_char) {
    let state = unsafe { &mut *pipeline_state_ptr };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    let value_json = unsafe { CStr::from_ptr(value_json_ptr).to_str().unwrap() };
    let value: Value = serde_json::from_str(value_json).unwrap_or_default();
    state.module_context.write().unwrap().insert(key.to_string(), value);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_method_c(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state_ptr };
    let c_str = CString::new(state.request_method.as_str()).unwrap();
    unsafe { alloc_fn(arena, c_str.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_path_c(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state_ptr };
    let c_str = CString::new(state.request_path.as_str()).unwrap();
    unsafe { alloc_fn(arena, c_str.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_query_c(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state_ptr };
    let c_str = CString::new(state.request_query.as_str()).unwrap();
    unsafe { alloc_fn(arena, c_str.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_header_c(pipeline_state_ptr: *mut PipelineState, key_ptr: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state_ptr };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    match state.request_headers.get(key) {
        Some(value) => {
            let c_str = CString::new(value.to_str().unwrap_or("")).unwrap();
            unsafe { alloc_fn(arena, c_str.as_ptr()) }
        }
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_headers_c(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state_ptr };
    let headers: HashMap<String, String> = state.request_headers.iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let json = serde_json::to_string(&headers).unwrap();
    let c_str = CString::new(json).unwrap();
    unsafe { alloc_fn(arena, c_str.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_body_c(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state_ptr };
    let c_str = CString::new(state.request_body.as_slice()).unwrap();
    unsafe { alloc_fn(arena, c_str.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_source_ip_c(pipeline_state_ptr: *mut PipelineState, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state_ptr };
    let c_str = CString::new(state.source_ip.to_string()).unwrap();
    unsafe { alloc_fn(arena, c_str.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_request_path_c(pipeline_state_ptr: *mut PipelineState, path_ptr: *const c_char) {
    let state = unsafe { &mut *pipeline_state_ptr };
    let path = unsafe { CStr::from_ptr(path_ptr).to_str().unwrap() };
    state.request_path = path.to_string();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_request_header_c(pipeline_state_ptr: *mut PipelineState, key_ptr: *const c_char, value_ptr: *const c_char) {
    let state = unsafe { &mut *pipeline_state_ptr };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    let value = unsafe { CStr::from_ptr(value_ptr).to_str().unwrap() };
    state.request_headers.insert(axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(), axum::http::HeaderValue::from_str(value).unwrap());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_source_ip_c(pipeline_state_ptr: *mut PipelineState, ip_ptr: *const c_char) {
    let state = unsafe { &mut *pipeline_state_ptr };
    let ip_str = unsafe { CStr::from_ptr(ip_ptr).to_str().unwrap() };
    match ip_str.parse::<SocketAddr>() {
        Ok(addr) => {
            state.source_ip = addr;
        }
        Err(e) => {
            // It's better to log an error than to panic
            // Use a logging framework in a real application
            eprintln!("Failed to parse IP address '{}': {}", ip_str, e);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_response_status_c(pipeline_state_ptr: *mut PipelineState) -> u16 {
    let state = unsafe { &*pipeline_state_ptr };
    state.status_code
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_response_header_c(pipeline_state_ptr: *mut PipelineState, key_ptr: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state_ptr };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    match state.response_headers.get(key) {
        Some(value) => {
            let c_str = CString::new(value.to_str().unwrap_or("")).unwrap();
            unsafe { alloc_fn(arena, c_str.as_ptr()) }
        }
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_status_c(pipeline_state_ptr: *mut PipelineState, status_code: u16) {
    let state = unsafe { &mut *pipeline_state_ptr };
    state.status_code = status_code;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_header_c(pipeline_state_ptr: *mut PipelineState, key_ptr: *const c_char, value_ptr: *const c_char) {
    let state = unsafe { &mut *pipeline_state_ptr };
    let key = unsafe { CStr::from_ptr(key_ptr).to_str().unwrap() };
    let value = unsafe { CStr::from_ptr(value_ptr).to_str().unwrap() };
    state.response_headers.insert(axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(), axum::http::HeaderValue::from_str(value).unwrap());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_body_c(pipeline_state_ptr: *mut PipelineState, body_ptr: *const u8, body_len: usize) {
    let state = unsafe { &mut *pipeline_state_ptr };
    let body_slice = unsafe { std::slice::from_raw_parts(body_ptr, body_len) };
    state.response_body = body_slice.to_vec();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn alloc_raw_c(arena: *mut c_void, size: usize, align: usize) -> *mut c_void {
    let arena = unsafe { &mut *(arena as *mut Bump) };
    let layout = unsafe { std::alloc::Layout::from_size_align_unchecked(size, align) };
    arena.alloc_layout(layout).as_ptr() as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn alloc_str_c(arena: *const c_void, s: *const c_char) -> *mut c_char {
    let arena = unsafe { &*(arena as *const Bump) };
    let s = unsafe { CStr::from_ptr(s).to_str().unwrap() };
    let allocated_str = arena.alloc_str(s);
    allocated_str.as_ptr() as *mut c_char
}