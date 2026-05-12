use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use futures::future::BoxFuture;
use async_trait::async_trait;
use tokio::sync::Mutex;
use jsonwebtoken::jwk::JwkSet;
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, Principal, PrincipalId,
    AuthSource, TenantId, GroupId, drivers::AuthDriver,
};

// ── Public types ─────────────────────────────────────────────────────────────

/// Returns the full JWKS JSON as a `String`, or an error message.
///
/// Injected at construction time; in production, use a `reqwest`-backed closure.
/// In tests, return a static fixture string.
///
/// Example production wiring (in the binary crate):
/// ```rust,ignore
/// let jwks_url = config.jwks_url.clone();
/// let fetch: JwksFetchFn = Arc::new(move || {
///     let url = jwks_url.clone();
///     Box::pin(async move {
///         reqwest::get(&url)
///             .await
///             .map_err(|e| format!("JWKS HTTP error: {}", e))?
///             .text()
///             .await
///             .map_err(|e| format!("JWKS body read error: {}", e))
///     })
/// });
/// ```
pub type JwksFetchFn = Arc<dyn Fn() -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

/// OIDC provider configuration.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// Issuer URL — must match the `iss` claim in the JWT.
    /// Example: `"https://dev-123.okta.com/oauth2/default"`
    pub issuer_url: String,
    /// Expected `aud` claim value.
    /// Example: `"api://my-service"` (Entra ID) or `"0oa123.okta.com"` (Okta)
    pub audience: String,
    /// JWKS endpoint URL.
    /// Example: `"https://dev-123.okta.com/oauth2/default/v1/keys"`
    pub jwks_url: String,
    /// Tenant identifier stamped on every `Principal` produced by this driver.
    pub tenant_id: TenantId,
}

// ── JWKS cache ────────────────────────────────────────────────────────────────

struct CachedJwks {
    jwks: JwkSet,
    fetched_at: Instant,
}

// ── Driver ────────────────────────────────────────────────────────────────────

/// Authentication driver that validates OIDC/JWT bearer tokens.
///
/// Accepts only `Credentials::BearerToken` — passes `Continue` for all other variants.
/// Fetches the JWKS from the injected `JwksFetchFn`, caches it in-process with a
/// configurable TTL, and verifies asymmetric-key JWT signatures (RS256, RS384, RS512,
/// ES256, ES384). Validates `iss`, `aud`, and `exp` claims before returning a `Principal`.
///
/// Infrastructure errors (JWKS fetch failure, JSON parse error) → `Continue` so
/// the pipeline can try another driver. Security failures (expired token, wrong
/// audience, unknown kid) → `Reject`.
pub struct OidcAuthDriver {
    config: OidcConfig,
    fetch_jwks: JwksFetchFn,
    cache: Arc<Mutex<Option<CachedJwks>>>,
    /// Maximum age of a cached JWKS before it is refreshed.
    cache_ttl: Duration,
}

impl OidcAuthDriver {
    /// Construct a driver with default JWKS cache TTL (5 minutes).
    pub fn new(config: OidcConfig, fetch_jwks: JwksFetchFn) -> Self {
        Self::with_ttl(config, fetch_jwks, Duration::from_secs(300))
    }

    /// Construct a driver with an explicit JWKS cache TTL.
    pub fn with_ttl(config: OidcConfig, fetch_jwks: JwksFetchFn, cache_ttl: Duration) -> Self {
        Self {
            config,
            fetch_jwks,
            cache: Arc::new(Mutex::new(None)),
            cache_ttl,
        }
    }

    /// Fetch the JWKS, using the in-process cache when still fresh.
    ///
    /// Returns `Err(String)` on infrastructure failure (fetch or parse error).
    async fn get_jwks(&self) -> Result<JwkSet, String> {
        let mut guard = self.cache.lock().await;
        if let Some(ref cached) = *guard {
            if cached.fetched_at.elapsed() < self.cache_ttl {
                // Clone the JwkSet from cache. JwkSet: Clone
                return Ok(cached.jwks.clone());
            }
        }
        // Cache miss or expired — fetch fresh
        let raw = (self.fetch_jwks)().await?;
        let jwks: JwkSet = serde_json::from_str(&raw)
            .map_err(|e| format!("JWKS parse error: {}", e))?;
        let result = jwks.clone();
        *guard = Some(CachedJwks { jwks, fetched_at: Instant::now() });
        Ok(result)
    }

