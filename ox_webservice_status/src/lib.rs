use libc::{c_char, c_void};
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogCallback, LogLevel, ModuleInterface, PipelineState, 
    ModuleStatus, FlowControl, ReturnParameters, Phase, CoreHostApi, WebServiceApiV1
};
use serde::Serialize;
use serde_json::Value;
use std::ffi::{CStr, CString};
use std::panic;
use std::sync::{Mutex};
use sysinfo::{System, Disks};
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_webservice_status";

pub struct OxModule {
    system: Mutex<System>,
    disks: Mutex<Disks>,
    api: &'static CoreHostApi,
    config_path: Option<String>,
}

#[derive(Serialize)]
struct StatusOutput {
    system_name: Option<String>,
    kernel_version: Option<String>,
    os_version: Option<String>,
    host_name: Option<String>,
    uptime: u64,
    cpu_count: usize,
    load_average: LoadAvg,
    total_memory: u64,
    used_memory: u64,
    total_swap: u64,
    used_swap: u64,
    disks: Vec<DiskInfo>,
    config_file: Option<String>,
    server_metrics: Option<Value>,
    configurations: Option<Value>,
}

#[derive(Serialize)]
struct LoadAvg {
    one: f64,
    five: f64,
    fifteen: f64,
}

#[derive(Serialize)]
struct DiskInfo {
    name: String,
    mount_point: String,
    total_space: u64,
    available_space: u64,
}

impl OxModule {
    pub fn new(api: &'static CoreHostApi, config_path: Option<String>) -> Self {
        Self {
            system: Mutex::new(System::new_all()),
            disks: Mutex::new(Disks::new_with_refreshed_list()),
            api,
            config_path,
        }
    }

    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(MODULE_NAME).unwrap();
            unsafe {
                (self.api.log_callback)(level, module_name.as_ptr(), c_message.as_ptr());
            }
        }
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

        // Initialize PluginContext
        let ctx = unsafe { ox_plugin::PluginContext::new(
            self.api, 
            pipeline_state_ptr as *mut c_void, 
            arena_ptr
        ) };
        
        let path_json = ctx.get("http.request.path");
        let query_json = ctx.get("http.request.query");
        let accept_json = ctx.get("http.request.header.Accept");

        let path = path_json.and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
        let query = query_json.and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
        let accept = accept_json.and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();

        // Determine mode
        let return_json = query.contains("format=json") || accept.contains("application/json");

        if !return_json {
            // Serve Static Assets
            let asset_path = if path == "/status" || path == "/status/" {
                "index.html".to_string()
            } else if path.starts_with("/status/") {
                 path.trim_start_matches("/status/").to_string()
            } else {
                "index.html".to_string() 
            };
            
            // Map file extension to content type
            let content_type = if asset_path.ends_with(".css") {
                "text/css"
            } else if asset_path.ends_with(".js") {
                "application/javascript"
            } else {
                "text/html"
            };

            let _ = ctx.set("http.response.header.Content-Type", serde_json::Value::String(content_type.to_string()));
            
            // Allocate path for StreamFile return
            let c_path = ctx.alloc_string(&asset_path);

            return HandlerResult {
                status: ModuleStatus::Modified,
                flow_control: FlowControl::StreamFile,
                return_parameters: ReturnParameters { return_data: c_path as *mut c_void },
            };
        }

        // --- JSON Mode ---

        // Fetch Server Metrics via ox_plugin generic helpers
        let metrics_json = ctx.get("server.metrics");

        // Fetch Configurations via ox_plugin generic helpers
        let configs_json = ctx.get("server.configs");

        let status_output = {
            let mut sys = self.system.lock().unwrap();
            let mut disks = self.disks.lock().unwrap();
            
            sys.refresh_all();
            disks.refresh_list();

            let load_avg = System::load_average();

            let disk_infos: Vec<DiskInfo> = disks.list().iter().map(|d| DiskInfo {
                name: d.name().to_string_lossy().to_string(),
                mount_point: d.mount_point().to_string_lossy().to_string(),
                total_space: d.total_space(),
                available_space: d.available_space(),
            }).collect();

            StatusOutput {
                system_name: System::name(),
                kernel_version: System::kernel_version(),
                os_version: System::os_version(),
                host_name: System::host_name(),
                uptime: System::uptime(),
                cpu_count: sys.cpus().len(),
                load_average: LoadAvg {
                    one: load_avg.one,
                    five: load_avg.five,
                    fifteen: load_avg.fifteen,
                },
                total_memory: sys.total_memory(),
                used_memory: sys.used_memory(),
                total_swap: sys.total_swap(),
                used_swap: sys.used_swap(),
                disks: disk_infos,
                config_file: self.config_path.clone(),
                server_metrics: metrics_json,
                configurations: configs_json,
            }
        };
        
        // Return JSON content
        let json_body = match serde_json::to_string(&status_output) {
            Ok(s) => s,
            Err(e) => {
                self.log(LogLevel::Error, format!("Failed to serialize status: {}", e));
                 let _ = ctx.set("http.response.status", serde_json::json!(500));
                return HandlerResult {
                    status: ModuleStatus::Modified,
                    flow_control: FlowControl::JumpTo,
                    return_parameters: ReturnParameters {
                        return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                    },
                };
            }
        };

        // Use ox_plugin set for response
        let _ = ctx.set("http.response.body", serde_json::Value::String(json_body));
        let _ = ctx.set("http.response.status", serde_json::json!(200));
        let _ = ctx.set("http.response.header.Content-Type", serde_json::Value::String("application/json".to_string()));
        
        HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue, 
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    _module_id: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };
    
    // Params optional
    let module_params_json = if !module_params_json_ptr.is_null() {
        unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap_or("{}") }
    } else {
        "{}"
    };
    
    let params: Value = serde_json::from_str(module_params_json).unwrap_or(Value::Null);
    let config_file = params.get("config_file").and_then(|v| v.as_str()).map(|s| s.to_string());

    let module = OxModule::new(api_instance, config_file);
    let instance_ptr = Box::into_raw(Box::new(module)) as *mut c_void;

    Box::into_raw(Box::new(ModuleInterface {
        instance_ptr,
        handler_fn: process_request_c,
        log_callback: api_instance.log_callback,
        get_config: get_config_c,
    }))
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
        Ok(handler_result) => handler_result,
        Err(e) => {
             let log_msg = CString::new(format!("Panic in ox_webservice_status: {:?}", e)).unwrap();
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

unsafe extern "C" fn get_config_c(
    instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    if instance_ptr.is_null() { return std::ptr::null_mut(); }
    let handler = unsafe { &*(instance_ptr as *mut OxModule) };
    
    let mut map = serde_json::Map::new();
    if let Some(path) = &handler.config_path {
        map.insert("config_file".to_string(), Value::String(path.clone()));
    }
    
    let json = serde_json::to_string_pretty(&Value::Object(map)).unwrap_or("{}".to_string());
    alloc_fn(arena, CString::new(json).unwrap().as_ptr())
}

#[cfg(test)]
mod tests;
