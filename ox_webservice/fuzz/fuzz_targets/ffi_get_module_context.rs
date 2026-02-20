#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use std::ptr;
use ox_webservice::pipeline::{get_module_context_value_c, alloc_str_c};
use ox_webservice_api::{PipelineState, ModuleContext};
use bumpalo::Bump;
use axum::http::HeaderMap;
use serde_json::Value;

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
        pipeline_ptr: std::ptr::null(),
        flags: std::collections::HashSet::new(),
        execution_history: Vec::new(),
        route_capture: None,
    };
    // Pre-populate context to allow successful lookups
    state.module_context.write().unwrap().insert("test_key".to_string(), Value::String("found".to_string()));

    let arena = Bump::new();
    let arena_ptr = &arena as *const Bump as *const c_void;

    if let Ok(c_key) = CString::new(data) {
        unsafe {
            let key_ptr = c_key.as_ptr();
            let state_ptr = &mut state as *mut PipelineState;
            let result_ptr = get_module_context_value_c(state_ptr, key_ptr, arena_ptr, alloc_str_c);
            if !result_ptr.is_null() {
                let _ = CStr::from_ptr(result_ptr);
            }
        }
    }
});
