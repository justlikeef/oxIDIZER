use ox_cert_core::{
    model::{
        AuditAction, AuditEvent, ApprovalRequest, ApprovalStatus,
        CertStoreConfig, Pagination,
    },
    store::{CertStore, OxPersistenceCertStore},
};
use ox_cert_issue_lib::{config::CertIssueConfig, handlers::handle_issue};
use serde::Deserialize;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct AutoApproveRule {
    pub identity_pattern: String,
    pub profiles: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct RaConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    #[serde(default = "default_queue")]
    pub resubmit_queue: String,
    #[serde(default = "default_priority")]
    pub resubmit_priority: u8,
    #[serde(default)]
    pub auto_approve_rules: Vec<AutoApproveRule>,
    pub notification_webhook: Option<String>,
    /// Path to the ox_cert_issue config file; required to sign certs on approval.
    pub issue_config_path: Option<String>,
}

fn default_queue() -> String { "tasks.pending".to_string() }
fn default_priority() -> u8 { 100 }

pub struct RaOutcome {
    pub http_status: u16,
    pub body_json: String,
}

fn pagination_from_query(query: &str) -> Pagination {
    let mut offset = 0u64;
    let mut limit = 50u64;
    for part in query.split('&') {
        let mut kv = part.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("offset"), Some(v)) => { offset = v.parse().unwrap_or(0); }
            (Some("limit"), Some(v)) => { limit = v.parse().unwrap_or(50).min(200); }
            _ => {}
        }
    }
    Pagination { offset, limit }
}

pub fn handle(
    config: &RaConfig,
    issue_config: Option<&CertIssueConfig>,
    method: &str,
    path: &str,
    query: &str,
    body: &str,
) -> RaOutcome {
    let tenant = &config.tenant_id;
    let request_id = Uuid::new_v4().to_string();

    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return RaOutcome {
                http_status: $status,
                body_json: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        };
    }

    let store = match OxPersistenceCertStore::open(config.store.db_path()) {
        Ok(s) => s,
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    // Route parsing
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    // /api/v1/ra/...
    if segs.len() < 4 || segs[0] != "api" || segs[1] != "v1" || segs[2] != "ra" {
        return RaOutcome { http_status: 404, body_json: "{}".to_string() };
    }

    match (method, segs.get(3), segs.get(4), segs.get(5)) {
        // GET /api/v1/ra/pending
        ("GET", Some(&"pending"), None, None) => {
            let page = pagination_from_query(query);
            let result = match store.list_ra_pending(tenant, &page) {
                Ok(r) => r,
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            };
            RaOutcome {
                http_status: 200,
                body_json: serde_json::json!({
                    "data": result.items,
                    "meta": { "tenant_id": tenant, "total": result.total, "offset": result.offset, "limit": result.limit }
                }).to_string(),
            }
        }

        // GET /api/v1/ra/pending/{id}
        ("GET", Some(&"pending"), Some(id), None) => {
            match store.get_ra_request(tenant, id) {
                Ok(Some(req)) => RaOutcome {
                    http_status: 200,
                    body_json: serde_json::json!({
                        "data": req,
                        "meta": { "tenant_id": tenant, "request_id": request_id }
                    }).to_string(),
                },
                Ok(None) => err!(404, "NOT_FOUND", format!("RA request '{}' not found", id)),
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            }
        }

        // POST /api/v1/ra/pending/{id}/approve
        ("POST", Some(&"pending"), Some(id), Some(&"approve")) => {
            handle_approve(&store, config, issue_config, id, body, &request_id)
        }

        // POST /api/v1/ra/pending/{id}/deny
        ("POST", Some(&"pending"), Some(id), Some(&"deny")) => {
            handle_deny(&store, config, id, body, &request_id)
        }

        // POST /api/v1/ra/sign  — admin direct sign, bypasses RA queue
        ("POST", Some(&"sign"), None, None) => {
            match issue_config {
                Some(cfg) => handle_direct_sign(cfg, body, tenant, &request_id),
                None => err!(503, "NOT_CONFIGURED", "issue_config_path not set in RA config"),
            }
        }

        // GET /api/v1/ra/history
        ("GET", Some(&"history"), None, None) => {
            let page = pagination_from_query(query);
            let result = match store.list_ra_pending(tenant, &page) {
                Ok(r) => r,
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            };
            let history: Vec<_> = result.items.into_iter()
                .filter(|r| r.status != ApprovalStatus::Pending)
                .collect();
            RaOutcome {
                http_status: 200,
                body_json: serde_json::json!({
                    "data": history,
                    "meta": { "tenant_id": tenant }
                }).to_string(),
            }
        }

        // GET /api/v1/ra/requests/{id}/certificate
        ("GET", Some(&"requests"), Some(id), Some(&"certificate")) => {
            let req = match store.get_ra_request(tenant, id) {
                Ok(Some(r)) => r,
                Ok(None) => err!(404, "NOT_FOUND", format!("RA request '{}' not found", id)),
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            };
            if req.status != ApprovalStatus::Approved {
                return RaOutcome {
                    http_status: 202,
                    body_json: serde_json::json!({
                        "data": { "status": "pending" },
                        "meta": { "tenant_id": tenant }
                    }).to_string(),
                };
            }
            let serial = match &req.certificate_serial {
                Some(s) => s.clone(),
                None => return RaOutcome {
                    http_status: 202,
                    body_json: serde_json::json!({
                        "data": { "status": "processing" },
                        "meta": { "tenant_id": tenant }
                    }).to_string(),
                },
            };
            match store.get_cert_by_serial(tenant, &serial) {
                Ok(Some(cert)) => RaOutcome {
                    http_status: 200,
                    body_json: serde_json::json!({
                        "data": cert,
                        "meta": { "tenant_id": tenant, "request_id": request_id }
                    }).to_string(),
                },
                Ok(None) => err!(404, "NOT_FOUND", "certificate not yet available"),
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            }
        }

        _ => RaOutcome { http_status: 404, body_json: "{}".to_string() },
    }
}

