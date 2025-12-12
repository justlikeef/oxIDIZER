use libc::{c_char, c_void};
use ox_webservice_api::{
    AllocFn, HandlerResult, LogCallback, LogLevel, ModuleInterface, PipelineState, WebServiceApiV1,
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
    api: &'static WebServiceApiV1,
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
    server_metrics: Option<Value>, // Additional metrics from API
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
    pub fn new(api: &'static WebServiceApiV1, config_path: Option<String>) -> Self {
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
            return HandlerResult::ModifiedJumpToError;
        }

        let pipeline_state = unsafe { &mut *pipeline_state_ptr };
        let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

        // Determine if JSON is requested
        let mut return_json = false;

        // Check Accept header
        let accept_header_key = CString::new("Accept").unwrap();
        let accept_header_ptr = unsafe {
            (self.api.get_request_header)(
                pipeline_state,
                accept_header_key.as_ptr(),
                arena_ptr,
                self.api.alloc_str,
            )
        };

        if !accept_header_ptr.is_null() {
            let accept_header = unsafe { CStr::from_ptr(accept_header_ptr).to_str().unwrap_or("") };
            if accept_header.contains("application/json") {
                return_json = true;
            }
        }

        // Check query string
        if !return_json {
            let query_ptr = unsafe {
                (self.api.get_request_query)(pipeline_state, arena_ptr, self.api.alloc_str)
            };
            if !query_ptr.is_null() {
                let query = unsafe { CStr::from_ptr(query_ptr).to_str().unwrap_or("") };
                if query.contains("format=json") {
                    return_json = true;
                }
            }
        }

        // Fetch Server Metrics
        let metrics_ptr = unsafe { (self.api.get_server_metrics)(arena_ptr, self.api.alloc_str) };
        let metrics_json: Option<Value> = if !metrics_ptr.is_null() {
             let json_str = unsafe { CStr::from_ptr(metrics_ptr).to_str().unwrap_or("{}") };
             serde_json::from_str(json_str).ok()
        } else {
            None
        };


        // Gather System Info
        let (status_output, cpu_usage_str) = {
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

            let output = StatusOutput {
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
                server_metrics: metrics_json.clone(),
            };
            
            let cpu_usage = format!("{:.2}%", sys.global_cpu_info().cpu_usage());
            (output, cpu_usage)
        };

        if return_json {
            let json_body = match serde_json::to_string(&status_output) {
                Ok(s) => s,
                Err(e) => {
                    self.log(LogLevel::Error, format!("Failed to serialize status: {}", e));
                    unsafe { (self.api.set_response_status)(pipeline_state, 500); }
                    return HandlerResult::ModifiedJumpToError;
                }
            };

            unsafe {
                let ct_k = CString::new("Content-Type").unwrap();
                let ct_v = CString::new("application/json").unwrap();
                (self.api.set_response_header)(pipeline_state, ct_k.as_ptr(), ct_v.as_ptr());

                (self.api.set_response_body)(
                    pipeline_state,
                    json_body.as_ptr(),
                    json_body.len(),
                );
            }
        } else {
            // Simple HTML
            // Try to pretty print metrics
            let metrics_html = if let Some(m) = &metrics_json {
                 format!("<pre>{}</pre>", serde_json::to_string_pretty(m).unwrap_or_default())
            } else {
                "<i>No metrics available</i>".to_string()
            };

            let html = format!(
                "<html><head><title>System Status</title></head><body>\
                <h1>System Status</h1>\
                <p><strong>Hostname:</strong> {:?}</p>\
                <p><strong>OS:</strong> {:?} {:?}</p>\
                <p><strong>Uptime:</strong> {}s</p>\
                <p><strong>CPU Usage:</strong> {}</p>\
                <p><strong>Memory:</strong> {} / {} MB</p>\
                <p><strong>Disks:</strong> {:?} disks</p>\
                <h2>Server Metrics</h2>\
                {}\
                </body></html>",
                status_output.host_name,
                status_output.system_name,
                status_output.os_version,
                status_output.uptime,
                cpu_usage_str,
                status_output.used_memory / 1024 / 1024,
                status_output.total_memory / 1024 / 1024,
                status_output.disks.len(),
                metrics_html
            );

            unsafe {
                let ct_k = CString::new("Content-Type").unwrap();
                let ct_v = CString::new("text/html").unwrap();
                (self.api.set_response_header)(pipeline_state, ct_k.as_ptr(), ct_v.as_ptr());

                (self.api.set_response_body)(
                    pipeline_state,
                    html.as_ptr(),
                    html.len(),
                );
            }
        }

        HandlerResult::ModifiedContinue
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    api_ptr: *const WebServiceApiV1,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api_instance = unsafe { &*api_ptr };

    if module_params_json_ptr.is_null() {
        let log_msg = CString::new("ox_webservice_status: module_params_json_ptr is null").unwrap();
        let module_name = CString::new(MODULE_NAME).unwrap();
        unsafe { (api_instance.log_callback)(LogLevel::Error, module_name.as_ptr(), log_msg.as_ptr()); }
        return std::ptr::null_mut();
    }

    let result = panic::catch_unwind(|| {
        let module_params_json = unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap() };
        let params: Value = serde_json::from_str(module_params_json).unwrap_or(Value::Null);

        let config_file = params.get("config_file").and_then(|v| v.as_str()).map(|s| s.to_string());

        let module = OxModule::new(api_instance, config_file);

        let instance_ptr = Box::into_raw(Box::new(module)) as *mut c_void;

        let module_interface = Box::new(ModuleInterface {
            instance_ptr,
            handler_fn: process_request_c,
            log_callback: api_instance.log_callback,
        });

        Box::into_raw(module_interface)
    });

    match result {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

unsafe extern "C" fn process_request_c(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void, 
) -> HandlerResult {
    if instance_ptr.is_null() {
        return HandlerResult::ModifiedJumpToError;
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
            HandlerResult::ModifiedJumpToError
        }
    }
}

#[cfg(test)]
mod tests;
