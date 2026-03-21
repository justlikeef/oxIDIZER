use std::sync::{Mutex};
use sysinfo::{System, Disks, Pid};
use serde::Serialize;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_INFO, OX_LOG_ERROR
};

const MODULE_NAME: &str = "ox_webservice_status";

pub struct ModuleContext {
    system: Mutex<System>,
    disks: Mutex<Disks>,
    api: CoreHostApi,
}

#[derive(Serialize)]
struct StatusOutput {
    system: SystemInfo,
    server: ServerInfo,
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

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let res_ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if res_ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(res_ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    _plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };
    if let Ok(c) = CString::new(format!("{} initialized", MODULE_NAME)) {
        (api.log)(std::ptr::null_mut(), OX_LOG_INFO, c.as_ptr());
    }
    let ctx = Box::new(ModuleContext {
        system: Mutex::new(System::new_all()),
        disks: Mutex::new(Disks::new_with_refreshed_list()),
        api,
    });
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    if plugin_config_ctx.is_null() {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }
    let context = unsafe { &*(plugin_config_ctx as *mut ModuleContext) };
    let api = &context.api;

    let format_val = get_field(api, task_ctx, "request.format");
    let accept_val = get_field(api, task_ctx, "request.header.Accept");
    let return_json = format_val == "json" || accept_val.contains("application/json");

    if !return_json {
        log(api, task_ctx, OX_LOG_INFO, "Status: Non-JSON request, skipping");
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    log(api, task_ctx, OX_LOG_INFO, "Status: Returning JSON status report");

    let status_output = {
        let mut sys = context.system.lock().unwrap();
        let mut disks = context.disks.lock().unwrap();
        sys.refresh_all();
        disks.refresh_list();
        let load_avg = System::load_average();
        let disk_infos: Vec<DiskInfo> = disks.list().iter().map(|d| DiskInfo {
            name: d.name().to_string_lossy().to_string(),
            mount_point: d.mount_point().to_string_lossy().to_string(),
            total_space: d.total_space(),
            available_space: d.available_space(),
        }).collect();
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
                load_average: LoadAvg { one: load_avg.one, five: load_avg.five, fifteen: load_avg.fifteen },
                memory: MemoryInfo { total: sys.total_memory(), used: sys.used_memory(), swap_total: sys.total_swap(), swap_used: sys.used_swap() },
                disks: disk_infos,
            },
            server: ServerInfo { process: process_info },
        }
    };

    match serde_json::to_string(&status_output) {
        Ok(json_body) => {
            set_field(api, task_ctx, "response.body", &json_body);
            set_field(api, task_ctx, "response.status", "200");
            set_field(api, task_ctx, "response.header.Content-Type", "application/json");
        }
        Err(e) => {
            log(api, task_ctx, OX_LOG_ERROR, &format!("Failed to serialize status: {}", e));
            set_field(api, task_ctx, "response.status", "500");
        }
    }

    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = Box::from_raw(plugin_config_ctx as *mut ModuleContext);
    }
}

#[cfg(test)]
mod tests;
