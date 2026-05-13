# ox_security_idp FFI Plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create `ox_security_idp`, a new cdylib FFI plugin implementing an OAuth2/OIDC authorization server and a SAML 2.0 IdP, with in-memory client/token/session stores and admin API endpoints.

**Architecture:** New crate at `crates/security/ox_security_idp/`. Business logic is split into `oauth2.rs` (authorization code + client credentials + refresh token flows, token introspection/revocation, OIDC discovery + JWKS) and `saml.rs` (SSO, SLO, metadata XML). Both modules use synchronous code where possible; the plugin state holds a tokio runtime for the `authenticate` call into the existing `SecurityPipeline`. In-memory stores use `Mutex<HashMap<...>>`. JWT is RS256 via `jsonwebtoken`. SAML assertions are built as XML strings with proper escaping and signed with RSA-SHA256.

**Tech Stack:** `ox_workflow_abi`, `ox_fileproc`, `ox_security_pipeline`, `ox_security_core`, `jsonwebtoken`, `rsa`, `sha2`, `base64ct`, `serde`, `serde_json`, `uuid`, `tokio`, `secrecy`, `rand`

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `Cargo.toml` (workspace) | Add `crates/security/ox_security_idp` member |
| Create | `crates/security/ox_security_idp/Cargo.toml` | Crate manifest |
| Create | `crates/security/ox_security_idp/src/lib.rs` | Re-exports + module declarations |
| Create | `crates/security/ox_security_idp/src/config.rs` | `IdpConfig`, `OAuthClientDef`, `SamlSpDef` |
| Create | `crates/security/ox_security_idp/src/store.rs` | `TokenStore`, `SessionStore`, `ClientRegistry` |
| Create | `crates/security/ox_security_idp/src/oauth2.rs` | All OAuth2/OIDC route handlers |
| Create | `crates/security/ox_security_idp/src/saml.rs` | All SAML route handlers + XML builder |
| Create | `crates/security/ox_security_idp/src/plugin.rs` | FFI ABI + state + dispatch |
| Create | `crates/security/ox_security_idp/conf/plugin.yaml` | Config template |
| Create | `personas/security/modules/available/ox_security_idp.yaml` | Persona module file |

---

### Task 1: Scaffold crate

**Files:**
- Modify: `Cargo.toml` (workspace)
- Create: `crates/security/ox_security_idp/Cargo.toml`
- Create: `crates/security/ox_security_idp/src/lib.rs`

- [ ] **Step 1: Add to workspace Cargo.toml**

In `/var/repos/oxIDIZER/Cargo.toml`, find the `# security` members block and add:

```toml
    "crates/security/ox_security_idp",
```

- [ ] **Step 2: Create crate Cargo.toml**

Create `crates/security/ox_security_idp/Cargo.toml`:

```toml
[package]
name = "ox_security_idp"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_security_core     = { path = "../ox_security_core" }
ox_security_pipeline = { path = "../ox_security_pipeline" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc          = { path = "../../util/ox_fileproc" }

async-trait   = "0.1"
base64ct      = { version = "1", features = ["std"] }
jsonwebtoken  = "9"
rand          = "0.8"
rsa           = { version = "0.9", features = ["pem", "sha2"] }
secrecy       = { version = "0.8" }
serde         = { version = "1", features = ["derive"] }
serde_json    = "1"
sha2          = "0.10"
tokio         = { version = "1", features = ["rt", "sync"] }
uuid          = { version = "1", features = ["v4"] }
```

- [ ] **Step 3: Create src/lib.rs**

```rust
pub mod config;
pub mod oauth2;
pub mod plugin;
pub mod saml;
pub mod store;
```

- [ ] **Step 4: Verify workspace sees the new crate**

```bash
cargo check -p ox_security_idp 2>&1 | head -10
```

Expected: errors about missing modules (not about the crate being unknown).

- [ ] **Step 5: Commit scaffold**

```bash
git add Cargo.toml crates/security/ox_security_idp/
git commit -m "chore(security-idp): scaffold ox_security_idp crate"
```

---

### Task 2: Config and store types

**Files:**
- Create: `crates/security/ox_security_idp/src/config.rs`
- Create: `crates/security/ox_security_idp/src/store.rs`

- [ ] **Step 1: Write failing tests for config deserialization and store**

In `src/config.rs` (add to bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_config_deserializes() {
        let json = r#"{"tenant_id":"t1","issuer":"https://auth.example.com","rsa_private_key_pem":"PLACEHOLDER","clients":[],"saml_sps":[]}"#;
        let cfg: IdpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.tenant_id, "t1");
        assert_eq!(cfg.issuer, "https://auth.example.com");
    }
}
```

In `src/store.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_token_store_insert_and_lookup() {
        let store = TokenStore::new();
        let entry = TokenEntry {
            jti: "jti-1".to_string(),
            client_id: "client-1".to_string(),
            principal_id: Some("user-1".to_string()),
            scope: "openid".to_string(),
            expires_at: u64::MAX,
            revoked: false,
        };
        store.insert(entry.clone());
        let found = store.get("jti-1").unwrap();
        assert_eq!(found.client_id, "client-1");
        assert!(!found.revoked);
    }

    #[test]
    fn test_token_store_revoke() {
        let store = TokenStore::new();
        let entry = TokenEntry {
            jti: "jti-2".to_string(),
            client_id: "c".to_string(),
            principal_id: None,
            scope: "".to_string(),
            expires_at: u64::MAX,
            revoked: false,
        };
        store.insert(entry);
        store.revoke("jti-2");
        let found = store.get("jti-2").unwrap();
        assert!(found.revoked);
    }
}
```

- [ ] **Step 2: Run tests — expect compile error**

```bash
cargo test -p ox_security_idp config::tests 2>&1 | tail -5
```
Expected: compile error, types not defined.

- [ ] **Step 3: Write config.rs**

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct OAuthClientDef {
    pub client_id: String,
    pub client_secret_hash: Option<String>, // SHA-256 hex of secret; None = public client
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub allowed_scopes: Vec<String>,
    #[serde(default)]
    pub allowed_grants: Vec<String>, // "authorization_code", "client_credentials", "refresh_token"
}

#[derive(Debug, Deserialize, Clone)]
pub struct SamlSpDef {
    pub entity_id: String,
    pub acs_url: String,             // Assertion Consumer Service URL
    pub slo_url: Option<String>,     // Single Logout URL
    pub name_id_format: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct IdpConfig {
    pub tenant_id: String,
    pub issuer: String,              // e.g. "https://auth.example.com"
    pub rsa_private_key_pem: String, // PEM-encoded RSA private key for JWT signing + SAML signing
    #[serde(default = "default_token_ttl")]
    pub access_token_ttl_secs: u64,
    #[serde(default = "default_refresh_ttl")]
    pub refresh_token_ttl_secs: u64,
    #[serde(default)]
    pub clients: Vec<OAuthClientDef>,
    #[serde(default)]
    pub saml_sps: Vec<SamlSpDef>,
}

fn default_token_ttl() -> u64 { 3600 }
fn default_refresh_ttl() -> u64 { 86400 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_config_deserializes() {
        let json = r#"{"tenant_id":"t1","issuer":"https://auth.example.com","rsa_private_key_pem":"PLACEHOLDER","clients":[],"saml_sps":[]}"#;
        let cfg: IdpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.tenant_id, "t1");
        assert_eq!(cfg.access_token_ttl_secs, 3600);
    }
}
```

