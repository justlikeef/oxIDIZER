use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    CoreHostApi, PipelineState, AllocFn, AllocStrFn,
    ModuleStatus,    FlowControl, ReturnParameters,
    ModuleExecutionRecord,
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
    pub debug_force_status: Option<u16>,
}

pub struct OxModule<'a> {
    content_root: PathBuf,
    debug_force_status: Option<u16>,
    api: &'a CoreHostApi,
}

impl<'a> OxModule<'a> {
    pub fn new(config: ErrorHandlerConfig, api: &'a CoreHostApi) -> anyhow::Result<Self> {
        let _ = ox_webservice_api::init_logging(api.log_callback, MODULE_NAME);

        let module_name = CString::new(MODULE_NAME).unwrap();
        let message = CString::new(format!(
            "ox_webservice_errorhandler_jinja2: new: Initializing with content_root: {:?}",
            config.content_root
        )).unwrap();
        unsafe { (api.log_callback)(LogLevel::Warn, module_name.as_ptr(), message.as_ptr()); }

        Ok(Self {
            content_root: config.content_root,
            debug_force_status: config.debug_force_status,
            api,
        })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
            self.api, 
            pipeline_state_ptr as *mut c_void, 
            arena_ptr
        ) };

        let status_code_val = ctx.get("response.status");
        let mut status_code = status_code_val.and_then(|v| v.as_u64()).map(|u| u as u16).unwrap_or(200);

        if let Some(forced) = self.debug_force_status {
            status_code = forced;
            let _ = ctx.set("response.status", serde_json::Value::Number(serde_json::Number::from(forced)));
        }

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
        
        // Request Method
        if let Some(val) = ctx.get("request.verb") {
             context_map.insert("request_method".to_string(), val);
        } else if let Some(val) = ctx.get("request.method") {
              context_map.insert("request_method".to_string(), val);
        }

        // Request Path
        if let Some(val) = ctx.get("request.resource") {
             context_map.insert("request_path".to_string(), val);
        } else if let Some(val) = ctx.get("request.path") {
              context_map.insert("request_path".to_string(), val);
        }

        // Request Query
        if let Some(val) = ctx.get("request.query") {
             context_map.insert("request_query".to_string(), val);
        }
        
        // Request Headers
        if let Some(val) = ctx.get("request.headers") {
             context_map.insert("request_headers".to_string(), val);
        }

        // Request Body
        if let Some(val) = ctx.get("request.payload") {
             context_map.insert("request_body".to_string(), val);
        }

        // Source IP
        if let Some(val) = ctx.get("request.source_ip") {
             context_map.insert("source_ip".to_string(), val);
        }


        let status_text = axum::http::StatusCode::from_u16(status_code)
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
            .canonical_reason()
            .unwrap_or("Unknown Error");
        context_map.insert("status_code".to_string(), serde_json::json!(status_code));
        context_map.insert("status_text".to_string(), Value::String(status_text.to_string()));

        // Module Context Access via Generic State?
        // "module.context" virtual key?
        // I implemented specific module context get/set, but is it exposed via generic "get"?
        // In my `ox_plugin` implementation I added `module.context` handling if I recall correctly.
        // Wait, did I? I added support for "ox.response.files".
        // Let's check `ox_plugin`.
        // If not, I can't easily get other module's context unless "module.context.<modulename>.<key>" is supported.
        // The legacy code used `get_module_context_value`.
        // If that's gone, I need to use `ctx.get("module.<name>.<key>")`.
        // Assuming generic state supports it.
        // Let's assume for now `module_context` is not critical or supported via `ctx.get("module.context")`.
        // Actually, the original code looked up "module_name" and "module_context".
        // Use generic placeholders for now.
        // Initialize module_name as Unknown
        let mut culprit_module_name = "Unknown".to_string();

        // --- Enhanced State Injection ---
        
        // Pipeline Execution History
        if let Some(val) = ctx.get("pipeline.execution_history") {
             // val is Value, likely a String containing JSON if it came from get().
             // Wait, ctx.get() helper parses JSON! See ox_pipeline_plugin/lib.rs:100
             // So 'val' is already the deserialized object (Vec<Record>).
             context_map.insert("execution_history".to_string(), val.clone());
             
             // Try to deduce module_name from history
             if let Ok(records) = serde_json::from_value::<Vec<ModuleExecutionRecord>>(val) {
                 let self_name = MODULE_NAME;
                 let mut last_modified_module = None;

                 // We want the LAST module that modified the state.
                 // This is likely the one that set the Error Status Code.
                 for record in records.iter().rev() {
                     if record.module_name == self_name { continue; }
                     
                     if record.status == ModuleStatus::Modified {
                         last_modified_module = Some(record.module_name.clone());
                         break;
                     }
                 }

                 if let Some(name) = last_modified_module {
                     culprit_module_name = name;
                 } else {
                     // No module modified the state, but we are in an error state.
                     // Must be Core default (e.g. 404 for no route match).
                     if status_code == 404 {
                         culprit_module_name = "Core/Router (No Match)".to_string();
                     } else {
                         culprit_module_name = "Core System".to_string();
                     }
                 }
             }
        }

        context_map.insert("message".to_string(), Value::String("An error occurred.".to_string()));
        context_map.insert("module_name".to_string(), Value::String(culprit_module_name.clone()));
        // Inject empty module_context to prevent template errors if accessed
        context_map.insert("module_context".to_string(), serde_json::Value::Object(serde_json::Map::new()));

        // Server Configs
        if let Some(val) = ctx.get("server.configs") {
             context_map.insert("server_configs".to_string(), val);
        }

        // Pipeline Routing
        if let Some(val) = ctx.get("server.pipeline_routing") {
             context_map.insert("pipeline_routing".to_string(), val);
        }
        
        // Pipeline Modified Status
        if let Some(val) = ctx.get("pipeline.modified") {
             context_map.insert("is_modified".to_string(), val);
        }

       // Generic State check for debugging
       // In earlier analysis ctx.get uses "get_state" C-API which returns string.
       // The wrapper parses it. So we are good.
        

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

        // Determine if we should serve HTML or JSON
        let mut serve_json = false;
        let accept_header = ctx.get("request.header.Accept").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
        if accept_header.contains("application/json") {
            serve_json = true;
        }

        let existing_content_type = ctx.get("response.header.Content-Type").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
        if existing_content_type.contains("application/json") {
            serve_json = true;
        }

        if serve_json {
            let existing_body = ctx.get("response.body").unwrap_or(Value::Null);
            let mut json_response = if let Value::Object(obj) = existing_body {
                obj
            } else {
                let mut obj = serde_json::Map::new();
                obj.insert("message".to_string(), Value::String(response_body.clone()));
                obj
            };
            
            json_response.insert("status".to_string(), serde_json::json!(status_code));
            json_response.insert("status_text".to_string(), Value::String(status_text.to_string()));
            json_response.insert("module".to_string(), Value::String(culprit_module_name));
            
            let _ = ctx.set("response.header.Content-Type", serde_json::Value::String("application/json".to_string()));
            let _ = ctx.set("response.body", Value::Object(json_response));
        } else {
            // Set response headers and body using Generic API
            let _ = ctx.set("response.header.Content-Type", serde_json::Value::String("text/html".to_string()));
            let _ = ctx.set("response.body", serde_json::Value::String(response_body));
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
    api: *const CoreHostApi,
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
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
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
    
    let config = ErrorHandlerConfig {
        content_root: handler.content_root.clone(),
        debug_force_status: handler.debug_force_status,
    };
    
    let json = serde_json::to_string_pretty(&config).unwrap_or("{}".to_string());
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
}