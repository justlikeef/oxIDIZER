# Stateless Auth Drivers Implementation Plan (ApiKeyAuthDriver + MtlsAuthDriver)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement two stateless auth drivers in `ox_security_auth`: a production-quality `ApiKeyAuthDriver` (lookup-fn based) and a new `MtlsAuthDriver` (validator-fn based for DER-encoded client certificates). Both follow the existing pipeline conventions — returning `Continue` for unrecognised credential types so other drivers may handle them.

**Architecture:** Both drivers are injected with a pluggable function at construction time — no direct I/O, fully testable without mocks. `ApiKeyAuthDriver` replaces the current stub in `src/drivers/api_key.rs`. `MtlsAuthDriver` is added as a new file `src/drivers/mtls.rs`. Both are wired into `drivers/mod.rs` and `lib.rs`.

**Tech Stack:** Rust, `ox_security_core` (all shared types), `async-trait`, `secrecy` (already in `[dependencies]`), `tokio` (dev-dependency), `x509-cert = "0.2"` (dev-dependency for test cert generation only)

---

## File Structure (changes only)

```
crates/security/ox_security_auth/
  Cargo.toml                         — add x509-cert dev-dep
  src/
    lib.rs                           — add MtlsAuthDriver to re-exports
    drivers/
      mod.rs                         — add mtls module + re-export
      api_key.rs                     — REPLACE stub with full implementation
      mtls.rs                        — NEW file
```

---

## Task 1: ApiKeyAuthDriver — full implementation

**Files:**
- Modify: `crates/security/ox_security_auth/src/drivers/api_key.rs`
- Modify: `crates/security/ox_security_auth/src/lib.rs` (add re-export)

### Step 1: Write the failing tests

APPEND to `crates/security/ox_security_auth/src/drivers/api_key.rs` (or add a `#[cfg(test)]` module at the bottom):

```rust
// At the bottom of api_key.rs, after the impl block:

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use ox_security_core::{
        AuthResult, AuthPipelineContext, Credentials, Principal, PrincipalId,
        AuthSource, TenantId, GroupId,
    };
    use secrecy::SecretString;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    fn make_driver() -> ApiKeyAuthDriver {
        ApiKeyAuthDriver::new(Arc::new(|key: &str| {
            if key == "valid-api-key" {
                Some(Principal {
                    id: PrincipalId::new(),
                    display_name: "service-account".to_string(),
                    source: AuthSource::ApiKey,
                    groups: vec![],
                    tenant_id: TenantId::from_str("test").unwrap(),
                    session_id: None,
                })
            } else {
                None
            }
        }))
    }

    #[tokio::test]
    async fn api_key_continues_for_non_api_key_creds() {
        let driver = make_driver();
        let creds = Credentials::UsernamePassword {
            username: "user".to_string(),
            password: SecretString::from("pass"),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn api_key_authenticates_known_key() {
        let driver = make_driver();
        let creds = Credentials::ApiKey {
            key: SecretString::from("valid-api-key"),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => assert_eq!(p.display_name, "service-account"),
            other => panic!("expected Authenticated, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn api_key_continues_for_unknown_key() {
        let driver = make_driver();
        let creds = Credentials::ApiKey {
            key: SecretString::from("unknown-key"),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        // Unknown key => Continue, not Reject — let another driver try
        assert!(matches!(result, AuthResult::Continue));
    }
}
```

### Step 2: Run tests to verify they fail

```bash
cargo test -p ox_security_auth api_key 2>&1 | head -20
```

Expected: FAIL — `ApiKeyAuthDriver::new` does not exist yet; the struct has no fields.

### Step 3: Implement `src/drivers/api_key.rs`

Replace the entire file content with:

```rust
use std::sync::Arc;
use async_trait::async_trait;
use secrecy::ExposeSecret;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, Principal, drivers::AuthDriver};

/// Given the plaintext API key, return the associated Principal or None.
/// Returning None means "not my concern" — the pipeline continues to the next driver.
pub type ApiKeyLookupFn = Arc<dyn Fn(&str) -> Option<Principal> + Send + Sync>;

pub struct ApiKeyAuthDriver {
    lookup: ApiKeyLookupFn,
}

impl ApiKeyAuthDriver {
    pub fn new(lookup: ApiKeyLookupFn) -> Self {
        Self { lookup }
    }
}

#[async_trait]
impl AuthDriver for ApiKeyAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let key = match credentials {
            Credentials::ApiKey { key } => key.expose_secret(),
            _ => return AuthResult::Continue,
        };

        match (self.lookup)(key) {
            Some(principal) => AuthResult::Authenticated(principal),
            // Unknown key => Continue, not Reject; another driver may recognise it
            None => AuthResult::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use ox_security_core::{
        AuthResult, AuthPipelineContext, Credentials, Principal, PrincipalId,
        AuthSource, TenantId,
    };
    use secrecy::SecretString;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    fn make_driver() -> ApiKeyAuthDriver {
        ApiKeyAuthDriver::new(Arc::new(|key: &str| {
            if key == "valid-api-key" {
                Some(Principal {
                    id: PrincipalId::new(),
                    display_name: "service-account".to_string(),
                    source: AuthSource::ApiKey,
                    groups: vec![],
                    tenant_id: TenantId::from_str("test").unwrap(),
                    session_id: None,
                })
            } else {
                None
            }
        }))
    }

    #[tokio::test]
    async fn api_key_continues_for_non_api_key_creds() {
        let driver = make_driver();
        let creds = Credentials::UsernamePassword {
            username: "user".to_string(),
            password: SecretString::from("pass"),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn api_key_authenticates_known_key() {
        let driver = make_driver();
        let creds = Credentials::ApiKey {
            key: SecretString::from("valid-api-key"),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => assert_eq!(p.display_name, "service-account"),
            other => panic!("expected Authenticated, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn api_key_continues_for_unknown_key() {
        let driver = make_driver();
        let creds = Credentials::ApiKey {
            key: SecretString::from("unknown-key"),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }
}
```

