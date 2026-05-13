use ox_cert_core::{
    issuer_params_from_cert_pem,
    model::{
        AuditAction, AuditEvent, AuditFilter, CaKeyRecord, CaKeyStatus, CertFilter,
        CertStoreConfig, CertificateRecord, KeyStoreConfig, KeyType, Pagination, ScepChallenge,
    },
    open_keystore,
    store::{CertStore, OxPersistenceCertStore},
    CertError,
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

fn default_ca_root_key_id() -> String { "ca-root".to_string() }
fn default_ca_intermediate_key_id() -> String { "ca-intermediate".to_string() }

#[derive(Debug, Deserialize)]
pub struct AdminConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    #[serde(default = "default_ca_root_key_id")]
    pub ca_root_key_id: String,
    #[serde(default = "default_ca_intermediate_key_id")]
    pub ca_intermediate_key_id: String,
    #[serde(default)]
    pub extensions: ExtensionsConfig,
}

pub struct AdminOutcome {
    pub http_status: u16,
    pub body_json: String,
}

pub fn handle(config: &AdminConfig, method: &str, path: &str, query: &str, body: &str) -> AdminOutcome {
    let tenant = &config.tenant_id;
    let request_id = Uuid::new_v4().to_string();

    macro_rules! err {
        ($status:expr, $code:expr, $msg:expr) => {
            return AdminOutcome {
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

    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    match (method, segs.as_slice()) {
        // GET /api/v1/certificates
        ("GET", ["api", "v1", "certificates"]) => {
            let filter = cert_filter_from_query(query);
            let page = pagination_from_query(query);
            match store.list_certs(tenant, &filter, &page) {
                Ok(r) => AdminOutcome {
                    http_status: 200,
                    body_json: serde_json::json!({
                        "data": r.items.iter().map(cert_record_to_json).collect::<Vec<_>>(),
                        "meta": { "tenant_id": tenant, "total": r.total, "offset": r.offset, "limit": r.limit }
                    }).to_string(),
                },
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            }
        }

        // GET /api/v1/certificates/expiring
        ("GET", ["api", "v1", "certificates", "expiring"]) => {
            let days = parse_query_u32(query, "days", 30).min(365);
            match store.list_expiring(tenant, days) {
                Ok(certs) => AdminOutcome {
                    http_status: 200,
                    body_json: serde_json::json!({
                        "data": certs.iter().map(cert_record_to_json).collect::<Vec<_>>(),
                        "meta": { "tenant_id": tenant, "days": days }
                    }).to_string(),
                },
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            }
        }

        // GET /api/v1/certificates/{serial}
        ("GET", ["api", "v1", "certificates", serial]) => {
            match store.get_cert_by_serial(tenant, serial) {
                Ok(Some(cert)) => AdminOutcome {
                    http_status: 200,
                    body_json: serde_json::json!({
                        "data": cert_record_to_json(&cert),
                        "meta": { "tenant_id": tenant, "request_id": request_id }
                    }).to_string(),
                },
                Ok(None) => err!(404, "NOT_FOUND", format!("certificate '{}' not found", serial)),
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            }
        }

        // GET /api/v1/audit
        ("GET", ["api", "v1", "audit"]) => {
            let filter = audit_filter_from_query(query);
            let page = pagination_from_query(query);
            match store.get_audit_log(tenant, &filter, &page) {
                Ok(r) => AdminOutcome {
                    http_status: 200,
                    body_json: serde_json::json!({
                        "data": r.items,
                        "meta": { "tenant_id": tenant, "total": r.total }
                    }).to_string(),
                },
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            }
        }

        // GET /api/v1/ca
        ("GET", ["api", "v1", "ca"]) => {
            handle_get_ca(config, &store, &request_id)
        }

        // POST /api/v1/ca/rollover
        ("POST", ["api", "v1", "ca", "rollover"]) => {
            handle_ca_rollover(config, &store, body, &request_id)
        }

        // POST /api/v1/ca/rollover/commit
        ("POST", ["api", "v1", "ca", "rollover", "commit"]) => {
            handle_rollover_commit(config, &store, &request_id)
        }

        // POST /api/v1/ca/rollover/abort
        ("POST", ["api", "v1", "ca", "rollover", "abort"]) => {
            handle_rollover_abort(config, &store, &request_id)
        }

        // POST /api/v1/ca/cross-sign
        ("POST", ["api", "v1", "ca", "cross-sign"]) => {
            handle_cross_sign(config, &store, body, &request_id)
        }

        // GET /api/v1/ca/cross-sign
        ("GET", ["api", "v1", "ca", "cross-sign"]) => {
            // List cross-signed certs (profile = ca_intermediate)
            let filter = CertFilter { profile: Some("ca_intermediate".to_string()), ..Default::default() };
            let page = pagination_from_query(query);
            match store.list_certs(tenant, &filter, &page) {
                Ok(r) => AdminOutcome {
                    http_status: 200,
                    body_json: serde_json::json!({ "data": r.items.iter().map(cert_record_to_json).collect::<Vec<_>>(), "meta": { "tenant_id": tenant } }).to_string(),
                },
                Err(e) => err!(500, "INTERNAL_ERROR", e.to_string()),
            }
        }

        // POST /api/v1/scep/challenges
        ("POST", ["api", "v1", "scep", "challenges"]) => {
            handle_scep_provision(config, &store, &request_id)
        }

        // GET /api/v1/scep/challenges
        ("GET", ["api", "v1", "scep", "challenges"]) => {
            // Stub: return empty list (real impl queries DB for active challenges)
            AdminOutcome {
                http_status: 200,
                body_json: serde_json::json!({ "data": [], "meta": { "tenant_id": tenant } }).to_string(),
            }
        }

        // DELETE /api/v1/scep/challenges/{id}
        ("DELETE", ["api", "v1", "scep", "challenges", _id]) => {
            // Stub: acknowledge deletion
            AdminOutcome {
                http_status: 200,
                body_json: serde_json::json!({ "data": { "deleted": true }, "meta": { "tenant_id": tenant } }).to_string(),
            }
        }

        // GET /api/v1/ssh/ca
        ("GET", ["api", "v1", "ssh", "ca"]) => {
            AdminOutcome {
                http_status: 200,
                body_json: serde_json::json!({
                    "data": { "user_ca": null, "host_ca": null },
                    "meta": { "tenant_id": tenant }
                }).to_string(),
            }
        }

        _ => AdminOutcome { http_status: 404, body_json: "{}".to_string() },
    }
}

fn handle_get_ca(config: &AdminConfig, store: &OxPersistenceCertStore, request_id: &str) -> AdminOutcome {
    let tenant = &config.tenant_id;
    let now = OffsetDateTime::now_utc();

    let active_key = store.get_active_ca_key(tenant).ok().flatten();

    let intermediate_info = parse_cert_info(&config.ca_intermediate_cert_path, now);
    let root_info = parse_cert_info(&config.ca_root_cert_path, now);
    let rollover_active = active_key.as_ref().map(|k| k.status == CaKeyStatus::Retiring).unwrap_or(false);

    AdminOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "data": {
                "intermediate": intermediate_info,
                "root": root_info,
                "rollover_active": rollover_active,
            },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn parse_cert_info(path: &str, now: OffsetDateTime) -> serde_json::Value {
    use x509_parser::prelude::*;
    let Ok(pem_str) = std::fs::read_to_string(path) else {
        return serde_json::json!({ "error": "cert not found" });
    };
    let Ok(pem) = ::pem::parse(pem_str.as_bytes()) else {
        return serde_json::json!({ "error": "PEM parse error" });
    };
    let der = pem.into_contents();
    let Ok((_, cert)) = X509Certificate::from_der(&der) else {
        return serde_json::json!({ "error": "DER parse error" });
    };
    let not_after_ts = cert.validity().not_after.timestamp();
    let days_until = (not_after_ts - now.unix_timestamp()) / 86400;
    serde_json::json!({
        "subject_dn": cert.subject().to_string(),
        "not_before": cert.validity().not_before.to_datetime().to_string(),
        "not_after": cert.validity().not_after.to_datetime().to_string(),
        "days_until_expiry": days_until,
    })
}

fn handle_ca_rollover(config: &AdminConfig, store: &OxPersistenceCertStore, body: &str, request_id: &str) -> AdminOutcome {
    let tenant = &config.tenant_id;
    let ks = match open_keystore(&config.keystore) {
        Ok(k) => k,
        Err(e) => return AdminOutcome {
            http_status: 503,
            body_json: serde_json::json!({ "error": { "code": "CA_NOT_READY", "message": e.to_string() } }).to_string(),
        },
    };

    // Check no rollover in progress
    if let Ok(Some(key)) = store.get_active_ca_key(tenant) {
        if key.status == CaKeyStatus::Retiring {
            return AdminOutcome {
                http_status: 409,
                body_json: serde_json::json!({
                    "error": { "code": "INVALID_REQUEST", "message": "rollover already in progress" }
                }).to_string(),
            };
        }
    }

    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let key_type_str = v.get("key_type").and_then(|k| k.as_str()).unwrap_or("EcP384");
    let new_key_id = format!("ca-intermediate-rollover-{}", Uuid::new_v4());
    let key_type = match key_type_str {
        "Rsa2048" => KeyType::Rsa2048,
        "Rsa4096" => KeyType::Rsa4096,
        "EcP256" => KeyType::EcP256,
        _ => KeyType::EcP384,
    };

    if let Err(e) = ks.generate_key(tenant, &new_key_id, key_type.clone(), false) {
        return AdminOutcome {
            http_status: 500,
            body_json: serde_json::json!({ "error": { "code": "INTERNAL_ERROR", "message": e.to_string() } }).to_string(),
        };
    }

    // Load root CA key for signing new intermediate
    let ca_cert_pem = match std::fs::read_to_string(&config.ca_intermediate_cert_path) {
        Ok(s) => s,
        Err(e) => return AdminOutcome {
            http_status: 503,
            body_json: serde_json::json!({ "error": { "code": "CA_NOT_READY", "message": e.to_string() } }).to_string(),
        },
    };
    let issuer_params = match issuer_params_from_cert_pem(&ca_cert_pem) {
        Ok(p) => p,
        Err(e) => return AdminOutcome {
            http_status: 503,
            body_json: serde_json::json!({ "error": { "code": "CA_NOT_READY", "message": e.to_string() } }).to_string(),
        },
    };

    // Capture the currently-active key ID before storing the new one (pointer would otherwise update).
    let old_key_id = store.get_active_ca_key(tenant).ok().flatten().map(|k| k.id);

    let now = OffsetDateTime::now_utc();
    let new_record = CaKeyRecord {
        id: new_key_id.clone(),
        tenant_id: tenant.clone(),
        key_type,
        cert_pem: String::new(), // placeholder — would be built from signing
        key_ref: new_key_id.clone(),
        status: CaKeyStatus::Active,
        not_before: now,
        not_after: now + ::time::Duration::days(365 * 3),
        name_constraints: None,
        path_length: Some(0),
        created_at: now,
    };
    let _ = store.store_ca_key(tenant, &new_record);

    // Mark old key retiring now that the new key is stored and the pointer updated.
    if let Some(old_id) = old_key_id {
        let _ = store.update_ca_key_status(tenant, &old_id, CaKeyStatus::Retiring);
    }

    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0,
        tenant_id: tenant.clone(),
        timestamp: now,
        action: AuditAction::CaRollover,
        serial: None,
        actor: String::new(),
        details: serde_json::json!({ "new_key_id": new_key_id }),
    });

    let _ = issuer_params; // used above for validation

    AdminOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "data": { "new_key_id": new_key_id, "status": "rollover_started" },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn handle_rollover_commit(config: &AdminConfig, store: &OxPersistenceCertStore, request_id: &str) -> AdminOutcome {
    let tenant = &config.tenant_id;

    // Find retiring key
    let active = match store.get_active_ca_key(tenant) {
        Ok(Some(k)) if k.status == CaKeyStatus::Retiring => k,
        _ => return AdminOutcome {
            http_status: 409,
            body_json: serde_json::json!({ "error": { "code": "INVALID_REQUEST", "message": "no rollover in progress" } }).to_string(),
        },
    };

    let _ = store.update_ca_key_status(tenant, &active.id, CaKeyStatus::Retired);
    let now = OffsetDateTime::now_utc();
    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0, tenant_id: tenant.clone(), timestamp: now,
        action: AuditAction::CaRolloverCommit, serial: None, actor: String::new(),
        details: serde_json::json!({ "retired_key_id": active.id }),
    });

    AdminOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "data": { "status": "committed" },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn handle_rollover_abort(config: &AdminConfig, store: &OxPersistenceCertStore, request_id: &str) -> AdminOutcome {
    let tenant = &config.tenant_id;

    let active = match store.get_active_ca_key(tenant) {
        Ok(Some(k)) if k.status == CaKeyStatus::Retiring => k,
        _ => return AdminOutcome {
            http_status: 409,
            body_json: serde_json::json!({ "error": { "code": "INVALID_REQUEST", "message": "no rollover in progress" } }).to_string(),
        },
    };

    let _ = store.update_ca_key_status(tenant, &active.id, CaKeyStatus::Active);
    let now = OffsetDateTime::now_utc();
    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0, tenant_id: tenant.clone(), timestamp: now,
        action: AuditAction::CaRolloverAbort, serial: None, actor: String::new(),
        details: serde_json::json!({ "aborted_key_id": active.id }),
    });

    AdminOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "data": { "status": "aborted" },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn handle_cross_sign(config: &AdminConfig, store: &OxPersistenceCertStore, body: &str, request_id: &str) -> AdminOutcome {
    let tenant = &config.tenant_id;
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let csr_pem = match v.get("csr_pem").and_then(|c| c.as_str()) {
        Some(s) => s.to_string(),
        None => return AdminOutcome {
            http_status: 400,
            body_json: serde_json::json!({ "error": { "code": "INVALID_CSR", "message": "csr_pem is required" } }).to_string(),
        },
    };

    let ks = match open_keystore(&config.keystore) {
        Ok(k) => k,
        Err(e) => return AdminOutcome {
            http_status: 503,
            body_json: serde_json::json!({ "error": { "code": "CA_NOT_READY", "message": e.to_string() } }).to_string(),
        },
    };

    // Cross-sign always uses the root CA cert/key.
    // ca_init auto-generates an intermediate cert for the end-entity issuance pipeline
    // (ox_cert_issue), but that intermediate must never be used to cross-sign sub-CAs:
    // it would insert an anonymous extra level into the trust chain that clients cannot
    // verify.  Sub-CA certs must chain directly to this CA's root cert.
    let ca_cert_path = config.ca_root_cert_path.as_str();
    let ca_key_id = config.ca_root_key_id.as_str();
    let ca_cert_pem = match std::fs::read_to_string(ca_cert_path) {
        Ok(s) => s,
        Err(e) => return AdminOutcome {
            http_status: 503,
            body_json: serde_json::json!({ "error": { "code": "CA_NOT_READY", "message": e.to_string() } }).to_string(),
        },
    };

    let ca_key_pem = match ks.load_key_pem(tenant, ca_key_id) {
        Ok(p) => p,
        Err(e) => return AdminOutcome {
            http_status: 503,
            body_json: serde_json::json!({ "error": { "code": "CA_NOT_READY", "message": e.to_string() } }).to_string(),
        },
    };

    let safe_error_msg = |e: &CertError| -> String {
        // Strip null bytes so the message survives CString conversion in the plugin ABI.
        e.to_string().replace('\0', "<NUL>")
    };

    match ox_cert_core::cross_sign_csr_with_pem(&csr_pem, &ca_cert_pem, &ca_key_pem, tenant, 3 * 365) {
        Ok(record) => {
            let _ = store.store_cert(tenant, &record);
            let now = OffsetDateTime::now_utc();
            let _ = store.store_audit_event(tenant, &AuditEvent {
                id: 0, tenant_id: tenant.clone(), timestamp: now,
                action: AuditAction::CrossSign, serial: Some(record.serial.clone()),
                actor: String::new(), details: serde_json::json!({}),
            });
            AdminOutcome {
                http_status: 201,
                body_json: serde_json::json!({
                    "data": { "serial": record.serial, "pem": record.pem },
                    "meta": { "tenant_id": tenant, "request_id": request_id }
                }).to_string(),
            }
        }
        Err(e) => {
            let msg = safe_error_msg(&e);
            // Write to a known path so a failed deploy can be diagnosed without SSH.
            let debug = format!(
                "cross-sign error:\n  ca_cert_path: {}\n  ca_key_id:    {}\n  error:        {}\n",
                ca_cert_path, ca_key_id, msg
            );
            let _ = std::fs::write("/tmp/ox_cross_sign_error.txt", &debug);
            AdminOutcome {
                http_status: 400,
                body_json: serde_json::json!({ "error": { "code": "INVALID_CSR", "message": msg } }).to_string(),
            }
        }
    }
}

fn handle_scep_provision(config: &AdminConfig, store: &OxPersistenceCertStore, request_id: &str) -> AdminOutcome {
    let tenant = &config.tenant_id;
    let password = random_password(16);
    let id = Uuid::new_v4().to_string();
    let now = OffsetDateTime::now_utc();

    let challenge = ScepChallenge {
        id: id.clone(),
        tenant_id: tenant.clone(),
        password_hash: format!("sha256:{}", sha256_hex(password.as_bytes())),
        used: false,
        expires_at: now + ::time::Duration::hours(24),
        created_at: now,
    };
    let _ = store.store_scep_challenge(tenant, &challenge);

    AdminOutcome {
        http_status: 201,
        body_json: serde_json::json!({
            "data": { "id": id, "password": password, "expires_at": challenge.expires_at.to_string() },
            "meta": { "tenant_id": tenant, "request_id": request_id }
        }).to_string(),
    }
}

fn random_password(len: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len).map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    // Simple SHA-256 using ring
    use ring::digest;
    let d = digest::digest(&digest::SHA256, data);
    let mut s = String::with_capacity(64);
    for b in d.as_ref() { let _ = write!(s, "{:02x}", b); }
    s
}

