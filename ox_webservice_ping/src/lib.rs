use libc::{c_char, c_void};
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogCallback, LogLevel, ModuleInterface, PipelineState, 
    ModuleStatus, FlowControl, ReturnParameters,
    CoreHostApi
};
// Use ox_plugin directly? ox_webservice_api re-exports it.
use serde::Serialize;
use std::ffi::{CStr, CString};
use std::panic;
use std::ptr;
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_webservice_ping";

#[derive(Serialize)]
struct PingResponse {
    response: String,
}

pub struct OxModule {
    api: &'static CoreHostApi,
    module_id: String,
}

impl OxModule {
    pub fn new(api: &'static CoreHostApi, module_id: String) -> Self {
        let _ = ox_webservice_api::init_logging(api.log_callback, &module_id);
        Self { api, module_id }
    }

    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(self.module_id.clone()).unwrap_or(CString::new(MODULE_NAME).unwrap());
            unsafe {
                (self.api.log_callback)(level, module_name.as_ptr(), c_message.as_ptr());
            }
        }
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        if pipeline_state_ptr.is_null() {
            self.log(LogLevel::Error, "Pipeline state is null".to_string());
             return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
             };
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        // Initialize PluginContext (Generic)
        // state_ptr is treated as *mut c_void by PluginContext/CoreHostApi
        let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
            self.api, 
            pipeline_state_ptr as *mut c_void, 
            arena_ptr
        ) };

        // Content Negotiation
        let verb = ctx.get("request.verb").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or("get".to_string());
        let default_format = if verb == "stream" { "json" } else { "html" };
        let format = ctx.get("request.format").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or(default_format.to_string());

        let (body_content, content_type) = if format == "html" {
            ("<html><body><h1>response: pong</h1></body></html>".to_string(), "text/html")
        } else {
            let response = PingResponse {
                response: "pong".to_string(),
            };
            (serde_json::to_string(&response).unwrap_or(r#"{"response":"pong"}"#.to_string()), "application/json")
        };

        self.log(LogLevel::Info, format!("Handling ping request (format: {})", format));

        // Set Generic Keys (Transport Agnostic)
        let _ = ctx.set("response.body", serde_json::Value::String(body_content));
        let _ = ctx.set("response.status", serde_json::json!(200));
        let _ = ctx.set("response.type", serde_json::Value::String(content_type.to_string()));

        // Set HTTP specific keys (Legacy/Fallback)
        // Removed as pipeline no longer supports them and they are redundant
        // let _ = ctx.set("http.response.body", serde_json::Value::String(body_content));
        // let _ = ctx.set("http.response.status", serde_json::json!(200));
        // let _ = ctx.set("http.response.header.Content-Type", serde_json::Value::String(content_type.to_string()));

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
    _module_params_json_ptr: *const c_char,
    module_id_ptr: *const c_char,
    api_ptr: *const CoreHostApi, // Generic API
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };
    
    let module_id = if !module_id_ptr.is_null() {
        CStr::from_ptr(module_id_ptr).to_string_lossy().to_string()
    } else {
        MODULE_NAME.to_string()
    };
    let c_module_id = CString::new(module_id.clone()).unwrap();

    // Log initialization
    if let Ok(c_message) = CString::new("ox_webservice_ping initialized") {
        unsafe { (api_instance.log_callback)(LogLevel::Info, c_module_id.as_ptr(), c_message.as_ptr()); }
    }

    let result = panic::catch_unwind(|| {
        // Safe as CoreHostApi has static lifetime for lifetime of module
        let module = OxModule::new(std::mem::transmute(api_instance), module_id);
        let instance_ptr = Box::into_raw(Box::new(module)) as *mut c_void;

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
        Err(_) => ptr::null_mut(),
    }
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void,
) -> HandlerResult {
    if instance_ptr.is_null() {
        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        };
    }

    let result = panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let handler = unsafe { &*(instance_ptr as *mut OxModule) };
        handler.process_request(pipeline_state_ptr)
    }));

    match result {
        Ok(handler_result) => handler_result,
        Err(e) => {
             let log_msg = CString::new(format!("Panic in ox_webservice_ping: {:?}", e)).unwrap();
             
             let handler_unsafe = unsafe { &*(instance_ptr as *mut OxModule) };
             let module_name = CString::new(handler_unsafe.module_id.clone()).unwrap_or(CString::new(MODULE_NAME).unwrap());
             
             unsafe { (log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
            HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            }
        }
    }
}

unsafe extern "C" fn get_config_c(
    _instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    let json = "null";
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
}

#[cfg(test)]
mod tests;