### Step 4: Run tests to verify they pass

```bash
cargo test -p ox_security_auth api_key 2>&1 | tail -10
```

Expected: 3 tests pass — `api_key_continues_for_non_api_key_creds`, `api_key_authenticates_known_key`, `api_key_continues_for_unknown_key`.

### Step 5: Commit

```bash
git add crates/security/ox_security_auth/src/drivers/api_key.rs
git commit -m "feat(security-auth): implement ApiKeyAuthDriver with lookup-fn and 3 tests"
```

---

## Task 2: MtlsAuthDriver — DER certificate validation

**Files:**
- Create: `crates/security/ox_security_auth/src/drivers/mtls.rs`
- Modify: `crates/security/ox_security_auth/src/drivers/mod.rs`
- Modify: `crates/security/ox_security_auth/src/lib.rs`
- Modify: `crates/security/ox_security_auth/Cargo.toml`

### Step 1: Write the failing tests

Create `crates/security/ox_security_auth/src/drivers/mtls.rs` with tests only (no implementation yet):

```rust
// Placeholder — tests written first; struct body follows in Step 3

pub struct MtlsAuthDriver;

#[cfg(test)]
mod tests {
    // tests go here in Step 1; impl provided in Step 3
}
```

Actually, because the struct is needed to compile the test module, write the full file at once in Step 3. For the TDD step, write the tests in isolation as an inline mod inside the (stub) file:

```rust
use async_trait::async_trait;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, drivers::AuthDriver};

pub struct MtlsAuthDriver;

#[async_trait]
impl AuthDriver for MtlsAuthDriver {
    async fn authenticate(&self, _credentials: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use ox_security_core::{
        AuthResult, AuthPipelineContext, Credentials, Principal, PrincipalId,
        AuthSource, TenantId,
    };
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    fn make_driver(accept: bool) -> super::MtlsAuthDriver {
        // Will be replaced with MtlsAuthDriver::new(...) after Task 2 Step 3
        todo!()
    }

    #[tokio::test]
    async fn mtls_continues_for_non_cert_creds() {
        todo!("blocked on MtlsAuthDriver::new")
    }

    #[tokio::test]
    async fn mtls_authenticates_valid_cert() {
        todo!("blocked on MtlsAuthDriver::new")
    }

    #[tokio::test]
    async fn mtls_rejects_invalid_cert() {
        todo!("blocked on MtlsAuthDriver::new")
    }
}
```

Wire into `mod.rs` and `lib.rs` (Step 2) so the crate compiles, then replace the stub + tests in Step 3.

### Step 2: Wire `mtls` into the module tree

Modify `crates/security/ox_security_auth/src/drivers/mod.rs` — add after the `totp` lines:

```rust
pub(crate) mod mtls;
pub use mtls::MtlsAuthDriver;
```

Full updated `mod.rs`:

```rust
pub(crate) mod ad;
pub(crate) mod api_key;
pub(crate) mod db;
pub(crate) mod kerberos;
pub(crate) mod ldap;
pub(crate) mod mtls;
pub(crate) mod radius;
pub(crate) mod tacacs;
pub(crate) mod totp;

pub use ad::AdAuthDriver;
pub use api_key::ApiKeyAuthDriver;
pub use db::DbAuthDriver;
pub use kerberos::KerberosAuthDriver;
pub use ldap::LdapAuthDriver;
pub use mtls::MtlsAuthDriver;
pub use radius::RadiusAuthDriver;
pub use tacacs::TacacsAuthDriver;
pub use totp::TotpAuthDriver;
```

Modify `crates/security/ox_security_auth/src/lib.rs` — add `MtlsAuthDriver` to the re-exports:

```rust
pub(crate) mod drivers;
pub(crate) mod pipeline;

pub use pipeline::AuthPipeline;
pub use drivers::{
    AdAuthDriver, ApiKeyAuthDriver, DbAuthDriver,
    KerberosAuthDriver, LdapAuthDriver, MtlsAuthDriver, RadiusAuthDriver,
    TacacsAuthDriver, TotpAuthDriver,
};
```

Verify the crate builds:

```bash
cargo build -p ox_security_auth 2>&1 | grep "^error" | head -5
```

### Step 3: Implement `src/drivers/mtls.rs`

Replace the entire file with the full implementation + real tests:

