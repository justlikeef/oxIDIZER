use ox_webservice_api::{
    HandlerResult, LogLevel, WebServiceApiV1, InitializeModuleFn, PipelineState, AllocStrFn,
};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use libloading::{Library, Symbol};
use std::path::PathBuf;
use cargo_metadata::MetadataCommand;
use std::fs;
use once_cell::sync::Lazy;
use std::sync::Mutex;
use bumpalo::Bump;
use log::{self, Log, Metadata, Record, set_logger, set_max_level, LevelFilter};

// Global state to store the expected request path for mocks
static MOCK_REQUEST_PATH: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
static MOCK_RESPONSE_BODY: Lazy<Mutex<Vec<u8>>> = Lazy::new(|| Mutex::new(Vec::new()));

// A static, mutable vector to store log messages received by the mock callback.
// Mutex is used to allow safe concurrent access in tests.
static LOG_MESSAGES: Mutex<Vec<(LogLevel, String, String)>> = Mutex::new(Vec::new());

struct MockLogger;

impl Log for MockLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level = match record.level() {
                log::Level::Error => LogLevel::Error,
                log::Level::Warn => LogLevel::Warn,
                log::Level::Info => LogLevel::Info,
                log::Level::Debug => LogLevel::Debug,
                log::Level::Trace => LogLevel::Trace,
            };
            LOG_MESSAGES.lock().unwrap().push((
                level,
                record.target().to_string(),
                format!("{}", record.args()),
            ));
        }
    }

    fn flush(&self) {}
}

static LOGGER: MockLogger = MockLogger;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mock_log_callback(
    level: LogLevel,
    module: *const c_char,
    message: *const c_char,
) { unsafe {
    let message = CStr::from_ptr(message).to_string_lossy().into_owned();
    let module_name = CStr::from_ptr(module).to_string_lossy().into_owned();
    println!("[MOCK LOG] {}: ({}) {}", level as u8, module_name, message);
}}

unsafe extern "C" fn mock_get_response_status(pipeline_state: *mut PipelineState) -> u16 {
    let state = unsafe { &*pipeline_state };
    state.status_code
}

unsafe extern "C" fn mock_set_response_status(pipeline_state: *mut PipelineState, status_code: u16) {
    let state = unsafe { &mut *pipeline_state };
    state.status_code = status_code;
}

unsafe extern "C" fn mock_set_response_body(pipeline_state: *mut PipelineState, body_ptr: *const u8, body_len: usize) {
    let state = unsafe { &mut *pipeline_state };
    state.response_body = unsafe { std::slice::from_raw_parts(body_ptr, body_len).to_vec() };
    *MOCK_RESPONSE_BODY.lock().unwrap() = state.response_body.clone(); // Store in global mock state
}

unsafe extern "C" fn mock_set_response_header(pipeline_state: *mut PipelineState, key_ptr: *const c_char, value_ptr: *const c_char) {
    let state = unsafe { &mut *pipeline_state };
    let key = unsafe { CStr::from_ptr(key_ptr).to_string_lossy().into_owned() };
    let value = unsafe { CStr::from_ptr(value_ptr).to_string_lossy().into_owned() };
    state.response_headers.insert(HeaderName::from_bytes(key.as_bytes()).unwrap(), HeaderValue::from_str(&value).unwrap());
}

