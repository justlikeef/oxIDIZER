# OidcAuthDriver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `OidcAuthDriver` in `ox_security_auth` — a production-quality OIDC/JWT bearer token driver that validates RS256/ES256 signed access tokens from Okta, Entra ID (Azure AD), Google Workspace, and any RFC 7517-compliant JWKS endpoint. The driver caches the JWKS in-process, decodes the JWT header to find the matching key by `kid`, verifies the signature, and validates `iss`, `aud`, and `exp` claims before returning a `Principal`.

**Architecture:** `OidcAuthDriver` follows the same injector pattern as `DbAuthDriver` and `ApiKeyAuthDriver` — a `JwksFetchFn` is injected at construction time so tests never hit a real network. In production the caller supplies a `reqwest`-backed closure. JWKS is cached in `Arc<Mutex<Option<CachedJwks>>>` with a configurable TTL. Token validation uses the `jsonwebtoken` crate. `serde_json` parses the JWKS payload. `futures` provides `BoxFuture` for the async fetch fn type alias.

**Tech Stack:**
- `jsonwebtoken = "9"` — JWT decode, verification, algorithm support (RS256, ES256)
- `reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }` — HTTPS JWKS fetch (production-only; injected in tests)
- `serde = { version = "1", features = ["derive"] }` — JWKS JSON deserialization
- `serde_json = "1"` — raw JSON parsing
- `futures = "0.3"` — `BoxFuture` for the fetch fn type alias
- `tokio = { version = "1", features = ["macros", "rt", "time", "sync"] }` — async runtime + `Mutex`

---

## File Structure (changes only)

```
crates/security/ox_security_auth/
  Cargo.toml                         — add jsonwebtoken, serde, serde_json, futures, reqwest
  src/
    lib.rs                           — add OidcAuthDriver to re-exports
    drivers/
      mod.rs                         — add oidc module + re-export
      oidc.rs                        — NEW file: OidcConfig, JwksFetchFn, OidcAuthDriver
```

---

## Task 1: Crate dependencies

**Files:**
- Modify: `crates/security/ox_security_auth/Cargo.toml`

### Step 1: Update `Cargo.toml`

Replace contents of `crates/security/ox_security_auth/Cargo.toml` with:

```toml
[package]
name = "ox_security_auth"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]
ox_security_core = { path = "../ox_security_core" }
async-trait      = "0.1"
secrecy          = { version = "0.8", features = ["serde"] }
jsonwebtoken     = "9"
serde            = { version = "1", features = ["derive"] }
serde_json       = "1"
futures          = "0.3"
tokio            = { version = "1", features = ["sync"] }

[dev-dependencies]
tokio     = { version = "1", features = ["macros", "rt", "time", "sync"] }
x509-cert = "0.2"
```

> Note: `reqwest` is intentionally omitted from `[dependencies]` here. Add it only when wiring up the production HTTP fetch closure (in the binary crate or an integration layer). The `OidcAuthDriver` itself is network-agnostic — all I/O is injected via `JwksFetchFn`.

### Step 2: Verify it resolves

```bash
cargo fetch --manifest-path crates/security/ox_security_auth/Cargo.toml 2>&1 | tail -5
```

Expected: no errors.

### Step 3: Commit

```bash
git add crates/security/ox_security_auth/Cargo.toml
git commit -m "chore(security-auth): add jsonwebtoken, serde, serde_json, futures deps for OidcAuthDriver"
```

---

## Task 2: OidcAuthDriver scaffold + wiring

**Files:**
- Create: `crates/security/ox_security_auth/src/drivers/oidc.rs` (stub)
- Modify: `crates/security/ox_security_auth/src/drivers/mod.rs`
- Modify: `crates/security/ox_security_auth/src/lib.rs`

### Step 1: Create `src/drivers/oidc.rs` stub

