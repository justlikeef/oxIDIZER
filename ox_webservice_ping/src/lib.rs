use libc::{c_char, c_void};
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogCallback, LogLevel, ModuleInterface, PipelineState, 
    ModuleStatus, FlowControl, ReturnParameters,
    CoreHostApi, WebServiceApiV1
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
    result: String,
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
                flow_control: FlowControl::Halt,
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

        // Determine format
        let mut return_json = false;

        // Check accept header
        if let Some(accept) = ctx.get("http.request.header.accept") {
            if let Some(s) = accept.as_str() {
                if s.contains("application/json") {
                    return_json = true;
                }
            }
        }

        // Check query string
        if !return_json {
            if let Some(query_val) = ctx.get("http.request.query") {
                 if let Some(query) = query_val.as_str() {
                    if query.contains("format=json") {
                        return_json = true;
                    }
                 }
            }
        }

        let body_content;
        let content_type;

        if return_json {
            let response = PingResponse {
                result: "pong".to_string(),
            };
            body_content = serde_json::to_string(&response).unwrap_or(r#"{"result":"pong"}"#.to_string());
            content_type = "application/json";
        } else {
            body_content = "<html><body>result: pong</body></html>".to_string();
            content_type = "text/html";
        }
        
        self.log(LogLevel::Info, format!("Handling ping request. Returning JSON: {}", return_json));

        // Use ox_plugin set for response and header
        // Using "response" sugar setter if supported, or "http.response.body"
        // ox_webservice_pipeline handles "response" setter in generic state??
        // Wait, "set_state_c" in ox_webservice only handles "http.*" and context. 
        // OX_PLUGIN handles "response" helper via generic set? 
        // YES: ox_plugin::PluginContext::set("response", val) -> translates to FFI calls?
        // Let's check ox_plugin source I wrote.
        // I REMOVED the "response" logic from ox_plugin when I made it generic!
        // It now only calls `self.api.set_state`.
        // So I must use "http.*" keys directly OR re-implement the helper in `ox_plugin`.
        // User wants GENERIC. HTTP is Specific.
        // So "http.response.body" is the way.
        
        let _ = ctx.set("http.response.body", serde_json::Value::String(body_content));
        let _ = ctx.set("http.response.status", serde_json::json!(200));
        let _ = ctx.set("http.response.header.Content-Type", serde_json::Value::String(content_type.to_string()));

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
            flow_control: FlowControl::Halt,
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
                flow_control: FlowControl::Halt,
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
