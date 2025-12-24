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
    // Split data into key and value
    if data.len() < 2 { return; }
    let split_idx = data.len() / 2;
    let (key_bytes, val_bytes) = data.split_at(split_idx);

    let key_c = match CString::new(key_bytes) { Ok(s) => s, Err(_) => return };
    let val_c = match CString::new(val_bytes) { Ok(s) => s, Err(_) => return };

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

    unsafe {
        let state_ptr = &mut state as *mut PipelineState;
        set_state_c(state_ptr as *mut c_void, key_c.as_ptr(), val_c.as_ptr());
    }
});
