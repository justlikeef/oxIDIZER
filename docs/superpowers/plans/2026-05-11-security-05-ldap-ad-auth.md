# LDAP and Active Directory Auth Drivers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `LdapAuthDriver` and `AdAuthDriver` stubs (which return `AuthResult::Continue` unconditionally) with real network-backed implementations. `LdapAuthDriver` performs a simple LDAP bind and group-membership lookup via `memberOf`. `AdAuthDriver` extends it by trying three username formats (`username`, `DOMAIN\username`, `user@upn_suffix`) before giving up.

**Architecture:** Both drivers hold a `connect_fn: LdapConnFactory` — an `Arc<dyn Fn(&str) -> BoxFuture<...>>` that defaults to a real `ldap3` connection but can be overridden in tests with a mock. This matches the `DbAuthDriver` pattern of injecting the external dependency rather than hard-wiring it. `AdAuthDriver` wraps an `LdapAuthDriver` internally and calls its bind logic with each candidate DN in turn.

**Tech Stack:** Rust, `ldap3` 0.11 (async, `tls-native` feature), `tokio` 1 (runtime + net), `futures` (for `BoxFuture`), `secrecy` (already present), `ox_security_core` (all shared types).

---

## File Structure

```
crates/security/ox_security_auth/
  Cargo.toml                          — add ldap3, tokio (net), futures
  src/
    drivers/
      ldap.rs                         — REPLACE stub: LdapConfig, LdapConnFactory, LdapAuthDriver
      ad.rs                           — REPLACE stub: AdConfig, AdAuthDriver
  tests/
    integration.rs                    — APPEND 6 new #[tokio::test] functions
```

All paths below are relative to the repository root (`/var/repos/oxIDIZER`).

---

## Background: ldap3 API used in this plan

```rust
// Connecting
use ldap3::{LdapConnAsync, LdapError};
let (conn, mut ldap) = LdapConnAsync::new(url).await?;
ldap3::drive!(conn);   // spawns connection driver task

// Simple bind
ldap.simple_bind(bind_dn, password).await?.success()?;

// Search
use ldap3::Scope;
let (entries, _res) = ldap
    .search(base_dn, Scope::Subtree, &format!("(uid={})", username), vec![group_attr])
    .await?.success()?;
for entry in entries {
    let entry = ldap3::SearchEntry::construct(entry);
    for val in entry.attrs.get(group_attr).unwrap_or(&vec![]) {
        groups.push(GroupId::new(val));
    }
}
ldap.unbind().await?;
```

The `connect_fn` abstraction hides exactly the `LdapConnAsync::new` call, so tests inject a mock. The rest of the bind/search logic remains in the driver and is exercised by tests.

---

## Task 1: Update Cargo.toml

**Files:**
- Modify: `crates/security/ox_security_auth/Cargo.toml`

- [ ] **Step 1: Open and edit `Cargo.toml`**

Replace the existing `[dependencies]` block:

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
ldap3            = { version = "0.11", default-features = false, features = ["tls-native"] }
futures          = "0.3"

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt", "net"] }
```

`futures` provides `BoxFuture` and `FutureExt`. `tokio` in dev-dependencies already has `macros` and `rt`; add `net` for any socket utilities used by mock helpers.

- [ ] **Step 2: Verify it compiles**

```bash
cargo build -p ox_security_auth 2>&1 | grep "^error" | head -10
```

Expected: zero errors (stubs still compile, new deps resolve).

- [ ] **Step 3: Commit**

```bash
git add crates/security/ox_security_auth/Cargo.toml
git commit -m "feat(security-ldap): add ldap3 and futures dependencies to ox_security_auth"
```

---

## Task 2: Implement LdapAuthDriver

**Files:**
- Modify: `crates/security/ox_security_auth/src/drivers/ldap.rs`
- Modify: `crates/security/ox_security_auth/tests/integration.rs`

### Design

`LdapAuthDriver` holds:
- `config: LdapConfig` — all static LDAP parameters
- `connect_fn: LdapConnFactory` — async factory; defaults to real ldap3; replaced in tests

`LdapConnFactory` wraps `ldap3::Ldap` behind a thin adapter trait so tests can return canned responses without a live server.

### Step 1: Write failing tests

- [ ] **APPEND to `crates/security/ox_security_auth/tests/integration.rs`:**

```rust
// ── LDAP / AD tests ────────────────────────────────────────────────────────
use ox_security_auth::{LdapAuthDriver, LdapConfig, AdAuthDriver, AdConfig};
use ox_security_core::{GroupId, TenantId};
use ox_security_auth::drivers::ldap::{LdapBindResult, MockLdapAdapter};
use std::str::FromStr;

