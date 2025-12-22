use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use std::ptr;
use ox_webservice::{
    pipeline::{
        alloc_str_c, 
        get_request_header_c, 
        set_request_header_c,
        set_request_path_c,
        set_source_ip_c,
        get_module_context_value_c,
        set_module_context_value_c,
        get_response_header_c,
        set_response_header_c
    },
    PipelineState,
    ModuleContext,
};
use bumpalo::Bump;
use axum::http::HeaderMap;
use serde_json::Value;

// Helper to create a dummy PipelineState
fn create_dummy_state() -> (Box<PipelineState>, Box<Bump>) {
    let arena = Box::new(Bump::new());
    let state = Box::new(PipelineState {
        arena: Bump::new(), // Note: internal arena is separate
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
    });
    (state, arena)
}

#[test]
fn test_ffi_alloc_str() {
    let (mut _state, mut arena) = create_dummy_state();
    let arena_ptr = &mut *arena as *mut Bump as *mut c_void;
    
    let input = "hello world";
    let c_input = CString::new(input).unwrap();
    
    unsafe {
        let res_ptr = alloc_str_c(arena_ptr, c_input.as_ptr());
        assert!(!res_ptr.is_null());
        let res_str = CStr::from_ptr(res_ptr).to_str().unwrap();
        assert_eq!(res_str, input);
    }
}

#[test]
fn test_ffi_get_request_header_valid() {
    let (mut state, mut arena) = create_dummy_state();
    state.request_headers.insert("Host", "localhost".parse().unwrap());
    
    let state_ptr = &mut *state as *mut PipelineState;
    let arena_ptr = &mut *arena as *mut Bump as *mut c_void;
    
    let key = CString::new("Host").unwrap();
    
    unsafe {
        let res_ptr = get_request_header_c(state_ptr, key.as_ptr(), arena_ptr, alloc_str_c);
        assert!(!res_ptr.is_null());
        let val = CStr::from_ptr(res_ptr).to_str().unwrap();
        assert_eq!(val, "localhost");
    }
}

#[test]
fn test_ffi_get_request_header_invalid_utf8() {
    let (mut state, mut arena) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState;
    let arena_ptr = &mut *arena as *mut Bump as *mut c_void;
    
    // Invalid UTF-8 key
    let bytes = b"Host\xFF";
    // We cannot easily create a CString with interior nulls, but we can pass a pointer to bytes directly
    // ensuring null termination if we construct it carefully or just cast.
    // CString::new checks for nulls.
    // We want to test logic handling INVALID UTF-8 bytes *inside* the CString (but CString enforces valid C-string, not UTF-8).
    // Rust's CString doesn't enforce UTF-8.
    let c_key = CString::new(bytes.as_slice()).unwrap(); 
    
    unsafe {
        // This used to panic before my fix
        let res_ptr = get_request_header_c(state_ptr, c_key.as_ptr(), arena_ptr, alloc_str_c);
        assert!(res_ptr.is_null());
    }
}

#[test]
fn test_ffi_set_request_path() {
    let (mut state, _) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState;
    
    let path = CString::new("/new/path").unwrap();
    unsafe {
        set_request_path_c(state_ptr, path.as_ptr());
    }
    assert_eq!(state.request_path, "/new/path");
}

#[test]
fn test_ffi_set_request_header() {
    let (mut state, _) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState;
    
    let key = CString::new("X-Test").unwrap();
    let val = CString::new("Value").unwrap();
    
    unsafe {
        set_request_header_c(state_ptr, key.as_ptr(), val.as_ptr());
    }
    assert_eq!(state.request_headers.get("X-Test").unwrap().to_str().unwrap(), "Value");
}

#[test]
fn test_ffi_module_context() {
    let (mut state, mut arena) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState;
    let arena_ptr = &mut *arena as *mut Bump as *mut c_void;
    
    // Set
    let key = CString::new("my_key").unwrap();
    let val_json = CString::new("\"my_val\"").unwrap();
    unsafe {
        set_module_context_value_c(state_ptr, key.as_ptr(), val_json.as_ptr());
    }
    
    // Get
    unsafe {
        let res_ptr = get_module_context_value_c(state_ptr, key.as_ptr(), arena_ptr, alloc_str_c);
        assert!(!res_ptr.is_null());
        let res_str = CStr::from_ptr(res_ptr).to_str().unwrap();
        // Returned is string rep of Value::String
        assert!(res_str.contains("my_val")); 
    }
}

#[test]
fn test_ffi_set_source_ip() {
    let (mut state, _) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState;
    
    let ip = CString::new("10.0.0.1:1234").unwrap();
    unsafe {
        set_source_ip_c(state_ptr, ip.as_ptr());
    }
    assert_eq!(state.source_ip.to_string(), "10.0.0.1:1234");
}

#[test]
fn test_ffi_response_header() {
    let (mut state, _) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState;
    
    let key = CString::new("Content-Type").unwrap();
    // Invalid UTF-8 value test?
    let val = CString::new("application/json").unwrap();
    unsafe {
        set_response_header_c(state_ptr, key.as_ptr(), val.as_ptr());
    }
    assert_eq!(state.response_headers.get("Content-Type").unwrap().to_str().unwrap(), "application/json");
}
