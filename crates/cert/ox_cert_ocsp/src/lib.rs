use ox_cert_core::{
    model::{CertStatus, CertStoreConfig, KeyStoreConfig},
    open_keystore,
    store::{CertStore, OxPersistenceCertStore},
};
use rcgen::{KeyPair, SigningKey};
use serde::Deserialize;
use time::OffsetDateTime;

#[derive(Debug, Deserialize)]
pub struct OcspConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub responder_key_id: String,
    pub delegated_cert_path: Option<String>,
    #[serde(default = "default_max_age")]
    pub max_age_secs: u64,
    #[serde(default = "default_next_update")]
    pub next_update_secs: u64,
}

fn default_max_age() -> u64 { 3600 }
fn default_next_update() -> u64 { 86400 }

// ---------------------------------------------------------------------------
// DER encoding helpers
// ---------------------------------------------------------------------------

fn der_len_bytes(n: usize) -> Vec<u8> {
    if n < 0x80 {
        vec![n as u8]
    } else if n < 0x100 {
        vec![0x81, n as u8]
    } else if n < 0x10000 {
        vec![0x82, (n >> 8) as u8, n as u8]
    } else {
        vec![0x83, (n >> 16) as u8, (n >> 8) as u8, n as u8]
    }
}

fn der_tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend_from_slice(&der_len_bytes(content.len()));
    out.extend_from_slice(content);
    out
}

fn der_sequence(content: &[u8]) -> Vec<u8> { der_tlv(0x30, content) }
fn der_octet_string(b: &[u8]) -> Vec<u8> { der_tlv(0x04, b) }
#[allow(dead_code)]
fn der_null() -> Vec<u8> { vec![0x05, 0x00] }
fn der_bit_string(b: &[u8]) -> Vec<u8> {
    let mut content = vec![0x00u8]; // 0 unused bits
    content.extend_from_slice(b);
    der_tlv(0x03, &content)
}
fn der_explicit(tag: u8, content: &[u8]) -> Vec<u8> { der_tlv(0xa0 | tag, content) }
fn der_generalized_time(dt: &OffsetDateTime) -> Vec<u8> {
    let s = format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}Z",
        dt.year(), dt.month() as u8, dt.day(),
        dt.hour(), dt.minute(), dt.second()
    );
    der_tlv(0x18, s.as_bytes())
}

/// AlgorithmIdentifier DER for the signing key's algorithm.
fn sig_alg_der(kp: &KeyPair) -> Vec<u8> {
    let alg = kp.algorithm();
    if alg == &rcgen::PKCS_ECDSA_P256_SHA256 {
        // ecdsaWithSHA256  1.2.840.10045.4.3.2
        vec![0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]
    } else if alg == &rcgen::PKCS_ECDSA_P384_SHA384 {
        // ecdsaWithSHA384  1.2.840.10045.4.3.3
        vec![0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03]
    } else if alg == &rcgen::PKCS_ED25519 {
        // id-EdDSA  1.3.101.112
        vec![0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70]
    } else {
        // sha256WithRSAEncryption  1.2.840.113549.1.1.11
        vec![0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b, 0x05, 0x00]
    }
}

// ---------------------------------------------------------------------------
// DER parsing helpers (OCSP request)
// ---------------------------------------------------------------------------

fn der_length(data: &[u8]) -> (usize, usize) {
    if data.is_empty() { return (0, 1); }
    if data[0] & 0x80 == 0 {
        (data[0] as usize, 1)
    } else {
        let n = (data[0] & 0x7f) as usize;
        if n == 0 || n > 4 || data.len() < n + 1 { return (0, 1); }
        let mut len = 0usize;
        for i in 0..n { len = (len << 8) | data[1 + i] as usize; }
        (len, 1 + n)
    }
}

/// Parse the direct TLV children of a DER value, returning (tag, content_bytes, full_tlv_bytes).
fn parse_children(data: &[u8]) -> Vec<(u8, &[u8], &[u8])> {
    let mut items = Vec::new();
    let mut i = 0;
    while i + 2 <= data.len() {
        let tag = data[i];
        let (len, hdr) = der_length(&data[i + 1..]);
        let end = i + 1 + hdr + len;
        if end > data.len() { break; }
        items.push((tag, &data[i + 1 + hdr..end], &data[i..end]));
        i = end;
    }
    items
}

