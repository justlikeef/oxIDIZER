use libc::{c_char, c_void};
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogCallback, LogLevel, ModuleInterface, PipelineState,
    ModuleStatus, FlowControl, ReturnParameters, CoreHostApi,
    UriMatcher
};
use serde::Serialize;
use serde_json::Value;
use std::ffi::{CStr, CString};
use std::panic;
use std::sync::{Mutex};
use sysinfo::{System, Disks, Pid};
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_webservice_status";

pub struct OxModule {
    system: Mutex<System>,
    disks: Mutex<Disks>,
    api: &'static CoreHostApi,
    config_path: Option<String>,
    module_id: String,
    // start_time removed, using sysinfo
}

#[derive(Serialize)]
struct StatusOutput {
    system: SystemInfo,
    server: ServerInfo,
    pipeline_routing: Option<Value>,
    configurations: Option<Value>,
}

#[derive(Serialize)]
struct SystemInfo {
    host_name: Option<String>,
    kernel_version: Option<String>,
    os_version: Option<String>,
    uptime: u64,
    cpu_count: usize,
    load_average: LoadAvg,
    memory: MemoryInfo,
    disks: Vec<DiskInfo>,
}

#[derive(Serialize)]
struct ServerInfo {
    process: Option<ProcessInfo>,
    metrics: Option<Value>,
}

#[derive(Serialize)]
struct ProcessInfo {
    uptime_seconds: u64,
    memory_bytes: u64,
    virtual_memory_bytes: u64,
    cpu_usage: f32,
}


#[derive(Serialize)]
struct MemoryInfo {
    total: u64,
    used: u64,
    swap_total: u64,
    swap_used: u64,
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
    pub fn new(api: &'static CoreHostApi, _config_path: Option<String>, module_id: String) -> Self {
        let _ = ox_webservice_api::init_logging(api.log_callback, &module_id);
        Self {
            system: Mutex::new(System::new_all()),
            disks: Mutex::new(Disks::new_with_refreshed_list()),
            api,
            config_path: None, // Deprecated in status output
            module_id,
        }
    }
    


    fn log(&self, level: LogLevel, message: String) {
        if let Ok(c_message) = CString::new(message) {
            let module_name = CString::new(self.module_id.clone()).unwrap_or(CString::new(MODULE_NAME).unwrap());
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
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters {
                    return_data: std::ptr::null_mut(),
                },
             };
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        // Initialize PipelineContext
        let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
            self.api, 
            pipeline_state_ptr as *mut c_void, 
            arena_ptr
        ) };
        
        // Generic Request State detection
        let format_val = ctx.get("request.format");
        
        let mut return_json = false;
        if let Some(val) = format_val {
            if let Some(f) = val.as_str() {
                if f == "json" { return_json = true; }
            }
        }
        
        // Fallback for strict tests or unexpected headers (though pipeline.rs should handle it now)
        if !return_json {
            if let Some(accept) = ctx.get("request.header.Accept") {
                if let Some(s) = accept.as_str() {
                    if s.contains("application/json") { return_json = true; }
                }
            }
        }

        if !return_json {
             self.log(LogLevel::Info, "Status: Non-JSON request, skipping to downstream (e.g. static HTML)".to_string());
             return HandlerResult {
                status: ModuleStatus::Unmodified,
                flow_control: FlowControl::Continue, 
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
            };
        }

        self.log(LogLevel::Info, "Status: Returning JSON status report".to_string());

        // --- JSON Mode ---

        // Fetch Server Metrics
        let metrics_json = ctx.get("server.metrics");

        // Fetch Pipeline Routing (With Metrics)
        let routing_json = ctx.get("server.pipeline_routing");

        // Fetch Module Configurations
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

            // Refresh and get process info
            let pid = Pid::from_u32(std::process::id());
            sys.refresh_process(pid);
            
            let process_info = sys.process(pid).map(|p| ProcessInfo {
                uptime_seconds: p.run_time(),
                memory_bytes: p.memory(),
                virtual_memory_bytes: p.virtual_memory(),
                cpu_usage: p.cpu_usage(),
            });

            StatusOutput {
                system: SystemInfo {
                    host_name: System::host_name(),
                    kernel_version: System::kernel_version(),
                    os_version: System::os_version(),
                    uptime: System::uptime(),
                    cpu_count: sys.cpus().len(),
                    load_average: LoadAvg {
                        one: load_avg.one,
                        five: load_avg.five,
                        fifteen: load_avg.fifteen,
                    },
                    memory: MemoryInfo {
                        total: sys.total_memory(),
                        used: sys.used_memory(),
                        swap_total: sys.total_swap(),
                        swap_used: sys.used_swap(),
                    },
                    disks: disk_infos,
                },
                server: ServerInfo {
                    process: process_info,
                    metrics: metrics_json,
                },
                pipeline_routing: routing_json,
                configurations: configs_json,
            }
        };
        
        // Return JSON content
        let json_body = match serde_json::to_string(&status_output) {
            Ok(s) => s,
            Err(e) => {
                self.log(LogLevel::Error, format!("Failed to serialize status: {}", e));
                 let _ = ctx.set("response.status", serde_json::json!(500));
                return HandlerResult {
                    status: ModuleStatus::Modified,
                    flow_control: FlowControl::Continue,
                    return_parameters: ReturnParameters {
                        return_data: std::ptr::null_mut(),
                    },
                };
            }
        };

        // Use generic keys
        let _ = ctx.set("response.body", serde_json::Value::String(json_body));
        let _ = ctx.set("response.status", serde_json::json!(200));
        let _ = ctx.set("response.type", serde_json::Value::String("application/json".to_string()));

               return HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    module_id_ptr: *const c_char,
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

    let module_id = if !module_id_ptr.is_null() {
        unsafe { CStr::from_ptr(module_id_ptr).to_string_lossy().to_string() }
    } else {
        MODULE_NAME.to_string()
    };

    let module = OxModule::new(api_instance, config_file, module_id);
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
            flow_control: FlowControl::Continue,
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
        Ok(handler_result) => handler_result,
        Err(e) => {
             let log_msg = CString::new(format!("Panic in ox_webservice_status: {:?}", e)).unwrap();
             
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
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
}

#[cfg(test)]
mod tests;
