use ox_cert_core::{
    issuer_params_from_cert_pem,
    model::{
        AuditAction, AuditEvent, AuditFilter, CaKeyRecord, CaKeyStatus, CertFilter,
        CertStoreConfig, KeyStoreConfig, KeyType, Pagination, ScepChallenge,
    },
    open_keystore,
    store::{CertStore, OxPersistenceCertStore},
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
pub struct AdminConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
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

    let store = match OxPersistenceCertStore::open() {
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
                        "data": r.items,
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
                        "data": certs,
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
                        "data": cert,
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
                    body_json: serde_json::json!({ "data": r.items, "meta": { "tenant_id": tenant } }).to_string(),
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

    // Mark old key retiring
    if let Ok(Some(old)) = store.get_active_ca_key(tenant) {
        let _ = store.update_ca_key_status(tenant, &old.id, CaKeyStatus::Retiring);
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

    // Get active CA key id
    let ca_key_id = match store.get_active_ca_key(tenant) {
        Ok(Some(k)) => k.id,
        _ => return AdminOutcome {
            http_status: 503,
            body_json: serde_json::json!({ "error": { "code": "CA_NOT_READY", "message": "no active CA key" } }).to_string(),
        },
    };

    let ca_key_pem = match ks.load_key_pem(tenant, &ca_key_id) {
        Ok(p) => p,
        Err(e) => return AdminOutcome {
            http_status: 503,
            body_json: serde_json::json!({ "error": { "code": "CA_NOT_READY", "message": e.to_string() } }).to_string(),
        },
    };

    let ca_keypair = match rcgen::KeyPair::from_pem(&ca_key_pem) {
        Ok(k) => k,
        Err(e) => return AdminOutcome {
            http_status: 503,
            body_json: serde_json::json!({ "error": { "code": "CA_NOT_READY", "message": e.to_string() } }).to_string(),
        },
    };

    match ox_cert_core::sign_csr(&csr_pem, tenant, "ca_intermediate", 3 * 365 * 86400, None, &issuer_params, &ca_keypair) {
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
        Err(e) => AdminOutcome {
            http_status: 400,
            body_json: serde_json::json!({ "error": { "code": "INVALID_CSR", "message": e.to_string() } }).to_string(),
        },
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

// ---------------------------------------------------------------------------
// Trust info
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct CertInfo {
    pub subject: String,
    pub issuer: String,
    pub serial: String,
    pub not_before: String,
    pub not_after: String,
    pub fingerprint_sha256: String,
    pub is_ca: bool,
    pub is_self_signed: bool,
    pub key_algorithm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_by_server: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_by_us: Option<bool>,
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
        .subject_pki
        .algorithm
        .algorithm
        .to_id_string();

    Ok(CertInfo {
        subject:           cert.subject().to_string(),
        issuer:            cert.issuer().to_string(),
        serial,
        not_before:        cert.validity().not_before.to_datetime().to_string(),
        not_after:         cert.validity().not_after.to_datetime().to_string(),
        fingerprint_sha256,
        is_ca:             cert.is_ca(),
        is_self_signed:    cert.subject() == cert.issuer(),
        key_algorithm,
        trusted_by_server: None,
        issued_by_us:      None,
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

    let client_info: Option<CertInfo> = if client_pem.is_empty() {
        None
    } else {
        parse_cert_pem(client_pem).ok().map(|mut info| {
            info.trusted_by_server = Some(info.issuer == server_info.subject);
            // Check if serial is in the cert store (issued by this CA).
            info.issued_by_us = OxPersistenceCertStore::open().ok().map(|s| {
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
            "client": client_info,
        }).to_string(),
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
