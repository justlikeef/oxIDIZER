use ox_cert_core::{
    model::{AuditAction, AuditEvent, CertStatus, CertStoreConfig, RevocationReason},
    store::{CertStore, OxPersistenceCertStore},

};
use serde::Deserialize;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CertRevokeConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
}

#[derive(Deserialize)]
struct RevokeRequest {
    reason: Option<String>,
}

fn reason_from_str(s: &str) -> Result<RevocationReason, String> {
    match s {
        "unspecified" => Ok(RevocationReason::Unspecified),
        "key_compromise" => Ok(RevocationReason::KeyCompromise),
        "ca_compromise" => Ok(RevocationReason::CaCompromise),
        "affiliation_changed" => Ok(RevocationReason::AffiliationChanged),
        "superseded" => Ok(RevocationReason::Superseded),
        "cessation_of_operation" => Ok(RevocationReason::CessationOfOperation),
        "certificate_hold" => Ok(RevocationReason::CertificateHold),
        "privilege_withdrawn" => Ok(RevocationReason::PrivilegeWithdrawn),
        other => Err(format!("unknown revocation reason: '{}'", other)),
    }
}

pub struct RevokeOutcome {
    pub http_status: u16,
    pub body_json: String,
}

pub fn handle_revoke(
    config: &CertRevokeConfig,
    serial: &str,
    request_body: &str,
) -> RevokeOutcome {
    let tenant = &config.tenant_id;
    let request_id = Uuid::new_v4().to_string();

    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return RevokeOutcome {
                http_status: $status,
                body_json: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                })
                .to_string(),
            }
        };
    }

    let req: RevokeRequest = match serde_json::from_str(request_body) {
        Ok(r) => r,
        Err(_) => RevokeRequest { reason: None },
    };

    let reason = match req.reason.as_deref().unwrap_or("unspecified") {
        s => match reason_from_str(s) {
            Ok(r) => r,
            Err(msg) => err!(400, "INVALID_REQUEST", msg),
        },
    };

    let store = match OxPersistenceCertStore::open(config.store.db_path()) {
        Ok(s) => s,
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    // Look up the cert
    let cert = match store.get_cert_by_serial(tenant, serial) {
        Ok(Some(c)) => c,
        Ok(None) => err!(404, "NOT_FOUND", format!("certificate '{}' not found", serial)),
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    if cert.status == CertStatus::Revoked {
        err!(409, "ALREADY_REVOKED", format!("certificate '{}' is already revoked", serial));
    }

    let now = OffsetDateTime::now_utc();
    if let Err(e) = store.mark_revoked(tenant, serial, reason, now) {
        err!(500, "INTERNAL_ERROR", e.to_string());
    }

    let _ = store.store_audit_event(
        tenant,
        &AuditEvent {
            id: 0,
            tenant_id: tenant.clone(),
            timestamp: now,
            action: AuditAction::Revoke,
            serial: Some(serial.to_string()),
            actor: String::new(),
            details: serde_json::json!({ "reason": req.reason.unwrap_or_default() }),
        },
    );

    let revoked_at = now
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();

    RevokeOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "data": {
                "serial": serial,
                "revoked_at": revoked_at,
                "reason": reason as u8,
            },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        })
        .to_string(),
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
        config: CertRevokeConfig,
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

    fn set_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
        set(api, task_ctx, "response.status", &status.to_string());
        set(api, task_ctx, "response.body", body);
        set(api, task_ctx, "response.header.Content-Type", "application/json");
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
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_revoke: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: CertRevokeConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_revoke: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_revoke: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_revoke: initialized for tenant '{}'", config.tenant_id));
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

            // Match POST /api/v1/certificates/{serial}/revoke
            let serial = extract_serial_from_revoke_path(&path);
            if method != "POST" || serial.is_none() { return cont; }
            let serial = serial.unwrap();

            let body = get(&state.api, task_ctx, "request.body");
            let outcome = handle_revoke(&state.config, &serial, &body);
            set_response(&state.api, task_ctx, outcome.http_status, &outcome.body_json);
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_revoke: panic");
                set_response(&state.api, task_ctx, 500,
                    r#"{"error":{"code":"INTERNAL_ERROR","message":"panic"}}"#);
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

    /// Extract serial from `/api/v1/certificates/{serial}/revoke`.
    fn extract_serial_from_revoke_path(path: &str) -> Option<String> {
        let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        // ["api", "v1", "certificates", "{serial}", "revoke"]
        if segs.len() == 5
            && segs[0] == "api" && segs[1] == "v1"
            && segs[2] == "certificates" && segs[4] == "revoke"
        {
            Some(segs[3].to_string())
        } else {
            None
        }
    }
}
