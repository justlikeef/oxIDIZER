use libc::{c_char, c_void};
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogLevel, ModuleInterface, PipelineState, WebServiceApiV1, CoreHostApi,
};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::{Arc, RwLock};
use bumpalo::Bump;
use axum::http::HeaderMap;
use serde_json::Value;

// --- Mocks ---

pub unsafe extern "C" fn mock_log(level: LogLevel, module: *const c_char, message: *const c_char) {
    if module.is_null() || message.is_null() {
        eprintln!("[{:?}] <null module/message pointer>", level);
        return;
    }
    let module_str = unsafe { CStr::from_ptr(module).to_string_lossy() };
    let message_str = unsafe { CStr::from_ptr(message).to_string_lossy() };
    println!("[{:?}] {}: {}", level, module_str, message_str);
}

pub unsafe extern "C" fn mock_alloc_str(arena: *const c_void, s: *const c_char) -> *mut c_char {
    if s.is_null() { return ptr::null_mut(); }
    let c_str = unsafe { CStr::from_ptr(s) };
    let bytes = c_str.to_bytes_with_nul();

    if !arena.is_null() {
        let bump = unsafe { &*(arena as *const Bump) };
        let slice = bump.alloc_slice_copy(bytes);
        slice.as_mut_ptr() as *mut c_char
    } else {
        // Fallback for tests
        let new_c_str = CString::new(c_str.to_bytes()).unwrap();
        new_c_str.into_raw()
    }
}

pub unsafe extern "C" fn mock_alloc_raw(arena: *mut c_void, size: usize, align: usize) -> *mut c_void {
    if !arena.is_null() {
         let bump = unsafe { &mut *(arena as *mut Bump) };
         let layout = std::alloc::Layout::from_size_align(size, align).unwrap();
         let ptr = bump.alloc_layout(layout);
         ptr.as_ptr() as *mut c_void
    } else {
        let layout = std::alloc::Layout::from_size_align(size, align.max(1)).unwrap();
        unsafe { std::alloc::alloc(layout) as *mut c_void }
    }
}

// --- Generic State Mock ---
pub unsafe extern "C" fn mock_get_state(state_ptr: *mut c_void, key: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    if state_ptr.is_null() || key.is_null() { return ptr::null_mut(); }
    let key_str = unsafe { CStr::from_ptr(key).to_string_lossy() };
    let pipeline_state = unsafe { &*(state_ptr as *mut PipelineState) };
    
    // Virtual Keys
    let val_json: Option<String> = if key_str == "request.method" || key_str == "request.verb" {
        Some(Value::String(pipeline_state.request_method.clone()).to_string())
    } else if key_str == "request.path" || key_str == "request.resource" {
        Some(Value::String(pipeline_state.request_path.clone()).to_string())
    } else if key_str == "request.capture" {
         if let Some(captured) = &pipeline_state.route_capture {
            Some(Value::String(captured.clone()).to_string())
        } else {
            None
        }
    } else if key_str == "request.query" {
        Some(Value::String(pipeline_state.request_query.clone()).to_string())
    } else if key_str == "request.source_ip" {
        Some(Value::String(pipeline_state.source_ip.to_string()).to_string())
    } else if key_str == "response.status" {
        Some(pipeline_state.status_code.to_string())
    } else if key_str.starts_with("request.header.") {
        let header_name = &key_str["request.header.".len()..];
        pipeline_state.request_headers.get(header_name).map(|v| Value::String(v.to_str().unwrap_or("").to_string()).to_string())
    } else if key_str.starts_with("response.header.") {
        let header_name = &key_str["response.header.".len()..];
        pipeline_state.response_headers.get(header_name).map(|v| Value::String(v.to_str().unwrap_or("").to_string()).to_string())
    } else if key_str == "pipeline.modified" {
        Some(pipeline_state.is_modified.to_string())
    } else {
        // Generic module context
         pipeline_state.module_context.read().unwrap().get(key_str.as_ref()).map(|v| v.to_string())
    };

    if let Some(s) = val_json {
        let c_s = CString::new(s).unwrap();
        unsafe { alloc_fn(arena, c_s.as_ptr()) }
    } else {
        ptr::null_mut()
    }
}