unsafe extern "C" fn mock_get_request_method(pipeline_state: *mut PipelineState, _arena: *const c_void, _alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state };
    CString::new(state.request_method.as_str()).unwrap().into_raw()
}
unsafe extern "C" fn mock_get_request_path(pipeline_state: *mut PipelineState, _arena: *const c_void, _alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state };
    CString::new(state.request_path.as_str()).unwrap().into_raw()
}
unsafe extern "C" fn mock_get_request_query(pipeline_state: *mut PipelineState, _arena: *const c_void, _alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state };
    CString::new(state.request_query.as_str()).unwrap().into_raw()
}
unsafe extern "C" fn mock_get_request_headers(pipeline_state: *mut PipelineState, _arena: *const c_void, _alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state };
    let headers: HashMap<_,_> = state.request_headers.iter().map(|(k,v)| (k.to_string(), v.to_str().unwrap_or(""))).collect();
    CString::new(serde_json::to_string(&headers).unwrap()).unwrap().into_raw()
}
 unsafe extern "C" fn mock_get_request_body(pipeline_state: *mut PipelineState, _arena: *const c_void, _alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state };
    CString::new(String::from_utf8(state.response_body.clone()).unwrap()).unwrap().into_raw()
}
unsafe extern "C" fn mock_get_source_ip(pipeline_state: *mut PipelineState, _arena: *const c_void, _alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state };
    CString::new(state.source_ip.to_string()).unwrap().into_raw()
}
 unsafe extern "C" fn mock_get_module_context_value(pipeline_state: *mut PipelineState, key_ptr: *const c_char, _arena: *const c_void, _alloc_fn: AllocStrFn) -> *mut c_char {
    let state = unsafe { &*pipeline_state };
    let key = unsafe { CStr::from_ptr(key_ptr).to_string_lossy() };
    if let Some(val) = state.module_context.read().unwrap().get(key.as_ref()) {
        CString::new(serde_json::to_string(val).unwrap()).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
    }
}
unsafe extern "C" fn dummy_set_module_context_value(_ctx: *mut PipelineState, _key: *const c_char, _value: *const c_char) {}
unsafe extern "C" fn dummy_set_request_path(_ctx: *mut PipelineState, _path: *const c_char) {}
unsafe extern "C" fn dummy_get_request_header(_ctx: *mut PipelineState, _key: *const c_char, _arena: *const c_void, _alloc_fn: AllocStrFn) -> *mut c_char { std::ptr::null_mut() }
unsafe extern "C" fn dummy_set_request_header(_ctx: *mut PipelineState, _key: *const c_char, _value: *const c_char) {}
unsafe extern "C" fn dummy_set_source_ip(_ctx: *mut PipelineState, _ip: *const c_char) {}
unsafe extern "C" fn dummy_get_response_header(_ctx: *mut PipelineState, _key: *const c_char, _arena: *const c_void, _alloc_fn: AllocStrFn) -> *mut c_char { std::ptr::null_mut() }
unsafe extern "C" fn dummy_alloc_str(_arena: *const c_void, _s: *const c_char) -> *mut c_char { std::ptr::null_mut() }
unsafe extern "C" fn dummy_alloc_raw(_arena: *mut c_void, _size: usize, _align: usize) -> *mut c_void { std::ptr::null_mut() }


fn create_mock_api() -> WebServiceApiV1 {
    WebServiceApiV1 {
        log_callback: mock_log_callback,
        get_response_status: mock_get_response_status,
        set_response_status: mock_set_response_status,
        set_response_body: mock_set_response_body,
        set_response_header: mock_set_response_header,
        get_request_method: mock_get_request_method,
        get_request_path: mock_get_request_path,
        get_request_query: mock_get_request_query,
        get_request_headers: mock_get_request_headers,
        get_request_body: mock_get_request_body,
        get_source_ip: mock_get_source_ip,
        get_module_context_value: mock_get_module_context_value,
        set_module_context_value: dummy_set_module_context_value,
        set_request_path: dummy_set_request_path,
        get_request_header: dummy_get_request_header,
        set_request_header: dummy_set_request_header,
        set_source_ip: dummy_set_source_ip,
        get_response_header: dummy_get_response_header,
        alloc_str: dummy_alloc_str,
        alloc_raw: dummy_alloc_raw,
    }
}

fn get_dynamic_library_path() -> PathBuf {
    let metadata = MetadataCommand::new().exec().unwrap();
    let mut path = PathBuf::from(metadata.target_directory.as_str());
    path.push("debug");
    path.push("libox_webservice_errorhandler_jinja2.so");
    path
}

