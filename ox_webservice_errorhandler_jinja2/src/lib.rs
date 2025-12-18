use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    WebServiceApiV1, PipelineState, AllocFn, AllocStrFn,
    ModuleStatus, FlowControl, Phase, ReturnParameters,
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic;
use std::path::PathBuf;
use std::error::Error;
use tera::{Context, Tera};
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_webservice_errorhandler_jinja2";

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct ErrorHandlerConfig {
    pub content_root: PathBuf,
}

pub struct OxModule<'a> {
    content_root: PathBuf,
    api: &'a WebServiceApiV1,
}

impl<'a> OxModule<'a> {
    pub fn new(config: ErrorHandlerConfig, api: &'a WebServiceApiV1) -> anyhow::Result<Self> {
        let _ = ox_webservice_api::init_logging(api.log_callback, MODULE_NAME);

        let module_name = CString::new(MODULE_NAME).unwrap();
        let message = CString::new(format!(
            "ox_webservice_errorhandler_jinja2: new: Initializing with content_root: {:?}",
            config.content_root
        )).unwrap();
        unsafe { (api.log_callback)(LogLevel::Debug, module_name.as_ptr(), message.as_ptr()); }

        Ok(Self {
            content_root: config.content_root,
            api,
        })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let status_code = unsafe { (self.api.get_response_status)(pipeline_state) };

        if status_code < 400 {
            return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            };
        }

        let module_name = CString::new(MODULE_NAME).unwrap();
        let message = CString::new(format!(
            "ox_webservice_errorhandler_jinja2: Handling error request with status code: {}",
            status_code
        )).unwrap();
        unsafe { (self.api.log_callback)(LogLevel::Debug, module_name.as_ptr(), message.as_ptr()); }

