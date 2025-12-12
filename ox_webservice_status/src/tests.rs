#[cfg(test)]
mod tests {
    use crate::initialize_module;
    use std::ptr;
    use std::ffi::{CString, CStr};
    use libc::{c_char, c_void};
    use ox_webservice_api::{
        WebServiceApiV1, PipelineState, LogLevel, AllocStrFn, HandlerResult,
    };
    use bumpalo::Bump;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use axum::http::HeaderMap;

    // --- Mocks ---

    unsafe extern "C" fn mock_log(level: LogLevel, module: *const c_char, message: *const c_char) {
        let module_str = unsafe { CStr::from_ptr(module).to_str().unwrap() };
        let message_str = unsafe { CStr::from_ptr(message).to_str().unwrap() };
        println!("[{:?}] {}: {}", level, module_str, message_str);
    }

    unsafe extern "C" fn mock_alloc_str(_arena: *const c_void, s: *const c_char) -> *mut c_char {
        let c_str = unsafe { CStr::from_ptr(s) };
        let new_c_str = CString::new(c_str.to_bytes()).unwrap();
        new_c_str.into_raw()
    }

    unsafe extern "C" fn mock_alloc_raw(_arena: *mut c_void, size: usize, _align: usize) -> *mut c_void {
        let layout = std::alloc::Layout::from_size_align(size, 1).unwrap();
        unsafe { std::alloc::alloc(layout) as *mut c_void }
    }

