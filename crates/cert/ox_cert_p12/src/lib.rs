use ox_cert_core::{
    decrypt_private_key,
    model::{AuditAction, AuditEvent, CertStatus, CertStoreConfig, KeyStoreConfig},
    store::{CertStore, OxPersistenceCertStore},
};
use pem as pem_crate;
use serde::Deserialize;
use time::OffsetDateTime;

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum Pkcs12Encryption {
    Aes256,
    TripleDes,
}

impl Default for Pkcs12Encryption {
    fn default() -> Self { Self::Aes256 }
}

#[derive(Debug, Deserialize)]
pub struct P12Config {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    #[allow(dead_code)]
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    #[serde(default)]
    pub encryption: Pkcs12Encryption,
}

pub struct P12Outcome {
    pub http_status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub content_disposition: Option<String>,
}

pub fn handle_p12(config: &P12Config, serial: &str, password: &str) -> P12Outcome {
    let tenant = &config.tenant_id;

    macro_rules! json_err {
        ($status:expr, $code:expr, $msg:expr) => {
            return P12Outcome {
                http_status: $status,
                content_type: "application/json".to_string(),
                body: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant }
                }).to_string().into_bytes(),
                content_disposition: None,
            }
        };
    }

    if password.is_empty() {
        json_err!(400, "INVALID_REQUEST", "password is required");
    }

    let store = match OxPersistenceCertStore::open(config.store.db_path()) {
        Ok(s) => s,
        Err(e) => json_err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    let record = match store.get_cert_by_serial(tenant, serial) {
        Ok(Some(r)) => r,
        Ok(None) => json_err!(404, "NOT_FOUND", format!("certificate '{}' not found", serial)),
        Err(e) => json_err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    if record.status == CertStatus::Revoked {
        json_err!(409, "ALREADY_REVOKED", "certificate is revoked");
    }

    let enc_b64 = match &record.private_key_encrypted {
        Some(e) => e.clone(),
        None => json_err!(409, "INVALID_REQUEST",
            "private key not held by CA — only available for server-generated certificates"),
    };

    // Get encryption passphrase from env
    let passphrase = match std::env::var("OX_CA_KEY_PASS") {
        Ok(p) => p,
        Err(_) => json_err!(500, "INTERNAL_ERROR", "OX_CA_KEY_PASS env var not set"),
    };

    // Decrypt the private key (decrypt_private_key handles base64 decode internally)
    let key_der = match decrypt_private_key(&enc_b64, &passphrase, tenant) {
        Ok(k) => k,
        Err(e) => json_err!(500, "INTERNAL_ERROR", format!("key decrypt: {}", e)),
    };

    // Load certificate chain
    let int_pem = match std::fs::read_to_string(&config.ca_intermediate_cert_path) {
        Ok(s) => s,
        Err(e) => json_err!(500, "INTERNAL_ERROR", format!("intermediate cert: {}", e)),
    };
    let root_pem = match std::fs::read_to_string(&config.ca_root_cert_path) {
        Ok(s) => s,
        Err(e) => json_err!(500, "INTERNAL_ERROR", format!("root cert: {}", e)),
    };

    // Parse chain certs as DER
    let leaf_der = match pem_crate::parse(record.pem.as_bytes()) {
        Ok(p) => p.into_contents(),
        Err(e) => json_err!(500, "INTERNAL_ERROR", format!("leaf cert PEM: {}", e)),
    };
    let int_der = match pem_crate::parse(int_pem.as_bytes()) {
        Ok(p) => p.into_contents(),
        Err(e) => json_err!(500, "INTERNAL_ERROR", format!("intermediate cert PEM: {}", e)),
    };
    let root_der = match pem_crate::parse(root_pem.as_bytes()) {
        Ok(p) => p.into_contents(),
        Err(e) => json_err!(500, "INTERNAL_ERROR", format!("root cert PEM: {}", e)),
    };

    // Build PKCS#12 bundle
    let p12_bytes = match build_p12(&key_der, &leaf_der, &[int_der, root_der], password) {
        Ok(b) => b,
        Err(e) => json_err!(500, "INTERNAL_ERROR", format!("PKCS#12 build: {}", e)),
    };

    let now = OffsetDateTime::now_utc();
    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0,
        tenant_id: tenant.clone(),
        timestamp: now,
        action: AuditAction::P12Export,
        serial: Some(serial.to_string()),
        actor: String::new(),
        details: serde_json::json!({}),
    });

    P12Outcome {
        http_status: 200,
        content_type: "application/x-pkcs12".to_string(),
        body: p12_bytes,
        content_disposition: Some(format!("attachment; filename=\"{}.p12\"", serial)),
    }
}

