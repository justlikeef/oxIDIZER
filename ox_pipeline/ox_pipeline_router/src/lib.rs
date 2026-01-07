use std::ffi::{CStr, CString};
use std::sync::Arc;
use libc::{c_char, c_void};
use serde::{Deserialize, Serialize};
use regex::Regex;
use bumpalo::Bump;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use ox_webservice_api::{
    ModuleInterface, PipelineState, HandlerResult,
    LogCallback, AllocFn, AllocStrFn,
    ModuleStatus, FlowControl, ReturnParameters, LogLevel, CoreHostApi,
    UriMatcher
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
    api: &'static CoreHostApi,
}

struct CompiledRoute {
    matcher_path_regex: Option<Regex>,
    matcher_header_regexes: HashMap<String, Regex>,
    matcher_query_regexes: HashMap<String, Regex>,
    module_id: String,
    matcher_protocol: Option<String>,
    matcher_hostname_regex: Option<Regex>,
    matcher_status_code_regex: Option<Regex>,
    dispatch_count: AtomicU64, // Metric
}

impl RouterModule {
    pub fn new(config: RouterConfig, api: &'static CoreHostApi) -> Result<Self, String> {
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
                dispatch_count: AtomicU64::new(0),
            });
        }

        Ok(RouterModule {
            config,
            compiled_routes,
            api,
        })
    }
}

