use libc::{c_void, c_char};
use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    WebServiceApiV1, AllocFn, AllocStrFn, PipelineState,
    ModuleStatus, FlowControl, Phase, ReturnParameters,
};
use std::ffi::{CStr, CString};
use std::panic;
use anyhow::Result;
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_webservice_forwarded_for";

pub struct OxModule<'a> {
    api: &'a WebServiceApiV1,
}

impl<'a> OxModule<'a> {
    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe {
                (self.api.log_callback)(level, module_name.as_ptr(), c_message.as_ptr());
            }
        }
    }

    pub fn new(api: &'a WebServiceApiV1) -> Result<Self> {
        Ok(Self { api })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        if pipeline_state_ptr.is_null() {
            self.log(LogLevel::Error, "Pipeline state is null".to_string());
            // Safe fallback
            return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::JumpTo,
                return_parameters: ReturnParameters {
                    return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                },
            };
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };

        // 1. Get X-Forwarded-For header
        let header_key = CString::new("x-forwarded-for").unwrap();
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;
        
        // Use Host get_request_header
        let header_val_ptr = unsafe {
            (self.api.get_request_header)(
                pipeline_state,
                header_key.as_ptr(),
                arena_ptr,
                self.api.alloc_str
            )
        };

        if header_val_ptr.is_null() {
            // Header not present, do nothing
            return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            };
        }

        let header_val = unsafe { CStr::from_ptr(header_val_ptr).to_string_lossy() };
        if header_val.is_empty() {
             return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            };
        }

        // 2. Parse the FIRST IP from the list (standard practice)
        // X-Forwarded-For: <client>, <proxy1>, <proxy2>
        let new_client_ip = match header_val.split(',').next() {
            Some(ip) => ip.trim().to_string(),
            None => {
                 return HandlerResult {
                    status: ModuleStatus::Unmodified,
                    flow_control: FlowControl::Continue,
                    return_parameters: ReturnParameters {
                        return_data: std::ptr::null_mut(),
                    },
                };
            }
        };

        // 3. Store the *original* source IP in module context for potential restoration
        // Get current source IP
        let current_ip_ptr = unsafe { (self.api.get_source_ip)(pipeline_state, arena_ptr, self.api.alloc_str) };
        let current_ip = if !current_ip_ptr.is_null() {
            unsafe { CStr::from_ptr(current_ip_ptr).to_string_lossy().into_owned() }
        } else {
            "unknown".to_string()
        };

        let ctx_key = CString::new("original_source_ip").unwrap();
        // We act like we are storing a JSON string, so quote it? Or just raw string?
        // The restore module expects a JSON string, so we should serialize it.
        let val_json = serde_json::to_string(&current_ip).unwrap_or(format!("\"{}\"", current_ip));
        let ctx_val = CString::new(val_json).unwrap();

        unsafe {
            (self.api.set_module_context_value)(pipeline_state, ctx_key.as_ptr(), ctx_val.as_ptr());
        }

        // 4. Update the Source IP in PipelineState
        let new_ip_c = match CString::new(new_client_ip.clone()) {
            Ok(s) => s,
            Err(_) => return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            },
        };

        unsafe {
            (self.api.set_source_ip)(pipeline_state, new_ip_c.as_ptr());
        }

        self.log(LogLevel::Info, format!("Updated Source IP from {} to {} based on X-Forwarded-For: {}", current_ip, new_client_ip, header_val));

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
    _module_id: *const c_char,
    api_ptr: *const WebServiceApiV1,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };

    // We expect no critical params, but we should consume the ptr safely if needed.
    // Basic verification of params presence could be added here if config is needed later.

    let glue = match OxModule::new(api_instance) {
        Ok(g) => g,
        Err(e) => {
             let log_msg = CString::new(format!("Failed to initialize: {}", e)).unwrap();
             let module_name = CString::new(MODULE_NAME).unwrap();
             unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
             return std::ptr::null_mut();
        }
    };
    
    // Log initialization
    let log_msg = CString::new("ox_webservice_forwarded_for initialized").unwrap();
    let module_name = CString::new(MODULE_NAME).unwrap();
    unsafe { (api_instance.log_callback)(LogLevel::Info, module_name.as_ptr(), log_msg.as_ptr()); }


    let instance_ptr = Box::into_raw(Box::new(glue)) as *mut c_void;

    Box::into_raw(Box::new(ModuleInterface {
        instance_ptr,
        handler_fn: process_request_c,
        log_callback: api_instance.log_callback,
        get_config: get_config_c,
    }))
}

