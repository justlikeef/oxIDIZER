# KerberosAuthDriver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `KerberosAuthDriver` in `crates/security/ox_security_auth/src/drivers/kerberos.rs` — a full Kerberos ticket validation driver that accepts `Credentials::KerberosTicket`, validates the ticket against a service principal using `cross-krb5`, extracts the client principal name from the validated context, and returns `AuthResult::Authenticated(Principal)` with `source: AuthSource::Kerberos`. Returns `Continue` for all other credential variants and `Reject` on validation failure.

**Architecture:** The driver holds a `KerberosConfig` (service principal, keytab path, realm, tenant_id) and a pluggable `TicketValidatorFn` (injected in tests, wired to `cross-krb5` in production). This separates the Kerberos GSS-API call from the auth driver logic, allowing unit tests to inject a mock validator without a live KDC.

**Tech Stack:** Rust, `ox_security_core` (all shared types), `async-trait`, `cross-krb5 = "0.5"`, `secrecy`, `tokio` (dev-dependency)

---

## Background: Existing Stub

`crates/security/ox_security_auth/src/drivers/kerberos.rs` currently reads:

```rust
use async_trait::async_trait;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, drivers::AuthDriver};

pub struct KerberosAuthDriver;

#[async_trait]
impl AuthDriver for KerberosAuthDriver {
    async fn authenticate(&self, _credentials: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Continue
    }
}
```

This plan replaces the stub entirely.

## Key Types (from `ox_security_core`)

```rust
// ox_security_core::credentials
pub enum Credentials {
    KerberosTicket { ticket: Vec<u8> },
    // ...other variants
}

// ox_security_core::types
pub enum AuthSource { Kerberos, /* ... */ }
pub struct TenantId(String);  // FromStr impl exists

// ox_security_core::principal
pub struct Principal {
    pub id: PrincipalId,
    pub display_name: String,
    pub source: AuthSource,
    pub groups: Vec<GroupId>,
    pub tenant_id: TenantId,
    pub session_id: Option<SessionId>,
}

// ox_security_core::drivers
pub enum AuthResult {
    Authenticated(Principal),
    Continue,
    Reject(String),
    MfaRequired(MfaChallenge),
}
```

## File Structure

```
crates/security/ox_security_auth/
  Cargo.toml                         — add cross-krb5 dependency
  src/
    drivers/
      kerberos.rs                    — KerberosConfig, TicketValidatorFn, KerberosAuthDriver
```

---

## Task 1: Add dependency and implement KerberosAuthDriver

**Files:**
- Modify: `crates/security/ox_security_auth/Cargo.toml`
- Modify: `crates/security/ox_security_auth/src/drivers/kerberos.rs`

### Step 1: Write the failing tests

Add a `#[cfg(test)]` module at the bottom of `crates/security/ox_security_auth/src/drivers/kerberos.rs`. Write all tests against the final struct shape before writing any implementation.

The complete file for this step (tests only, struct stubs that do not compile):

```rust
// crates/security/ox_security_auth/src/drivers/kerberos.rs
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, TenantId,
    Principal, PrincipalId, AuthSource,
    drivers::AuthDriver,
};

pub type TicketValidatorFn = Arc<dyn Fn(&[u8]) -> Result<String, String> + Send + Sync>;

pub struct KerberosConfig {
    pub service_principal: String,
    pub keytab_path: String,
    pub realm: String,
    pub tenant_id: TenantId,
}

pub struct KerberosAuthDriver {
    config: KerberosConfig,
    validator: TicketValidatorFn,
}

impl KerberosAuthDriver {
    pub fn new(config: KerberosConfig, validator: TicketValidatorFn) -> Self {
        Self { config, validator }
    }
}

#[async_trait]
impl AuthDriver for KerberosAuthDriver {
    async fn authenticate(
        &self,
        _credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        AuthResult::Continue  // placeholder — real logic in Step 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use ox_security_core::AuthPipelineContext;

    fn test_config() -> KerberosConfig {
        KerberosConfig {
            service_principal: "HTTP/myservice.example.com@EXAMPLE.COM".to_string(),
            keytab_path: "/etc/krb5.keytab".to_string(),
            realm: "EXAMPLE.COM".to_string(),
            tenant_id: "test".parse().unwrap(),
        }
    }

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: "test".parse().unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    #[tokio::test]
    async fn kerberos_continues_for_non_ticket_creds() {
        let driver = KerberosAuthDriver::new(
            test_config(),
            Arc::new(|_ticket: &[u8]| Ok("principal@EXAMPLE.COM".to_string())),
        );
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: "pass".to_string().into(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn kerberos_authenticates_valid_ticket() {
        let driver = KerberosAuthDriver::new(
            test_config(),
            Arc::new(|_ticket: &[u8]| Ok("alice@EXAMPLE.COM".to_string())),
        );
        let creds = Credentials::KerberosTicket {
            ticket: b"fake-ticket-bytes".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => {
                assert_eq!(p.display_name, "alice@EXAMPLE.COM");
                assert_eq!(p.source, AuthSource::Kerberos);
                assert!(p.groups.is_empty());
            }
            other => panic!("expected Authenticated, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn kerberos_rejects_invalid_ticket() {
        let driver = KerberosAuthDriver::new(
            test_config(),
            Arc::new(|_ticket: &[u8]| Err("ticket validation failed: clock skew too great".to_string())),
        );
        let creds = Credentials::KerberosTicket {
            ticket: b"invalid-ticket".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Reject(msg) => {
                assert!(msg.contains("ticket validation failed"), "unexpected message: {}", msg);
            }
            other => panic!("expected Reject, got {:?}", other),
        }
    }
}
```

