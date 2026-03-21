use std::ffi::{CStr, CString};
use std::sync::Arc;
use libc::{c_char, c_void};
use serde::{Deserialize, Serialize};
use regex::Regex;
use std::collections::HashMap;

use ox_webservice_api::{
    CoreHostApi, FlowControl, ModuleConfig, UriMatcher,
    FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_DEBUG
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RouterConfig {
    #[serde(default)]
    pub routes: Vec<RouterRouteEntry>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RouterRouteEntry {
    pub matcher: Option<UriMatcher>,
    pub module_id: String,
    pub priority: u16
}

pub struct RouterModule {
    config: RouterConfig,
    compiled_routes: Vec<CompiledRoute>,
}

struct CompiledRoute {
    matcher_path_regex: Option<Regex>,
    matcher_header_regexes: HashMap<String, Regex>,
    matcher_query_regexes: HashMap<String, Regex>,
    module_id: String,
    matcher_protocol: Option<String>,
    matcher_hostname_regex: Option<Regex>,
    matcher_status_code_regex: Option<Regex>,
}

impl RouterModule {
    pub fn new(config: RouterConfig) -> Result<Self, String> {
        let mut compiled_routes = Vec::new();

        for route in &config.routes {
            let mut path_regex = None;
            let mut header_regexes = HashMap::new();
            let mut query_regexes = HashMap::new();
            let mut matcher_hostname_regex = None;
            let mut matcher_status_code_regex = None;

            if let Some(matcher) = &route.matcher {
                if !matcher.path.is_empty() {
                    path_regex = Some(Regex::new(&matcher.path).map_err(|e| format!("Invalid path regex '{}': {}", matcher.path, e))?);
                }
                
                if let Some(host) = &matcher.hostname {
                    matcher_hostname_regex = Some(Regex::new(host).map_err(|e| format!("Invalid hostname regex '{}': {}", host, e))?);
                }
                if let Some(sc) = &matcher.status_code {
                     matcher_status_code_regex = Some(Regex::new(sc).map_err(|e| format!("Invalid status code regex '{}': {}", sc, e))?);
                }

                if let Some(headers) = &matcher.headers {
                    for (k, v) in headers {
                        let re = Regex::new(v).map_err(|e| format!("Invalid header regex for '{}': {}", k, e))?;
                        header_regexes.insert(k.clone(), re);
                    }
                }

                if let Some(query) = &matcher.query {
                    for (k, v) in query {
                        let re = Regex::new(v).map_err(|e| format!("Invalid query regex for '{}': {}", k, e))?;
                        query_regexes.insert(k.clone(), re);
                    }
                }
            }

            compiled_routes.push(CompiledRoute {
                matcher_path_regex: path_regex,
                matcher_header_regexes: header_regexes,
                matcher_query_regexes: query_regexes,
                module_id: route.module_id.clone(),
                matcher_protocol: route.matcher.as_ref().and_then(|m| m.protocol.clone()),
                matcher_hostname_regex,
                matcher_status_code_regex,
            });
        }

        Ok(RouterModule {
            config,
            compiled_routes,
        })
    }
}

pub struct ModuleContext {
    module: Arc<RouterModule>,
    api: CoreHostApi,
    _module_id: String,
}

// Helper to interact with CoreHostApi
fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let res_ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if res_ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(res_ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    let c_key = CString::new(key).unwrap();
    let c_val = CString::new(value).unwrap();
    (api.set_field)(task_ctx, c_key.as_ptr(), c_val.as_ptr());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api = unsafe { *api_ptr };

    let params_str = if !plugin_config_ctx.is_null() {
        unsafe { CStr::from_ptr(plugin_config_ctx).to_string_lossy().to_string() }
    } else {
        "{}".to_string()
    };
    
    let config: RouterConfig = serde_json::from_str(&params_str).unwrap_or(RouterConfig { routes: vec![] });
    
    let module = match RouterModule::new(config) {
        Ok(m) => Arc::new(m),
        Err(e) => {
             let msg = CString::new(format!("Failed to init router: {}", e)).unwrap();
             (api.log)(std::ptr::null_mut(), OX_LOG_ERROR, msg.as_ptr());
             return std::ptr::null_mut();
        }
    };

    let ctx = Box::new(ModuleContext {
        module,
        api,
        _module_id: "ox_webservice_router".to_string(),
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

    let protocol = get_field(api, task_ctx, "request.protocol");
    let original_path = get_field(api, task_ctx, "request.path");

    for route in &context.module.compiled_routes {
        if let Some(proto) = &route.matcher_protocol {
            if !proto.eq_ignore_ascii_case(&protocol) && !protocol.is_empty() { continue; }
        }

        if let Some(re) = &route.matcher_hostname_regex {
            let host_val = get_field(api, task_ctx, "request.header.Host");
            if !re.is_match(&host_val) { continue; }
        }

        let mut path_match_capture = None;
        if let Some(re) = &route.matcher_path_regex {
            if let Some(caps) = re.captures(&original_path) {
                if let Some(m) = caps.get(1) {
                    path_match_capture = Some(m.as_str().to_string());
                }
            } else {
                continue;
            }
        }

        let mut headers_match = true;
        for (key, re) in &route.matcher_header_regexes {
            let val = if key.eq_ignore_ascii_case("Method") {
                 get_field(api, task_ctx, "request.method")
            } else {
                let lookup_key = format!("request.header.{}", key);
                let mut v = get_field(api, task_ctx, &lookup_key);
                if v.is_empty() {
                    let lower_key = format!("request.header.{}", key.to_lowercase());
                    v = get_field(api, task_ctx, &lower_key);
                }
                v
            };

            if !re.is_match(&val) {
                headers_match = false;
                break;
            }
        }
        if !headers_match { continue; }

        let mut query_match = true;
        for (key, re) in &route.matcher_query_regexes {
            let lookup_key = format!("request.query.{}", key);
            let mut val = get_field(api, task_ctx, &lookup_key);
            
            if val.is_empty() {
                let query_str = get_field(api, task_ctx, "request.query");
                let params: HashMap<String, String> = url::form_urlencoded::parse(query_str.as_bytes())
                    .into_owned()
                    .collect();
                val = params.get(key).cloned().unwrap_or_default();
            }

            if !re.is_match(&val) {
                query_match = false;
                break;
            }
        }
        if !query_match { continue; }

        if let Some(re) = &route.matcher_status_code_regex {
            let status_val = get_field(api, task_ctx, "response.status");
            if !re.is_match(&status_val) { continue; }
        }

        // Match found! Updates flags and route target.
        if let Some(cap) = path_match_capture {
            let mut existing_capture = get_field(api, task_ctx, "request.capture");
            existing_capture.push_str(&cap);
            set_field(api, task_ctx, "request.capture", &existing_capture);
        }

        set_field(api, task_ctx, "route.target", &route.module_id);
        
        let msg = CString::new(format!("Router matched route -> target: {}", route.module_id)).unwrap();
        (api.log)(task_ctx, OX_LOG_DEBUG, msg.as_ptr());

        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }

    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = Box::from_raw(plugin_config_ctx as *mut ModuleContext);
    }
}
