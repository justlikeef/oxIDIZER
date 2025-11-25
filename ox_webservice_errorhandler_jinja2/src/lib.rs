use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface, RequestContext,
    WebServiceApiV1,
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic;
use std::path::PathBuf;
use tera::{Context, Tera};

#[derive(Debug, Deserialize)]
pub struct ErrorHandlerConfig {
    pub content_root: PathBuf,
}

pub struct OxModule {
    content_root: PathBuf,
    api: WebServiceApiV1,
}

impl OxModule {
    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            unsafe {
                (self.api.log_callback)(level, c_message.as_ptr());
            }
        }
    }

    pub fn new(config: ErrorHandlerConfig, api: WebServiceApiV1) -> anyhow::Result<Self> {
        let temp_logger = |level, msg: String| {
            if let Ok(c_message) = CString::new(msg) {
                unsafe { (api.log_callback)(level, c_message.as_ptr()); }
            }
        };
        temp_logger(
            LogLevel::Debug,
            format!(
                "ox_webservice_errorhandler_jinja2: new: Initializing with content_root: {:?}",
                config.content_root
            ),
        );

        Ok(Self {
            content_root: config.content_root,
            api,
        })
    }

    pub fn process_request(&self, context: &mut RequestContext) -> HandlerResult {
        let status_code = unsafe { (self.api.get_response_status)(context) };

        if status_code < 400 {
            return HandlerResult::UnmodifiedContinue;
        }

        self.log(
            LogLevel::Debug,
            format!(
                "ox_webservice_errorhandler_jinja2: Handling error request with status code: {}",
                status_code
            ),
        );

        let mut render_context = Context::new();
        unsafe {
            let c_str_method = (self.api.get_request_method)(context);
            render_context.insert(
                "request_method",
                &CStr::from_ptr(c_str_method).to_string_lossy().into_owned(),
            );

            let c_str_path = (self.api.get_request_path)(context);
            render_context.insert(
                "request_path",
                &CStr::from_ptr(c_str_path).to_string_lossy().into_owned(),
            );

            let c_str_query = (self.api.get_request_query)(context);
            render_context.insert(
                "request_query",
                &CStr::from_ptr(c_str_query).to_string_lossy().into_owned(),
            );

            let c_str_headers = (self.api.get_request_headers)(context);
            let headers_json = CStr::from_ptr(c_str_headers).to_string_lossy();
            let headers_value: Value = serde_json::from_str(&headers_json).unwrap_or_default();
            render_context.insert("request_headers", &headers_value);

            let c_str_body = (self.api.get_request_body)(context);
            render_context.insert(
                "request_body",
                &CStr::from_ptr(c_str_body).to_string_lossy().into_owned(),
            );

            let c_str_ip = (self.api.get_source_ip)(context);
            render_context.insert(
                "source_ip",
                &CStr::from_ptr(c_str_ip).to_string_lossy().into_owned(),
            );

            let status_text = axum::http::StatusCode::from_u16(status_code)
                .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .canonical_reason()
                .unwrap_or("Unknown Error");
            render_context.insert("status_code", &status_code);
            render_context.insert("status_text", &status_text);

            let module_name_key = CString::new("module_name").unwrap();
            let module_context_key = CString::new("module_context").unwrap();

            let c_str_module_name =
                (self.api.get_module_context_value)(context, module_name_key.as_ptr());
            let module_name_json = if !c_str_module_name.is_null() {
                CStr::from_ptr(c_str_module_name)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "\"Unknown\"".to_string()
            };
            let module_name: String = serde_json::from_str(&module_name_json).unwrap_or("Unknown".to_string());


            let c_str_module_context =
                (self.api.get_module_context_value)(context, module_context_key.as_ptr());
            let module_context = if !c_str_module_context.is_null() {
                CStr::from_ptr(c_str_module_context)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "{}".to_string()
            };

            render_context.insert("message", "An error occurred.");
            render_context.insert("module_name", &module_name);
            render_context.insert("module_context", &module_context);
        }

        let status_template_path = self.content_root.join(format!("{}.jinja2", status_code));
        let index_template_path = self.content_root.join("index.jinja2");

        let template_to_use = if status_template_path.exists() {
            Some(status_template_path)
        } else if index_template_path.exists() {
            Some(index_template_path)
        } else {
            None
        };

        let response_body = match template_to_use {
            Some(path) => {
                self.log(
                    LogLevel::Debug,
                    format!("Attempting to render error template: {:?}", path),
                );
                match std::fs::read_to_string(&path) {
                    Ok(template_str) => {
                        match Tera::one_off(&template_str, &render_context, false) {
                            Ok(html) => html,
                            Err(e) => {
                                self.log(
                                    LogLevel::Error,
                                    format!("Failed to render template '{:?}': {}", path, e),
                                );
                                "500 Internal Server Error".to_string()
                            }
                        }
                    }
                    Err(e) => {
                        self.log(
                            LogLevel::Error,
                            format!("Failed to read template file '{:?}': {}", path, e),
                        );
                        "500 Internal Server Error".to_string()
                    }
                }
            }
            None => {
                self.log(LogLevel::Warn, format!("No specific error template found for status {}. No index.jinja2 fallback found. Falling back to default text response.", status_code));
                let reason = axum::http::StatusCode::from_u16(status_code)
                    .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .canonical_reason()
                    .unwrap_or("Internal Server Error");
                format!("{} {}", status_code, reason)
            }
        };

        unsafe {
            let c_body = CString::new(response_body).unwrap();
            let c_content_type_key = CString::new("Content-Type").unwrap();
            let c_content_type_value = CString::new("text/plain").unwrap();

            (self.api.set_response_header)(
                context,
                c_content_type_key.as_ptr(),
                c_content_type_value.as_ptr(),
            );
            (self.api.set_response_body)(context, c_body.as_ptr().cast(), c_body.as_bytes().len());
        }

        HandlerResult::ModifiedContinue
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    api: *const WebServiceApiV1,
) -> *mut ModuleInterface {
    let result = panic::catch_unwind(|| {
        let api = unsafe { &*api }; // Correctly dereference api pointer
        let temp_logger = |level, msg: String| {
            if let Ok(c_message) = CString::new(msg) {
                unsafe { (api.log_callback)(level, c_message.as_ptr()); } // Correctly use api.log_callback
            }
        };

        let module_params_json = unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap() }; // Wrap in unsafe
        let params: Value =
            serde_json::from_str(module_params_json).expect("Failed to parse module params JSON");

        let config_file_name = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                temp_logger(
                    LogLevel::Error,
                    "'config_file' parameter is missing or not a string.".to_string(),
                );
                return std::ptr::null_mut();
            }
        };

        let contents = match std::fs::read_to_string(config_file_name) {
            Ok(c) => c,
            Err(e) => {
                temp_logger(
                    LogLevel::Error,
                    format!("Failed to read config file '{}': {}", config_file_name, e),
                );
                return std::ptr::null_mut();
            }
        };

        let config: ErrorHandlerConfig = match serde_yaml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                temp_logger(
                    LogLevel::Error,
                    format!("Failed to deserialize ErrorHandlerConfig: {}", e),
                );
                return std::ptr::null_mut();
            }
        };

        let handler = match OxModule::new(config, *api) {
            Ok(eh) => eh,
            Err(e) => {
                temp_logger(
                    LogLevel::Error,
                    format!("Failed to create OxModule: {}", e),
                );
                return std::ptr::null_mut();
            }
        };

        let instance_ptr = Box::into_raw(Box::new(handler)) as *mut c_void;

        let module_interface = Box::new(ModuleInterface {
            instance_ptr,
            handler_fn: process_request_c,
            log_callback: api.log_callback,
        });

        Box::into_raw(module_interface)
    });

    match result {
        Ok(ptr) => ptr,
        Err(e) => {
            // Cannot safely log here as we might not have the api pointer.
            eprintln!("Panic during module initialization: {:?}", e);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    context_ptr: *mut RequestContext,
    log_callback: LogCallback,
) -> HandlerResult {
    let result = panic::catch_unwind(|| {
        let handler = unsafe { &*(instance_ptr as *mut OxModule) };
        let context = unsafe { &mut *context_ptr };
        handler.process_request(context)
    });

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
            let log_msg =
                CString::new(format!("Panic occurred in process_request_c: {:?}.", e)).unwrap();
            unsafe { (log_callback)(LogLevel::Error, log_msg.as_ptr()); } // Correctly wrap in unsafe
            HandlerResult::ModifiedJumpToError
        }
    }
}
