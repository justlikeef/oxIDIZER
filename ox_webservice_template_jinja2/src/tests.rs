#[cfg(test)]
mod tests {
    use super::*;
    use crate::initialize_module;
    use std::ptr;
    use std::ffi::{CString, CStr};
    use libc::{c_char, c_void};
    use ox_webservice_api::{
        WebServiceApiV1, PipelineState, LogLevel, AllocStrFn, AllocFn, ModuleContext, LogCallback, HandlerResult
    };
    use bumpalo::Bump;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use axum::http::HeaderMap;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // --- Mocks ---

    unsafe extern "C" fn mock_log(level: LogLevel, module: *const c_char, message: *const c_char) {
        let module_str = unsafe { CStr::from_ptr(module).to_str().unwrap() };
        let message_str = unsafe { CStr::from_ptr(message).to_str().unwrap() };
        println!("[{:?}] {}: {}", level, module_str, message_str);
    }

    unsafe extern "C" fn mock_alloc_str(_arena: *const c_void, s: *const c_char) -> *mut c_char {
        // Just duplicate using libc functions for simulation, or rust CString
        let c_str = unsafe { CStr::from_ptr(s) };
        let new_c_str = CString::new(c_str.to_bytes()).unwrap();
        new_c_str.into_raw()
    }

    unsafe extern "C" fn mock_alloc_raw(_arena: *mut c_void, size: usize, _align: usize) -> *mut c_void {
        let layout = std::alloc::Layout::from_size_align(size, 1).unwrap();
        unsafe { std::alloc::alloc(layout) as *mut c_void }
    }