        let mut context_map = serde_json::Map::new();
        unsafe {
            let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;
            let c_str_method = (self.api.get_request_method)(pipeline_state, arena_ptr, self.api.alloc_str);
            context_map.insert(
                "request_method".to_string(),
                Value::String(CStr::from_ptr(c_str_method).to_string_lossy().into_owned()),
            );

            let c_str_path = (self.api.get_request_path)(pipeline_state, arena_ptr, self.api.alloc_str);
            context_map.insert(
                "request_path".to_string(),
                Value::String(CStr::from_ptr(c_str_path).to_string_lossy().into_owned()),
            );

            let c_str_query = (self.api.get_request_query)(pipeline_state, arena_ptr, self.api.alloc_str);
            context_map.insert(
                "request_query".to_string(),
                Value::String(CStr::from_ptr(c_str_query).to_string_lossy().into_owned()),
            );

            let c_str_headers = (self.api.get_request_headers)(pipeline_state, arena_ptr, self.api.alloc_str);
            let headers_json = CStr::from_ptr(c_str_headers).to_string_lossy();
            let headers_value: Value = serde_json::from_str(&headers_json).unwrap_or_default();
            context_map.insert("request_headers".to_string(), headers_value);

            let c_str_body = (self.api.get_request_body)(pipeline_state, arena_ptr, self.api.alloc_str);
            context_map.insert(
                "request_body".to_string(),
                Value::String(CStr::from_ptr(c_str_body).to_string_lossy().into_owned()),
            );

            let c_str_ip = (self.api.get_source_ip)(pipeline_state, arena_ptr, self.api.alloc_str);
            context_map.insert(
                "source_ip".to_string(),
                Value::String(CStr::from_ptr(c_str_ip).to_string_lossy().into_owned()),
            );

            let status_text = axum::http::StatusCode::from_u16(status_code)
                .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .canonical_reason()
                .unwrap_or("Unknown Error");
            context_map.insert("status_code".to_string(), serde_json::json!(status_code));
            context_map.insert("status_text".to_string(), Value::String(status_text.to_string()));

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
            let module_context_val: Value = if !c_str_module_context.is_null() {
                 let json_str = CStr::from_ptr(c_str_module_context).to_string_lossy();
                 serde_json::from_str(&json_str).unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            
            context_map.insert("message".to_string(), Value::String("An error occurred.".to_string()));
            context_map.insert("module_name".to_string(), Value::String(module_name));
            context_map.insert("module_context".to_string(), module_context_val);
        }

        let module_name_c = CString::new(MODULE_NAME).unwrap();
        // Log the full context for debugging purposes
        if let Ok(context_json) = serde_json::to_string(&context_map) {
             let message = CString::new(format!("Render Context: {}", context_json)).unwrap_or_default();
             unsafe { (self.api.log_callback)(LogLevel::Debug, module_name_c.as_ptr(), message.as_ptr()); }
        } else {
             let message = CString::new("Failed to serialize render context").unwrap();
             unsafe { (self.api.log_callback)(LogLevel::Warn, module_name_c.as_ptr(), message.as_ptr()); }
        }

        let render_context = Context::from_value(Value::Object(context_map)).unwrap_or_else(|_| Context::new());

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
                let module_name = CString::new(MODULE_NAME).unwrap();
                let message = CString::new(format!("Attempting to render error template: {:?}", path)).unwrap();
                unsafe { (self.api.log_callback)(LogLevel::Debug, module_name.as_ptr(), message.as_ptr()); }

                match std::fs::read_to_string(&path) {
                    Ok(template_str) => {
                        match Tera::one_off(&template_str, &render_context, false) {
                            Ok(html) => html,
                            Err(e) => {
                                let mut logged_as_missing_var = false;
                                let error_desc = e.to_string();
                                let source_desc = e.source().map(|s| s.to_string()).unwrap_or_default();
                                
                                if error_desc.contains("not found in context") || source_desc.contains("not found in context") {
                                    let module_name = CString::new(MODULE_NAME).unwrap();
                                    let message = CString::new(format!("Missing variable in template \"{:?}\": {}", path, source_desc)).unwrap();
                                    unsafe { (self.api.log_callback)(LogLevel::Info, module_name.as_ptr(), message.as_ptr()); }
                                    logged_as_missing_var = true;
                                }

                                if !logged_as_missing_var {
                                    let module_name = CString::new(MODULE_NAME).unwrap();
                                    let message = CString::new(format!("Failed to render template \"{:?}\": {}", path, e)).unwrap();
                                    unsafe { (self.api.log_callback)(LogLevel::Error, module_name.as_ptr(), message.as_ptr()); }
                                }
                                "500 Internal Server Error".to_string()
                            }
                        }
                    }
                    Err(e) => {
                        let module_name = CString::new(MODULE_NAME).unwrap();
                        let message = CString::new(format!("Failed to read template file \"{:?}\": {}", path, e)).unwrap();
                        unsafe { (self.api.log_callback)(LogLevel::Error, module_name.as_ptr(), message.as_ptr()); }
                        "500 Internal Server Error".to_string()
                    }
                }
            }
            None => {
                let module_name = CString::new(MODULE_NAME).unwrap();
                let message = CString::new(format!("No specific error template found for status {}. No index.jinja2 fallback found. Falling back to default text response.", status_code)).unwrap();
                unsafe { (self.api.log_callback)(LogLevel::Warn, module_name.as_ptr(), message.as_ptr()); }
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

        HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    _module_id: *const c_char,
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
                let module_name = CString::new(MODULE_NAME).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let config_path = PathBuf::from(config_file_name);
        
        let config: ErrorHandlerConfig = match ox_fileproc::process_file(&config_path, 5) {
            Ok(value) => match serde_json::from_value(value) {
                Ok(c) => c,
                Err(e) => {
                     let log_msg = CString::new(format!("Failed to deserialize ErrorHandlerConfig: {}", e)).unwrap();
                     let module_name = CString::new(MODULE_NAME).unwrap();
                     unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                     return std::ptr::null_mut();
                }
            },
            Err(e) => {
                 let log_msg = CString::new(format!("Failed to process config file '{}': {}", config_file_name, e)).unwrap();
                 let module_name = CString::new(MODULE_NAME).unwrap();
                 unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                 return std::ptr::null_mut();
            }
        };

        let handler = match OxModule::new(config, api_instance) {
            Ok(eh) => eh,
            Err(e) => {
                let log_msg = CString::new(format!("Failed to create OxModule: {}", e)).unwrap();
                let module_name = CString::new(MODULE_NAME).unwrap();
                unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
                return std::ptr::null_mut();
            }
        };

        let instance_ptr = Box::into_raw(Box::new(handler)) as *mut c_void;

        let module_interface = Box::new(ModuleInterface {
            instance_ptr,
            handler_fn: process_request_c,
            log_callback: api_instance.log_callback,
            get_config: get_config_c,
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
                format!("Panic occurred in process_request_c: {:?}.", e);
            let c_log_msg = CString::new(log_msg).unwrap();
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe { (log_callback)(LogLevel::Error, module_name.as_ptr(), c_log_msg.as_ptr()); } 
            HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::JumpTo,
                return_parameters: ReturnParameters {
                    return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                },
            }
        }
    }

    }


unsafe extern "C" fn get_config_c(
    instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    if instance_ptr.is_null() { return std::ptr::null_mut(); }
    let handler = unsafe { &*(instance_ptr as *mut OxModule) };
    
    // Config struct is private in lib? No, OxModule stores config.
    // OxModule struct:
    // pub struct OxModule<'a> {
    //     content_root: PathBuf,
    //     api: &'a WebServiceApiV1,
    // }
    // It doesn't store the full `ErrorHandlerConfig` struct, just fields.
    // I can reconstruct a serializable object.
    
    let config = ErrorHandlerConfig {
        content_root: handler.content_root.clone(),
    };
    
    let json = serde_json::to_string_pretty(&config).unwrap_or("{}".to_string());
    alloc_fn(arena, CString::new(json).unwrap().as_ptr())
}