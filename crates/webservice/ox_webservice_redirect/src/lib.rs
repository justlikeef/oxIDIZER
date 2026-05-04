use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, FLOW_CONTROL_END, OX_LOG_INFO, OX_LOG_ERROR,
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
    /// Replacement string. Supports regex capture groups ($1, $2, …) and {host}
    /// placeholder (substituted with the request Host header before regex replace).
    /// Required unless `skip` is true.
    pub replace_string: Option<String>,
    /// When true, matching this rule suppresses any further rule evaluation and
    /// allows the request to pass through without redirecting. Use to exclude paths
    /// (e.g. ACME http-01 challenge paths) from a catch-all rule below.
    #[serde(default)]
    pub skip: bool,
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct RedirectConfig {
    /// Port suffix for HTTPS redirects, e.g. ":8443". Empty string (default) means
    /// standard port 443 is used (no explicit port in the redirect URL).
    #[serde(default)]
    pub https_port: String,
    /// HTTP status code for redirects. Use 301 for permanent (production),
    /// 302 for temporary (avoids browser caching during development). Default: 302.
    #[serde(default = "default_status_code")]
    pub status_code: u16,
    pub rules: Vec<RedirectRule>,
}

fn default_status_code() -> u16 { 302 }

pub struct CompiledRule {
    regex: Regex,
    replace_string: Option<String>,
    skip: bool,
}

pub struct ModuleContext {
    rules: Vec<CompiledRule>,
    https_port: String,
    status_code: u16,
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

    let mut compiled = Vec::new();
    for rule in &config.rules {
        if !rule.skip && rule.replace_string.is_none() {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("Rule '{}' must have replace_string or skip: true", rule.match_pattern));
            return std::ptr::null_mut();
        }
        match Regex::new(&rule.match_pattern) {
            Ok(r) => compiled.push(CompiledRule {
                regex: r,
                replace_string: rule.replace_string.clone(),
                skip: rule.skip,
            }),
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("Invalid regex '{}': {}", rule.match_pattern, e));
                return std::ptr::null_mut();
            }
        }
    }

    log(&api, std::ptr::null_mut(), OX_LOG_INFO,
        &format!("{} initialized with {} rules", MODULE_NAME, compiled.len()));
    let ctx = Box::new(ModuleContext { rules: compiled, https_port: config.https_port, status_code: config.status_code, api });
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

    let path = get_field(api, task_ctx, "request.path");
    let host = get_field(api, task_ctx, "request.header.host");
    let host_noport = host.split(':').next().unwrap_or(&host).to_string();

    for rule in &context.rules {
        if rule.regex.is_match(&path) {
            if rule.skip {
                // Matched an exclusion rule — pass through without redirecting.
                return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
            }

            let replace_template = rule.replace_string.as_deref().unwrap_or("");
            let replaced = replace_template
                .replace("{host_noport}", &host_noport)
                .replace("{https_port}", &context.https_port)
                .replace("{host}", &host);
            let redirect_to = rule.regex.replace(&path, replaced.as_str()).to_string();

            log(api, task_ctx, OX_LOG_INFO, &format!("Redirecting '{}' -> '{}'", path, redirect_to));

            let status_str = context.status_code.to_string();
            let body = format!("{} {}", context.status_code,
                if context.status_code == 301 { "Moved Permanently" } else { "Found" });
            set_field(api, task_ctx, "response.status", &status_str);
            set_field(api, task_ctx, "response.header.Location", &redirect_to);
            set_field(api, task_ctx, "response.header.Content-Type", "text/plain");
            set_field(api, task_ctx, "response.body", &body);
            return FlowControl { code: FLOW_CONTROL_END, payload: std::ptr::null() };
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
    fn test_path_redirect() {
        let regex = Regex::new("^(.*)$").unwrap();
        let replace = "https://ca.example.com$1";
        let result = regex.replace("/api/v1/certs", replace).to_string();
        assert_eq!(result, "https://ca.example.com/api/v1/certs");
    }

    #[test]
    fn test_host_placeholder_substitution() {
        let template = "https://{host}$1";
        let host = "ca.example.com";
        let replaced = template.replace("{host}", host);
        let regex = Regex::new("^(.*)$").unwrap();
        let result = regex.replace("/api/v1/certs", replaced.as_str()).to_string();
        assert_eq!(result, "https://ca.example.com/api/v1/certs");
    }

    #[test]
    fn test_host_noport_strips_port() {
        let host = "localhost:8091";
        let host_noport = host.split(':').next().unwrap_or(host);
        let https_port = ":8443";
        let template = "https://{host_noport}{https_port}$1";
        let replaced = template
            .replace("{host_noport}", host_noport)
            .replace("{https_port}", https_port)
            .replace("{host}", host);
        let regex = Regex::new("^(.*)$").unwrap();
        let result = regex.replace("/path", replaced.as_str()).to_string();
        assert_eq!(result, "https://localhost:8443/path");
    }

    #[test]
    fn test_host_noport_standard_port() {
        let host = "example.com";
        let host_noport = host.split(':').next().unwrap_or(host);
        let https_port = "";
        let template = "https://{host_noport}{https_port}$1";
        let replaced = template
            .replace("{host_noport}", host_noport)
            .replace("{https_port}", https_port)
            .replace("{host}", host);
        let regex = Regex::new("^(.*)$").unwrap();
        let result = regex.replace("/path", replaced.as_str()).to_string();
        assert_eq!(result, "https://example.com/path");
    }

    #[test]
    fn test_skip_rule_does_not_produce_redirect_string() {
        let rule = CompiledRule {
            regex: Regex::new("^/.well-known/acme-challenge/").unwrap(),
            replace_string: None,
            skip: true,
        };
        let path = "/.well-known/acme-challenge/abc123";
        assert!(rule.regex.is_match(path));
        assert!(rule.skip);
    }
}
