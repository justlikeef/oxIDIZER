use libc::{c_void, c_char};
use ox_webservice_api::{
    HandlerResult, LogCallback, LogLevel, ModuleInterface,
    CoreHostApi, WebServiceApiV1, AllocFn, AllocStrFn, PipelineState,
    ModuleStatus, FlowControl, ReturnParameters,
};
use std::ffi::{CStr, CString};
use std::panic;
use anyhow::Result;
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_webservice_forwarded_for";

pub struct OxModule<'a> {
    api: &'a CoreHostApi,
    module_id: String,
}

impl<'a> OxModule<'a> {
    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(self.module_id.clone()).unwrap_or(CString::new(MODULE_NAME).unwrap());
            unsafe {
                (self.api.log_callback)(level, module_name.as_ptr(), c_message.as_ptr());
            }
        }
    }

    pub fn new(api: &'a CoreHostApi, module_id: String) -> Result<Self> {
        let _ = ox_webservice_api::init_logging(api.log_callback, &module_id);
        Ok(Self { api, module_id })
    }

    pub fn process_request(&self, pipeline_state_ptr: *mut PipelineState) -> HandlerResult {
        if pipeline_state_ptr.is_null() {
            self.log(LogLevel::Error, "Pipeline state is null".to_string());
            // Safe fallback
            return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            };
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;
        
        let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
            self.api, 
            pipeline_state_ptr as *mut c_void, 
            arena_ptr
        ) };

        // 1. Get X-Forwarded-For header via Generic State
        let header_val = match ctx.get("request.header.X-Forwarded-For") {
             Some(val) => val.as_str().unwrap_or("").to_string(),
             None => String::new(),
        };

        if header_val.is_empty() {
             return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
            };
        }

        // 2. Parse the FIRST IP from the list (standard practice)
        // X-Forwarded-For: <client>, <proxy1>, <proxy2>
        let new_client_ip = match header_val.split(',').next() {
            Some(ip) => ip.trim().to_string(),
            None => {
                 return HandlerResult {
                    status: ModuleStatus::Unmodified,
                    flow_control: FlowControl::Continue,
                    return_parameters: ReturnParameters {
                        return_data: std::ptr::null_mut(),
                    },
                };
            }
        };

        // 3. Store the *original* source IP in module context for potential restoration
        // Get current source IP (try generic first)
        let current_ip = match ctx.get("request.source_ip") {
            Some(val) => val.as_str().unwrap_or("unknown").to_string(),
            None => match ctx.get("request.source_ip") {
                 Some(val) => val.as_str().unwrap_or("unknown").to_string(),
                 None => "unknown".to_string(),
            }
        };

        let _ = ctx.set("original_source_ip", serde_json::Value::String(current_ip.clone()));

        // 4. Update the Source IP in PipelineState via Generic State
        // Use generic key; pipeline now maps this to state.source_ip
        let _ = ctx.set("request.source_ip", serde_json::Value::String(new_client_ip.clone()));

        self.log(LogLevel::Info, format!("Updated Source IP from {} to {} based on X-Forwarded-For: {}", current_ip, new_client_ip, header_val));

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
    module_id_ptr: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };

    let module_id = if !module_id_ptr.is_null() {
        unsafe { CStr::from_ptr(module_id_ptr).to_string_lossy().to_string() }
    } else {
        MODULE_NAME.to_string()
    };
    let c_module_id = CString::new(module_id.clone()).unwrap();

    let glue = match OxModule::new(api_instance, module_id) {
        Ok(g) => g,
        Err(e) => {
             let log_msg = CString::new(format!("Failed to initialize: {}", e)).unwrap();
             unsafe { (api_instance.log_callback)(LogLevel::Error, c_module_id.as_ptr(), log_msg.as_ptr()); }
             return std::ptr::null_mut();
        }
    };
    
    // Log initialization
    let log_msg = CString::new("ox_webservice_forwarded_for initialized").unwrap();
    unsafe { (api_instance.log_callback)(LogLevel::Info, c_module_id.as_ptr(), log_msg.as_ptr()); }


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
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
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
            flow_control: FlowControl::Halt,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
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
             let log_msg = CString::new(format!("Panic in ox_webservice_forwarded_for: {:?}", e)).unwrap();
             
             let handler_unsafe = unsafe { &*(instance_ptr as *mut OxModule) };
             let module_name = CString::new(handler_unsafe.module_id.clone()).unwrap_or(CString::new(MODULE_NAME).unwrap());
             
             unsafe { (log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
             HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
             }
        }
    }
}
