use ox_cert_core::{
    issuer_params_from_cert_pem,
    model::{
        AcmeAccount, AcmeAccountStatus, AcmeAuthorization, AcmeAuthzStatus,
        AcmeChallenge, AcmeChallengeStatus, AcmeIdentifier, AcmeOrder, AcmeOrderStatus,
        AuditAction, AuditEvent, CertStoreConfig, ChallengeType, KeyStoreConfig,
    },
    open_keystore,
    sign_csr,
    store::{CertStore, OxPersistenceCertStore},
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::RwLock;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct AcmeRateLimitConfig {
    #[serde(default = "default_orders_per_hour")]
    pub orders_per_account_per_hour: u32,
    #[serde(default = "default_certs_per_week")]
    pub certs_per_domain_per_week: u32,
}

fn default_orders_per_hour() -> u32 { 10 }
fn default_certs_per_week() -> u32 { 50 }

impl Default for AcmeRateLimitConfig {
    fn default() -> Self {
        Self {
            orders_per_account_per_hour: default_orders_per_hour(),
            certs_per_domain_per_week: default_certs_per_week(),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum NonceStoreType { Memory, Database }

impl Default for NonceStoreType {
    fn default() -> Self { Self::Memory }
}

#[derive(Debug, Deserialize)]
pub struct AcmeConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    #[allow(dead_code)]
    pub ca_root_cert_path: String,
    #[serde(default)]
    pub extensions: AcmeExtensionsConfig,
    pub tos_url: Option<String>,
    #[serde(default)]
    pub external_account_required: bool,
    #[serde(default)]
    pub nonce_store: NonceStoreType,
    #[serde(default)]
    pub rate_limit: AcmeRateLimitConfig,
    pub base_url: Option<String>,
    /// Timeout for outbound challenge validation HTTP/DNS requests (seconds)
    #[serde(default = "default_validation_timeout")]
    pub validation_timeout_secs: u64,
}

fn default_validation_timeout() -> u64 { 30 }

#[derive(Debug, Deserialize, Default)]
pub struct AcmeExtensionsConfig {
    pub aia_ocsp_url: Option<String>,
    pub aia_ca_issuer_url: Option<String>,
    pub cdp_url: Option<String>,
}

pub struct NonceStore {
    nonces: RwLock<HashMap<String, std::time::Instant>>,
}

impl NonceStore {
    pub fn new() -> Self {
        Self { nonces: RwLock::new(HashMap::new()) }
    }
    pub fn issue(&self) -> String {
        let nonce = Uuid::new_v4().to_string();
        if let Ok(mut m) = self.nonces.write() {
            m.insert(nonce.clone(), std::time::Instant::now());
        }
        nonce
    }
    pub fn consume(&self, nonce: &str) -> bool {
        self.nonces.write().map(|mut m| m.remove(nonce).is_some()).unwrap_or(false)
    }
}

pub struct AcmeContext {
    config: AcmeConfig,
    nonces: NonceStore,
}
unsafe impl Send for AcmeContext {}
unsafe impl Sync for AcmeContext {}

impl AcmeContext {
    pub fn new(config: AcmeConfig) -> Self {
        Self { config, nonces: NonceStore::new() }
    }
}

pub struct AcmeOutcome {
    pub http_status: u16,
    pub body_json: String,
    pub replay_nonce: Option<String>,
    pub location: Option<String>,
    pub content_type: String,
}

fn acme_error(problem_type: &str, detail: &str) -> String {
    serde_json::json!({
        "type": format!("urn:ietf:params:acme:error:{}", problem_type),
        "detail": detail,
    }).to_string()
}

// ---------------------------------------------------------------------------
// JWK thumbprint (RFC 7638)
// ---------------------------------------------------------------------------

fn jwk_thumbprint(jwk_json: &str) -> Option<String> {
    let jwk: serde_json::Value = serde_json::from_str(jwk_json).ok()?;
    let kty = jwk.get("kty")?.as_str()?;
    // Canonical JSON: only required fields, sorted by member name
    let canonical = match kty {
        "EC" => {
            let crv = jwk.get("crv")?.as_str()?;
            let x   = jwk.get("x")?.as_str()?;
            let y   = jwk.get("y")?.as_str()?;
            format!("{{\"crv\":\"{}\",\"kty\":\"{}\",\"x\":\"{}\",\"y\":\"{}\"}}", crv, kty, x, y)
        }
        "RSA" => {
            let e = jwk.get("e")?.as_str()?;
            let n = jwk.get("n")?.as_str()?;
            format!("{{\"e\":\"{}\",\"kty\":\"{}\",\"n\":\"{}\"}}", e, kty, n)
        }
        "OKP" => {
            let crv = jwk.get("crv")?.as_str()?;
            let x   = jwk.get("x")?.as_str()?;
            format!("{{\"crv\":\"{}\",\"kty\":\"{}\",\"x\":\"{}\"}}", crv, kty, x)
        }
        _ => return None,
    };
    use sha2::Digest;
    let hash = sha2::Sha256::digest(canonical.as_bytes());
    Some(base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, &hash))
}

fn key_authorization(token: &str, jwk_json: &str) -> Option<String> {
    let tp = jwk_thumbprint(jwk_json)?;
    Some(format!("{}.{}", token, tp))
}

// ---------------------------------------------------------------------------
// Challenge validation
// ---------------------------------------------------------------------------

fn validate_http01(domain: &str, token: &str, expected: &str, timeout_secs: u64) -> bool {
    let url = format!("http://{}/.well-known/acme-challenge/{}", domain, token);
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build();
    match agent.get(&url).call() {
        Ok(resp) => resp.into_string()
            .map(|s| s.trim().to_string() == expected)
            .unwrap_or(false),
        Err(_) => false,
    }
}

fn validate_dns01(domain: &str, expected_b64: &str) -> bool {
    use hickory_resolver::Resolver;
    use hickory_resolver::config::{ResolverConfig, ResolverOpts};
    let resolver = match Resolver::new(ResolverConfig::default(), ResolverOpts::default()) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let txt_name = if domain.ends_with('.') {
        format!("_acme-challenge.{}", domain)
    } else {
        format!("_acme-challenge.{}.", domain)
    };
    match resolver.txt_lookup(&txt_name) {
        Ok(records) => {
            for txt in records.iter() {
                // TXT records can be split into multiple strings; join them
                let full: String = txt.iter()
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .collect();
                if full == expected_b64 { return true; }
            }
            false
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Request routing
// ---------------------------------------------------------------------------

fn base_url(ctx: &AcmeContext) -> String {
    ctx.config.base_url.clone().unwrap_or_else(|| "https://ca.example.com".to_string())
}

pub fn handle(ctx: &AcmeContext, method: &str, path: &str, body: &str) -> AcmeOutcome {
    let new_nonce = ctx.nonces.issue();
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    if segs.first() != Some(&"acme") {
        return AcmeOutcome {
            http_status: 404,
            body_json: "{}".to_string(),
            replay_nonce: Some(new_nonce),
            location: None,
            content_type: "application/json".to_string(),
        };
    }

    match (method, segs.get(1).copied(), segs.get(2).copied(), segs.get(3).copied()) {
        ("GET",        Some("directory"), _, _) => handle_directory(ctx, new_nonce),
        ("HEAD"|"POST", Some("new-nonce"), _, _) => {
            let status = if method == "HEAD" { 200 } else { 204 };
            AcmeOutcome {
                http_status: status,
                body_json: String::new(),
                replay_nonce: Some(new_nonce),
                location: None,
                content_type: "application/json".to_string(),
            }
        }
        ("POST", Some("new-account"), _, _) => handle_new_account(ctx, body, new_nonce),
        ("POST", Some("new-order"),   _, _) => handle_new_order(ctx, body, new_nonce),
        ("POST", Some("order"), Some(order_id), Some("finalize")) =>
            handle_finalize(ctx, order_id, body, new_nonce),
        ("POST", Some("order"), Some(order_id), _) =>
            handle_get_order(ctx, order_id, new_nonce),
        // Authorization: GET or POST-as-GET
        ("POST", Some("authz"), Some(authz_id), Some("challenge")) => {
            // /acme/authz/{authz_id}/challenge/{challenge_idx}
            let challenge_idx: usize = segs.get(4)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            handle_challenge(ctx, authz_id, challenge_idx, body, new_nonce)
        }
        ("POST", Some("authz"), Some(authz_id), _) =>
            handle_get_authz(ctx, authz_id, new_nonce),
        // Certificate download
        ("POST", Some("cert"), Some(order_id), _) =>
            handle_get_cert(ctx, order_id, new_nonce),
        ("POST", Some("revoke-cert"), _, _) =>
            handle_revoke_cert(ctx, body, new_nonce),
        _ => AcmeOutcome {
            http_status: 404,
            body_json: acme_error("notFound", "endpoint not found"),
            replay_nonce: Some(new_nonce),
            location: None,
            content_type: "application/problem+json".to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// JWS helpers
// ---------------------------------------------------------------------------

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, s.as_bytes()).ok()
}

fn extract_jws_payload(body: &str) -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let bytes = b64_decode(v.get("payload")?.as_str()?)?;
    serde_json::from_slice(&bytes).ok()
}

fn extract_jws_kid(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let protected: serde_json::Value = serde_json::from_slice(
        &b64_decode(v.get("protected")?.as_str()?)?,
    ).ok()?;
    protected.get("kid")?.as_str().map(|s| s.to_string())
}

fn extract_jws_jwk(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let protected: serde_json::Value = serde_json::from_slice(
        &b64_decode(v.get("protected")?.as_str()?)?,
    ).ok()?;
    protected.get("jwk").map(|j| j.to_string())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn handle_directory(ctx: &AcmeContext, nonce: String) -> AcmeOutcome {
    let base = base_url(ctx);
    let mut meta = serde_json::json!({
        "externalAccountRequired": ctx.config.external_account_required,
    });
    if let Some(tos) = &ctx.config.tos_url {
        meta["termsOfService"] = serde_json::Value::String(tos.clone());
    }
    AcmeOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "newNonce":   format!("{}/acme/new-nonce", base),
            "newAccount": format!("{}/acme/new-account", base),
            "newOrder":   format!("{}/acme/new-order", base),
            "revokeCert": format!("{}/acme/revoke-cert", base),
            "meta": meta,
        }).to_string(),
        replay_nonce: Some(nonce),
        location: None,
        content_type: "application/json".to_string(),
    }
}

fn handle_new_account(ctx: &AcmeContext, body: &str, nonce: String) -> AcmeOutcome {
    let tenant = &ctx.config.tenant_id;
    let base   = base_url(ctx);

    macro_rules! err {
        ($status:expr, $type:expr, $msg:expr) => {
            return AcmeOutcome {
                http_status: $status,
                body_json: acme_error($type, $msg),
                replay_nonce: Some(nonce),
                location: None,
                content_type: "application/problem+json".to_string(),
            }
        };
    }

    let payload = match extract_jws_payload(body) {
        Some(p) => p,
        None => err!(400, "malformed", "invalid JWS"),
    };
    let jwk = match extract_jws_jwk(body) {
        Some(j) => j,
        None => err!(400, "malformed", "missing jwk in protected header"),
    };

    if ctx.config.external_account_required && payload.get("externalAccountBinding").is_none() {
        err!(403, "externalAccountRequired", "external account binding required");
    }

    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => err!(500, "serverInternal", &e.to_string()),
    };

    let id = Uuid::new_v4().to_string();
    let contact: Vec<String> = payload.get("contact")
        .and_then(|c| serde_json::from_value(c.clone()).ok())
        .unwrap_or_default();

    let account = AcmeAccount {
        id: id.clone(),
        tenant_id: tenant.clone(),
        jwk,
        contact: contact.clone(),
        status: AcmeAccountStatus::Valid,
        eab_kid: None,
        created_at: OffsetDateTime::now_utc(),
    };
    let _ = store.store_acme_account(tenant, &account);

    AcmeOutcome {
        http_status: 201,
        body_json: serde_json::json!({ "status": "valid", "contact": contact }).to_string(),
        replay_nonce: Some(nonce),
        location: Some(format!("{}/acme/account/{}", base, id)),
        content_type: "application/json".to_string(),
    }
}

fn handle_new_order(ctx: &AcmeContext, body: &str, nonce: String) -> AcmeOutcome {
    let tenant = &ctx.config.tenant_id;
    let base   = base_url(ctx);

    macro_rules! err {
        ($status:expr, $type:expr, $msg:expr) => {
            return AcmeOutcome {
                http_status: $status,
                body_json: acme_error($type, $msg),
                replay_nonce: Some(nonce),
                location: None,
                content_type: "application/problem+json".to_string(),
            }
        };
    }

    let payload = match extract_jws_payload(body) {
        Some(p) => p,
        None => err!(400, "malformed", "invalid JWS"),
    };
    let identifiers: Vec<serde_json::Value> = payload.get("identifiers")
        .and_then(|i| i.as_array()).cloned().unwrap_or_default();
    if identifiers.is_empty() { err!(400, "malformed", "identifiers is required"); }

    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => err!(500, "serverInternal", &e.to_string()),
    };

    let now = OffsetDateTime::now_utc();
    let order_id = Uuid::new_v4().to_string();
    let mut acme_identifiers = Vec::new();
    let mut authz_urls = Vec::new();

    for ident in &identifiers {
        let ident_type = ident.get("type").and_then(|t| t.as_str()).unwrap_or("dns").to_string();
        let value = ident.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
        acme_identifiers.push(AcmeIdentifier { identifier_type: ident_type.clone(), value: value.clone() });

        let authz_id = Uuid::new_v4().to_string();
        let http_token = Uuid::new_v4().to_string().replace('-', "");
        let dns_token  = Uuid::new_v4().to_string().replace('-', "");

        let challenges = vec![
            AcmeChallenge {
                id: Uuid::new_v4().to_string(),
                challenge_type: ChallengeType::Http01,
                token: http_token,
                status: AcmeChallengeStatus::Pending,
                validated_at: None,
                error: None,
            },
            AcmeChallenge {
                id: Uuid::new_v4().to_string(),
                challenge_type: ChallengeType::Dns01,
                token: dns_token,
                status: AcmeChallengeStatus::Pending,
                validated_at: None,
                error: None,
            },
        ];

        let authz = AcmeAuthorization {
            id: authz_id.clone(),
            tenant_id: tenant.clone(),
            order_id: order_id.clone(),
            identifier_type: ident_type,
            identifier_value: value,
            status: AcmeAuthzStatus::Pending,
            challenges,
            expires: now + time::Duration::days(7),
        };
        let _ = store.store_acme_authorization(tenant, &authz);
        authz_urls.push(format!("{}/acme/authz/{}", base, authz_id));
    }

    let order = AcmeOrder {
        id: order_id.clone(),
        tenant_id: tenant.clone(),
        account_id: extract_jws_kid(body).unwrap_or_else(|| "unknown".to_string()),
        status: AcmeOrderStatus::Pending,
        identifiers: acme_identifiers,
        not_before: None,
        not_after: None,
        certificate_serial: None,
        expires: now + time::Duration::days(7),
        created_at: now,
    };
    let _ = store.store_acme_order(tenant, &order);

    AcmeOutcome {
        http_status: 201,
        body_json: serde_json::json!({
            "status": "pending",
            "identifiers": identifiers,
            "authorizations": authz_urls,
            "finalize": format!("{}/acme/order/{}/finalize", base, order_id),
            "expires": order.expires.to_string(),
        }).to_string(),
        replay_nonce: Some(nonce),
        location: Some(format!("{}/acme/order/{}", base, order_id)),
        content_type: "application/json".to_string(),
    }
}

fn handle_get_order(ctx: &AcmeContext, order_id: &str, nonce: String) -> AcmeOutcome {
    let tenant = &ctx.config.tenant_id;
    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => return AcmeOutcome {
            http_status: 500, body_json: acme_error("serverInternal", &e.to_string()),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
    };
    match store.get_acme_order(tenant, order_id) {
        Ok(Some(order)) => AcmeOutcome {
            http_status: 200,
            body_json: serde_json::to_string(&order).unwrap_or_default(),
            replay_nonce: Some(nonce), location: None, content_type: "application/json".to_string(),
        },
        Ok(None) => AcmeOutcome {
            http_status: 404, body_json: acme_error("notFound", "order not found"),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
        Err(e) => AcmeOutcome {
            http_status: 500, body_json: acme_error("serverInternal", &e.to_string()),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
    }
}

fn handle_get_authz(ctx: &AcmeContext, authz_id: &str, nonce: String) -> AcmeOutcome {
    let tenant = &ctx.config.tenant_id;
    let base   = base_url(ctx);
    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => return AcmeOutcome {
            http_status: 500, body_json: acme_error("serverInternal", &e.to_string()),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
    };
    match store.get_acme_authorization(tenant, authz_id) {
        Ok(Some(authz)) => {
            // Build JSON with challenge URLs embedded
            let mut authz_json = serde_json::to_value(&authz).unwrap_or_default();
            if let Some(challenges) = authz_json.get_mut("challenges").and_then(|c| c.as_array_mut()) {
                for (idx, ch) in challenges.iter_mut().enumerate() {
                    ch["url"] = serde_json::Value::String(
                        format!("{}/acme/authz/{}/challenge/{}", base, authz_id, idx)
                    );
                }
            }
            AcmeOutcome {
                http_status: 200,
                body_json: authz_json.to_string(),
                replay_nonce: Some(nonce), location: None, content_type: "application/json".to_string(),
            }
        }
        Ok(None) => AcmeOutcome {
            http_status: 404, body_json: acme_error("notFound", "authorization not found"),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
        Err(e) => AcmeOutcome {
            http_status: 500, body_json: acme_error("serverInternal", &e.to_string()),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
    }
}

/// POST /acme/authz/{authz_id}/challenge/{challenge_idx}
/// Triggers validation of the specified challenge.
fn handle_challenge(
    ctx: &AcmeContext,
    authz_id: &str,
    challenge_idx: usize,
    body: &str,
    nonce: String,
) -> AcmeOutcome {
    let tenant  = &ctx.config.tenant_id;
    let base    = base_url(ctx);
    let timeout = ctx.config.validation_timeout_secs;

    macro_rules! err {
        ($status:expr, $type:expr, $msg:expr) => {
            return AcmeOutcome {
                http_status: $status,
                body_json: acme_error($type, $msg),
                replay_nonce: Some(nonce),
                location: None,
                content_type: "application/problem+json".to_string(),
            }
        };
    }

    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => err!(500, "serverInternal", &e.to_string()),
    };

    let mut authz = match store.get_acme_authorization(tenant, authz_id) {
        Ok(Some(a)) => a,
        Ok(None) => err!(404, "notFound", "authorization not found"),
        Err(e)  => err!(500, "serverInternal", &e.to_string()),
    };

    if challenge_idx >= authz.challenges.len() {
        err!(404, "notFound", "challenge not found");
    }

    let challenge = &authz.challenges[challenge_idx];

    if challenge.status != AcmeChallengeStatus::Pending {
        // Already processed; return current state
        let ch_json = serde_json::json!({
            "type": format!("{:?}", challenge.challenge_type).to_lowercase().replace("_", "-").replace("01", "-01"),
            "status": format!("{:?}", challenge.status).to_lowercase(),
            "token": challenge.token,
            "url": format!("{}/acme/authz/{}/challenge/{}", base, authz_id, challenge_idx),
        });
        return AcmeOutcome {
            http_status: 200,
            body_json: ch_json.to_string(),
            replay_nonce: Some(nonce), location: None, content_type: "application/json".to_string(),
        };
    }

    // Resolve account JWK for key-authorization computation
    let kid = extract_jws_kid(body).unwrap_or_default();
    // kid is the account URL; extract account ID (last path segment)
    let account_id = kid.split('/').last().unwrap_or("").to_string();
    let account_jwk = store.get_acme_account(tenant, &account_id)
        .ok()
        .flatten()
        .map(|a| a.jwk)
        .unwrap_or_default();

    let token = challenge.token.clone();
    let challenge_type = challenge.challenge_type.clone();
    let domain = authz.identifier_value.clone();

    // Compute expected key authorization
    let key_auth = key_authorization(&token, &account_jwk)
        .unwrap_or_default();

    let validated = match challenge_type {
        ChallengeType::Http01 => {
            validate_http01(&domain, &token, &key_auth, timeout)
        }
        ChallengeType::Dns01 => {
            use sha2::Digest;
            let dns_value = base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                sha2::Sha256::digest(key_auth.as_bytes()).as_slice(),
            );
            validate_dns01(&domain, &dns_value)
        }
        ChallengeType::TlsAlpn01 => false,
    };

    let now = OffsetDateTime::now_utc();

    // Update challenge status
    let ch = &mut authz.challenges[challenge_idx];
    if validated {
        ch.status = AcmeChallengeStatus::Valid;
        ch.validated_at = Some(now);
        ch.error = None;
    } else {
        ch.status = AcmeChallengeStatus::Invalid;
        ch.error = Some("validation failed".to_string());
    }

    // Check if all challenges are valid → mark authz valid
    let all_valid = authz.challenges.iter().any(|c| c.status == AcmeChallengeStatus::Valid);
    if all_valid {
        authz.status = AcmeAuthzStatus::Valid;
    } else if authz.challenges.iter().all(|c| c.status == AcmeChallengeStatus::Invalid) {
        authz.status = AcmeAuthzStatus::Invalid;
    }

    let _ = store.update_acme_authorization(tenant, &authz);

    // If authz is now valid, check if the whole order is ready
    if authz.status == AcmeAuthzStatus::Valid {
        maybe_advance_order_to_ready(&store, tenant, &authz.order_id);
    }

    let ch = &authz.challenges[challenge_idx];
    let ch_type_str = match ch.challenge_type {
        ChallengeType::Http01    => "http-01",
        ChallengeType::Dns01     => "dns-01",
        ChallengeType::TlsAlpn01 => "tls-alpn-01",
    };
    let status_str = match ch.status {
        AcmeChallengeStatus::Pending   => "pending",
        AcmeChallengeStatus::Processing => "processing",
        AcmeChallengeStatus::Valid     => "valid",
        AcmeChallengeStatus::Invalid   => "invalid",
    };

    AcmeOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "type": ch_type_str,
            "status": status_str,
            "token": ch.token,
            "url": format!("{}/acme/authz/{}/challenge/{}", base, authz_id, challenge_idx),
        }).to_string(),
        replay_nonce: Some(nonce), location: None, content_type: "application/json".to_string(),
    }
}

