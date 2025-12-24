use libc::{c_void, c_char};
use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    CoreHostApi, AllocFn, AllocStrFn, PipelineState,
    ModuleStatus, FlowControl, Phase, ReturnParameters,
};
use std::ffi::{CStr, CString};
use std::panic;
use anyhow::Result;
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_webservice_restore_ip";

pub struct OxModule<'a> {
    api: &'a CoreHostApi,
}

impl<'a> OxModule<'a> {
    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe {
                (self.api.log_callback)(level, module_name.as_ptr(), c_message.as_ptr());
            }
        }
    }

    pub fn new(api: &'a CoreHostApi) -> Result<Self> {
        Ok(Self { api })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        if pipeline_state_ptr.is_null() {
            self.log(LogLevel::Error, "Pipeline state is null".to_string());
            return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::JumpTo,
                return_parameters: ReturnParameters {
                    return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                },
            };
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        let ctx = unsafe { ox_plugin::PluginContext::new(
            self.api, 
            pipeline_state_ptr as *mut c_void, 
            arena_ptr
        ) };

        // 1. Retrieve "original_source_ip" from module context
        if let Some(val) = ctx.get("original_source_ip") {
             if let Some(ip_str) = val.as_str() {
                 let _ = ctx.set("http.source_ip", serde_json::Value::String(ip_str.to_string()));
                 self.log(LogLevel::Info, format!("Restored Source IP to {}", ip_str));

                 return HandlerResult {
                    status: ModuleStatus::Modified,
                    flow_control: FlowControl::Continue,
                    return_parameters: ReturnParameters {
                        return_data: std::ptr::null_mut(),
                    },
                };
             } else {
                 self.log(LogLevel::Warn, format!("Found 'original_source_ip' but it is not a string. Value: {:?}", val));
             }
        } else {
             self.log(LogLevel::Debug, "No 'original_source_ip' found in module context. Skipping restore.".to_string());
        }

        HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    _module_params_json_ptr: *const c_char,
    _module_id: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };

    let glue = match OxModule::new(api_instance) {
        Ok(g) => g,
        Err(e) => {
             let log_msg = CString::new(format!("Failed to initialize: {}", e)).unwrap();
             let module_name = CString::new(MODULE_NAME).unwrap();
             unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
             return std::ptr::null_mut();
        }
    };
    
    let log_msg = CString::new("ox_webservice_restore_ip initialized").unwrap();
    let module_name = CString::new(MODULE_NAME).unwrap();
    unsafe { (api_instance.log_callback)(LogLevel::Info, module_name.as_ptr(), log_msg.as_ptr()); }

    let instance_ptr = Box::into_raw(Box::new(glue)) as *mut c_void;

    Box::into_raw(Box::new(ModuleInterface {
        instance_ptr,
        handler_fn: process_request_c,
        log_callback: api_instance.log_callback,
        get_config: get_config_c,
    }))
}

unsafe extern "C" fn get_config_c(
    _instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    let json = "null";
    alloc_fn(arena, CString::new(json).unwrap().as_ptr())
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void, 
) -> HandlerResult {
    if instance_ptr.is_null() {
        return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::JumpTo,
            return_parameters: ReturnParameters {
                return_data: (Phase::ErrorHandling as usize) as *mut c_void,
            },
        };
    }

    let result = panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let handler = unsafe { &*(instance_ptr as *mut OxModule) };
        handler.process_request(pipeline_state_ptr)
    }));

     match result {
        Ok(r) => r,
        Err(e) => {
             let log_msg = CString::new(format!("Panic in ox_webservice_restore_ip: {:?}", e)).unwrap();
             let module_name = CString::new(MODULE_NAME).unwrap();
             unsafe { (log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
             HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::JumpTo,
                return_parameters: ReturnParameters {
                    return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                },
             }
        }
    }
}
