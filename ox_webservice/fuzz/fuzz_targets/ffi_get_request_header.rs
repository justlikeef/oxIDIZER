#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use std::ptr;
use ox_webservice::pipeline::{get_state_c, alloc_str_c};
use ox_webservice_api::PipelineState;
use bumpalo::Bump;
use axum::http::HeaderMap;

fuzz_target!(|data: &[u8]| {
    // Create valid PipelineState
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "application/json".parse().unwrap());
    
    let state = PipelineState {
        arena: Bump::new(),
        protocol: "HTTP/1.1".to_string(),
        request_method: "GET".to_string(),
        request_path: "/test".to_string(),
        request_query: "".to_string(),
        request_headers: headers,
        request_body: Vec::new(),
        source_ip: "127.0.0.1:8080".parse().unwrap(),
        status_code: 200,
        response_headers: HeaderMap::new(),
        response_body: Vec::new(),
        module_context: Arc::new(RwLock::new(HashMap::new())),
        pipeline_ptr: ptr::null(),
        is_modified: false,
        execution_history: Vec::new(),
        route_capture: None,
    };
    
    // We need a stable pointer to state, but PipelineState is moved? 
    // Wait, state is local.
    // Convert state to *mut PipelineState (which matches *mut c_void expected by get_state_c with internal cast)
    let state_ptr = &state as *const PipelineState as *mut c_void;

    // Prepare Arena for allocator
    let arena = Bump::new();
    let arena_ptr = &arena as *const Bump as *const c_void;

    // Convert fuzzed data to CString for the key
    if let Ok(c_key) = CString::new(data) {
        unsafe {
            let key_ptr = c_key.as_ptr();
            
            // Call the FFI function
            // get_state_c(instance, key, arena, alloc)
            let result_ptr = get_state_c(state_ptr, key_ptr, arena_ptr, alloc_str_c);
            
            if !result_ptr.is_null() {
                // If it returned something (found), we can verify it if we expect a match.
                // But for fuzzing, we just ensure it doesn't crash.
                // We should check if it's a valid C string if not null.
                let _ = CStr::from_ptr(result_ptr);
            }
        }
    }
});
