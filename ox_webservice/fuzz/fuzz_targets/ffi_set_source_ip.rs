#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use std::ptr;
use ox_webservice::{
    pipeline::set_state_c,
    PipelineState,
};
use bumpalo::Bump;
use axum::http::HeaderMap;

fuzz_target!(|data: &[u8]| {
    let mut state = PipelineState {
        arena: Bump::new(),
        protocol: "HTTP/1.1".to_string(),
        request_method: "GET".to_string(),
        request_path: "/test".to_string(),
        request_query: "".to_string(),
        request_headers: HeaderMap::new(),
        request_body: Vec::new(),
        source_ip: "127.0.0.1:8080".parse().unwrap(),
        status_code: 200,
        response_headers: HeaderMap::new(),
        response_body: Vec::new(),
        module_context: Arc::new(RwLock::new(HashMap::new())),
        pipeline_ptr: ptr::null(),
        is_modified: false,
        execution_history: Vec::new(),
    };

    if let Ok(c_ip) = CString::new(data) {
        unsafe {
            let state_ptr = &mut state as *mut PipelineState;
            let key = CString::new("http.source_ip").unwrap();
            set_state_c(state_ptr as *mut c_void, key.as_ptr(), c_ip.as_ptr());
        }
    }
});