#[test]
fn test_module_loading_and_execution() {
    set_logger(&LOGGER).unwrap();
    set_max_level(LevelFilter::Trace);

    let temp_dir = tempfile::tempdir().unwrap();
    let content_root = temp_dir.path().to_path_buf();
    fs::create_dir_all(&content_root).unwrap();
    let error_500_template_path = content_root.join("500.jinja2");
    fs::write(&error_500_template_path, "Error page: {{ status_code }}").unwrap();
    let index_template_path = content_root.join("index.jinja2");
    fs::write(&index_template_path, "Generic error page: {{ status_code }}").unwrap();

    let config_file_path = temp_dir.path().join("error_handler_config.yaml");
    fs::write(&config_file_path, format!(r#"
content_root: "{}"
"#, content_root.to_str().unwrap())).unwrap();

    let api = create_mock_api();
    let params_json = serde_json::json!({
        "config_file": config_file_path.to_str().unwrap(),
    }).to_string();

    let params_cstring = CString::new(params_json).unwrap();

    unsafe {
        let lib = Library::new(get_dynamic_library_path()).unwrap();
        let init_func: Symbol<InitializeModuleFn> = lib.get(b"initialize_module").unwrap();
        let module_interface_ptr = init_func(params_cstring.as_ptr(), &api);
        assert!(!module_interface_ptr.is_null());
        let module_interface = Box::from_raw(module_interface_ptr);
        
        let arena = Bump::new();

        // Test with a specific error template (500.jinja2)
        let mut state_err_500 = PipelineState {
            arena,
            protocol: "http".to_string(),
            request_method: "GET".to_string(),
            request_path: "/error".to_string(),
            request_query: "".to_string(),
            request_headers: HeaderMap::new(),
            request_body: Vec::new(),
            source_ip: "127.0.0.1:12345".parse().unwrap(),
            status_code: 500,
            response_body: Vec::new(),
            response_headers: HeaderMap::new(),
            module_context: std::sync::Arc::new(std::sync::RwLock::new(HashMap::new())),
        };
        let result_err_500 = (module_interface.handler_fn)(module_interface.instance_ptr, &mut state_err_500, mock_log_callback, dummy_alloc_raw, &state_err_500.arena as *const Bump as *const c_void);
        assert_eq!(result_err_500, HandlerResult::ModifiedContinue);
        assert!(String::from_utf8_lossy(&MOCK_RESPONSE_BODY.lock().unwrap()).contains("Error page: 500"));

        // Test with a generic index.jinja2 template for a non-specific error (e.g., 404)
        let mut state_err_404 = PipelineState {
            arena: Bump::new(),
            protocol: "http".to_string(),
            request_method: "GET".to_string(),
            request_path: "/notfound".to_string(),
            request_query: "".to_string(),
            request_headers: HeaderMap::new(),
            request_body: Vec::new(),
            source_ip: "127.0.0.1:12345".parse().unwrap(),
            status_code: 404,
            response_body: Vec::new(),
            response_headers: HeaderMap::new(),
            module_context: std::sync::Arc::new(std::sync::RwLock::new(HashMap::new())),
        };
        let result_err_404 = (module_interface.handler_fn)(module_interface.instance_ptr, &mut state_err_404, mock_log_callback, dummy_alloc_raw, &state_err_404.arena as *const Bump as *const c_void);
        assert_eq!(result_err_404, HandlerResult::ModifiedContinue);
        assert!(String::from_utf8_lossy(&MOCK_RESPONSE_BODY.lock().unwrap()).contains("Generic error page: 404"));

        // Test with a status < 400 (should be UnmodifiedContinue)
        let mut state_ok = PipelineState {
            arena: Bump::new(),
            protocol: "http".to_string(),
            request_method: "GET".to_string(),
            request_path: "/".to_string(),
            request_query: "".to_string(),
            request_headers: HeaderMap::new(),
            request_body: Vec::new(),
            source_ip: "127.0.0.1:12345".parse().unwrap(),
            status_code: 200,
            response_body: Vec::new(),
            response_headers: HeaderMap::new(),
            module_context: std::sync::Arc::new(std::sync::RwLock::new(HashMap::new())),
        };
        let result_ok = (module_interface.handler_fn)(module_interface.instance_ptr, &mut state_ok, mock_log_callback, dummy_alloc_raw, &state_ok.arena as *const Bump as *const c_void);
        assert_eq!(result_ok, HandlerResult::UnmodifiedContinue);

        // Test for a non-existent specific error template and no index.jinja2 fallback
        fs::remove_file(&index_template_path).unwrap(); // Remove index fallback
        let mut state_err_401_no_template = PipelineState {
            arena: Bump::new(),
            protocol: "http".to_string(),
            request_method: "GET".to_string(),
            request_path: "/unauthorized".to_string(),
            request_query: "".to_string(),
            request_headers: HeaderMap::new(),
            request_body: Vec::new(),
            source_ip: "127.0.0.1:12345".parse().unwrap(),
            status_code: 401,
            response_body: Vec::new(),
            response_headers: HeaderMap::new(),
            module_context: std::sync::Arc::new(std::sync::RwLock::new(HashMap::new())),
        };
        let result_err_401_no_template = (module_interface.handler_fn)(module_interface.instance_ptr, &mut state_err_401_no_template, mock_log_callback, dummy_alloc_raw, &state_err_401_no_template.arena as *const Bump as *const c_void);
        assert_eq!(result_err_401_no_template, HandlerResult::ModifiedContinue);
        assert!(String::from_utf8_lossy(&MOCK_RESPONSE_BODY.lock().unwrap()).contains("401 Unauthorized"));

        // Test with invalid template syntax in 500.jinja2
        fs::write(&error_500_template_path, "Error page: {{ status_code").unwrap(); // Invalid syntax
        let mut state_invalid_template = PipelineState {
            arena: Bump::new(),
            protocol: "http".to_string(),
            request_method: "GET".to_string(),
            request_path: "/error".to_string(),
            request_query: "".to_string(),
            request_headers: HeaderMap::new(),
            request_body: Vec::new(),
            source_ip: "127.0.0.1:12345".parse().unwrap(),
            status_code: 500,
            response_body: Vec::new(),
            response_headers: HeaderMap::new(),
            module_context: std::sync::Arc::new(std::sync::RwLock::new(HashMap::new())),
        };
        let result_invalid_template = (module_interface.handler_fn)(module_interface.instance_ptr, &mut state_invalid_template, mock_log_callback, dummy_alloc_raw, &state_invalid_template.arena as *const Bump as *const c_void);
        assert_eq!(result_invalid_template, HandlerResult::ModifiedContinue);
        assert!(String::from_utf8_lossy(&MOCK_RESPONSE_BODY.lock().unwrap()).contains("500 Internal Server Error"));
    }
}
