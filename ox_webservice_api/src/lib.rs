use libc::{c_char, c_void};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// Define the C-compatible function signature for handlers
pub type WebServiceHandler = unsafe extern "C" fn(instance_ptr: *mut c_void, context: *mut RequestContext) -> HandlerResult;

#[repr(C)]
pub struct ModuleInterface {
    pub instance_ptr: *mut c_void,
    pub handler_fn: WebServiceHandler,
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

// --- FFI Helper Function Pointer Types ---

// --- Request Getters ---
pub type GetRequestMethodFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;
pub type GetRequestPathFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;
pub type GetRequestQueryFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;
pub type GetRequestHeaderFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char) -> *mut c_char;
pub type GetRequestHeadersFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char; // Returns JSON
pub type GetRequestBodyFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char; // Returns raw body
pub type GetSourceIpFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;

// --- Request Setters ---
pub type SetRequestPathFn = unsafe extern "C" fn(context: *mut RequestContext, path: *const c_char);
pub type SetRequestHeaderFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char, value: *const c_char);
pub type SetSourceIpFn = unsafe extern "C" fn(context: *mut RequestContext, ip: *const c_char);

// --- Response Getters ---
pub type GetResponseStatusFn = unsafe extern "C" fn(context: *mut RequestContext) -> u16;
pub type GetResponseHeaderFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char) -> *mut c_char;

// --- Response Setters ---
pub type SetResponseStatusFn = unsafe extern "C" fn(context: *mut RequestContext, status_code: u16);
pub type SetResponseHeaderFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char, value: *const c_char);
pub type SetResponseBodyFn = unsafe extern "C" fn(context: *mut RequestContext, body: *const u8, body_len: usize);

pub type InitializeModuleFn = unsafe extern "C" fn(
    module_params_json_ptr: *const c_char,
    render_template_ffi: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char,
    log_callback: LogCallback,
    // Request Getters
    get_request_method_fn: GetRequestMethodFn,
    get_request_path_fn: GetRequestPathFn,
    get_request_query_fn: GetRequestQueryFn,
    get_request_header_fn: GetRequestHeaderFn,
    get_request_headers_fn: GetRequestHeadersFn,
    get_request_body_fn: GetRequestBodyFn,
    get_source_ip_fn: GetSourceIpFn,
    // Request Setters
    set_request_path_fn: SetRequestPathFn,
    set_request_header_fn: SetRequestHeaderFn,
    set_source_ip_fn: SetSourceIpFn,
    // Response Getters
    get_response_status_fn: GetResponseStatusFn,
    get_response_header_fn: GetResponseHeaderFn,
    // Response Setters
    set_response_status_fn: SetResponseStatusFn,
    set_response_header_fn: SetResponseHeaderFn,
    set_response_body_fn: SetResponseBodyFn,
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
pub struct ModuleConfig {
    pub name: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub uris: Option<Vec<String>>,
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

#[repr(C)]
pub struct RequestContext {
    pub pipeline_state_ptr: *mut c_void, // Pointer to the PipelineState struct
}

