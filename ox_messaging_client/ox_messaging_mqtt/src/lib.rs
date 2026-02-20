use rumqttd::{Broker, Config};
use std::ffi::{c_char, c_void, CStr, CString};
use ox_webservice_api::{
    AllocFn, AllocStrFn, HandlerResult, LogCallback, LogLevel, ModuleInterface, PipelineState, 
    ModuleStatus, FlowControl, ReturnParameters, CoreHostApi
};
use std::thread;
use std::time::Duration;
use std::net::TcpStream;

const MODULE_NAME: &str = "ox_messaging";

pub struct OxModule {
    api: &'static CoreHostApi,
    module_id: String,
}

impl OxModule {
    pub fn new(api: &'static CoreHostApi, module_id: String) -> Self {
        Self { api, module_id }
    }
}

// Boilerplate C-exports
#[no_mangle]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    module_id_ptr: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = &*api_ptr;

    let module_id = if !module_id_ptr.is_null() {
        CStr::from_ptr(module_id_ptr).to_string_lossy().to_string()
    } else {
        MODULE_NAME.to_string()
    };

    // Initialize Logging
    let _ = ox_webservice_api::init_logging(api.log_callback, &module_id);
    let log_msg = CString::new("Starting Broker...").unwrap();
    let mod_name_c = CString::new(module_id.clone()).unwrap();
    (api.log_callback)(LogLevel::Info, mod_name_c.as_ptr(), log_msg.as_ptr());
    eprintln!("OxMessagingMqtt: Initializing...");

    // Parse params immediately (Main thread is unsafe context, pointers valid here)
    let params_str = unsafe {
         if module_params_json_ptr.is_null() {
             "{}"
         } else {
             std::ffi::CStr::from_ptr(module_params_json_ptr).to_str().unwrap_or("{}")
         }
    };
    
    // Parse to ensure we have an owned copy or just use string
    // Let's create an owned String to move into thread
    let params_string = params_str.to_string();
    
    // Parse params immediately (Main thread is unsafe context, pointers valid here)
    let params: serde_json::Value = serde_json::from_str(&params_string).unwrap_or(serde_json::json!({}));
    
    let config_path_str = params.get("config_file").and_then(|v| v.as_str()).unwrap_or("ox_messaging_mqtt/conf/broker.yaml").to_string();
    let broker_port = params.get("broker_port").and_then(|v| v.as_u64()).map(|v| v as u16);
    let console_port = params.get("console_port").and_then(|v| v.as_u64()).map(|v| v as u16);
    let max_connections = params.get("max_connections").and_then(|v| v.as_u64()).map(|v| v as usize);
    
    // START BROKER IN BACKGROUND
    let broker_port_for_thread = broker_port;
    let console_port_for_thread = console_port;
    let max_connections_for_thread = max_connections;
    thread::spawn(move || {
        // Parse params to get config file path
        
        let config_path = std::path::Path::new(&config_path_str);
        
        let mut config: rumqttd::Config = if config_path.exists() {
             match ox_fileproc::process_file(&config_path, 5) { // Max depth 5
                 Ok(value) => {
                     // ox_fileproc returns serde_json::Value.
                     match serde_json::from_value(value) {
                         Ok(c) => c,
                         Err(e) => {
                             eprintln!("Failed to deserialize config from {}: {}. Using default.", config_path_str, e);
                             default_config()
                         }
                     }
                 },
                 Err(e) => {
                     eprintln!("ox_fileproc processing failed for {}: {}. Using default.", config_path_str, e);
                     default_config()
                 }
             }
        } else {
             eprintln!("Config file {} not found. Using default.", config_path_str);
             default_config()
        };

        // Apply Overrides from main config
        if let Some(p) = broker_port_for_thread {
            eprintln!("OxMessagingMqtt: Overriding ports with broker_port={}", p);
            // v4 (HashMap<String, ServerSettings>)
            for server in config.v4.values_mut() {
                server.listen = format!("0.0.0.0:{}", p).parse().expect("Invalid v4 SocketAddr");
                eprintln!("OxMessagingMqtt: Updated v4 server.listen to {:?}", server.listen);
            }
            // v5 (Option<HashMap<String, ServerSettings>>)
            if let Some(v5) = config.v5.as_mut() {
                for server in v5.values_mut() {
                    server.listen = format!("0.0.0.0:{}", p).parse().expect("Invalid v5 SocketAddr");
                    eprintln!("OxMessagingMqtt: Updated v5 server.listen to {:?}", server.listen);
                }
            }
            // ws (Option<HashMap<String, ServerSettings>>)
            if let Some(ws) = config.ws.as_mut() {
                for server in ws.values_mut() {
                    server.listen = format!("0.0.0.0:{}", p).parse().expect("Invalid ws SocketAddr");
                    eprintln!("OxMessagingMqtt: Updated ws server.listen to {:?}", server.listen);
                }
            }
        }

        if let Some(cp) = console_port_for_thread {
            eprintln!("OxMessagingMqtt: Overriding console port with console_port={}", cp);
            config.console.listen = format!("0.0.0.0:{}", cp).parse().expect("Invalid console SocketAddr");
        }

        if let Some(m) = max_connections_for_thread {
            config.router.max_connections = m;
        }

        let mut broker = Broker::new(config);
        if let Err(e) = broker.start() {
             eprintln!("Broker implementation error: {}", e);
        }
    });

    // BLOCK UNTIL BROKER IS UP
    let mut up = false;
    let check_port = broker_port.unwrap_or(1883);
    let check_addr = format!("127.0.0.1:{}", check_port);
    for i in 0..10 {
         if TcpStream::connect(&check_addr).is_ok() {
             up = true;
             break;
         }
         thread::sleep(Duration::from_millis(500));
         let log_retry = CString::new(format!("Waiting for broker on {} (attempt {})...", check_addr, i)).unwrap();
         (api.log_callback)(LogLevel::Info, mod_name_c.as_ptr(), log_retry.as_ptr());
    }

    if !up {
         let log_err = CString::new(format!("Broker failed to start on port {}", check_port)).unwrap();
         (api.log_callback)(LogLevel::Error, mod_name_c.as_ptr(), log_err.as_ptr());
         return std::ptr::null_mut(); // Failed init
    }

    let module = OxModule::new(api, module_id);
    let instance_ptr = Box::into_raw(Box::new(module)) as *mut c_void;

    Box::into_raw(Box::new(ModuleInterface {
        instance_ptr,
        handler_fn: process_request_c,
        log_callback: api.log_callback,
        get_config: get_config_c,
    }))
}