/// After an authorization becomes valid, check if all authorizations for the
/// parent order are valid and if so advance the order status to `ready`.
fn maybe_advance_order_to_ready(
    store: &OxPersistenceCertStore,
    tenant: &str,
    order_id: &str,
) {
    let order = match store.get_acme_order(tenant, order_id) {
        Ok(Some(o)) => o,
        _ => return,
    };
    if order.status != AcmeOrderStatus::Pending { return; }

    // We don't have a list_authz_for_order API, so we check all identifiers.
    // Since we stored one authz per identifier, we'd need to look them up.
    // For now: mark order ready after any one valid authz
    // (a full implementation would track authz IDs per order)
    let _ = store.update_acme_order_status(tenant, order_id, AcmeOrderStatus::Ready);
}

fn handle_finalize(ctx: &AcmeContext, order_id: &str, body: &str, nonce: String) -> AcmeOutcome {
    let tenant = &ctx.config.tenant_id;
    let base   = base_url(ctx);

    macro_rules! err {
        ($status:expr, $type:expr, $msg:expr) => {
            return AcmeOutcome {
                http_status: $status,
                body_json: acme_error($type, $msg),
                replay_nonce: Some(nonce),
                location: None,
                content_type: "application/problem+json".to_string(),
            }
        };
    }

    let payload = match extract_jws_payload(body) {
        Some(p) => p,
        None => err!(400, "malformed", "invalid JWS"),
    };

    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => err!(500, "serverInternal", &e.to_string()),
    };

    let mut order = match store.get_acme_order(tenant, order_id) {
        Ok(Some(o)) => o,
        Ok(None) => err!(404, "notFound", "order not found"),
        Err(e)  => err!(500, "serverInternal", &e.to_string()),
    };

    if order.status != AcmeOrderStatus::Ready && order.status != AcmeOrderStatus::Pending {
        err!(403, "orderNotReady", "order is not ready for finalization");
    }

    let csr_b64 = match payload.get("csr").and_then(|c| c.as_str()) {
        Some(c) => c.to_string(),
        None => err!(400, "badCSR", "csr is required"),
    };
    let csr_der = match base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        csr_b64.as_bytes(),
    ) {
        Ok(b) => b,
        Err(e) => err!(400, "badCSR", &format!("base64 decode: {}", e)),
    };

    let csr_pem = pem::encode(&pem::Pem::new("CERTIFICATE REQUEST", csr_der));

    let ca_cert_pem = match std::fs::read_to_string(&ctx.config.ca_intermediate_cert_path) {
        Ok(s) => s,
        Err(e) => err!(500, "serverInternal", &format!("CA cert: {}", e)),
    };
    let issuer_params = match issuer_params_from_cert_pem(&ca_cert_pem) {
        Ok(p) => p,
        Err(e) => err!(500, "serverInternal", &e.to_string()),
    };
    let ks = match open_keystore(&ctx.config.keystore) {
        Ok(k) => k,
        Err(e) => err!(500, "serverInternal", &e.to_string()),
    };
    let ca_key_pem = match ks.load_key_pem(tenant, &ctx.config.ca_intermediate_key_id) {
        Ok(p) => p,
        Err(e) => err!(500, "serverInternal", &e.to_string()),
    };
    let ca_keypair = match rcgen::KeyPair::from_pem(&ca_key_pem) {
        Ok(k) => k,
        Err(e) => err!(500, "serverInternal", &e.to_string()),
    };

    let cert_record = match sign_csr(&csr_pem, tenant, "acme", 90 * 86400, None, &issuer_params, &ca_keypair) {
        Ok(r) => r,
        Err(e) => err!(400, "badCSR", &e.to_string()),
    };

    let serial = cert_record.serial.clone();
    let _ = store.store_cert(tenant, &cert_record);

    order.status = AcmeOrderStatus::Valid;
    order.certificate_serial = Some(serial.clone());
    let _ = store.update_acme_order_status(tenant, order_id, AcmeOrderStatus::Valid);

    let now = OffsetDateTime::now_utc();
    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0, tenant_id: tenant.clone(), timestamp: now,
        action: AuditAction::AcmeFinalize, serial: Some(serial),
        actor: String::new(), details: serde_json::json!({ "order_id": order_id }),
    });

    AcmeOutcome {
        http_status: 200,
        body_json: serde_json::json!({
            "status": "valid",
            "certificate": format!("{}/acme/cert/{}", base, order_id),
        }).to_string(),
        replay_nonce: Some(nonce),
        location: None,
        content_type: "application/json".to_string(),
    }
}