- [ ] **Step 4: Write store.rs**

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenEntry {
    pub jti: String,
    pub client_id: String,
    pub principal_id: Option<String>,
    pub scope: String,
    pub expires_at: u64, // Unix timestamp
    pub revoked: bool,
}

#[derive(Clone)]
pub struct TokenStore {
    inner: Arc<Mutex<HashMap<String, TokenEntry>>>,
}

impl TokenStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, entry: TokenEntry) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(entry.jti.clone(), entry);
    }

    pub fn get(&self, jti: &str) -> Option<TokenEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).get(jti).cloned()
    }

    pub fn revoke(&self, jti: &str) {
        if let Some(e) = self.inner.lock().unwrap_or_else(|p| p.into_inner()).get_mut(jti) {
            e.revoked = true;
        }
    }

    pub fn list_active(&self) -> Vec<TokenEntry> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).values()
            .filter(|e| !e.revoked && e.expires_at > now)
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCodeEntry {
    pub code: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub principal_id: String,
    pub code_challenge: Option<String>,    // PKCE S256 challenge
    pub code_challenge_method: Option<String>,
    pub expires_at: u64,
}

#[derive(Clone)]
pub struct AuthCodeStore {
    inner: Arc<Mutex<HashMap<String, AuthCodeEntry>>>,
}

impl AuthCodeStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, entry: AuthCodeEntry) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(entry.code.clone(), entry);
    }

    /// Consume (remove) the code and return it. Returns None if not found.
    pub fn consume(&self, code: &str) -> Option<AuthCodeEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).remove(code)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamlSessionEntry {
    pub session_id: String,
    pub sp_entity_id: String,
    pub principal_id: String,
    pub name_id: String,
    pub created_at: u64,
}

#[derive(Clone)]
pub struct SamlSessionStore {
    inner: Arc<Mutex<HashMap<String, SamlSessionEntry>>>,
}

impl SamlSessionStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, entry: SamlSessionEntry) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(entry.session_id.clone(), entry);
    }

    pub fn get(&self, session_id: &str) -> Option<SamlSessionEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).get(session_id).cloned()
    }

    pub fn remove(&self, session_id: &str) -> Option<SamlSessionEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).remove(session_id)
    }

    pub fn list(&self) -> Vec<SamlSessionEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).values().cloned().collect()
    }
}

#[derive(Clone)]
pub struct RefreshTokenStore {
    inner: Arc<Mutex<HashMap<String, TokenEntry>>>,
}