unsafe extern "C" fn process_request_c(
    _instance_ptr: *mut c_void,
    _pipeline_state_ptr: *mut PipelineState,
    _log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void, 
) -> HandlerResult {
    // Messaging module doesn't handle HTTP requests yet, just serves as broker.
    // Future: Admin API here.
    HandlerResult {
        status: ModuleStatus::Unmodified,
        flow_control: FlowControl::Continue,
        return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
    }
}

unsafe extern "C" fn get_config_c(
    _instance_ptr: *mut c_void,
    _arena: *const c_void,
    _alloc_fn: AllocStrFn,
) -> *mut c_char {
    std::ptr::null_mut()
}

fn default_config() -> rumqttd::Config {
    let mut config = rumqttd::Config::default();
    config.id = 0;
    
    // Router config
    // config.router.id = 0; // Does not exist in 0.17?
    // config.router.dir = "/tmp/rumqttd-default".to_string(); // Does not exist
    config.router.max_segment_size = 1048576;
    config.router.max_segment_count = 10;
    config.router.max_connections = 100;
    config.router.max_outgoing_packet_count = 200;

    // V4 Listener
    // Note: rumqttd::Config structure for v4 might be map of maps?
    // Based on broker.toml structure: [v4] -> [v4.1]
    // In Rust Config struct: 
    // pub v4: Option<HashMap<String, Listener>> (Not quite, it's HashMap<String, HashMap<String, ListenerConfig>>?)
    // Actually, expecting `v4: HashMap<String, HashMap<String, ...>>` is complex.
    // Let's use serde_json to construct it safely based on the known working JSON schema I saw earlier!
    
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
        "console": {
            "listen": "0.0.0.0:3030"
        }
    });

    serde_json::from_value(default_json).unwrap_or_else(|e| {
        eprintln!("Critical Error constructing default config: {}", e);
        rumqttd::Config::default()
    })
}