    // Stub other functions
    unsafe extern "C" fn mock_get_context(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_set_context(_ps: *mut PipelineState, _k: *const c_char, _v: *const c_char) {}
    unsafe extern "C" fn mock_get_str(_ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { ptr::null_mut() }
    unsafe extern "C" fn mock_get_str_path(ps: *mut PipelineState, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { 
        let path = unsafe { &(*ps).request_path };
        let c_path = CString::new(path.as_str()).unwrap();
        c_path.into_raw()
    }
    
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

    unsafe extern "C" fn mock_get_request_header(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { ptr::null_mut() } 

    // Ignored unused
    unsafe extern "C" fn mock_noop_cchar(_ps: *mut PipelineState, _v: *const c_char) {} 
    unsafe extern "C" fn mock_noop_cchar_2(_ps: *mut PipelineState, _k: *const c_char, _v: *const c_char) {}
    unsafe extern "C" fn mock_get_u16(_ps: *mut PipelineState) -> u16 { 0 }
    unsafe extern "C" fn mock_get_cchar_key(_ps: *mut PipelineState, _k: *const c_char, _a: *const c_void, _f: AllocStrFn) -> *mut c_char { ptr::null_mut() }


    fn create_mock_api() -> WebServiceApiV1 {
        WebServiceApiV1 {
            log_callback: mock_log,
            alloc_str: mock_alloc_str,
            alloc_raw: mock_alloc_raw,
            get_module_context_value: mock_get_context,
            set_module_context_value: mock_set_context,
            get_request_method: mock_get_str, // unused
            get_request_path: mock_get_str_path, // USED
            get_request_query: mock_get_str, // unused
            get_request_header: mock_get_request_header,
            get_request_headers: mock_get_str,
            get_request_body: mock_get_str,
            get_source_ip: mock_get_str,
            set_request_path: mock_noop_cchar,
            set_request_header: mock_noop_cchar_2,
            set_source_ip: mock_noop_cchar,
            get_response_status: mock_get_u16,
            get_response_header: mock_get_cchar_key,
            set_response_status: mock_set_resp_status, // USED
            set_response_header: mock_set_resp_header, // USED
            set_response_body: mock_set_resp_body, // USED
        }
    }

    #[test]
    fn test_initialize_null_ptrs() {
        let api = create_mock_api();
        let api_ptr = &api as *const WebServiceApiV1;
        unsafe {
            let res = initialize_module(ptr::null(), api_ptr);
            assert!(res.is_null(), "Should return null if params ptr is null");

            let json = CString::new("{}").unwrap();
            let res2 = initialize_module(json.as_ptr(), ptr::null());
            assert!(res2.is_null(), "Should return null if api ptr is null");
        }
    }

    #[test]
    fn test_initialize_invalid_json() {
        let api = create_mock_api();
        let api_ptr = &api as *const WebServiceApiV1;
        let json = CString::new("{ invalid json").unwrap();
        unsafe {
            let res = initialize_module(json.as_ptr(), api_ptr);
            assert!(res.is_null(), "Should return null on invalid json");
        }
    }

    #[test]
    fn test_initialize_missing_config_file() {
         let api = create_mock_api();
         let api_ptr = &api as *const WebServiceApiV1;
         let json = CString::new(r#"{"config_file": "/path/does/not/exist.yaml"}"#).unwrap();
         unsafe {
             let res = initialize_module(json.as_ptr(), api_ptr);
             assert!(res.is_null(), "Should return null if config file missing");
         }
    }

    #[test]
    fn test_pass_unhandled_extension() {
         // Setup
        let mut mimetypes_file = NamedTempFile::new().unwrap();
        writeln!(mimetypes_file, "mimetypes:\n  - extension: html\n    mimetype: text/html\n    handler: template").unwrap();
        
        let mut content_file = tempfile::Builder::new().suffix(".txt").tempfile().unwrap(); // .txt not handled
        write!(content_file, "Ignored content").unwrap();

        let mut config_file = NamedTempFile::new().unwrap();
        let config_content = format!(
            "content_root: \"{}\"\nmimetypes_file: \"{}\"\ndefault_documents: []",
            content_file.path().parent().unwrap().to_str().unwrap(),
            mimetypes_file.path().to_str().unwrap()
        );
        writeln!(config_file, "{}", config_content).unwrap();

         // Initialize
        let api = create_mock_api();
        let api_ptr = &api as *const WebServiceApiV1;
        let params_json = format!(r#"{{"config_file": "{}"}}"#, config_file.path().to_str().unwrap());
        let c_params = CString::new(params_json).unwrap();
        let module_interface_ptr = unsafe { initialize_module(c_params.as_ptr(), api_ptr) };
        let module_interface = unsafe { &*module_interface_ptr };
        let handler_instance = module_interface.instance_ptr;

         // Request
        let mut pipeline_state = PipelineState {
            arena: Bump::new(),
            protocol: "HTTP/1.1".to_string(),
            request_method: "GET".to_string(),
            request_path: format!("/{}", content_file.path().file_name().unwrap().to_str().unwrap()),
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

        // Process -> Should pass (ModifiedContinue, empty body)
        let result = unsafe { (module_interface.handler_fn)(handler_instance, ps_ptr, mock_log, mock_alloc_raw, ptr::null()) };
        assert_eq!(result, HandlerResult::ModifiedContinue);
        assert!(pipeline_state.response_body.is_empty(), "Should not handle .txt file");
    }

    #[test]
    fn test_template_rendering() {
        // Setup temporary config files
        let mut mimetypes_file = NamedTempFile::new().unwrap();
        writeln!(mimetypes_file, "mimetypes:\n  - extension: html\n    mimetype: text/html\n    handler: template").unwrap();
        
        // Template Content
        let mut content_file = tempfile::Builder::new().suffix(".html").tempfile().unwrap();
        write!(content_file, "Hello {{{{ \"Jinja\" }}}}").unwrap(); // Escaped double curly braces for Rust format string

        let mut config_file = NamedTempFile::new().unwrap();
        let config_content = format!(
            "content_root: \"{}\"\nmimetypes_file: \"{}\"\ndefault_documents: []",
            content_file.path().parent().unwrap().to_str().unwrap(),
            mimetypes_file.path().to_str().unwrap()
        );
        writeln!(config_file, "{}", config_content).unwrap();

        // Initialize (Host side)
        let api = create_mock_api();
        let api_ptr = &api as *const WebServiceApiV1;
        let params_json = format!(r#"{{"config_file": "{}"}}"#, config_file.path().to_str().unwrap());
        let c_params = CString::new(params_json).unwrap();

        let module_interface_ptr = unsafe { initialize_module(c_params.as_ptr(), api_ptr) };
        assert!(!module_interface_ptr.is_null());
        
        let module_interface = unsafe { &*module_interface_ptr };
        let handler_instance = module_interface.instance_ptr;

        // Process Request (Host side)
        let mut pipeline_state = PipelineState {
            arena: Bump::new(),
            protocol: "HTTP/1.1".to_string(),
            request_method: "GET".to_string(),
            request_path: format!("/{}", content_file.path().file_name().unwrap().to_str().unwrap()),
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

        // Call generic process_request via C wrapper
        let result = unsafe { (module_interface.handler_fn)(handler_instance, ps_ptr, mock_log, mock_alloc_raw, ptr::null()) };
        
        // Assertions
        assert_eq!(result, HandlerResult::ModifiedContinue);
        assert_eq!(pipeline_state.response_body, b"Hello Jinja");
        assert_eq!(pipeline_state.response_headers.get("Content-Type").unwrap(), "text/html");
    }
}
