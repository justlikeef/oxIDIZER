# ox_security_auth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `ox_security_auth` crate — the authentication pipeline that chains multiple `AuthDriver` implementations, handles MFA orchestration, and manages session lifecycle.

**Architecture:** A stack of `AuthDriver` trait objects is iterated on each authentication attempt. Each driver returns `Authenticated`, `MfaRequired`, `Continue`, or `Reject`. The pipeline holds the `AuthPipelineContext` across the chain. `DbAuthDriver` is the only concrete driver provided (local credential store via a pluggable lookup function). All other drivers (LDAP, AD, Kerberos, etc.) are trait-compatible stubs that return `Continue`, ready for full implementation when the corresponding data crate drivers are available.

**Tech Stack:** Rust, `ox_security_core` (all shared types), `async-trait`, `tokio` (dev-dependency)

---

## File Structure

```
crates/security/ox_security_auth/
  Cargo.toml
  src/
    lib.rs               — pub mod declarations + re-exports
    pipeline.rs          — AuthPipeline struct: stack of Arc<dyn AuthDriver>, authenticate()
    drivers/
      mod.rs             — pub use of each driver
      db.rs              — DbAuthDriver (local credential lookup via injected fn)
      ldap.rs            — LdapAuthDriver stub (returns Continue)
      ad.rs              — AdAuthDriver stub (returns Continue)
      kerberos.rs        — KerberosAuthDriver stub (returns Continue)
      radius.rs          — RadiusAuthDriver stub (returns Continue)
      tacacs.rs          — TacacsAuthDriver stub (returns Continue)
      totp.rs            — TotpAuthDriver stub (returns Continue)
      api_key.rs         — ApiKeyAuthDriver stub (returns Continue)
  tests/
    integration.rs       — pipeline tests, DbAuthDriver tests, MFA flow test
```

---

## Task 1: Crate scaffold + AuthPipeline skeleton

**Files:**
- Create: `crates/security/ox_security_auth/Cargo.toml`
- Create: `crates/security/ox_security_auth/src/lib.rs`
- Create: `crates/security/ox_security_auth/src/pipeline.rs`
- Create: `crates/security/ox_security_auth/tests/integration.rs`
- Modify: `Cargo.toml` (workspace root) — add crate to members

- [ ] **Step 1: Write the failing tests**

Create `crates/security/ox_security_auth/tests/integration.rs`:

```rust
use ox_security_auth::pipeline::AuthPipeline;
use ox_security_core::{
    Credentials, AuthResult, Principal, AuthSource, PrincipalId, TenantId, GroupId,
    AuthPipelineContext,
};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::drivers::AuthDriver;

struct AlwaysRejectDriver;

#[async_trait]
impl AuthDriver for AlwaysRejectDriver {
    async fn authenticate(&self, _creds: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Reject("rejected".to_string())
    }
}

struct AlwaysContinueDriver;

#[async_trait]
impl AuthDriver for AlwaysContinueDriver {
    async fn authenticate(&self, _creds: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Continue
    }
}

fn test_principal() -> Principal {
    Principal {
        id: PrincipalId::new(),
        display_name: "Test User".to_string(),
        source: AuthSource::Local,
        groups: vec![],
        tenant_id: TenantId::from_str("test").unwrap(),
        session_id: None,
    }
}

fn test_ctx() -> AuthPipelineContext {
    AuthPipelineContext {
        partial_principal: None,
        tenant_id: TenantId::from_str("test").unwrap(),
        source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    }
}

#[tokio::test]
async fn empty_pipeline_rejects() {
    let pipeline = AuthPipeline::new(vec![]);
    let creds = Credentials::UsernamePassword {
        username: "user".to_string(),
        password: "pass".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = pipeline.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}

#[tokio::test]
async fn pipeline_stops_at_reject() {
    let pipeline = AuthPipeline::new(vec![
        Arc::new(AlwaysRejectDriver),
        Arc::new(AlwaysContinueDriver),
    ]);
    let creds = Credentials::UsernamePassword {
        username: "u".to_string(),
        password: "p".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = pipeline.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}

#[tokio::test]
async fn pipeline_continues_through_misses() {
    use ox_security_core::SessionId;

    struct AlwaysAuthDriver;
    #[async_trait]
    impl AuthDriver for AlwaysAuthDriver {
        async fn authenticate(&self, _c: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
            AuthResult::Authenticated(Principal {
                id: PrincipalId::new(),
                display_name: "auto".to_string(),
                source: AuthSource::Local,
                groups: vec![],
                tenant_id: TenantId::from_str("test").unwrap(),
                session_id: None,
            })
        }
    }

    let pipeline = AuthPipeline::new(vec![
        Arc::new(AlwaysContinueDriver),
        Arc::new(AlwaysContinueDriver),
        Arc::new(AlwaysAuthDriver),
    ]);
    let creds = Credentials::UsernamePassword {
        username: "u".to_string(),
        password: "p".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = pipeline.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Authenticated(_)));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_auth 2>&1 | head -10
```
Expected: FAIL — crate does not exist.