fn cert_filter_from_query(query: &str) -> CertFilter {
    let mut f = CertFilter::default();
    for part in query.split('&') {
        let mut kv = part.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("subject_cn"), Some(v)) => f.subject_cn = Some(v.to_string()),
            (Some("san"), Some(v)) => f.san = Some(v.to_string()),
            (Some("profile"), Some(v)) => f.profile = Some(v.to_string()),
            _ => {}
        }
    }
    f
}

fn audit_filter_from_query(_query: &str) -> AuditFilter {
    AuditFilter::default()
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

fn parse_query_u32(query: &str, key: &str, default: u32) -> u32 {
    for part in query.split('&') {
        let mut kv = part.splitn(2, '=');
        if kv.next() == Some(key) {
            if let Some(v) = kv.next() {
                return v.parse().unwrap_or(default);
            }
        }
    }
    default
}

fn fmt_dt(dt: time::OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}

fn cert_record_to_json(cert: &CertificateRecord) -> serde_json::Value {
    serde_json::json!({
        "serial":      cert.serial,
        "tenant_id":   cert.tenant_id,
        "subject_cn":  cert.subject_cn,
        "subject_dn":  cert.subject_dn,
        "sans":        cert.sans,
        "issuer_dn":   cert.issuer_dn,
        "not_before":  fmt_dt(cert.not_before),
        "not_after":   fmt_dt(cert.not_after),
        "key_type":    cert.key_type,
        "profile":     cert.profile,
        "status":      cert.status,
        "revoked_at":  cert.revoked_at.map(fmt_dt),
        "created_at":  fmt_dt(cert.created_at),
        "pem":         cert.pem,
    })
}

// ---------------------------------------------------------------------------
// Trust info
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct CertExtension {
    pub name: String,
    pub critical: bool,
    pub value: String,
}

#[derive(serde::Serialize)]
pub struct CertInfo {
    pub version: u32,
    pub serial: String,
    pub signature_algorithm: String,
    pub subject: String,
    pub issuer: String,
    pub not_before: String,
    pub not_after: String,
    pub key_algorithm: String,
    pub extensions: Vec<CertExtension>,
    pub fingerprint_sha256: String,
    pub is_ca: bool,
    pub is_self_signed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_by_server: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_by_us: Option<bool>,
}

fn format_general_name(gn: &x509_parser::extensions::GeneralName) -> String {
    use x509_parser::extensions::GeneralName;
    match gn {
        GeneralName::DNSName(s)      => format!("DNS:{}", s),
        GeneralName::RFC822Name(s)   => format!("email:{}", s),
        GeneralName::URI(s)          => format!("URI:{}", s),
        GeneralName::DirectoryName(dn) => format!("DirName:{}", dn),
        GeneralName::RegisteredID(oid) => format!("OID:{}", oid.to_id_string()),
        GeneralName::IPAddress(ip) => {
            if ip.len() == 4 {
                format!("IP:{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
            } else {
                format!("IP:{}", ip.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(":"))
            }
        },
        GeneralName::OtherName(oid, _) => format!("OtherName:{}", oid.to_id_string()),
        _ => "(unknown GeneralName)".to_string(),
    }
}

fn format_extension(ext: &x509_parser::extensions::X509Extension) -> (String, String) {
    use x509_parser::extensions::ParsedExtension;

    let name = match ext.parsed_extension() {
        ParsedExtension::AuthorityKeyIdentifier(_)  => "Authority Key Identifier",
        ParsedExtension::SubjectKeyIdentifier(_)    => "Subject Key Identifier",
        ParsedExtension::KeyUsage(_)                => "Key Usage",
        ParsedExtension::CertificatePolicies(_)     => "Certificate Policies",
        ParsedExtension::SubjectAlternativeName(_)  => "Subject Alternative Name",
        ParsedExtension::IssuerAlternativeName(_)   => "Issuer Alternative Name",
        ParsedExtension::BasicConstraints(_)        => "Basic Constraints",
        ParsedExtension::NameConstraints(_)         => "Name Constraints",
        ParsedExtension::PolicyConstraints(_)       => "Policy Constraints",
        ParsedExtension::ExtendedKeyUsage(_)        => "Extended Key Usage",
        ParsedExtension::CRLDistributionPoints(_)   => "CRL Distribution Points",
        ParsedExtension::InhibitAnyPolicy(_)        => "Inhibit Any Policy",
        ParsedExtension::AuthorityInfoAccess(_)     => "Authority Info Access",
        ParsedExtension::NSCertType(_)              => "Netscape Cert Type",
        ParsedExtension::NsCertComment(_)           => "Netscape Comment",
        ParsedExtension::UnsupportedExtension { .. } => "Unsupported Extension",
        ParsedExtension::ParseError { .. }          => "Parse Error",
        _                                           => "Unknown Extension",
    }.to_string();

    let value = match ext.parsed_extension() {
        ParsedExtension::SubjectKeyIdentifier(ki) => {
            ki.0.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(":")
        },
        ParsedExtension::AuthorityKeyIdentifier(aki) => {
            let mut parts = Vec::new();
            if let Some(ki) = &aki.key_identifier {
                parts.push(format!("keyid:{}", ki.0.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(":")));
            }
            if let Some(issuers) = &aki.authority_cert_issuer {
                for gn in issuers {
                    parts.push(format!("issuer:{}", format_general_name(gn)));
                }
            }
            if let Some(serial) = aki.authority_cert_serial {
                parts.push(format!("serial:{}", serial.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(":")));
            }
            parts.join(", ")
        },
        ParsedExtension::KeyUsage(ku) => {
            let mut bits = Vec::new();
            if ku.digital_signature() { bits.push("Digital Signature"); }
            if ku.non_repudiation()   { bits.push("Non Repudiation"); }
            if ku.key_encipherment()  { bits.push("Key Encipherment"); }
            if ku.data_encipherment() { bits.push("Data Encipherment"); }
            if ku.key_agreement()     { bits.push("Key Agreement"); }
            if ku.key_cert_sign()     { bits.push("Certificate Sign"); }
            if ku.crl_sign()          { bits.push("CRL Sign"); }
            if ku.encipher_only()     { bits.push("Encipher Only"); }
            if ku.decipher_only()     { bits.push("Decipher Only"); }
            bits.join(", ")
        },
        ParsedExtension::BasicConstraints(bc) => {
            let mut s = if bc.ca { "CA:TRUE".to_string() } else { "CA:FALSE".to_string() };
            if let Some(pl) = bc.path_len_constraint {
                s.push_str(&format!(", pathlen:{}", pl));
            }
            s
        },
        ParsedExtension::SubjectAlternativeName(san) => {
            san.general_names.iter().map(format_general_name).collect::<Vec<_>>().join(", ")
        },
        ParsedExtension::IssuerAlternativeName(ian) => {
            ian.general_names.iter().map(format_general_name).collect::<Vec<_>>().join(", ")
        },
        ParsedExtension::ExtendedKeyUsage(eku) => {
            let mut uses: Vec<String> = Vec::new();
            if eku.server_auth    { uses.push("TLS Web Server Authentication".into()); }
            if eku.client_auth    { uses.push("TLS Web Client Authentication".into()); }
            if eku.code_signing   { uses.push("Code Signing".into()); }
            if eku.email_protection { uses.push("E-mail Protection".into()); }
            if eku.time_stamping  { uses.push("Time Stamping".into()); }
            if eku.ocsp_signing   { uses.push("OCSP Signing".into()); }
            for oid in &eku.other { uses.push(oid.to_id_string()); }
            uses.join(", ")
        },
        ParsedExtension::CRLDistributionPoints(cdp) => {
            use x509_parser::extensions::DistributionPointName;
            cdp.iter()
                .filter_map(|dp| dp.distribution_point.as_ref())
                .map(|dpn| match dpn {
                    DistributionPointName::FullName(names) =>
                        names.iter().map(format_general_name).collect::<Vec<_>>().join(", "),
                    DistributionPointName::NameRelativeToCRLIssuer(rdn) => format!("{:?}", rdn),
                })
                .collect::<Vec<_>>()
                .join("; ")
        },
        ParsedExtension::AuthorityInfoAccess(aia) => {
            aia.iter()
                .map(|ad| format!("{}: {}", ad.access_method.to_id_string(), format_general_name(&ad.access_location)))
                .collect::<Vec<_>>()
                .join(", ")
        },
        ParsedExtension::NsCertComment(s) => s.to_string(),
        ParsedExtension::UnsupportedExtension { oid } => format!("OID {}", oid.to_id_string()),
        _ => "(not decoded)".to_string(),
    };

    (name, value)
}

fn parse_cert_pem(pem_str: &str) -> Result<CertInfo, String> {
    use x509_parser::prelude::*;

    let pem = ::pem::parse(pem_str).map_err(|e| e.to_string())?;
    let der = pem.into_contents();

    let (_, cert) = X509Certificate::from_der(&der).map_err(|e| e.to_string())?;

    let fp = ring::digest::digest(&ring::digest::SHA256, &der);
    let fingerprint_sha256 = fp.as_ref().iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");

    let serial = cert.raw_serial().iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");

    let key_algorithm = cert.tbs_certificate
        .subject_pki.algorithm.algorithm.to_id_string();

    let signature_algorithm = cert.signature_algorithm.algorithm.to_id_string();

    let extensions = cert.tbs_certificate.iter_extensions()
        .map(|ext| {
            let (name, value) = format_extension(ext);
            CertExtension { name, critical: ext.critical, value }
        })
        .collect::<Vec<_>>();

    Ok(CertInfo {
        version:             cert.version().0 + 1,
        serial,
        signature_algorithm,
        subject:             cert.subject().to_string(),
        issuer:              cert.issuer().to_string(),
        not_before:          cert.validity().not_before.to_datetime().to_string(),
        not_after:           cert.validity().not_after.to_datetime().to_string(),
        key_algorithm,
        extensions,
        fingerprint_sha256,
        is_ca:               cert.is_ca(),
        is_self_signed:      cert.subject() == cert.issuer(),
        trusted_by_server:   None,
        issued_by_us:        None,
    })
}

pub fn trust_info(config: &AdminConfig, client_pem: &str) -> AdminOutcome {
    let server_pem = match std::fs::read_to_string(&config.ca_root_cert_path) {
        Ok(p) => p,
        Err(_) => return AdminOutcome {
            http_status: 503,
            body_json: r#"{"error":{"code":"CA_NOT_READY","message":"root certificate not yet generated"}}"#.to_string(),
        },
    };

    let server_info = match parse_cert_pem(&server_pem) {
        Ok(i) => i,
        Err(e) => return AdminOutcome {
            http_status: 500,
            body_json: serde_json::json!({
                "error": { "code": "CERT_PARSE_ERROR", "message": e }
            }).to_string(),
        },
    };

    // Build the issuing chain above this CA cert.
    // Look for chain.pem first (installed via --chain), then parent-root-ca.crt (TOFU download).
    let ca_dir = std::path::Path::new(&config.ca_root_cert_path)
        .parent()
        .unwrap_or(std::path::Path::new("/etc/pki/ox_webservice/ca"));
    let chain_candidates = [ca_dir.join("chain.pem"), ca_dir.join("parent-root-ca.crt")];
    let mut chain: Vec<CertInfo> = Vec::new();
    for path in &chain_candidates {
        if let Ok(pem_text) = std::fs::read_to_string(path) {
            for single_pem in split_pem_certs(&pem_text) {
                if let Ok(info) = parse_cert_pem(&single_pem) {
                    // Exclude the server cert itself (e.g. if chain.pem is the same file)
                    if info.fingerprint_sha256 != server_info.fingerprint_sha256 {
                        chain.push(info);
                    }
                }
            }
            if !chain.is_empty() { break; }
        }
    }

    let client_info: Option<CertInfo> = if client_pem.is_empty() {
        None
    } else {
        parse_cert_pem(client_pem).ok().map(|mut info| {
            info.trusted_by_server = Some(info.issuer == server_info.subject);
            // Check if serial is in the cert store (issued by this CA).
            info.issued_by_us = OxPersistenceCertStore::open(config.store.db_path()).ok().map(|s| {
                let serial_clean = info.serial.replace(':', "");
                s.get_cert_by_serial(&config.tenant_id, &serial_clean).ok().flatten().is_some()
            });
            info
        })
    };

    AdminOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "server": server_info,
            "chain": chain,
            "client": client_info,
        }).to_string(),
    }
}