    /// Find the JWK matching the JWT header's `kid`.
    ///
    /// If the JWT has no `kid` and the JWKS contains exactly one key, that key is used.
    fn find_key<'a>(jwks: &'a JwkSet, kid: &Option<String>) -> Result<&'a jsonwebtoken::jwk::Jwk, String> {
        match kid {
            None => {
                if jwks.keys.len() == 1 {
                    Ok(&jwks.keys[0])
                } else {
                    Err("JWT has no kid and JWKS contains multiple keys — cannot determine which key to use".to_string())
                }
            }
            Some(k) => jwks
                .find(k)
                .ok_or_else(|| format!("no JWK found for kid={}", k)),
        }
    }
}

#[async_trait]
impl AuthDriver for OidcAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let token = match credentials {
            Credentials::BearerToken { token } => token.as_str(),
            _ => return AuthResult::Continue,
        };

        // 1. Decode the JWT header to get `kid` and `alg` — no signature verification yet
        let header = match jsonwebtoken::decode_header(token) {
            Ok(h) => h,
            Err(e) => return AuthResult::Reject(format!("JWT header decode error: {}", e)),
        };

        // 2. Fetch JWKS (cached); infrastructure failure → Continue
        let jwks = match self.get_jwks().await {
            Ok(k) => k,
            Err(_err) => {
                // Infrastructure error — return Continue per codebase convention.
                // Log the error string so operators can diagnose JWKS outages.
                let _ = _err; // Replace with tracing::warn!() when tracing is added
                return AuthResult::Continue;
            }
        };

        // 3. Find matching key by kid
        let jwk = match Self::find_key(&jwks, &header.kid) {
            Ok(k) => k,
            Err(e) => return AuthResult::Reject(e),
        };

        // 4. Build decoding key from JWK
        let decoding_key = match jsonwebtoken::DecodingKey::from_jwk(jwk) {
            Ok(k) => k,
            // Infrastructure/config error — treat as Continue
            Err(_err) => return AuthResult::Continue,
        };

        // 5. Determine algorithm — prefer JWK's alg, fall back to JWT header
        // KeyAlgorithm::to_algorithm is private in jsonwebtoken 9; round-trip via Display + from_str.
        let algorithm = if let Some(key_alg) = jwk.common.key_algorithm {
            let alg_str = key_alg.to_string();
            match jsonwebtoken::Algorithm::from_str(&alg_str) {
                Ok(a) => a,
                Err(_) => return AuthResult::Reject(format!("unsupported JWK algorithm: {}", alg_str)),
            }
        } else {
            header.alg
        };

        // 5b. Enforce explicit algorithm allowlist — reject anything outside the
        // approved asymmetric-key set, even if the JWK or header specifies it.
        match algorithm {
            jsonwebtoken::Algorithm::RS256
            | jsonwebtoken::Algorithm::RS384
            | jsonwebtoken::Algorithm::RS512
            | jsonwebtoken::Algorithm::ES256
            | jsonwebtoken::Algorithm::ES384 => {}
            other => {
                return AuthResult::Reject(format!("algorithm {:?} is not permitted", other));
            }
        }

        // 6. Verify signature + claims (iss, aud, exp)
        let mut validation = jsonwebtoken::Validation::new(algorithm);
        // Require iss, aud, and exp to be present in the token (not just checked when present).
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        validation.set_issuer(&[&self.config.issuer_url]);
        validation.set_audience(&[&self.config.audience]);

        let token_data = match jsonwebtoken::decode::<serde_json::Value>(
            token,
            &decoding_key,
            &validation,
        ) {
            Ok(td) => td,
            Err(e) => {
                let msg = match e.kind() {
                    jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                        "token is expired".to_string()
                    }
                    jsonwebtoken::errors::ErrorKind::InvalidAudience => {
                        "invalid audience claim".to_string()
                    }
                    jsonwebtoken::errors::ErrorKind::InvalidIssuer => {
                        "invalid issuer claim".to_string()
                    }
                    _ => format!("token validation failed: {}", e),
                };
                return AuthResult::Reject(msg);
            }
        };

        // 7. Extract claims from the validated token
        let claims = token_data.claims;

        let sub = match claims.get("sub").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return AuthResult::Reject("JWT missing 'sub' claim".to_string()),
        };

        let display_name = claims
            .get("preferred_username")
            .or_else(|| claims.get("email"))
            .and_then(|v| v.as_str())
            .unwrap_or(sub.as_str())
            .to_string();

        // Collect groups from `groups` and `roles` claims
        let groups: Vec<GroupId> = {
            let mut g = Vec::new();
            for field in &["groups", "roles"] {
                if let Some(arr) = claims.get(*field).and_then(|v| v.as_array()) {
                    for item in arr {
                        if let Some(name) = item.as_str() {
                            g.push(GroupId::new(name));
                        }
                    }
                }
            }
            g
        };

        // 8. Build Principal
        let principal = Principal {
            id: PrincipalId::new(),
            display_name,
            source: AuthSource::Oidc,
            groups,
            tenant_id: self.config.tenant_id.clone(),
            session_id: None,
        };

        AuthResult::Authenticated(principal)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::time::{SystemTime, UNIX_EPOCH};
    use base64ct::{Base64UrlUnpadded, Encoding};
    use futures::future::BoxFuture;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, TenantId};
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::traits::PublicKeyParts;
    use rsa::{RsaPrivateKey, RsaPublicKey};
    use serde::Serialize;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_config() -> OidcConfig {
        OidcConfig {
            issuer_url: "https://idp.example.com".to_string(),
            audience: "my-api".to_string(),
            jwks_url: "https://idp.example.com/.well-known/jwks.json".to_string(),
            tenant_id: TenantId::from_str("test").unwrap(),
        }
    }

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    /// Returns a `JwksFetchFn` that immediately yields the provided JWKS JSON string.
    fn static_jwks_fn(json: String) -> JwksFetchFn {
        Arc::new(move || {
            let json = json.clone();
            Box::pin(async move { Ok(json) }) as BoxFuture<'static, Result<String, String>>
        })
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn generate_test_key_pair() -> (RsaPrivateKey, RsaPublicKey) {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("key generation failed");
        let public_key = RsaPublicKey::from(&private_key);
        (private_key, public_key)
    }

    fn jwks_fixture(public_key: &RsaPublicKey, kid: &str) -> String {
        let n = Base64UrlUnpadded::encode_string(&public_key.n().to_bytes_be());
        let e = Base64UrlUnpadded::encode_string(&public_key.e().to_bytes_be());
        serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "kid": kid,
                "alg": "RS256",
                "use": "sig",
                "n": n,
                "e": e
            }]
        })
        .to_string()
    }

    #[derive(Serialize)]
    struct TestClaims {
        sub: String,
        iss: String,
        aud: String,
        exp: u64,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        groups: Vec<String>,
    }

    fn mint_token(private_key: &RsaPrivateKey, kid: &str, claims: &TestClaims) -> String {
        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .unwrap();
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        encode(&header, claims, &encoding_key).unwrap()
    }

    // ── Test 1: unsupported credential type ───────────────────────────────────

    #[tokio::test]
    async fn oidc_continues_for_non_bearer_creds() {
        let driver = OidcAuthDriver::new(
            test_config(),
            static_jwks_fn("{}".to_string()),
        );
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: secrecy::SecretString::new("secret".to_string()),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    // ── Test 2: valid JWT ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn oidc_authenticates_valid_jwt() {
        let (private_key, public_key) = generate_test_key_pair();
        let kid = "test-kid-1";
        let jwks_json = jwks_fixture(&public_key, kid);

        let driver = OidcAuthDriver::new(test_config(), static_jwks_fn(jwks_json));

        let claims = TestClaims {
            sub: "user-42".to_string(),
            iss: "https://idp.example.com".to_string(),
            aud: "my-api".to_string(),
            exp: now_secs() + 300,
            groups: vec!["admin".to_string()],
        };
        let token = mint_token(&private_key, kid, &claims);

        let creds = Credentials::BearerToken { token };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => {
                assert_eq!(p.source, AuthSource::Oidc);
                // sub is used as display_name when preferred_username absent
                assert_eq!(p.display_name, "user-42");
                assert_eq!(p.tenant_id.as_str(), "test");
            }
            _ => panic!("expected Authenticated, got something else"),
        }
    }

    // ── Test 3: expired token ─────────────────────────────────────────────────

    #[tokio::test]
    async fn oidc_rejects_expired_token() {
        let (private_key, public_key) = generate_test_key_pair();
        let kid = "test-kid-exp";
        let jwks_json = jwks_fixture(&public_key, kid);

        let driver = OidcAuthDriver::new(test_config(), static_jwks_fn(jwks_json));

        let claims = TestClaims {
            sub: "user-99".to_string(),
            iss: "https://idp.example.com".to_string(),
            aud: "my-api".to_string(),
            exp: now_secs() - 300, // expired 300 seconds ago (past 60s leeway)
            groups: vec![],
        };
        let token = mint_token(&private_key, kid, &claims);

        let creds = Credentials::BearerToken { token };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Reject(msg) => assert!(
                msg.contains("expired"),
                "expected 'expired' in rejection message, got: {}",
                msg
            ),
            _ => panic!("expected Reject, got something else"),
        }
    }

    // ── Test 4: wrong audience ────────────────────────────────────────────────

    #[tokio::test]
    async fn oidc_rejects_wrong_audience() {
        let (private_key, public_key) = generate_test_key_pair();
        let kid = "test-kid-aud";
        let jwks_json = jwks_fixture(&public_key, kid);

        let driver = OidcAuthDriver::new(test_config(), static_jwks_fn(jwks_json));

        let claims = TestClaims {
            sub: "user-88".to_string(),
            iss: "https://idp.example.com".to_string(),
            aud: "wrong-api".to_string(), // not "my-api"
            exp: now_secs() + 300,
            groups: vec![],
        };
        let token = mint_token(&private_key, kid, &claims);

        let creds = Credentials::BearerToken { token };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Reject(msg) => assert!(
                msg.contains("audience"),
                "expected 'audience' in rejection message, got: {}",
                msg
            ),
            _ => panic!("expected Reject, got something else"),
        }
    }

    // ── Test 5: wrong issuer ──────────────────────────────────────────────────

    #[tokio::test]
    async fn oidc_rejects_wrong_issuer() {
        let (private_key, public_key) = generate_test_key_pair();
        let kid = "test-kid-iss";
        let jwks_json = jwks_fixture(&public_key, kid);

        let driver = OidcAuthDriver::new(test_config(), static_jwks_fn(jwks_json));

        let claims = TestClaims {
            sub: "user-55".to_string(),
            iss: "https://evil.example.com".to_string(), // wrong issuer
            aud: "my-api".to_string(),
            exp: now_secs() + 300,
            groups: vec![],
        };
        let token = mint_token(&private_key, kid, &claims);

        let creds = Credentials::BearerToken { token };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Reject(_)));
    }

    // ── Test 6: JWKS fetch failure → Continue ────────────────────────────────

    #[tokio::test]
    async fn oidc_continues_on_jwks_fetch_failure() {
        // Mint a real (well-formed) token so decode_header succeeds.
        // The JWKS fetch will fail before any signature check.
        let (private_key, public_key) = generate_test_key_pair();
        let kid = "test-kid-fetch-fail";
        // Use a JWKS fn that succeeds once so we can mint, then swap to failing fn.
        // Simpler: just mint a token — the fetch_fn below always fails, so we
        // never get past step 2; we only need decode_header to succeed.
        let _ = public_key; // key used only to satisfy generate_test_key_pair signature
        let claims = TestClaims {
            sub: "user-net-fail".to_string(),
            iss: "https://idp.example.com".to_string(),
            aud: "my-api".to_string(),
            exp: now_secs() + 3600,
            groups: vec![],
        };
        let token = mint_token(&private_key, kid, &claims);

        let fetch_fn: JwksFetchFn = Arc::new(|| {
            Box::pin(async { Err("simulated network failure".to_string()) })
        });
        let driver = OidcAuthDriver::new(test_config(), fetch_fn);
        let creds = Credentials::BearerToken { token };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    // ── Test 7: unknown kid ───────────────────────────────────────────────────

    #[tokio::test]
    async fn oidc_rejects_unknown_kid() {
        let (private_key, public_key) = generate_test_key_pair();
        // JWKS has "real-kid-1", but the JWT will use "nonexistent-kid"
        let jwks_json = jwks_fixture(&public_key, "real-kid-1");

        let driver = OidcAuthDriver::new(test_config(), static_jwks_fn(jwks_json));

        let claims = TestClaims {
            sub: "user-77".to_string(),
            iss: "https://idp.example.com".to_string(),
            aud: "my-api".to_string(),
            exp: now_secs() + 300,
            groups: vec![],
        };
        let token = mint_token(&private_key, "nonexistent-kid", &claims);

        let creds = Credentials::BearerToken { token };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Reject(msg) => assert!(
                msg.contains("kid"),
                "expected 'kid' in rejection message, got: {}",
                msg
            ),
            _ => panic!("expected Reject, got something else"),
        }
    }
}
