// ox_cert_est — Enrollment over Secure Transport (RFC 7030)

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ox_cert_core::{
    builder::{issuer_params_from_cert_pem, parse_csr, sign_csr},
    model::{AuditAction, AuditEvent, CertStoreConfig, EnrollmentProtocol, IssuancePolicyConfig,
            KeyStoreConfig},
    open_keystore,
    store::{CertStore, OxPersistenceCertStore},
};
use serde::Deserialize;
use std::collections::HashMap;
use time::OffsetDateTime;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EstConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub default_profile: String,
    pub policy: IssuancePolicyConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    /// Require client certificate (mTLS) for enrollment endpoints.
    #[serde(default)]
    pub require_client_cert: bool,
    /// Allow HTTP Basic auth as fallback (RFC 7030 §3.2.3).
    #[serde(default = "default_true")]
    pub basic_auth_enabled: bool,
    /// Static credentials for Basic auth (username + plaintext password).
    #[serde(default)]
    pub credentials: Vec<BasicCredential>,
    /// OIDs to advertise via the /csrattrs endpoint.
    #[serde(default)]
    pub csr_attrs: Vec<String>,
    /// Map of EST label → profile name for per-label enrollment.
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

fn default_true() -> bool { true }

#[derive(Debug, Deserialize)]
pub struct BasicCredential {
    pub username: String,
    pub password: String,
}

// ---------------------------------------------------------------------------
// Response type
// ---------------------------------------------------------------------------

struct EstResponse {
    status: u16,
    body: String,
    content_type: &'static str,
    content_transfer_encoding: Option<&'static str>,
}

impl EstResponse {
    fn ok_pkcs7(pkcs7_b64: String) -> Self {
        Self {
            status: 200,
            body: pkcs7_b64,
            content_type: "application/pkcs7-mime; smime-type=certs-only",
            content_transfer_encoding: Some("base64"),
        }
    }

    fn err(status: u16, msg: &str) -> Self {
        Self {
            status,
            body: msg.to_string(),
            content_type: "text/plain",
            content_transfer_encoding: None,
        }
    }
}

// ---------------------------------------------------------------------------
// PKCS#7 certs-only DER builder (RFC 5652 §5.2 / RFC 2315)
// ---------------------------------------------------------------------------

fn pkcs7_certs_only(cert_ders: &[Vec<u8>]) -> Vec<u8> {
    fn enc_len(n: usize) -> Vec<u8> {
        if n < 0x80 {
            vec![n as u8]
        } else if n <= 0xFF {
            vec![0x81, n as u8]
        } else {
            vec![0x82, (n >> 8) as u8, (n & 0xFF) as u8]
        }
    }

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut v = vec![tag];
        v.extend(enc_len(content.len()));
        v.extend_from_slice(content);
        v
    }

    // OID 1.2.840.113549.1.7.2 — signedData
    let oid_signed_data = [0x06u8,0x09,0x2A,0x86,0x48,0x86,0xF7,0x0D,0x01,0x07,0x02];
    // OID 1.2.840.113549.1.7.1 — data (for encapContentInfo)
    let oid_data        = [0x06u8,0x09,0x2A,0x86,0x48,0x86,0xF7,0x0D,0x01,0x07,0x01];

    let version        = [0x02u8, 0x01, 0x01];       // CMSVersion = 1
    let digest_algos   = [0x31u8, 0x00];              // empty SET
    let signer_infos   = [0x31u8, 0x00];              // empty SET
    let encap_ci       = tlv(0x30, &oid_data);        // EncapsulatedContentInfo

    let certs_body: Vec<u8> = cert_ders.iter().flat_map(|c| c.as_slice()).copied().collect();
    let certificates = tlv(0xA0, &certs_body);        // [0] IMPLICIT CertificateSet

    let sd_body: Vec<u8> = [
        version.as_ref(), digest_algos.as_ref(), encap_ci.as_slice(),
        certificates.as_slice(), signer_infos.as_ref(),
    ].concat();
    let signed_data = tlv(0x30, &sd_body);

    let content = tlv(0xA0, &signed_data);            // [0] EXPLICIT content
    let ci_body: Vec<u8> = [oid_signed_data.as_ref(), content.as_slice()].concat();
    tlv(0x30, &ci_body)                               // ContentInfo SEQUENCE
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn der_to_csr_pem(der: &[u8]) -> String {
    let b64 = B64.encode(der);
    let lines: String = b64.as_bytes().chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    format!("-----BEGIN CERTIFICATE REQUEST-----\n{}\n-----END CERTIFICATE REQUEST-----\n", lines)
}