fn ldap_config() -> LdapConfig {
    LdapConfig {
        url: "ldap://localhost:389".to_string(),
        bind_dn_template: "uid={},ou=users,dc=example,dc=com".to_string(),
        base_dn: "dc=example,dc=com".to_string(),
        group_attr: "memberOf".to_string(),
        tenant_id: TenantId::from_str("test").unwrap(),
    }
}

// ── Test 1 ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ldap_driver_continues_for_non_password_creds() {
    let driver = LdapAuthDriver::with_mock(
        ldap_config(),
        MockLdapAdapter::new(LdapBindResult::Success { groups: vec![] }),
    );
    let creds = Credentials::ApiKey { key: "key123".to_string().into() };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Continue));
}

// ── Test 2 ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ldap_driver_authenticates_valid_user() {
    let groups = vec!["cn=admins,dc=example,dc=com".to_string()];
    let driver = LdapAuthDriver::with_mock(
        ldap_config(),
        MockLdapAdapter::new(LdapBindResult::Success { groups: groups.clone() }),
    );
    let creds = Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: "correct".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    match result {
        AuthResult::Authenticated(p) => {
            assert_eq!(p.display_name, "alice");
            assert_eq!(p.groups, vec![GroupId::new("cn=admins,dc=example,dc=com")]);
        }
        other => panic!("expected Authenticated, got {:?}", other),
    }
}

// ── Test 3 ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ldap_driver_rejects_bad_password() {
    let driver = LdapAuthDriver::with_mock(
        ldap_config(),
        MockLdapAdapter::new(LdapBindResult::InvalidCredentials),
    );
    let creds = Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: "wrong".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}

// ── Test 4 ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ldap_driver_rejects_unknown_user() {
    let driver = LdapAuthDriver::with_mock(
        ldap_config(),
        MockLdapAdapter::new(LdapBindResult::NoSuchEntry),
    );
    let creds = Credentials::UsernamePassword {
        username: "ghost".to_string(),
        password: "anything".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p ox_security_auth ldap 2>&1 | head -20
```

Expected: FAIL — `LdapConfig`, `LdapAuthDriver::with_mock`, `MockLdapAdapter`, `LdapBindResult` do not exist.

### Step 3: Implement `src/drivers/ldap.rs`

- [ ] **Replace the entire file with:**

```rust
use std::sync::Arc;
use async_trait::async_trait;
use futures::future::BoxFuture;
use secrecy::ExposeSecret;

use ox_security_core::{
    AuthResult, AuthPipelineContext, AuthSource, Credentials,
    GroupId, Principal, PrincipalId, TenantId,
    drivers::AuthDriver,
};

// ── Public config ──────────────────────────────────────────────────────────

/// Static configuration for an LDAP authentication driver.
#[derive(Clone)]
pub struct LdapConfig {
    /// Full LDAP URL, e.g. "ldap://ldap.example.com:389" or "ldaps://...".
    pub url: String,
    /// DN template; `{}` is replaced with the username at bind time.
    /// Example: "uid={},ou=users,dc=example,dc=com"
    pub bind_dn_template: String,
    /// Base DN for group-membership search.
    /// Example: "dc=example,dc=com"
    pub base_dn: String,
    /// LDAP attribute that holds group DNs.  Defaults to "memberOf".
    pub group_attr: String,
    /// Tenant this driver serves.
    pub tenant_id: TenantId,
}

// ── Adapter trait (real ldap3 / mock) ─────────────────────────────────────

/// Result returned by an LDAP bind attempt.
#[derive(Clone, Debug)]
pub enum LdapBindResult {
    /// Bind succeeded; `groups` contains the raw values of the `memberOf` attribute.
    Success { groups: Vec<String> },
    /// The password was wrong (LDAP resultCode 49 — invalidCredentials).
    InvalidCredentials,
    /// The DN did not exist in the directory (LDAP resultCode 32 — noSuchObject).
    NoSuchEntry,
    /// Any other LDAP or network error.
    Error(String),
}

/// Abstraction over an LDAP connection.  The real implementation calls
/// `ldap3::LdapConnAsync`; tests inject `MockLdapAdapter`.
pub trait LdapAdapter: Send + Sync + 'static {
    /// Attempt a simple bind for `bind_dn` / `password`, then search for group
    /// memberships using `base_dn` / `group_attr`.
    fn bind_and_search(
        &self,
        url: String,
        bind_dn: String,
        password: String,
        base_dn: String,
        group_attr: String,
    ) -> BoxFuture<'static, LdapBindResult>;
}