- [ ] **Step 3: Add to workspace**

Add to `[workspace] members` in `/var/repos/oxIDIZER/.worktrees/security-auth/Cargo.toml`:
```toml
"crates/security/ox_security_auth",
```

- [ ] **Step 4: Create `crates/security/ox_security_auth/Cargo.toml`**

```toml
[package]
name = "ox_security_auth"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]
ox_security_core = { path = "../ox_security_core" }
async-trait      = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt"] }
```

- [ ] **Step 5: Create `src/pipeline.rs`**

```rust
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::drivers::AuthDriver;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials};

pub struct AuthPipeline {
    drivers: Vec<Arc<dyn AuthDriver>>,
}

impl AuthPipeline {
    pub fn new(drivers: Vec<Arc<dyn AuthDriver>>) -> Self {
        Self { drivers }
    }

    pub async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        for driver in &self.drivers {
            match driver.authenticate(credentials, ctx).await {
                AuthResult::Continue => continue,
                result => return result,
            }
        }
        AuthResult::Reject("no driver handled the credentials".to_string())
    }
}
```

- [ ] **Step 6: Create `src/lib.rs`**

```rust
pub mod drivers;
pub mod pipeline;

pub use pipeline::AuthPipeline;
```

- [ ] **Step 7: Create `src/drivers/mod.rs` with all stub driver files**

Create `src/drivers/mod.rs`:
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

Create stub for each driver that returns `Continue`. Here is the pattern — repeat for `ad.rs`, `api_key.rs`, `kerberos.rs`, `ldap.rs`, `radius.rs`, `tacacs.rs`, `totp.rs` replacing the struct name:

`src/drivers/ldap.rs`:
```rust
use async_trait::async_trait;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, drivers::AuthDriver};

pub struct LdapAuthDriver;

#[async_trait]
impl AuthDriver for LdapAuthDriver {
    async fn authenticate(&self, _credentials: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Continue
    }
}
```

`src/drivers/ad.rs` — same pattern, `AdAuthDriver`
`src/drivers/kerberos.rs` — same pattern, `KerberosAuthDriver`
`src/drivers/radius.rs` — same pattern, `RadiusAuthDriver`
`src/drivers/tacacs.rs` — same pattern, `TacacsAuthDriver`
`src/drivers/totp.rs` — same pattern, `TotpAuthDriver`
`src/drivers/api_key.rs` — same pattern, `ApiKeyAuthDriver`

`src/drivers/db.rs` (placeholder struct only — Task 2 will implement it):
```rust
use async_trait::async_trait;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, drivers::AuthDriver};

pub struct DbAuthDriver;

#[async_trait]
impl AuthDriver for DbAuthDriver {
    async fn authenticate(&self, _credentials: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Continue
    }
}
```

- [ ] **Step 8: Run test to verify it passes**

```bash
cd /var/repos/oxIDIZER/.worktrees/security-auth
cargo test -p ox_security_auth 2>&1 | tail -10
```
Expected: 3 pipeline tests pass.

- [ ] **Step 9: Commit**

```bash
cd /var/repos/oxIDIZER/.worktrees/security-auth
git add crates/security/ox_security_auth Cargo.toml
git commit -m "feat(security-auth): scaffold AuthPipeline with chain execution and stub drivers"
```

---

## Task 2: DbAuthDriver — local credential lookup

**Files:**
- Modify: `crates/security/ox_security_auth/src/drivers/db.rs`
- Modify: `crates/security/ox_security_auth/tests/integration.rs`

The `DbAuthDriver` takes an injected lookup function rather than direct DB access, so it can be tested without a database. The function signature is:

```rust
type CredentialLookupFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
// Given a username, return the stored bcrypt/argon2 password hash, or None if user not found.
```

For password verification: compare using constant-time string comparison (since we cannot import bcrypt without adding a dependency, use a simple pluggable verifier instead):

```rust
type PasswordVerifierFn = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;
// Given (plaintext_password, stored_hash) -> bool
```

- [ ] **Step 1: Write the failing tests**

APPEND to `tests/integration.rs`:

