use libc::{c_char, c_void};
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogCallback, LogLevel, ModuleInterface, PipelineState, WebServiceApiV1,
    ModuleStatus, FlowControl, ReturnParameters, Phase,
};
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
    api: &'static WebServiceApiV1,
}

impl OxModule {
    pub fn new(api: &'static WebServiceApiV1) -> Self {
        Self { api }
    }

    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(MODULE_NAME).unwrap();
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
                flow_control: FlowControl::JumpTo,
                return_parameters: ReturnParameters {
                    return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                },
             };
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        // Determine format
        let mut return_json = false;

        // Check Accept header
        let accept_header_key = CString::new("Accept").unwrap();
        let accept_header_ptr = unsafe {
            (self.api.get_request_header)(
                pipeline_state,
                accept_header_key.as_ptr(),
                arena_ptr,
                self.api.alloc_str,
            )
        };

        if !accept_header_ptr.is_null() {
            let accept_header = unsafe { CStr::from_ptr(accept_header_ptr).to_str().unwrap_or("") };
            if accept_header.contains("application/json") {
                return_json = true;
            }
        }

        // Check query string
        if !return_json {
            let query_ptr = unsafe {
                (self.api.get_request_query)(pipeline_state, arena_ptr, self.api.alloc_str)
            };
            if !query_ptr.is_null() {
                let query = unsafe { CStr::from_ptr(query_ptr).to_str().unwrap_or("") };
                if query.contains("format=json") {
                    return_json = true;
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

        unsafe {
            let ct_k = CString::new("Content-Type").unwrap();
            let ct_v = CString::new(content_type).unwrap();
            (self.api.set_response_header)(pipeline_state, ct_k.as_ptr(), ct_v.as_ptr());

            (self.api.set_response_status)(pipeline_state, 200);

            (self.api.set_response_body)(
                pipeline_state,
                body_content.as_ptr(),
                body_content.len(),
            );
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
    _module_params_json_ptr: *const c_char, // Unused
    _module_id: *const c_char,
    api_ptr: *const WebServiceApiV1,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };
    
    // Log initialization
    if let Ok(c_message) = CString::new("ox_webservice_ping initialized") {
        let module_name = CString::new(MODULE_NAME).unwrap();
        unsafe { (api_instance.log_callback)(LogLevel::Info, module_name.as_ptr(), c_message.as_ptr()); }
    }

    let result = panic::catch_unwind(|| {
        let module = OxModule::new(api_instance);
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
            flow_control: FlowControl::JumpTo,
            return_parameters: ReturnParameters {
                return_data: (Phase::ErrorHandling as usize) as *mut c_void,
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
             let module_name = CString::new(MODULE_NAME).unwrap();
             unsafe { (log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
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
    _instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    let json = "null";
    alloc_fn(arena, CString::new(json).unwrap().as_ptr())
}