// ── Real ldap3 adapter ─────────────────────────────────────────────────────

/// Production adapter that calls `ldap3::LdapConnAsync`.
pub struct RealLdapAdapter;

impl LdapAdapter for RealLdapAdapter {
    fn bind_and_search(
        &self,
        url: String,
        bind_dn: String,
        password: String,
        base_dn: String,
        group_attr: String,
    ) -> BoxFuture<'static, LdapBindResult> {
        Box::pin(async move {
            use ldap3::{LdapConnAsync, Scope, SearchEntry};

            let (conn, mut ldap) = match LdapConnAsync::new(&url).await {
                Ok(pair) => pair,
                Err(e) => return LdapBindResult::Error(e.to_string()),
            };
            ldap3::drive!(conn);

            match ldap.simple_bind(&bind_dn, &password).await {
                Err(e) => return LdapBindResult::Error(e.to_string()),
                Ok(res) => {
                    use ldap3::result::LdapResultCode;
                    match res.rc {
                        0 => {} // success — fall through to group search
                        49 => return LdapBindResult::InvalidCredentials,
                        32 => return LdapBindResult::NoSuchEntry,
                        _ => {
                            return LdapBindResult::Error(format!(
                                "LDAP bind returned rc={}",
                                res.rc
                            ))
                        }
                    }
                }
            }

            // Bind succeeded — search for group memberships.
            let username_part = bind_dn
                .split(',')
                .next()
                .and_then(|rdn| rdn.split('=').nth(1))
                .unwrap_or("*");
            let filter = format!("(uid={})", username_part);
            let attrs = vec![group_attr.as_str()];

            let groups = match ldap
                .search(&base_dn, Scope::Subtree, &filter, attrs)
                .await
            {
                Err(e) => return LdapBindResult::Error(e.to_string()),
                Ok(res) => match res.success() {
                    Err(e) => return LdapBindResult::Error(e.to_string()),
                    Ok((entries, _)) => {
                        let mut groups = Vec::new();
                        for entry in entries {
                            let e = SearchEntry::construct(entry);
                            if let Some(vals) = e.attrs.get(&group_attr) {
                                for v in vals {
                                    groups.push(v.clone());
                                }
                            }
                        }
                        groups
                    }
                },
            };

            let _ = ldap.unbind().await;
            LdapBindResult::Success { groups }
        })
    }
}

// ── Mock adapter (test-only) ───────────────────────────────────────────────

/// Deterministic mock: always returns the canned `LdapBindResult` regardless of inputs.
/// Expose this type so integration tests can import it.
pub struct MockLdapAdapter {
    result: LdapBindResult,
}

impl MockLdapAdapter {
    pub fn new(result: LdapBindResult) -> Self {
        Self { result }
    }
}

impl LdapAdapter for MockLdapAdapter {
    fn bind_and_search(
        &self,
        _url: String,
        _bind_dn: String,
        _password: String,
        _base_dn: String,
        _group_attr: String,
    ) -> BoxFuture<'static, LdapBindResult> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

// ── LdapAuthDriver ─────────────────────────────────────────────────────────

pub struct LdapAuthDriver {
    config: LdapConfig,
    adapter: Arc<dyn LdapAdapter>,
}

impl LdapAuthDriver {
    /// Production constructor — uses the real ldap3 network adapter.
    pub fn new(config: LdapConfig) -> Self {
        Self {
            config,
            adapter: Arc::new(RealLdapAdapter),
        }
    }

