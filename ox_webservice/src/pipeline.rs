use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::net::SocketAddr;
use std::ffi::{CStr, CString, c_void};
use bumpalo::Bump;
use axum::body::Body;
use axum::http::{HeaderMap, Request};
use axum::response::Response;
use axum::extract::ws::{WebSocket, Message, CloseFrame};
use log::{info, debug, error};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use libloading::{Library, Symbol};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, AtomicU64, AtomicBool, Ordering};
use once_cell::sync::Lazy;

use ox_webservice_api::{
    ModuleConfig, InitializeModuleFn,
    ModuleInterface, WebServiceApiV1,
    PipelineState, FlowControl, ModuleStatus, HandlerResult, ReturnParameters,
    AllocStrFn, // Added
};

use libc::c_char; // Added

use crate::ServerConfig;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use futures::StreamExt;
use tempfile::NamedTempFile;
// use std::io::Write; // Removed unused import

// --- Response Body Enum ---
pub enum PipelineResponseBody {
    Memory(Vec<u8>),
    Files(Vec<PathBuf>),
}

pub enum PipelineRequestBody {
    Memory(Vec<u8>),
    File(PathBuf, u64),
}

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
    pub active_pipelines_by_phase: RwLock<HashMap<String, Arc<AtomicUsize>>>,
    pub global_memory_allocated: AtomicU64,
}

