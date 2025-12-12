use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::net::SocketAddr;
use std::ffi::{CStr, CString, c_void};
use bumpalo::Bump;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, Request};
use axum::response::{Response, IntoResponse};
use axum::extract::ws::{WebSocket, Message, CloseFrame, WebSocketUpgrade};
use axum::extract::FromRequest;
use log::{info, debug, error};
use serde::Serialize;
use serde_json::Value;
use libloading::{Library, Symbol};
use regex::Regex;
use std::path::Path;
use std::time::Instant;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, AtomicU64, AtomicBool, Ordering};
use once_cell::sync::Lazy;

use ox_webservice_api::{
    ModuleConfig, InitializeModuleFn, Phase, HandlerResult,
    ModuleInterface, WebServiceApiV1,
    PipelineState,
};

use crate::ServerConfig;

// --- Metrics Structures ---

static METRICS_ENABLED: AtomicBool = AtomicBool::new(true);

#[derive(Serialize)]
pub struct ModuleMetrics {
    pub execution_count: AtomicU64,
    pub total_duration_micros: AtomicU64,
    pub memory_allocated: AtomicU64, // In bytes
}

impl ModuleMetrics {
    fn new() -> Self {
        Self {
            execution_count: AtomicU64::new(0),
            total_duration_micros: AtomicU64::new(0),
            memory_allocated: AtomicU64::new(0),
        }
    }
}

pub struct ServerMetrics {
    pub active_pipelines_by_phase: RwLock<HashMap<Phase, AtomicUsize>>,
    pub global_memory_allocated: AtomicU64,
}

static SERVER_METRICS: Lazy<ServerMetrics> = Lazy::new(|| {
    let mut phases = HashMap::new();
    // Pre-populate keys for all phases
    let all_phases = [
        Phase::PreEarlyRequest, Phase::EarlyRequest, Phase::PostEarlyRequest, Phase::PreAuthentication,
        Phase::Authentication, Phase::PostAuthentication, Phase::PreAuthorization, Phase::Authorization,
        Phase::PostAuthorization, Phase::PreContent, Phase::Content, Phase::PostContent, Phase::PreAccounting,
        Phase::Accounting, Phase::PostAccounting, Phase::PreErrorHandling, Phase::ErrorHandling,
        Phase::PostErrorHandling, Phase::PreLateRequest, Phase::LateRequest, Phase::PostLateRequest,
    ];
    for phase in all_phases {
        phases.insert(phase, AtomicUsize::new(0));
    }
    
    ServerMetrics {
        active_pipelines_by_phase: RwLock::new(phases),
        global_memory_allocated: AtomicU64::new(0),
    }
});

#[derive(Serialize)]
struct MetricsSnapshot {
    active_pipelines_by_phase: HashMap<String, usize>,
    global_memory_allocated: u64,
    modules: HashMap<String, ModuleMetricsSnapshot>,
}

#[derive(Serialize)]
struct ModuleMetricsSnapshot {
    execution_count: u64,
    total_duration_micros: u64,
    memory_allocated: u64,
}

thread_local! {
    static CURRENT_MODULE_METRICS: RefCell<Option<Arc<ModuleMetrics>>> = RefCell::new(None);
}

// ---------------------------

