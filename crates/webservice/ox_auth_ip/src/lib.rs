use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_JUMP,
    OX_LOG_DEBUG, OX_LOG_ERROR, OX_LOG_WARN,
};
use serde::Deserialize;
use std::ffi::{c_char, c_void, CStr, CString};
use std::net::IpAddr;
use ipnetwork::IpNetwork;
use std::sync::{Arc, Mutex};

const MODULE_NAME: &str = "ox_auth_ip";

#[derive(Deserialize, Debug)]
struct Config {
    #[serde(default = "default_config_file")]
    config_file: String,
}

fn default_config_file() -> String { "conf/ip_rules.yaml".to_string() }

#[derive(Deserialize, prost::Message, Clone)]
struct RulesConfig {
    #[serde(default)]
    #[prost(string, repeated, tag = "1")]
    allow: Vec<String>,
    #[serde(default)]
    #[prost(string, repeated, tag = "2")]
    deny: Vec<String>,
    #[serde(default = "default_order")]
    #[prost(string, tag = "3")]
    order: String,
    #[serde(default)]
    #[prost(string, optional, tag = "4")]
    jump_target: Option<String>,
}

fn default_order() -> String { "deny, allow".to_string() }

struct IpRules {
    allow: Vec<IpNetwork>,
    deny: Vec<IpNetwork>,
    order_allow_deny: bool,
    jump_target: String,
}

impl IpRules {
    fn load(config_file: &str) -> Self {
        let path = std::path::Path::new(config_file);
        let rules: RulesConfig = match ox_fileproc::process_file(path, 5) {
            Ok(v) => serde_json::from_value(v).unwrap_or_default(),
            Err(_) => RulesConfig::default(),
        };

        let parse = |list: Vec<String>| -> Vec<IpNetwork> {
            list.into_iter().filter_map(|s| {
                s.parse::<IpNetwork>().ok()
                    .or_else(|| s.parse::<IpAddr>().ok().map(IpNetwork::from))
            }).collect()
        };

        let order_clean = rules.order.replace("order:", "").trim().to_lowercase();
        let order_allow_deny = order_clean.starts_with("allow");

        IpRules {
            allow: parse(rules.allow),
            deny: parse(rules.deny),
            order_allow_deny,
            jump_target: rules.jump_target.unwrap_or_else(|| "ErrorHandling".to_string()),
        }
    }

    fn check(&self, ip: IpAddr) -> bool {
        let in_allow = self.allow.iter().any(|net| net.contains(ip));
        let in_deny = self.deny.iter().any(|net| net.contains(ip));

        if self.order_allow_deny {
            in_allow && !in_deny
        } else {
            in_allow || !in_deny
        }
    }
}

pub struct ModuleContext {
    rules: Arc<Mutex<IpRules>>,
    api: CoreHostApi,
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

    let config: Config = serde_json::from_str(&params_str).unwrap_or(Config { config_file: default_config_file() });
    let rules = IpRules::load(&config.config_file);

    log(&api, std::ptr::null_mut(), OX_LOG_DEBUG, &format!("{} initialized with {} allow / {} deny rules", MODULE_NAME, rules.allow.len(), rules.deny.len()));

    let ctx = Box::new(ModuleContext {
        rules: Arc::new(Mutex::new(rules)),
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

    let source_ip_str = get_field(api, task_ctx, "request.source_ip");
    let client_ip: IpAddr = match source_ip_str.parse() {
        Ok(ip) => ip,
        Err(_) => {
            log(api, task_ctx, OX_LOG_ERROR, &format!("Invalid source IP: {}", source_ip_str));
            set_field(api, task_ctx, "response.status", "403");
            return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        }
    };

    let rules = context.rules.lock().unwrap();
    if rules.check(client_ip) {
        log(api, task_ctx, OX_LOG_DEBUG, &format!("Access ALLOWED for {}", client_ip));
        FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
    } else {
        log(api, task_ctx, OX_LOG_WARN, &format!("Access DENIED for {}", client_ip));
        set_field(api, task_ctx, "response.status", "403");

        let target = CString::new(rules.jump_target.clone()).unwrap();
        let payload = target.into_raw() as *const c_char;
        FlowControl { code: FLOW_CONTROL_JUMP, payload }
    }
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