fn split_pem_certs(pem_str: &str) -> Vec<String> {
    let mut certs = Vec::new();
    let mut current = String::new();
    for line in pem_str.lines() {
        current.push_str(line);
        current.push('\n');
        if line.trim_end() == "-----END CERTIFICATE-----" {
            certs.push(current.clone());
            current.clear();
        }
    }
    certs
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
        config: AdminConfig,
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
        // CString fails on interior null bytes; replace them so binary error messages
        // (e.g. x509-parser printing raw DER bytes) don't silently swallow the body.
        let sanitized = val.replace('\0', "");
        if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(sanitized)) {
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
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_admin: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: AdminConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_admin: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_admin: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_admin: initialized for tenant '{}'", config.tenant_id));
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
            let query = get(&state.api, task_ctx, "request.query");
            let body = get(&state.api, task_ctx, "request.body");

            // Root CA certificate download — served directly from the configured cert path.
            if method == "GET" && path == "/ca/root.crt" {
                match std::fs::read_to_string(&state.config.ca_root_cert_path) {
                    Ok(pem) => {
                        set(&state.api, task_ctx, "response.status", "200");
                        set(&state.api, task_ctx, "response.body", &pem);
                        set(&state.api, task_ctx, "response.header.Content-Type", "application/x-pem-file");
                        set(&state.api, task_ctx, "response.header.Content-Disposition",
                            "attachment; filename=\"root.crt\"");
                    }
                    Err(_) => {
                        set(&state.api, task_ctx, "response.status", "503");
                        set(&state.api, task_ctx, "response.body", r#"{"error":{"code":"CA_NOT_READY","message":"root certificate not yet generated"}}"#);
                        set(&state.api, task_ctx, "response.header.Content-Type", "application/json");
                    }
                }
                return cont;
            }

            // Trust info endpoint — served directly here (does not go through handle()).
            if method == "GET" && path == "/api/v1/trust/info" {
                let client_pem = {
                    let h = get(&state.api, task_ctx, "request.tls.client_cert");
                    if h.is_empty() { get(&state.api, task_ctx, "request.header.X-Client-Cert") }
                    else { h }
                };
                let outcome = trust_info(&state.config, &client_pem);
                set(&state.api, task_ctx, "response.status", &outcome.http_status.to_string());
                set(&state.api, task_ctx, "response.body", &outcome.body_json);
                set(&state.api, task_ctx, "response.header.Content-Type", "application/json");
                return cont;
            }

            let is_admin_path = path.starts_with("/api/v1/certificates")
                || path.starts_with("/api/v1/audit")
                || path.starts_with("/api/v1/ca")
                || path.starts_with("/api/v1/ssh/ca")
                || path.starts_with("/api/v1/scep")
                || path.starts_with("/api/v1/trust");
            if !is_admin_path { return cont; }

            let outcome = handle(&state.config, &method, &path, &query, &body);
            set(&state.api, task_ctx, "response.status", &outcome.http_status.to_string());
            set(&state.api, task_ctx, "response.body", &outcome.body_json);
            set(&state.api, task_ctx, "response.header.Content-Type", "application/json");
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_admin: panic");
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