static SERVER_METRICS: Lazy<ServerMetrics> = Lazy::new(|| {
    let phases = HashMap::new();
    // Dynamic phases - initialized empty
    
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
    let s = CStr::from_ptr(s).to_string_lossy();
    let c_string = CString::new(s.into_owned()).unwrap_or_default();
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
    let mut active_pipelines_by_phase = HashMap::new();
    for (phase, count) in phases_guard.iter() {
        active_pipelines_by_phase.insert(phase.clone(), count.load(Ordering::Relaxed));
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
        active_pipelines_by_phase,
        global_memory_allocated: SERVER_METRICS.global_memory_allocated.load(Ordering::Relaxed),
        modules: module_snapshots,
    };

    let json = serde_json::to_string(&snapshot).unwrap_or("{}".to_string());
    alloc_fn(arena, CString::new(json).unwrap().as_ptr())
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_all_configs_c(
    pipeline_state_ptr: *mut PipelineState,
    arena: *const c_void,
    alloc_fn: ox_webservice_api::AllocStrFn
) -> *mut libc::c_char { unsafe {
    let pipeline_state = &*pipeline_state_ptr;
    let pipeline_ptr = pipeline_state.pipeline_ptr as *const Pipeline;
    if pipeline_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let pipeline = &*pipeline_ptr;

    let mut configs_list = Vec::new();
    let mut seen_modules = std::collections::HashSet::new();

    // 1. Host Config (Main)
    if let Ok(main_val) = serde_json::from_str::<Value>(&pipeline.main_config_json) {
         let mut entry = serde_json::Map::new();
         entry.insert("name".to_string(), Value::String("ox_webservice (Host)".to_string()));
         entry.insert("config".to_string(), main_val);
         configs_list.push(Value::Object(entry));
    }

    // 2. Iterate Pipeline Stages (Execution Order)
    for stage in &pipeline.core.stages {
        for module in &stage.modules {
            let name = module.name();
            if !seen_modules.contains(name) {
                let mut entry = serde_json::Map::new();
                entry.insert("name".to_string(), Value::String(name.to_string()));
                entry.insert("config".to_string(), module.get_config());
                entry.insert("stage".to_string(), Value::String(stage.name.clone()));
                configs_list.push(Value::Object(entry));
                seen_modules.insert(name.to_string());
            }
        }
    }

    // 3. Append any other loaded modules (e.g., loaded but unused/auxiliary)
    if let Ok(registry) = GLOBAL_MODULE_REGISTRY.read() {
        // Collect keys to sort them for deterministic output of remaining ones
        let mut keys: Vec<_> = registry.keys().cloned().collect();
        keys.sort();
        
        for id in keys {
            if !seen_modules.contains(&id) {
                if let Some(module) = registry.get(&id) {
                    let mut entry = serde_json::Map::new();
                    entry.insert("name".to_string(), Value::String(id.clone()));
                    entry.insert("config".to_string(), module.get_config());
                    entry.insert("stage".to_string(), Value::String("Auxiliary".to_string()));
                    configs_list.push(Value::Object(entry));
                }
            }
        }
    }

    let configs_json = serde_json::to_string(&configs_list).unwrap_or("[]".to_string());
    alloc_fn(arena, CString::new(configs_json).unwrap().as_ptr())
}}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_module_context_value_c(
    pipeline_state_ptr: *mut PipelineState,
    key: *const libc::c_char,
    arena: *const c_void,
    alloc_fn: unsafe extern "C" fn(*const c_void, *const libc::c_char) -> *mut libc::c_char,
) -> *mut libc::c_char { unsafe {
    let key = match CStr::from_ptr(key).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
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
    let key = CStr::from_ptr(key).to_string_lossy().to_string();
    let value_json = CStr::from_ptr(value_json).to_string_lossy();
    let value: Value = serde_json::from_str(&value_json).unwrap_or(Value::Null);
    let pipeline_state = &mut *pipeline_state_ptr;
    pipeline_state.module_context.write().unwrap().insert(key, value);
}}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_config_c(
    _pipeline_state_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: ox_webservice_api::AllocStrFn,
) -> *mut libc::c_char { unsafe {
    // Current module config not tracked in MVP. Return empty object.
    alloc_fn(arena, CString::new("{}").unwrap().as_ptr())
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_state_c(
    pipeline_state_ptr: *mut c_void,
    key: *const libc::c_char,
    arena: *const c_void,
    alloc_fn: ox_webservice_api::AllocStrFn,
) -> *mut libc::c_char { unsafe {
    if key.is_null() { return std::ptr::null_mut(); }
    let key_str_c = CStr::from_ptr(key);
    let key_str = match key_str_c.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let pipeline_state = &*(pipeline_state_ptr as *mut PipelineState);
    
    // --- Virtual Keys ---
    let val_json: Option<String> = if key_str == "request.method" {
        Some(Value::String(pipeline_state.request_method.clone()).to_string())
    } else if key_str == "request.verb" {
        let verb = match pipeline_state.request_method.as_str() {
            "GET" => "get",
            "POST" => "create",
            "PUT" => "update",
            "DELETE" => "delete",
            "WEBSOCKET" => "stream",
            _ => "execute",
        };
        Some(Value::String(verb.to_string()).to_string())
    } else if key_str == "request.path" {
        Some(Value::String(pipeline_state.request_path.clone()).to_string())
    } else if key_str == "request.capture" {
        if let Some(captured) = &pipeline_state.route_capture {
            Some(Value::String(captured.clone()).to_string())
        } else {
            None
        }
    } else if key_str == "request.query" {
        Some(Value::String(pipeline_state.request_query.clone()).to_string())
    } else if key_str.starts_with("request.query.") {
        let param_name = &key_str["request.query.".len()..];
        let query_params: HashMap<String, String> = url::form_urlencoded::parse(pipeline_state.request_query.as_bytes())
            .into_owned()
            .collect();
        if let Some(val) = query_params.get(param_name) {
             Some(Value::String(val.clone()).to_string())
        } else { None }
    } else if key_str == "request.body" {
        Some(Value::String(String::from_utf8_lossy(&pipeline_state.request_body).into_owned()).to_string())
    } else if key_str == "request.source_ip" {
        Some(Value::String(pipeline_state.source_ip.to_string()).to_string())
    } else if key_str == "response.status" {
        Some(pipeline_state.status_code.to_string()) 
    } else if key_str == "response.body" {
        Some(Value::String(String::from_utf8_lossy(&pipeline_state.response_body).into_owned()).to_string())
    } else if key_str == "response.type" {
        pipeline_state.response_headers.get(axum::http::header::CONTENT_TYPE).map(|v| Value::String(v.to_str().unwrap_or("").to_string()).to_string())    } else if key_str.starts_with("request.header.") {
        let header_name = &key_str["request.header.".len()..];
        // HeaderMap::get is case-insensitive for str
        if let Some(val) = pipeline_state.request_headers.get(header_name) {
             Some(Value::String(val.to_str().unwrap_or("").to_string()).to_string())
        } else {
             // Fallback to case-insensitive scan
             let mut found = None;
             for (k, v) in &pipeline_state.request_headers {
                 if k.as_str().eq_ignore_ascii_case(header_name) {
                     found = Some(Value::String(v.to_str().unwrap_or("").to_string()).to_string());
                     break;
                 }
             }
             found
        }
    } else if key_str == "request.headers" {
        let mut headers_map = serde_json::Map::new();
        for (k, v) in &pipeline_state.request_headers {
            headers_map.insert(k.to_string(), Value::String(v.to_str().unwrap_or("").to_string()));
        }
        Some(Value::Object(headers_map).to_string())
    } else if key_str.starts_with("response.header.") {
        let header_name = &key_str["response.header.".len()..];
        if let Some(val) = pipeline_state.response_headers.get(header_name) {
             Some(Value::String(val.to_str().unwrap_or("").to_string()).to_string())
        } else { None }
    } else if key_str == "server.metrics" {
        let m_ptr = get_server_metrics_c(arena, alloc_fn);
        if !m_ptr.is_null() {
             return m_ptr; 
        }
        None
    } else if key_str == "server.configs" {
         let c_ptr = get_all_configs_c(pipeline_state_ptr as *mut PipelineState, arena, alloc_fn);
         if !c_ptr.is_null() {
             return c_ptr;
         }
         None
    } else if key_str == "pipeline.modified" {
        Some(Value::String(pipeline_state.has_flag("content_modified").to_string()).to_string())
    } else if key_str == "pipeline.execution_history" {
        let json = serde_json::to_string(&pipeline_state.execution_history).unwrap_or("[]".to_string());
        Some(json)
    } else if key_str == "server.routes" {
        if pipeline_state.pipeline_ptr.is_null() {
            return std::ptr::null_mut();
        }
        let pipeline = unsafe { &*(pipeline_state.pipeline_ptr as *const Pipeline) };
        Some(pipeline.main_config_json.clone()) 
    } else if key_str == "server.pipeline_routing" {
        if pipeline_state.pipeline_ptr.is_null() {
            return std::ptr::null_mut();
        }
        let pipeline = &*(pipeline_state.pipeline_ptr as *const Pipeline);
        let mut routing_list = Vec::new();

        // Iterate stages to preserve execution order
        for stage in &pipeline.core.stages {
            let phase = &stage.name;
            if let Some(router_module_id) = pipeline.router_map.get(phase) {
                 let mut entry = serde_json::Map::new();
                 entry.insert("phase".to_string(), Value::String(phase.clone()));
                 
                 // Find module instance for config
                 let mut found_config = Value::Null;
                 if let Some(module) = stage.modules.iter().find(|m| m.name() == router_module_id) {
                     found_config = module.get_config();
                 }
                 
                 entry.insert("router_instance".to_string(), Value::String(router_module_id.clone()));
                 entry.insert("config".to_string(), found_config);
                 
                 routing_list.push(Value::Object(entry));
            }
        }
        
        let json = serde_json::to_string(&routing_list).unwrap_or("[]".to_string());
        return alloc_fn(arena, CString::new(json).unwrap().as_ptr());
    } else {
        // Generic State (module_context)
        pipeline_state.module_context.read().unwrap().get(key_str).map(|v| v.to_string())
    };

    if let Some(json) = val_json {
        alloc_fn(arena, CString::new(json).unwrap().as_ptr())
    } else {
        std::ptr::null_mut()
    }
}}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_state_c(
    pipeline_state_ptr: *mut c_void,
    key: *const libc::c_char,
    value_json: *const libc::c_char,
) { unsafe {
    if key.is_null() || value_json.is_null() { return; }
    let key_str = CStr::from_ptr(key).to_string_lossy().to_string();
    let val_str = CStr::from_ptr(value_json).to_string_lossy();
    
    // Value parsing
    let value: Value = if let Ok(v) = serde_json::from_str::<Value>(&val_str) {
        v
    } else {
        return; // Invalid JSON
    };

    let pipeline_state = &mut *(pipeline_state_ptr as *mut PipelineState);

    // --- Virtual Keys Setters ---
    // --- Virtual Keys Setters ---
    if key_str == "request.path" {
        if let Some(s) = value.as_str() {
            pipeline_state.request_path = s.to_string();
        }
    } else if key_str == "request.capture" {
        pipeline_state.route_capture = value.as_str().map(|s| s.to_string());
    } else if key_str == "request.source_ip" {
        if let Some(s) = value.as_str() {
             if let Ok(ip) = s.parse::<std::net::IpAddr>() {
                 pipeline_state.source_ip = std::net::SocketAddr::new(ip, pipeline_state.source_ip.port());
             }
        }
    } else if key_str == "response.status" {
        if let Some(i) = value.as_u64() {
             pipeline_state.status_code = i as u16;
        }
    } else if key_str == "response.body" {
        if let Some(s) = value.as_str() {
            pipeline_state.response_body = s.as_bytes().to_vec();
        }
    } else if key_str == "response.type" {
        if let Some(s) = value.as_str() {
            if let Ok(v) = s.parse() {
                pipeline_state.response_headers.insert(axum::http::header::CONTENT_TYPE, v);
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
    } else if key_str.starts_with("response.header.") {
         let header_name = &key_str["response.header.".len()..];
         if let Some(s) = value.as_str() {
            if let Ok(k) = axum::http::header::HeaderName::from_bytes(header_name.as_bytes()) {
                if let Ok(v) = s.parse() {
                     pipeline_state.response_headers.insert(k, v);
                }
            }
         }
    } else {
        // Generic State
        pipeline_state.module_context.write().unwrap().insert(key_str, value);
    }
}}

// =========================================================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execute_module_c(
    state_ptr: *mut c_void,
    module_id: *const libc::c_char
) -> HandlerResult {
    if state_ptr.is_null() || module_id.is_null() {
        return HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Halt,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }

    let module_id_str = unsafe { CStr::from_ptr(module_id).to_string_lossy() };
    
    // Lookup module
    let module_arc = {
        let registry = GLOBAL_MODULE_REGISTRY.read().unwrap();
        registry.get(module_id_str.as_ref()).cloned()
    };

    if let Some(module) = module_arc {
        let pipeline_state_ptr = state_ptr as *mut PipelineState;
        
        // Use shared internal execution logic
        unsafe { module.execute_internal(pipeline_state_ptr) }
    } else {
        error!("execute_module: Module '{}' not found", module_id_str);
        return HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Halt,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }
}

static MODULE_METRICS_REGISTRY: Lazy<RwLock<HashMap<String, Arc<ModuleMetrics>>>> = 
    Lazy::new(|| RwLock::new(HashMap::new()));

static GLOBAL_MODULE_REGISTRY: Lazy<RwLock<HashMap<String, Arc<LoadedModule>>>> = 
    Lazy::new(|| RwLock::new(HashMap::new()));


pub struct LoadedModule {
    _library: Arc<Library>,
    pub module_name: String,
    pub module_id: String,
    pub module_interface: Box<ModuleInterface>,
    pub module_config: ModuleConfig,
    pub metrics: Arc<ModuleMetrics>,
    pub alloc_raw: ox_webservice_api::AllocFn,
    pub phase_metric: Option<Arc<AtomicUsize>>,
}

unsafe impl Send for LoadedModule {}
unsafe impl Sync for LoadedModule {}
 
unsafe extern "C" fn alloc_string_c(arena: *const c_void, s: *const libc::c_char) -> *mut libc::c_char {
    let bump = unsafe { &*(arena as *const Bump) };
    let c_str = unsafe { CStr::from_ptr(s) };
    let bytes = c_str.to_bytes_with_nul();
    
    let layout = std::alloc::Layout::from_size_align(bytes.len(), 1).unwrap();
    let ptr = bump.alloc_layout(layout).as_ptr(); 
    
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    ptr as *mut libc::c_char
}

impl LoadedModule {
    fn name(&self) -> &str {
        &self.module_id
    }

    fn get_config(&self) -> serde_json::Value {
        let mut config_val = serde_json::to_value(&self.module_config).unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        
        // Fetch dynamic config from module
        let arena = Bump::new();
        let arena_ptr = &arena as *const Bump as *const c_void;
        
        // Attempt to call dynamic config logic if module exposes it
        // Note: we can't easily check for validity of function pointer if wrapping box is opaque, 
        // but LoadedModule builds with valid pointers.
        
        let c_json_ptr = unsafe {
            (self.module_interface.get_config)(
                self.module_interface.instance_ptr,
                arena_ptr,
                alloc_string_c
            )
        };
        
        if !c_json_ptr.is_null() {
            let c_str = unsafe { std::ffi::CStr::from_ptr(c_json_ptr) };
            if let Ok(json_str) = c_str.to_str() {
                if let Ok(dynamic_val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    // Merge dynamic into static
                    if let Some(static_obj) = config_val.as_object_mut() {
                        if let Some(dynamic_obj) = dynamic_val.as_object() {
                            for (k, v) in dynamic_obj {
                                static_obj.insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
            }
        }
        
        config_val
    }

    // Refactored execute: Assumes routing decision is already made by Pipeline
    // Returns Result for Wrapper
    fn execute_handler(&self, state: ox_pipeline::State) -> Result<ox_pipeline::PipelineStatus, String> {
        // Downcast generic state to the specific PipelineState we use
        let pipeline_state_arc = state.downcast_ref::<RwLock<PipelineState>>()
            .ok_or("Invalid State Type: Expected RwLock<PipelineState>")?;

        let pipeline_state_ptr = {
            let mut write_guard = pipeline_state_arc.write().map_err(|e| e.to_string())?;
            &mut *write_guard as *mut PipelineState
        };
        
        // --- CRITICAL: Release lock before module execution to prevent deadlocks ---
        // The raw pointer remains valid because we hold the Arc in 'state'.
        
        let result = unsafe { self.execute_internal(pipeline_state_ptr) };

        // Re-acquire lock to handle results
        let mut write_guard = pipeline_state_arc.write().map_err(|e| e.to_string())?;

        // Handle result (Wrapper responsibility)
        match result.flow_control {
             FlowControl::Halt => return Err("Halted by module".to_string()),
             FlowControl::JumpTo => {
                 let target_ptr = result.return_parameters.return_data as *const libc::c_char;
                 if !target_ptr.is_null() {
                     let target_c = unsafe { CStr::from_ptr(target_ptr) };
                     let target = target_c.to_string_lossy().into_owned();
                     debug!("Module '{}' JumpTo phase '{}'", self.module_id, target);
                     return Ok(ox_pipeline::PipelineStatus::JumpTo(target));
                 } else {
                     return Err("JumpTo requrested but no target phase provided".to_string());
                 }
             },
             FlowControl::StreamFile => {
                 let path_ptr = result.return_parameters.return_data as *mut libc::c_char;
                 if !path_ptr.is_null() {
                     let c_str = unsafe { CString::from_raw(path_ptr) };
                     if let Ok(s) = c_str.into_string() {
                         let mut ctx = write_guard.module_context.write().unwrap();
                         if let Some(existing) = ctx.get_mut("ox.response.files") {
                             if let Some(arr) = existing.as_array_mut() {
                                 arr.push(Value::String(s));
                             }
                         } else {
                             ctx.insert("ox.response.files".to_string(), Value::Array(vec![Value::String(s)]));
                         }
                     }
                 }
                 debug!("StreamFile processing complete. Pipeline content_modified={}", write_guard.has_flag("content_modified"));
                 Ok(ox_pipeline::PipelineStatus::Continue)
             },
             _ => Ok(ox_pipeline::PipelineStatus::Continue),
        }
    }

    // Unsafe Internal Execution Logic (Shared by Wrapper and C-API)
    // Takes raw pointer to PipelineState (must be valid and mutable)
    unsafe fn execute_internal(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        let write_guard = &mut *pipeline_state_ptr;

         // Check metrics gating
        let metrics_enabled = METRICS_ENABLED.load(Ordering::Relaxed);
        let start_time = if metrics_enabled { Some(std::time::Instant::now()) } else { None };

        // Module-level constraints (Headers/Query)
        // These apply regardless of which route selected this module
        // Note: Route-level headers/query are checked during dispatch
        let mut skip_module = false;
        if let Some(config_headers) = &self.module_config.headers {
            for (key, pattern) in config_headers {
                let actual = if let Some(val) = write_guard.request_headers.get(key) {
                     val.to_str().unwrap_or("")
                } else { "" };
                
                if let Ok(re) = Regex::new(pattern) {
                     if !re.is_match(actual) { skip_module = true; break; }
                }
            }
        }
        if !skip_module {
            if let Some(config_query) = &self.module_config.query {
                let query_params: HashMap<String, String> = url::form_urlencoded::parse(write_guard.request_query.as_bytes())
                    .into_owned()
                    .collect();

                for (key, pattern) in config_query {
                    let actual = query_params.get(key).map(|s| s.as_str()).unwrap_or("");
                    
                     if let Ok(re) = Regex::new(pattern) {
                         if !re.is_match(actual) { skip_module = true; break; }
                     }
                }
            }
        }
        
        if skip_module {
            return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
            };
        }

        if metrics_enabled {
            CURRENT_MODULE_METRICS.with(|m| *m.borrow_mut() = Some(self.metrics.clone()));
            self.metrics.execution_count.fetch_add(1, Ordering::Relaxed);
        }

        // Increment Phase Metric
        if let Some(pm) = &self.phase_metric {
            pm.fetch_add(1, Ordering::Relaxed);
        }

        // Call Module Handler (Defensive: Catch Panics)
        let handler_fn = self.module_interface.handler_fn;
        let instance_ptr = self.module_interface.instance_ptr;
        
        // Safety: We are passing raw pointers. catch_unwind requires AssertUnwindSafe because we are sharing mutable state
        // via raw pointers across the unwind boundary. If a panic occurs, the state in write_guard might be corrupt, 
        // but we assume the module isolate it OR we accept the risk as better than full process crash.
        // Since PipelineState is managed by us and mostly POD or std containers, it *might* survive simple panics.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (handler_fn)(
                instance_ptr,
                pipeline_state_ptr,
                self.module_interface.log_callback,
                self.alloc_raw,
                (&write_guard.arena) as *const Bump as *const c_void
            )
        })).unwrap_or_else(|_| {
            error!("Module '{}' PANICKED during execution. Replacing result with FlowControl::Halt.", self.module_id);
            HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
            }
        });

        if metrics_enabled {
             if let Some(start) = start_time {
                 let duration = start.elapsed().as_micros() as u64;
                 self.metrics.total_duration_micros.fetch_add(duration, Ordering::Relaxed);
             }
             CURRENT_MODULE_METRICS.with(|m| *m.borrow_mut() = None);
        }
        
        // Decrement Phase Metric
        if let Some(pm) = &self.phase_metric {
            pm.fetch_sub(1, Ordering::Relaxed);
        }

        // --- Host Wrapper Logic: State Latch & History ---
        
        // 1. Update State Latch (Monotonic)
        if result.status == ModuleStatus::Modified {
            write_guard.add_flag("content_modified");
            debug!("Module '{}' returned Modified. Pipeline content_modified set to true.", self.module_id);
        }

        // 2. Record Execution History
        let record = ox_webservice_api::ModuleExecutionRecord {
            module_name: self.module_id.clone(),
            status: result.status,
            flow_control: result.flow_control,
            return_data: result.return_parameters.return_data,
        };
        write_guard.execution_history.push(record);
        
        result
    }
}


struct LoadedModuleWrapper(Arc<LoadedModule>);

impl ox_pipeline::PipelineModule for LoadedModuleWrapper {
    fn name(&self) -> &str {
        &self.0.module_id
    }

    fn get_config(&self) -> serde_json::Value {
        self.0.get_config()
    }

    fn execute(&self, state: ox_pipeline::State) -> Result<ox_pipeline::PipelineStatus, String> {
        // Wrapper now calls execute_handler. 
        // Note: usage of Wrapper implies "Any request runs module", but we only use Wrapper if dispatched.
        self.0.execute_handler(state)
    }
}



#[derive(Clone)]
pub struct DispatchEntry {
    pub matcher: Option<ox_webservice_api::UriMatcher>, // None = Always Match
    pub priority: u16,
    pub module: Arc<LoadedModule>,
}

#[derive(Clone)]
pub struct Pipeline {
    pub core: Arc<ox_pipeline::Pipeline>,
    pub main_config_json: String,
    pub router_map: HashMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct RouterConfig {
    routes: Vec<RouterRouteEntry>
}
#[derive(Serialize, Deserialize)]
struct RouterRouteEntry {
    matcher: Option<ox_webservice_api::UriMatcher>,
    module_id: String,
    priority: u16
}

unsafe extern "C" fn host_render_form(
    arena: *const c_void,
    alloc_fn: AllocStrFn,
    form_def_json: *const c_char,
    props_json: *const c_char,
) -> *mut c_char {
    let form_def = CStr::from_ptr(form_def_json).to_string_lossy();
    let props_str = CStr::from_ptr(props_json).to_string_lossy();
    
    let props_val: Value = serde_json::from_str(&props_str).unwrap_or(Value::Null);

    // Call ox_forms_api
    match ox_forms_api::render_form(&form_def, &props_val) {
        Ok(html) => {
            let c_str = CString::new(html).unwrap_or_default();
            alloc_fn(arena, c_str.as_ptr())
        },
        Err(e) => {
            eprintln!("Form render error: {}", e);
            let c_str = CString::new(format!("<div class='alert alert-danger'>Error rendering form: {}</div>", e)).unwrap_or_default();
            alloc_fn(arena, c_str.as_ptr())
        }
    }
}

impl Pipeline {
    pub fn new(config: &ServerConfig, main_config_json: String) -> Result<Self, String> {
        // Initialize metrics gating
        if let Some(enabled) = config.enable_metrics {
            METRICS_ENABLED.store(enabled, Ordering::Relaxed);
        } else {
             METRICS_ENABLED.store(true, Ordering::Relaxed);
        }

        let mut phases: HashMap<String, Vec<DispatchEntry>> = HashMap::new();
        let mut loaded_libraries: HashMap<String, Arc<Library>> = HashMap::new();

        // Initialize WebServiceApiV1 struct
        let api = Box::new(WebServiceApiV1 {
            log_callback: log_callback_c,
            alloc_str: alloc_str_c,
            alloc_raw: alloc_raw_c,
            get_state: get_state_c,
            set_state: set_state_c,
            get_config: get_config_c,
            execute_module: execute_module_c,
            render_form: host_render_form,
        });

        let api_ptr = Box::leak(api) as *const WebServiceApiV1;
        let core_api_ptr = api_ptr as *const ox_webservice_api::CoreHostApi;

        // Pre-process URLs to inject into modules
        let mut extra_uris: HashMap<String, Vec<ox_webservice_api::UriMatcher>> = HashMap::new();
        for route in &config.routes {
             let url = if let Some(u) = &route.url {
                 u.clone()
             } else {
                 error!("Route definition missing 'url'. Skipping.");
                 continue;
             };

             let module_id = if let Some(mid) = &route.module_id {
                 mid.clone()
             } else {
                 error!("Route definition missing 'module_id'. Skipping.");
                 continue;
             };

             let matcher = ox_webservice_api::UriMatcher {
                 protocol: route.protocol.clone(),
                 hostname: route.hostname.clone(),
                 path: url,
                 headers: route.headers.clone(),
                 query: route.query.clone(),
                 priority: route.priority,
                 phase: route.phase.clone(),
                 status_code: route.status_code.clone(),
              };
             extra_uris.entry(module_id).or_default().push(matcher);
        }

        for module_config_ref in &config.modules {
            let mut module_config = module_config_ref.clone();
            
            let module_id = module_config.id.clone().unwrap_or(module_config.name.clone());
            if let Some(extras) = extra_uris.get(&module_id) {
                if module_config.routes.is_none() {
                    module_config.routes = Some(Vec::new());
                }
                if let Some(routes) = &mut module_config.routes {
                    routes.extend(extras.clone());
                }
                info!("Injected {} extra URI routes into module '{}'", extras.len(), module_id);
            }
            
            let lib_path = if let Some(path) = &module_config.path {
                Path::new(path).to_path_buf()
            } else {
                Path::new("target/debug").join(format!("lib{}.so", module_config.name))
            };

            info!("Loading module '{}' from {:?}", module_id, lib_path);

            // Defensive Loading: Wrap in block to capture errors without returning early from function
            let load_result = (|| -> Result<Arc<LoadedModule>, String> {
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

                let mut merged_params = module_config.extra_params.clone();
                if let Some(Value::Object(nested_params)) = module_config.params.clone() {
                    for (k, v) in nested_params {
                        merged_params.insert(k, v);
                    }
                }
                let params_json = serde_json::to_string(&merged_params).unwrap();
                let c_params_json = CString::new(params_json).unwrap();
                
                let c_module_id = CString::new(module_id.clone()).unwrap();

                // Safety: calling foreign code. We trust the module not to segfault on init.
                let module_interface_ptr = unsafe { init_fn(c_params_json.as_ptr(), c_module_id.as_ptr(), core_api_ptr) };

                if module_interface_ptr.is_null() {
                    return Err(format!("Failed to initialize module '{}' (returned null interface)", module_id));
                }

                let module_interface = unsafe { Box::from_raw(module_interface_ptr) };

                let metrics = Arc::new(ModuleMetrics::new());
                if let Ok(mut registry) = MODULE_METRICS_REGISTRY.write() {
                    registry.insert(module_id.clone(), metrics.clone());
                }

                let loaded_module = Arc::new(LoadedModule {
                    _library: lib.clone(),
                    module_name: module_config.name.clone(),
                    module_id: module_id.clone(),
                    module_interface,
                    module_config: module_config.clone(),
                    metrics, 
                    alloc_raw: alloc_raw_c,
                    phase_metric: None,
                });
                
                // Register in GLOBAL_MODULE_REGISTRY
                if let Ok(mut registry) = GLOBAL_MODULE_REGISTRY.write() {
                    registry.insert(module_id.clone(), loaded_module.clone());
                }

                Ok(loaded_module)
            })();

            match load_result {
                Ok(loaded_module) => {
                    if let Some(routes) = &module_config.routes {
                         for route in routes {
                             let entry = DispatchEntry {
                                 matcher: Some(route.clone()),
                                 priority: route.priority,
                                 module: loaded_module.clone(),
                             };
                             let target_phase = route.phase.clone().unwrap_or_else(|| "Content".to_string());
                             phases.entry(target_phase).or_default().push(entry);
                         }
                    } else {
                         // Fallback: If module has a phase specified in extra_params but no routes, add a catch-all.
                         let target_phase = module_config.extra_params.get("phase")
                             .and_then(|v| v.as_str())
                             .map(|s| s.to_string());
                         
                         if let Some(phase) = target_phase {
                             info!("Module '{}' has no routes but is in phase '{}'. Adding catch-all route.", loaded_module.module_name, phase);
                             
                             let priority = module_config.extra_params.get("priority")
                                 .and_then(|v| v.as_u64())
                                 .map(|v| v as u16)
                                 .unwrap_or(100);

                             let entry = DispatchEntry {
                                 matcher: Some(ox_webservice_api::UriMatcher {
                                     protocol: None,
                                     hostname: None,
                                     path: ".*".to_string(),
                                     headers: None,
                                     query: None,
                                     priority,
                                     phase: Some(phase.clone()),
                                     status_code: None,
                                 }),
                                 priority,
                                 module: loaded_module.clone(),
                             };
                             phases.entry(phase).or_default().push(entry);
                         } else {
                             info!("Module '{}' produces no routes and has no phase. It will strictly be available for manual dispatch.", loaded_module.module_name);
                         }
                    }
                },
                Err(e) => {
                    error!("CRITICAL: {}", e);
                    // Continue to next module
                    continue;
                }
            }
        }
        
        for entries in phases.values_mut() {
            entries.sort_by(|a, b| a.priority.cmp(&b.priority));
        }

        // Parse Execution Order and Routers
        let (execution_order, router_map) = if let Some(pipeline_config) = &config.pipeline {
             if let Some(configured_phases) = &pipeline_config.phases {
                 let mut order = Vec::new();
                 let mut routers = HashMap::new();
                 for map in configured_phases {
                     for (phase_str, router) in map {
                         // Parse Phase manually from string
                          let phase = phase_str;
                         order.push(phase.clone());
                         routers.insert(phase.clone(), router.clone());
                     }
                 }
                 (order, routers)
            } else {
                return Err("Pipeline configuration missing 'phases' definition. Execution order must be defined in config.".to_string());
            }
        } else {
            return Err("Pipeline configuration missing. Execution order must be defined in config.".to_string());
        };

        // Construct ox_pipeline::Pipeline with Dynamic Stage Routers
        let mut stages = Vec::new();
        // Populate Metric Keys first
        {
            if let Ok(mut map) = SERVER_METRICS.active_pipelines_by_phase.write() {
                for phase in &execution_order {
                    map.entry(phase.clone()).or_insert_with(|| Arc::new(AtomicUsize::new(0)));
                }
            }
        }
        
        for phase in &execution_order {
            let entries = phases.get(phase).cloned().unwrap_or_default();
            
            // Determine Router Module ID
            // Default to "ox_pipeline_router" if not specified in router_map
            let mut router_id = router_map.get(phase).cloned().unwrap_or("ox_pipeline_router".to_string());

            if router_id == "default" {
                router_id = "ox_pipeline_router".to_string();
            }
            
            // Prepare Config
            let router_config = RouterConfig {
                routes: entries.iter().map(|e| RouterRouteEntry {
                    matcher: e.matcher.clone(),
                    module_id: e.module.module_id.clone(),
                    priority: e.priority
                }).collect()
            };
            let router_config_json = serde_json::to_string(&router_config).unwrap();
            info!("Initializing router for phase {:?} with {} routes", phase, router_config.routes.len());
            let c_router_config = CString::new(router_config_json).unwrap();
            
            // Locate Router Lib
            // 1. Check if router module is defined in configuration with a custom path
            let mut router_lib_path = PathBuf::from("target/debug").join(format!("lib{}.so", router_id));
            

            
            let mut manual_routes: Vec<RouterRouteEntry> = Vec::new();
            
            for mod_cfg in &config.modules {
                if mod_cfg.name == router_id {
                    if let Some(custom_path) = &mod_cfg.path {
                        router_lib_path = PathBuf::from(custom_path);
                        info!("Using custom path for router '{}': {:?}", router_id, router_lib_path);
                    }
                    
                    // MERGE routes from params if available
                    let params_routes_opt = mod_cfg.params.as_ref().and_then(|p| p.get("routes"))
                        .or_else(|| mod_cfg.extra_params.get("routes"));
                        
                    if let Some(Value::Array(param_routes)) = params_routes_opt {
                         info!("Found {} manual routes in router '{}' configuration. Merging...", param_routes.len(), router_id);
                         for route_val in param_routes {
                             if let Ok(route) = serde_json::from_value::<RouterRouteEntry>(route_val.clone()) {
                                 manual_routes.push(route);
                             }
                         }
                    }
                    break;
                }
            }
 
             if !router_lib_path.exists() {
                 return Err(format!("Router module '{}' not found at {:?}", router_id, router_lib_path));
            }
            
             let lib = if let Some(lib) = loaded_libraries.get(router_lib_path.to_str().unwrap()) {
                 lib.clone()
            } else {
                let lib = unsafe { Library::new(&router_lib_path) }.map_err(|e| format!("Failed to load router library {:?}: {}", router_lib_path, e))?;
                let lib_arc = Arc::new(lib);
                loaded_libraries.insert(router_lib_path.to_str().unwrap().to_string(), lib_arc.clone());
                lib_arc
            };
            
            let init_fn: Symbol<InitializeModuleFn> = unsafe {
                lib.get(b"initialize_module").map_err(|e| format!("Failed to find 'initialize_module' in {:?}: {}", router_lib_path, e))?
            };
            
            // Unique ID for this router instance
            let stage_router_instance_id = format!("{:?}_Router", phase);
            let c_phase_id = CString::new(stage_router_instance_id.clone()).unwrap();
            
            // Prepare Config
            let mut all_routes: Vec<RouterRouteEntry> = entries.iter().map(|e| RouterRouteEntry {
                    matcher: e.matcher.clone(),
                    module_id: e.module.module_id.clone(),
                    priority: e.priority
            }).collect();
            
            all_routes.extend(manual_routes);
            
            // Critical: Sort routes by priority (Ascending) to ensure deterministic dispatch order.
            // HashMap sourcing makes order random otherwise.
            all_routes.sort_by(|a, b| a.priority.cmp(&b.priority));

            let router_config = RouterConfig {
                routes: all_routes
            };

            let router_config_json = serde_json::to_string(&router_config).unwrap();
            info!("Initializing router for phase {:?} with {} routes", phase, router_config.routes.len());
            let c_router_config = CString::new(router_config_json).unwrap();
            
            let module_interface_ptr = unsafe { init_fn(c_router_config.as_ptr(), c_phase_id.as_ptr(), core_api_ptr) };
             if module_interface_ptr.is_null() {
                return Err(format!("Failed to initialize router '{}' for phase {:?}", router_id, phase));
            }
            
            let module_interface = unsafe { Box::from_raw(module_interface_ptr) };
            
            // Fetch phase metric ref
            let phase_metric = {
                let map = SERVER_METRICS.active_pipelines_by_phase.read().unwrap();
                map.get(phase).cloned()
            };

            let loaded_router = Arc::new(LoadedModule {
                _library: lib.clone(),
                module_name: router_id.clone(),
                module_id: stage_router_instance_id.clone(),
                module_interface,
                module_config: Default::default(),
                metrics: Arc::new(ModuleMetrics::new()), 
                alloc_raw: alloc_raw_c,
                phase_metric,
            });
            
            // Register in Global Registries to expose via status/metrics
            if let Ok(mut registry) = GLOBAL_MODULE_REGISTRY.write() {
                registry.insert(stage_router_instance_id.clone(), loaded_router.clone());
            }
            if let Ok(mut metrics_registry) = MODULE_METRICS_REGISTRY.write() {
                metrics_registry.insert(stage_router_instance_id.clone(), loaded_router.metrics.clone());
            }
            
            stages.push(ox_pipeline::Stage {
                name: phase.clone(),
                modules: vec![Box::new(LoadedModuleWrapper(loaded_router))],
            });
        }

        // Store the router module ID used for each phase in router_map for later retrieval
        // This is separate from initial `router_map` variable which was just config. We need the final resolved mapping.
        // Wait, `router_map` variable used above *is* the mapping. It maps phase -> router name.
        // It has user configured defaults. If implicit default was used, we need to capture that too.
        
        let mut final_router_map = HashMap::new();
        // Since we iterated execution_order to build stages, we can rebuild the map or just use what we used there.
        // Let's iterate again or modify the loop. Since `stages` are built, let's just re-iterate execution order logic to be safe/consistent.
        for phase in &execution_order {
            let mut router_id = router_map.get(phase).cloned().unwrap_or("ox_pipeline_router".to_string());
            if router_id == "default" {
                router_id = "ox_pipeline_router".to_string();
            }
             // NOTE: The router's MODULE ID in the pipeline is `format!("{:?}_Router", phase)` (e.g. "Content_Router"), 
             // NOT the module name "ox_pipeline_router".
             // We need to store the *instance ID* so we can look it up in GLOBAL_MODULE_REGISTRY.
             let instance_id = format!("{:?}_Router", phase);
             final_router_map.insert(phase.clone(), instance_id);
        }

        let core_pipeline = Arc::new(ox_pipeline::Pipeline::new(stages));

        Ok(Pipeline { core: core_pipeline, main_config_json, router_map: final_router_map })
    }



    pub async fn execute_pipeline(
        self: Arc<Self>, 
        source_ip: SocketAddr, 
        method: String, 
        path: String, 
        query: String, 
        headers: HeaderMap, 
        body_data: PipelineRequestBody, 
        protocol: String
    ) -> (u16, HeaderMap, PipelineResponseBody) {
        
        let (request_body_bytes, request_body_path, request_content_length) = match body_data {
            PipelineRequestBody::Memory(bytes) => (bytes, None, 0),
            PipelineRequestBody::File(path, len) => (Vec::new(), Some(path), len),
        };
        
        let state = PipelineState {
            arena: Bump::new(),
            protocol,
            request_method: method.clone(),
            request_path: path.clone(),
            request_query: query,
            request_headers: headers,
            request_body: request_body_bytes,
            source_ip,
            status_code: 500, 
            response_headers: HeaderMap::new(),
            response_body: Vec::new(),
            module_context: Arc::new(RwLock::new(std::collections::HashMap::new())),
            pipeline_ptr: Arc::as_ptr(&self) as *const c_void,
            flags: std::collections::HashSet::new(),
            execution_history: Vec::new(),
            route_capture: None,
        };

        match state.module_context.write() {
             Ok(mut ctx) => {
                 ctx.insert("module_name".to_string(), Value::String("NONE".to_string()));
                 
                 // --- Generic Request Mapping ---
                 let verb = match method.as_str() {
                    "GET" => "get",
                    "POST" => "create", 
                    "PUT" => "update",
                    "DELETE" => "delete",
                    "WEBSOCKET" => "stream",
                    _ => "execute", 
                 };
                 let verb_norm = verb.to_string(); 
                 ctx.insert("request.verb".to_string(), serde_json::Value::String(verb_norm));
                 
                 ctx.insert("request.resource".to_string(), serde_json::Value::String(path.clone()));
                 
                  let query_params: HashMap<String, String> = url::form_urlencoded::parse(state.request_query.as_bytes())
                    .into_owned()
                    .collect();

                  let format_query = query_params.get("format").cloned();
                  if let Some(f) = format_query {
                      ctx.insert("request.format".to_string(), serde_json::Value::String(f));
                  } else if let Some(accept) = state.request_headers.get("Accept") {
                    if let Ok(s) = accept.to_str() {
                         if s.contains("application/json") {
                             ctx.insert("request.format".to_string(), serde_json::Value::String("json".to_string()));
                         } else if s.contains("text/html") {
                             ctx.insert("request.format".to_string(), serde_json::Value::String("html".to_string()));
                         }
                    }
                  }

                 // Map Body to Payload
                 // Map Body to Payload / File
                 if let Some(path) = &request_body_path {
                      ctx.insert("request.body_path".to_string(), serde_json::Value::String(path.to_string_lossy().to_string()));
                      ctx.insert("request.content_length".to_string(), serde_json::json!(request_content_length));
                 } else {
                     // Try to convert to string (UTF-8) from memory
                     if let Ok(s) = String::from_utf8(state.request_body.clone()) {
                         ctx.insert("request.payload".to_string(), serde_json::Value::String(s));
                     }
                 }
             }
             Err(e) => error!("Failed to lock module context for initialization: {}", e),
        }

        // Generic State
        let generic_state: ox_pipeline::State = Arc::new(RwLock::new(state));

        // Execute Core Pipeline
        let result = self.core.start(generic_state);

        // Recover State
        let final_state_arc = match result {
            ox_pipeline::PipelineResult::Completed(s) => s,
            ox_pipeline::PipelineResult::Aborted(reason, s) => {
                // Should we log the abort reason?
                debug!("Pipeline aborted: {}", reason);
                s
            },
        };

        // Downcast back to PipelineState
        if let Ok(state_lock) = final_state_arc.downcast::<RwLock<PipelineState>>() {
             if let Ok(mut state) = state_lock.write() {
                 // --- Generic Response Mapping ---
                 // Extract values first to avoid borrowing state while mutating it
                 let (generic_status, generic_body, generic_type) = if let Ok(ctx) = state.module_context.read() {
                     (
                         ctx.get("response.status").and_then(|v| v.as_u64()).map(|v| v as u16),
                         ctx.get("response.body").and_then(|v| v.as_str()).map(|s| s.as_bytes().to_vec()),
                         ctx.get("response.type").and_then(|v| v.as_str()).map(|s| s.to_string())
                     )
                 } else { (None, None, None) };
 
                 if let Some(code) = generic_status {
                     state.status_code = code;
                 }
                 if let Some(body) = generic_body {
                     state.response_body = body;
                 }
                 if let Some(ctype) = generic_type {
                     if let Ok(val) = ctype.parse() {
                         eprintln!("DEBUG: Recovery: generic_type = {:?},", ctype); state.response_headers.insert(axum::http::header::CONTENT_TYPE, val);
                     }
                 }
 
                 // Response Body Variant Logic
                 let body_clone = state.response_body.clone();
                 let files_list = if let Ok(ctx) = state.module_context.read() {
                      ctx.get("ox.response.files").and_then(|v| v.as_array()).map(|arr| {
                          arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<String>>()
                      })
                 } else { None };

                 let body_variant = if let Some(files) = files_list {
                      if !files.is_empty() {
                          PipelineResponseBody::Files(files.into_iter().map(PathBuf::from).collect())
                      } else {
                          PipelineResponseBody::Memory(body_clone)
                      }
                 } else if body_clone.is_empty() {
                      PipelineResponseBody::Memory(vec![])
                 } else {
                      PipelineResponseBody::Memory(body_clone)
                 };

                 (state.status_code, state.response_headers.clone(), body_variant)
             } else {
                 // Fallback if state recovery fails (Should not happen)
                 error!("Failed to recover pipeline state after execution.");
                 (500, HeaderMap::new(), PipelineResponseBody::Memory(Vec::from("Internal Server Error")))
             }
        } else {
            // Fallback if state recovery fails (Should not happen)
            error!("Failed to recover pipeline state after execution.");
            (500, HeaderMap::new(), PipelineResponseBody::Memory(Vec::from("Internal Server Error")))
        }
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
                    let (status, _, body_variant) = self.clone().execute_pipeline(
                        source_ip,
                        "WEBSOCKET".to_string(),
                        path.clone(),
                        "".to_string(),
                        HeaderMap::new(),
                        PipelineRequestBody::Memory(t.as_bytes().to_vec()),
                        protocol.clone()
                    ).await;
                    
                    if status == 200 {
                        match body_variant {
                            PipelineResponseBody::Memory(body) => {
                                if let Ok(response_text) = String::from_utf8(body) {
                                     if let Err(e) = socket.send(Message::Text(response_text)).await {
                                          error!("Failed to send WebSocket text message: {}", e);
                                          break;
                                      }
                                }
                            }
                            PipelineResponseBody::Files(_) => {
                                error!("File streaming not supported over WebSocket Text frame");
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
                      let (status, _, body_variant) = self.clone().execute_pipeline(
                         source_ip,
                         "WEBSOCKET".to_string(),
                         path.clone(),
                         "".to_string(),
                         HeaderMap::new(),
                         PipelineRequestBody::Memory(b),
                         protocol.clone()
                     ).await;
 
                     if status == 200 {
                                                   match body_variant {
                            PipelineResponseBody::Memory(body) => {
                                if let Err(e) = socket.send(Message::Binary(body)).await {
                                    error!("Failed to send WebSocket binary message: {}", e);
                                    break;
                                }
                            }
                            PipelineResponseBody::Files(_) => {
                                error!("File streaming not supported over WebSocket Binary frame");
                            }
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
 
     pub async fn execute_request(
        self: Arc<Self>,
        source_ip: SocketAddr,
        req: Request<Body>,
        protocol: String,
    ) -> Response {
        let (parts, body) = req.into_parts();
        
        // 1. Create Temp File (auto-cleanup on drop)
        // We use NamedTempFile to manage the lifecycle (deletion on drop), but we open an async handle for writing.
        let temp_file = match NamedTempFile::new() {
             Ok(f) => f,
             Err(e) => {
                 error!("Failed to create temp file: {}", e);
                 return Response::builder().status(500).body(Body::from("Internal Server Error")).unwrap();
             }
        };
        let temp_path = temp_file.path().to_owned();

        // 2. Open Async File via Tokio
        let mut async_file = match File::create(&temp_path).await {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to create async file handle: {}", e);
                 return Response::builder().status(500).body(Body::from("Internal Server Error")).unwrap();
            }
        };

        // 3. Stream Body
        let mut body_stream = body.into_data_stream();
        let mut total_len = 0u64;

        while let Some(chunk_res) = body_stream.next().await {
             match chunk_res {
                 Ok(bytes) => {
                     if let Err(e) = async_file.write_all(&bytes).await {
                          error!("Failed to write to temp file: {}", e);
                          return Response::builder().status(500).body(Body::from("Internal Server Error")).unwrap();
                     }
                     total_len += bytes.len() as u64;
                 }
                 Err(e) => {
                      error!("Error reading body stream: {}", e);
                      return Response::builder().status(500).body(Body::from("Internal Server Error")).unwrap();
                 }
             }
        }
        
        if let Err(e) = async_file.flush().await {
             error!("Failed to flush temp file: {}", e);
             return Response::builder().status(500).body(Body::from("Internal Server Error")).unwrap();
        }
        drop(async_file);
        
        let path = parts.uri.path().to_string();
        let query = parts.uri.query().unwrap_or("").to_string();
        let method = parts.method.to_string();
        let headers = parts.headers;

        let (status_code, headers, body_variant) = self.execute_pipeline(
            source_ip,
            method,
            path,
            query,
            headers,
            PipelineRequestBody::File(temp_path, total_len),
            protocol,
        ).await;

        let mut response_builder = Response::builder().status(status_code);
        for (key, value) in headers {
            if let Some(k) = key {
                response_builder = response_builder.header(k, value);
            }
        }

        match body_variant {
            PipelineResponseBody::Memory(bytes) => {
                response_builder.body(Body::from(bytes)).unwrap_or_else(|_| Response::builder().status(500).body(Body::from("Internal Server Error")).unwrap())
            }
            PipelineResponseBody::Files(files) => {
                if !files.is_empty() {
                    // Single file stream
                    let path = files[0].clone();
                    match File::open(&path).await {
                        Ok(file) => {
                             let stream = ReaderStream::new(file);
                             let body = Body::from_stream(stream);
                             response_builder.body(body).unwrap_or_else(|_| Response::builder().status(500).body(Body::from("Internal Server Error")).unwrap())
                        },
                        Err(e) => {
                             error!("Failed to open file for streaming: {:?} - {}", path, e);
                             Response::builder().status(404).body(Body::from("Not Found")).unwrap()
                        }
                    }
                } else {
                     Response::builder().status(500).body(Body::from("Internal Server Error: No files to stream")).unwrap()
                }
            }
        }
    }

 }
 
 
 #[unsafe(no_mangle)]
 pub unsafe extern "C" fn log_callback_c(level: ox_webservice_api::LogLevel, module: *const libc::c_char, message: *const libc::c_char) {
     let module = unsafe { CStr::from_ptr(module).to_string_lossy() };
     let message = unsafe { CStr::from_ptr(message).to_string_lossy() };
     match level {
         ox_webservice_api::LogLevel::Error => log::error!("[{}] {}", module, message),
         ox_webservice_api::LogLevel::Warn => log::warn!("[{}] {}", module, message),
         ox_webservice_api::LogLevel::Info => log::info!("[{}] {}", module, message),
         ox_webservice_api::LogLevel::Debug => log::debug!("[{}] {}", module, message),
         ox_webservice_api::LogLevel::Trace => log::trace!("[{}] {}", module, message),
     }
 }

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc};
    use ox_webservice_api::{ModuleConfig, ModuleInterface, LogLevel, ReturnParameters, LogCallback, AllocFn};
    use axum::http::HeaderMap;

    
    unsafe extern "C" fn mock_log(_level: LogLevel, _module: *const libc::c_char, _msg: *const libc::c_char) {
        let msg = unsafe { std::ffi::CStr::from_ptr(_msg).to_string_lossy() };
        println!("MOCK LOG: {}", msg);
    }

    unsafe extern "C" fn mock_get_config(_inst: *mut c_void, _arena: *const c_void, _alloc: ox_webservice_api::AllocStrFn) -> *mut libc::c_char {
        std::ptr::null_mut()
    }

    #[tokio::test]
    async fn test_pipeline_routing_headers_and_query() {
        // 1. Setup Mock Pipeline modules
        
        // We need a dummy library. Try using current exe or libm
        let lib_path = "libm.so.6";
        let lib_res = unsafe { Library::new(lib_path) };
        let lib = lib_res.unwrap_or_else(|_| {
             // Fallback
             unsafe { Library::new("libc.so.6") }.expect("Failed to load dummy lib")
        });
        
        let lib_arc = Arc::new(lib);
        
        // Module A: Header Matcher
        let mut config_a = ModuleConfig::default();
        config_a.name = "ModuleA".to_string();
        let mut headers = HashMap::new();
        headers.insert("x-test".to_string(), "^true$".to_string());
        config_a.headers = Some(headers);
        
        unsafe extern "C" fn handler_a(
            _instance: *mut c_void,
            state: *mut PipelineState,
            _log: LogCallback,
            _alloc: AllocFn,
            _arena: *const c_void
        ) -> HandlerResult {
            let state = unsafe { &mut *state };
            state.response_headers.insert("X-Executed-A", "1".parse().unwrap());
            HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
            }
        }
        
        unsafe extern "C" fn handler_b(
            _instance: *mut c_void,
            state: *mut PipelineState,
            _log: LogCallback,
            _alloc: AllocFn,
            _arena: *const c_void
        ) -> HandlerResult {
            let state = unsafe { &mut *state };
            state.response_headers.insert("X-Executed-B", "1".parse().unwrap());
            HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
            }
        }

        let module_a = Arc::new(LoadedModule {
            _library: lib_arc.clone(),
            module_name: "ModuleA".to_string(),
            module_id: "ModuleA".to_string(),
            module_interface: Box::new(ModuleInterface {
                instance_ptr: std::ptr::null_mut(),
                handler_fn: handler_a,
                log_callback: mock_log,
                get_config: mock_get_config,
            }),
            module_config: config_a,
            metrics: Arc::new(ModuleMetrics::new()),
            alloc_raw: alloc_raw_c,
            phase_metric: None,
        });

        // Module B: Query Matcher
        let mut config_b = ModuleConfig::default();
        config_b.name = "ModuleB".to_string();
        let mut query = HashMap::new();
        query.insert("mode".to_string(), "special".to_string());
        config_b.query = Some(query);
        
        let module_b = Arc::new(LoadedModule {
            _library: lib_arc.clone(),
            module_name: "ModuleB".to_string(),
            module_id: "ModuleB".to_string(),
            module_interface: Box::new(ModuleInterface {
                instance_ptr: std::ptr::null_mut(),
                handler_fn: handler_b,
                log_callback: mock_log,
                get_config: mock_get_config,
            }),
            module_config: config_b,
            metrics: Arc::new(ModuleMetrics::new()),
            alloc_raw: alloc_raw_c,
            phase_metric: None,
        });

        // Construct generic pipeline
        let generic_modules: Vec<Box<dyn ox_pipeline::PipelineModule>> = vec![
            Box::new(LoadedModuleWrapper(module_a)),
            Box::new(LoadedModuleWrapper(module_b)),
        ];
        
        let stage = ox_pipeline::Stage {
            name: "Content".to_string(),
            modules: generic_modules,
        };
        
        let core_pipeline = Arc::new(ox_pipeline::Pipeline::new(vec![stage]));

        let pipeline = Pipeline {
             core: core_pipeline,
             main_config_json: "{}".to_string(),
             router_map: HashMap::new(),
        };
        let pipeline_arc = Arc::new(pipeline);

        // Case 1: Header Match
        let mut headers = HeaderMap::new();
        headers.insert("x-test", "true".parse().unwrap());
        let (_, resp_headers, _) = pipeline_arc.clone().execute_pipeline(
            "127.0.0.1:8080".parse().unwrap(),
            "GET".to_string(),
            "/".to_string(),
            "".to_string(),
            headers,
            PipelineRequestBody::Memory(vec![]),
            "HTTP/1.1".to_string()
        ).await;
        
        assert!(resp_headers.contains_key("X-Executed-A"));
        assert!(!resp_headers.contains_key("X-Executed-B"));

        // Case 2: Query Match
        let (_, resp_headers_2, _) = pipeline_arc.clone().execute_pipeline(
            "127.0.0.1:8080".parse().unwrap(),
            "GET".to_string(),
            "/".to_string(),
            "mode=special".to_string(),
            HeaderMap::new(),
            PipelineRequestBody::Memory(vec![]),
            "HTTP/1.1".to_string()
        ).await;
        
        assert!(!resp_headers_2.contains_key("X-Executed-A"));
        assert!(resp_headers_2.contains_key("X-Executed-B"));

        // Case 3: Both Match
        let mut headers_3 = HeaderMap::new();
        headers_3.insert("x-test", "true".parse().unwrap());
        let (_, resp_headers_3, _) = pipeline_arc.clone().execute_pipeline(
            "127.0.0.1:8080".parse().unwrap(),
            "GET".to_string(),
            "/".to_string(),
            "mode=special".to_string(),
            headers_3,
            PipelineRequestBody::Memory(vec![]),
            "HTTP/1.1".to_string()
        ).await;
        
        assert!(resp_headers_3.contains_key("X-Executed-A"));
        // Relaxing expectation for Module B: Generic pipeline runs all modules in stage.
        assert!(resp_headers_3.contains_key("X-Executed-B")); 
        
        // Case 4: No Match
        let (_, resp_headers_4, _) = pipeline_arc.clone().execute_pipeline(
            "127.0.0.1:8080".parse().unwrap(),
            "GET".to_string(),
            "/".to_string(),
            "mode=normal".to_string(),
            HeaderMap::new(),
            PipelineRequestBody::Memory(vec![]),
            "HTTP/1.1".to_string()
        ).await;
        
        assert!(!resp_headers_4.contains_key("X-Executed-A"));
        assert!(!resp_headers_4.contains_key("X-Executed-B"));
    }
}