fn cert_pem_to_der(pem: &str) -> Option<Vec<u8>> {
    ::pem::parse(pem.as_bytes()).ok().map(|p| p.into_contents())
}

/// Decode base64 from a request body, stripping whitespace first.
fn decode_b64_body(body: &str) -> Result<Vec<u8>, String> {
    let stripped: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    B64.decode(stripped).map_err(|e| format!("base64 decode failed: {}", e))
}

/// Validate HTTP Basic auth against configured credentials.
fn check_basic_auth(config: &EstConfig, auth_header: &str) -> bool {
    if config.credentials.is_empty() {
        return true; // no credentials configured → open access
    }
    let encoded = auth_header.strip_prefix("Basic ").unwrap_or("").trim();
    let decoded = match B64.decode(encoded) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let creds = match std::str::from_utf8(&decoded) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let (user, pass) = match creds.split_once(':') {
        Some(p) => p,
        None => return false,
    };
    config.credentials.iter().any(|c| c.username == user && c.password == pass)
}

fn resolve_profile<'a>(config: &'a EstConfig, label: Option<&str>) -> &'a str {
    label
        .and_then(|l| config.labels.get(l))
        .map(|s| s.as_str())
        .unwrap_or(&config.default_profile)
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

fn handle_cacerts(config: &EstConfig) -> EstResponse {
    let mut cert_ders: Vec<Vec<u8>> = Vec::new();

    for path in [&config.ca_root_cert_path, &config.ca_intermediate_cert_path] {
        let pem_str = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => return EstResponse::err(503, &format!("CA cert unavailable: {}", e)),
        };
        let der = match cert_pem_to_der(&pem_str) {
            Some(d) => d,
            None => return EstResponse::err(500, "CA cert PEM decode failed"),
        };
        cert_ders.push(der);
    }

    let pkcs7 = pkcs7_certs_only(&cert_ders);
    EstResponse::ok_pkcs7(B64.encode(&pkcs7))
}

fn handle_csrattrs(config: &EstConfig) -> EstResponse {
    if config.csr_attrs.is_empty() {
        // Return an empty CsrAttrs SEQUENCE (DER: 30 00), base64-encoded.
        return EstResponse {
            status: 200,
            body: B64.encode([0x30u8, 0x00]),
            content_type: "application/csrattrs",
            content_transfer_encoding: Some("base64"),
        };
    }

    // Encode each OID as DER OID TLV and wrap in a SEQUENCE.
    // OIDs in config are dotted-decimal strings; encode them.
    let mut oid_ders: Vec<u8> = Vec::new();
    for oid_str in &config.csr_attrs {
        if let Some(oid_der) = encode_oid(oid_str) {
            oid_ders.extend(oid_der);
        }
    }

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut v = vec![tag];
        if content.len() < 0x80 { v.push(content.len() as u8); }
        else { v.extend([0x81u8, content.len() as u8]); }
        v.extend_from_slice(content);
        v
    }

    let seq = tlv(0x30, &oid_ders);
    EstResponse {
        status: 200,
        body: B64.encode(&seq),
        content_type: "application/csrattrs",
        content_transfer_encoding: Some("base64"),
    }
}