fn build_p12(
    key_der: &[u8],
    leaf_cert_der: &[u8],
    chain_certs_der: &[Vec<u8>],
    password: &str,
) -> Result<Vec<u8>, String> {
    let chain_refs: Vec<&[u8]> = chain_certs_der.iter().map(|v| v.as_slice()).collect();
    let pfx = p12::PFX::new_with_cas(leaf_cert_der, key_der, &chain_refs, password, "ox_cert")
        .ok_or_else(|| "pfx creation failed".to_string())?;

    Ok(pfx.to_der())
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

    struct PluginState {
        api: CoreHostApi,
        config: P12Config,
    }
    unsafe impl Send for PluginState {}
    unsafe impl Sync for PluginState {}

    fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
        if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
    }

    fn get(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
        let Ok(k) = CString::new(key) else { return String::new() };
        let ptr = (api.get_field)(task_ctx, k.as_ptr());
        if ptr.is_null() { return String::new(); }
        unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
    }

    fn set(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, val: &str) {
        if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(val)) {
            (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
        }
    }

    fn set_bytes(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, data: &[u8]) {
        if let Ok(k) = CString::new(key) {
            (api.set_field_bytes)(task_ctx, k.as_ptr(), data.as_ptr(), data.len());
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
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_p12: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: P12Config = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_p12: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_p12: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_p12: initialized for tenant '{}'", config.tenant_id));
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
            let path = get(&state.api, task_ctx, "request.path");

            // Match *.p12 suffix
            let serial = extract_serial_p12(&path);
            if serial.is_none() || (method != "GET" && method != "POST") { return cont; }
            let serial = serial.unwrap();

            let password = if method == "GET" {
                let query = get(&state.api, task_ctx, "request.query");
                parse_query_value(&query, "password")
            } else {
                let body = get(&state.api, task_ctx, "request.body");
                let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                v.get("password").and_then(|p| p.as_str()).unwrap_or("").to_string()
            };

            let outcome = handle_p12(&state.config, &serial, &password);

            set(&state.api, task_ctx, "response.status", &outcome.http_status.to_string());
            set(&state.api, task_ctx, "response.header.Content-Type", &outcome.content_type);
            if let Some(cd) = &outcome.content_disposition {
                set(&state.api, task_ctx, "response.header.Content-Disposition", cd);
            }
            if outcome.content_type == "application/x-pkcs12" {
                set_bytes(&state.api, task_ctx, "response.body", &outcome.body);
            } else {
                if let Ok(s) = std::str::from_utf8(&outcome.body) {
                    set(&state.api, task_ctx, "response.body", s);
                }
            }
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_p12: panic");
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

    fn extract_serial_p12(path: &str) -> Option<String> {
        // /api/v1/certificates/{serial}.p12
        let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        if segs.len() == 4 && segs[0] == "api" && segs[1] == "v1" && segs[2] == "certificates" {
            if let Some(name) = segs[3].strip_suffix(".p12") {
                return Some(name.to_string());
            }
        }
        None
    }

    fn parse_query_value(query: &str, key: &str) -> String {
        for part in query.split('&') {
            let mut kv = part.splitn(2, '=');
            if kv.next() == Some(key) {
                return kv.next().unwrap_or("").to_string();
            }
        }
        String::new()
    }
}