```rust
use std::sync::Arc;
use futures::future::BoxFuture;
use async_trait::async_trait;
use tokio::sync::Mutex;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, TenantId, drivers::AuthDriver};

/// Returns the full JWKS JSON as a `String`, or an error message.
/// Injected at construction time; in production, use a `reqwest`-backed closure.
/// In tests, return a static fixture string.
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
    /// JWKS endpoint URL. Example: `"https://dev-123.okta.com/oauth2/default/v1/keys"`
    pub jwks_url: String,
    /// Tenant identifier stamped on every `Principal` produced by this driver.
    pub tenant_id: TenantId,
}

struct CachedJwks {
    raw_json: String,
    fetched_at: std::time::Instant,
}

pub struct OidcAuthDriver {
    config: OidcConfig,
    fetch_jwks: JwksFetchFn,
    cache: Arc<Mutex<Option<CachedJwks>>>,
    /// Maximum age of a cached JWKS before it is refreshed.
    cache_ttl: std::time::Duration,
}

impl OidcAuthDriver {
    pub fn new(config: OidcConfig, fetch_jwks: JwksFetchFn) -> Self {
        Self::with_ttl(config, fetch_jwks, std::time::Duration::from_secs(300))
    }

    pub fn with_ttl(
        config: OidcConfig,
        fetch_jwks: JwksFetchFn,
        cache_ttl: std::time::Duration,
    ) -> Self {
        Self {
            config,
            fetch_jwks,
            cache: Arc::new(Mutex::new(None)),
            cache_ttl,
        }
    }
}

#[async_trait]
impl AuthDriver for OidcAuthDriver {
    async fn authenticate(
        &self,
        _credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        AuthResult::Continue
    }
}
```

### Step 2: Wire into `mod.rs`

Replace `crates/security/ox_security_auth/src/drivers/mod.rs` with:

```rust
pub(crate) mod ad;
pub(crate) mod api_key;
pub(crate) mod db;
pub(crate) mod kerberos;
pub(crate) mod ldap;
pub(crate) mod mtls;
pub(crate) mod oidc;
pub(crate) mod radius;
pub(crate) mod tacacs;
pub(crate) mod totp;

pub use ad::AdAuthDriver;
pub use api_key::ApiKeyAuthDriver;
pub use db::DbAuthDriver;
pub use kerberos::KerberosAuthDriver;
pub use ldap::LdapAuthDriver;
pub use mtls::MtlsAuthDriver;
pub use oidc::{OidcAuthDriver, OidcConfig, JwksFetchFn};
pub use radius::RadiusAuthDriver;
pub use tacacs::TacacsAuthDriver;
pub use totp::TotpAuthDriver;
```

### Step 3: Wire into `lib.rs`

```rust
pub(crate) mod drivers;
pub(crate) mod pipeline;

pub use pipeline::AuthPipeline;
pub use drivers::{
    AdAuthDriver, ApiKeyAuthDriver, DbAuthDriver,
    KerberosAuthDriver, LdapAuthDriver, MtlsAuthDriver,
    OidcAuthDriver, OidcConfig, JwksFetchFn,
    RadiusAuthDriver, TacacsAuthDriver, TotpAuthDriver,
};
```

### Step 4: Verify it compiles

```bash
cargo build -p ox_security_auth 2>&1 | grep "^error" | head -5
```

Expected: zero errors.

### Step 5: Commit

```bash
git add crates/security/ox_security_auth/
git commit -m "feat(security-auth): scaffold OidcAuthDriver with config, fetch-fn injector, JWKS cache"
```

---

## Task 3: Write the failing tests

**Files:**
- Modify: `crates/security/ox_security_auth/src/drivers/oidc.rs` — append `#[cfg(test)] mod tests { ... }`

The tests use `jsonwebtoken` directly to mint valid/invalid tokens and a static JWKS fixture. The RS256 key pair is generated at test-time using `jsonwebtoken`'s built-in test helpers.

### Step 1: Add the test module to `src/drivers/oidc.rs`

Append to the bottom of `oidc.rs` (after all impl blocks):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::time::{SystemTime, UNIX_EPOCH};
    use futures::future::BoxFuture;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, TenantId};
    use serde::{Deserialize, Serialize};
    use std::net::{IpAddr, Ipv4Addr};

    // ── Minimal RSA test key pair (2048-bit, PEM) ────────────────────────────
    // These keys are for testing only. Never use in production.
    const TEST_RSA_PRIVATE_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEA2a2rwplBQLzHPZe5TSd4XmT0QVNAQ8gJfzBzNSSlBqCIRBmG
