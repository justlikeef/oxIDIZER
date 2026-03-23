use std::collections::HashMap;
use std::sync::RwLock;
use serde::{Deserialize, Serialize};
use lazy_static::lazy_static;
use std::ffi::{CString, CStr};
use libc::{c_char, c_void};
use ox_webservice_api::{
    ModuleInterface, PipelineState, HandlerResult,
    LogCallback, AllocFn, AllocStrFn,
    ModuleStatus, FlowControl, ReturnParameters, CoreHostApi,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionaryConfig {
    pub driver: String,
    pub parameters: HashMap<String, String>,
}

lazy_static! {
    static ref GLOBAL_DICTIONARY_CONFIG: RwLock<Option<DictionaryConfig>> = RwLock::new(None);
}

pub fn set_dictionary_config(config: DictionaryConfig) {
    let mut writer = GLOBAL_DICTIONARY_CONFIG.write().unwrap();
    *writer = Some(config);
}

pub fn get_dictionary_config() -> Option<DictionaryConfig> {
    let reader = GLOBAL_DICTIONARY_CONFIG.read().unwrap();
    reader.clone()
}

// --- Module Interface Implementation ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    _module_id: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() { return std::ptr::null_mut(); }

    let params_str = if !module_params_json_ptr.is_null() {
        unsafe { CStr::from_ptr(module_params_json_ptr).to_string_lossy().to_string() }
    } else {
        "{}".to_string()
    };
    
    // Parse config and set global dictionary key
    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&params_str) {
        if let Some(driver) = json_val.get("driver").and_then(|v| v.as_str()) {
             let mut parameters = HashMap::new();
             if let Some(params_obj) = json_val.get("parameters").and_then(|v| v.as_object()) {
                 for (k, v) in params_obj {
                     if let Some(s) = v.as_str() {
                         parameters.insert(k.clone(), s.to_string());
                     }
                 }
             }
             let config = DictionaryConfig {
                 driver: driver.to_string(),
                 parameters,
             };
             set_dictionary_config(config);
        }
    }

    let interface = Box::new(ModuleInterface {
        instance_ptr: std::ptr::null_mut(), // No instance context needed for static global config
        handler_fn: process_request,
        log_callback: unsafe { (*api_ptr).log_callback },
        get_config: get_config,
    });

    Box::into_raw(interface)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn process_request(
    _instance_ptr: *mut c_void,
    _pipeline_state_ptr: *mut PipelineState,
    _log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void,
) -> HandlerResult {
    // This module is purely configuration; it doesn't handle requests.
    HandlerResult {
        status: ModuleStatus::Unmodified,
        flow_control: FlowControl::Continue,
        return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_config(
    _instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    let config = get_dictionary_config();
    let json = serde_json::to_string(&config).unwrap_or("{}".to_string());
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
}
