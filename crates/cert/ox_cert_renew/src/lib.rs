use ox_cert_core::{
    issuer_params_from_cert_pem,
    model::{
        AuditAction, AuditEvent, CertStatus, CertStoreConfig,
        KeyStoreConfig, RevocationReason, SanType,
    },
    store::{CertStore, OxPersistenceCertStore},
    open_keystore,
    sign_csr,
};
use serde::Deserialize;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Deserialize, Default)]
pub struct ExtensionsConfig {
    pub aia_ocsp_url: Option<String>,
    pub aia_ca_issuer_url: Option<String>,
    pub cdp_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CertRenewConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub auto_revoke_on_renew: bool,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    #[allow(dead_code)]
    pub ca_root_cert_path: String,
    #[allow(dead_code)]
    pub extensions: ExtensionsConfig,
}

#[derive(Deserialize, Default)]
struct RenewRequest {
    csr: Option<String>,
    validity_seconds: Option<u64>,
}

pub struct RenewOutcome {
    pub http_status: u16,
    pub body_json: String,
}

pub fn handle_renew(config: &CertRenewConfig, serial: &str, body: &str) -> RenewOutcome {
    let tenant = &config.tenant_id;
    let request_id = Uuid::new_v4().to_string();

    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return RenewOutcome {
                http_status: $status,
                body_json: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        };
    }

    let req: RenewRequest = serde_json::from_str(body).unwrap_or_default();

    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    let existing = match store.get_cert_by_serial(tenant, serial) {
        Ok(Some(c)) => c,
        Ok(None) => err!(404, "NOT_FOUND", format!("certificate '{}' not found", serial)),
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    match existing.status {
        CertStatus::Revoked => err!(409, "ALREADY_REVOKED", "cannot renew a revoked certificate"),
        CertStatus::PendingApproval => err!(409, "INVALID_REQUEST", "cannot renew a cert pending approval"),
        _ => {}
    }

    // Determine validity: from request or reuse original window
    let validity_secs = req.validity_seconds.unwrap_or_else(|| {
        let orig_secs = (existing.not_after - existing.not_before).whole_seconds().max(0) as u64;
        if orig_secs == 0 { 31_536_000 } else { orig_secs }
    });

    // Load CA cert and key
    let ca_cert_pem = match std::fs::read_to_string(&config.ca_intermediate_cert_path) {
        Ok(s) => s,
        Err(e) => err!(503, "CA_NOT_READY", format!("CA cert unreadable: {}", e)),
    };
    let issuer_params = match issuer_params_from_cert_pem(&ca_cert_pem) {
        Ok(p) => p,
        Err(e) => err!(503, "CA_NOT_READY", e.to_string()),
    };
    let ks = match open_keystore(&config.keystore) {
        Ok(k) => k,
        Err(e) => err!(503, "CA_NOT_READY", e.to_string()),
    };
    let ca_key_pem = match ks.load_key_pem(tenant, &config.ca_intermediate_key_id) {
        Ok(p) => p,
        Err(e) => err!(503, "CA_NOT_READY", e.to_string()),
    };
    let ca_keypair = match rcgen::KeyPair::from_pem(&ca_key_pem) {
        Ok(k) => k,
        Err(e) => err!(503, "CA_NOT_READY", e.to_string()),
    };

    let new_serial = Uuid::new_v4().to_string();

    // Build new cert: use provided CSR or synthesize from existing cert fields
    let new_record = if let Some(csr_pem) = &req.csr {
        // Sign the provided CSR
        match sign_csr(csr_pem, tenant, &existing.profile, validity_secs, None, &issuer_params, &ca_keypair) {
            Ok(mut r) => { r.serial = new_serial.clone(); r }
            Err(e) => err!(400, "INVALID_CSR", e.to_string()),
        }
    } else {
        // Re-sign with same SANs using existing CSR if available
        let csr_pem = match &existing.csr_pem {
            Some(c) => c.clone(),
            None => err!(400, "INVALID_REQUEST", "no CSR available for renewal; provide a csr in the request body"),
        };
        let override_sans: Vec<SanType> = existing.sans.iter()
            .filter_map(|s| san_from_str(s))
            .collect();
        match sign_csr(&csr_pem, tenant, &existing.profile, validity_secs, Some(&override_sans), &issuer_params, &ca_keypair) {
            Ok(mut r) => { r.serial = new_serial.clone(); r }
            Err(e) => err!(400, "INVALID_CSR", e.to_string()),
        }
    };

    if let Err(e) = store.store_cert(tenant, &new_record) {
        err!(500, "INTERNAL_ERROR", e.to_string());
    }

    if config.auto_revoke_on_renew {
        let _ = store.mark_revoked(tenant, serial, RevocationReason::Superseded, OffsetDateTime::now_utc());
    }

    let now = OffsetDateTime::now_utc();
    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0,
        tenant_id: tenant.clone(),
        timestamp: now,
        action: AuditAction::Renew,
        serial: Some(new_serial.clone()),
        actor: String::new(),
        details: serde_json::json!({ "old_serial": serial, "new_serial": new_serial }),
    });

    let not_after = new_record.not_after
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();

    RenewOutcome {
        http_status: 201,
        body_json: serde_json::json!({
            "data": {
                "serial": new_serial,
                "pem": new_record.pem,
                "not_after": not_after,
                "subject_cn": new_record.subject_cn,
                "old_serial": serial,
            },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn san_from_str(s: &str) -> Option<SanType> {
    use std::net::IpAddr;
    if let Ok(ip) = s.parse::<IpAddr>() { return Some(SanType::Ip(ip)); }
    if s.contains('@') { return Some(SanType::Email(s.to_string())); }
    if s.starts_with("http://") || s.starts_with("https://") {
        return Some(SanType::Uri(s.to_string()));
    }
    Some(SanType::Dns(s.to_string()))
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
        config: CertRenewConfig,
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
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_renew: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: CertRenewConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_renew: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_renew: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_renew: initialized for tenant '{}'", config.tenant_id));
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

            let serial = extract_serial_from_renew_path(&path);
            if method != "POST" || serial.is_none() { return cont; }
            let serial = serial.unwrap();

            let body = get(&state.api, task_ctx, "request.body");
            let outcome = handle_renew(&state.config, &serial, &body);

            set(&state.api, task_ctx, "response.status", &outcome.http_status.to_string());
            set(&state.api, task_ctx, "response.body", &outcome.body_json);
            set(&state.api, task_ctx, "response.header.Content-Type", "application/json");
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_renew: panic");
                set(&state.api, task_ctx, "response.status", "500");
                set(&state.api, task_ctx, "response.body",
                    r#"{"error":{"code":"INTERNAL_ERROR","message":"panic"}}"#);
                set(&state.api, task_ctx, "response.header.Content-Type", "application/json");
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

    /// Extract serial from `/api/v1/certificates/{serial}/renew`.
    fn extract_serial_from_renew_path(path: &str) -> Option<String> {
        let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        if segs.len() == 5
            && segs[0] == "api" && segs[1] == "v1"
            && segs[2] == "certificates" && segs[4] == "renew"
        {
            Some(segs[3].to_string())
        } else {
            None
        }
    }
}