4ZRuvQONvYCQgU+RLTL7YnmZGkN2OEBXFTiOQCPrLhRuvMNJpRsNrNqJrBp2mBdN
bBJQUXe6fBRkzuwGNbz6EBmUqsYXPqAo2dGa8Iv0LsFLGzLhT0P7FZ4XA4h5oJUL
cHRqizM1ePQwz7mzNvzZCxKz8E2gFqBh+rBQhWB7QbNUf+VEWI8G2B2KRWy0h+Oo
K1wMuWJ1iBL2T4XBQE4KJzj3LYFZd7TuG+vLcBcL4l8T5h1zJCr0gzlqLzqw1tH
2JYe9g/yB3Fz5cQQ2n/x7vJQZ2mB5QIDAQABAAKAI...
-----END RSA PRIVATE KEY-----";
    // NOTE: The above is intentionally truncated — replace with a real 2048-bit RSA key
    // generated with: openssl genrsa -out test_key.pem 2048
    // For tests that mint tokens, use jsonwebtoken::DecodingKey and EncodingKey from PEM.

    // ── Recommended approach: use jsonwebtoken's EncodingKey::from_rsa_pem ──
    // In tests, generate the key pair inline using the `rsa` crate (dev-dep) or
    // embed a known test key. The test structure below shows the pattern regardless
    // of which keygen approach you choose.

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

    /// Returns a JwksFetchFn that immediately yields the provided JWKS JSON string.
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

    // ── Test 1 ────────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn oidc_continues_for_non_bearer_creds() {
        let driver = OidcAuthDriver::new(
            test_config(),
            static_jwks_fn("{}".to_string()),
        );
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: secrecy::SecretString::from("secret"),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    // ── Tests 2–5 require a real signed JWT. ─────────────────────────────────
    // Pattern: generate an RS256 key pair, build a JWKS fixture from the public key,
    // mint a JWT with jsonwebtoken::encode, then call driver.authenticate.
    //
    // Add `rsa = "0.9"` + `base64ct = "1"` to [dev-dependencies] to build the
    // JWK modulus/exponent from a generated RSA key. Alternatively, embed a
    // static PEM pair and the corresponding JWKS fixture string as test constants.
    //
    // The test skeletons below compile immediately. Fill in key material
    // and JWKS fixtures in Step 3 once the implementation is in place.

    #[tokio::test]
    async fn oidc_authenticates_valid_jwt() {
        // 1. Generate RSA key pair (or use embedded constants)
        // 2. Build JWKS JSON fixture from public key modulus/exponent
        // 3. Mint a JWT:
        //    Header { alg: RS256, kid: "test-kid-1" }
        //    Claims { iss, aud, sub, exp: now+300, groups: [] }
        // 4. Build driver with static JWKS fetch fn
        // 5. Assert AuthResult::Authenticated with correct sub
        todo!("requires key generation — implement after Step 3")
    }

    #[tokio::test]
    async fn oidc_rejects_expired_token() {
        // Same as above but exp = now - 60
        // Assert AuthResult::Reject(msg) where msg mentions "expired"
        todo!("requires key generation — implement after Step 3")
    }

    #[tokio::test]
    async fn oidc_rejects_wrong_audience() {
        // Mint token with aud = "wrong-api"
        // Assert AuthResult::Reject(msg) where msg mentions "audience"
        todo!("requires key generation — implement after Step 3")
    }

    #[tokio::test]
    async fn oidc_rejects_unknown_kid() {
        // Mint token with kid = "nonexistent-kid"
        // JWKS has a key with kid = "real-kid-1"
        // Assert AuthResult::Reject(msg) where msg mentions "kid"
        todo!("requires key generation — implement after Step 3")
    }
}
```

### Step 2: Verify test 1 passes, tests 2–5 are todos

```bash
cargo test -p ox_security_auth oidc 2>&1 | tail -20
```

Expected: `oidc_continues_for_non_bearer_creds` passes; tests 2–5 marked as todos (they panic with `todo!` — that is expected and acceptable at this stage; replace with real assertions in Task 4).

### Step 3: Commit failing tests

```bash
git add crates/security/ox_security_auth/src/drivers/oidc.rs
git commit -m "test(security-auth): add OidcAuthDriver test skeletons (1 passing, 4 todos)"
```

---

## Task 4: Implement OidcAuthDriver

**Files:**
- Modify: `crates/security/ox_security_auth/src/drivers/oidc.rs` — replace stub `authenticate` with full implementation

### Step 1: Add `rsa` and `base64ct` dev-dependencies for test key material

Modify `crates/security/ox_security_auth/Cargo.toml` dev-dependencies:

```toml
[dev-dependencies]
tokio     = { version = "1", features = ["macros", "rt", "time", "sync"] }
x509-cert = "0.2"
rsa       = { version = "0.9", features = ["pem"] }
base64ct  = { version = "1", features = ["std"] }
```

### Step 2: Implement `authenticate` in `oidc.rs`

Replace the stub `authenticate` method with the full implementation. Full file content:

```rust
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use futures::future::BoxFuture;
use async_trait::async_trait;
use tokio::sync::Mutex;
use serde::{Deserialize, Serialize};
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, Principal, PrincipalId,
    AuthSource, TenantId, GroupId, drivers::AuthDriver,
};

