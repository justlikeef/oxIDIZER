use std::panic;
use std::io::{self, Read, Write};
use axum::http::StatusCode;
use ox_webservice_api::{CErrorHandler, ErrorHandler, ModuleConfig, LogCallback, LogLevel};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use tera::{Context, Tera};
use regex::Regex;

static mut LOGGER_CALLBACK: Option<LogCallback> = None;

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
    tera: Tera,
    content_root: PathBuf,
}

impl Jinja2ErrorHandler {
    pub fn new(config: ErrorHandlerConfig) -> anyhow::Result<Self> {
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: new: Initializing Tera with content_root: {:?}", config.content_root);
        // Store the content_root in the handler for later use
        Ok(Self { tera: Tera::default(), content_root: config.content_root })
    }
}

impl ErrorHandler for Jinja2ErrorHandler {
    fn handle_error(&self, status_code: u16, message: &str, module_name: &str, params: &Value, module_context: &str) -> String {
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: Entering handle_error for status_code: {}.", status_code);
        let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let status_text = status.canonical_reason().unwrap_or("Unknown Status");
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error: status_text: {}.", status_text);

        let mut render_context = Context::new();
        render_context.insert("status_code", &status_code);
        render_context.insert("status_text", &status_text);
        render_context.insert("message", message);
        let actual_module_name = if module_name.is_empty() { "Unknown Module" } else { module_name };
        render_context.insert("module_name", actual_module_name);
        render_context.insert("path", &params["request_path"]);
        render_context.insert("module_context", module_context);
        let params_string = serde_json::to_string_pretty(params).unwrap_or_else(|e| {
            module_log!(LogLevel::Error, "Failed to pretty-print params to string: {}", e);
            params.to_string() // Fallback to compact string if pretty-printing fails
        });
        render_context.insert("server_context", &params_string);

        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error: Context created.");

        let template_filename = format!("{}.jinja2", status_code);
        let template_path = self.content_root.join(&template_filename);

        let final_template_name;
        let html_content;

        if template_path.exists() {
            final_template_name = template_filename;
            module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error: Attempting to render template: {}.", final_template_name);
            let render_result = Tera::one_off(&std::fs::read_to_string(&template_path).unwrap_or_default(), &render_context, false);
            match render_result {
                Ok(html) => html_content = html,
                Err(e) => {
                    module_log!(LogLevel::Error, "ox_webservice_errorhandler_jinja2: handle_error: Failed to render template '{}': {}.", final_template_name, e);
                    // Fallback to a generic error message
                    return format!("<h1>{} {}</h1><p>{}</p>", status_code, status_text, message);
                }
            }
        } else {
            module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error: Template {} not found, checking for index.jinja2.", template_filename);
            final_template_name = "index.jinja2".to_string();
            let index_path = self.content_root.join(&final_template_name);

            if index_path.exists() {
                module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error: Attempting to render template: {}.", final_template_name);
                let render_result = Tera::one_off(&std::fs::read_to_string(&index_path).unwrap_or_default(), &render_context, false);
                match render_result {
                    Ok(html) => html_content = html,
                    Err(e) => {
                        module_log!(LogLevel::Error, "ox_webservice_errorhandler_jinja2: handle_error: Failed to render template '{}': {}.", final_template_name, e);
                        // Fallback to a generic error message
                        return format!("<h1>{} {}</h1><p>{}</p>", status_code, status_text, message);
                    }
                }
            }
            else {
                module_log!(LogLevel::Warn, "ox_webservice_errorhandler_jinja2: handle_error: Neither {} nor {} found. Returning generic error.", template_filename, final_template_name);
                return format!("<h1>{} {}</h1><p>{}</p>", status_code, status_text, message);
            }
        }
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error: Template rendered successfully.");
        html_content
    }
}

// ... (rest of the file)

#[derive(Debug, Deserialize)]
pub struct ErrorHandlerConfig {
    pub content_root: PathBuf,
}

// ... (some other code)