pub unsafe extern "C" fn mock_set_state(state_ptr: *mut c_void, key: *const c_char, value_json: *const c_char) {
    if state_ptr.is_null() || key.is_null() || value_json.is_null() { return; }
    let key_str = unsafe { CStr::from_ptr(key).to_string_lossy().to_string() };
    let val_str = unsafe { CStr::from_ptr(value_json).to_string_lossy() };
    let value: Value = serde_json::from_str(&val_str).unwrap_or(Value::Null);

    let pipeline_state = unsafe { &mut *(state_ptr as *mut PipelineState) };

    if key_str == "request.path" {
        if let Some(s) = value.as_str() {
             pipeline_state.request_path = s.to_string();
        }
    } else if key_str == "request.capture" {
         pipeline_state.route_capture = value.as_str().map(|s| s.to_string());
    } else if key_str == "request.source_ip" {
         if let Some(s) = value.as_str() {
             if let Ok(ip) = s.parse() {
                 pipeline_state.source_ip = ip;
             }
         }
    } else if key_str == "response.status" {
         if let Some(i) = value.as_u64() {
             pipeline_state.status_code = i as u16;
         }
    } else if key_str == "response.body" {
         if let Some(s) = value.as_str() {
             pipeline_state.response_body = s.as_bytes().to_vec();
         } else if let Some(arr) = value.as_array() {
             // Handle array of bytes if passed as array
             let bytes: Vec<u8> = arr.iter().filter_map(|v| v.as_u64().map(|b| b as u8)).collect();
             pipeline_state.response_body = bytes;
         }
    } else if key_str.starts_with("response.header.") {
        let header_name = &key_str["response.header.".len()..];
        if let Some(s) = value.as_str() {
             if let Ok(k) = axum::http::header::HeaderName::from_bytes(header_name.as_bytes()) {
                 if let Ok(v) = s.parse() {
                     pipeline_state.response_headers.insert(k, v);
                 }
             }
        }
    } else if key_str.starts_with("request.header.") {
        let header_name = &key_str["request.header.".len()..];
        if let Some(s) = value.as_str() {
             if let Ok(k) = axum::http::header::HeaderName::from_bytes(header_name.as_bytes()) {
                 if let Ok(v) = s.parse() {
                     pipeline_state.request_headers.insert(k, v);
                 }
             }
        }
    } else if key_str == "response.type" {
         if let Some(s) = value.as_str() {
             if let Ok(v) = s.parse() {
                 pipeline_state.response_headers.insert(axum::http::header::CONTENT_TYPE, v);
             }
         }
    } else {
        pipeline_state.module_context.write().unwrap().insert(key_str, value);
    }
}

pub unsafe extern "C" fn mock_get_config(_state_ptr: *mut c_void, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char {
    let json = CString::new("{}").unwrap();
    unsafe { alloc_fn(arena, json.as_ptr()) }
}


pub unsafe extern "C" fn mock_execute_module(
    _state: *mut c_void,
    module_id: *const c_char,
) -> HandlerResult {
    if !module_id.is_null() {
        let id = unsafe { CStr::from_ptr(module_id).to_string_lossy() };
        println!("[MOCK] execute_module called for '{}'", id);
    }
    HandlerResult {
        status: ox_webservice_api::ModuleStatus::Unmodified,
        flow_control: ox_webservice_api::FlowControl::Continue,
        return_parameters: ox_webservice_api::ReturnParameters { return_data: std::ptr::null_mut() },
    }
}

pub fn create_mock_api() -> CoreHostApi {
    CoreHostApi {
        log_callback: mock_log,
        alloc_str: mock_alloc_str,
        alloc_raw: mock_alloc_raw,
        get_state: mock_get_state,
        set_state: mock_set_state,
        get_config: mock_get_config,
        execute_module: mock_execute_module,
    }
}

// --- Module Loader Helper ---

pub struct ModuleLoader {
    pub interface_ptr: *mut ModuleInterface,
}

impl ModuleLoader {
    pub fn load(
        init_fn: unsafe extern "C" fn(*const c_char, *const c_char, *const CoreHostApi) -> *mut ModuleInterface,
        config_json: &str,
        module_id: &str,
        api: &CoreHostApi,
    ) -> Result<Self, String> {
        let c_config = CString::new(config_json).unwrap();
        let c_id = CString::new(module_id).unwrap();
        let interface_ptr = unsafe { init_fn(c_config.as_ptr(), c_id.as_ptr(), api as *const CoreHostApi) };

        if interface_ptr.is_null() {
            return Err("initialize_module returned null".to_string());
        }

        Ok(Self { interface_ptr })
    }

    pub fn process_request(
        &self,
        pipeline_state: &mut PipelineState,
        log_callback: unsafe extern "C" fn(LogLevel, *const c_char, *const c_char),
        alloc_fn: unsafe extern "C" fn(*mut c_void, usize, usize) -> *mut c_void,
    ) -> HandlerResult {
        unsafe {
            let interface = &*self.interface_ptr;
            (interface.handler_fn)(
                interface.instance_ptr,
                pipeline_state as *mut PipelineState,
                log_callback,
                alloc_fn,
                &pipeline_state.arena as *const Bump as *const c_void,
            )
        }
    }
}

pub fn create_stub_pipeline_state() -> PipelineState {
     PipelineState {
        arena: Bump::new(),
        protocol: "HTTP/1.1".to_string(),
        request_method: "GET".to_string(),
        request_path: "/".to_string(),
        request_query: "".to_string(),
        request_headers: HeaderMap::new(),
        request_body: Vec::new(),
        source_ip: std::net::SocketAddr::from(([127, 0, 0, 1], 8080)),
        status_code: 200,
        response_headers: HeaderMap::new(),
        response_body: Vec::new(),
        module_context: Arc::new(RwLock::new(HashMap::new())),
        pipeline_ptr: std::ptr::null_mut(),
        is_modified: false,
        execution_history: Vec::new(),
        route_capture: None,
    }
}