    /// Test constructor — injects a mock adapter.
    pub fn with_mock(config: LdapConfig, mock: impl LdapAdapter) -> Self {
        Self {
            config,
            adapter: Arc::new(mock),
        }
    }

    /// Core bind logic, shared with AdAuthDriver.
    /// Accepts a fully-formed `bind_dn` rather than building it from the template,
    /// so AD can pass the three candidate forms directly.
    pub(crate) async fn try_bind(
        &self,
        bind_dn: &str,
        password: &str,
        display_name: &str,
        auth_source: AuthSource,
    ) -> AuthResult {
        let result = self
            .adapter
            .bind_and_search(
                self.config.url.clone(),
                bind_dn.to_string(),
                password.to_string(),
                self.config.base_dn.clone(),
                self.config.group_attr.clone(),
            )
            .await;

        match result {
            LdapBindResult::Success { groups } => {
                let group_ids: Vec<GroupId> = groups.into_iter().map(GroupId::new).collect();
                AuthResult::Authenticated(Principal {
                    id: PrincipalId::new(),
                    display_name: display_name.to_string(),
                    source: auth_source,
                    groups: group_ids,
                    tenant_id: self.config.tenant_id.clone(),
                    session_id: None,
                })
            }
            LdapBindResult::InvalidCredentials => {
                AuthResult::Reject(format!("invalid credentials for '{}'", display_name))
            }
            LdapBindResult::NoSuchEntry => {
                AuthResult::Reject(format!("user '{}' not found in directory", display_name))
            }
            LdapBindResult::Error(msg) => {
                AuthResult::Reject(format!("LDAP error for '{}': {}", display_name, msg))
            }
        }
    }
}

#[async_trait]
impl AuthDriver for LdapAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let (username, password) = match credentials {
            Credentials::UsernamePassword { username, password } => {
                (username.as_str(), password.expose_secret())
            }
            _ => return AuthResult::Continue,
        };