#[no_mangle]
pub extern "C" fn create_error_handler(module_config_ptr: *mut c_void, log_callback: LogCallback) -> *mut CErrorHandler {
    unsafe {
        LOGGER_CALLBACK = Some(log_callback);
    }

    let result = panic::catch_unwind(|| {
        let module_config = unsafe { &*(module_config_ptr as *mut ModuleConfig) };
        let params = match module_config.params.as_ref() {
            Some(p) => p,
            None => {
                module_log!(LogLevel::Error, "ox_webservice_errorhandler_jinja2: Module parameters are missing. config_file parameter is required.");
                return std::ptr::null_mut();
            }
        };

        let config_file_name = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                module_log!(LogLevel::Error, "ox_webservice_errorhandler_jinja2: 'config_file' parameter is missing or not a string.");
                return std::ptr::null_mut();
            }
        };
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: Attempting to read config file: {}", config_file_name);

        let contents = match std::fs::read_to_string(config_file_name) {
            Ok(c) => c,
            Err(e) => {
                module_log!(LogLevel::Error, "ox_webservice_errorhandler_jinja2: Failed to read error handler config file '{}': {}", config_file_name, e);
                return std::ptr::null_mut();
            }
        };
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: Config file content: {}", contents);

        let config: ErrorHandlerConfig = match serde_yaml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                module_log!(LogLevel::Error, "ox_webservice_errorhandler_jinja2: Failed to deserialize ErrorHandlerConfig: {}", e);
                return std::ptr::null_mut();
            }
        };
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: Parsed ErrorHandlerConfig: {:?}", config);

        let error_handler = match Jinja2ErrorHandler::new(config) {
            Ok(eh) => eh,
            Err(e) => {
                module_log!(LogLevel::Error, "ox_webservice_errorhandler_jinja2: Failed to create Jinja2ErrorHandler: {}", e);
                return std::ptr::null_mut();
            }
        };

        let c_error_handler = Box::new(CErrorHandler {
            instance_ptr: Box::into_raw(Box::new(error_handler)) as *mut c_void,
            handle_error_fn: handle_error_c,
        });

        Box::into_raw(c_error_handler)
    });

    match result {
        Ok(ptr) => ptr,
        Err(e) => {
            module_log!(LogLevel::Error, "Panic occurred in create_error_handler: {:?}. Returning null.", e);
            std::ptr::null_mut() // Return a null pointer on error
        }
    }
}





unsafe extern "C" fn handle_error_c(
    instance_ptr: *mut c_void,
    status_code: u16,
    message_ptr: *mut c_char,
    module_name_ptr: *mut c_char,
    params_ptr: *mut c_char,
    module_context_ptr: *mut c_char,
) -> *mut c_char {
    let result = panic::catch_unwind(|| {
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error_c: Inside catch_unwind closure.");
        let handler = &mut *(instance_ptr as *mut Jinja2ErrorHandler);
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error_c: Before message_ptr to_string_lossy.");
        let message = CStr::from_ptr(message_ptr).to_string_lossy().into_owned();
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error_c: Before module_name_ptr to_string_lossy.");
        let module_name = CStr::from_ptr(module_name_ptr).to_string_lossy().into_owned();
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error_c: Before params_ptr to_string_lossy.");
        let params_json = CStr::from_ptr(params_ptr).to_string_lossy().into_owned();
        let module_context_json = CStr::from_ptr(module_context_ptr).to_string_lossy().into_owned();
        
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error_c: Before serde_json::from_str(params_json).");
        let params: Value = serde_json::from_str(&params_json).unwrap_or_default();

        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error_c: Before calling handler.handle_error.");
        let html = handler.handle_error(status_code, &message, &module_name, &params, &module_context_json);
        
        module_log!(LogLevel::Debug, "ox_webservice_errorhandler_jinja2: handle_error_c: Before CString::new(html).");
        CString::new(html).unwrap().into_raw()
    });

    match result {
        Ok(ptr) => ptr,
        Err(_) => {
            let error_message = format!("<h1>Internal Server Error</h1><p>Panic occurred in error handler for status code {}.</p>", status_code);
            CString::new(error_message).unwrap().into_raw()
        }
    }
} 

#[no_mangle]
pub extern "C" fn destroy_error_handler(handler_ptr: *mut CErrorHandler) {
    if !handler_ptr.is_null() {
        unsafe {
            let boxed_c_error_handler = Box::from_raw(handler_ptr);
            // Recreate the Box for Jinja2ErrorHandler and let it drop
            let _ = Box::from_raw(boxed_c_error_handler.instance_ptr as *mut Jinja2ErrorHandler);
        }
    }
}