### Step 2: Run test to verify it fails

```bash
cargo test -p ox_security_auth kerberos 2>&1 | head -20
```

Expected: FAIL — `AuthResult::Continue` is returned for `KerberosTicket` in the placeholder, so `kerberos_authenticates_valid_ticket` fails, and `kerberos_rejects_invalid_ticket` fails. `kerberos_continues_for_non_ticket_creds` passes trivially (since all credentials return `Continue`), but the other two fail.

### Step 3: Add `cross-krb5` to Cargo.toml

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
cross-krb5       = "0.5"

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt"] }
```

### Step 4: Implement KerberosAuthDriver

Replace `crates/security/ox_security_auth/src/drivers/kerberos.rs` with the complete implementation:

```rust
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, TenantId,
    Principal, PrincipalId, AuthSource,
    drivers::AuthDriver,
};

/// Pluggable ticket validator.
///
/// Given the raw Kerberos ticket bytes, returns the client principal name
/// (e.g. `"alice@EXAMPLE.COM"`) on success, or an error message on failure.
///
/// In production, wire this to a `cross_krb5::ServerCtx` that accepts the token
/// against the configured service principal and keytab:
///
/// ```rust,ignore
/// use cross_krb5::{ServerCtx, Step};
/// let validator: TicketValidatorFn = Arc::new(move |ticket: &[u8]| {
///     let mut ctx = ServerCtx::new(Some(&service_principal), Some(&keytab_path))
///         .map_err(|e| e.to_string())?;
///     match ctx.step(ticket).map_err(|e| e.to_string())? {
///         Step::Finished(tok) => Ok(ctx.client().map_err(|e| e.to_string())?.to_string()),
///         Step::Continue(_) => Err("unexpected continuation token from server ctx".to_string()),
///     }
/// });
/// ```
pub type TicketValidatorFn = Arc<dyn Fn(&[u8]) -> Result<String, String> + Send + Sync>;

/// Configuration for `KerberosAuthDriver`.
pub struct KerberosConfig {
    /// Fully-qualified service principal, e.g. `"HTTP/myservice.example.com@EXAMPLE.COM"`.
    pub service_principal: String,
    /// Path to the keytab file for service credential, e.g. `"/etc/krb5.keytab"`.
    pub keytab_path: String,
    /// Kerberos realm, e.g. `"EXAMPLE.COM"`.
    pub realm: String,
    /// Tenant this driver is scoped to.
    pub tenant_id: TenantId,
}

/// Authentication driver that validates Kerberos service tickets.
///
/// Accepts only `Credentials::KerberosTicket` — passes `Continue` for all other variants.
/// On validation success, returns `AuthResult::Authenticated` with `source: AuthSource::Kerberos`.
/// On validation failure, returns `AuthResult::Reject` with the validator's error message.
pub struct KerberosAuthDriver {
    config: KerberosConfig,
    validator: TicketValidatorFn,
}

impl KerberosAuthDriver {
    /// Construct a driver with an explicit ticket validator.
    ///
    /// For production use, pass a validator wrapping `cross_krb5::ServerCtx`.
    /// For tests, pass a closure that mocks success or failure.
    pub fn new(config: KerberosConfig, validator: TicketValidatorFn) -> Self {
        Self { config, validator }
    }
}

