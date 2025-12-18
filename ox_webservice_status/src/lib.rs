use libc::{c_char, c_void};
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogCallback, LogLevel, ModuleInterface, PipelineState, WebServiceApiV1,
    ModuleStatus, FlowControl, ReturnParameters, Phase,
};
use serde::Serialize;
use serde_json::Value;
use std::ffi::{CStr, CString};
use std::panic;
use std::sync::{Mutex};
use sysinfo::{System, Disks};
use bumpalo::Bump;

const MODULE_NAME: &str = "ox_webservice_status";

fn render_nested_metrics(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut html = String::new();
            for (key, val) in map {
                match val {
                    Value::Object(_) => {
                        // Nested Object -> Subheader + Indented Block
                        html.push_str(&format!(
                            r#"
                            <div style="margin-top: 1rem; margin-bottom: 0.5rem;">
                                <h4 style="margin: 0; color: var(--accent); font-size: 0.95rem;">{}</h4>
                                <div style="margin-left: 0.75rem; border-left: 2px solid #334155; padding-left: 0.75rem;">
                                    {}
                                </div>
                            </div>
                            "#, 
                            key, 
                            render_nested_metrics(val)
                        ));
                    },
                    Value::Array(_arr) => {
                         // Array -> Subheader + List
                        html.push_str(&format!(
                            r#"
                            <div style="margin-top: 0.5rem;">
                                <h4 style="margin: 0; color: var(--text-secondary); font-size: 0.9rem;">{} (Array)</h4>
                                <div style="margin-left: 0.75rem;">
                                    {}
                                </div>
                            </div>
                            "#, 
                            key, 
                            render_nested_metrics(val)
                        ));
                    },
                    _ => {
                        // Primitive -> Stat Row
                        let val_str = match val {
                             Value::Number(n) => n.to_string(),
                             Value::String(s) => s.clone(),
                             Value::Bool(b) => b.to_string(),
                             Value::Null => "null".to_string(),
                             _ => String::new(),
                        };
                        html.push_str(&format!(
                            r#"<div class="stat-row"><span class="label">{}</span><span class="value">{}</span></div>"#, 
                            key, val_str
                        ));
                    }
                }
            }
            html
        },
        Value::Array(arr) => {
             let mut html = String::new();
             for (i, val) in arr.iter().enumerate() {
                 let content = render_nested_metrics(val);
                 // If content is simple (just a value), display inline-ish? 
                 // If complex, block.
                 if val.is_object() || val.is_array() {
                      html.push_str(&format!(r#"<div style="margin-bottom: 0.5rem;"><strong>[{}]</strong>{}</div>"#, i, content));
                 } else {
                      html.push_str(&format!(r#"<div class="stat-row"><span class="label">[{}]</span><span class="value">{}</span></div>"#, i, content));
                 }
             }
             html
        },
         _ => {
            // Leaf Primitive (called from Array loop or root primitive? root primitive handled elsewhere usually)
            match v {
                 Value::Number(n) => n.to_string(),
                 Value::String(s) => s.clone(),
                 Value::Bool(b) => b.to_string(),
                 Value::Null => "null".to_string(),
                 _ => String::new(),
            }
        }
    }
}

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

        // Fetch Configurations
        let configs_ptr = unsafe { (self.api.get_all_configs)(pipeline_state, arena_ptr, self.api.alloc_str) };
        let configs_json: Option<Value> = if !configs_ptr.is_null() {
             let json_str = unsafe { CStr::from_ptr(configs_ptr).to_str().unwrap_or("{}") };
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
                configurations: configs_json.clone(),
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
                    return HandlerResult {
                        status: ModuleStatus::Modified,
                        flow_control: FlowControl::JumpTo,
                        return_parameters: ReturnParameters {
                            return_data: (Phase::ErrorHandling as usize) as *mut c_void,
                        },
                    };
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
            // Enhanced Premium HTML
            let metrics_html = if let Some(m) = &metrics_json {
                 if let Some(obj) = m.as_object() {
                      let mut main_metrics = String::new();
                      let mut other_metrics = String::new();
                      
                      for (k, v) in obj {
                           match v {
                               Value::Object(_) | Value::Array(_) => {
                                   let rendered_content = render_nested_metrics(v);
                                   other_metrics.push_str(&format!(
                                       r#"
                                       <div class="card">
                                            <h3>{}</h3>
                                            {}
                                       </div>
                                       "#, 
                                       k, rendered_content
                                   ));
                               },
                               _ => {
                                    let val_str = match v {
                                         Value::Number(n) => n.to_string(),
                                         Value::String(s) => s.clone(),
                                         Value::Bool(b) => b.to_string(),
                                         Value::Null => "null".to_string(),
                                         _ => serde_json::to_string(v).unwrap_or_default(),
                                    };
                                    main_metrics.push_str(&format!(
                                        r#"<div class="stat-row"><span class="label">{}</span><span class="value">{}</span></div>"#, 
                                        k, val_str
                                    ));
                               }
                           }
                      }
                      
                      let mut combined = String::new();
                      if !main_metrics.is_empty() {
                            combined.push_str(&format!(
                                r#"
                                <div class="card">
                                     <h3>General Metrics</h3>
                                     {}
                                </div>
                                "#, 
                                main_metrics
                            ));
                      }
                      combined.push_str(&other_metrics);
                      combined
                 } else {
                     match serde_json::to_string_pretty(m) {
                         Ok(s) => format!("<pre class=\"json-dump\">{}</pre>", s),
                         Err(_) => "<i>Metrics serialization failed</i>".to_string()
                     }
                 }
            } else {
                "<i>No metrics available</i>".to_string()
            };

            let disks_html: String = status_output.disks.iter().map(|d| {
                let pct = if d.total_space > 0 {
                    100.0 - (d.available_space as f64 / d.total_space as f64 * 100.0)
                } else {
                    0.0
                };
                format!(r#"
                <div style="margin-bottom: 1.25rem; border-bottom: 1px solid #334155; padding-bottom: 1rem;">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.5rem;">
                        <div style="display: flex; flex-direction: column;">
                             <strong style="font-size: 1.05rem;">{}</strong>
                             <span style="color: var(--text-secondary); font-size: 0.85rem;">{}</span>
                        </div>
                        <div style="text-align: right;">
                             <div style="font-weight: 600; font-size: 1.1rem;">{:.1}%</div>
                             <div style="color: var(--text-secondary); font-size: 0.85rem;">{:.1} / {:.1} GB</div>
                        </div>
                    </div>
                    <div class="progress-bar" style="margin-top: 0.25rem; height: 6px;">
                        <div class="progress-fill" style="width: {:.1}%"></div>
                    </div>
                </div>
                "#, 
                d.name, 
                d.mount_point, 
                pct,
                (d.total_space - d.available_space) as f64 / 1_073_741_824.0,
                d.total_space as f64 / 1_073_741_824.0,
                pct
                )
            }).collect::<Vec<String>>().join("\n");

            let configs_html = if let Some(c) = &configs_json {
                if let Some(obj) = c.as_object() {
                    let mut html = String::new();
                    for (k, v) in obj {
                         let content = render_nested_metrics(v);
                         html.push_str(&format!(
                             r#"
                             <details class="card" style="margin-bottom: 1rem;">
                                 <summary style="cursor: pointer; font-weight: bold; color: var(--accent);">{} Configuration</summary>
                                 <div style="margin-top: 1rem;">
                                    {}
                                 </div>
                             </details>
                             "#, 
                             k, content
                         ));
                    }
                    html
                } else {
                    "<p>Invalid configuration format</p>".to_string()
                }
            } else {
                "<p>No configurations available</p>".to_string()
            };


            let html = format!(
                r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>System Status | ox_webservice</title>
    <style>
        :root {{
            --bg-color: #1a1c20; /* Dark Gunmetal */
            --card-bg: #25282e;   /* Dark Steel */
            --text-primary: #e2e8f0; /* Light Steel */
            --text-secondary: #94a3b8; /* Oxidized Silver */
            --accent: #d35400;    /* Rust Orange */
            --accent-glow: rgba(211, 84, 0, 0.4);
            --border: #3f444e;    /* Structural Steel */
            --success: #27ae60;
            --warning: #f39c12;
        }}
        body {{
            font-family: 'Inter', system-ui, -apple-system, sans-serif;
            background-color: var(--bg-color);
            color: var(--text-primary);
            margin: 0;
            padding: 2rem;
            line-height: 1.5;
        }}
        .container {{
            max-width: 1200px;
            margin: 0 auto;
        }}
        header {{
            margin-bottom: 2rem;
            border-bottom: 2px solid var(--border);
            padding-bottom: 1rem;
        }}
        h1 {{
            font-weight: 800;
            font-size: 2.25rem;
            margin: 0;
            /* Metallic Rust Gradient */
            background: linear-gradient(to right, #e67e22, #d35400);
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
        }}
        h2 {{
            color: var(--text-primary);
            margin-top: 0;
            border-bottom: 1px solid var(--border);
            padding-bottom: 0.5rem;
            margin-bottom: 1rem;
        }}
        .grid {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 1.5rem;
            margin-bottom: 2rem;
        }}
        .card {{
            background-color: var(--card-bg);
            border-radius: 8px; /* Sharper corners for industrial look */
            padding: 1.5rem;
            box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.3);
            border: 1px solid var(--border);
            transition: transform 0.2s, border-color 0.2s;
        }}
        .card:hover {{
            transform: translateY(-2px);
            border-color: var(--accent);
        }}
        .stat-row {{
            display: flex;
            justify-content: space-between;
            margin-bottom: 0.75rem;
            border-bottom: 1px solid var(--border);
            padding-bottom: 0.25rem;
        }}
        .stat-row:last-child {{
            border-bottom: none;
            margin-bottom: 0;
            padding-bottom: 0;
        }}
        .label {{
            color: var(--text-secondary);
            font-weight: 500;
        }}
        .value {{
            font-weight: 600;
            font-family: 'Fira Code', monospace;
            color: var(--text-primary);
        }}
        .progress-bar {{
            height: 8px;
            background-color: #111; /* Deep Black for contrast */
            border-radius: 4px;
            margin-top: 1rem;
            overflow: hidden;
            border: 1px solid var(--border);
        }}
        .progress-fill {{
            height: 100%;
            background-color: var(--accent);
            border-radius: 2px;
            box-shadow: 0 0 10px var(--accent-glow);
        }}
        .disk-stats {{
            display: flex;
            justify-content: space-between;
            font-size: 0.875rem;
            color: var(--text-secondary);
            margin-top: 0.5rem;
        }}
        .json-dump {{
            background-color: #111;
            padding: 1rem;
            border-radius: 4px;
            overflow-x: auto;
            color: #dcdcdc;
            font-family: 'Fira Code', monospace;
            font-size: 0.875rem;
            border: 1px solid var(--border);
        }}
        .subtitle {{
            color: var(--text-secondary);
            font-size: 0.875rem;
            margin-top: -0.5rem;
            margin-bottom: 1rem;
        }}
        .status-badge {{
            background-color: rgba(39, 174, 96, 0.15);
            color: var(--success);
            padding: 0.25rem 0.75rem;
            border-radius: 4px;
            font-size: 0.875rem;
            font-weight: 600;
            border: 1px solid rgba(39, 174, 96, 0.3);
        }}
        details > summary {{
            list-style: none;
        }}
        details > summary::-webkit-details-marker {{
            display: none;
        }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <div style="display: flex; justify-content: space-between; align-items: flex-start;">
                <div style="display: flex; flex-direction: column; align-items: flex-start;">
                    <img src="/images/logo.png" alt="oxIDIZER" style="height: 8rem; margin-bottom: 0.5rem; filter: drop-shadow(0 0 5px rgba(0,0,0,0.5));">
                    <h1 style="font-size: 3rem; line-height: 1.1;">System Status</h1>
                </div>
                <span class="status-badge" style="margin-top: 1rem;">ONLINE</span>
            </div>
            <p style="color: var(--text-secondary); margin-top: 0.5rem;">{}</p>
        </header>

        <div class="grid">
            <!-- System Info Card -->
            <div class="card">
                <h2>System Info</h2>
                <div class="stat-row"><span class="label">Hostname</span><span class="value">{}</span></div>
                <div class="stat-row"><span class="label">OS</span><span class="value">{} {}</span></div>
                <div class="stat-row"><span class="label">Kernel</span><span class="value">{}</span></div>
                <div class="stat-row"><span class="label">Uptime</span><span class="value">{}s</span></div>
            </div>

            <!-- Resources Card -->
            <div class="card">
                <h2>Resources</h2>
                <div class="stat-row"><span class="label">CPU Usage</span><span class="value">{}</span></div>
                <div class="stat-row"><span class="label">CPU Cores</span><span class="value">{}</span></div>
                <div class="stat-row"><span class="label">Load Avg</span><span class="value">{:.2} / {:.2} / {:.2}</span></div>
                
                <div style="margin-top: 1.5rem;">
                    <div class="stat-row"><span class="label">Memory</span><span class="value">{:.1} / {:.1} GB</span></div>
                    <div class="progress-bar">
                        <div class="progress-fill" style="width: {:.1}%"></div>
                    </div>
                </div>
            </div>
        </div>

        <div class="card" style="margin-bottom: 2rem;">
            <h2 style="margin-top: 0; margin-bottom: 1.5rem;">Storage</h2>
            <div style="display: flex; flex-direction: column;">
                {}
            </div>
        </div>

        <h2 style="margin-bottom: 1rem;">Server Metrics</h2>
        <div class="grid">
            {}
        </div>

        <h2 style="margin-bottom: 1rem;">Configurations</h2>
        <div>
            {}
        </div>
    </div>
</body>
</html>"#,
                status_output.config_file.as_deref().unwrap_or("No config file"),
                status_output.host_name.as_deref().unwrap_or("N/A"),
                status_output.system_name.as_deref().unwrap_or("N/A"),
                status_output.os_version.as_deref().unwrap_or(""),
                status_output.kernel_version.as_deref().unwrap_or("N/A"),
                status_output.uptime,
                cpu_usage_str,
                status_output.cpu_count,
                status_output.load_average.one,
                status_output.load_average.five,
                status_output.load_average.fifteen,
                status_output.used_memory as f64 / 1_073_741_824.0,
                status_output.total_memory as f64 / 1_073_741_824.0,
                (status_output.used_memory as f64 / status_output.total_memory as f64) * 100.0,

                disks_html,
                metrics_html,
                configs_html
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

        HandlerResult {
            status: ModuleStatus::Modified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters {
                return_data: std::ptr::null_mut(),
            },
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    _module_id: *const c_char,
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
            get_config: get_config_c,
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
