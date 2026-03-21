use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use std::ptr;
use axum::http::HeaderMap;
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE};
use serde_json::Value;

// ────────────────────────────────────────────────────────────────────────────
// Simple in-memory state store used by mock functions
// ────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct MockTaskState {
    pub fields: HashMap<String, String>,
}

// Global mock state – tests should use `create_task_state` instead and pass the
// raw pointer into the mock CoreHostApi.
thread_local! {
    static MOCK_STATE: RwLock<MockTaskState> = RwLock::new(MockTaskState::default());
}

// ────────────────────────────────────────────────────────────────────────────
// Mock CoreHostApi functions
// ────────────────────────────────────────────────────────────────────────────

pub extern "C" fn mock_get_field(task_ctx: *mut c_void, key: *const c_char) -> *const c_char {
    if task_ctx.is_null() || key.is_null() { return ptr::null(); }
    let key_str = unsafe { CStr::from_ptr(key).to_string_lossy().into_owned() };
    let state = unsafe { &*(task_ctx as *const RwLock<MockTaskState>) };
    let lock = state.read().unwrap();
    if let Some(v) = lock.fields.get(&key_str) {
        // Leak the string — in tests this is acceptable
        let c = CString::new(v.as_str()).unwrap();
        return c.into_raw() as *const c_char;
    }
    ptr::null()
}

pub extern "C" fn mock_set_field(task_ctx: *mut c_void, key: *const c_char, value: *const c_char) {
    if task_ctx.is_null() || key.is_null() { return; }
    let key_str = unsafe { CStr::from_ptr(key).to_string_lossy().into_owned() };
    let val_str = if value.is_null() { String::new() } else { unsafe { CStr::from_ptr(value).to_string_lossy().into_owned() } };
    let state = unsafe { &*(task_ctx as *const RwLock<MockTaskState>) };
    let mut lock = state.write().unwrap();
    lock.fields.insert(key_str, val_str);
}

pub extern "C" fn mock_get_metadata(task_ctx: *mut c_void, key: *const c_char) -> *const c_char {
    ptr::null()
}

pub extern "C" fn mock_insert_into_flow(_task_ctx: *mut c_void, _flow: *const c_char) -> bool { false }
pub extern "C" fn mock_pause_task(_task_ctx: *mut c_void, _key: *const c_char) {}
pub extern "C" fn mock_log(_task_ctx: *mut c_void, level: u8, message: *const c_char) {
    if message.is_null() { return; }
    let msg = unsafe { CStr::from_ptr(message).to_string_lossy() };
    println!("[MOCK LOG lvl={}] {}", level, msg);
}
pub extern "C" fn mock_set_flag(_task_ctx: *mut c_void, _flag: *const c_char, _scope: u8) {}
pub extern "C" fn mock_set_flags(_task_ctx: *mut c_void, _flags: *const *const c_char, _scope: u8) {}
pub extern "C" fn mock_has_flag(_task_ctx: *mut c_void, _flag: *const c_char, _scope: u8) -> bool { false }
pub extern "C" fn mock_clear_flag(_task_ctx: *mut c_void, _flag: *const c_char, _scope: u8) {}

/// Create a `CoreHostApi` pointing to mock functions.
pub fn create_mock_api() -> CoreHostApi {
    CoreHostApi {
        get_field: mock_get_field,
        set_field: mock_set_field,
        get_metadata: mock_get_metadata,
        insert_into_flow: mock_insert_into_flow,
        pause_task: mock_pause_task,
        log: mock_log,
        set_flag: mock_set_flag,
        set_flags: mock_set_flags,
        has_flag: mock_has_flag,
        clear_flag: mock_clear_flag,
    }
}

/// Create a boxed `RwLock<MockTaskState>` as a raw pointer for use as `task_ctx`.
/// Caller must free with `drop_task_state`.
pub fn create_task_state() -> *mut c_void {
    let state = Box::new(RwLock::new(MockTaskState::default()));
    Box::into_raw(state) as *mut c_void
}

pub unsafe fn drop_task_state(ptr: *mut c_void) {
    if !ptr.is_null() {
        let _ = Box::from_raw(ptr as *mut RwLock<MockTaskState>);
    }
}

/// Helper to set a field on the mock task state.
pub fn set_mock_field(task_ctx: *mut c_void, key: &str, value: &str) {
    let state = unsafe { &*(task_ctx as *const RwLock<MockTaskState>) };
    let mut lock = state.write().unwrap();
    lock.fields.insert(key.to_string(), value.to_string());
}

/// Helper to get a field from the mock task state.
pub fn get_mock_field(task_ctx: *mut c_void, key: &str) -> Option<String> {
    let state = unsafe { &*(task_ctx as *const RwLock<MockTaskState>) };
    let lock = state.read().unwrap();
    lock.fields.get(key).cloned()
}

// ────────────────────────────────────────────────────────────────────────────
// Plugin invocation helper
// ────────────────────────────────────────────────────────────────────────────

pub struct PluginHandle {
    pub config_ctx: *mut c_void,
}

impl PluginHandle {
    pub fn init(
        init_fn: unsafe extern "C" fn(*const c_char, *const CoreHostApi, u32) -> *mut c_void,
        config_json: &str,
        api: &CoreHostApi,
    ) -> Result<Self, String> {
        let c_config = CString::new(config_json).unwrap();
        let config_ctx = unsafe {
            init_fn(c_config.as_ptr(), api as *const CoreHostApi, ox_workflow_abi::OX_WORKFLOW_ABI_VERSION)
        };
        if config_ctx.is_null() {
            return Err("ox_plugin_init returned null".to_string());
        }
        Ok(Self { config_ctx })
    }

    pub fn process(
        &self,
        process_fn: unsafe extern "C" fn(*mut c_void, *mut c_void) -> FlowControl,
        task_ctx: *mut c_void,
    ) -> FlowControl {
        unsafe { process_fn(self.config_ctx, task_ctx) }
    }
}