/// Walk DER tree, find all SEQUENCE nodes matching the CertID pattern:
///   SEQUENCE { SEQUENCE (AlgID), OCTET STRING, OCTET STRING, INTEGER }
/// Returns Vec<(certid_der, serial_bytes)>.
fn extract_cert_ids(data: &[u8], out: &mut Vec<(Vec<u8>, Vec<u8>)>) {
    let children = parse_children(data);
    for (tag, content, full) in &children {
        if *tag == 0x30 {
            // Check if this matches CertID pattern
            let inner = parse_children(content);
            if inner.len() >= 4
                && inner[0].0 == 0x30  // AlgorithmIdentifier
                && inner[1].0 == 0x04  // issuerNameHash OCTET STRING
                && inner[2].0 == 0x04  // issuerKeyHash OCTET STRING
                && inner[3].0 == 0x02  // serialNumber INTEGER
            {
                out.push((full.to_vec(), inner[3].1.to_vec()));
            } else {
                extract_cert_ids(content, out);
            }
        } else if matches!(*tag, 0x31 | 0xa0 | 0xa1 | 0xa2 | 0xa3 | 0xa4 | 0xa5) {
            extract_cert_ids(content, out);
        }
    }
}

/// Convert serial bytes (DER INTEGER value) to UUID string.
/// Handles leading 0x00 padding byte from DER unsigned encoding.
fn serial_bytes_to_uuid(bytes: &[u8]) -> Option<String> {
    let b = if bytes.len() == 17 && bytes[0] == 0x00 { &bytes[1..] } else { bytes };
    if b.len() != 16 { return None; }
    let arr: [u8; 16] = b.try_into().ok()?;
    Some(uuid::Uuid::from_bytes(arr).to_string())
}

// ---------------------------------------------------------------------------
// OCSP cert status
// ---------------------------------------------------------------------------

enum OcspCertStatus {
    Good,
    Revoked { at: OffsetDateTime, reason: u8 },
    Unknown,
}

// ---------------------------------------------------------------------------
// Build RFC 6960 signed BasicOCSPResponse
// ---------------------------------------------------------------------------

fn build_basic_ocsp_response(
    keypair: &KeyPair,
    cert_id_der: &[u8],
    cert_status: &OcspCertStatus,
    now: OffsetDateTime,
    next_update: OffsetDateTime,
) -> Result<Vec<u8>, String> {
    // CertStatus encoding
    let cert_status_der = match cert_status {
        OcspCertStatus::Good => vec![0x80, 0x00],
        OcspCertStatus::Revoked { at, reason } => {
            let mut inner = der_generalized_time(at);
            if *reason != 0 {
                inner.extend_from_slice(&der_explicit(0, &der_tlv(0x0a, &[*reason])));
            }
            der_tlv(0xa1, &inner)
        }
        OcspCertStatus::Unknown => vec![0x82, 0x00],
    };

    // SingleResponse ::= SEQUENCE { certID, certStatus, thisUpdate, nextUpdate [0] }
    let mut single = Vec::new();
    single.extend_from_slice(cert_id_der);
    single.extend_from_slice(&cert_status_der);
    single.extend_from_slice(&der_generalized_time(&now));
    single.extend_from_slice(&der_explicit(0, &der_generalized_time(&next_update)));
    let single = der_sequence(&single);

    // ResponderID ::= [2] EXPLICIT KeyHash (SHA-1 of subjectPublicKey)
    use sha1::Digest;
    let key_hash = sha1::Sha1::digest(keypair.public_key_raw());
    let responder_id = der_tlv(0xa2, &der_octet_string(&key_hash));

    // ResponseData ::= SEQUENCE { responderID, producedAt, responses }
    let mut tbs = Vec::new();
    tbs.extend_from_slice(&responder_id);
    tbs.extend_from_slice(&der_generalized_time(&now));
    tbs.extend_from_slice(&der_sequence(&single)); // responses SEQUENCE OF SingleResponse
    let tbs = der_sequence(&tbs);

    // Sign tbsResponseData
    let signature = keypair.sign(&tbs).map_err(|e| e.to_string())?;

    // BasicOCSPResponse ::= SEQUENCE { tbsResponseData, signatureAlgorithm, signature }
    let mut basic = Vec::new();
    basic.extend_from_slice(&tbs);
    basic.extend_from_slice(&sig_alg_der(keypair));
    basic.extend_from_slice(&der_bit_string(&signature));
    let basic = der_sequence(&basic);

    Ok(basic)
}