fn handle_approve(
    store: &OxPersistenceCertStore,
    config: &RaConfig,
    issue_config: Option<&CertIssueConfig>,
    id: &str,
    body: &str,
    request_id: &str,
) -> RaOutcome {
    let tenant = &config.tenant_id;

    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return RaOutcome {
                http_status: $status,
                body_json: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        };
    }

    let req: ApprovalRequest = match store.get_ra_request(tenant, id) {
        Ok(Some(r)) => r,
        Ok(None) => err!(404, "NOT_FOUND", format!("RA request '{}' not found", id)),
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    if req.status != ApprovalStatus::Pending {
        err!(409, "INVALID_REQUEST", "request already processed");
    }

    let reviewer_notes: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let notes = reviewer_notes.get("reviewer_notes").and_then(|v| v.as_str()).unwrap_or("");

    // Issue the certificate before marking approved, so we can record the serial
    let serial: Option<String> = if let Some(cfg) = issue_config {
        match handle_issue(cfg, &req.csr_pem, "application/pkcs10", true, None) {
            Ok(outcome) if outcome.http_status == 201 => outcome.serial,
            Ok(outcome) => {
                return RaOutcome {
                    http_status: outcome.http_status,
                    body_json: outcome.body_json,
                };
            }
            Err(e) => err!(500, "ISSUANCE_FAILED", e.message),
        }
    } else {
        None
    };

    if let Err(e) = store.update_ra_request(tenant, id, ApprovalStatus::Approved, "", notes) {
        err!(500, "INTERNAL_ERROR", e.to_string());
    }

    // Patch the serial into the RA record
    if let Some(ref s) = serial {
        if let Ok(Some(mut patched)) = store.get_ra_request(tenant, id) {
            patched.certificate_serial = Some(s.clone());
            let _ = store.store_ra_request(tenant, &patched);
        }
    }

    let now = OffsetDateTime::now_utc();
    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0,
        tenant_id: tenant.clone(),
        timestamp: now,
        action: AuditAction::RaApprove,
        serial: serial.clone(),
        actor: String::new(),
        details: serde_json::json!({ "ra_request_id": id }),
    });

    RaOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "data": { "id": id, "status": "approved", "serial": serial },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn handle_deny(
    store: &OxPersistenceCertStore,
    config: &RaConfig,
    id: &str,
    body: &str,
    request_id: &str,
) -> RaOutcome {
    let tenant = &config.tenant_id;

    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return RaOutcome {
                http_status: $status,
                body_json: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        };
    }

    let req: ApprovalRequest = match store.get_ra_request(tenant, id) {
        Ok(Some(r)) => r,
        Ok(None) => err!(404, "NOT_FOUND", format!("RA request '{}' not found", id)),
        Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
    };

    if req.status != ApprovalStatus::Pending {
        err!(409, "INVALID_REQUEST", "request already processed");
    }

    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let reason = match v.get("reason").and_then(|r| r.as_str()) {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => err!(400, "INVALID_REQUEST", "reason is required for denial"),
    };

    if let Err(e) = store.update_ra_request(tenant, id, ApprovalStatus::Denied, "", &reason) {
        err!(500, "INTERNAL_ERROR", e.to_string());
    }

    let now = OffsetDateTime::now_utc();
    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0,
        tenant_id: tenant.clone(),
        timestamp: now,
        action: AuditAction::RaDeny,
        serial: None,
        actor: String::new(),
        details: serde_json::json!({ "ra_request_id": id, "reason": reason }),
    });

    RaOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "data": { "id": id, "status": "denied" },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn handle_direct_sign(
    issue_config: &CertIssueConfig,
    body: &str,
    tenant: &str,
    request_id: &str,
) -> RaOutcome {
    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return RaOutcome {
                http_status: $status,
                body_json: serde_json::json!({
                    "error": { "code": $code, "message": $msg },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        };
    }

    // Accept either raw CSR PEM in body or JSON {"csr":"..."}
    let csr_pem = if body.trim_start().starts_with("-----BEGIN") {
        body.to_string()
    } else {
        match serde_json::from_str::<serde_json::Value>(body) {
            Ok(v) => match v.get("csr").and_then(|c| c.as_str()) {
                Some(s) => s.to_string(),
                None => err!(400, "INVALID_REQUEST", "body must contain 'csr' field or raw PEM"),
            },
            Err(_) => err!(400, "INVALID_REQUEST", "invalid request body"),
        }
    };

    match handle_issue(issue_config, &csr_pem, "application/pkcs10", true, None) {
        Ok(outcome) => RaOutcome {
            http_status: outcome.http_status,
            body_json: outcome.body_json,
        },
        Err(e) => RaOutcome {
            http_status: e.http_status,
            body_json: e.to_body(tenant),
        },
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

    struct PluginState {
        api: CoreHostApi,
        config: RaConfig,
        issue_config: Option<CertIssueConfig>,
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
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_ra: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: RaConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_ra: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_ra: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };

        let issue_config: Option<CertIssueConfig> = if let Some(path) = &config.issue_config_path {
            match ox_fileproc::process_file(Path::new(path), 5) {
                Ok(v) => match serde_json::from_value(v) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                            &format!("ox_cert_ra: issue config error: {}", e));
                        None
                    }
                },
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_ra: failed to load issue config: {}", e));
                    None
                }
            }
        } else {
            None
        };

        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_ra: initialized for tenant '{}'", config.tenant_id));
        Box::into_raw(Box::new(PluginState { api, config, issue_config })) as *mut c_void
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
            let query = get(&state.api, task_ctx, "request.query");
            let body = get(&state.api, task_ctx, "request.body");

            if !path.starts_with("/api/v1/ra/") { return cont; }

            let outcome = handle(&state.config, state.issue_config.as_ref(), &method, &path, &query, &body);
            set(&state.api, task_ctx, "response.status", &outcome.http_status.to_string());
            set(&state.api, task_ctx, "response.body", &outcome.body_json);
            set(&state.api, task_ctx, "response.header.Content-Type", "application/json");
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_ra: panic");
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
