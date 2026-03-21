use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_ERROR, OX_LOG_INFO, OX_LOG_ERROR,
};
use serde::Deserialize;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use regex::Regex;

const MODULE_NAME: &str = "ox_webservice_redirect";

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct RedirectRule {
    pub match_pattern: String,
    pub replace_string: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct RedirectConfig {
    pub rules: Vec<RedirectRule>,
}

pub struct ModuleContext {
    config: RedirectConfig,
    regexes: Vec<Regex>,
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

    let params: Value = serde_json::from_str(&params_str).unwrap_or(Value::Null);
    let config_file = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "Missing config_file"); return std::ptr::null_mut(); }
    };

    let config: RedirectConfig = match ox_fileproc::process_file(&PathBuf::from(&config_file), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to parse config: {}", e)); return std::ptr::null_mut(); }
        },
        Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Failed to read config: {}", e)); return std::ptr::null_mut(); }
    };

    let mut regexes = Vec::new();
    for rule in &config.rules {
        match Regex::new(&rule.match_pattern) {
            Ok(r) => regexes.push(r),
            Err(e) => { log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("Invalid regex '{}': {}", rule.match_pattern, e)); return std::ptr::null_mut(); }
        }
    }

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, &format!("{} initialized with {} rules", MODULE_NAME, regexes.len()));
    let ctx = Box::new(ModuleContext { config, regexes, api });
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

    let path_str = get_field(api, task_ctx, "request.path");

    for (i, regex) in context.regexes.iter().enumerate() {
        if regex.is_match(&path_str) {
            let rule = &context.config.rules[i];
            let redirect_to = regex.replace(&path_str, rule.replace_string.as_str()).to_string();
            log(api, task_ctx, OX_LOG_INFO, &format!("Redirecting '{}' -> '{}'", path_str, redirect_to));

            let html_content = format!(
                "<html><head><meta http-equiv=\"refresh\" content=\"0;url={}\"></head><body>Redirecting...</body></html>",
                redirect_to
            );
            set_field(api, task_ctx, "response.header.Content-Type", "text/html");
            set_field(api, task_ctx, "response.body", &html_content);
            set_field(api, task_ctx, "response.status", "301");
            return FlowControl { code: FLOW_CONTROL_ERROR, payload: std::ptr::null() }; // Halt the pipeline
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
mod tests {
    use super::*;

    #[test]
    fn test_regex_replacement() {
        let rules = vec![
            RedirectRule { match_pattern: "^/old/(.*)$".to_string(), replace_string: "/new/$1".to_string() },
        ];
        let regexes: Vec<Regex> = rules.iter().map(|r| Regex::new(&r.match_pattern).unwrap()).collect();
        let new_path = regexes[0].replace("/old/page", rules[0].replace_string.as_str()).to_string();
        assert_eq!(new_path, "/new/page");
    }
}