// ── Public types ─────────────────────────────────────────────────────────────

/// Returns the full JWKS JSON as a `String`, or an error message.
/// Injected at construction time; in production, use a `reqwest`-backed closure.
pub type JwksFetchFn = Arc<dyn Fn() -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

/// OIDC provider configuration.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// Issuer URL — must match the `iss` claim in the JWT.
    pub issuer_url: String,
    /// Expected `aud` claim value.
    pub audience: String,
    /// JWKS endpoint URL.
    pub jwks_url: String,
    /// Tenant identifier stamped on every `Principal` produced by this driver.
    pub tenant_id: TenantId,
}

// ── Internal JWKS model ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize, Clone)]
struct Jwk {
    /// Key type, e.g. "RSA" or "EC"
    kty: String,
    /// Key ID — matched against the JWT header's `kid`
    kid: Option<String>,
    /// Algorithm, e.g. "RS256" or "ES256"
    alg: Option<String>,
    // RSA fields
    n: Option<String>,   // Base64url-encoded modulus
    e: Option<String>,   // Base64url-encoded exponent
    // EC fields
    crv: Option<String>, // Curve name, e.g. "P-256"
    x: Option<String>,   // Base64url-encoded x coordinate
    y: Option<String>,   // Base64url-encoded y coordinate
}

// ── Internal claims model ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OidcClaims {
    /// Subject — becomes the PrincipalId display name
    sub: String,
    /// Issuer
    iss: String,
    /// Audience (string or array of strings — jsonwebtoken handles both)
    aud: jsonwebtoken::decode::AudienceAsStrings,
    /// Expiry (Unix timestamp)
    exp: u64,
    /// Optional: preferred_username or email for display_name
    #[serde(default)]
    preferred_username: Option<String>,
    /// Optional: groups claim (Okta, Entra ID)
    #[serde(default)]
    groups: Vec<String>,
    /// Optional: roles claim (Entra ID app roles)
    #[serde(default)]
    roles: Vec<String>,
}

// ── Cache ─────────────────────────────────────────────────────────────────────

struct CachedJwks {
    jwks: Jwks,
    fetched_at: Instant,
}

// ── Driver ────────────────────────────────────────────────────────────────────

pub struct OidcAuthDriver {
    config: OidcConfig,
    fetch_jwks: JwksFetchFn,
    cache: Arc<Mutex<Option<CachedJwks>>>,
    cache_ttl: Duration,
}

impl OidcAuthDriver {
    pub fn new(config: OidcConfig, fetch_jwks: JwksFetchFn) -> Self {
        Self::with_ttl(config, fetch_jwks, Duration::from_secs(300))
    }

    pub fn with_ttl(config: OidcConfig, fetch_jwks: JwksFetchFn, cache_ttl: Duration) -> Self {
        Self {
            config,
            fetch_jwks,
            cache: Arc::new(Mutex::new(None)),
            cache_ttl,
        }
    }

    /// Fetch the JWKS, using the in-process cache when still fresh.
    async fn get_jwks(&self) -> Result<Vec<Jwk>, String> {
        let mut guard = self.cache.lock().await;
        if let Some(ref cached) = *guard {
            if cached.fetched_at.elapsed() < self.cache_ttl {
                return Ok(cached.jwks.keys.clone());
            }
        }
        // Cache miss or expired — fetch fresh
        let raw = (self.fetch_jwks)().await?;
        let jwks: Jwks = serde_json::from_str(&raw)
            .map_err(|e| format!("JWKS parse error: {}", e))?;
        let keys = jwks.keys.clone();
        *guard = Some(CachedJwks { jwks, fetched_at: Instant::now() });
        Ok(keys)
    }

