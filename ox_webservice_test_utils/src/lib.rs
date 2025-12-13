use libc::{c_char, c_void};
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogLevel, ModuleInterface, PipelineState, WebServiceApiV1,
};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::{Arc, RwLock};
use bumpalo::Bump;
use axum::http::HeaderMap;

// --- Mocks ---

pub unsafe extern "C" fn mock_log(level: LogLevel, module: *const c_char, message: *const c_char) {
    if module.is_null() || message.is_null() {
        eprintln!("[{:?}] <null module/message pointer>", level);
        return;
    }
    let module_str = unsafe { CStr::from_ptr(module).to_string_lossy() };
    let message_str = unsafe { CStr::from_ptr(message).to_string_lossy() };
    println!("[{:?}] {}: {}", level, module_str, message_str);
}

pub unsafe extern "C" fn mock_alloc_str(_arena: *const c_void, s: *const c_char) -> *mut c_char {
    if s.is_null() { return ptr::null_mut(); }
    let c_str = unsafe { CStr::from_ptr(s) };
    let new_c_str = CString::new(c_str.to_bytes()).unwrap();
    new_c_str.into_raw()
}

pub unsafe extern "C" fn mock_alloc_raw(_arena: *mut c_void, size: usize, _align: usize) -> *mut c_void {
    // In a real scenario, we might use the arena. For tests, malloc is fine, 
    // but we intentionally leak to simulate the arena 'owning' it until request end.
    let layout = std::alloc::Layout::from_size_align(size, 1).unwrap();
    unsafe { std::alloc::alloc(layout) as *mut c_void }
}

