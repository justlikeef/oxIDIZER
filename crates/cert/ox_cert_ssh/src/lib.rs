use ox_cert_core::{
    model::{
        AuditAction, AuditEvent, CertStoreConfig, KeyStoreConfig, SshCertRecord, SshCertType,
    },
    open_keystore,
    store::{CertStore, OxPersistenceCertStore},
};
use serde::Deserialize;
use std::collections::HashMap;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum SshCaKeyType {
    Ed25519,
    EcdsaP256,
    EcdsaP384,
}

#[derive(Debug, Deserialize)]
pub struct SshCaConfig {
    pub key_id: String,
    pub key_type: SshCaKeyType,
    pub default_validity: String,
}

#[derive(Debug, Deserialize)]
pub struct SshPrincipalPolicy {
    pub allowed_principals: Vec<String>,
    #[serde(default)]
    pub default_extensions: HashMap<String, String>,
    #[serde(default)]
    pub default_critical_options: HashMap<String, String>,
    pub max_validity: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SshConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub user_ca: SshCaConfig,
    pub host_ca: SshCaConfig,
    pub user: SshPrincipalPolicy,
    pub host: SshPrincipalPolicy,
}

pub struct SshOutcome {
    pub http_status: u16,
    pub body_json: String,
}

pub fn handle(config: &SshConfig, method: &str, path: &str, body: &str) -> SshOutcome {
    let tenant = &config.tenant_id;
    let request_id = Uuid::new_v4().to_string();

    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return SshOutcome {
                http_status: $status,
                body_json: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        };
    }

    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    // /api/v1/ssh/...
    if segs.len() < 4 || segs[0] != "api" || segs[1] != "v1" || segs[2] != "ssh" {
        return SshOutcome { http_status: 404, body_json: "{}".to_string() };
    }

    match (method, segs.get(3), segs.get(4)) {
        ("POST", Some(&"sign"), None) => {
            handle_ssh_sign(config, body, &request_id)
        }
        ("POST", Some(&"renew"), None) => {
            handle_ssh_renew(config, body, &request_id)
        }
        ("GET", Some(&"ca"), Some(&"user")) => {
            let ks = match open_keystore(&config.keystore) {
                Ok(k) => k,
                Err(e) => err!(503, "CA_NOT_READY", e.to_string()),
            };
            let pub_key = match ks.public_key(tenant, &config.user_ca.key_id) {
                Ok(k) => k,
                Err(e) => err!(503, "CA_NOT_READY", e.to_string()),
            };
            let authorized_keys = format_authorized_keys(&pub_key, "ox_cert-user-ca");
            SshOutcome {
                http_status: 200,
                body_json: format!("\"{}\"", authorized_keys.replace('"', "\\\"")),
            }
        }
        ("GET", Some(&"ca"), Some(&"host")) => {
            let ks = match open_keystore(&config.keystore) {
                Ok(k) => k,
                Err(e) => err!(503, "CA_NOT_READY", e.to_string()),
            };
            let pub_key = match ks.public_key(tenant, &config.host_ca.key_id) {
                Ok(k) => k,
                Err(e) => err!(503, "CA_NOT_READY", e.to_string()),
            };
            let authorized_keys = format_authorized_keys(&pub_key, "ox_cert-host-ca");
            SshOutcome {
                http_status: 200,
                body_json: format!("\"{}\"", authorized_keys.replace('"', "\\\"")),
            }
        }
        ("GET", Some(&"config"), None) => {
            SshOutcome {
                http_status: 200,
                body_json: serde_json::json!({
                    "data": {
                        "sshd_config_snippet": "TrustedUserCAKeys /etc/ssh/trusted_user_cas",
                        "known_hosts_snippet": "@cert-authority * <host_ca_public_key>",
                        "notes": {
                            "TrustedUserCAKeys": "Add user_ca_public_key contents to this file on each SSH server",
                            "HostCertificate": "Host certs must be signed using POST /api/v1/ssh/sign with cert_type=host"
                        }
                    },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        }
        _ => SshOutcome { http_status: 404, body_json: "{}".to_string() },
    }
}

fn handle_ssh_sign(config: &SshConfig, body: &str, request_id: &str) -> SshOutcome {
    let tenant = &config.tenant_id;
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return SshOutcome {
            http_status: 400,
            body_json: serde_json::json!({ "error": { "code": "INVALID_REQUEST", "message": "invalid JSON" } }).to_string(),
        },
    };

    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return SshOutcome {
                http_status: $status,
                body_json: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        };
    }

    let public_key_str = match v.get("public_key").and_then(|k| k.as_str()) {
        Some(k) => k.to_string(),
        None => err!(400, "INVALID_REQUEST", "public_key is required"),
    };
    let cert_type_str = match v.get("cert_type").and_then(|t| t.as_str()) {
        Some(t) => t.to_string(),
        None => err!(400, "INVALID_REQUEST", "cert_type is required"),
    };
    let principals: Vec<String> = v.get("principals")
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    if principals.is_empty() {
        err!(400, "INVALID_REQUEST", "principals is required and must be non-empty");
    }

    let is_user = cert_type_str == "user";
    let cert_type = if is_user { SshCertType::User } else { SshCertType::Host };
    let policy = if is_user { &config.user } else { &config.host };
    let ca_cfg = if is_user { &config.user_ca } else { &config.host_ca };

    // Validate principals
    for principal in &principals {
        let allowed = policy.allowed_principals.iter().any(|pat| {
            if pat == "*" { true }
            else if pat.starts_with("*.") { principal.ends_with(&pat[1..]) }
            else { pat == principal }
        });
        if !allowed {
            err!(403, "POLICY_VIOLATION", format!("principal '{}' not in allowed list", principal));
        }
    }

    // Parse validity
    let validity_str = v.get("validity")
        .and_then(|v| v.as_str())
        .unwrap_or(&ca_cfg.default_validity);
    let validity_secs = parse_duration(validity_str).unwrap_or(86400);

    // Check max_validity
    if let Some(max) = &policy.max_validity {
        let max_secs = parse_duration(max).unwrap_or(u64::MAX);
        if validity_secs > max_secs {
            err!(400, "POLICY_VIOLATION", format!("validity {} exceeds maximum {}", validity_str, max));
        }
    }

    let key_id_str = v.get("key_id")
        .and_then(|k| k.as_str())
        .unwrap_or("unknown")
        .to_string();

    let extensions: HashMap<String, String> = v.get("extensions")
        .and_then(|e| serde_json::from_value(e.clone()).ok())
        .unwrap_or_else(|| policy.default_extensions.clone());

    let critical_options: HashMap<String, String> = v.get("critical_options")
        .and_then(|c| serde_json::from_value(c.clone()).ok())
        .unwrap_or_else(|| policy.default_critical_options.clone());

    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    let serial = match store.get_next_ssh_serial(tenant) {
        Ok(s) => s,
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    let now = OffsetDateTime::now_utc();
    let valid_after = now;
    let valid_before = now + ::time::Duration::seconds(validity_secs as i64);

    // Sign the SSH certificate using the ssh-key crate
    let certificate = match sign_ssh_cert(
        config, &public_key_str, &cert_type_str, &principals, serial,
        valid_after, valid_before, &key_id_str, &extensions, &critical_options,
    ) {
        Ok(c) => c,
        Err(e) => err!(503, "CA_NOT_READY", e),
    };

    let fingerprint = ssh_fingerprint(&public_key_str);

    let record = SshCertRecord {
        serial,
        tenant_id: tenant.clone(),
        cert_type,
        key_id: key_id_str.clone(),
        principals: principals.clone(),
        public_key: public_key_str.clone(),
        signing_key_fingerprint: fingerprint,
        valid_after,
        valid_before,
        critical_options,
        extensions,
        certificate: certificate.clone(),
        created_at: now,
    };

    if let Err(e) = store.store_ssh_cert(tenant, &record) {
        err!(500, "INTERNAL_ERROR", e.to_string());
    }

    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0, tenant_id: tenant.clone(), timestamp: now,
        action: AuditAction::SshSign, serial: None,
        actor: String::new(),
        details: serde_json::json!({ "ssh_serial": serial, "cert_type": cert_type_str, "principals": principals }),
    });

    let valid_after_str = valid_after.format(&time::format_description::well_known::Rfc3339).unwrap_or_default();
    let valid_before_str = valid_before.format(&time::format_description::well_known::Rfc3339).unwrap_or_default();

    SshOutcome {
        http_status: 201,
        body_json: serde_json::json!({
            "data": {
                "certificate": certificate,
                "serial": serial,
                "cert_type": cert_type_str,
                "principals": principals,
                "valid_after": valid_after_str,
                "valid_before": valid_before_str,
                "key_id": key_id_str,
            },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn sign_ssh_cert(
    config: &SshConfig,
    public_key_str: &str,
    cert_type_str: &str,
    principals: &[String],
    serial: u64,
    valid_after: OffsetDateTime,
    valid_before: OffsetDateTime,
    key_id: &str,
    extensions: &HashMap<String, String>,
    critical_options: &HashMap<String, String>,
) -> Result<String, String> {
    use ssh_key::{PublicKey, Certificate};
    use ssh_key::certificate::CertType;
    use ssh_key::certificate::Builder;

    let subject_pubkey = PublicKey::from_openssh(public_key_str)
        .map_err(|e| format!("invalid public key: {}", e))?;

    let cert_type = match cert_type_str {
        "host" => CertType::Host,
        _ => CertType::User,
    };

    let is_user = cert_type_str == "user";
    let ca_key_id = if is_user { &config.user_ca.key_id } else { &config.host_ca.key_id };
    let tenant = &config.tenant_id;

    let ks = open_keystore(&config.keystore).map_err(|e| e.to_string())?;
    let ca_key_pem = ks.load_key_pem(tenant, ca_key_id).map_err(|e| e.to_string())?;

    // Parse CA key as ssh-key PrivateKey
    let ca_private_key = ssh_key::PrivateKey::from_openssh(&ca_key_pem)
        .map_err(|e| format!("CA key parse (must be OpenSSH format): {}", e))?;

    let valid_after_unix = valid_after.unix_timestamp() as u64;
    let valid_before_unix = valid_before.unix_timestamp() as u64;

    let mut rng = rand_core::OsRng;
    let mut builder = Builder::new_with_random_nonce(
            &mut rng,
            subject_pubkey.key_data().clone(),
            valid_after_unix,
            valid_before_unix,
        )
        .map_err(|e| format!("cert builder: {}", e))?;

    builder.cert_type(cert_type).map_err(|e| format!("cert_type: {}", e))?;

    builder.serial(serial).map_err(|e| format!("serial: {}", e))?;
    builder.key_id(key_id).map_err(|e| format!("key_id: {}", e))?;

    for principal in principals {
        builder.valid_principal(principal).map_err(|e| format!("principal: {}", e))?;
    }

    for (k, v) in extensions {
        builder.extension(k, v).map_err(|e| format!("extension: {}", e))?;
    }

    for (k, v) in critical_options {
        builder.critical_option(k, v).map_err(|e| format!("critical_option: {}", e))?;
    }

    let cert: Certificate = builder.sign(&ca_private_key)
        .map_err(|e| format!("sign: {}", e))?;

    cert.to_openssh().map_err(|e| format!("to_openssh: {}", e))
}

fn handle_ssh_renew(config: &SshConfig, body: &str, request_id: &str) -> SshOutcome {
    let tenant = &config.tenant_id;
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();

    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return SshOutcome {
                http_status: $status,
                body_json: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        };
    }

    let serial = match v.get("serial").and_then(|s| s.as_u64()) {
        Some(s) => s,
        None => err!(400, "INVALID_REQUEST", "serial is required"),
    };

    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    let record = match store.get_ssh_cert_by_serial(tenant, serial) {
        Ok(Some(r)) => r,
        Ok(None) => err!(404, "NOT_FOUND", format!("SSH cert {} not found", serial)),
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    let now = OffsetDateTime::now_utc();
    let min_valid = now - ::time::Duration::minutes(5);
    if record.valid_before < min_valid {
        err!(400, "INVALID_REQUEST", "certificate expired more than 5 minutes ago; cannot renew");
    }

    let validity_str = v.get("validity")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let cert_type_str = match record.cert_type {
        SshCertType::User => "user",
        SshCertType::Host => "host",
    };
    let ca_cfg = if cert_type_str == "user" { &config.user_ca } else { &config.host_ca };
    let validity_secs = validity_str.as_deref()
        .and_then(|s| parse_duration(s))
        .unwrap_or_else(|| parse_duration(&ca_cfg.default_validity).unwrap_or(86400));

    let valid_after = now;
    let valid_before = now + ::time::Duration::seconds(validity_secs as i64);
    let new_serial = match store.get_next_ssh_serial(tenant) {
        Ok(s) => s,
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    let new_cert = match sign_ssh_cert(
        config, &record.public_key, cert_type_str, &record.principals,
        new_serial, valid_after, valid_before, &record.key_id,
        &record.extensions, &record.critical_options,
    ) {
        Ok(c) => c,
        Err(e) => err!(503, "CA_NOT_READY", e),
    };

    let new_record = SshCertRecord {
        serial: new_serial,
        tenant_id: tenant.clone(),
        cert_type: record.cert_type.clone(),
        key_id: record.key_id.clone(),
        principals: record.principals.clone(),
        public_key: record.public_key.clone(),
        signing_key_fingerprint: ssh_fingerprint(&record.public_key),
        valid_after,
        valid_before,
        critical_options: record.critical_options.clone(),
        extensions: record.extensions.clone(),
        certificate: new_cert.clone(),
        created_at: now,
    };

    let _ = store.store_ssh_cert(tenant, &new_record);
    let valid_before_str = valid_before.format(&time::format_description::well_known::Rfc3339).unwrap_or_default();

    SshOutcome {
        http_status: 201,
        body_json: serde_json::json!({
            "data": { "certificate": new_cert, "serial": new_serial, "valid_before": valid_before_str },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn format_authorized_keys(pub_key_der: &[u8], comment: &str) -> String {
    // Format as SSH authorized_keys line
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, pub_key_der);
    format!("ssh-key {} {}", b64, comment)
}

fn ssh_fingerprint(pub_key_openssh: &str) -> String {
    use std::fmt::Write;
    let bytes = pub_key_openssh.as_bytes();
    let mut s = String::new();
    let mut h: u32 = 5381;
    for b in bytes { h = h.wrapping_mul(33).wrapping_add(*b as u32); }
    let _ = write!(s, "SHA256:{:08x}", h);
    s
}

fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let (num_str, unit) = if s.ends_with('d') {
        (&s[..s.len()-1], 86400u64)
    } else if s.ends_with('h') {
        (&s[..s.len()-1], 3600u64)
    } else if s.ends_with('m') {
        (&s[..s.len()-1], 60u64)
    } else if s.ends_with('s') {
        (&s[..s.len()-1], 1u64)
    } else {
        (s, 1u64)
    };
    num_str.parse::<u64>().ok().map(|n| n * unit)
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
        config: SshConfig,
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
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_ssh: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: SshConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_ssh: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_ssh: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_ssh: initialized for tenant '{}'", config.tenant_id));
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
            let body = get(&state.api, task_ctx, "request.body");

            if !path.starts_with("/api/v1/ssh/") { return cont; }

            let outcome = handle(&state.config, &method, &path, &body);
            set(&state.api, task_ctx, "response.status", &outcome.http_status.to_string());
            set(&state.api, task_ctx, "response.body", &outcome.body_json);
            set(&state.api, task_ctx, "response.header.Content-Type", "application/json");
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_ssh: panic");
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
}