unsafe extern "C" fn get_config_c(
    _instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    let json = "null";
    alloc_fn(arena, CString::new(json).unwrap().as_ptr())
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
        Ok(r) => r,
        Err(e) => {
             let log_msg = CString::new(format!("Panic in ox_webservice_forwarded_for: {:?}", e)).unwrap();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ptr;
    use std::ffi::CStr;
    use ox_webservice_api::{PipelineState, WebServiceApiV1, LogLevel};

    thread_local! {
        static MOCK_HEADERS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
        static MOCK_SOURCE_IP: RefCell<String> = RefCell::new("127.0.0.1".to_string());
        static MOCK_CONTEXT: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    }

    unsafe extern "C" fn mock_log(_level: LogLevel, _module: *const c_char, _msg: *const c_char) {}

    unsafe extern "C" fn mock_alloc_str(_arena: *const c_void, s: *const c_char) -> *mut c_char {
        let c_str = unsafe { CStr::from_ptr(s) };
        let s = c_str.to_string_lossy();
        let c_string = CString::new(s.into_owned()).unwrap();
        c_string.into_raw()
    }

    unsafe extern "C" fn mock_get_header(_ps: *mut PipelineState, key: *const c_char, _arena: *const c_void, _alloc: AllocFn) -> *mut c_char {
        let key_str = unsafe { CStr::from_ptr(key).to_string_lossy() };
        let result = MOCK_HEADERS.with(|h| h.borrow().get(key_str.as_ref()).cloned());
        match result {
            Some(v) => CString::new(v).unwrap().into_raw(),
            None => ptr::null_mut(),
        }
    }

    unsafe extern "C" fn mock_get_source_ip(_ps: *mut PipelineState, _arena: *const c_void, _alloc: AllocFn) -> *mut c_char {
        let ip = MOCK_SOURCE_IP.with(|ip| ip.borrow().clone());
        CString::new(ip).unwrap().into_raw()
    }

    unsafe extern "C" fn mock_set_source_ip(_ps: *mut PipelineState, ip: *const c_char) {
        let ip_str = unsafe { CStr::from_ptr(ip).to_string_lossy() };
        MOCK_SOURCE_IP.with(|ip| *ip.borrow_mut() = ip_str.into_owned());
    }

    unsafe extern "C" fn mock_set_context(_ps: *mut PipelineState, key: *const c_char, val_json: *const c_char) {
        let key_str = unsafe { CStr::from_ptr(key).to_string_lossy() };
        let val_str = unsafe { CStr::from_ptr(val_json).to_string_lossy() };
        MOCK_CONTEXT.with(|ctx| ctx.borrow_mut().insert(key_str.into_owned(), val_str.into_owned()));
    }

    unsafe extern "C" fn mock_alloc_raw(_a: *mut c_void, _s: usize, _align: usize) -> *mut c_void { ptr::null_mut() }
    unsafe extern "C" fn mock_get_ctx(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_method(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_path(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_query(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_headers(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_body(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_set_req_path(_ps: *mut PipelineState, _p: *const c_char) { }
    unsafe extern "C" fn mock_set_req_header(_ps: *mut PipelineState, _k: *const c_char, _v: *const c_char) { }
    unsafe extern "C" fn mock_get_resp_status(_ps: *mut PipelineState) -> u16 { 200 }
    unsafe extern "C" fn mock_get_resp_header(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_set_resp_status(_ps: *mut PipelineState, _s: u16) { }
    unsafe extern "C" fn mock_set_resp_header(_ps: *mut PipelineState, _k: *const c_char, _v: *const c_char) { }
    unsafe extern "C" fn mock_set_resp_body(_ps: *mut PipelineState, _b: *const u8, _l: usize) { }


    fn make_mock_api() -> WebServiceApiV1 {
        WebServiceApiV1 {
            log_callback: mock_log,
            alloc_str: mock_alloc_str,
            alloc_raw: mock_alloc_raw,
            get_module_context_value: unsafe { std::mem::transmute(mock_get_ctx as *const ()) },
            set_module_context_value: mock_set_context,
            get_request_method: unsafe { std::mem::transmute(mock_get_req_method as *const ()) },
            get_request_path: unsafe { std::mem::transmute(mock_get_req_path as *const ()) },
            get_request_query: unsafe { std::mem::transmute(mock_get_req_query as *const ()) },
            get_request_header: unsafe { std::mem::transmute(mock_get_header as *const ()) },
            get_request_headers: unsafe { std::mem::transmute(mock_get_req_headers as *const ()) },
            get_request_body: unsafe { std::mem::transmute(mock_get_req_body as *const ()) },
            get_source_ip: unsafe { std::mem::transmute(mock_get_source_ip as *const ()) },
            set_request_path: mock_set_req_path,
            set_request_header: mock_set_req_header,
            set_source_ip: mock_set_source_ip,
            get_response_status: mock_get_resp_status,
            get_response_header: unsafe { std::mem::transmute(mock_get_resp_header as *const ()) },
            set_response_status: mock_set_resp_status,
            set_response_header: mock_set_resp_header,
            set_response_body: mock_set_resp_body,
            get_server_metrics: unsafe { std::mem::transmute(mock_get_req_body as *const ()) }, // Reuse dummy
            get_response_body: unsafe { std::mem::transmute(mock_get_req_body as *const ()) }, // Reuse dummy
            get_all_configs: unsafe { std::mem::transmute(mock_get_req_body as *const ()) }, // Reuse dummy
        }
    }

    #[test]
    fn test_process_request_with_header() {
        MOCK_HEADERS.with(|h| h.borrow_mut().insert("x-forwarded-for".to_string(), "10.0.0.1, 192.168.1.1".to_string()));
        MOCK_SOURCE_IP.with(|ip| *ip.borrow_mut() = "127.0.0.1".to_string());
        MOCK_CONTEXT.with(|ctx| ctx.borrow_mut().clear());

        let api = make_mock_api();
        let module = OxModule::new(&api).unwrap();
        
        let mut ps = PipelineState {
            arena: Bump::new(),
            protocol: "".to_string(),
            request_method: "".to_string(),
            request_path: "".to_string(),
            request_query: "".to_string(),
            request_headers: axum::http::HeaderMap::new(),
            request_body: vec![],
            source_ip: "127.0.0.1:0".parse().unwrap(),
            status_code: 200,
            response_headers: axum::http::HeaderMap::new(),
            response_body: vec![],
            module_context: std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            pipeline_ptr: std::ptr::null(),
        };

        let result = module.process_request(&mut ps as *mut _);

        assert_eq!(result, HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        });
        MOCK_SOURCE_IP.with(|ip| assert_eq!(*ip.borrow(), "10.0.0.1"));
        MOCK_CONTEXT.with(|ctx| {
            assert_eq!(ctx.borrow().get("original_source_ip").unwrap(), "\"127.0.0.1\"");
        });
    }

    #[test]
    fn test_process_request_no_header() {
        MOCK_HEADERS.with(|h| h.borrow_mut().clear());
        MOCK_SOURCE_IP.with(|ip| *ip.borrow_mut() = "127.0.0.1".to_string());
        
        let api = make_mock_api();
        let module = OxModule::new(&api).unwrap();
        
        let mut ps = PipelineState {
            arena: Bump::new(),
            protocol: "".to_string(),
            request_method: "".to_string(),
            request_path: "".to_string(),
            request_query: "".to_string(),
            request_headers: axum::http::HeaderMap::new(),
            request_body: vec![],
            source_ip: "127.0.0.1:0".parse().unwrap(),
            status_code: 200,
            response_headers: axum::http::HeaderMap::new(),
            response_body: vec![],
            module_context: std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            pipeline_ptr: std::ptr::null(),
        };

        let result = module.process_request(&mut ps as *mut _);

        assert_eq!(result, HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        });
        MOCK_SOURCE_IP.with(|ip| assert_eq!(*ip.borrow(), "127.0.0.1"));
    }
}