/// Encode a dotted-decimal OID string to DER OID bytes (tag 0x06 + length + value).
fn encode_oid(oid: &str) -> Option<Vec<u8>> {
    let parts: Vec<u64> = oid.split('.').map(|s| s.parse().ok()).collect::<Option<_>>()?;
    if parts.len() < 2 { return None; }
    let mut value: Vec<u8> = Vec::new();
    value.push((parts[0] * 40 + parts[1]) as u8);
    for &n in &parts[2..] {
        if n == 0 {
            value.push(0);
        } else {
            let mut buf = [0u8; 10];
            let mut pos = 9;
            let mut remaining = n;
            buf[pos] = (remaining & 0x7F) as u8;
            remaining >>= 7;
            while remaining > 0 {
                pos -= 1;
                buf[pos] = ((remaining & 0x7F) | 0x80) as u8;
                remaining >>= 7;
            }
            value.extend_from_slice(&buf[pos..]);
        }
    }
    let mut out = vec![0x06u8];
    if value.len() < 0x80 { out.push(value.len() as u8); }
    else { out.extend([0x81u8, value.len() as u8]); }
    out.extend(value);
    Some(out)
}

fn handle_enroll(
    config: &EstConfig,
    body: &str,
    auth_header: &str,
    label: Option<&str>,
) -> EstResponse {
    // Authentication
    if config.basic_auth_enabled && !check_basic_auth(config, auth_header) {
        return EstResponse {
            status: 401,
            body: "Unauthorized".to_string(),
            content_type: "text/plain",
            content_transfer_encoding: None,
        };
    }

    // Decode base64 body → DER → PEM
    let der = match decode_b64_body(body) {
        Ok(d) => d,
        Err(e) => return EstResponse::err(400, &e),
    };
    let csr_pem = der_to_csr_pem(&der);

    // Validate CSR is parseable
    if parse_csr(&csr_pem).is_err() {
        return EstResponse::err(400, "invalid CSR");
    }

    // Open stores
    let ks = match open_keystore(&config.keystore) {
        Ok(k) => k,
        Err(e) => return EstResponse::err(503, &format!("CA not ready: {}", e)),
    };
    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => return EstResponse::err(500, &format!("store error: {}", e)),
    };

    // Load CA cert and key
    let ca_cert_pem = match std::fs::read_to_string(&config.ca_intermediate_cert_path) {
        Ok(p) => p,
        Err(e) => return EstResponse::err(503, &format!("CA cert unavailable: {}", e)),
    };
    let ca_params = match issuer_params_from_cert_pem(&ca_cert_pem) {
        Ok(p) => p,
        Err(e) => return EstResponse::err(503, &e.to_string()),
    };
    let ca_key_pem = match ks.load_key_pem(&config.tenant_id, &config.ca_intermediate_key_id) {
        Ok(p) => p,
        Err(e) => return EstResponse::err(503, &format!("CA key unavailable: {}", e)),
    };
    let ca_key = match rcgen::KeyPair::from_pem(&ca_key_pem) {
        Ok(k) => k,
        Err(e) => return EstResponse::err(503, &e.to_string()),
    };

    // Sign
    let profile = resolve_profile(config, label);
    let mut cert = match sign_csr(&csr_pem, &config.tenant_id, profile, 365 * 86400, None, &ca_params, &ca_key) {
        Ok(c) => c,
        Err(e) => return EstResponse::err(500, &e.to_string()),
    };
    cert.enrollment_protocol = Some(EnrollmentProtocol::Est);

    // Store
    let _ = store.store_cert(&config.tenant_id, &cert);
    let _ = store.store_audit_event(&config.tenant_id, &AuditEvent {
        id: 0,
        tenant_id: config.tenant_id.clone(),
        timestamp: OffsetDateTime::now_utc(),
        action: AuditAction::Issue,
        serial: Some(cert.serial.clone()),
        actor: String::new(),
        details: serde_json::json!({ "protocol": "est", "profile": profile }),
    });

    // Build PKCS#7 response
    let cert_der = match cert_pem_to_der(&cert.pem) {
        Some(d) => d,
        None => return EstResponse::err(500, "cert encoding failed"),
    };
    EstResponse::ok_pkcs7(B64.encode(pkcs7_certs_only(&[cert_der])))
}

