use libc::{c_void, c_char};
use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    WebServiceApiV1, AllocFn, PipelineState,
};
use std::ffi::{CStr, CString};
use std::panic;
use anyhow::Result;
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_webservice_restore_ip";

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
            return HandlerResult::ModifiedJumpToError;
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        // 1. Retrieve "original_source_ip" from module context
        let ctx_key = CString::new("original_source_ip").unwrap();
        let json_value_ptr = unsafe {
            (self.api.get_module_context_value)(
                pipeline_state,
                ctx_key.as_ptr(),
                arena_ptr,
                self.api.alloc_str
            )
        };

        if json_value_ptr.is_null() {
            self.log(LogLevel::Debug, "No 'original_source_ip' found in module context. Skipping restore.".to_string());
            return HandlerResult::UnmodifiedContinue;
        }

        let json_str = unsafe { CStr::from_ptr(json_value_ptr).to_string_lossy() };
        
        // 2. Parse the IP (it's a JSON string, e.g., "\"127.0.0.1\"")
        let original_ip: String = match serde_json::from_str(&json_str) {
            Ok(ip) => ip,
            Err(e) => {
                self.log(LogLevel::Warn, format!("Failed to parse 'original_source_ip' JSON: {}. Value was: {}", e, json_str));
                return HandlerResult::UnmodifiedContinue;
            }
        };

        // 3. Restore Source IP
        let ip_c = match CString::new(original_ip.clone()) {
            Ok(s) => s,
            Err(_) => return HandlerResult::UnmodifiedContinue,
        };

        unsafe {
            (self.api.set_source_ip)(pipeline_state, ip_c.as_ptr());
        }

        self.log(LogLevel::Info, format!("Restored Source IP to {}", original_ip));

        HandlerResult::ModifiedContinue
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    _module_params_json_ptr: *const c_char,
    api_ptr: *const WebServiceApiV1,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };

    let glue = match OxModule::new(api_instance) {
        Ok(g) => g,
        Err(e) => {
             let log_msg = CString::new(format!("Failed to initialize: {}", e)).unwrap();
             let module_name = CString::new(MODULE_NAME).unwrap();
             unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
             return std::ptr::null_mut();
        }
    };
    
    let log_msg = CString::new("ox_webservice_restore_ip initialized").unwrap();
    let module_name = CString::new(MODULE_NAME).unwrap();
    unsafe { (api_instance.log_callback)(LogLevel::Info, module_name.as_ptr(), log_msg.as_ptr()); }

    let instance_ptr = Box::into_raw(Box::new(glue)) as *mut c_void;

    Box::into_raw(Box::new(ModuleInterface {
        instance_ptr,
        handler_fn: process_request_c,
        log_callback: api_instance.log_callback,
    }))
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void, 
) -> HandlerResult {
    if instance_ptr.is_null() {
        return HandlerResult::ModifiedJumpToError;
    }

    let result = panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let handler = unsafe { &*(instance_ptr as *mut OxModule) };
        handler.process_request(pipeline_state_ptr)
    }));

     match result {
        Ok(r) => r,
        Err(e) => {
             let log_msg = CString::new(format!("Panic in ox_webservice_restore_ip: {:?}", e)).unwrap();
             let module_name = CString::new(MODULE_NAME).unwrap();
             unsafe { (log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
             HandlerResult::ModifiedJumpToError
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
        static MOCK_SOURCE_IP: RefCell<String> = RefCell::new("10.0.0.1".to_string());
        static MOCK_CONTEXT: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    }

    unsafe extern "C" fn mock_log(_level: LogLevel, _module: *const c_char, _msg: *const c_char) {}

    unsafe extern "C" fn mock_get_context(_ps: *mut PipelineState, key: *const c_char, _arena: *const c_void, _alloc: AllocFn) -> *mut c_char {
        let key_str = unsafe { CStr::from_ptr(key).to_string_lossy() };
        let result = MOCK_CONTEXT.with(|ctx| ctx.borrow().get(key_str.as_ref()).cloned());
        match result {
            Some(v) => CString::new(v).unwrap().into_raw(),
            None => ptr::null_mut(),
        }
    }

    unsafe extern "C" fn mock_set_source_ip(_ps: *mut PipelineState, ip: *const c_char) {
        let ip_str = unsafe { CStr::from_ptr(ip).to_string_lossy() };
        MOCK_SOURCE_IP.with(|ip| *ip.borrow_mut() = ip_str.into_owned());
    }
    
    unsafe extern "C" fn mock_alloc_str(_a: *const c_void, _s: *const c_char) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_alloc_raw(_a: *mut c_void, _s: usize, _align: usize) -> *mut c_void { ptr::null_mut() }
    unsafe extern "C" fn mock_set_ctx(_ps: *mut PipelineState, _k: *const c_char, _v: *const c_char) {}
    unsafe extern "C" fn mock_get_req_method(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_path(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_query(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_headers(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_header(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_req_body(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_src_ip(_ps: *mut PipelineState, _a: *const c_void, _f: AllocFn) -> *mut c_char { ptr::null_mut() }
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
            get_module_context_value: unsafe { std::mem::transmute(mock_get_context as *const ()) },
            set_module_context_value: mock_set_ctx,
            get_request_method: unsafe { std::mem::transmute(mock_get_req_method as *const ()) },
            get_request_path: unsafe { std::mem::transmute(mock_get_req_path as *const ()) },
            get_request_query: unsafe { std::mem::transmute(mock_get_req_query as *const ()) },
            get_request_header: unsafe { std::mem::transmute(mock_get_req_header as *const ()) },
            get_request_headers: unsafe { std::mem::transmute(mock_get_req_headers as *const ()) },
            get_request_body: unsafe { std::mem::transmute(mock_get_req_body as *const ()) },
            get_source_ip: unsafe { std::mem::transmute(mock_get_src_ip as *const ()) },
            set_request_path: mock_set_req_path,
            set_request_header: mock_set_req_header,
            set_source_ip: mock_set_source_ip,
            get_response_status: mock_get_resp_status,
            get_response_header: unsafe { std::mem::transmute(mock_get_resp_header as *const ()) },
            set_response_status: mock_set_resp_status,
            set_response_header: mock_set_resp_header,
            set_response_body: mock_set_resp_body,
        }
    }

    #[test]
    fn test_restore_ip_success() {
        MOCK_SOURCE_IP.with(|ip| *ip.borrow_mut() = "10.0.0.1".to_string());
        MOCK_CONTEXT.with(|ctx| {
            ctx.borrow_mut().insert("original_source_ip".to_string(), "\"127.0.0.1\"".to_string());
        });

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
            source_ip: "10.0.0.1:0".parse().unwrap(),
            status_code: 200,
            response_headers: axum::http::HeaderMap::new(),
            response_body: vec![],
            module_context: std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        };

        let result = module.process_request(&mut ps as *mut _);

        assert_eq!(result, HandlerResult::ModifiedContinue);
        MOCK_SOURCE_IP.with(|ip| assert_eq!(*ip.borrow(), "127.0.0.1"));
    }

    #[test]
    fn test_restore_ip_missing_context() {
        MOCK_SOURCE_IP.with(|ip| *ip.borrow_mut() = "10.0.0.1".to_string());
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
            source_ip: "10.0.0.1:0".parse().unwrap(),
            status_code: 200,
            response_headers: axum::http::HeaderMap::new(),
            response_body: vec![],
            module_context: std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        };

        let result = module.process_request(&mut ps as *mut _);

        assert_eq!(result, HandlerResult::UnmodifiedContinue);
        MOCK_SOURCE_IP.with(|ip| assert_eq!(*ip.borrow(), "10.0.0.1"));
    }
}