// Implement C-API Callbacks here within pipeline.rs so they are available for Pipeline::new initialization
// AND not dependent on main.rs.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn alloc_str_c(arena: *const c_void, s: *const libc::c_char) -> *mut libc::c_char { unsafe {
    let arena = &*(arena as *const Bump);
    let s = CStr::from_ptr(s).to_str().unwrap();
    let c_string = CString::new(s).unwrap();
    let ptr = c_string.as_ptr() as *mut libc::c_char;
    let len = c_string.as_bytes_with_nul().len();
    
    // Allocate
    let allocated = arena.alloc_slice_copy(std::slice::from_raw_parts(ptr as *const u8, len));
    
    // Metrics
    if METRICS_ENABLED.load(Ordering::Relaxed) {
        let size_bytes = len as u64;
        SERVER_METRICS.global_memory_allocated.fetch_add(size_bytes, Ordering::Relaxed);
        CURRENT_MODULE_METRICS.with(|m| {
            if let Some(metrics) = &*m.borrow() {
                metrics.memory_allocated.fetch_add(size_bytes, Ordering::Relaxed);
            }
        });
    }

    allocated.as_mut_ptr() as *mut libc::c_char
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn alloc_raw_c(arena: *mut c_void, size: usize, align: usize) -> *mut c_void { unsafe {
    let arena = &mut *(arena as *mut Bump);
    let layout = std::alloc::Layout::from_size_align(size, align).unwrap();
    
    // Allocate
    let ptr = arena.alloc_layout(layout).as_ptr() as *mut c_void;

    // Metrics
    if METRICS_ENABLED.load(Ordering::Relaxed) {
        let size_bytes = size as u64; // Approximation, ignoring pad
        SERVER_METRICS.global_memory_allocated.fetch_add(size_bytes, Ordering::Relaxed);
        CURRENT_MODULE_METRICS.with(|m| {
            if let Some(metrics) = &*m.borrow() {
                metrics.memory_allocated.fetch_add(size_bytes, Ordering::Relaxed);
            }
        });
    }

    ptr
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_server_metrics_c(arena: *const c_void, alloc_fn: ox_webservice_api::AllocStrFn) -> *mut libc::c_char { unsafe {
    if !METRICS_ENABLED.load(Ordering::Relaxed) {
         return std::ptr::null_mut();
    }

    let phases_guard = SERVER_METRICS.active_pipelines_by_phase.read().unwrap();
    let mut active_pipelines = HashMap::new();
    for (phase, count) in phases_guard.iter() {
        active_pipelines.insert(format!("{:?}", phase), count.load(Ordering::Relaxed));
    }
    
    let mut module_snapshots = HashMap::new();
    if let Ok(registry) = MODULE_METRICS_REGISTRY.read() {
        for (name, metrics) in registry.iter() {
            module_snapshots.insert(name.clone(), ModuleMetricsSnapshot {
                execution_count: metrics.execution_count.load(Ordering::Relaxed),
                total_duration_micros: metrics.total_duration_micros.load(Ordering::Relaxed),
                memory_allocated: metrics.memory_allocated.load(Ordering::Relaxed),
            });
        }
    }

    let snapshot = MetricsSnapshot {
        active_pipelines_by_phase: active_pipelines,
        global_memory_allocated: SERVER_METRICS.global_memory_allocated.load(Ordering::Relaxed),
        modules: module_snapshots,
    };

    let json = serde_json::to_string(&snapshot).unwrap_or("{}".to_string());
    alloc_fn(arena, CString::new(json).unwrap().as_ptr())
}}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_module_context_value_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let key = CStr::from_ptr(key).to_str().unwrap();
    let pipeline_state = &mut *pipeline_state_ptr;
    let value = pipeline_state.module_context.read().unwrap().get(key).cloned();
    if let Some(value) = value {
        let value_str = value.to_string();
        alloc_fn(arena, CString::new(value_str).unwrap().as_ptr())
    } else {
        std::ptr::null_mut()
    }
}}