```rust
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, Principal, drivers::AuthDriver,
};

/// Given the raw DER bytes of a client certificate, validate the certificate chain and
/// extract the subject as a `Principal`. Return `Ok(Principal)` on success or
/// `Err(String)` with a human-readable rejection reason.
pub type CertValidatorFn = Arc<dyn Fn(&[u8]) -> Result<Principal, String> + Send + Sync>;

pub struct MtlsAuthDriver {
    validator: CertValidatorFn,
}

impl MtlsAuthDriver {
    pub fn new(validator: CertValidatorFn) -> Self {
        Self { validator }
    }
}

#[async_trait]
impl AuthDriver for MtlsAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let der = match credentials {
            Credentials::ClientCert { der } => der.as_slice(),
            _ => return AuthResult::Continue,
        };

        match (self.validator)(der) {
            Ok(principal) => AuthResult::Authenticated(principal),
            Err(reason) => AuthResult::Reject(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use ox_security_core::{
        AuthResult, AuthPipelineContext, Credentials, Principal, PrincipalId,
        AuthSource, TenantId,
    };
    use secrecy::SecretString;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    /// Build a driver whose validator accepts a specific magic DER payload.
    fn make_driver() -> MtlsAuthDriver {
        MtlsAuthDriver::new(Arc::new(|der: &[u8]| {
            if der == b"valid-cert-der" {
                Ok(Principal {
                    id: PrincipalId::new(),
                    display_name: "CN=device-001".to_string(),
                    source: AuthSource::Mtls,
                    groups: vec![],
                    tenant_id: TenantId::from_str("test").unwrap(),
                    session_id: None,
                })
            } else {
                Err("certificate validation failed: unknown issuer".to_string())
            }
        }))
    }

    #[tokio::test]
    async fn mtls_continues_for_non_cert_creds() {
        let driver = make_driver();
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::from("secret"),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn mtls_authenticates_valid_cert() {
        let driver = make_driver();
        let creds = Credentials::ClientCert {
            der: b"valid-cert-der".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => assert_eq!(p.display_name, "CN=device-001"),
            other => panic!("expected Authenticated, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn mtls_rejects_invalid_cert() {
        let driver = make_driver();
        let creds = Credentials::ClientCert {
            der: b"garbage-bytes".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Reject(msg) => assert!(msg.contains("certificate validation failed")),
            other => panic!("expected Reject, got {:?}", other),
        }
    }
}
```

### Step 4: Add `x509-cert` dev-dependency for future real certificate tests

Modify `crates/security/ox_security_auth/Cargo.toml`:

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

[dev-dependencies]
tokio     = { version = "1", features = ["macros", "rt"] }
x509-cert = "0.2"
```

> **Note:** `x509-cert` is a dev-dependency only. The `CertValidatorFn` injector pattern means the crate itself has zero I/O or TLS dependencies. When a real production validator is wired in (e.g. via `rustls-native-certs` + `x509-cert`), that logic lives in the consuming binary, not this library crate.

### Step 5: Run tests to verify they pass

```bash
cargo test -p ox_security_auth mtls 2>&1 | tail -15
```

Expected: 3 tests pass — `mtls_continues_for_non_cert_creds`, `mtls_authenticates_valid_cert`, `mtls_rejects_invalid_cert`.

Run the full suite to confirm nothing regressed:

```bash
cargo test -p ox_security_auth 2>&1 | tail -10
```

Expected: all prior tests still pass.

### Step 6: Commit

```bash
git add crates/security/ox_security_auth/
git commit -m "feat(security-auth): implement MtlsAuthDriver with validator-fn and 3 tests"
```

---

## Task 3: Final wiring verification

### Step 1: Full build + test run

```bash
cargo build -p ox_security_auth 2>&1 | grep "^error" | head -5
cargo test -p ox_security_auth 2>&1 | tail -15
```

Expected: zero build errors; all tests pass (at minimum: 3 pipeline + 4 db + 3 api_key + 3 mtls = 13 tests).

### Step 2: Commit (if Task 1 and 2 were committed separately, this step is a no-op)

```bash
git status crates/security/ox_security_auth
```

If any files remain unstaged, stage and commit them:

```bash
git add crates/security/ox_security_auth
git commit -m "feat(security-auth): wire MtlsAuthDriver into lib.rs re-exports"
```

---

## Summary of changes

| File | Change |
|---|---|
| `crates/security/ox_security_auth/Cargo.toml` | Add `x509-cert = "0.2"` to `[dev-dependencies]` |
| `crates/security/ox_security_auth/src/drivers/api_key.rs` | Replace stub with `ApiKeyLookupFn` + `ApiKeyAuthDriver::new` + 3 tests |
| `crates/security/ox_security_auth/src/drivers/mtls.rs` | New file: `CertValidatorFn` + `MtlsAuthDriver::new` + 3 tests |
| `crates/security/ox_security_auth/src/drivers/mod.rs` | Add `pub(crate) mod mtls` + `pub use mtls::MtlsAuthDriver` |
| `crates/security/ox_security_auth/src/lib.rs` | Add `MtlsAuthDriver` to re-exports |
