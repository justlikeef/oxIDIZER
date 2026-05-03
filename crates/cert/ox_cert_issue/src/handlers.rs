use crate::config::CertIssueConfig;
use ox_cert_core::{
    builder::{issuer_params_from_cert_pem, parse_csr, sign_csr, CertBuilder},
    model::{
        ApprovalRequest, ApprovalStatus, AuditAction, AuditEvent, CertificateRecord,
        EnrollmentProtocol, IssuancePolicy, KeyType, SanType,
    },
    open_keystore,
    store::{CertStore, OxPersistenceCertStore},
    CertError,
};
use serde::Deserialize;
use time::OffsetDateTime;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct IssueRequest {
    pub csr: Option<String>,
    pub profile: Option<String>,
    pub validity_seconds: Option<u64>,
    /// Override/supplement SANs from the CSR. Strings may be DNS names or IP addresses.
    pub sans: Option<Vec<String>>,
    /// Key type for server-keygen mode (when `csr` is omitted).
    pub key_type: Option<String>,
    /// Subject DN string for server-keygen mode.
    pub subject: Option<String>,
}

pub struct IssueOutcome {
    pub http_status: u16,
    pub body_json: String,
    pub serial: Option<String>,
    pub not_after_rfc3339: Option<String>,
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn error_body(code: &str, message: &str, tenant_id: &str) -> String {
    serde_json::json!({
        "error": { "code": code, "message": message },
        "meta": { "tenant_id": tenant_id }
    })
    .to_string()
}

fn ok_body(record: &CertificateRecord, request_id: &str) -> String {
    let not_before = record.not_before.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let not_after = record.not_after.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    serde_json::json!({
        "data": {
            "serial": record.serial,
            "subject_cn": record.subject_cn,
            "subject_dn": record.subject_dn,
            "not_before": not_before,
            "not_after": not_after,
            "profile": record.profile,
            "certificate": record.pem,
            "scts": record.scts,
        },
        "meta": {
            "tenant_id": record.tenant_id,
            "request_id": request_id,
        }
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// SAN string → SanType
// ---------------------------------------------------------------------------

fn san_from_str(s: &str) -> SanType {
    if let Ok(ip) = s.parse::<std::net::IpAddr>() {
        SanType::Ip(ip)
    } else if s.contains('@') {
        SanType::Email(s.to_string())
    } else if s.starts_with("http://") || s.starts_with("https://") {
        SanType::Uri(s.to_string())
    } else {
        SanType::Dns(s.to_string())
    }
}

fn key_type_from_str(s: &str) -> Result<KeyType, CertError> {
    match s {
        "ecc-p256" | "ecdsa-p256" => Ok(KeyType::EcP256),
        "ecc-p384" | "ecdsa-p384" => Ok(KeyType::EcP384),
        "ecc-p521" | "ecdsa-p521" => Ok(KeyType::EcP521),
        "ed25519" => Ok(KeyType::Ed25519),
        "rsa-2048" => Ok(KeyType::Rsa2048),
        "rsa-3072" => Ok(KeyType::Rsa3072),
        "rsa-4096" => Ok(KeyType::Rsa4096),
        other => Err(CertError::Validation(format!("unknown key_type: {}", other))),
    }
}

// ---------------------------------------------------------------------------
// Core issue function
// ---------------------------------------------------------------------------

/// Process a certificate issuance request.
///
/// `ra_approved` — true if `cert.ra.approved == "true"` in TaskState
/// `webhook_enrichment` — raw JSON from `cert.webhook.enrichment` (may be None)
pub fn handle_issue(
    config: &CertIssueConfig,
    request_body: &str,
    content_type: &str,
    ra_approved: bool,
    webhook_enrichment: Option<&str>,
) -> Result<IssueOutcome, IssueError> {
    let tenant = &config.tenant_id;
    let request_id = Uuid::new_v4().to_string();

    // ------------------------------------------------------------------
    // 1. Parse request body
    // ------------------------------------------------------------------
    let req: IssueRequest = if content_type.contains("application/pkcs10") {
        IssueRequest {
            csr: Some(request_body.to_string()),
            profile: None,
            validity_seconds: None,
            sans: None,
            key_type: None,
            subject: None,
        }
    } else {
        serde_json::from_str(request_body).map_err(|e| IssueError {
            http_status: 400,
            error_code: "INVALID_REQUEST",
            message: format!("request body parse error: {}", e),
        })?
    };

    // ------------------------------------------------------------------
    // 2. Resolve profile and validity
    // ------------------------------------------------------------------
    let profile_name = req
        .profile
        .as_deref()
        .unwrap_or(&config.default_profile)
        .to_string();

    let policy = IssuancePolicy::from_config(&config.policy).map_err(|e| IssueError {
        http_status: 500,
        error_code: "INTERNAL_ERROR",
        message: format!("policy config error: {}", e),
    })?;

    // Default validity: 1 year; clamped by profile max if set
    let validity_seconds = req.validity_seconds.unwrap_or(365 * 86400);

    // ------------------------------------------------------------------
    // 3. Determine effective SANs (request body overrides CSR SANs)
    // ------------------------------------------------------------------
    let mut override_sans: Option<Vec<SanType>> = req
        .sans
        .as_ref()
        .map(|v| v.iter().map(|s| san_from_str(s)).collect());

    // Apply webhook enrichment additional_sans
    if let Some(enrichment_json) = webhook_enrichment {
        if let Ok(enrichment) = serde_json::from_str::<serde_json::Value>(enrichment_json) {
            if let Some(additional) = enrichment.get("additional_sans").and_then(|v| v.as_array()) {
                let extra: Vec<SanType> = additional
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(san_from_str)
                    .collect();
                if !extra.is_empty() {
                    override_sans
                        .get_or_insert_with(Vec::new)
                        .extend(extra);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 4–6. Parse CSR or prepare server-keygen
    // ------------------------------------------------------------------
    let store = OxPersistenceCertStore::open().map_err(|e| IssueError {
        http_status: 500,
        error_code: "INTERNAL_ERROR",
        message: e.to_string(),
    })?;

    let ks = open_keystore(&config.keystore).map_err(|e| IssueError {
        http_status: 503,
        error_code: "CA_NOT_READY",
        message: e.to_string(),
    })?;

    // Load intermediate CA params and key pair
    let ca_cert_pem = std::fs::read_to_string(&config.ca_intermediate_cert_path).map_err(|e| {
        IssueError {
            http_status: 503,
            error_code: "CA_NOT_READY",
            message: format!("intermediate cert read failed: {}", e),
        }
    })?;
    let ca_params = issuer_params_from_cert_pem(&ca_cert_pem).map_err(|e| IssueError {
        http_status: 503,
        error_code: "CA_NOT_READY",
        message: e.to_string(),
    })?;

    let ca_pem = ks
        .load_key_pem(tenant, &config.ca_intermediate_key_id)
        .map_err(|e| IssueError {
            http_status: 503,
            error_code: "CA_NOT_READY",
            message: format!("intermediate key load failed: {}", e),
        })?;
    let ca_key = rcgen::KeyPair::from_pem(&ca_pem).map_err(|e| IssueError {
        http_status: 503,
        error_code: "CA_NOT_READY",
        message: format!("intermediate key parse failed: {}", e),
    })?;

    let record = if let Some(csr_pem) = &req.csr {
        // ------------------------------------------------------------------
        // CSR-based issuance
        // ------------------------------------------------------------------
        let csr_info = parse_csr(csr_pem).map_err(|e| IssueError {
            http_status: 400,
            error_code: "INVALID_CSR",
            message: e.to_string(),
        })?;

        // Effective SANs for policy validation
        let effective_sans: Vec<SanType> = override_sans
            .as_deref()
            .unwrap_or(&csr_info.sans)
            .to_vec();
        let mut validation_info = csr_info.clone();
        validation_info.sans = effective_sans;

        policy.validate_csr(&validation_info).map_err(|msg| IssueError {
            http_status: 403,
            error_code: "POLICY_VIOLATION",
            message: msg,
        })?;

        // RA approval check
        if policy.require_ra_approval && !ra_approved {
            let ra_id = Uuid::new_v4().to_string();
            let now = OffsetDateTime::now_utc();
            let ra_req = ApprovalRequest {
                id: ra_id.clone(),
                tenant_id: tenant.clone(),
                csr_pem: csr_pem.clone(),
                requester_identity: String::new(),
                profile: profile_name.clone(),
                sans: sans_to_strings(validation_info.sans.as_slice()),
                status: ApprovalStatus::Pending,
                reviewer: None,
                review_notes: None,
                reviewed_at: None,
                certificate_serial: None,
                created_at: now,
            };
            let _ = store.store_ra_request(tenant, &ra_req);
            let _ = store.store_audit_event(tenant, &AuditEvent {
                id: 0,
                tenant_id: tenant.clone(),
                timestamp: now,
                action: AuditAction::Issue,
                serial: None,
                actor: String::new(),
                details: serde_json::json!({ "status": "pending_approval", "ra_request_id": ra_id }),
            });
            let body = serde_json::json!({
                "data": { "request_id": ra_id, "status": "pending_approval" },
                "meta": { "tenant_id": tenant, "request_id": request_id }
            }).to_string();
            return Ok(IssueOutcome {
                http_status: 202,
                body_json: body,
                serial: None,
                not_after_rfc3339: None,
            });
        }

        let mut cert = sign_csr(
            csr_pem,
            tenant,
            &profile_name,
            validity_seconds,
            override_sans.as_deref(),
            &ca_params,
            &ca_key,
        )
        .map_err(|e| IssueError {
            http_status: 500,
            error_code: "INTERNAL_ERROR",
            message: e.to_string(),
        })?;
        cert.enrollment_protocol = Some(EnrollmentProtocol::Rest);
        cert
    } else if let Some(key_type_str) = &req.key_type {
        // ------------------------------------------------------------------
        // Server-keygen mode
        // ------------------------------------------------------------------
        let key_type = key_type_from_str(key_type_str).map_err(|e| IssueError {
            http_status: 400,
            error_code: "INVALID_REQUEST",
            message: e.to_string(),
        })?;

        let serial = Uuid::new_v4().to_string();
        ks.generate_key(tenant, &serial, key_type.clone(), false)
            .map_err(|e| IssueError {
                http_status: 500,
                error_code: "INTERNAL_ERROR",
                message: format!("key generation failed: {}", e),
            })?;

        let key_pem = ks.load_key_pem(tenant, &serial).map_err(|e| IssueError {
            http_status: 500,
            error_code: "INTERNAL_ERROR",
            message: e.to_string(),
        })?;
        let subject_key = rcgen::KeyPair::from_pem(&key_pem).map_err(|e| IssueError {
            http_status: 500,
            error_code: "INTERNAL_ERROR",
            message: e.to_string(),
        })?;

        let subject = req.subject.as_deref().unwrap_or("CN=server");
        let effective_sans = override_sans.unwrap_or_default();
        // Validate sans from request against policy
        let csr_info = ox_cert_core::model::CsrInfo {
            subject_dn: subject.to_string(),
            subject_cn: extract_cn(subject),
            sans: effective_sans.clone(),
            key_type: key_type.clone(),
            key_bits: key_bits_for(key_type),
            public_key_der: vec![],
            raw_der: vec![],
        };
        policy.validate_csr(&csr_info).map_err(|msg| IssueError {
            http_status: 403,
            error_code: "POLICY_VIOLATION",
            message: msg,
        })?;

        let mut cert = CertBuilder::new()
            .subject(subject)
            .sans(effective_sans)
            .validity_seconds(validity_seconds)
            .profile(&profile_name)
            .sign_with_issuer(tenant, &subject_key, &ca_params, &ca_key)
            .map_err(|e| IssueError {
                http_status: 500,
                error_code: "INTERNAL_ERROR",
                message: e.to_string(),
            })?;
        cert.enrollment_protocol = Some(EnrollmentProtocol::Rest);
        cert
    } else {
        return Err(IssueError {
            http_status: 400,
            error_code: "INVALID_REQUEST",
            message: "request must include 'csr' or 'key_type'".to_string(),
        });
    };

    // ------------------------------------------------------------------
    // Store cert and audit event
    // ------------------------------------------------------------------
    let _ = store.store_cert(tenant, &record);
    let _ = store.store_audit_event(
        tenant,
        &AuditEvent {
            id: 0,
            tenant_id: tenant.clone(),
            timestamp: OffsetDateTime::now_utc(),
            action: AuditAction::Issue,
            serial: Some(record.serial.clone()),
            actor: String::new(),
            details: serde_json::json!({ "profile": record.profile }),
        },
    );

    let not_after_str = record
        .not_after
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let serial = record.serial.clone();
    let body = ok_body(&record, &request_id);

    Ok(IssueOutcome {
        http_status: 201,
        body_json: body,
        serial: Some(serial),
        not_after_rfc3339: Some(not_after_str),
    })
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

pub struct IssueError {
    pub http_status: u16,
    pub error_code: &'static str,
    pub message: String,
}

impl IssueError {
    pub fn to_body(&self, tenant_id: &str) -> String {
        error_body(self.error_code, &self.message, tenant_id)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sans_to_strings(sans: &[SanType]) -> Vec<String> {
    sans.iter()
        .map(|s| match s {
            SanType::Dns(n) => n.clone(),
            SanType::Ip(a) => a.to_string(),
            SanType::Email(e) => e.clone(),
            SanType::Uri(u) => u.clone(),
        })
        .collect()
}

fn extract_cn(dn: &str) -> String {
    for part in dn.split(',') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k.trim().eq_ignore_ascii_case("CN") {
                return v.trim().to_string();
            }
        }
    }
    dn.to_string()
}

fn key_bits_for(kt: KeyType) -> u32 {
    match kt {
        KeyType::EcP256 | KeyType::Ed25519 => 256,
        KeyType::EcP384 => 384,
        KeyType::EcP521 => 521,
        KeyType::Rsa2048 => 2048,
        KeyType::Rsa3072 => 3072,
        KeyType::Rsa4096 => 4096,
    }
}
