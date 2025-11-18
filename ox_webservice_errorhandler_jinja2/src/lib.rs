use std::panic;
use ox_webservice_api::{
    ModuleInterface, LogCallback, LogLevel, RequestContext, HandlerResult,
    GetRequestMethodFn, GetRequestPathFn, GetRequestQueryFn, GetRequestHeaderFn, GetRequestHeadersFn,
    GetRequestBodyFn, GetSourceIpFn, SetRequestPathFn, SetRequestHeaderFn, SetSourceIpFn, GetResponseStatusFn,
    GetResponseHeaderFn, SetResponseStatusFn, SetResponseHeaderFn, SetResponseBodyFn,
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use tera::{Context, Tera};

// --- Static FFI Function Pointers ---
static mut LOGGER_CALLBACK: Option<LogCallback> = None;
static mut GET_REQUEST_METHOD_FN: Option<GetRequestMethodFn> = None;
static mut GET_REQUEST_PATH_FN: Option<GetRequestPathFn> = None;
static mut GET_REQUEST_QUERY_FN: Option<GetRequestQueryFn> = None;
static mut GET_REQUEST_HEADER_FN: Option<GetRequestHeaderFn> = None;
static mut GET_REQUEST_HEADERS_FN: Option<GetRequestHeadersFn> = None;
static mut GET_REQUEST_BODY_FN: Option<GetRequestBodyFn> = None;
static mut GET_SOURCE_IP_FN: Option<GetSourceIpFn> = None;
static mut SET_REQUEST_PATH_FN: Option<SetRequestPathFn> = None;
static mut SET_REQUEST_HEADER_FN: Option<SetRequestHeaderFn> = None;
static mut SET_SOURCE_IP_FN: Option<SetSourceIpFn> = None;
static mut GET_RESPONSE_STATUS_FN: Option<GetResponseStatusFn> = None;
static mut GET_RESPONSE_HEADER_FN: Option<GetResponseHeaderFn> = None;
static mut SET_RESPONSE_STATUS_FN: Option<SetResponseStatusFn> = None;
static mut SET_RESPONSE_HEADER_FN: Option<SetResponseHeaderFn> = None;
static mut SET_RESPONSE_BODY_FN: Option<SetResponseBodyFn> = None;


macro_rules! module_log {
    ($level:expr, $($arg:tt)*) => ({
        if let Some(log_cb) = unsafe { LOGGER_CALLBACK } {
            let message = format!($($arg)*);
            if let Ok(c_message) = CString::new(message) {
                unsafe {
                    log_cb($level, c_message.as_ptr());
                }
            }
        }
    });
}

pub struct Jinja2ErrorHandler {
    content_root: PathBuf,
}

impl Jinja2ErrorHandler {
    pub fn new(config: ErrorHandlerConfig) -> anyhow::Result<Self> {
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: new: Initializing with content_root: {:?}", config.content_root);
        Ok(Self { content_root: config.content_root })
    }

    pub fn process_request(&self, context: &mut RequestContext) -> HandlerResult {
        let status_code = unsafe { GET_RESPONSE_STATUS_FN.unwrap()(context) };

        // This module only acts on requests that are already in an error state.
        if status_code < 400 {
            return HandlerResult::UnmodifiedContinue;
        }

        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: Handling error request with status code: {}", status_code);

        let mut render_context = Context::new();
        render_context.insert("status_code", &status_code);
        render_context.insert("message", "An error occurred.");
        render_context.insert("module_name", "Unknown");

        let template_filename = format!("{}.jinja2", status_code);
        let template_path = self.content_root.join(&template_filename);

        let html_content = if template_path.exists() {
            match std::fs::read_to_string(&template_path) {
                Ok(template_str) => {
                    match Tera::one_off(&template_str, &render_context, false) {
                        Ok(html) => html,
                        Err(e) => {
                            module_log!(LogLevel::Error, "Failed to render template '{}': {}", template_filename, e);
                            format!("<h1>Internal Server Error</h1><p>Failed to render error template.</p>")
                        }
                    }
                }
                Err(e) => {
                    module_log!(LogLevel::Error, "Failed to read template file '{}': {}", template_filename, e);
                    format!("<h1>Internal Server Error</h1><p>Failed to read error template file.</p>")
                }
            }
        } else {
            module_log!(LogLevel::Warn, "No specific error template found for status {}. No fallback implemented yet.", status_code);
            format!("<h1>Error {}</h1><p>No error template configured.</p>", status_code)
        };

        unsafe {
            if let (Some(set_body_fn), Some(set_header_fn)) = (SET_RESPONSE_BODY_FN, SET_RESPONSE_HEADER_FN) {
                let c_body = CString::new(html_content).unwrap();
                let c_content_type_key = CString::new("Content-Type").unwrap();
                let c_content_type_value = CString::new("text/html").unwrap();

                set_header_fn(context, c_content_type_key.as_ptr(), c_content_type_value.as_ptr());
                set_body_fn(context, c_body.as_ptr().cast(), c_body.as_bytes().len());
            }
        }

        HandlerResult::ModifiedContinue
    }
}

#[derive(Debug, Deserialize)]
pub struct ErrorHandlerConfig {
    pub content_root: PathBuf,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    _render_template_ffi: unsafe extern "C" fn(*mut c_char, *mut c_char) -> *mut c_char,
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
) -> *mut ModuleInterface {
    unsafe {
        LOGGER_CALLBACK = Some(log_callback);
        // Request Getters
        GET_REQUEST_METHOD_FN = Some(get_request_method_fn);
        GET_REQUEST_PATH_FN = Some(get_request_path_fn);
        GET_REQUEST_QUERY_FN = Some(get_request_query_fn);
        GET_REQUEST_HEADER_FN = Some(get_request_header_fn);
        GET_REQUEST_HEADERS_FN = Some(get_request_headers_fn);
        GET_REQUEST_BODY_FN = Some(get_request_body_fn);
        GET_SOURCE_IP_FN = Some(get_source_ip_fn);
        // Request Setters
        SET_REQUEST_PATH_FN = Some(set_request_path_fn);
        SET_REQUEST_HEADER_FN = Some(set_request_header_fn);
        SET_SOURCE_IP_FN = Some(set_source_ip_fn);
        // Response Getters
        GET_RESPONSE_STATUS_FN = Some(get_response_status_fn);
        GET_RESPONSE_HEADER_FN = Some(get_response_header_fn);
        // Response Setters
        SET_RESPONSE_STATUS_FN = Some(set_response_status_fn);
        SET_RESPONSE_HEADER_FN = Some(set_response_header_fn);
        SET_RESPONSE_BODY_FN = Some(set_response_body_fn);
    }

    let result = panic::catch_unwind(|| {
        let module_params_json = unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap() };
        let params: Value = serde_json::from_str(module_params_json)
            .expect("Failed to parse module params JSON");

        let config_file_name = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                module_log!(LogLevel::Error, "'config_file' parameter is missing or not a string.");
                return std::ptr::null_mut();
            }
        };

        let contents = match std::fs::read_to_string(config_file_name) {
            Ok(c) => c,
            Err(e) => {
                module_log!(LogLevel::Error, "Failed to read config file '{}': {}", config_file_name, e);
                return std::ptr::null_mut();
            }
        };

        let config: ErrorHandlerConfig = match serde_yaml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                module_log!(LogLevel::Error, "Failed to deserialize ErrorHandlerConfig: {}", e);
                return std::ptr::null_mut();
            }
        };

        let handler = match Jinja2ErrorHandler::new(config) {
            Ok(eh) => eh,
            Err(e) => {
                module_log!(LogLevel::Error, "Failed to create Jinja2ErrorHandler: {}", e);
                return std::ptr::null_mut();
            }
        };

        let instance_ptr = Box::into_raw(Box::new(handler)) as *mut c_void;

        let module_interface = Box::new(ModuleInterface {
            instance_ptr,
            handler_fn: process_request_c,
        });

        Box::into_raw(module_interface)
    });

    match result {
        Ok(ptr) => ptr,
        Err(e) => {
            module_log!(LogLevel::Error, "Panic occurred during module initialization: {:?}.", e);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn process_request_c(instance_ptr: *mut c_void, context_ptr: *mut RequestContext) -> HandlerResult {
    let result = panic::catch_unwind(|| {
        let handler = unsafe { &*(instance_ptr as *mut Jinja2ErrorHandler) };
        let context = unsafe { &mut *context_ptr };
        handler.process_request(context)
    });

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
            module_log!(LogLevel::Error, "Panic occurred in process_request_c: {:?}.", e);
            // If the handler panics, we should signal an error to the pipeline
            HandlerResult::ModifiedJumpToError
        }
    }
}

// No destroy_module function needed for now, as the Box will be dropped when the LoadedModule is dropped.