#[async_trait]
impl AuthDriver for KerberosAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let ticket = match credentials {
            Credentials::KerberosTicket { ticket } => ticket.as_slice(),
            _ => return AuthResult::Continue,
        };

        match (self.validator)(ticket) {
            Ok(client_principal) => AuthResult::Authenticated(Principal {
                id: PrincipalId::new(),
                display_name: client_principal,
                source: AuthSource::Kerberos,
                groups: vec![],
                tenant_id: self.config.tenant_id.clone(),
                session_id: None,
            }),
            Err(reason) => AuthResult::Reject(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use ox_security_core::AuthPipelineContext;

    fn test_config() -> KerberosConfig {
        KerberosConfig {
            service_principal: "HTTP/myservice.example.com@EXAMPLE.COM".to_string(),
            keytab_path: "/etc/krb5.keytab".to_string(),
            realm: "EXAMPLE.COM".to_string(),
            tenant_id: "test".parse().unwrap(),
        }
    }

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: "test".parse().unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    #[tokio::test]
    async fn kerberos_continues_for_non_ticket_creds() {
        let driver = KerberosAuthDriver::new(
            test_config(),
            Arc::new(|_ticket: &[u8]| Ok("principal@EXAMPLE.COM".to_string())),
        );
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: "pass".to_string().into(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn kerberos_authenticates_valid_ticket() {
        let driver = KerberosAuthDriver::new(
            test_config(),
            Arc::new(|_ticket: &[u8]| Ok("alice@EXAMPLE.COM".to_string())),
        );
        let creds = Credentials::KerberosTicket {
            ticket: b"fake-ticket-bytes".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => {
                assert_eq!(p.display_name, "alice@EXAMPLE.COM");
                assert_eq!(p.source, AuthSource::Kerberos);
                assert!(p.groups.is_empty());
            }
            other => panic!("expected Authenticated, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn kerberos_rejects_invalid_ticket() {
        let driver = KerberosAuthDriver::new(
            test_config(),
            Arc::new(|_ticket: &[u8]| Err("ticket validation failed: clock skew too great".to_string())),
        );
        let creds = Credentials::KerberosTicket {
            ticket: b"invalid-ticket".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Reject(msg) => {
                assert!(msg.contains("ticket validation failed"), "unexpected message: {}", msg);
            }
            other => panic!("expected Reject, got {:?}", other),
        }
    }
}
```

### Step 5: Run test to verify all three pass

```bash
cargo test -p ox_security_auth kerberos 2>&1 | tail -10
```

Expected: 3 tests pass — `kerberos_continues_for_non_ticket_creds`, `kerberos_authenticates_valid_ticket`, `kerberos_rejects_invalid_ticket`.

### Step 6: Run the full crate test suite

```bash
cargo test -p ox_security_auth 2>&1 | tail -10
```

Expected: all tests pass (existing DbAuthDriver tests + 3 new Kerberos tests).

### Step 7: Commit

```bash
git add crates/security/ox_security_auth/src/drivers/kerberos.rs \
        crates/security/ox_security_auth/Cargo.toml
git commit -m "feat(security-auth): implement KerberosAuthDriver — ticket validation with cross-krb5, 3 tests"
```

---

## Production Wiring Note

When wiring `KerberosAuthDriver` into the `SecurityPipelineBuilder`, construct the validator using `cross_krb5::ServerCtx`. The `cross-krb5` crate must be able to locate the keytab file at `config.keytab_path` and the KDC for `config.realm` via `krb5.conf` or the `KRB5_CONFIG` environment variable. The driver itself is stateless with respect to the GSS-API context — a new `ServerCtx` is created per ticket validation call, which is correct for service-side GSS-API (each client presents a fresh ticket).

```rust,ignore
use cross_krb5::{ServerCtx, Step};
use std::sync::Arc;
use ox_security_auth::drivers::kerberos::{KerberosAuthDriver, KerberosConfig, TicketValidatorFn};

fn production_kerberos_driver(config: KerberosConfig) -> KerberosAuthDriver {
    let service_principal = config.service_principal.clone();
    let keytab_path = config.keytab_path.clone();
    let validator: TicketValidatorFn = Arc::new(move |ticket: &[u8]| {
        let mut ctx = ServerCtx::new(Some(&service_principal), Some(&keytab_path))
            .map_err(|e| format!("failed to create Kerberos server context: {}", e))?;
        match ctx.step(ticket).map_err(|e| format!("Kerberos step failed: {}", e))? {
            Step::Finished(_) => ctx
                .client()
                .map(|c| c.to_string())
                .map_err(|e| format!("failed to read client principal: {}", e)),
            Step::Continue(_) => Err("unexpected continuation token — single-step validation required".to_string()),
        }
    });
    KerberosAuthDriver::new(config, validator)
}
```
