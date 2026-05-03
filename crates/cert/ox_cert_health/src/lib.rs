use ox_cert_core::{
    model::{CertStoreConfig, KeyStoreConfig},
    open_keystore,
    store::{CertStore, OxPersistenceCertStore},
};
use serde::Deserialize;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct HealthConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    /// Warn (degraded) if CA cert expires within this many days (default: 365).
    #[serde(default = "default_warn_days")]
    pub ca_cert_warn_days: u32,
}

fn default_warn_days() -> u32 { 365 }

// ---------------------------------------------------------------------------
// Check results
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct CheckResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

impl CheckResult {
    fn ok(latency_ms: u64) -> Self {
        Self { ok: true, message: None, latency_ms: Some(latency_ms) }
    }
    fn ok_msg(msg: impl Into<String>) -> Self {
        Self { ok: true, message: Some(msg.into()), latency_ms: None }
    }
    fn fail(msg: impl Into<String>) -> Self {
        Self { ok: false, message: Some(msg.into()), latency_ms: None }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct Checks {
    pub ca_key: CheckResult,
    pub database: CheckResult,
    pub ca_cert_valid: CheckResult,
    pub root_cert_valid: CheckResult,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OverallStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

pub struct HealthOutcome {
    pub status: OverallStatus,
    pub checks: Checks,
}

// ---------------------------------------------------------------------------
// Core checks
// ---------------------------------------------------------------------------

pub fn run_checks(config: &HealthConfig) -> HealthOutcome {
    let tenant = &config.tenant_id;

    // CA key check
    let ca_key_check = {
        let t0 = std::time::Instant::now();
        match open_keystore(&config.keystore).and_then(|ks| {
            ks.key_exists(tenant, &config.ca_intermediate_key_id)
        }) {
            Ok(true) => CheckResult::ok(t0.elapsed().as_millis() as u64),
            Ok(false) => CheckResult::fail("intermediate CA key not found in keystore"),
            Err(e) => CheckResult::fail(e.to_string()),
        }
    };

    // Database check (open store)
    let db_check = {
        let t0 = std::time::Instant::now();
        match OxPersistenceCertStore::open().and_then(|s| s.migrate()) {
            Ok(()) => CheckResult::ok(t0.elapsed().as_millis() as u64),
            Err(e) => CheckResult::fail(e.to_string()),
        }
    };

    // CA cert validity
    let ca_cert_check = cert_validity_check(
        &config.ca_intermediate_cert_path,
        "intermediate CA",
        config.ca_cert_warn_days,
    );

    // Root cert validity
    let root_cert_check = cert_validity_check(
        &config.ca_root_cert_path,
        "root CA",
        config.ca_cert_warn_days,
    );

    let overall = if !ca_key_check.ok || !db_check.ok {
        OverallStatus::Unhealthy
    } else if !ca_cert_check.ok || !root_cert_check.ok {
        OverallStatus::Unhealthy
    } else if matches!(ca_cert_check.message, Some(ref m) if m.starts_with("expiring")) ||
              matches!(root_cert_check.message, Some(ref m) if m.starts_with("expiring")) {
        OverallStatus::Degraded
    } else {
        OverallStatus::Healthy
    };

    HealthOutcome {
        status: overall,
        checks: Checks {
            ca_key: ca_key_check,
            database: db_check,
            ca_cert_valid: ca_cert_check,
            root_cert_valid: root_cert_check,
        },
    }
}

fn cert_validity_check(cert_path: &str, label: &str, warn_days: u32) -> CheckResult {
    use x509_parser::prelude::*;

    let pem_str = match std::fs::read_to_string(cert_path) {
        Ok(s) => s,
        Err(e) => return CheckResult::fail(format!("{} cert not found: {}", label, e)),
    };
    let der = match ::pem::parse(pem_str.as_bytes()) {
        Ok(p) => p.into_contents(),
        Err(e) => return CheckResult::fail(format!("{} cert PEM invalid: {}", label, e)),
    };
    let cert = match X509Certificate::from_der(&der) {
        Ok((_, c)) => c,
        Err(e) => return CheckResult::fail(format!("{} cert DER invalid: {}", label, e)),
    };
    let not_after = cert.validity().not_after.timestamp();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let days = (not_after - now) / 86400;

    if days < 0 {
        CheckResult::fail(format!("{} cert expired {} days ago", label, -days))
    } else if days < warn_days as i64 {
        CheckResult::ok_msg(format!("expiring in {} days (warn threshold: {})", days, warn_days))
    } else {
        CheckResult::ok_msg(format!("{} cert valid for {} more days", label, days))
    }
}

// ---------------------------------------------------------------------------
// Plugin ABI
// ---------------------------------------------------------------------------

pub mod plugin {
    use super::*;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::path::Path;
    use std::panic;
    use ox_workflow_abi::{
        CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
        OX_WORKFLOW_ABI_VERSION,
    };

    #[allow(dead_code)]
    struct PluginState {
        api: CoreHostApi,
        config: HealthConfig,
    }
    unsafe impl Send for PluginState {}
    unsafe impl Sync for PluginState {}

    fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
        if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
    }

    fn get(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
        let Ok(c_key) = CString::new(key) else { return String::new() };
        let ptr = (api.get_field)(task_ctx, c_key.as_ptr());
        if ptr.is_null() { return String::new(); }
        unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
    }

    fn set(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, val: &str) {
        if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(val)) {
            (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_init(
        config_ptr: *const c_char,
        api_ptr: *const CoreHostApi,
        abi_version: u32,
    ) -> *mut c_void {
        if abi_version != OX_WORKFLOW_ABI_VERSION || api_ptr.is_null() {
            return std::ptr::null_mut();
        }
        let api = unsafe { *api_ptr };
        let params_str = if !config_ptr.is_null() {
            unsafe { CStr::from_ptr(config_ptr).to_string_lossy().to_string() }
        } else { String::new() };
        let params: serde_json::Value = serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);
        let config_path = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_health: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: HealthConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_health: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_health: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_health: initialized for tenant '{}'", config.tenant_id));
        Box::into_raw(Box::new(PluginState { api, config })) as *mut c_void
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_process(
        plugin_ctx: *mut c_void,
        task_ctx: *mut c_void,
    ) -> FlowControl {
        let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
        if plugin_ctx.is_null() { return cont; }
        let state = unsafe { &*(plugin_ctx as *mut PluginState) };

        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let method = get(&state.api, task_ctx, "request.method").to_uppercase();
            let path   = get(&state.api, task_ctx, "request.path");

            if method != "GET" { return cont; }

            match path.as_str() {
                "/healthz" => {
                    set(&state.api, task_ctx, "response.status", "200");
                    set(&state.api, task_ctx, "response.body", "ok");
                    set(&state.api, task_ctx, "response.header.Content-Type", "text/plain");
                }
                "/readyz" => {
                    let outcome = run_checks(&state.config);
                    let (status, body) = match outcome.status {
                        OverallStatus::Unhealthy => {
                            let reason = first_failure(&outcome.checks);
                            (503u16, serde_json::json!({"status":"not_ready","reason":reason}).to_string())
                        }
                        _ => (200u16, r#"{"status":"ready"}"#.to_string()),
                    };
                    set(&state.api, task_ctx, "response.status", &status.to_string());
                    set(&state.api, task_ctx, "response.body", &body);
                    set(&state.api, task_ctx, "response.header.Content-Type", "application/json");
                }
                "/api/v1/health" => {
                    let request_id = Uuid::new_v4().to_string();
                    let outcome = run_checks(&state.config);
                    let status_str = match outcome.status {
                        OverallStatus::Healthy  => "healthy",
                        OverallStatus::Degraded => "degraded",
                        OverallStatus::Unhealthy => "unhealthy",
                    };
                    let body = serde_json::json!({
                        "data": {
                            "status": status_str,
                            "tenant_id": state.config.tenant_id,
                            "checks": outcome.checks,
                        },
                        "meta": { "tenant_id": state.config.tenant_id, "request_id": request_id }
                    }).to_string();
                    set(&state.api, task_ctx, "response.status", "200");
                    set(&state.api, task_ctx, "response.body", &body);
                    set(&state.api, task_ctx, "response.header.Content-Type", "application/json");
                }
                _ => return cont,
            }
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_health: panic");
                cont
            }
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_error(_ctx: *mut c_void, _task: *mut c_void) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
        if !plugin_ctx.is_null() {
            unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)); }
        }
    }

    fn first_failure(checks: &Checks) -> String {
        if !checks.ca_key.ok {
            return checks.ca_key.message.clone().unwrap_or_else(|| "ca_key failed".to_string());
        }
        if !checks.database.ok {
            return checks.database.message.clone().unwrap_or_else(|| "database failed".to_string());
        }
        if !checks.ca_cert_valid.ok {
            return checks.ca_cert_valid.message.clone().unwrap_or_else(|| "ca_cert expired".to_string());
        }
        if !checks.root_cert_valid.ok {
            return checks.root_cert_valid.message.clone().unwrap_or_else(|| "root_cert expired".to_string());
        }
        "unknown failure".to_string()
    }
}