pub struct ModuleContext {
    module: Arc<RouterModule>,
    module_id: String,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn initialize_module(
    module_params_json_ptr: *const c_char,
    module_id: *const c_char,
    api_ptr: *const CoreHostApi,
) -> *mut ModuleInterface {
    if api_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let api = unsafe { &*api_ptr };

    let module_id_str = if !module_id.is_null() {
        unsafe { CStr::from_ptr(module_id).to_string_lossy().to_string() }
    } else {
        "unknown_router".to_string()
    };

    let _ = ox_webservice_api::init_logging(api.log_callback, &module_id_str);

    let params_str = if !module_params_json_ptr.is_null() {
        unsafe { CStr::from_ptr(module_params_json_ptr).to_string_lossy().to_string() }
    } else {
        "{}".to_string()
    };
    
    // Attempt to parse config. 
    // Note: If config is empty or invalid, we default to empty routes, effectively passing through (Continue).
    let config: RouterConfig = serde_json::from_str(&params_str).unwrap_or(RouterConfig { routes: vec![] });
    
    let module = match RouterModule::new(config, api) {
        Ok(m) => {
             Arc::new(m)
        },
        Err(e) => {
             let msg = CString::new(format!("Failed to init router: {}", e)).unwrap();
             unsafe { (api.log_callback)(LogLevel::Error, module_id, msg.as_ptr()); }
             return std::ptr::null_mut();
        }
    };

    let module_id_str = if !module_id.is_null() {
        unsafe { CStr::from_ptr(module_id).to_string_lossy().to_string() }
    } else {
        "ox_pipeline_router".to_string()
    };

    let ctx = Box::new(ModuleContext {
        module,
        module_id: module_id_str,
    });

    let interface = Box::new(ModuleInterface {
        instance_ptr: Box::into_raw(ctx) as *mut c_void,
        handler_fn: process_request,
        log_callback: api.log_callback,
        get_config: get_config,
    });

    Box::into_raw(interface)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn process_request(
    instance_ptr: *mut c_void,
    pipeline_state_ptr: *mut PipelineState,
    _log_callback: LogCallback,
    _alloc_fn: AllocFn,
    _arena: *const c_void,
) -> HandlerResult {
    if instance_ptr.is_null() {
         return HandlerResult {
            status: ModuleStatus::Unmodified,
            flow_control: FlowControl::Continue,
            return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
        };
    }
    let context = unsafe { &*(instance_ptr as *mut ModuleContext) };
    let pipeline_state = unsafe { &mut *pipeline_state_ptr };
    let arena_ptr = &pipeline_state.arena as *const Bump as *const c_void;

    let ctx = unsafe { ox_pipeline_plugin::PipelineContext::new(
        context.module.api, 
        pipeline_state_ptr as *mut c_void, 
        arena_ptr
    ) };

    // Common State Lookups
    let _path = match ctx.get("request.path") {
        Some(v) => v.as_str().unwrap_or("/").to_string(),
        None => "/".to_string()
    };
    let protocol = match ctx.get("request.protocol") {
        Some(v) => v.as_str().unwrap_or("HTTP/1.1").to_string(),
        None => "HTTP/1.1".to_string()
    };
    
    // Capture the original path once to avoid matching against captures from previous routes in the loop
    let original_path = match ctx.get("request.path") {
        Some(v) => v.as_str().unwrap_or("/").to_string(),
        None => "/".to_string(),
    };

    // Iterate Routes
    let mut current_status = ModuleStatus::Unmodified;

    for route in &context.module.compiled_routes {
        // 1. Protocol Match
        if let Some(proto) = &route.matcher_protocol {
            if !proto.eq_ignore_ascii_case(&protocol) { continue; }
        }

        // 2. Hostname Match
        if let Some(re) = &route.matcher_hostname_regex {
            let host_val = match ctx.get("request.header.Host") {
                Some(v) => v.as_str().unwrap_or("").to_string(),
                None => "".to_string(),
            };
            if !re.is_match(&host_val) { continue; }
        }

        // 3. Path Match
        let mut path_match_capture = None;
        if let Some(re) = &route.matcher_path_regex {
            if let Some(caps) = re.captures(&original_path) {
                // If there's a capture group (index 1), store it for later update
                if let Some(m) = caps.get(1) {
                    path_match_capture = Some(m.as_str().to_string());
                } else {
                    path_match_capture = Some("".to_string());
                }
            } else {
                continue;
            }
        }

        // 4. Headers Match
        let mut headers_match = true;
        for (key, re) in &route.matcher_header_regexes {
            let val = if key.eq_ignore_ascii_case("Method") {
                 match ctx.get("request.method") {
                     Some(v) => v.as_str().unwrap_or("").to_string(),
                     None => "".to_string(),
                 }
            } else {
                let lookup_key = format!("request.header.{}", key);
                match ctx.get(&lookup_key) {
                    Some(v) => v.as_str().unwrap_or("").to_string(),
                    None => {
                        // Try lowercase if not found and not "Method"
                        let lower_key = format!("request.header.{}", key.to_lowercase());
                        match ctx.get(&lower_key) {
                            Some(v) => v.as_str().unwrap_or("").to_string(),
                            None => "".to_string(),
                        }
                    }
                }
            };

             if !re.is_match(&val) {
                 headers_match = false;
                 break;
             }
        }
        if !headers_match { continue; }

        // 5. Query Match
        let mut query_match = true;
        for (key, re) in &route.matcher_query_regexes {
            let lookup_key = format!("request.query.{}", key);
            let val = match ctx.get(&lookup_key) {
                Some(v) => v.as_str().unwrap_or("").to_string(),
                None => {
                    // Fallback to manual parse if ctx.get fails (though ox_webservice implements it)
                    let query_str = match ctx.get("request.query") {
                        Some(v) => v.as_str().unwrap_or("").to_string(),
                        None => "".to_string(),
                    };
                    let params: HashMap<String, String> = url::form_urlencoded::parse(query_str.as_bytes())
                        .into_owned()
                        .collect();
                    params.get(key).cloned().unwrap_or_default()
                }
            };

            if !re.is_match(&val) {
                query_match = false;
                break;
            }
        }
        if !query_match { continue; }

        // 6. Status Code Match
        if let Some(re) = &route.matcher_status_code_regex {
            let status_val = match ctx.get("response.status") {
                Some(v) => v.to_string(),
                None => "0".to_string(),
            };
            if !re.is_match(&status_val) { continue; }
        }

        // ALL FILTERS PASSED! UPDATE CAPTURE AND EXECUTE
        if let Some(cap) = path_match_capture {
            let mut existing_capture = match ctx.get("request.capture") {
                Some(v) => v.as_str().unwrap_or("").to_string(),
                None => "".to_string(),
            };
            existing_capture.push_str(&cap);
            let _ = ctx.set("request.capture", serde_json::Value::String(existing_capture));
        }

        route.dispatch_count.fetch_add(1, Ordering::Relaxed); // Metric Increment
        let res = ctx.execute_module(&route.module_id);
        
        // Track status propagation
        if res.status == ModuleStatus::Modified {
            current_status = ModuleStatus::Modified;
        }

        if res.flow_control == FlowControl::Halt || res.flow_control == FlowControl::StreamFile {
            return res;
        }
        // If Continue, we keep searching for more matching routes (sequential pipeline)
    }

    HandlerResult {
        status: current_status,
        flow_control: FlowControl::Continue,
        return_parameters: ReturnParameters { return_data: std::ptr::null_mut() }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_config(
    instance_ptr: *mut c_void,
    arena: *const c_void,
    alloc_fn: AllocStrFn,
) -> *mut c_char {
    if instance_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let context = unsafe { &*(instance_ptr as *mut ModuleContext) };
    
    // Construct Dynamic Config with Metrics
    let mut dynamic_routes = Vec::new();
    for (i, route) in context.module.compiled_routes.iter().enumerate() {
        let original_route = context.module.config.routes.get(i);
        
        let mut route_val = serde_json::to_value(original_route).unwrap_or(serde_json::Value::Null);
        if let Some(obj) = route_val.as_object_mut() {
            obj.insert("dispatch_count".to_string(), serde_json::Value::Number(serde_json::Number::from(route.dispatch_count.load(Ordering::Relaxed))));
        }
        dynamic_routes.push(route_val);
    }
    
    let mut config_map = serde_json::Map::new();
    config_map.insert("routes".to_string(), serde_json::Value::Array(dynamic_routes));
    
    let json = serde_json::to_string(&config_map).unwrap_or("{}".to_string());
    unsafe { alloc_fn(arena, CString::new(json).unwrap().as_ptr()) }
}
// mod test_regression; // Disabled until updated
