use ox_pipeline_plugin::{
    CoreHostApi, FlowControl, HandlerResult, LogCallback, ModuleStatus, ReturnParameters,
    AllocFn, AllocStrFn, LogLevel,
};
use ox_webservice_api::{PipelineState, ModuleInterface, init_logging};
use serde::Deserialize;
use std::ffi::{c_char, c_void, CStr, CString};
use std::net::IpAddr;
use ipnetwork::IpNetwork;
use std::sync::{Arc, Mutex};

// --- Configuration ---

#[derive(Deserialize, Debug)]
struct Config {
    config_file: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            config_file: "conf/ip_rules.yaml".to_string(), // Default path relative to CWD
        }
    }
}

#[derive(Deserialize, Debug)]
struct RulesConfig {
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
    #[serde(default = "default_order")]
    order: String,
    #[serde(default)]
    jump_target: Option<String>,
}

fn default_order() -> String {
    "order: deny, allow".to_string() 
}

impl Default for RulesConfig {
    fn default() -> Self {
        RulesConfig {
            allow: Vec::new(),
            deny: Vec::new(),
            order: "deny, allow".to_string(),
            jump_target: None,
        }
    }
}

// --- Module State ---

struct ModuleState {
    allow: Vec<IpNetwork>,
    deny: Vec<IpNetwork>,
    order_allow_deny: bool, // true: Allow,Deny; false: Deny,Allow
    jump_target: String,
}

impl ModuleState {
    fn load(config: Config) -> Self {
        let path = std::path::Path::new(&config.config_file);
        
        let rules = match ox_fileproc::process_file(path, 5) {
            Ok(value) => {
                match serde_json::from_value::<RulesConfig>(value) {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("Failed to parse rules from {}: {}", config.config_file, e);
                        RulesConfig::default()
                    }
                }
            },
            Err(e) => {
                log::error!("Failed to load config file {}: {}", config.config_file, e);
                RulesConfig::default()
            }
        };

        let parse_networks = |list: Vec<String>| -> Vec<IpNetwork> {
            list.into_iter()
                .filter_map(|s| {
                    if let Ok(net) = s.parse::<IpNetwork>() {
                        Some(net)
                    } else if let Ok(ip) = s.parse::<IpAddr>() {
                        Some(IpNetwork::from(ip))
                    } else {
                        log::error!("Invalid IP/CIDR in rules: {}", s);
                        None
                    }
                })
                .collect()
        };

        let allow = parse_networks(rules.allow);
        let deny = parse_networks(rules.deny);
        
        // Parse order
        // user example: order: "allow, deny"
        // Cleanup string: remove "order:", trim
        let order_clean = rules.order.replace("order:", "").trim().to_lowercase();
        let order_allow_deny = order_clean.contains("allow") && order_clean.find("allow") < order_clean.find("deny");
        
        let jump_target = rules.jump_target.unwrap_or("ErrorHandling".to_string());

        ModuleState {
            allow,
            deny,
            order_allow_deny,
            jump_target,
        }
    }

    fn check(&self, ip: IpAddr) -> bool {
        let matches_allow = self.allow.iter().any(|net| net.contains(ip));
        let matches_deny = self.deny.iter().any(|net| net.contains(ip));

        if self.order_allow_deny {
            // Order Allow, Deny
            // First Allow, then Deny. Default Deny.
            // Allowed if (InAllow) AND (NotInDeny)
            matches_allow && !matches_deny
        } else {
            // Order Deny, Allow
            // First Deny, then Allow. Default Allow.
            // Allowed if (InAllow) OR (NotInDeny)
            matches_allow || !matches_deny
        }
    }
}

// Global state wrapped in Arc/Mutex
static mut STATE: Option<Arc<Mutex<ModuleState>>> = None;

// --- Interface ---

unsafe extern "C" fn noop_log(_level: LogLevel, _module: *const c_char, _msg: *const c_char) {}

#[no_mangle]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    _module_id: *const c_char,
    _api: *const CoreHostApi,
) -> *mut ModuleInterface {
    let params_json = unsafe { CStr::from_ptr(module_params_json_ptr).to_str().unwrap_or("{}") };
    let config: Config = serde_json::from_str(params_json).unwrap_or_else(|e| {
        eprintln!("ox_auth_ip: Failed to parse params: {}. Using default.", e);
        Config::default()
    });

    let state = ModuleState::load(config);
    unsafe {
        STATE = Some(Arc::new(Mutex::new(state)));
    }

    Box::into_raw(Box::new(ModuleInterface {
        instance_ptr: std::ptr::null_mut(),
        handler_fn: handle_request,
        log_callback: noop_log,
        get_config: get_config_schema,
    }))
}

pub unsafe extern "C" fn get_config_schema(
    _state: *mut c_void, 
    _arena: *const c_void, 
    _alloc_fn: AllocStrFn
) -> *mut c_char {
    let schema = r#"{
        "type": "object",
        "properties": {
            "config_file": { "type": "string" }
        }
    }"#;
    
    let c_str = CString::new(schema).unwrap();
    c_str.into_raw()
}

pub unsafe extern "C" fn handle_request(
    _instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    log_callback: LogCallback,
    alloc_fn: AllocFn,
    arena: *const c_void,
) -> HandlerResult {
    let _ = init_logging(log_callback, "ox_auth_ip");
    let pipeline = &mut *pipeline_state_ptr;
    
    // Safety: Accessing global state
    let state_lock = match unsafe { &*std::ptr::addr_of!(STATE) } {
        Some(s) => s.lock().unwrap(),
        None => {
            log::error!("Module state not initialized");
            return HandlerResult {
                status: ModuleStatus::Modified, 
                flow_control: FlowControl::Halt,
                return_parameters: ReturnParameters { return_data: std::ptr::null_mut() },
            };
        }
    };

    let client_ip = pipeline.source_ip.ip();
    log::debug!("Checking access for IP: {}", client_ip);

    if state_lock.check(client_ip) {
         log::debug!("Access ALLOWED for {}", client_ip);
         HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        }
    } else {
        log::warn!("Access DENIED for {}", client_ip);
        pipeline.status_code = 403;
        
        let target = &state_lock.jump_target;
        let c_target = CString::new(target.clone()).unwrap();
        let bytes = c_target.as_bytes_with_nul();
        
        // Allocate in arena
        let ptr = alloc_fn(arena as *mut c_void, bytes.len(), 1);
        if !ptr.is_null() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
        }

        HandlerResult {
            status: ModuleStatus::Modified, 
            flow_control: FlowControl::JumpTo,
            return_parameters: ReturnParameters { return_data: ptr }
        }
    }
}
