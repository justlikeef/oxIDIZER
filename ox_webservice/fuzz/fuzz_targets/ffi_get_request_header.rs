#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use std::ptr;
use ox_webservice::{
    pipeline::{get_request_header_c, alloc_str_c},
    PipelineState,
    ModuleContext,
};
use bumpalo::Bump;
use axum::http::HeaderMap;

fuzz_target!(|data: &[u8]| {
    // Create valid PipelineState
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "application/json".parse().unwrap());
    
    let mut state = PipelineState {
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
    };

    // Prepare Arena for allocator
    let arena = Bump::new();
    let arena_ptr = &arena as *const Bump as *const c_void;

    // Convert fuzzed data to CString for the key
    // We must ensure it doesn't contain interior nulls if we want to treat it as a C-string,
    // OR calling CStr::from_bytes_with_nul if we want to test that specific handling.
    // However, the FFI function takes *const c_char and does CStr::from_ptr(key).
    // So we need a valid null-terminated buffer.
    
    if let Ok(c_key) = CString::new(data) {
        unsafe {
            let key_ptr = c_key.as_ptr();
            let state_ptr = &mut state as *mut PipelineState;
            
            // Call the FFI function
            let result_ptr = get_request_header_c(state_ptr, key_ptr, arena_ptr, alloc_str_c);
            
            if !result_ptr.is_null() {
                // If it returned something (found), check if valid string?
                // The alloc_str_c allocates in arena.
                // We access it via CStr/String?
                let _ = CStr::from_ptr(result_ptr);
            }
        }
    }
});
