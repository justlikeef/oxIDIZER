use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    CoreHostApi, PipelineState, AllocFn, AllocStrFn,
    ModuleStatus, FlowControl, ReturnParameters,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::panic;
use std::sync::Arc;

const MODULE_NAME: &str = "ox_webservice_errorhandler_json";

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")] // "append", "replace", "ignore"
pub enum Action {
    Append,
    Replace,
    Ignore,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_on_success")]
    pub on_success: Action,
    #[serde(default = "default_on_error")]
    pub on_error: Action,
}

fn default_on_success() -> Action { Action::Ignore }
fn default_on_error() -> Action { Action::Append }

pub struct OxModule<'a> {
    config: Config,
    api: &'a CoreHostApi,
}

impl<'a> OxModule<'a> {
    pub fn new(config: Config, api: &'a CoreHostApi) -> anyhow::Result<Self> {
        let _ = ox_webservice_api::init_logging(api.log_callback, MODULE_NAME);
        Ok(Self { config, api })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        
        // Use logic based on status code
        let status = pipeline_state.status_code;
        let is_error = status >= 400;
        
        let action = if is_error { self.config.on_error } else { self.config.on_success };

        if action == Action::Ignore {
             return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
            };
        }

        let mut context_map = serde_json::Map::new();
        context_map.insert("status".to_string(), serde_json::json!(status));
        
        // Get status text
        let status_text = axum::http::StatusCode::from_u16(status)
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
            .canonical_reason()
            .unwrap_or("Unknown Error");
        context_map.insert("message".to_string(), Value::String(status_text.to_string()));

        // If error, try to get more info (like from existing body if it's JSON?)
        // Or just wrap the existing body?
        // Logic:
        // Replace: New JSON body overwrites everything.
        // Append: New JSON body includes "original_body" field? Or merges?
        // User asked to "append successful http status to the existing body"
        
        // Let's assume the goal is to envelope the response.
        
        let mut final_json = context_map;

        if action == Action::Append {
            // Read existing body
            let existing_body_str = String::from_utf8_lossy(&pipeline_state.response_body);
            // Try parse as JSON
            if let Ok(existing_json) = serde_json::from_str::<Value>(&existing_body_str) {
                if let Some(obj) = existing_json.as_object() {
                    for (k, v) in obj {
                         final_json.insert(k.clone(), v.clone());
                    }
                } else {
                     final_json.insert("data".to_string(), existing_json);
                }
            } else {
                 // Text content
                 if !existing_body_str.is_empty() {
                     final_json.insert("content".to_string(), Value::String(existing_body_str.to_string()));
                 }
            }
        }

        // Set Content-Type
        pipeline_state.response_headers.insert(
            axum::http::header::CONTENT_TYPE, 
            axum::http::HeaderValue::from_static("application/json")
        );

        // Serialize and set body
        if let Ok(json_bytes) = serde_json::to_vec(&final_json) {
            pipeline_state.response_body = json_bytes;
            
            // Set flag
            pipeline_state.add_flag("error_handled");
            
            HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
            }
        } else {
            // Failed to serialize? Should not happen.
            HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
            }
        }
    }
}

// Boilerplate C-API
#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    _module_id: *const c_char,
    api: *const CoreHostApi,
) -> *mut ModuleInterface {
    let result = panic::catch_unwind(|| {
        let api_instance = unsafe { &*api };
        let module_params_json = unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap() };
        let params: Value = serde_json::from_str(module_params_json).unwrap_or(Value::Null);

        // Parse config from params directly or load file?
        // Usually params has "config_file".
        // For simplicity reusing params as config if no file, or creating default.
        let config: Config = serde_json::from_value(params.clone()).unwrap_or(Config {
            on_success: Action::Ignore,
            on_error: Action::Append,
        });

        let handler = OxModule::new(config, api_instance).unwrap();
        let instance_ptr = Box::into_raw(Box::new(handler)) as *mut c_void;

        let interface = Box::new(ModuleInterface {
            instance_ptr,
            handler_fn: process_request_c,
            log_callback: api_instance.log_callback,
            get_config: get_config_c,
        });
        Box::into_raw(interface)
    });
    match result { Ok(ptr) => ptr, Err(_) => std::ptr::null_mut() }
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    _log: LogCallback,
    _alloc: AllocFn,
    _arena: *const c_void,
) -> HandlerResult {
    let handler = unsafe { &*(instance_ptr as *mut OxModule) };
    handler.process_request(pipeline_state_ptr)
}

unsafe extern "C" fn get_config_c(
    instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    let handler = unsafe { &*(instance_ptr as *mut OxModule) };
    let json = serde_json::to_string(&handler.config).unwrap_or("{}".to_string());
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
}