// ... (Other getters/setters unchanged, skipping needed boilerplate for tool length limits, will use replace/rewrite carefully if partial updates were possible, but here we rewrite whole file to ensure integrity of static atomic)
// Actually, I can't skip boilerplate. I must include all functions.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_module_context_value_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    value_json: *const libc::c_char,
) { unsafe {
    let key = CStr::from_ptr(key).to_str().unwrap().to_string();
    let value_json = CStr::from_ptr(value_json).to_str().unwrap();
    let value: Value = serde_json::from_str(value_json).unwrap();
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.module_context.write().unwrap().insert(key, value);
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_method_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    alloc_fn(
        arena,
        CString::new(pipeline_state.request_method.as_str())
            .unwrap()
            .as_ptr(),
    )
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_path_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    alloc_fn(
        arena,
        CString::new(pipeline_state.request_path.as_str())
            .unwrap()
            .as_ptr(),
    )
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_query_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    alloc_fn(
        arena,
        CString::new(pipeline_state.request_query.as_str())
            .unwrap()
            .as_ptr(),
    )
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_header_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let key = CStr::from_ptr(key).to_str().unwrap();
    let pipeline_state = &*pipeline_state_ptr;
    if let Some(value) = pipeline_state.request_headers.get(key) {
        alloc_fn(
            arena,
            CString::new(value.to_str().unwrap()).unwrap().as_ptr(),
        )
    } else {
        std::ptr::null_mut()
    }
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_headers_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    let headers: HashMap<String, String> = pipeline_state
        .request_headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
        .collect();
    let headers_json = serde_json::to_string(&headers).unwrap();
    alloc_fn(arena, CString::new(headers_json).unwrap().as_ptr())
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_request_body_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    let body_str = String::from_utf8_lossy(&pipeline_state.request_body);
    alloc_fn(arena, CString::new(body_str.as_ref()).unwrap().as_ptr())
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_source_ip_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    alloc_fn(
        arena,
        CString::new(pipeline_state.source_ip.to_string())
            .unwrap()
            .as_ptr(),
    )
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_request_path_c(
    pipeline_state_ptr: *mut PipelineState,
    path: *const libc::c_char,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.request_path = CStr::from_ptr(path).to_str().unwrap().to_string();
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_request_header_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    value: *const libc::c_char,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    let key = CStr::from_ptr(key).to_str().unwrap();
    let value = CStr::from_ptr(value).to_str().unwrap();
    pipeline_state
        .request_headers
        .insert(key, value.parse().unwrap());
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_source_ip_c(
    pipeline_state_ptr: *mut PipelineState,
    ip: *const libc::c_char,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.source_ip = CStr::from_ptr(ip).to_str().unwrap().parse().unwrap();
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_response_status_c(pipeline_state_ptr: *mut PipelineState) -> u16 { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    pipeline_state.status_code
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_status_c(
    pipeline_state_ptr: *mut PipelineState,
    status_code: u16,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.status_code = status_code;
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_header_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    value: *const libc::c_char,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    let key = CStr::from_ptr(key).to_str().unwrap();
    let value = CStr::from_ptr(value).to_str().unwrap();
    pipeline_state
        .response_headers
        .insert(key, value.parse().unwrap());
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_response_body_c(
    pipeline_state_ptr: *mut PipelineState,
    body: *const u8,
    body_len: usize,
) { unsafe {
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.response_body = std::slice::from_raw_parts(body, body_len).to_vec();
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_response_header_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let key = CStr::from_ptr(key).to_str().unwrap();
    let pipeline_state = &*pipeline_state_ptr;
    if let Some(value) = pipeline_state.response_headers.get(key) {
        alloc_fn(
            arena,
            CString::new(value.to_str().unwrap()).unwrap().as_ptr(),
        )
    } else {
        std::ptr::null_mut()
    }
}}

// =========================================================================

static MODULE_METRICS_REGISTRY: Lazy<RwLock<HashMap<String, Arc<ModuleMetrics>>>> = 
    Lazy::new(|| RwLock::new(HashMap::new()));

pub struct LoadedModule {
    _library: Arc<Library>,
    pub module_name: String,
    pub module_interface: Box<ModuleInterface>,
    pub module_config: ModuleConfig,
    pub metrics: Arc<ModuleMetrics>,
}

unsafe impl Send for LoadedModule {}
unsafe impl Sync for LoadedModule {}

#[derive(Clone)]
pub struct Pipeline {
    phases: HashMap<Phase, Vec<Arc<LoadedModule>>>,
}

impl Pipeline {
    pub fn new(config: &ServerConfig) -> Result<Self, String> {
        // Initialize metrics gating
        if let Some(enabled) = config.enable_metrics {
            METRICS_ENABLED.store(enabled, Ordering::Relaxed);
        } else {
             METRICS_ENABLED.store(true, Ordering::Relaxed);
        }

        let mut phases: HashMap<Phase, Vec<Arc<LoadedModule>>> = HashMap::new();
        let mut loaded_libraries: HashMap<String, Arc<Library>> = HashMap::new();

        // Initialize WebServiceApiV1 struct
        let api = Box::new(WebServiceApiV1 {
            log_callback: log_callback_c,
            alloc_str: alloc_str_c,
            alloc_raw: alloc_raw_c,
            get_module_context_value: get_module_context_value_c,
            set_module_context_value: set_module_context_value_c,
            get_request_method: get_request_method_c,
            get_request_path: get_request_path_c,
            get_request_query: get_request_query_c,
            get_request_header: get_request_header_c,
            get_request_headers: get_request_headers_c,
            get_request_body: get_request_body_c,
            get_source_ip: get_source_ip_c,
            set_request_path: set_request_path_c,
            set_request_header: set_request_header_c,
            set_source_ip: set_source_ip_c,
            get_response_status: get_response_status_c,
            get_response_header: get_response_header_c,
            set_response_status: set_response_status_c,
            set_response_header: set_response_header_c,
            set_response_body: set_response_body_c,
            get_server_metrics: get_server_metrics_c,
        });

        // Box::leak to keep it alive
        let api_ptr = Box::leak(api) as *const WebServiceApiV1;

        for module_config in &config.modules {
            let lib_path = if let Some(path) = &module_config.path {
                Path::new(path).to_path_buf()
            } else {
                // Default naming convention: lib<name>.so
                Path::new("target/debug").join(format!("lib{}.so", module_config.name))
            };

            info!("Loading module '{}' from {:?}", module_config.name, lib_path);

            let lib = if let Some(lib) = loaded_libraries.get(lib_path.to_str().unwrap()) {
                 lib.clone()
            } else {
                let lib = unsafe { Library::new(&lib_path) }.map_err(|e| format!("Failed to load library {:?}: {}", lib_path, e))?;
                let lib_arc = Arc::new(lib);
                loaded_libraries.insert(lib_path.to_str().unwrap().to_string(), lib_arc.clone());
                lib_arc
            };

            let init_fn: Symbol<InitializeModuleFn> = unsafe {
                lib.get(b"initialize_module").map_err(|e| format!("Failed to find 'initialize_module' in {:?}: {}", lib_path, e))?
            };

            let params_json = serde_json::to_string(&module_config.params.clone().unwrap_or(Value::Null)).unwrap();
            let c_params_json = CString::new(params_json).unwrap();

            let module_interface_ptr = unsafe { init_fn(c_params_json.as_ptr(), api_ptr) };

            if module_interface_ptr.is_null() {
                return Err(format!("Failed to initialize module '{}'", module_config.name));
            }

            let module_interface = unsafe { Box::from_raw(module_interface_ptr) };

            let metrics = Arc::new(ModuleMetrics::new());
             // Register metrics
            if let Ok(mut registry) = MODULE_METRICS_REGISTRY.write() {
                registry.insert(module_config.name.clone(), metrics.clone());
            }

            let loaded_module = Arc::new(LoadedModule {
                _library: lib.clone(),
                module_name: module_config.name.clone(),
                module_interface,
                module_config: module_config.clone(),
                metrics, 
            });

            phases.entry(module_config.phase).or_default().push(loaded_module.clone());
            
            // Sort modules in this phase by priority
            phases.get_mut(&module_config.phase).unwrap().sort_by_key(|m| m.module_config.priority);
        }

        Ok(Pipeline { phases })
    }

    pub async fn execute_pipeline(
        self: Arc<Self>, 
        source_ip: SocketAddr, 
        method: String, 
        path: String, 
        query: String, 
        headers: HeaderMap, 
        body_bytes: Vec<u8>, 
        protocol: String
    ) -> (u16, HeaderMap, Vec<u8>) {
        const PHASES: &[Phase] = &[
            Phase::PreEarlyRequest, Phase::EarlyRequest, Phase::PostEarlyRequest, Phase::PreAuthentication,
            Phase::Authentication, Phase::PostAuthentication, Phase::PreAuthorization, Phase::Authorization,
            Phase::PostAuthorization, Phase::PreContent, Phase::Content, Phase::PostContent, Phase::PreAccounting,
            Phase::Accounting, Phase::PostAccounting, Phase::PreErrorHandling, Phase::ErrorHandling,
            Phase::PostErrorHandling, Phase::PreLateRequest, Phase::LateRequest, Phase::PostLateRequest,
        ];

        let mut state = PipelineState {
            arena: Bump::new(),
            protocol,
            request_method: method,
            request_path: path,
            request_query: query,
            request_headers: headers,
            request_body: body_bytes,
            source_ip,
            status_code: 200,
            response_headers: HeaderMap::new(),
            response_body: Vec::new(),
            module_context: Arc::new(RwLock::new(HashMap::new()))
        };

        state.module_context.write().unwrap().insert("module_name".to_string(), Value::String("NONE".to_string()));
        state.module_context.write().unwrap().insert("module_context".to_string(), Value::String("No context".to_string()));

        let mut content_was_handled = false;
        let mut current_phase_index = 0;
        
        while current_phase_index < PHASES.len() {
            let current_phase = &PHASES[current_phase_index];
            debug!("Executing phase: {:?}, Body len: {}", current_phase, state.response_body.len());
            
            let metrics_enabled = METRICS_ENABLED.load(Ordering::Relaxed);

            // Update Active Pipeline Metric
            if metrics_enabled {
                if let Ok(phases_guard) = SERVER_METRICS.active_pipelines_by_phase.read() {
                    if let Some(counter) = phases_guard.get(current_phase) {
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            if let Some(modules) = self.phases.get(current_phase) {
                info!("Executing phase: {:?} with {} modules", current_phase, modules.len());
                let mut jumped_to_next_phase = false;

                for module in modules {
                   if let Some(uris) = &module.module_config.uris {
                        let full_uri = if state.request_query.is_empty() {
                            state.request_path.clone()
                        } else {
                            format!("{}?{}", state.request_path, state.request_query)
                        };
                        
                        let hostname = state.request_headers.get(axum::http::header::HOST)
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.split(':').next().unwrap_or("").to_string());

                        let mut matched = false;
                        for uri_matcher in uris {
                            let protocol_pattern = uri_matcher.protocol.as_deref().unwrap_or(".*");
                            let hostname_pattern = uri_matcher.hostname.as_deref().unwrap_or(".*");
                            let protocol_regex = match Regex::new(protocol_pattern) {
                                Ok(re) => re,
                                Err(e) => {
                                    error!("Invalid regex for protocol '{}': {}", protocol_pattern, e);
                                    continue;
                                }
                            };
                            let hostname_regex = match Regex::new(hostname_pattern) {
                                Ok(re) => re,
                                Err(e) => {
                                    error!("Invalid regex for hostname '{}': {}", hostname_pattern, e);
                                    continue;
                                }
                            };
                            if !protocol_regex.is_match(&state.protocol) {
                                continue;
                            }
                            if !hostname_regex.is_match(hostname.as_deref().unwrap_or("")) {
                                continue;
                            }
                            match Regex::new(&uri_matcher.path) {
                                Ok(regex) => {
                                    if regex.is_match(&full_uri) {
                                        matched = true;
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error!("Invalid regex pattern '{}' for module '{}': {}", uri_matcher.path, module.module_name, e);
                                }
                            }
                        }

                        if !matched {
                            info!("Request URI '{}' did not match any URI patterns for module '{}'. Skipping module.", full_uri, module.module_name);
                            continue;
                        }
                    }

                    // --- Pre-Execution Metrics Setup ---
                    let start_time = if metrics_enabled {
                        CURRENT_MODULE_METRICS.with(|m| *m.borrow_mut() = Some(module.metrics.clone()));
                        Some(Instant::now())
                    } else {
                        None
                    };

                    let module_interface = &module.module_interface;
                    let handler_result = unsafe {
                        (module_interface.handler_fn)(
                            module_interface.instance_ptr,
                            &mut state as *mut _,
                            module_interface.log_callback,
                            alloc_raw_c,
                            &state.arena as *const Bump as *const c_void,
                        )
                    };

                    // --- Post-Execution Metrics Cleanup ---
                    if metrics_enabled {
                         if let Some(start) = start_time {
                             let elapsed = start.elapsed();
                             module.metrics.execution_count.fetch_add(1, Ordering::Relaxed);
                             module.metrics.total_duration_micros.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
                             CURRENT_MODULE_METRICS.with(|m| *m.borrow_mut() = None);
                         }
                    }


                    match handler_result {
                        HandlerResult::ModifiedContinue | HandlerResult::ModifiedNextPhase | HandlerResult::ModifiedJumpToError => {
                            let mut module_context_write_guard = state.module_context.write().unwrap();
                            module_context_write_guard.insert("module_name".to_string(), Value::String(module.module_name.clone()));
                            module_context_write_guard.insert("module_context".to_string(), Value::String("{\"status\":\"modified\"}".to_string()));
                        },
                        _ => {}
                    }

                    match handler_result {
                        HandlerResult::UnmodifiedContinue => {} 
                        HandlerResult::ModifiedContinue => {
                            if *current_phase == Phase::Content {
                                content_was_handled = true;
                            }
                        }
                        HandlerResult::UnmodifiedNextPhase => {
                            jumped_to_next_phase = true;
                            break; 
                        }
                        HandlerResult::ModifiedNextPhase => {
                            if *current_phase == Phase::Content {
                                content_was_handled = true;
                            }
                            jumped_to_next_phase = true;
                            break; 
                        }
                        HandlerResult::UnmodifiedJumpToError | HandlerResult::ModifiedJumpToError => {
                            if *current_phase == Phase::Content {
                                content_was_handled = true;
                            }
                            // Cleanup counter for current phase before jumping
                             if metrics_enabled {
                                if let Ok(phases_guard) = SERVER_METRICS.active_pipelines_by_phase.read() {
                                    if let Some(counter) = phases_guard.get(current_phase) {
                                        counter.fetch_sub(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            current_phase_index = PHASES.iter().position(|&p| p == Phase::PreErrorHandling).unwrap_or(PHASES.len());
                            jumped_to_next_phase = true;
                            break; 
                        }
                        HandlerResult::HaltProcessing => {
                            state.status_code = 500;
                            // Cleanup counter
                             if metrics_enabled {
                                if let Ok(phases_guard) = SERVER_METRICS.active_pipelines_by_phase.read() {
                                    if let Some(counter) = phases_guard.get(current_phase) {
                                        counter.fetch_sub(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            return (500, HeaderMap::new(), Vec::from("Pipeline stopped by module due to fatal error."));
                        }
                    }
                }

                if jumped_to_next_phase {
                    continue; 
                }
            }

            if *current_phase == Phase::Content && !content_was_handled {
                info!("No content module handled the request. Setting status to 500.");
                state.status_code = 500;
                let mut module_context_write_guard = state.module_context.write().unwrap();
                module_context_write_guard.insert("module_name".to_string(), Value::String("NONE".to_string()));
                module_context_write_guard.insert("module_context".to_string(), Value::String("No context module matched".to_string()));
                
                 if metrics_enabled {
                    if let Ok(phases_guard) = SERVER_METRICS.active_pipelines_by_phase.read() {
                        if let Some(counter) = phases_guard.get(current_phase) {
                            counter.fetch_sub(1, Ordering::Relaxed);
                        }
                    }
                }
                
                current_phase_index = PHASES.iter().position(|&p| p == Phase::PreErrorHandling).unwrap_or(PHASES.len());
                continue;
            }

            // Cleanup counter for current phase before moving to next
             if metrics_enabled {
                if let Ok(phases_guard) = SERVER_METRICS.active_pipelines_by_phase.read() {
                    if let Some(counter) = phases_guard.get(current_phase) {
                        counter.fetch_sub(1, Ordering::Relaxed);
                    }
                }
            }

            current_phase_index += 1;
        }

        (state.status_code, state.response_headers, state.response_body)
    }

    pub async fn handle_socket(
        self: Arc<Self>, 
        mut socket: WebSocket, 
        source_ip: SocketAddr, 
        path: String, 
        protocol: String
    ) {
        info!("WebSocket connection established for {}", source_ip);
        
        while let Some(msg) = socket.recv().await {
            let msg = match msg {
                Ok(msg) => msg,
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
            };

            match msg {
                Message::Text(t) => {
                    let (status, _, body) = self.clone().execute_pipeline(
                        source_ip,
                        "WEBSOCKET".to_string(),
                        path.clone(),
                        "".to_string(),
                        HeaderMap::new(),
                        t.as_bytes().to_vec(),
                        protocol.clone()
                    ).await;
                    
                    if status == 200 {
                        if let Ok(response_text) = String::from_utf8(body) {
                             if let Err(e) = socket.send(Message::Text(response_text)).await {
                                  error!("Failed to send WebSocket text message: {}", e);
                                  break;
                              }
                        }
                    } else {
                        if let Err(e) = socket.send(Message::Close(Some(CloseFrame { code: 1000, reason: "Request Failed".into()}))).await {
                             error!("Failed to send Close frame: {}", e);
                        }
                        break;
                    }
                }
                Message::Binary(b) => {
                      let (status, _, body) = self.clone().execute_pipeline(
                         source_ip,
                         "WEBSOCKET".to_string(),
                         path.clone(),
                         "".to_string(),
                         HeaderMap::new(),
                         b,
                         protocol.clone()
                     ).await;
 
                     if status == 200 {
                          if let Err(e) = socket.send(Message::Binary(body)).await {
                              error!("Failed to send WebSocket binary message: {}", e);
                              break;
                          }
                     } else {
                          if let Err(e) = socket.send(Message::Close(Some(CloseFrame { code: 1000, reason: "Request Failed".into()}))).await {
                              error!("Failed to send Close frame: {}", e);
                         }
                         break;
                     }
                 }
                 Message::Close(_) => {
                     info!("Client closed WebSocket connection");
                     break;
                 }
                 _ => {} // Ping/Pong handled by axum
             }
         }
     }
 
     pub async fn execute_request(self: Arc<Self>, source_ip: SocketAddr, req: Request<Body>, protocol: String) -> Response {
         
         let (parts, body) = req.into_parts();
         let body_bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap().to_vec();
 
         let request_method = parts.method.to_string();
         let request_path = parts.uri.path().to_string();
         let request_query = parts.uri.query().unwrap_or("").to_string();
 
         let is_upgrade = parts.headers.get("upgrade")
             .and_then(|h| h.to_str().ok())
             .map(|s| s.eq_ignore_ascii_case("websocket"))
             .unwrap_or(false);
 
         let (status_code, response_headers, response_body) = self.clone().execute_pipeline(
             source_ip,
             request_method.clone(),
             request_path.clone(),
             request_query.clone(),
             parts.headers.clone(),
             body_bytes.clone(),
             protocol.clone()
         ).await;
 
         if is_upgrade && (status_code == 200 || status_code == 101) {
             let req = Request::from_parts(parts, Body::from(body_bytes));
             match WebSocketUpgrade::from_request(req, &()).await {
                  Ok(ws) => {
                      let executor = self.clone();
                      let path_clone = request_path.clone();
                      let protocol_clone = protocol.clone();
                      
                      let mut response = ws.on_upgrade(move |socket| {
                          executor.handle_socket(socket, source_ip, path_clone, protocol_clone)
                      }).into_response();
 
                      for (key, value) in response_headers.iter() {
                          response.headers_mut().insert(key, value.clone());
                      }
                      return response;
                  }
                  Err(e) => {
                      error!("Failed to extract WebSocketUpgrade: {}", e);
                      return e.into_response();
                  }
             }
         }
 
         let mut response = Response::builder()
             .status(StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
         
         for (key, value) in response_headers.iter() {
             response = response.header(key, value);
         }
 
         response.body(Body::from(response_body)).unwrap()
     }
 }
 
 
 #[unsafe(no_mangle)]
 pub unsafe extern "C" fn log_callback_c(level: ox_webservice_api::LogLevel, module: *const libc::c_char, message: *const libc::c_char) {
     let module = unsafe { CStr::from_ptr(module).to_str().unwrap() };
     let message = unsafe { CStr::from_ptr(message).to_str().unwrap() };
     match level {
         ox_webservice_api::LogLevel::Error => log::error!("[{}] {}", module, message),
         ox_webservice_api::LogLevel::Warn => log::warn!("[{}] {}", module, message),
         ox_webservice_api::LogLevel::Info => log::info!("[{}] {}", module, message),
         ox_webservice_api::LogLevel::Debug => log::debug!("[{}] {}", module, message),
         ox_webservice_api::LogLevel::Trace => log::trace!("[{}] {}", module, message),
     }
 }