    unsafe extern "C" fn mock_get_context(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_set_context(_ps: *mut PipelineState, _k: *const c_char, _v: *const c_char) {}
    unsafe extern "C" fn mock_get_str(_ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { ptr::null_mut() } 
    unsafe extern "C" fn mock_set_resp_status(ps: *mut PipelineState, status: u16) {
        unsafe { (*ps).status_code = status; }
    }
    unsafe extern "C" fn mock_set_resp_header(ps: *mut PipelineState, k: *const c_char, v: *const c_char) {
         let key = unsafe { CStr::from_ptr(k).to_str().unwrap().to_string() };
         let val = unsafe { CStr::from_ptr(v).to_str().unwrap().to_string() };
         unsafe {
             (*ps).response_headers.insert(
                 axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                 axum::http::HeaderValue::from_str(&val).unwrap(),
             );
         }
    }
    unsafe extern "C" fn mock_set_resp_body(ps: *mut PipelineState, body: *const u8, len: usize) {
        let slice = unsafe { std::slice::from_raw_parts(body, len) };
        unsafe { (*ps).response_body = slice.to_vec(); }
    }

    unsafe extern "C" fn mock_request_query_json(_ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char {
        CString::new("format=json").unwrap().into_raw()
    }

    unsafe extern "C" fn mock_request_query_empty(_ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char {
        ptr::null_mut()
    }

    unsafe extern "C" fn mock_noop_cchar(_ps: *mut PipelineState, _v: *const c_char) {} 
    unsafe extern "C" fn mock_noop_cchar_2(_ps: *mut PipelineState, _k: *const c_char, _v: *const c_char) {}
    unsafe extern "C" fn mock_get_u16(_ps: *mut PipelineState) -> u16 { 0 }
    unsafe extern "C" fn mock_get_cchar_key(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { ptr::null_mut() }

    unsafe extern "C" fn mock_get_request_header_json(_ps: *mut PipelineState, k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { 
         let key = unsafe { CStr::from_ptr(k).to_str().unwrap() };
         if key == "Accept" {
             return CString::new("application/json").unwrap().into_raw();
         }
         ptr::null_mut()
    }

     unsafe extern "C" fn mock_get_request_header_empty(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { 
         ptr::null_mut()
    }

    unsafe extern "C" fn mock_get_server_metrics(_arena: *const c_void, _f: AllocStrFn) -> *mut c_char {
        ptr::null_mut()
    }

    fn create_mock_api() -> WebServiceApiV1 {
        WebServiceApiV1 {
            log_callback: mock_log,
            alloc_str: mock_alloc_str,
            alloc_raw: mock_alloc_raw,
            get_module_context_value: mock_get_context,
            set_module_context_value: mock_set_context,
            get_request_method: mock_get_str,
            get_request_path: mock_get_str,
            get_request_query: mock_request_query_empty,
            get_request_header: mock_get_request_header_empty,
            get_request_headers: mock_get_str,
            get_request_body: mock_get_str,
            get_source_ip: mock_get_str,
            set_request_path: mock_noop_cchar,
            set_request_header: mock_noop_cchar_2,
            set_source_ip: mock_noop_cchar,
            get_response_status: mock_get_u16,
            get_response_header: mock_get_cchar_key,
            set_response_status: mock_set_resp_status,
            set_response_header: mock_set_resp_header,
            set_response_body: mock_set_resp_body,
            get_server_metrics: mock_get_server_metrics,
        }
    }

    #[test]
    fn test_initialize_null_ptrs() {
        let api = create_mock_api();
        let api_ptr = &api as *const WebServiceApiV1;
        unsafe {
            let res = initialize_module(ptr::null(), api_ptr);
            assert!(res.is_null());

            let json = CString::new("{}").unwrap();
            let res2 = initialize_module(json.as_ptr(), ptr::null());
            assert!(res2.is_null());
        }
    }

    #[test]
    fn test_html_output_default() {
         let api = create_mock_api();
        let api_ptr = &api as *const WebServiceApiV1;
        let c_params = CString::new("{}").unwrap();

        let module_interface_ptr = unsafe { initialize_module(c_params.as_ptr(), api_ptr) };
        assert!(!module_interface_ptr.is_null());
        
        let module_interface = unsafe { &*module_interface_ptr };
        let handler_instance = module_interface.instance_ptr;

        let mut pipeline_state = PipelineState {
            arena: Bump::new(),
            protocol: "HTTP/1.1".to_string(),
            request_method: "GET".to_string(),
            request_path: "/status".to_string(),
            request_query: "".to_string(),
            request_headers: HeaderMap::new(),
            request_body: Vec::new(),
            source_ip: "127.0.0.1:1234".parse().unwrap(),
            status_code: 0,
            response_headers: HeaderMap::new(),
            response_body: Vec::new(),
            module_context: Arc::new(RwLock::new(HashMap::new())),
        };
        let ps_ptr = &mut pipeline_state as *mut PipelineState;

        let result = unsafe { (module_interface.handler_fn)(handler_instance, ps_ptr, mock_log, mock_alloc_raw, ptr::null()) };
        
        assert_eq!(result, HandlerResult::ModifiedContinue);
        assert!(!pipeline_state.response_body.is_empty());
        let body = String::from_utf8(pipeline_state.response_body).unwrap();
        assert!(body.contains("<html>"));
        assert_eq!(pipeline_state.response_headers.get("Content-Type").unwrap(), "text/html");
    }

     #[test]
    fn test_json_output_query() {
        let mut api = create_mock_api();
        api.get_request_query = mock_request_query_json; // Mock format=json
        
        let api_ptr = &api as *const WebServiceApiV1;
        let c_params = CString::new("{}").unwrap();

        let module_interface_ptr = unsafe { initialize_module(c_params.as_ptr(), api_ptr) };
        let module_interface = unsafe { &*module_interface_ptr };
        let handler_instance = module_interface.instance_ptr;

        let mut pipeline_state = PipelineState {
            arena: Bump::new(),
            protocol: "HTTP/1.1".to_string(),
            request_method: "GET".to_string(),
            request_path: "/status".to_string(),
            request_query: "format=json".to_string(),
            request_headers: HeaderMap::new(),
            request_body: Vec::new(),
            source_ip: "127.0.0.1:1234".parse().unwrap(),
            status_code: 0,
            response_headers: HeaderMap::new(),
            response_body: Vec::new(),
            module_context: Arc::new(RwLock::new(HashMap::new())),
        };
        let ps_ptr = &mut pipeline_state as *mut PipelineState;

        let result = unsafe { (module_interface.handler_fn)(handler_instance, ps_ptr, mock_log, mock_alloc_raw, ptr::null()) };
        
        assert_eq!(result, HandlerResult::ModifiedContinue);
         let body = String::from_utf8(pipeline_state.response_body).unwrap();
        assert!(body.starts_with("{"));
        assert_eq!(pipeline_state.response_headers.get("Content-Type").unwrap(), "application/json");
         // Verify fields exist
         let json: serde_json::Value = serde_json::from_str(&body).unwrap();
         assert!(json.get("uptime").is_some());
    }

    #[test]
    fn test_json_output_header() {
        let mut api = create_mock_api();
        api.get_request_header = mock_get_request_header_json; // Mock Accept header
        
        let api_ptr = &api as *const WebServiceApiV1;
        let c_params = CString::new("{}").unwrap();

        let module_interface_ptr = unsafe { initialize_module(c_params.as_ptr(), api_ptr) };
        let module_interface = unsafe { &*module_interface_ptr };
        let handler_instance = module_interface.instance_ptr;

        let mut pipeline_state = PipelineState {
            arena: Bump::new(),
            protocol: "HTTP/1.1".to_string(),
            request_method: "GET".to_string(),
            request_path: "/status".to_string(),
            request_query: "".to_string(),
            request_headers: HeaderMap::new(),
            request_body: Vec::new(),
            source_ip: "127.0.0.1:1234".parse().unwrap(),
            status_code: 0,
            response_headers: HeaderMap::new(),
            response_body: Vec::new(),
            module_context: Arc::new(RwLock::new(HashMap::new())),
        };
        let ps_ptr = &mut pipeline_state as *mut PipelineState;

        let result = unsafe { (module_interface.handler_fn)(handler_instance, ps_ptr, mock_log, mock_alloc_raw, ptr::null()) };
        
         assert_eq!(result, HandlerResult::ModifiedContinue);
         let body = String::from_utf8(pipeline_state.response_body).unwrap();
        assert!(body.starts_with("{"));
        assert_eq!(pipeline_state.response_headers.get("Content-Type").unwrap(), "application/json");
    }
}
