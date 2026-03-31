use std::sync::{Mutex};
use sysinfo::{System, Disks, Pid};
use serde::Serialize;
use std::ffi::{c_char, c_void, CStr, CString};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_INFO, OX_LOG_ERROR
};

const MODULE_NAME: &str = "ox_webservice_status";

pub struct ModuleContext {
    system: Mutex<System>,
    disks: Mutex<Disks>,
    api: CoreHostApi,
    server_config_json: Option<String>,
}

#[derive(Serialize)]
struct StatusOutput {
    system: SystemInfo,
    server: ServerInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    workflow_routing: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    configurations: Option<serde_json::Value>,
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

/// Build workflow_routing and configurations from the stored server config JSON.
fn build_config_data(config_json: &str) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    let Ok(cfg) = serde_json::from_str::<serde_json::Value>(config_json) else {
        return (None, None);
    };

    // Build workflow_routing: ordered array of { phase, router_instance, config }
    let workflow_routing = cfg.get("workflow")
        .and_then(|w| w.get("stages"))
        .and_then(|s| s.as_array())
        .map(|stages| {
            let items: Vec<serde_json::Value> = stages.iter().filter_map(|stage| {
                let phase = stage.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown");
                let router_plugin = stage.get("plugins")
                    .and_then(|p| p.as_array())
                    .and_then(|plugins| plugins.iter().find(|p| {
                        p.get("name").and_then(|n| n.as_str()) == Some("ox_webservice_router")
                    }))?;
                Some(serde_json::json!({
                    "phase": phase,
                    "router_instance": "ox_webservice_router",
                    "config": router_plugin.get("config").cloned().unwrap_or(serde_json::Value::Null)
                }))
            }).collect();
            serde_json::Value::Array(items)
        });

    // Build configurations: array of { name, config }
    let configurations = cfg.get("modules")
        .and_then(|m| m.as_array())
        .map(|modules| {
            let items: Vec<serde_json::Value> = modules.iter().filter_map(|m| {
                let name = m.get("id")
                    .and_then(|v| v.as_str())
                    .or_else(|| m.get("name").and_then(|v| v.as_str()))
                    .unwrap_or("unknown");
                if name == "ox_webservice_router" { return None; }
                let config = m.get("params").cloned()
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                Some(serde_json::json!({ "name": name, "config": config }))
            }).collect();
            serde_json::Value::Array(items)
        });

    (workflow_routing, configurations)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };
    if let Ok(c) = CString::new(format!("{} initialized", MODULE_NAME)) {
        (api.log)(std::ptr::null_mut(), OX_LOG_INFO, c.as_ptr());
    }

    let server_config_json = if !plugin_config_ctx.is_null() {
        let params_str = unsafe { CStr::from_ptr(plugin_config_ctx).to_string_lossy().to_string() };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&params_str) {
            v.get("_server_config_json")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        } else {
            None
        }
    } else {
        None
    };

    let ctx = Box::new(ModuleContext {
        system: Mutex::new(System::new_all()),
        disks: Mutex::new(Disks::new_with_refreshed_list()),
        api,
        server_config_json,
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

    let query_val = get_field(api, task_ctx, "request.query");
    let accept_val = get_field(api, task_ctx, "request.header.accept");
    let return_json = query_val.contains("format=json") || accept_val.contains("application/json");

    if !return_json {
        log(api, task_ctx, OX_LOG_INFO, "Status: Non-JSON request, skipping");
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    log(api, task_ctx, OX_LOG_INFO, "Status: Returning JSON status report");

    let mut status_output = {
        let pid = Pid::from_u32(std::process::id());
        let mut sys = context.system.lock().unwrap();
        // Refresh only what we need — refresh_all() scans every /proc entry which
        // can take several seconds on WSL2, blocking the tokio runtime.
        sys.refresh_memory();
        sys.refresh_cpu_usage();
        sys.refresh_process(pid);
        let load_avg = System::load_average();
        let process_info = sys.process(pid).map(|p| ProcessInfo {
            uptime_seconds: p.run_time(),
            memory_bytes: p.memory(),
            virtual_memory_bytes: p.virtual_memory(),
            cpu_usage: p.cpu_usage(),
        });
        let cpu_count = sys.cpus().len();
        let memory = MemoryInfo {
            total: sys.total_memory(),
            used: sys.used_memory(),
            swap_total: sys.total_swap(),
            swap_used: sys.used_swap(),
        };
        drop(sys); // release system lock before acquiring disks lock

        let mut disks = context.disks.lock().unwrap();
        disks.refresh_list();
        let disk_infos: Vec<DiskInfo> = disks.list().iter().map(|d| DiskInfo {
            name: d.name().to_string_lossy().to_string(),
            mount_point: d.mount_point().to_string_lossy().to_string(),
            total_space: d.total_space(),
            available_space: d.available_space(),
        }).collect();
        drop(disks);
        StatusOutput {
            system: SystemInfo {
                host_name: System::host_name(),
                kernel_version: System::kernel_version(),
                os_version: System::os_version(),
                uptime: System::uptime(),
                cpu_count,
                load_average: LoadAvg { one: load_avg.one, five: load_avg.five, fifteen: load_avg.fifteen },
                memory,
                disks: disk_infos,
            },
            server: ServerInfo { process: process_info },
            workflow_routing: None,
            configurations: None,
        }
    };

    let (workflow_routing, configurations) = context.server_config_json.as_deref()
        .map(build_config_data)
        .unwrap_or((None, None));
    status_output.workflow_routing = workflow_routing;
    status_output.configurations = configurations;

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