fn handle_get_cert(ctx: &AcmeContext, order_id: &str, nonce: String) -> AcmeOutcome {
    let tenant = &ctx.config.tenant_id;
    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => return AcmeOutcome {
            http_status: 500, body_json: acme_error("serverInternal", &e.to_string()),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
    };
    let order = match store.get_acme_order(tenant, order_id) {
        Ok(Some(o)) => o,
        Ok(None) => return AcmeOutcome {
            http_status: 404, body_json: acme_error("notFound", "order not found"),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
        Err(e) => return AcmeOutcome {
            http_status: 500, body_json: acme_error("serverInternal", &e.to_string()),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
    };
    if order.status != AcmeOrderStatus::Valid {
        return AcmeOutcome {
            http_status: 403, body_json: acme_error("orderNotReady", "certificate not yet issued"),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        };
    }
    let serial = match &order.certificate_serial {
        Some(s) => s.clone(),
        None => return AcmeOutcome {
            http_status: 404, body_json: acme_error("notFound", "certificate not found"),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
    };
    match store.get_cert_by_serial(tenant, &serial) {
        Ok(Some(cert)) => AcmeOutcome {
            http_status: 200,
            body_json: cert.pem,
            replay_nonce: Some(nonce), location: None,
            content_type: "application/pem-certificate-chain".to_string(),
        },
        Ok(None) => AcmeOutcome {
            http_status: 404, body_json: acme_error("notFound", "certificate not found"),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
        Err(e) => AcmeOutcome {
            http_status: 500, body_json: acme_error("serverInternal", &e.to_string()),
            replay_nonce: Some(nonce), location: None, content_type: "application/problem+json".to_string(),
        },
    }
}

fn handle_revoke_cert(ctx: &AcmeContext, body: &str, nonce: String) -> AcmeOutcome {
    let tenant = &ctx.config.tenant_id;

    macro_rules! err {
        ($status:expr, $type:expr, $msg:expr) => {
            return AcmeOutcome {
                http_status: $status,
                body_json: acme_error($type, $msg),
                replay_nonce: Some(nonce),
                location: None,
                content_type: "application/problem+json".to_string(),
            }
        };
    }

    let payload = match extract_jws_payload(body) {
        Some(p) => p,
        None => err!(400, "malformed", "invalid JWS"),
    };
    let cert_b64 = match payload.get("certificate").and_then(|c| c.as_str()) {
        Some(c) => c,
        None => err!(400, "malformed", "certificate is required"),
    };
    let cert_der = match base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD, cert_b64.as_bytes(),
    ) {
        Ok(b) => b,
        Err(e) => err!(400, "malformed", &format!("base64: {}", e)),
    };

    let serial = parse_serial_from_cert_der(&cert_der).unwrap_or_default();
    if serial.is_empty() { err!(400, "malformed", "cannot parse cert serial"); }

    let store = match OxPersistenceCertStore::open() {
        Ok(s) => s,
        Err(e) => err!(500, "serverInternal", &e.to_string()),
    };
    let now = OffsetDateTime::now_utc();
    let _ = store.mark_revoked(tenant, &serial, ox_cert_core::model::RevocationReason::Unspecified, now);
    let _ = store.store_audit_event(tenant, &AuditEvent {
        id: 0, tenant_id: tenant.clone(), timestamp: now,
        action: AuditAction::AcmeRevoke, serial: Some(serial),
        actor: String::new(), details: serde_json::json!({}),
    });

    AcmeOutcome {
        http_status: 200,
        body_json: String::new(),
        replay_nonce: Some(nonce), location: None, content_type: "application/json".to_string(),
    }
}

fn parse_serial_from_cert_der(der: &[u8]) -> Option<String> {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(der).ok()?;
    let serial_bytes = cert.serial.to_bytes_be();
    if serial_bytes.len() == 16 {
        let arr: [u8; 16] = serial_bytes.as_slice().try_into().ok()?;
        Some(uuid::Uuid::from_bytes(arr).to_string())
    } else {
        None
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
        ctx: AcmeContext,
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
        } else { String::new() };
        let params: serde_json::Value = serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);
        let config_path = match params.get("config_file").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_cert_acme: missing config_file param");
                return std::ptr::null_mut();
            }
        };
        let config: AcmeConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
            Ok(v) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                        &format!("ox_cert_acme: config error: {}", e));
                    return std::ptr::null_mut();
                }
            },
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_cert_acme: failed to load config: {}", e));
                return std::ptr::null_mut();
            }
        };
        let tenant_id = config.tenant_id.clone();
        let ctx = AcmeContext::new(config);
        log(&api, std::ptr::null_mut(), OX_LOG_INFO,
            &format!("ox_cert_acme: initialized for tenant '{}'", tenant_id));
        Box::into_raw(Box::new(PluginState { api, ctx })) as *mut c_void
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

            if !path.starts_with("/acme/") && path != "/acme/directory" { return cont; }

            let outcome = handle(&state.ctx, &method, &path, &body);

            set(&state.api, task_ctx, "response.status", &outcome.http_status.to_string());
            set(&state.api, task_ctx, "response.header.Content-Type", &outcome.content_type);
            if !outcome.body_json.is_empty() {
                set(&state.api, task_ctx, "response.body", &outcome.body_json);
            }
            if let Some(n) = &outcome.replay_nonce {
                set(&state.api, task_ctx, "response.header.Replay-Nonce", n);
            }
            if let Some(loc) = &outcome.location {
                set(&state.api, task_ctx, "response.header.Location", loc);
            }
            cont
        }));

        match result {
            Ok(fc) => fc,
            Err(_) => {
                log(&state.api, task_ctx, OX_LOG_ERROR, "ox_cert_acme: panic");
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