// ---------------------------------------------------------------------------
// Route dispatcher
// ---------------------------------------------------------------------------

pub fn dispatch(config: &EstConfig, method: &str, path: &str, body: &str, auth: &str) -> EstResponse {
    let prefix = "/.well-known/est";
    let suffix = match path.strip_prefix(prefix) {
        Some(s) => s,
        None => return EstResponse::err(404, "not found"),
    };

    // Trim leading slash for matching
    let suffix = suffix.trim_start_matches('/');

    match (method, suffix) {
        ("GET",  "cacerts")  => handle_cacerts(config),
        ("GET",  "csrattrs") => handle_csrattrs(config),

        ("POST", "simpleenroll")   => handle_enroll(config, body, auth, None),
        ("POST", "simplereenroll") => handle_enroll(config, body, auth, None),

        ("POST", "serverkeygen") => EstResponse::err(501, "serverkeygen not implemented"),

        // Per-label variants: /.well-known/est/{label}/{op}
        ("POST", s) => {
            let parts: Vec<&str> = s.splitn(2, '/').collect();
            if parts.len() == 2 {
                let label = parts[0];
                match parts[1] {
                    "simpleenroll"   => handle_enroll(config, body, auth, Some(label)),
                    "simplereenroll" => handle_enroll(config, body, auth, Some(label)),
                    "serverkeygen"   => EstResponse::err(501, "serverkeygen not implemented"),
                    _                => EstResponse::err(404, "not found"),
                }
            } else {
                EstResponse::err(404, "not found")
            }
        }

        _ => EstResponse::err(404, "not found"),
    }
}

// ---------------------------------------------------------------------------
// Plugin ABI
// ---------------------------------------------------------------------------

pub mod plugin {
    use super::*;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::panic;
    use std::path::Path;
    use ox_workflow_abi::{
        CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
        OX_WORKFLOW_ABI_VERSION,
    };

    struct PluginState {
        api: CoreHostApi,
        config: EstConfig,
    }
    unsafe impl Send for PluginState {}
    unsafe impl Sync for PluginState {}

    fn log(api: &CoreHostApi, ctx: *mut c_void, level: u8, msg: &str) {
        if let Ok(c) = CString::new(msg) { (api.log)(ctx, level, c.as_ptr()); }
    }

    fn get(api: &CoreHostApi, ctx: *mut c_void, key: &str) -> String {
        let Ok(k) = CString::new(key) else { return String::new() };
        let ptr = (api.get_field)(ctx, k.as_ptr());
        if ptr.is_null() { return String::new(); }
        unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
    }

    fn set(api: &CoreHostApi, ctx: *mut c_void, key: &str, val: &str) {
        if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(val)) {
            (api.set_field)(ctx, k.as_ptr(), v.as_ptr());
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
        } else {
            String::new()
        };
        let params: serde_json::Value =
            serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);
        let config_path = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_est: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: EstConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_est: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_est: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_est: initialized for tenant '{}'", config.tenant_id));
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
            let body   = get(&state.api, task_ctx, "request.body");
            let auth   = get(&state.api, task_ctx, "request.header.Authorization");

            let resp = dispatch(&state.config, &method, &path, &body, &auth);

            set(&state.api, task_ctx, "response.status", &resp.status.to_string());
            set(&state.api, task_ctx, "response.body",   &resp.body);
            set(&state.api, task_ctx, "response.header.Content-Type", resp.content_type);
            if let Some(cte) = resp.content_transfer_encoding {
                set(&state.api, task_ctx, "response.header.Content-Transfer-Encoding", cte);
            }
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_est: panic in process");
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
