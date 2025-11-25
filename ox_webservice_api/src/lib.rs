use libc::{c_char, c_void};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap; // Added
use std::sync::{Arc, RwLock}; // Added

// Define the C-compatible function signature for handlers
pub type WebServiceHandler = unsafe extern "C" fn(
    instance_ptr: *mut c_void, 
    context: *mut RequestContext, 
    log_callback: LogCallback
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
pub type ModuleContext = Arc<RwLock<HashMap<String, Value>>>; // Added

// --- FFI Helper Function Pointer Types ---
pub type GetModuleContextValueFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char) -> *mut c_char;
pub type SetModuleContextValueFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char, value_json: *const c_char);
pub type GetRequestMethodFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;
pub type GetRequestPathFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;
pub type GetRequestQueryFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;
pub type GetRequestHeaderFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char) -> *mut c_char;
pub type GetRequestHeadersFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;
pub type GetRequestBodyFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;
pub type GetSourceIpFn = unsafe extern "C" fn(context: *mut RequestContext) -> *mut c_char;
pub type SetRequestPathFn = unsafe extern "C" fn(context: *mut RequestContext, path: *const c_char);
pub type SetRequestHeaderFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char, value: *const c_char);
pub type SetSourceIpFn = unsafe extern "C" fn(context: *mut RequestContext, ip: *const c_char);
pub type GetResponseStatusFn = unsafe extern "C" fn(context: *mut RequestContext) -> u16;
pub type GetResponseHeaderFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char) -> *mut c_char;
pub type SetResponseStatusFn = unsafe extern "C" fn(context: *mut RequestContext, status_code: u16);
pub type SetResponseHeaderFn = unsafe extern "C" fn(context: *mut RequestContext, key: *const c_char, value: *const c_char);
pub type SetResponseBodyFn = unsafe extern "C" fn(context: *mut RequestContext, body: *const u8, body_len: usize);
pub type RenderTemplateFn = unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WebServiceApiV1 {
    pub log_callback: LogCallback,
    pub render_template: RenderTemplateFn,
    
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

#[repr(C)]
pub struct RequestContext {
    pub pipeline_state_ptr: *mut c_void, // Pointer to the PipelineState struct
}

