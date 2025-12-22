#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use std::ptr;
use ox_webservice::{
    pipeline::set_request_path_c,
    PipelineState,
    ModuleContext,
};
use bumpalo::Bump;
use axum::http::HeaderMap;

fuzz_target!(|data: &[u8]| {
    let mut state = PipelineState {
        arena: Bump::new(),
        protocol: "HTTP/1.1".to_string(),
        request_method: "GET".to_string(),
        request_path: "/original".to_string(),
        request_query: "".to_string(),
        request_headers: HeaderMap::new(),
        request_body: Vec::new(),
        source_ip: "127.0.0.1:8080".parse().unwrap(),
        status_code: 200,
        response_headers: HeaderMap::new(),
        response_body: Vec::new(),
        module_context: Arc::new(RwLock::new(HashMap::new())),
        pipeline_ptr: ptr::null(),
    };

    if let Ok(c_path) = CString::new(data) {
        unsafe {
            let state_ptr = &mut state as *mut PipelineState;
            set_request_path_c(state_ptr, c_path.as_ptr());
        }
    }
});
