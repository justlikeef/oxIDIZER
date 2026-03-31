use rumqttd::Broker;
use std::ffi::{c_char, c_void, CStr, CString};
use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_INFO, OX_LOG_ERROR,
};
use std::thread;
use std::time::Duration;
use std::net::TcpStream;

const MODULE_NAME: &str = "ox_messaging_mqtt";

pub struct ModuleContext {
    #[allow(dead_code)]
    api: CoreHostApi,
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

fn get_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> Option<Vec<u8>> {
    let c_key = CString::new(key).unwrap();
    let mut len: usize = 0;
    let ptr = (api.get_field_bytes)(task_ctx, c_key.as_ptr(), &mut len as *mut usize);
    if ptr.is_null() || len == 0 { return None; }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

fn set_field_bytes_data(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
    let c_key = CString::new(key).unwrap();
    (api.set_field_bytes)(task_ctx, c_key.as_ptr(), data.as_ptr(), data.len());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };

    let params_str = if !plugin_config_ctx.is_null() {
        unsafe { CStr::from_ptr(plugin_config_ctx).to_string_lossy().to_string() }
    } else { "{}".to_string() };

    let params: serde_json::Value = serde_json::from_str(&params_str).unwrap_or(serde_json::json!({}));

    let config_path_str = params.get("config_file").and_then(|v| v.as_str())
        .unwrap_or("ox_messaging_mqtt/conf/broker.yaml").to_string();
    let broker_port: Option<u16> = params.get("broker_port").and_then(|v| v.as_u64()).map(|v| v as u16);
    let console_port: Option<u16> = params.get("console_port").and_then(|v| v.as_u64()).map(|v| v as u16);
    let max_connections: Option<usize> = params.get("max_connections").and_then(|v| v.as_u64()).map(|v| v as usize);

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, &format!("{}: Starting MQTT broker...", MODULE_NAME));

    thread::spawn(move || {
        let config_path = std::path::Path::new(&config_path_str);
        let mut config: rumqttd::Config = if config_path.exists() {
            match ox_fileproc::process_file(config_path, 5) {
                Ok(v) => serde_json::from_value(v).unwrap_or_else(|_| default_config()),
                Err(_) => default_config(),
            }
        } else {
            default_config()
        };

        if let Some(p) = broker_port {
            for server in config.v4.values_mut() {
                server.listen = format!("0.0.0.0:{}", p).parse().expect("Invalid addr");
            }
        }
        if let Some(cp) = console_port {
            config.console.listen = format!("0.0.0.0:{}", cp).parse().expect("Invalid console addr");
        }
        if let Some(m) = max_connections {
            config.router.max_connections = m;
        }

        let mut broker = Broker::new(config);
        if let Err(e) = broker.start() {
            eprintln!("{}: Broker error: {}", MODULE_NAME, e);
        }
    });

    let check_port = broker_port.unwrap_or(1883);
    let check_addr = format!("127.0.0.1:{}", check_port);
    let mut up = false;
    for _ in 0..10 {
        if TcpStream::connect(&check_addr).is_ok() { up = true; break; }
        thread::sleep(Duration::from_millis(500));
    }

    if !up {
        log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("{}: Broker failed to start on port {}", MODULE_NAME, check_port));
        return std::ptr::null_mut();
    }

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, &format!("{}: MQTT broker started on port {}", MODULE_NAME, check_port));

    let ctx = Box::new(ModuleContext { api });
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) -> FlowControl {
    // MQTT broker runs in background; no per-request processing needed yet.
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

fn default_config() -> rumqttd::Config {
    let default_json = serde_json::json!({
        "id": 0,
        "router": {
            "max_segment_size": 1048576,
            "max_segment_count": 10,
            "max_connections": 100,
            "max_outgoing_packet_count": 200
        },
        "v4": {
            "v4.1": {
                "name": "v4-1",
                "listen": "0.0.0.0:1883",
                "next_connection_delay_ms": 1,
                "connections": {
                    "max_payload_size": 2048,
                    "max_inflight_count": 100,
                    "connection_timeout_ms": 100
                }
            }
        },
        "console": { "listen": "0.0.0.0:3030" }
    });
    serde_json::from_value(default_json).unwrap_or_default()
}
