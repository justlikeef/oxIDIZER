use std::ffi::{c_char, c_void, CStr, CString};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// --- Generic Types ---

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowControl {
    Continue = 0,
    NextPhase = 1,
    JumpTo = 2,
    Halt = 3,
    StreamFile = 4,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleStatus {
    Unmodified = 0,
    Modified = 1,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnParameters {
    pub return_data: *mut c_void,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandlerResult {
    pub status: ModuleStatus,
    pub flow_control: FlowControl,
    pub return_parameters: ReturnParameters,
}

// Function Pointer Types
pub type LogCallback = unsafe extern "C" fn(level: LogLevel, module: *const c_char, message: *const c_char);
pub type AllocStrFn = unsafe extern "C" fn(arena: *const c_void, s: *const c_char) -> *mut c_char;
pub type AllocFn = unsafe extern "C" fn(arena: *mut c_void, size: usize, align: usize) -> *mut c_void;
pub type GetStateFn = unsafe extern "C" fn(state: *mut c_void, key: *const c_char, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type SetStateFn = unsafe extern "C" fn(state: *mut c_void, key: *const c_char, value_json: *const c_char);
pub type GetConfigFn = unsafe extern "C" fn(state: *mut c_void, arena: *const c_void, alloc_fn: AllocStrFn) -> *mut c_char;
pub type ExecuteModuleFn = unsafe extern "C" fn(state: *mut c_void, module_id: *const c_char) -> HandlerResult;

// --- Core Host API ---
// This is the generic interface that any host (ox_webservice, etc.) must implement.
#[repr(C)]
pub struct CoreHostApi {
    pub log_callback: LogCallback,
    pub alloc_str: AllocStrFn,
    pub alloc_raw: AllocFn,
    pub get_state: GetStateFn,
    pub set_state: SetStateFn,
    pub get_config: GetConfigFn,
    pub execute_module: ExecuteModuleFn,
}

// --- Plugin Context ---
// Safe wrapper for modules.

pub struct PipelineContext<'a> {
    pub api: &'a CoreHostApi,
    pub state_ptr: *mut c_void, // Opaque state pointer
    pub arena_ptr: *const c_void, // Opaque arena pointer
}

impl<'a> PipelineContext<'a> {
    /// Creates a new PipelineContext.
    /// 
    /// # Safety
    /// pointers must be valid.
    pub unsafe fn new(
        api: &'a CoreHostApi,
        state_ptr: *mut c_void,
        arena_ptr: *const c_void,
    ) -> Self {
        Self { api, state_ptr, arena_ptr }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let c_key = CString::new(key).ok()?;
        let ptr = unsafe { (self.api.get_state)(self.state_ptr, c_key.as_ptr(), self.arena_ptr, self.api.alloc_str) };
        if ptr.is_null() {
            return None;
        }
        unsafe {
            let s = CStr::from_ptr(ptr).to_string_lossy();
            serde_json::from_str(&s).ok()
        }
    }

    pub fn set(&self, key: &str, value: Value) -> Result<(), String> {
        let c_key = CString::new(key).map_err(|e| e.to_string())?;
        let json_str = serde_json::to_string(&value).map_err(|e| e.to_string())?;
        let c_val = CString::new(json_str).map_err(|e| e.to_string())?;

        unsafe {
            (self.api.set_state)(self.state_ptr, c_key.as_ptr(), c_val.as_ptr());
        }
        Ok(())
    }
    
    // Helpers relying on convention
    pub fn get_config(&self) -> Option<Value> {
         let ptr = unsafe { (self.api.get_config)(self.state_ptr, self.arena_ptr, self.api.alloc_str) };
         if ptr.is_null() { return None; }
         unsafe {
             let s = CStr::from_ptr(ptr).to_string_lossy();
             serde_json::from_str(&s).ok()
         }
    }

    pub fn execute_module(&self, module_id: &str) -> HandlerResult {
        let c_id = match CString::new(module_id) {
            Ok(c) => c,
            Err(_) => return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
            },
        };
        unsafe { (self.api.execute_module)(self.state_ptr, c_id.as_ptr()) }
    }

    /// Allocates a string in the host's arena.
    pub fn alloc_string(&self, s: &str) -> *mut c_char {
        let c_str = match CString::new(s) {
            Ok(c) => c,
            Err(_) => return std::ptr::null_mut(),
        };
        unsafe { (self.api.alloc_str)(self.arena_ptr, c_str.as_ptr()) }
    }
}

// Re-export common types
pub mod types {
    use super::*;
    // Any other helpers
}