pub unsafe extern "C" fn mock_get_context(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { ptr::null_mut() }
pub unsafe extern "C" fn mock_set_context(_ps: *mut PipelineState, _k: *const c_char, _v: *const c_char) {}

pub unsafe extern "C" fn mock_get_str_path(ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { 
    if ps.is_null() { return ptr::null_mut(); }
    let path = unsafe { &(*ps).request_path };
    let c_path = CString::new(path.as_str()).unwrap();
    c_path.into_raw()
}

pub unsafe extern "C" fn mock_get_str_empty(_ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { 
    let empty = CString::new("").unwrap();
    empty.into_raw()
}

// Request Headers Mocking
pub unsafe extern "C" fn mock_get_request_header(ps: *mut PipelineState, k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { 
    if ps.is_null() || k.is_null() { return ptr::null_mut(); }
    let key_str = unsafe { CStr::from_ptr(k).to_string_lossy() };
    let headers = unsafe { &(*ps).request_headers };
    
    if let Some(val) = headers.get(key_str.as_ref()) {
         let c_val = CString::new(val.as_bytes()).unwrap();
         c_val.into_raw()
    } else {
        ptr::null_mut()
    }
}

pub unsafe extern "C" fn mock_get_request_headers(ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char {
    if ps.is_null() { return ptr::null_mut(); }
    // Return empty JSON object for simplicity unless needed
    let json = CString::new("{}").unwrap();
    json.into_raw()
}

pub unsafe extern "C" fn mock_get_request_query(ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char {
    if ps.is_null() { return ptr::null_mut(); }
     let query = unsafe { &(*ps).request_query };
    let c_query = CString::new(query.as_str()).unwrap();
    c_query.into_raw()
}

pub unsafe extern "C" fn mock_get_request_method(ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char {
    if ps.is_null() { return ptr::null_mut(); }
     let method = unsafe { &(*ps).request_method };
    let c_method = CString::new(method.as_str()).unwrap();
    c_method.into_raw()
}

pub unsafe extern "C" fn mock_get_request_body(ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char {
    if ps.is_null() { return ptr::null_mut(); }
    let body = unsafe { &(*ps).request_body };
    // Warning: Body might be binary, but this API returns char*. 
    // Assuming UTF-8 for tests.
    let s = String::from_utf8_lossy(body);
    let c_s = CString::new(s.as_ref()).unwrap();
    c_s.into_raw()
}

pub unsafe extern "C" fn mock_get_source_ip(ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char {
    if ps.is_null() { return ptr::null_mut(); }
     let ip = unsafe { (*ps).source_ip.to_string() };
    let c_ip = CString::new(ip).unwrap();
    c_ip.into_raw()
}


// Response Setters
pub unsafe extern "C" fn mock_set_resp_status(ps: *mut PipelineState, status: u16) {
    if !ps.is_null() {
        unsafe { (*ps).status_code = status; }
    }
}

pub unsafe extern "C" fn mock_set_resp_header(ps: *mut PipelineState, k: *const c_char, v: *const c_char) {
     if ps.is_null() || k.is_null() || v.is_null() { return; }
     let key = unsafe { CStr::from_ptr(k).to_string_lossy().to_string() };
     let val = unsafe { CStr::from_ptr(v).to_string_lossy().to_string() };
     unsafe {
         (*ps).response_headers.insert(
             axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(),
             axum::http::HeaderValue::from_str(&val).unwrap(),
         );
     }
}

pub unsafe extern "C" fn mock_set_resp_body(ps: *mut PipelineState, body: *const u8, len: usize) {
    if ps.is_null() { return; }
    if body.is_null() && len > 0 { 
        eprintln!("mock_set_resp_body: Body is null but len is {}", len);
        return; 
    }
    if body.is_null() { return; } // Empty body

    let slice = unsafe { std::slice::from_raw_parts(body, len) };
    unsafe { (*ps).response_body = slice.to_vec(); }
}

pub unsafe extern "C" fn mock_get_response_status(ps: *mut PipelineState) -> u16 {
    if ps.is_null() { return 0; }
    unsafe { (*ps).status_code }
}

pub unsafe extern "C" fn mock_get_response_header(ps: *mut PipelineState, k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char {
     if ps.is_null() || k.is_null() { return ptr::null_mut(); }
    let key_str = unsafe { CStr::from_ptr(k).to_string_lossy() };
    let headers = unsafe { &(*ps).response_headers };
    
    if let Some(val) = headers.get(key_str.as_ref()) {
         let c_val = CString::new(val.as_bytes()).unwrap();
         c_val.into_raw()
    } else {
        ptr::null_mut()
    }
}

pub unsafe extern "C" fn mock_get_response_body(ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char {
    if ps.is_null() { return ptr::null_mut(); }
    let body = unsafe { &(*ps).response_body };
    let s = String::from_utf8_lossy(body);
    let c_s = CString::new(s.as_ref()).unwrap();
    c_s.into_raw()
}

pub unsafe extern "C" fn mock_noop_cchar(_ps: *mut PipelineState, _v: *const c_char) {} 
pub unsafe extern "C" fn mock_noop_cchar_2(_ps: *mut PipelineState, _k: *const c_char, _v: *const c_char) {}
pub unsafe extern "C" fn mock_get_server_metrics(_a: *const c_void, _f: AllocStrFn) -> *mut c_char { ptr::null_mut() } 

pub fn create_mock_api() -> WebServiceApiV1 {
    WebServiceApiV1 {
        log_callback: mock_log,
        alloc_str: mock_alloc_str,
        alloc_raw: mock_alloc_raw,
        get_module_context_value: mock_get_context,
        set_module_context_value: mock_set_context,
        get_request_method: mock_get_request_method,
        get_request_path: mock_get_str_path,
        get_request_query: mock_get_request_query,
        get_request_header: mock_get_request_header,
        get_request_headers: mock_get_request_headers,
        get_request_body: mock_get_request_body,
        get_source_ip: mock_get_source_ip,
        set_request_path: mock_noop_cchar,
        set_request_header: mock_noop_cchar_2,
        set_source_ip: mock_noop_cchar,
        get_response_status: mock_get_response_status,
        get_response_header: mock_get_response_header,
        get_response_body: mock_get_response_body,
        set_response_status: mock_set_resp_status,
        set_response_header: mock_set_resp_header,
        set_response_body: mock_set_resp_body, 
        get_server_metrics: mock_get_server_metrics,
    }
}

// --- Module Loader Helper ---

pub struct ModuleLoader {
    pub interface_ptr: *mut ModuleInterface,
    // Keep alive if needed, or allow leak for tests
}

impl ModuleLoader {
    pub fn load(
        init_fn: unsafe extern "C" fn(*const c_char, *const WebServiceApiV1) -> *mut ModuleInterface,
        config_json: &str,
        api: &WebServiceApiV1,
    ) -> Result<Self, String> {
        let c_config = CString::new(config_json).unwrap();
        let interface_ptr = unsafe { init_fn(c_config.as_ptr(), api as *const WebServiceApiV1) };

        if interface_ptr.is_null() {
            return Err("initialize_module returned null".to_string());
        }

        Ok(Self { interface_ptr })
    }

    pub fn process_request(
        &self,
        pipeline_state: &mut PipelineState,
        log_callback: unsafe extern "C" fn(LogLevel, *const c_char, *const c_char),
        alloc_fn: unsafe extern "C" fn(*mut c_void, usize, usize) -> *mut c_void,
    ) -> HandlerResult {
        unsafe {
            let interface = &*self.interface_ptr;
            (interface.handler_fn)(
                interface.instance_ptr,
                pipeline_state as *mut _,
                log_callback,
                alloc_fn,
                ptr::null(), // arena
            )
        }
    }
}

pub fn create_stub_pipeline_state() -> PipelineState {
     PipelineState {
        arena: Bump::new(),
        protocol: "HTTP/1.1".to_string(),
        request_method: "GET".to_string(),
        request_path: "/".to_string(),
        request_query: "".to_string(),
        request_headers: HeaderMap::new(),
        request_body: Vec::new(),
        source_ip: "127.0.0.1:1234".parse().unwrap(),
        status_code: 0,
        response_headers: HeaderMap::new(),
        response_body: Vec::new(),
        module_context: Arc::new(RwLock::new(HashMap::new())),
    }
}