        let bind_dn = self.config.bind_dn_template.replace("{}", username);
        self.try_bind(&bind_dn, password, username, AuthSource::Ldap).await
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p ox_security_auth ldap 2>&1 | tail -15
```

Expected: 4 new LDAP tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_auth/src/drivers/ldap.rs \
        crates/security/ox_security_auth/tests/integration.rs
git commit -m "feat(security-ldap): implement LdapAuthDriver with bind/memberOf search and mock adapter"
```

---

## Task 3: Implement AdAuthDriver

**Files:**
- Modify: `crates/security/ox_security_auth/src/drivers/ad.rs`
- Modify: `crates/security/ox_security_auth/tests/integration.rs`

### Design

`AdAuthDriver` wraps an `LdapAuthDriver` (sharing its adapter and config). On each authenticate call it tries three candidate bind DNs in order:

1. `uid=<username>,ou=users,...` — plain username via `bind_dn_template`
2. `DOMAIN\username` — sAMAccountName / pre-Windows 2000 format
3. `username@upn_suffix` — UPN format

The first call that does not return `Reject` (i.e., returns `Authenticated` or a non-reject) wins. If all three are rejected the driver returns `Reject`.

For the mock adapter the test controls which form produces success by setting the mock's `result`; the AD driver's sequential-try logic is verified by checking that `Authenticated` arrives and that the `display_name` reflects the username form that succeeded.

### Step 1: Write failing tests

- [ ] **APPEND to `crates/security/ox_security_auth/tests/integration.rs`:**

```rust
// ── AD-specific tests ──────────────────────────────────────────────────────

fn ad_config() -> AdConfig {
    AdConfig {
        ldap: ldap_config(),
        domain: "EXAMPLE".to_string(),
        upn_suffix: "example.com".to_string(),
    }
}

// ── Test 5 ─────────────────────────────────────────────────────────────────

/// Verifies that the AD driver tries the UPN form (user@domain) and succeeds.
#[tokio::test]
async fn ad_driver_tries_upn_format() {
    // MockLdapAdapter::SuccessOnUpn only returns Success when the bind_dn ends with "@example.com"
    use ox_security_auth::drivers::ad::BindDnCapture;

    let capture = BindDnCapture::new(LdapBindResult::Success {
        groups: vec!["cn=users,dc=example,dc=com".to_string()],
    });
    let driver = AdAuthDriver::with_mock(ad_config(), capture.clone());

    let creds = Credentials::UsernamePassword {
        username: "bob".to_string(),
        password: "pass".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;

    // Driver should have been called; last attempted DN recorded.
    let attempted = capture.last_bind_dn();
    assert!(
        attempted.iter().any(|dn| dn.ends_with("@example.com")),
        "expected a UPN attempt (user@example.com), got: {:?}",
        attempted
    );
    assert!(matches!(result, AuthResult::Authenticated(_)));
}

// ── Test 6 ─────────────────────────────────────────────────────────────────

/// Verifies that `DOMAIN\username` format is tried and, when it succeeds,
/// the driver returns Authenticated.
#[tokio::test]
async fn ad_driver_authenticates_via_domain_prefix() {
    use ox_security_auth::drivers::ad::BindDnCapture;

    // First attempt (plain uid=…) is InvalidCredentials.
    // Second attempt (DOMAIN\user) succeeds.
    let capture = BindDnCapture::new_sequence(vec![
        LdapBindResult::InvalidCredentials,
        LdapBindResult::Success { groups: vec![] },
    ]);
    let driver = AdAuthDriver::with_mock(ad_config(), capture.clone());

    let creds = Credentials::UsernamePassword {
        username: "carol".to_string(),
        password: "pass".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;

    let attempted = capture.last_bind_dn();
    assert!(
        attempted.iter().any(|dn| dn.starts_with("EXAMPLE\\")),
        "expected a DOMAIN\\username attempt, got: {:?}",
        attempted
    );
    assert!(matches!(result, AuthResult::Authenticated(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p ox_security_auth ad 2>&1 | head -20
```

Expected: FAIL — `AdConfig`, `AdAuthDriver::with_mock`, `BindDnCapture` do not exist.

### Step 3: Implement `src/drivers/ad.rs`

- [ ] **Replace the entire file with:**

```rust
use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use futures::future::BoxFuture;
use secrecy::ExposeSecret;

use ox_security_core::{
    AuthResult, AuthPipelineContext, AuthSource, Credentials,
    drivers::AuthDriver,
};

use crate::drivers::ldap::{LdapAdapter, LdapAuthDriver, LdapBindResult, LdapConfig};

// ── Public config ──────────────────────────────────────────────────────────

/// Configuration for Active Directory authentication.
/// Extends `LdapConfig` with AD-specific naming conventions.
#[derive(Clone)]
pub struct AdConfig {
    /// Underlying LDAP settings (URL, base DN, group attribute, tenant).
    pub ldap: LdapConfig,
    /// NetBIOS domain name, e.g. "EXAMPLE". Used to form "EXAMPLE\\username".
    pub domain: String,
    /// UPN suffix, e.g. "example.com". Used to form "username@example.com".
    pub upn_suffix: String,
}

// ── BindDnCapture (test helper, pub for integration tests) ─────────────────

/// A recording `LdapAdapter` used in tests.
///
/// It is constructed with either a single fixed `LdapBindResult` (returned for
/// every call) or a sequence (popped from the front on each call, last value
/// repeated after the sequence is exhausted).  Every `bind_dn` passed to
/// `bind_and_search` is appended to an internal log.
#[derive(Clone)]
pub struct BindDnCapture {
    sequence: Arc<Mutex<Vec<LdapBindResult>>>,
    log: Arc<Mutex<Vec<String>>>,
}

impl BindDnCapture {
    /// Always returns `result` for every bind attempt.
    pub fn new(result: LdapBindResult) -> Self {
        Self {
            sequence: Arc::new(Mutex::new(vec![result])),
            log: Arc::new(Mutex::new(vec![])),
        }
    }

    /// Returns items from `results` in order; repeats the last one once exhausted.
    pub fn new_sequence(results: Vec<LdapBindResult>) -> Self {
        assert!(!results.is_empty(), "BindDnCapture::new_sequence requires ≥1 result");
        Self {
            sequence: Arc::new(Mutex::new(results)),
            log: Arc::new(Mutex::new(vec![])),
        }
    }

    /// Returns all bind DNs that were attempted, in order.
    pub fn last_bind_dn(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl LdapAdapter for BindDnCapture {
    fn bind_and_search(
        &self,
        _url: String,
        bind_dn: String,
        _password: String,
        _base_dn: String,
        _group_attr: String,
    ) -> BoxFuture<'static, LdapBindResult> {
        self.log.lock().unwrap().push(bind_dn);
        let result = {
            let mut seq = self.sequence.lock().unwrap();
            if seq.len() > 1 {
                seq.remove(0)
            } else {
                seq[0].clone()
            }
        };
        Box::pin(async move { result })
    }
}

// ── AdAuthDriver ───────────────────────────────────────────────────────────

pub struct AdAuthDriver {
    config: AdConfig,
    inner: LdapAuthDriver,
}

impl AdAuthDriver {
    /// Production constructor.
    pub fn new(config: AdConfig) -> Self {
        let inner = LdapAuthDriver::new(config.ldap.clone());
        Self { config, inner }
    }

    /// Test constructor — injects a mock adapter into the inner LDAP driver.
    pub fn with_mock(config: AdConfig, mock: impl LdapAdapter) -> Self {
        let inner = LdapAuthDriver::with_mock(config.ldap.clone(), mock);
        Self { config, inner }
    }
}

#[async_trait]
impl AuthDriver for AdAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let (username, password) = match credentials {
            Credentials::UsernamePassword { username, password } => {
                (username.as_str(), password.expose_secret())
            }
            _ => return AuthResult::Continue,
        };

        // Candidate 1: plain uid=username (via bind_dn_template)
        let plain_dn = self.config.ldap.bind_dn_template.replace("{}", username);

        // Candidate 2: DOMAIN\username (sAMAccountName format)
        let netbios_dn = format!("{}\\{}", self.config.domain, username);

        // Candidate 3: username@upn_suffix (User Principal Name format)
        let upn_dn = format!("{}@{}", username, self.config.upn_suffix);

        let candidates: &[(&str, &str)] = &[
            (&plain_dn, username),
            (&netbios_dn, username),
            (&upn_dn, username),
        ];

        let mut last_reject: Option<AuthResult> = None;

        for (bind_dn, display) in candidates {
            let result = self
                .inner
                .try_bind(bind_dn, password, display, AuthSource::Ad)
                .await;

            match result {
                AuthResult::Authenticated(_) => return result,
                AuthResult::Reject(_) => {
                    last_reject = Some(result);
                    // Try next candidate.
                }
                // Continue or MfaRequired bubble up immediately.
                other => return other,
            }
        }

        last_reject.unwrap_or_else(|| {
            AuthResult::Reject(format!("AD: all bind forms rejected for '{}'", username))
        })
    }
}
```

- [ ] **Step 4: Expose `BindDnCapture` and `LdapAdapter` in `drivers/mod.rs`**

The integration tests import from `ox_security_auth::drivers::ad::BindDnCapture` and `ox_security_auth::drivers::ldap::LdapBindResult`. Verify that `drivers/mod.rs` already has `pub(crate)` for both modules and that the re-exports in `lib.rs` include both driver structs. No change needed if the existing `mod.rs` already reads:

```rust
pub(crate) mod ad;
pub(crate) mod ldap;
// ...
pub use ad::AdAuthDriver;
pub use ldap::LdapAuthDriver;
```

For the test imports to work (`ox_security_auth::drivers::ldap::LdapBindResult`) the modules must be `pub` rather than `pub(crate)`. Change `drivers/mod.rs`:

```rust
pub mod ad;
pub mod api_key;
pub mod db;
pub mod kerberos;
pub mod ldap;
pub mod radius;
pub mod tacacs;
pub mod totp;

pub use ad::AdAuthDriver;
pub use api_key::ApiKeyAuthDriver;
pub use db::DbAuthDriver;
pub use kerberos::KerberosAuthDriver;
pub use ldap::LdapAuthDriver;
pub use radius::RadiusAuthDriver;
pub use tacacs::TacacsAuthDriver;
pub use totp::TotpAuthDriver;
```

Also add `LdapConfig` and `AdConfig` to `lib.rs` re-exports:

```rust
pub mod drivers;
pub(crate) mod pipeline;

pub use pipeline::AuthPipeline;
pub use drivers::{
    AdAuthDriver, ApiKeyAuthDriver, DbAuthDriver,
    KerberosAuthDriver, LdapAuthDriver, RadiusAuthDriver,
    TacacsAuthDriver, TotpAuthDriver,
};
pub use drivers::ldap::{LdapConfig, LdapAuthDriver};
pub use drivers::ad::{AdConfig, AdAuthDriver};
```

- [ ] **Step 5: Run all tests to verify they pass**

```bash
cargo test -p ox_security_auth 2>&1 | tail -20
```

Expected: all prior tests still pass + 6 new LDAP/AD tests pass (10 total: 3 pipeline + 4 db + 4 ldap + 2 ad = 13 total — wait, 3 pipeline + 4 db = 7 existing, + 4 ldap + 2 ad = 6 new = 13 total).

- [ ] **Step 6: Commit**

```bash
git add crates/security/ox_security_auth/src/drivers/ad.rs \
        crates/security/ox_security_auth/src/drivers/ldap.rs \
        crates/security/ox_security_auth/src/drivers/mod.rs \
        crates/security/ox_security_auth/src/lib.rs \
        crates/security/ox_security_auth/tests/integration.rs
git commit -m "feat(security-ldap): implement AdAuthDriver with three-form bind (uid, DOMAIN\\user, UPN)"
```

---

## Task 4: Clean build verification

- [ ] **Step 1: Build the full security workspace**

```bash
cargo build -p ox_security_auth -p ox_security_core 2>&1 | grep "^error" | head -10
```

Expected: zero errors.

- [ ] **Step 2: Run full test suite**

```bash
cargo test -p ox_security_auth 2>&1 | grep -E "^test |FAILED|ok$|error\[" | tail -30
```

Expected: 13 tests, 0 failures.

- [ ] **Step 3: Clippy**

```bash
cargo clippy -p ox_security_auth -- -D warnings 2>&1 | grep "^error" | head -10
```

Expected: zero warnings promoted to errors.

- [ ] **Step 4: Final commit if any clippy fixes needed**

```bash
git add crates/security/ox_security_auth
git commit -m "fix(security-ldap): address clippy warnings in ldap/ad drivers"
```

---

## Summary of new public API

| Symbol | Crate path | Purpose |
|---|---|---|
| `LdapConfig` | `ox_security_auth::LdapConfig` | Config for LDAP driver |
| `LdapAuthDriver::new(config)` | `ox_security_auth::LdapAuthDriver` | Production LDAP driver |
| `LdapAuthDriver::with_mock(config, mock)` | `ox_security_auth::LdapAuthDriver` | Test constructor |
| `LdapAdapter` | `ox_security_auth::drivers::ldap::LdapAdapter` | Trait for bind+search |
| `LdapBindResult` | `ox_security_auth::drivers::ldap::LdapBindResult` | Enum of bind outcomes |
| `MockLdapAdapter` | `ox_security_auth::drivers::ldap::MockLdapAdapter` | Fixed-result mock |
| `AdConfig` | `ox_security_auth::AdConfig` | Config for AD driver |
| `AdAuthDriver::new(config)` | `ox_security_auth::AdAuthDriver` | Production AD driver |
| `AdAuthDriver::with_mock(config, mock)` | `ox_security_auth::AdAuthDriver` | Test constructor |
| `BindDnCapture` | `ox_security_auth::drivers::ad::BindDnCapture` | Recording mock for AD tests |

All existing `AuthDriver` pipeline wiring is unchanged; drop-in replacement for the stubs.