fn build_ocsp_response_successful(basic_der: &[u8]) -> Vec<u8> {
    // OID 1.3.6.1.5.5.7.48.1.1  (id-pkix-ocsp-basic)
    let oid: &[u8] = &[0x06, 0x09, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x01, 0x01];
    let response_bytes = der_sequence(&[oid, der_octet_string(basic_der).as_slice()].concat());
    let mut resp = Vec::new();
    resp.extend_from_slice(&[0x0a, 0x01, 0x00]); // responseStatus: successful
    resp.extend_from_slice(&der_explicit(0, &response_bytes));
    der_sequence(&resp)
}

fn build_ocsp_error_response(status_byte: u8) -> Vec<u8> {
    der_sequence(&[0x0a, 0x01, status_byte])
}

// ---------------------------------------------------------------------------
// Public handler
// ---------------------------------------------------------------------------

pub struct OcspOutcome {
    pub http_status: u16,
    pub body: Vec<u8>,
    pub max_age_secs: u64,
}

pub fn handle_ocsp(config: &OcspConfig, request_der: &[u8]) -> OcspOutcome {
    let err = |status: u8| OcspOutcome {
        http_status: 200,
        body: build_ocsp_error_response(status),
        max_age_secs: 0,
    };

    if request_der.is_empty() { return err(1); /* MalformedRequest */ }

    // Extract CertIDs from request
    let mut cert_id_pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    extract_cert_ids(request_der, &mut cert_id_pairs);
    if cert_id_pairs.is_empty() { return err(1); }

    // Open keystore and load responder key
    let ks = match open_keystore(&config.keystore) {
        Ok(k) => k,
        Err(_) => return err(2), // InternalError
    };
    let key_pem = match ks.load_key_pem(&config.tenant_id, &config.responder_key_id) {
        Ok(p) => p,
        Err(_) => return err(2),
    };
    let keypair = match KeyPair::from_pem(&key_pem) {
        Ok(k) => k,
        Err(_) => return err(2),
    };

    // Open cert store
    let store = match OxPersistenceCertStore::open(config.store.db_path()) {
        Ok(s) => s,
        Err(_) => return err(2),
    };
    let tenant = &config.tenant_id;

    let now = OffsetDateTime::now_utc();
    let next_update = now + time::Duration::seconds(config.next_update_secs as i64);

    // Build a response for the first serial (batch support: clients typically request one)
    let (cert_id_der, serial_bytes) = &cert_id_pairs[0];

    let cert_status = match serial_bytes_to_uuid(serial_bytes) {
        Some(uuid_str) => {
            match store.get_cert_by_serial(tenant, &uuid_str) {
                Ok(Some(cert)) if cert.status == CertStatus::Revoked => {
                    OcspCertStatus::Revoked {
                        at: cert.revoked_at.unwrap_or(now),
                        reason: cert.revocation_reason.map(|r| r as u8).unwrap_or(0),
                    }
                }
                Ok(Some(_)) => OcspCertStatus::Good,
                Ok(None) => OcspCertStatus::Unknown,
                Err(_) => return err(3), // TryLater
            }
        }
        None => OcspCertStatus::Unknown,
    };

    match build_basic_ocsp_response(&keypair, cert_id_der, &cert_status, now, next_update) {
        Ok(basic_der) => OcspOutcome {
            http_status: 200,
            body: build_ocsp_response_successful(&basic_der),
            max_age_secs: config.max_age_secs,
        },
        Err(_) => err(2),
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
        config: OcspConfig,
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

    fn get_bytes(api: &CoreHostApi, ctx: *mut c_void, key: &str) -> Vec<u8> {
        let Ok(k) = CString::new(key) else { return vec![] };
        let mut len: usize = 0;
        let ptr = (api.get_field_bytes)(ctx, k.as_ptr(), &mut len);
        if ptr.is_null() || len == 0 { return vec![]; }
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    }

    fn set_bytes(api: &CoreHostApi, ctx: *mut c_void, key: &str, data: &[u8]) {
        if let Ok(k) = CString::new(key) {
            (api.set_field_bytes)(ctx, k.as_ptr(), data.as_ptr(), data.len());
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
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_ocsp: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: OcspConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_ocsp: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_ocsp: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_ocsp: initialized for tenant '{}', responder key '{}'",
                config.tenant_id, config.responder_key_id));
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

            if !path.starts_with("/ocsp") { return cont; }
            if method != "GET" && method != "POST" { return cont; }

            let request_der = if method == "POST" {
                get_bytes(&state.api, task_ctx, "request.body")
            } else {
                // GET /ocsp/{base64url-encoded-DER}
                let encoded = path.trim_start_matches("/ocsp/");
                if encoded.is_empty() {
                    // Bare GET — return a human-readable info page.
                    let html = concat!(
                        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"UTF-8\">",
                        "<title>OCSP Responder</title>",
                        "<style>body{font-family:system-ui,sans-serif;background:#0f172a;color:#f1f5f9;padding:2rem;max-width:700px;margin:auto}",
                        "h1{color:#38bdf8}code{background:rgba(0,0,0,.4);padding:.15rem .4rem;border-radius:3px;font-size:.9em;color:#a5b4fc}",
                        "pre{background:rgba(0,0,0,.4);border:1px solid #334155;border-radius:6px;padding:1rem;overflow-x:auto}",
                        "p,li{color:#94a3b8;line-height:1.7}</style></head>",
                        "<body><h1>OCSP Responder</h1>",
                        "<p>This is an Online Certificate Status Protocol (RFC 6960) responder. ",
                        "It is used automatically by browsers and TLS clients to check whether a certificate has been revoked.</p>",
                        "<h2>Usage</h2>",
                        "<p><strong>POST</strong> &mdash; send a DER-encoded <code>OCSPRequest</code>:</p>",
                        "<pre>curl -s -X POST /ocsp/ \\\n  -H 'Content-Type: application/ocsp-request' \\\n  --data-binary @request.der \\\n  -o response.der</pre>",
                        "<p><strong>GET</strong> &mdash; base64url-encode the DER request and append to the path:</p>",
                        "<pre>curl -s \"/ocsp/$(base64 -w0 request.der | tr '+/' '-_' | tr -d '=')\" -o response.der</pre>",
                        "<p>Responses are DER-encoded <code>OCSPResponse</code> with <code>Content-Type: application/ocsp-response</code>.</p>",
                        "<p><a href=\"/ca/\" style=\"color:#38bdf8\">&larr; Certificate Authority Dashboard</a></p>",
                        "</body></html>"
                    );
                    set(&state.api, task_ctx, "response.status", "200");
                    set(&state.api, task_ctx, "response.header.Content-Type", "text/html; charset=utf-8");
                    set_bytes(&state.api, task_ctx, "response.body", html.as_bytes());
                    return cont;
                }
                base64::Engine::decode(
                    &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                    encoded.as_bytes(),
                ).unwrap_or_default()
            };

            let outcome = handle_ocsp(&state.config, &request_der);

            set(&state.api, task_ctx, "response.status", &outcome.http_status.to_string());
            set(&state.api, task_ctx, "response.header.Content-Type", "application/ocsp-response");
            if outcome.max_age_secs > 0 {
                set(&state.api, task_ctx, "response.header.Cache-Control",
                    &format!("max-age={}", outcome.max_age_secs));
            }
            set_bytes(&state.api, task_ctx, "response.body", &outcome.body);
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_ocsp: panic");
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