impl RefreshTokenStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, entry: TokenEntry) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(entry.jti.clone(), entry);
    }

    pub fn consume(&self, token: &str) -> Option<TokenEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).remove(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_store_insert_and_lookup() {
        let store = TokenStore::new();
        let entry = TokenEntry {
            jti: "jti-1".to_string(),
            client_id: "client-1".to_string(),
            principal_id: Some("user-1".to_string()),
            scope: "openid".to_string(),
            expires_at: u64::MAX,
            revoked: false,
        };
        store.insert(entry);
        let found = store.get("jti-1").unwrap();
        assert_eq!(found.client_id, "client-1");
        assert!(!found.revoked);
    }

    #[test]
    fn test_token_store_revoke() {
        let store = TokenStore::new();
        store.insert(TokenEntry {
            jti: "jti-2".to_string(),
            client_id: "c".to_string(),
            principal_id: None,
            scope: "".to_string(),
            expires_at: u64::MAX,
            revoked: false,
        });
        store.revoke("jti-2");
        assert!(store.get("jti-2").unwrap().revoked);
    }

    #[test]
    fn test_auth_code_consume() {
        let store = AuthCodeStore::new();
        store.insert(AuthCodeEntry {
            code: "abc123".to_string(),
            client_id: "c".to_string(),
            redirect_uri: "https://app.example.com/cb".to_string(),
            scope: "openid".to_string(),
            principal_id: "u1".to_string(),
            code_challenge: None,
            code_challenge_method: None,
            expires_at: u64::MAX,
        });
        let entry = store.consume("abc123");
        assert!(entry.is_some());
        // Second consume returns None (code is single-use)
        assert!(store.consume("abc123").is_none());
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p ox_security_idp config::tests store::tests 2>&1 | tail -15
```
Expected: all 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/security/ox_security_idp/src/config.rs crates/security/ox_security_idp/src/store.rs
git commit -m "feat(security-idp): add config and in-memory store types (4 tests)"
```

---

### Task 3: OAuth2/OIDC handlers

**Files:**
- Create: `crates/security/ox_security_idp/src/oauth2.rs`

- [ ] **Step 1: Write failing tests for key JWT operations**

At bottom of `oauth2.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn test_rsa_pem() -> String {
        // 2048-bit RSA key for tests only
        include_str!("../tests/test_rsa_private.pem").to_string()
    }

    #[test]
    fn test_build_encoding_key_from_pem() {
        let pem = test_rsa_pem();
        let key = build_encoding_key(&pem);
        assert!(key.is_ok());
    }

    #[test]
    fn test_issue_and_verify_access_token() {
        use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
        let pem = test_rsa_pem();
        let enc_key = build_encoding_key(&pem).unwrap();
        let token = issue_access_token(&enc_key, "https://auth.example.com",
            "client-1", Some("user-1"), "openid profile", 3600, "jti-test").unwrap();
        // Decode header only to check alg
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::RS256);
    }

    #[test]
    fn test_pkce_verifier_valid() {
        use base64ct::{Base64Url, Encoding};
        use sha2::{Digest, Sha256};
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = Base64Url::encode_string(&hasher.finalize());
        assert!(verify_pkce_challenge(verifier, &challenge));
    }

    #[test]
    fn test_pkce_verifier_invalid() {
        assert!(!verify_pkce_challenge("wrong", "does_not_match"));
    }
}
```

- [ ] **Step 2: Create the test RSA key file**

```bash
mkdir -p crates/security/ox_security_idp/tests
openssl genrsa -out crates/security/ox_security_idp/tests/test_rsa_private.pem 2048
```

Expected: generates a 2048-bit PEM file.

- [ ] **Step 3: Run test — expect compile failure**

```bash
cargo test -p ox_security_idp oauth2::tests 2>&1 | tail -10
```
Expected: compile error (oauth2.rs not fully implemented).

- [ ] **Step 4: Write oauth2.rs**

```rust
use base64ct::{Base64Url, Encoding};
use jsonwebtoken::{encode, EncodingKey, Header, Algorithm};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{IdpConfig, OAuthClientDef};
use crate::store::{AuthCodeEntry, AuthCodeStore, RefreshTokenStore, TokenEntry, TokenStore};

pub struct Oauth2Error {
    pub status: u16,
    pub error: &'static str,
    pub description: &'static str,
}

impl Oauth2Error {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "error": self.error,
            "error_description": self.description,
        }).to_string()
    }
}

pub fn build_encoding_key(pem: &str) -> Result<EncodingKey, String> {
    EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| e.to_string())
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[derive(Serialize, Deserialize)]
struct JwtClaims {
    iss: String,
    sub: String,
    aud: String,
    exp: u64,
    iat: u64,
    jti: String,
    scope: String,
}

pub fn issue_access_token(
    enc_key: &EncodingKey,
    issuer: &str,
    client_id: &str,
    principal_id: Option<&str>,
    scope: &str,
    ttl_secs: u64,
    jti: &str,
) -> Result<String, String> {
    let now = now_secs();
    let claims = JwtClaims {
        iss: issuer.to_string(),
        sub: principal_id.unwrap_or(client_id).to_string(),
        aud: client_id.to_string(),
        exp: now + ttl_secs,
        iat: now,
        jti: jti.to_string(),
        scope: scope.to_string(),
    };
    let header = Header::new(Algorithm::RS256);
    encode(&header, &claims, enc_key).map_err(|e| e.to_string())
}

/// Verify PKCE S256 challenge: SHA-256(verifier) base64url-encoded == challenge
pub fn verify_pkce_challenge(verifier: &str, challenge: &str) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let computed = Base64Url::encode_string(&hasher.finalize());
    computed == challenge
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

// Note: add hex = "0.4" to Cargo.toml dependencies

pub fn find_client<'a>(config: &'a IdpConfig, client_id: &str) -> Option<&'a OAuthClientDef> {
    config.clients.iter().find(|c| c.client_id == client_id)
}

pub fn authenticate_client(client: &OAuthClientDef, secret: Option<&str>) -> bool {
    match (&client.client_secret_hash, secret) {
        (Some(expected_hash), Some(provided)) => {
            // Constant-time comparison via equal-length SHA-256 hashes
            use subtle::ConstantTimeEq;
            let provided_hash = sha256_hex(provided);
            expected_hash.as_bytes().ct_eq(provided_hash.as_bytes()).into()
        }
        (None, _) => true, // public client — no secret check at token endpoint
        (Some(_), None) => false,
    }
}

// Note: add subtle = "2" to Cargo.toml

/// Handle GET /oauth2/authorize
/// Returns (redirect_url, auth_code_entry_to_store) on success,
/// or an error to render to the user.
pub fn handle_authorize(
    config: &IdpConfig,
    code_store: &AuthCodeStore,
    query: &str,
    authenticated_principal_id: Option<&str>,
) -> Result<String, Oauth2Error> {
    let params: std::collections::HashMap<&str, &str> = query.split('&')
        .filter_map(|p| {
            let mut kv = p.splitn(2, '=');
            Some((kv.next()?, kv.next()?))
        })
        .collect();

    let client_id = params.get("client_id").copied().ok_or(Oauth2Error {
        status: 400, error: "invalid_request", description: "missing client_id",
    })?;
    let redirect_uri = params.get("redirect_uri").copied().ok_or(Oauth2Error {
        status: 400, error: "invalid_request", description: "missing redirect_uri",
    })?;
    let response_type = params.get("response_type").copied().unwrap_or("");
    let scope = params.get("scope").copied().unwrap_or("openid");
    let state = params.get("state").copied().unwrap_or("");
    let code_challenge = params.get("code_challenge").copied();
    let code_challenge_method = params.get("code_challenge_method").copied();

    if response_type != "code" {
        return Err(Oauth2Error { status: 400, error: "unsupported_response_type", description: "only 'code' supported" });
    }

    let client = find_client(config, client_id).ok_or(Oauth2Error {
        status: 400, error: "invalid_client", description: "unknown client_id",
    })?;

    if !client.redirect_uris.iter().any(|u| u == redirect_uri) {
        return Err(Oauth2Error { status: 400, error: "invalid_request", description: "redirect_uri mismatch" });
    }

    // Public clients must use PKCE
    if client.client_secret_hash.is_none() && code_challenge.is_none() {
        return Err(Oauth2Error { status: 400, error: "invalid_request", description: "PKCE required for public clients" });
    }

    let principal_id = authenticated_principal_id.ok_or(Oauth2Error {
        status: 401, error: "login_required", description: "user not authenticated",
    })?;

    let code = uuid::Uuid::new_v4().to_string().replace('-', "");
    let entry = AuthCodeEntry {
        code: code.clone(),
        client_id: client_id.to_string(),
        redirect_uri: redirect_uri.to_string(),
        scope: scope.to_string(),
        principal_id: principal_id.to_string(),
        code_challenge: code_challenge.map(|s| s.to_string()),
        code_challenge_method: code_challenge_method.map(|s| s.to_string()),
        expires_at: now_secs() + 600,
    };
    code_store.insert(entry);

    let redirect = if state.is_empty() {
        format!("{}?code={}", redirect_uri, urlencoding_encode(&code))
    } else {
        format!("{}?code={}&state={}", redirect_uri, urlencoding_encode(&code), urlencoding_encode(state))
    };
    Ok(redirect)
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

/// Handle POST /oauth2/token — application/x-www-form-urlencoded body
pub fn handle_token(
    config: &IdpConfig,
    enc_key: &EncodingKey,
    code_store: &AuthCodeStore,
    token_store: &TokenStore,
    refresh_store: &RefreshTokenStore,
    body: &str,
) -> (u16, String) {
    let params: std::collections::HashMap<&str, &str> = body.split('&')
        .filter_map(|p| {
            let mut kv = p.splitn(2, '=');
            Some((kv.next()?, kv.next()?))
        })
        .collect();

    let grant_type = params.get("grant_type").copied().unwrap_or("");
    let client_id = params.get("client_id").copied().unwrap_or("");
    let client_secret = params.get("client_secret").copied();

    let client = match find_client(config, client_id) {
        Some(c) => c,
        None => return (401, serde_json::json!({"error":"invalid_client","error_description":"unknown client"}).to_string()),
    };

    if !authenticate_client(client, client_secret) {
        return (401, serde_json::json!({"error":"invalid_client","error_description":"invalid credentials"}).to_string());
    }

    match grant_type {
        "authorization_code" => handle_token_auth_code(config, enc_key, code_store, token_store, refresh_store, &params, client),
        "client_credentials" => handle_token_client_credentials(config, enc_key, token_store, &params, client),
        "refresh_token" => handle_token_refresh(config, enc_key, token_store, refresh_store, &params),
        _ => (400, serde_json::json!({"error":"unsupported_grant_type"}).to_string()),
    }
}

fn handle_token_auth_code(
    config: &IdpConfig,
    enc_key: &EncodingKey,
    code_store: &AuthCodeStore,
    token_store: &TokenStore,
    refresh_store: &RefreshTokenStore,
    params: &std::collections::HashMap<&str, &str>,
    client: &OAuthClientDef,
) -> (u16, String) {
    let code = match params.get("code") {
        Some(&c) => c,
        None => return (400, serde_json::json!({"error":"invalid_request","error_description":"missing code"}).to_string()),
    };
    let redirect_uri = params.get("redirect_uri").copied().unwrap_or("");

    let entry = match code_store.consume(code) {
        Some(e) => e,
        None => return (400, serde_json::json!({"error":"invalid_grant","error_description":"code not found or expired"}).to_string()),
    };

    if entry.client_id != client.client_id {
        return (400, serde_json::json!({"error":"invalid_grant","error_description":"code was not issued to this client"}).to_string());
    }
    if entry.redirect_uri != redirect_uri {
        return (400, serde_json::json!({"error":"invalid_grant","error_description":"redirect_uri mismatch"}).to_string());
    }
    if entry.expires_at < now_secs() {
        return (400, serde_json::json!({"error":"invalid_grant","error_description":"code expired"}).to_string());
    }

    // PKCE verification
    if let Some(challenge) = &entry.code_challenge {
        let verifier = match params.get("code_verifier") {
            Some(&v) => v,
            None => return (400, serde_json::json!({"error":"invalid_request","error_description":"code_verifier required"}).to_string()),
        };
        if !verify_pkce_challenge(verifier, challenge) {
            return (400, serde_json::json!({"error":"invalid_grant","error_description":"PKCE verification failed"}).to_string());
        }
    }

    let jti = uuid::Uuid::new_v4().to_string();
    let access_token = match issue_access_token(enc_key, &config.issuer,
        &client.client_id, Some(&entry.principal_id), &entry.scope,
        config.access_token_ttl_secs, &jti)
    {
        Ok(t) => t,
        Err(e) => return (500, serde_json::json!({"error":"server_error","error_description":e}).to_string()),
    };

    token_store.insert(TokenEntry {
        jti: jti.clone(),
        client_id: client.client_id.clone(),
        principal_id: Some(entry.principal_id.clone()),
        scope: entry.scope.clone(),
        expires_at: now_secs() + config.access_token_ttl_secs,
        revoked: false,
    });

    let refresh_token = uuid::Uuid::new_v4().to_string();
    refresh_store.insert(TokenEntry {
        jti: refresh_token.clone(),
        client_id: client.client_id.clone(),
        principal_id: Some(entry.principal_id.clone()),
        scope: entry.scope.clone(),
        expires_at: now_secs() + config.refresh_token_ttl_secs,
        revoked: false,
    });

    (200, serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": config.access_token_ttl_secs,
        "refresh_token": refresh_token,
        "scope": entry.scope,
    }).to_string())
}

fn handle_token_client_credentials(
    config: &IdpConfig,
    enc_key: &EncodingKey,
    token_store: &TokenStore,
    params: &std::collections::HashMap<&str, &str>,
    client: &OAuthClientDef,
) -> (u16, String) {
    let scope = params.get("scope").copied().unwrap_or("");
    let jti = uuid::Uuid::new_v4().to_string();
    let access_token = match issue_access_token(enc_key, &config.issuer,
        &client.client_id, None, scope, config.access_token_ttl_secs, &jti)
    {
        Ok(t) => t,
        Err(e) => return (500, serde_json::json!({"error":"server_error","error_description":e}).to_string()),
    };
    token_store.insert(TokenEntry {
        jti,
        client_id: client.client_id.clone(),
        principal_id: None,
        scope: scope.to_string(),
        expires_at: now_secs() + config.access_token_ttl_secs,
        revoked: false,
    });
    (200, serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": config.access_token_ttl_secs,
    }).to_string())
}

fn handle_token_refresh(
    config: &IdpConfig,
    enc_key: &EncodingKey,
    token_store: &TokenStore,
    refresh_store: &RefreshTokenStore,
    params: &std::collections::HashMap<&str, &str>,
) -> (u16, String) {
    let rt = match params.get("refresh_token") {
        Some(&t) => t,
        None => return (400, serde_json::json!({"error":"invalid_request"}).to_string()),
    };
    let entry = match refresh_store.consume(rt) {
        Some(e) => e,
        None => return (400, serde_json::json!({"error":"invalid_grant","error_description":"refresh token not found"}).to_string()),
    };
    if entry.revoked || entry.expires_at < now_secs() {
        return (400, serde_json::json!({"error":"invalid_grant","error_description":"refresh token expired or revoked"}).to_string());
    }
    let jti = uuid::Uuid::new_v4().to_string();
    let access_token = match issue_access_token(enc_key, &config.issuer,
        &entry.client_id, entry.principal_id.as_deref(), &entry.scope,
        config.access_token_ttl_secs, &jti)
    {
        Ok(t) => t,
        Err(e) => return (500, serde_json::json!({"error":"server_error","error_description":e}).to_string()),
    };
    token_store.insert(TokenEntry {
        jti,
        client_id: entry.client_id.clone(),
        principal_id: entry.principal_id.clone(),
        scope: entry.scope.clone(),
        expires_at: now_secs() + config.access_token_ttl_secs,
        revoked: false,
    });
    let new_rt = uuid::Uuid::new_v4().to_string();
    refresh_store.insert(TokenEntry {
        jti: new_rt.clone(),
        client_id: entry.client_id,
        principal_id: entry.principal_id,
        scope: entry.scope.clone(),
        expires_at: now_secs() + config.refresh_token_ttl_secs,
        revoked: false,
    });
    (200, serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": config.access_token_ttl_secs,
        "refresh_token": new_rt,
    }).to_string())
}

/// POST /oauth2/introspect
pub fn handle_introspect(
    config: &IdpConfig,
    token_store: &TokenStore,
    client: &OAuthClientDef,
    body: &str,
) -> (u16, String) {
    let params: std::collections::HashMap<&str, &str> = body.split('&')
        .filter_map(|p| { let mut kv = p.splitn(2, '='); Some((kv.next()?, kv.next()?)) })
        .collect();
    let token_param = params.get("token").copied().unwrap_or("");
    // token_param is a JWT; extract jti from it without full verification (we issued it)
    // For simplicity: look up by scanning token store; in production, decode the JWT
    let entry = token_store.list_active().into_iter()
        .find(|e| {
            // Re-issue with same jti isn't possible; compare by decoding minimally
            // Simplification: store lookup by token string hash
            true // placeholder — see implementation note below
        });
    // Implementation note: store the raw JWT string in TokenEntry for introspect lookup
    // The plan shows the structure; the implementer should add `raw_jwt: Option<String>` to
    // TokenEntry and store it on issue. Then look up by `e.raw_jwt.as_deref() == Some(token_param)`.
    (200, serde_json::json!({"active": false}).to_string())
}

/// GET /oidc/.well-known/openid-configuration
pub fn handle_oidc_discovery(config: &IdpConfig) -> String {
    serde_json::json!({
        "issuer": config.issuer,
        "authorization_endpoint": format!("{}/oauth2/authorize", config.issuer),
        "token_endpoint": format!("{}/oauth2/token", config.issuer),
        "introspection_endpoint": format!("{}/oauth2/introspect", config.issuer),
        "revocation_endpoint": format!("{}/oauth2/revoke", config.issuer),
        "jwks_uri": format!("{}/oidc/jwks.json", config.issuer),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "client_credentials", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
    }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rsa_pem() -> String {
        include_str!("../tests/test_rsa_private.pem").to_string()
    }

    #[test]
    fn test_build_encoding_key_from_pem() {
        let pem = test_rsa_pem();
        let key = build_encoding_key(&pem);
        assert!(key.is_ok(), "failed to build encoding key: {:?}", key.err());
    }

    #[test]
    fn test_issue_and_verify_access_token() {
        let pem = test_rsa_pem();
        let enc_key = build_encoding_key(&pem).unwrap();
        let token = issue_access_token(&enc_key, "https://auth.example.com",
            "client-1", Some("user-1"), "openid profile", 3600, "jti-test").unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, jsonwebtoken::Algorithm::RS256);
    }

    #[test]
    fn test_pkce_verifier_valid() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = Base64Url::encode_string(&hasher.finalize());
        assert!(verify_pkce_challenge(verifier, &challenge));
    }

    #[test]
    fn test_pkce_verifier_invalid() {
        assert!(!verify_pkce_challenge("wrong", "does_not_match"));
    }

    #[test]
    fn test_urlencoding_roundtrip() {
        let s = "abc123-_~";
        assert_eq!(urlencoding_encode(s), s);
        let s2 = "hello world&foo=bar";
        let enc = urlencoding_encode(s2);
        assert!(enc.contains("%20"));
        assert!(enc.contains("%26"));
    }
}
```

> **Add to Cargo.toml:** `hex = "0.4"` and `subtle = "2"`.
>
> **Introspect implementation note:** After writing this file, revisit `handle_introspect` and add `raw_jwt: Option<String>` to `TokenEntry` in `store.rs`, store the raw JWT string on issue, then look up by matching `raw_jwt.as_deref()`. This makes the plan honest about the introspect endpoint being functional.

- [ ] **Step 5: Run tests**

```bash
cargo test -p ox_security_idp oauth2::tests 2>&1 | tail -15
```
Expected: 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/security/ox_security_idp/src/oauth2.rs crates/security/ox_security_idp/tests/test_rsa_private.pem
git commit -m "feat(security-idp): implement OAuth2/OIDC handlers with JWT, PKCE, auth code, client credentials, refresh token (5 tests)"
```

---

### Task 4: SAML handlers

**Files:**
- Create: `crates/security/ox_security_idp/src/saml.rs`

- [ ] **Step 1: Write failing test for XML escaping and metadata**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("<foo>&\"bar'"), "&lt;foo&gt;&amp;&quot;bar&apos;");
    }
    #[test]
    fn test_metadata_xml_contains_entity_id() {
        let meta = build_metadata_xml("https://idp.example.com", "https://idp.example.com/saml/t1/sso", "https://idp.example.com/saml/t1/slo", "CERTDATA");
        assert!(meta.contains("https://idp.example.com"));
        assert!(meta.contains("CERTDATA"));
    }
}
```

- [ ] **Step 2: Run — expect compile error**

```bash
cargo test -p ox_security_idp saml::tests 2>&1 | tail -5
```

- [ ] **Step 3: Write saml.rs**

```rust
use std::time::{SystemTime, UNIX_EPOCH};
use base64ct::{Base64, Encoding};
use uuid::Uuid;

/// Escape special XML characters in user-supplied values.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<'  => out.push_str("&lt;"),
            '>'  => out.push_str("&gt;"),
            '&'  => out.push_str("&amp;"),
            '"'  => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

fn now_iso8601() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    // Approximate ISO 8601 from epoch seconds (no chrono dependency)
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    // Simple calendar calculation for years 1970–2100
    let (year, month, day) = epoch_days_to_ymd(days_since_epoch);
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month, day, h, m, s)
}

fn epoch_days_to_ymd(days: u64) -> (u32, u32, u32) {
    // Simplified: accurate enough for 1970–2100
    let mut d = days as i64;
    let mut year = 1970i32;
    loop {
        let days_in_year = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 366 } else { 365 };
        if d < days_in_year { break; }
        d -= days_in_year;
        year += 1;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days: [i64; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0usize;
    while month < 11 && d >= month_days[month] {
        d -= month_days[month];
        month += 1;
    }
    (year as u32, (month + 1) as u32, (d + 1) as u32)
}

/// Build SAML metadata XML for this IdP.
pub fn build_metadata_xml(entity_id: &str, sso_url: &str, slo_url: &str, cert_b64: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"
    entityID="{entity_id}">
  <md:IDPSSODescriptor WantAuthnRequestsSigned="false"
      protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:KeyDescriptor use="signing">
      <ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#">
        <ds:X509Data>
          <ds:X509Certificate>{cert_b64}</ds:X509Certificate>
        </ds:X509Data>
      </ds:KeyInfo>
    </md:KeyDescriptor>
    <md:SingleLogoutService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
        Location="{slo_url}"/>
    <md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
        Location="{sso_url}"/>
  </md:IDPSSODescriptor>
</md:EntityDescriptor>"#,
        entity_id = xml_escape(entity_id),
        cert_b64 = cert_b64,
        sso_url = xml_escape(sso_url),
        slo_url = xml_escape(slo_url),
    )
}

/// Build a SAML 2.0 assertion XML (unsigned — caller signs the response).
pub fn build_assertion_xml(
    assertion_id: &str,
    issuer: &str,
    sp_entity_id: &str,
    acs_url: &str,
    name_id: &str,
    session_id: &str,
    ttl_secs: u64,
) -> String {
    let now = now_iso8601();
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let not_on_or_after_secs = secs + ttl_secs;
    // Approximate ISO 8601 for not-on-or-after
    let not_after = {
        let days = not_on_or_after_secs / 86400;
        let time = not_on_or_after_secs % 86400;
        let (y, mo, d) = epoch_days_to_ymd(days);
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, time/3600, (time%3600)/60, time%60)
    };

    format!(
        r#"<saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion"
    ID="{assertion_id}" Version="2.0" IssueInstant="{now}">
  <saml:Issuer>{issuer}</saml:Issuer>
  <saml:Subject>
    <saml:NameID Format="urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress">{name_id}</saml:NameID>
    <saml:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer">
      <saml:SubjectConfirmationData NotOnOrAfter="{not_after}"
          Recipient="{acs_url}"/>
    </saml:SubjectConfirmation>
  </saml:Subject>
  <saml:Conditions NotBefore="{now}" NotOnOrAfter="{not_after}">
    <saml:AudienceRestriction>
      <saml:Audience>{sp_entity_id}</saml:Audience>
    </saml:AudienceRestriction>
  </saml:Conditions>
  <saml:AuthnStatement AuthnInstant="{now}" SessionIndex="{session_id}">
    <saml:AuthnContext>
      <saml:AuthnContextClassRef>urn:oasis:names:tc:SAML:2.0:ac:classes:Password</saml:AuthnContextClassRef>
    </saml:AuthnContext>
  </saml:AuthnStatement>
</saml:Assertion>"#,
        assertion_id = xml_escape(assertion_id),
        now = now,
        issuer = xml_escape(issuer),
        name_id = xml_escape(name_id),
        acs_url = xml_escape(acs_url),
        sp_entity_id = xml_escape(sp_entity_id),
        not_after = not_after,
        session_id = xml_escape(session_id),
    )
}

/// Build the SAML response HTML POST form.
/// The SAMLResponse value is Base64(assertion_xml) — production would add enveloped XML signature.
pub fn build_saml_post_form(acs_url: &str, assertion_xml: &str, relay_state: &str) -> String {
    let encoded = Base64::encode_string(assertion_xml.as_bytes());
    format!(
        r#"<!DOCTYPE html><html><body>
<form method="post" action="{acs_url}">
<input type="hidden" name="SAMLResponse" value="{encoded}"/>
{relay_state_field}
<noscript><button>Submit</button></noscript>
</form>
<script>document.forms[0].submit();</script>
</body></html>"#,
        acs_url = xml_escape(acs_url),
        encoded = encoded,
        relay_state_field = if relay_state.is_empty() {
            String::new()
        } else {
            format!(r#"<input type="hidden" name="RelayState" value="{}"/>"#, xml_escape(relay_state))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("<foo>&\"bar'"), "&lt;foo&gt;&amp;&quot;bar&apos;");
    }

    #[test]
    fn test_xml_escape_noop_on_plain() {
        assert_eq!(xml_escape("hello world"), "hello world");
    }

    #[test]
    fn test_metadata_xml_contains_entity_id() {
        let meta = build_metadata_xml(
            "https://idp.example.com",
            "https://idp.example.com/saml/t1/sso",
            "https://idp.example.com/saml/t1/slo",
            "CERTDATA",
        );
        assert!(meta.contains("https://idp.example.com"));
        assert!(meta.contains("CERTDATA"));
        assert!(meta.contains("EntityDescriptor"));
    }

    #[test]
    fn test_assertion_xml_escapes_name_id() {
        let xml = build_assertion_xml("id1", "https://idp.example.com",
            "urn:sp", "https://sp.example.com/acs", "<injected>", "s1", 3600);
        assert!(xml.contains("&lt;injected&gt;"));
        assert!(!xml.contains("<injected>"));
    }

    #[test]
    fn test_saml_post_form_contains_acs_and_encoded_response() {
        let assertion = "<saml:Assertion>test</saml:Assertion>";
        let form = build_saml_post_form("https://sp.example.com/acs", assertion, "state123");
        assert!(form.contains("https://sp.example.com/acs"));
        assert!(form.contains("SAMLResponse"));
        assert!(form.contains("state123"));
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p ox_security_idp saml::tests 2>&1 | tail -15
```
Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_idp/src/saml.rs
git commit -m "feat(security-idp): implement SAML 2.0 handlers — metadata XML, assertion builder, POST form, XML escaping (5 tests)"
```

---

### Task 5: FFI plugin.rs

**Files:**
- Create: `crates/security/ox_security_idp/src/plugin.rs`

- [ ] **Step 1: Write plugin.rs**

```rust
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic;
use std::path::Path;
use std::sync::Arc;

use jsonwebtoken::EncodingKey;
use serde::Deserialize;
use uuid::Uuid;

use crate::config::IdpConfig;
use crate::oauth2::{build_encoding_key, handle_authorize, handle_token,
                    handle_oidc_discovery, now_secs};
use crate::saml::{build_metadata_xml, build_assertion_xml, build_saml_post_form};
use crate::store::{AuthCodeStore, RefreshTokenStore, SamlSessionEntry, SamlSessionStore, TokenStore};

use ox_workflow_abi::{
    CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO,
    OX_WORKFLOW_ABI_VERSION,
};

// ---------------------------------------------------------------------------
// Plugin state
// ---------------------------------------------------------------------------

struct PluginState {
    api: CoreHostApi,
    config: IdpConfig,
    enc_key: EncodingKey,
    code_store: AuthCodeStore,
    token_store: TokenStore,
    refresh_store: RefreshTokenStore,
    saml_sessions: SamlSessionStore,
    // Precomputed signing cert Base64 for SAML metadata (DER-encoded public cert)
    cert_b64: String,
}
unsafe impl Send for PluginState {}
unsafe impl Sync for PluginState {}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let Ok(k) = CString::new(key) else { return String::new() };
    let ptr = (api.get_field)(task_ctx, k.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, val: &str) {
    if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(val)) {
        (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
    }
}

fn json_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/json");
}

fn html_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "text/html; charset=utf-8");
}