```rust
use ox_security_auth::drivers::DbAuthDriver;

fn make_db_driver(username: &str, password: &str) -> DbAuthDriver {
    let u = username.to_string();
    let stored = password.to_string();
    DbAuthDriver::new(
        TenantId::from_str("test").unwrap(),
        Arc::new(move |user: &str| {
            if user == u { Some(stored.clone()) } else { None }
        }),
        Arc::new(|plaintext: &str, hash: &str| plaintext == hash),
    )
}

#[tokio::test]
async fn db_driver_authenticates_valid_user() {
    let driver = make_db_driver("alice", "secret");
    let creds = Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: "secret".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    match result {
        AuthResult::Authenticated(p) => assert_eq!(p.display_name, "alice"),
        other => panic!("expected Authenticated, got {:?}", other),
    }
}

#[tokio::test]
async fn db_driver_rejects_wrong_password() {
    let driver = make_db_driver("alice", "secret");
    let creds = Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: "wrong".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}

#[tokio::test]
async fn db_driver_continues_for_unknown_user() {
    let driver = make_db_driver("alice", "secret");
    let creds = Credentials::UsernamePassword {
        username: "nobody".to_string(),
        password: "anything".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Continue));
}

#[tokio::test]
async fn db_driver_continues_for_non_password_credentials() {
    let driver = make_db_driver("alice", "secret");
    let creds = Credentials::ApiKey { key: "somekey".to_string().into() };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Continue));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /var/repos/oxIDIZER/.worktrees/security-auth
cargo test -p ox_security_auth db 2>&1 | head -15
```
Expected: FAIL — DbAuthDriver doesn't have `new()` or fields.

- [ ] **Step 3: Implement `src/drivers/db.rs`**

```rust
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, TenantId,
    Principal, PrincipalId, AuthSource,
    drivers::AuthDriver,
};
use secrecy::ExposeSecret;

pub type CredentialLookupFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
pub type PasswordVerifierFn = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

pub struct DbAuthDriver {
    tenant_id: TenantId,
    lookup: CredentialLookupFn,
    verify: PasswordVerifierFn,
}

impl DbAuthDriver {
    pub fn new(
        tenant_id: TenantId,
        lookup: CredentialLookupFn,
        verify: PasswordVerifierFn,
    ) -> Self {
        Self { tenant_id, lookup, verify }
    }
}

#[async_trait]
impl AuthDriver for DbAuthDriver {
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

        match (self.lookup)(username) {
            None => AuthResult::Continue,
            Some(stored_hash) => {
                if (self.verify)(password, &stored_hash) {
                    AuthResult::Authenticated(Principal {
                        id: PrincipalId::new(),
                        display_name: username.to_string(),
                        source: AuthSource::Local,
                        groups: vec![],
                        tenant_id: self.tenant_id.clone(),
                        session_id: None,
                    })
                } else {
                    AuthResult::Reject(format!("invalid credentials for '{}'", username))
                }
            }
        }
    }
}
```

Note: `Credentials::UsernamePassword.password` is a `SecretString`. Use `secrecy::ExposeSecret` trait to access the plaintext. Add `secrecy` to `Cargo.toml`:

```toml
[dependencies]
ox_security_core = { path = "../ox_security_core" }
async-trait      = "0.1"
secrecy          = { version = "0.8", features = ["serde"] }
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /var/repos/oxIDIZER/.worktrees/security-auth
cargo test -p ox_security_auth 2>&1 | tail -10
```
Expected: 3 pipeline tests + 4 db tests = 7 total.

- [ ] **Step 5: Commit**

```bash
cd /var/repos/oxIDIZER/.worktrees/security-auth
git add crates/security/ox_security_auth
git commit -m "feat(security-auth): implement DbAuthDriver with pluggable lookup and verifier"
```

---

## Task 3: Wire lib.rs and verify clean build

**Files:**
- Modify: `crates/security/ox_security_auth/src/lib.rs`

- [ ] **Step 1: Update `src/lib.rs` with full re-exports**

```rust
pub mod drivers;
pub mod pipeline;

pub use pipeline::AuthPipeline;
pub use drivers::{
    AdAuthDriver, ApiKeyAuthDriver, DbAuthDriver,
    KerberosAuthDriver, LdapAuthDriver, RadiusAuthDriver,
    TacacsAuthDriver, TotpAuthDriver,
};
```

- [ ] **Step 2: Build and test**

```bash
cd /var/repos/oxIDIZER/.worktrees/security-auth
cargo build -p ox_security_auth 2>&1 | grep "^error" | head -5
cargo test -p ox_security_auth 2>&1 | tail -10
```
Expected: zero errors, 7 tests pass.

- [ ] **Step 3: Commit**

```bash
cd /var/repos/oxIDIZER/.worktrees/security-auth
git add crates/security/ox_security_auth
git commit -m "feat(security-auth): complete ox_security_auth — pipeline, DbAuthDriver, all driver stubs"
```
