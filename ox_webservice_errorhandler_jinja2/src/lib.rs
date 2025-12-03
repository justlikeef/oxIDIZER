use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    WebServiceApiV1, PipelineState, AllocFn,
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic;
use std::path::PathBuf;
use tera::{Context, Tera};
use bumpalo::Bump;

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
        if let Ok(c_message) = CString::new(format!(
            "ox_webservice_errorhandler_jinja2: new: Initializing with content_root: {:?}",
            config.content_root
        )) {
            unsafe { (api.log_callback)(LogLevel::Debug, c_message.as_ptr()); }
        }

        Ok(Self {
            content_root: config.content_root,
            api,
        })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let status_code = unsafe { (self.api.get_response_status)(pipeline_state) };

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
            let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;
            let c_str_method = (self.api.get_request_method)(pipeline_state, arena_ptr, self.api.alloc_str);
            render_context.insert(
                "request_method",
                &CStr::from_ptr(c_str_method).to_string_lossy().into_owned(),
            );

            let c_str_path = (self.api.get_request_path)(pipeline_state, arena_ptr, self.api.alloc_str);
            render_context.insert(
                "request_path",
                &CStr::from_ptr(c_str_path).to_string_lossy().into_owned(),
            );

            let c_str_query = (self.api.get_request_query)(pipeline_state, arena_ptr, self.api.alloc_str);
            render_context.insert(
                "request_query",
                &CStr::from_ptr(c_str_query).to_string_lossy().into_owned(),
            );

            let c_str_headers = (self.api.get_request_headers)(pipeline_state, arena_ptr, self.api.alloc_str);
            let headers_json = CStr::from_ptr(c_str_headers).to_string_lossy();
            let headers_value: Value = serde_json::from_str(&headers_json).unwrap_or_default();
            render_context.insert("request_headers", &headers_value);

            let c_str_body = (self.api.get_request_body)(pipeline_state, arena_ptr, self.api.alloc_str);
            render_context.insert(
                "request_body",
                &CStr::from_ptr(c_str_body).to_string_lossy().into_owned(),
            );

            let c_str_ip = (self.api.get_source_ip)(pipeline_state, arena_ptr, self.api.alloc_str);
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
                (self.api.get_module_context_value)(pipeline_state, module_name_key.as_ptr(), arena_ptr, self.api.alloc_str);
            let module_name_json = if !c_str_module_name.is_null() {
                CStr::from_ptr(c_str_module_name)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "\"Unknown\"".to_string()
            };
            let module_name: String = serde_json::from_str(&module_name_json).unwrap_or("Unknown".to_string());


            let c_str_module_context =
                (self.api.get_module_context_value)(pipeline_state, module_context_key.as_ptr(), arena_ptr, self.api.alloc_str);
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
                                    format!("Failed to render template \"{:?}\": {}", path, e),
                                );
                                "500 Internal Server Error".to_string()
                            }
                        }
                    }
                    Err(e) => {
                        self.log(
                            LogLevel::Error,
                            format!("Failed to read template file \"{:?}\": {}", path, e),
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
            let c_content_type_value = CString::new("text/html").unwrap();

            (self.api.set_response_header)(
                pipeline_state,
                c_content_type_key.as_ptr(),
                c_content_type_value.as_ptr(),
            );
            (self.api.set_response_body)(pipeline_state, c_body.as_ptr().cast(), c_body.as_bytes().len());
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
        let api_instance = unsafe { &*api }; 
        let module_params_json = unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap() }; 
        let params: Value =
            serde_json::from_str(module_params_json).expect("Failed to parse module params JSON");

        let config_file_name = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                let log_msg = CString::new("\"config_file\" parameter is missing or not a string.").unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let contents = match std::fs::read_to_string(config_file_name) {
            Ok(c) => c,
            Err(e) => {
                let log_msg = CString::new(format!("Failed to read config file \"{}\": {}", config_file_name, e)).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let config: ErrorHandlerConfig = match serde_yaml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                let log_msg = CString::new(format!("Failed to deserialize ErrorHandlerConfig: {}", e)).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let handler = match OxModule::new(config, *api_instance) {
            Ok(eh) => eh,
            Err(e) => {
                let log_msg = CString::new(format!("Failed to create OxModule: {}", e)).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let instance_ptr = Box::into_raw(Box::new(handler)) as *mut c_void;

        let module_interface = Box::new(ModuleInterface {
            instance_ptr,
            handler_fn: process_request_c,
            log_callback: api_instance.log_callback,
        });

        Box::into_raw(module_interface)
    });

    match result {
        Ok(ptr) => ptr,
        Err(e) => {
            eprintln!("Panic during module initialization: {:?}", e);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    log_callback: LogCallback,
    _alloc_raw_c: AllocFn, 
    _arena: *const c_void, 
) -> HandlerResult {
    let result = panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let handler = unsafe { &*(instance_ptr as *mut OxModule) };
        handler.process_request(pipeline_state_ptr)
    }));

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
            let log_msg =
                CString::new(format!("Panic occurred in process_request_c: {:?}.", e)).unwrap();
            unsafe { (log_callback)(LogLevel::Error, log_msg.as_ptr()); } 
            HandlerResult::ModifiedJumpToError
        }
    }
}