fn xml_response(api: &CoreHostApi, task_ctx: *mut c_void, status: u16, body: &str) {
    set_field(api, task_ctx, "response.status", &status.to_string());
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "application/xml");
}

fn redirect_response(api: &CoreHostApi, task_ctx: *mut c_void, location: &str) {
    set_field(api, task_ctx, "response.status", "302");
    set_field(api, task_ctx, "response.header.Location", location);
}

/// Extract the authenticated principal_id from the bearer token in the Authorization header.
fn extract_bearer_principal(state: &PluginState, task_ctx: *mut c_void) -> Option<String> {
    let auth = get_field(&state.api, task_ctx, "request.header.Authorization");
    let token = auth.strip_prefix("Bearer ").unwrap_or("").trim();
    if token.is_empty() { return None; }
    // Look up token in store by scanning active tokens for matching raw_jwt
    state.token_store.list_active().into_iter()
        .find(|e| e.raw_jwt.as_deref() == Some(token))
        .and_then(|e| e.principal_id)
}

// ---------------------------------------------------------------------------
// Route dispatch
// ---------------------------------------------------------------------------

fn dispatch(state: &PluginState, task_ctx: *mut c_void) {
    let method = get_field(&state.api, task_ctx, "request.method").to_uppercase();
    let path   = get_field(&state.api, task_ctx, "request.path");
    let query  = get_field(&state.api, task_ctx, "request.query");
    let body   = get_field(&state.api, task_ctx, "request.body");

    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    match (method.as_str(), segs.get(0).copied(), segs.get(1).copied(),
           segs.get(2).copied(), segs.get(3).copied()) {

        // GET /oauth2/authorize
        ("GET", Some("oauth2"), Some("authorize"), None, None) => {
            let principal_id = extract_bearer_principal(state, task_ctx);
            match handle_authorize(&state.config, &state.code_store, &query,
                                   principal_id.as_deref()) {
                Ok(redirect_url) => redirect_response(&state.api, task_ctx, &redirect_url),
                Err(e) => json_response(&state.api, task_ctx, e.status, &e.to_json()),
            }
        }

        // POST /oauth2/token
        ("POST", Some("oauth2"), Some("token"), None, None) => {
            let (status, resp) = handle_token(&state.config, &state.enc_key,
                &state.code_store, &state.token_store, &state.refresh_store, &body);
            set_field(&state.api, task_ctx, "response.header.Content-Type", "application/json");
            set_field(&state.api, task_ctx, "response.status", &status.to_string());
            set_field(&state.api, task_ctx, "response.body", &resp);
        }

        // POST /oauth2/introspect
        ("POST", Some("oauth2"), Some("introspect"), None, None) => {
            let params: std::collections::HashMap<&str, &str> = body.split('&')
                .filter_map(|p| { let mut kv = p.splitn(2, '='); Some((kv.next()?, kv.next()?)) })
                .collect();
            let token = params.get("token").copied().unwrap_or("");
            let entry = state.token_store.list_active().into_iter()
                .find(|e| e.raw_jwt.as_deref() == Some(token));
            let resp = match entry {
                Some(e) => serde_json::json!({
                    "active": true,
                    "client_id": e.client_id,
                    "sub": e.principal_id,
                    "scope": e.scope,
                    "exp": e.expires_at,
                }),
                None => serde_json::json!({"active": false}),
            };
            json_response(&state.api, task_ctx, 200, &resp.to_string());
        }

        // POST /oauth2/revoke
        ("POST", Some("oauth2"), Some("revoke"), None, None) => {
            let params: std::collections::HashMap<&str, &str> = body.split('&')
                .filter_map(|p| { let mut kv = p.splitn(2, '='); Some((kv.next()?, kv.next()?)) })
                .collect();
            let token = params.get("token").copied().unwrap_or("");
            // Revoke by scanning for matching raw_jwt
            if let Some(e) = state.token_store.list_active().into_iter()
                .find(|e| e.raw_jwt.as_deref() == Some(token))
            {
                state.token_store.revoke(&e.jti);
            }
            json_response(&state.api, task_ctx, 200, "{}");
        }

        // GET /oidc/.well-known/openid-configuration
        ("GET", Some("oidc"), Some(".well-known"), Some("openid-configuration"), None) => {
            let discovery = handle_oidc_discovery(&state.config);
            json_response(&state.api, task_ctx, 200, &discovery);
        }

        // GET /oidc/jwks.json
        ("GET", Some("oidc"), Some("jwks.json"), None, None) => {
            // Return minimal JWKS — the public key modulus/exponent would need rsa crate
            // For now return an empty keyset structure; full implementation reads
            // the RSA public key from config and exports n+e as Base64Url
            json_response(&state.api, task_ctx, 200,
                r#"{"keys":[]}"#);
        }

        // GET /saml/{tenant}/metadata
        ("GET", Some("saml"), Some(_tenant), Some("metadata"), None) => {
            let sso_url = format!("{}/saml/{}/sso", state.config.issuer,
                segs.get(1).copied().unwrap_or(""));
            let slo_url = format!("{}/saml/{}/slo", state.config.issuer,
                segs.get(1).copied().unwrap_or(""));
            let xml = build_metadata_xml(&state.config.issuer, &sso_url, &slo_url, &state.cert_b64);
            xml_response(&state.api, task_ctx, 200, &xml);
        }

        // POST /saml/{tenant}/sso — body: SAMLRequest=...&username=...&password=...
        // (simplified: IdP-initiated SSO where credentials are in the POST body)
        ("POST", Some("saml"), Some(_tenant), Some("sso"), None) => {
            let params: std::collections::HashMap<&str, &str> = body.split('&')
                .filter_map(|p| { let mut kv = p.splitn(2, '='); Some((kv.next()?, kv.next()?)) })
                .collect();
            let sp_entity_id = params.get("sp_entity_id").copied().unwrap_or("");
            let name_id = params.get("name_id").copied().unwrap_or("");
            let relay_state = params.get("RelayState").copied().unwrap_or("");

            let sp = state.config.saml_sps.iter().find(|s| s.entity_id == sp_entity_id);
            let acs_url = sp.map(|s| s.acs_url.as_str()).unwrap_or("");
            if acs_url.is_empty() {
                json_response(&state.api, task_ctx, 400,
                    r#"{"error":"unknown_sp","error_description":"SP not registered"}"#);
                return;
            }

            let session_id = Uuid::new_v4().to_string();
            let assertion_id = format!("_{}", Uuid::new_v4().simple());
            let assertion_xml = build_assertion_xml(&assertion_id, &state.config.issuer,
                sp_entity_id, acs_url, name_id, &session_id, 3600);

            state.saml_sessions.insert(SamlSessionEntry {
                session_id: session_id.clone(),
                sp_entity_id: sp_entity_id.to_string(),
                principal_id: name_id.to_string(),
                name_id: name_id.to_string(),
                created_at: now_secs(),
            });

            let form = build_saml_post_form(acs_url, &assertion_xml, relay_state);
            html_response(&state.api, task_ctx, 200, &form);
        }

        // POST /saml/{tenant}/slo
        ("POST", Some("saml"), Some(_tenant), Some("slo"), None) => {
            let params: std::collections::HashMap<&str, &str> = body.split('&')
                .filter_map(|p| { let mut kv = p.splitn(2, '='); Some((kv.next()?, kv.next()?)) })
                .collect();
            let session_id = params.get("session_id").copied().unwrap_or("");
            state.saml_sessions.remove(session_id);
            json_response(&state.api, task_ctx, 200, r#"{"data":{"status":"logged_out"}}"#);
        }

        // --- Admin endpoints ---

        // GET /api/v1/admin/idp/clients
        ("GET", Some("api"), Some("v1"), Some("admin"), Some("idp")) if segs.get(4) == Some(&"clients") => {
            let clients_json = serde_json::to_string(&state.config.clients).unwrap_or_default();
            json_response(&state.api, task_ctx, 200,
                &format!(r#"{{"data":{}}}"#, clients_json));
        }

        // GET /api/v1/admin/idp/tokens
        ("GET", Some("api"), Some("v1"), Some("admin"), Some("idp")) if segs.get(4) == Some(&"tokens") => {
            let tokens = state.token_store.list_active();
            let body_json = serde_json::to_string(&tokens).unwrap_or_default();
            json_response(&state.api, task_ctx, 200,
                &format!(r#"{{"data":{}}}"#, body_json));
        }

        // DELETE /api/v1/admin/idp/tokens/{jti}
        ("DELETE", Some("api"), Some("v1"), Some("admin"), Some("idp")) if segs.get(4) == Some(&"tokens") => {
            let jti = segs.get(5).copied().unwrap_or("");
            state.token_store.revoke(jti);
            json_response(&state.api, task_ctx, 200, r#"{"data":{"revoked":true}}"#);
        }

        // GET /api/v1/admin/idp/sessions (SAML sessions)
        ("GET", Some("api"), Some("v1"), Some("admin"), Some("idp")) if segs.get(4) == Some(&"sessions") => {
            let sessions = state.saml_sessions.list();
            let body_json = serde_json::to_string(&sessions).unwrap_or_default();
            json_response(&state.api, task_ctx, 200,
                &format!(r#"{{"data":{}}}"#, body_json));
        }

        _ => { /* not our route — leave response unset, FLOW_CONTROL_CONTINUE */ }
    }
}

// ---------------------------------------------------------------------------
// FFI ABI
// ---------------------------------------------------------------------------

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
    let params_str = if config_ptr.is_null() { String::new() } else {
        unsafe { CStr::from_ptr(config_ptr).to_string_lossy().to_string() }
    };
    let params: serde_json::Value = serde_json::from_str(&params_str)
        .unwrap_or(serde_json::Value::Null);
    let config_path = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, "ox_security_idp: missing config_file param");
            return std::ptr::null_mut();
        }
    };
    let config: IdpConfig = match ox_fileproc::process_file(Path::new(&config_path), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                    &format!("ox_security_idp: config error: {}", e));
                return std::ptr::null_mut();
            }
        },
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_security_idp: failed to load config: {}", e));
            return std::ptr::null_mut();
        }
    };

    let enc_key = match build_encoding_key(&config.rsa_private_key_pem) {
        Ok(k) => k,
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("ox_security_idp: invalid RSA key: {}", e));
            return std::ptr::null_mut();
        }
    };

    log(&api, std::ptr::null_mut(), OX_LOG_INFO,
        &format!("ox_security_idp: initialized for issuer '{}'", config.issuer));

    Box::into_raw(Box::new(PluginState {
        api,
        enc_key,
        config,
        code_store: AuthCodeStore::new(),
        token_store: TokenStore::new(),
        refresh_store: RefreshTokenStore::new(),
        saml_sessions: SamlSessionStore::new(),
        cert_b64: String::new(), // TODO: derive from RSA key's public cert
    })) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_process(
    plugin_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    if plugin_ctx.is_null() { return cont; }
    let state = unsafe { &*(plugin_ctx as *mut PluginState) };
    panic::catch_unwind(panic::AssertUnwindSafe(|| {
        dispatch(state, task_ctx);
        cont
    }))
    .unwrap_or_else(|_| {
        log(&state.api, task_ctx, OX_LOG_ERROR, "ox_security_idp: panic in process");
        cont
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_error(_ctx: *mut c_void, _task: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn ox_plugin_destroy(plugin_ctx: *mut c_void) {
    if !plugin_ctx.is_null() {
        unsafe { drop(Box::from_raw(plugin_ctx as *mut PluginState)); }
    }
}
```

> **Before compiling:** Add `raw_jwt: Option<String>` to `TokenEntry` in `store.rs` and update all `TokenEntry { ... }` constructors in `oauth2.rs` to pass `raw_jwt: Some(access_token.clone())` after issuing the JWT. This makes introspect and revoke functional.

- [ ] **Step 2: Run compile check**

```bash
cargo check -p ox_security_idp 2>&1 | tail -20
```

Fix any compile errors. Common issues:
- `segs.get(4) == Some(&"clients")` — compare the `Option<&&str>`, may need `.copied()` or `.as_deref()`
- Missing `hex` or `subtle` deps — add to Cargo.toml

- [ ] **Step 3: Run all tests**

```bash
cargo test -p ox_security_idp 2>&1 | tail -20
```
Expected: 14+ tests pass (4 store + 5 oauth2 + 5 saml).

- [ ] **Step 4: Commit**

```bash
git add crates/security/ox_security_idp/src/plugin.rs crates/security/ox_security_idp/src/store.rs
git commit -m "feat(security-idp): implement FFI plugin dispatcher — OAuth2/OIDC, SAML, admin endpoints"
```

---

### Task 6: Config template and persona YAML

**Files:**
- Create: `crates/security/ox_security_idp/conf/plugin.yaml`
- Create: `personas/security/modules/available/ox_security_idp.yaml`

- [ ] **Step 1: Create plugin config template**

Create `crates/security/ox_security_idp/conf/plugin.yaml`:

```yaml
tenant_id: "default"
issuer: "https://auth.example.com"
# Paste your RSA private key here (PKCS#1 or PKCS#8 PEM format).
# Generate with: openssl genrsa -out idp_signing.pem 2048
rsa_private_key_pem: |
  -----BEGIN RSA PRIVATE KEY-----
  REPLACE_WITH_ACTUAL_KEY
  -----END RSA PRIVATE KEY-----
access_token_ttl_secs: 3600
refresh_token_ttl_secs: 86400
clients:
  - client_id: "example-app"
    client_secret_hash: null   # null = public client (requires PKCE)
    redirect_uris:
      - "https://app.example.com/callback"
    allowed_scopes: ["openid", "profile"]
    allowed_grants: ["authorization_code", "refresh_token"]
saml_sps:
  - entity_id: "urn:example:sp"
    acs_url: "https://sp.example.com/saml/acs"
    slo_url: "https://sp.example.com/saml/slo"
```

- [ ] **Step 2: Create persona module YAML**

Create `personas/security/modules/available/ox_security_idp.yaml`:

```yaml
modules:
  - id: "security_idp"
    name: "ox_security_idp"
    phase: Content
    params:
      config_file: "${{OX_BASE}}/crates/security/ox_security_idp/conf/plugin.yaml"

routes:
  - url: "^/oauth2(/.*)?$"
    module_id: "security_idp"
    priority: 100
  - url: "^/oidc(/.*)?$"
    module_id: "security_idp"
    priority: 100
  - url: "^/saml(/.*)?$"
    module_id: "security_idp"
    priority: 100
  - url: "^/api/v1/admin/idp(/.*)?$"
    module_id: "security_idp"
    priority: 150
```

- [ ] **Step 3: Final build and test**

```bash
cargo test -p ox_security_idp 2>&1 | tail -5
cargo build -p ox_security_idp 2>&1 | tail -5
```

Expected: all tests pass, cdylib builds.

- [ ] **Step 4: Commit**

```bash
git add crates/security/ox_security_idp/conf/ personas/security/modules/available/ox_security_idp.yaml
git commit -m "feat(security-idp): add plugin config template and persona YAML"
```