    /// Find the JWK matching the JWT header's `kid`.
    fn find_key<'a>(keys: &'a [Jwk], kid: &Option<String>) -> Result<&'a Jwk, String> {
        match kid {
            None => {
                // No kid — use the first key if there is exactly one
                if keys.len() == 1 {
                    Ok(&keys[0])
                } else {
                    Err("JWT has no kid and JWKS contains multiple keys — cannot determine which key to use".to_string())
                }
            }
            Some(k) => keys
                .iter()
                .find(|jwk| jwk.kid.as_deref() == Some(k.as_str()))
                .ok_or_else(|| format!("no JWK found for kid={}", k)),
        }
    }

    /// Build a `jsonwebtoken::DecodingKey` from a JWK.
    fn decoding_key(jwk: &Jwk) -> Result<jsonwebtoken::DecodingKey, String> {
        match jwk.kty.as_str() {
            "RSA" => {
                let n = jwk.n.as_deref().ok_or("JWK RSA missing 'n'")?;
                let e = jwk.e.as_deref().ok_or("JWK RSA missing 'e'")?;
                jsonwebtoken::DecodingKey::from_rsa_components(n, e)
                    .map_err(|e| format!("RSA key construction failed: {}", e))
            }
            "EC" => {
                let x = jwk.x.as_deref().ok_or("JWK EC missing 'x'")?;
                let y = jwk.y.as_deref().ok_or("JWK EC missing 'y'")?;
                jsonwebtoken::DecodingKey::from_ec_components(x, y)
                    .map_err(|e| format!("EC key construction failed: {}", e))
            }
            other => Err(format!("unsupported JWK kty={}", other)),
        }
    }

    /// Map an `alg` string to `jsonwebtoken::Algorithm`.
    fn parse_algorithm(alg: Option<&str>) -> Result<jsonwebtoken::Algorithm, String> {
        match alg.unwrap_or("RS256") {
            "RS256" => Ok(jsonwebtoken::Algorithm::RS256),
            "RS384" => Ok(jsonwebtoken::Algorithm::RS384),
            "RS512" => Ok(jsonwebtoken::Algorithm::RS512),
            "ES256" => Ok(jsonwebtoken::Algorithm::ES256),
            "ES384" => Ok(jsonwebtoken::Algorithm::ES384),
            other => Err(format!("unsupported JWT algorithm: {}", other)),
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

        // 2. Fetch JWKS (cached)
        let keys = match self.get_jwks().await {
            Ok(k) => k,
            Err(e) => return AuthResult::Reject(format!("JWKS fetch failed: {}", e)),
        };

        // 3. Find matching key by kid
        let jwk = match Self::find_key(&keys, &header.kid) {
            Ok(k) => k,
            Err(e) => return AuthResult::Reject(e),
        };

        // 4. Build decoding key
        let decoding_key = match Self::decoding_key(jwk) {
            Ok(k) => k,
            Err(e) => return AuthResult::Reject(e),
        };

        // 5. Determine algorithm (prefer JWK's alg, fall back to JWT header)
        let alg_str = jwk.alg.as_deref()
            .or_else(|| header.alg.to_str().ok())
            .map(|s| s.to_string());
        let algorithm = match Self::parse_algorithm(alg_str.as_deref()) {
            Ok(a) => a,
            Err(e) => return AuthResult::Reject(e),
        };

        // 6. Verify signature + claims (iss, aud, exp)
        let mut validation = jsonwebtoken::Validation::new(algorithm);
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

        // 7. Extract claims
        let claims = token_data.claims;

        let sub = match claims.get("sub").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return AuthResult::Reject("JWT missing 'sub' claim".to_string()),
        };

        let display_name = claims
            .get("preferred_username")
            .or_else(|| claims.get("email"))
            .and_then(|v| v.as_str())
            .unwrap_or(&sub)
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
```

> **Note on `OidcClaims` vs `serde_json::Value`:** The implementation above deserialises claims as `serde_json::Value` to avoid issues with `aud` being either a string or an array (a common OIDC quirk). `jsonwebtoken`'s `Validation` handles the `aud`/`iss`/`exp` checks natively. The `sub`, `preferred_username`, `groups`, and `roles` fields are then extracted manually from the `Value`.

### Step 3: Fill in the real test implementations

Replace the `todo!()` bodies in the `#[cfg(test)] mod tests` block with real token minting. The recommended approach uses `jsonwebtoken`'s built-in test support with an RS256 key pair:

```rust
// Add to [dev-dependencies] in Cargo.toml:
//   rsa = { version = "0.9", features = ["pem"] }
//   base64ct = { version = "1", features = ["std"] }

use base64ct::{Base64UrlUnpadded, Encoding};
use rsa::{RsaPrivateKey, RsaPublicKey};
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::traits::PublicKeyParts;

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

fn mint_token(
    private_key: &RsaPrivateKey,
    kid: &str,
    claims: &TestClaims,
) -> String {
    let pem = private_key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF).unwrap();
    let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());
    encode(&header, claims, &encoding_key).unwrap()
}

// ── Test implementations ──────────────────────────────────────────────────────

#[tokio::test]
async fn oidc_authenticates_valid_jwt() {
    let (private_key, public_key) = generate_test_key_pair();
    let kid = "test-kid-1";
    let jwks_json = jwks_fixture(&public_key, kid);

    let driver = OidcAuthDriver::new(
        test_config(),
        static_jwks_fn(jwks_json),
    );

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
        }
        other => panic!("expected Authenticated, got {:?}", other),
    }
}

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
        exp: now_secs() - 60, // expired 60 seconds ago
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
        other => panic!("expected Reject, got {:?}", other),
    }
}

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
        other => panic!("expected Reject, got {:?}", other),
    }
}

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
        other => panic!("expected Reject, got {:?}", other),
    }
}
```

Also add `rand` to dev-dependencies:

```toml
[dev-dependencies]
tokio     = { version = "1", features = ["macros", "rt", "time", "sync"] }
x509-cert = "0.2"
rsa       = { version = "0.9", features = ["pem"] }
base64ct  = { version = "1", features = ["std"] }
rand      = "0.8"
```

### Step 4: Run all OIDC tests

```bash
cargo test -p ox_security_auth oidc 2>&1 | tail -20
```

Expected: all 5 tests pass — `oidc_continues_for_non_bearer_creds`, `oidc_authenticates_valid_jwt`, `oidc_rejects_expired_token`, `oidc_rejects_wrong_audience`, `oidc_rejects_unknown_kid`.

### Step 5: Run full suite

```bash
cargo test -p ox_security_auth 2>&1 | tail -10
```

Expected: all tests pass (at minimum 13 pre-existing + 5 OIDC = 18 tests).

### Step 6: Commit

```bash
git add crates/security/ox_security_auth/
git commit -m "feat(security-auth): implement OidcAuthDriver — JWKS cache, RS256/ES256 verify, 5 tests"
```

---

## Task 5: Production JWKS fetch (reqwest wiring — optional, binary-side)

This task is performed in the consuming binary/integration layer, not in `ox_security_auth` itself, because `reqwest` is an I/O dependency that should not be forced on every consumer.

### Cargo.toml for the binary crate

```toml
[dependencies]
ox_security_auth = { path = "../../crates/security/ox_security_auth" }
reqwest  = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
futures  = "0.3"
tokio    = { version = "1", features = ["full"] }
```

### Building the fetch closure

```rust
use ox_security_auth::{OidcAuthDriver, OidcConfig, JwksFetchFn};
use std::sync::Arc;

fn make_oidc_driver(config: OidcConfig) -> OidcAuthDriver {
    let jwks_url = config.jwks_url.clone();
    let fetch: JwksFetchFn = Arc::new(move || {
        let url = jwks_url.clone();
        Box::pin(async move {
            reqwest::get(&url)
                .await
                .map_err(|e| format!("JWKS HTTP error: {}", e))?
                .text()
                .await
                .map_err(|e| format!("JWKS body read error: {}", e))
        })
    });
    OidcAuthDriver::new(config, fetch)
}
```

No commit needed — this is documentation for the integration author.

---

## Summary of changes

| File | Change |
|---|---|
| `crates/security/ox_security_auth/Cargo.toml` | Add `jsonwebtoken`, `serde`, `serde_json`, `futures`, `tokio[sync]`; add `rsa`, `base64ct`, `rand` to dev-deps |
| `crates/security/ox_security_auth/src/drivers/oidc.rs` | New file: `OidcConfig`, `JwksFetchFn`, `CachedJwks`, `OidcAuthDriver`, 5 tests |
| `crates/security/ox_security_auth/src/drivers/mod.rs` | Add `pub(crate) mod oidc` + `pub use oidc::{OidcAuthDriver, OidcConfig, JwksFetchFn}` |
| `crates/security/ox_security_auth/src/lib.rs` | Add `OidcAuthDriver`, `OidcConfig`, `JwksFetchFn` to re-exports |
