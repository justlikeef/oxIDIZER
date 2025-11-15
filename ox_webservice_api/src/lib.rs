use libc::{c_char, c_void, c_uint};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use axum::response::{IntoResponse, Response};
use std::future::Future;

// Define the C-compatible function signature for handlers
pub type WebServiceHandler = unsafe extern "C" fn(*mut c_char) -> *mut c_char;

#[derive(Debug, Clone, Copy)] // Copy is important for raw function pointers
#[repr(C)]
pub struct SendableWebServiceHandler(pub WebServiceHandler);

unsafe impl Send for SendableWebServiceHandler {}
unsafe impl Sync for SendableWebServiceHandler {}

#[repr(C)]
pub enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

pub type LogCallback = unsafe extern "C" fn(level: LogLevel, message: *const c_char);

pub type InitializeModuleFn = unsafe extern "C" fn(
    *mut c_void,
    unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char,
    LogCallback, // Add this
) -> SendableWebServiceHandler;


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
    pub error_path: Option<String>,
}

impl Default for ModuleConfig {
    fn default() -> Self {
        ModuleConfig {
            name: String::new(),
            params: None,
            error_path: None,
        }
    }
}

// C-compatible function signature for error handlers
pub type CErrorHandlerFn = unsafe extern "C" fn(
    *mut c_void, // A pointer to the actual error handler instance (self)
    u16,         // status_code as u16
    *mut c_char, // message as CString
    *mut c_char, // context as CString
    *mut c_char, // params as CString (JSON string)
    *mut c_char, // module_context as CString (JSON string)
) -> *mut c_char; // Returned HTML as CString

// C-compatible struct to represent an error handler
#[repr(C)]
pub struct CErrorHandler {
    pub instance_ptr: *mut c_void, // Pointer to the actual ErrorHandler instance
    pub handle_error_fn: CErrorHandlerFn,
}

unsafe impl Send for CErrorHandler {}
unsafe impl Sync for CErrorHandler {}

// Type alias for the factory function that creates CErrorHandler
pub type ErrorHandlerFactory = unsafe extern "C" fn(*mut c_void, LogCallback) -> *mut CErrorHandler; // Add LogCallback

#[derive(Debug, Clone, Copy)]
pub struct SendableCErrorHandler(pub *mut CErrorHandler);

unsafe impl Send for SendableCErrorHandler {}

pub trait ErrorHandler {
    fn handle_error(&self, status_code: u16, message: &str, context: &str, params: &Value, module_context: &str) -> String;
}

#[derive(Debug, Deserialize)]
pub struct InitializationData {
    pub config_path: String,
    pub context: WebServiceContext,
}

