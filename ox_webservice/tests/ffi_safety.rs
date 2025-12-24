use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use std::ptr;
use ox_webservice::{
    pipeline::{
        alloc_str_c, 
        get_state_c,
        set_state_c,
    },
    PipelineState,
};
use bumpalo::Bump;
use axum::http::HeaderMap;

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
        is_modified: false,
        execution_history: Vec::new(),
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
fn test_ffi_get_state_header_valid() {
    let (mut state, mut arena) = create_dummy_state();
    state.request_headers.insert("Host", "localhost".parse().unwrap());
    
    let state_ptr = &mut *state as *mut PipelineState as *mut c_void;
    let arena_ptr = &mut *arena as *mut Bump as *mut c_void;
    
    let key = CString::new("http.request.header.Host").unwrap();
    
    unsafe {
        let res_ptr = get_state_c(state_ptr, key.as_ptr(), arena_ptr, alloc_str_c);
        assert!(!res_ptr.is_null());
        let val = CStr::from_ptr(res_ptr).to_str().unwrap();
        // get_state returns JSON string of the value
        assert_eq!(val, "\"localhost\"");
    }
}

#[test]
fn test_ffi_get_state_invalid_utf8_key() {
    let (mut state, mut arena) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState as *mut c_void;
    let arena_ptr = &mut *arena as *mut Bump as *mut c_void;
    
    // Invalid UTF-8 key
    let bytes = b"http.request.header.Host\xFF";
    let c_key = CString::new(bytes.as_slice()).unwrap(); 
    
    unsafe {
        let res_ptr = get_state_c(state_ptr, c_key.as_ptr(), arena_ptr, alloc_str_c);
        assert!(res_ptr.is_null());
    }
}

#[test]
fn test_ffi_set_state_path() {
    let (mut state, _) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState as *mut c_void;
    
    let key = CString::new("http.request.path").unwrap();
    let val = CString::new("\"/new/path\"").unwrap(); // JSON String
    unsafe {
        set_state_c(state_ptr, key.as_ptr(), val.as_ptr());
    }
    assert_eq!(state.request_path, "/new/path");
}

#[test]
fn test_ffi_set_state_header() {
    let (mut state, _) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState as *mut c_void;
    
    let key = CString::new("http.response.header.X-Test").unwrap();
    let val = CString::new("\"Value\"").unwrap(); // JSON String
    unsafe {
        set_state_c(state_ptr, key.as_ptr(), val.as_ptr());
    }
    assert_eq!(state.response_headers.get("X-Test").unwrap().to_str().unwrap(), "Value");
}

#[test]
fn test_ffi_module_context() {
    let (mut state, mut arena) = create_dummy_state();
    let state_ptr = &mut *state as *mut PipelineState as *mut c_void;
    let arena_ptr = &mut *arena as *mut Bump as *mut c_void;
    
    // Set
    let key = CString::new("my_key").unwrap();
    let val_json = CString::new("\"my_val\"").unwrap();
    unsafe {
        set_state_c(state_ptr, key.as_ptr(), val_json.as_ptr());
    }
    
    // Get
    unsafe {
        let res_ptr = get_state_c(state_ptr, key.as_ptr(), arena_ptr, alloc_str_c);
        assert!(!res_ptr.is_null());
        let res_str = CStr::from_ptr(res_ptr).to_str().unwrap();
        assert_eq!(res_str, "\"my_val\""); 
    }
}

#[test]
fn test_ffi_set_state_source_ip() {
    let (mut state, _) = create_dummy_state();
    state.source_ip = "127.0.0.1:8080".parse().unwrap();
    let state_ptr = &mut *state as *mut PipelineState as *mut c_void;
    
    let key = CString::new("http.source_ip").unwrap();
    // Setting IP string. The logic in set_state_c handles string -> IpAddr and preserves port.
    let val = CString::new("\"10.0.0.1\"").unwrap(); 
    unsafe {
        set_state_c(state_ptr, key.as_ptr(), val.as_ptr());
    }
    // Logic should preserve the original port 8080
    assert_eq!(state.source_ip.to_string(), "10.0.0.1:8080");
}
