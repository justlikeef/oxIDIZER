# TOTP MFA Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `TotpAuthDriver` in `ox_security_auth`. The driver validates a time-based one-time passcode (RFC 6238) supplied via `Credentials::MfaPasscode`. It is the second step in a two-factor pipeline: the first driver (e.g. `DbAuthDriver`) authenticates the password and sets `ctx.partial_principal`; `TotpAuthDriver` then validates the TOTP code and promotes the partial principal to a full `Principal`.

**Architecture:** `TotpAuthDriver` holds a `TotpSecretLookupFn` — an injected function that maps a `PrincipalId` to an optional base32-encoded TOTP secret. On `MfaPasscode` credentials: look up `ctx.partial_principal`, retrieve the secret via the lookup function, validate the code with a ±1 step window using `totp-rs`, and return `Authenticated` or `Reject`. All other credential variants return `Continue`.

**Tech Stack:** Rust, `ox_security_core` (all shared types), `async-trait`, `totp-rs = { version = "5", features = ["gen_secret"] }` (RFC 6238 TOTP), `tokio` (dev-dep)

---

## File Structure

```
crates/security/ox_security_auth/
  Cargo.toml                     — add totp-rs dep
  src/
    drivers/
      totp.rs                    — TotpAuthDriver (replaces stub)
      mod.rs                     — unchanged (TotpAuthDriver already re-exported)
```

No new files or crates are required. `TotpAuthDriver` and `TotpSecretLookupFn` are already re-exported from `ox_security_auth` via the existing `drivers/mod.rs` → `lib.rs` chain.

---

## Background: TOTP and the two-step MFA pipeline

A typical two-factor login proceeds as follows:

1. **Step 1 — Password driver** (e.g. `DbAuthDriver`): validates username + password, then instead of returning `Authenticated` it returns `AuthResult::MfaRequired(MfaChallenge::CodeRequired { session_token })` and stores a `PartialPrincipal` in `ctx.partial_principal`.

2. **Step 2 — TOTP driver** (`TotpAuthDriver`): client submits `Credentials::MfaPasscode { session_token, code }`. The driver:
   a. Verifies the session token matches `ctx.partial_principal` (the session token was stored by the pipeline on step 1).
   b. Looks up the TOTP secret for `partial_principal.id`.
   c. Validates the 6-digit code using RFC 6238 with a ±1 step window (30-second steps).
   d. On success: returns `AuthResult::Authenticated(partial_principal.into_principal(None))`.
   e. On failure: returns `AuthResult::Reject`.

**Why `Continue` for non-`MfaPasscode` creds?** The pipeline may have other drivers after `TotpAuthDriver` (e.g. API key). `Continue` lets the pipeline fall through.

**Why `Continue` when no partial principal?** If `ctx.partial_principal` is `None` the MFA step hasn't been primed — some other driver may handle the credentials instead.

**`totp-rs` usage:**

```rust
use totp_rs::{Algorithm, TOTP, Secret};

let totp = TOTP::new(
    Algorithm::SHA1,
    6,       // digits
    1,       // step count (unused — step is always 30s)
    30,      // step seconds
    Secret::Encoded(secret_b32.to_string()).to_bytes().unwrap(),
    None,
    "issuer".to_string(),
).unwrap();

let current_time = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap()
    .as_secs();

// Check current step and ±1 for clock skew tolerance
for offset in [-1i64, 0, 1] {
    let t = (current_time as i64 + offset * 30) as u64;
    if totp.generate(t) == code {
        return true;
    }
}
```

---

## Task 1: `TotpAuthDriver`

**Files:**
- Modify: `crates/security/ox_security_auth/Cargo.toml`
- Modify: `crates/security/ox_security_auth/src/drivers/totp.rs`

- [ ] **Step 1: Add `totp-rs` to `Cargo.toml`**

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
md-5             = "0.10"
rand             = "0.8"
futures          = "0.3"
totp-rs          = { version = "5", features = ["gen_secret"] }

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt"] }
```

Note: if the TACACS+ plan (2026-05-11-security-08) has already been implemented,
`md-5`, `rand`, and `futures` will already be present. Only add the missing ones.

- [ ] **Step 2: Write the failing tests**

APPEND to `crates/security/ox_security_auth/src/drivers/totp.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::str::FromStr;
    use ox_security_core::{
        AuthPipelineContext, AuthResult, Credentials,
        PrincipalId, PartialPrincipal, AuthSource, TenantId, SessionToken,
        GroupId,
    };
    use totp_rs::{Algorithm, TOTP, Secret};

    fn make_partial_principal(id: PrincipalId) -> PartialPrincipal {
        PartialPrincipal {
            id,
            display_name: "alice".to_string(),
            source: AuthSource::Local,
            groups: vec![GroupId::new("users")],
            tenant_id: TenantId::from_str("test").unwrap(),
        }
    }

    fn make_ctx_with_partial(pp: PartialPrincipal) -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: Some(pp),
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    fn make_ctx_empty() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    /// Generate a valid TOTP code for a given base32 secret at the current time.
    fn current_code(secret_b32: &str) -> String {
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            Secret::Encoded(secret_b32.to_string()).to_bytes().unwrap(),
            None,
            "test".to_string(),
        ).unwrap();
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        totp.generate(t)
    }

    // ── Test 1 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_continues_for_non_mfa_creds() {
        use secrecy::SecretString;
        let driver = TotpAuthDriver::new(Arc::new(|_id| Some("JBSWY3DPEHPK3PXP".to_string())));
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("pass".to_string()),
        };
        let mut ctx = make_ctx_empty();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    // ── Test 2 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_continues_when_no_partial_principal() {
        let driver = TotpAuthDriver::new(Arc::new(|_id| Some("JBSWY3DPEHPK3PXP".to_string())));
        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "123456".to_string(),
        };
        let mut ctx = make_ctx_empty(); // no partial principal
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    // ── Test 3 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_validates_correct_code() {
        let secret_b32 = "JBSWY3DPEHPK3PXP";
        let id = PrincipalId::new();
        let id_clone = id.clone();

        let driver = TotpAuthDriver::new(Arc::new(move |pid: &PrincipalId| {
            if pid == &id_clone {
                Some(secret_b32.to_string())
            } else {
                None
            }
        }));

        let code = current_code(secret_b32);
        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code,
        };
        let pp = make_partial_principal(id);
        let mut ctx = make_ctx_with_partial(pp);

        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(
            matches!(result, AuthResult::Authenticated(_)),
            "expected Authenticated, got {:?}", result
        );
        if let AuthResult::Authenticated(p) = result {
            assert_eq!(p.display_name, "alice");
            assert!(matches!(p.source, AuthSource::Local));
        }
    }

    // ── Test 4 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_rejects_wrong_code() {
        let secret_b32 = "JBSWY3DPEHPK3PXP";
        let id = PrincipalId::new();
        let id_clone = id.clone();

        let driver = TotpAuthDriver::new(Arc::new(move |pid: &PrincipalId| {
            if pid == &id_clone {
                Some(secret_b32.to_string())
            } else {
                None
            }
        }));

        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "000000".to_string(), // deliberately wrong
        };
        let pp = make_partial_principal(id);
        let mut ctx = make_ctx_with_partial(pp);

        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Reject(_)));
    }

    // ── Test 5 ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn totp_rejects_when_no_secret_configured() {
        // Lookup returns None — user has no TOTP secret enrolled.
        // Even with a valid-looking code, must Reject.
        let driver = TotpAuthDriver::new(Arc::new(|_id: &PrincipalId| None));

        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "123456".to_string(),
        };
        let pp = make_partial_principal(PrincipalId::new());
        let mut ctx = make_ctx_with_partial(pp);

        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(
            matches!(result, AuthResult::Reject(_)),
            "must Reject when user has no TOTP secret configured"
        );
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test -p ox_security_auth totp 2>&1 | head -20
```
Expected: FAIL — `TotpAuthDriver` is a stub with no `TotpSecretLookupFn` or TOTP logic.

- [ ] **Step 4: Implement `src/drivers/totp.rs`**

Replace the entire file:

```rust
//! TOTP MFA authentication driver.
//!
//! This driver handles the second factor in a two-step pipeline.
//! It accepts only `Credentials::MfaPasscode` and validates the 6-digit code
//! against the user's TOTP secret using RFC 6238 (SHA-1, 30-second steps, ±1 window).
//!
//! For all other credential variants, and when no `partial_principal` is set in
//! the pipeline context, it returns `AuthResult::Continue` so the pipeline can
//! pass through to the next driver.

use std::sync::Arc;
use async_trait::async_trait;
use totp_rs::{Algorithm, TOTP, Secret};
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, PrincipalId,
    drivers::AuthDriver,
};

/// Returns the base32-encoded TOTP secret for a given `PrincipalId`, or `None`
/// if the user has no TOTP configured.
///
/// In production this will query the user store (via a data crate); in tests it
/// is an in-closure lookup over a fixed map.
pub type TotpSecretLookupFn = Arc<dyn Fn(&PrincipalId) -> Option<String> + Send + Sync>;

/// TOTP MFA authentication driver.
///
/// Position in pipeline: after the primary credential driver (e.g. `DbAuthDriver`)
/// which sets `ctx.partial_principal` and returns `MfaRequired`.
pub struct TotpAuthDriver {
    lookup: TotpSecretLookupFn,
}

impl TotpAuthDriver {
    pub fn new(lookup: TotpSecretLookupFn) -> Self {
        Self { lookup }
    }
}

/// Validate a 6-digit TOTP code against a base32-encoded secret.
/// Accepts codes from the current 30-second step and the immediately
/// preceding and following steps (±1 window) to tolerate clock skew.
///
/// Returns `true` if the code is valid for any of the three steps.
fn validate_totp(secret_b32: &str, code: &str) -> bool {
    let secret_bytes = match Secret::Encoded(secret_b32.to_string()).to_bytes() {
        Ok(b) => b,
        Err(_) => return false,
    };

    let totp = match TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        None,
        "ox".to_string(),
    ) {
        Ok(t) => t,
        Err(_) => return false,
    };

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Check current step and ±1 for clock skew tolerance.
    for offset in [-1i64, 0, 1] {
        let t = (now_secs as i64 + offset * 30) as u64;
        if totp.generate(t) == code {
            return true;
        }
    }
    false
}

#[async_trait]
impl AuthDriver for TotpAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        // Only handle MfaPasscode credentials.
        let code = match credentials {
            Credentials::MfaPasscode { code, .. } => code.as_str(),
            _ => return AuthResult::Continue,
        };

        // Only proceed if a partial principal has been set by a prior driver.
        let partial = match ctx.partial_principal.take() {
            Some(p) => p,
            None => return AuthResult::Continue,
        };

        // Look up the TOTP secret for this principal.
        let secret = match (self.lookup)(&partial.id) {
            Some(s) => s,
            None => {
                return AuthResult::Reject(format!(
                    "no TOTP secret configured for principal '{}'",
                    partial.id.as_uuid()
                ));
            }
        };

        if validate_totp(&secret, code) {
            AuthResult::Authenticated(partial.into_principal(None))
        } else {
            AuthResult::Reject("invalid TOTP code".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::str::FromStr;
    use ox_security_core::{
        AuthPipelineContext, AuthResult, Credentials,
        PrincipalId, PartialPrincipal, AuthSource, TenantId, SessionToken,
        GroupId,
    };
    use totp_rs::{Algorithm, TOTP, Secret};

    fn make_partial_principal(id: PrincipalId) -> PartialPrincipal {
        PartialPrincipal {
            id,
            display_name: "alice".to_string(),
            source: AuthSource::Local,
            groups: vec![GroupId::new("users")],
            tenant_id: TenantId::from_str("test").unwrap(),
        }
    }

    fn make_ctx_with_partial(pp: PartialPrincipal) -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: Some(pp),
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    fn make_ctx_empty() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    /// Generate a valid TOTP code for a given base32 secret at the current time.
    fn current_code(secret_b32: &str) -> String {
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            Secret::Encoded(secret_b32.to_string()).to_bytes().unwrap(),
            None,
            "test".to_string(),
        ).unwrap();
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        totp.generate(t)
    }

    #[tokio::test]
    async fn totp_continues_for_non_mfa_creds() {
        use secrecy::SecretString;
        let driver = TotpAuthDriver::new(Arc::new(|_id| Some("JBSWY3DPEHPK3PXP".to_string())));
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("pass".to_string()),
        };
        let mut ctx = make_ctx_empty();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn totp_continues_when_no_partial_principal() {
        let driver = TotpAuthDriver::new(Arc::new(|_id| Some("JBSWY3DPEHPK3PXP".to_string())));
        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "123456".to_string(),
        };
        let mut ctx = make_ctx_empty();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn totp_validates_correct_code() {
        let secret_b32 = "JBSWY3DPEHPK3PXP";
        let id = PrincipalId::new();
        let id_clone = id.clone();

        let driver = TotpAuthDriver::new(Arc::new(move |pid: &PrincipalId| {
            if pid == &id_clone {
                Some(secret_b32.to_string())
            } else {
                None
            }
        }));

        let code = current_code(secret_b32);
        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code,
        };
        let pp = make_partial_principal(id);
        let mut ctx = make_ctx_with_partial(pp);

        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(
            matches!(result, AuthResult::Authenticated(_)),
            "expected Authenticated, got {:?}", result
        );
        if let AuthResult::Authenticated(p) = result {
            assert_eq!(p.display_name, "alice");
            assert!(matches!(p.source, AuthSource::Local));
        }
    }

    #[tokio::test]
    async fn totp_rejects_wrong_code() {
        let secret_b32 = "JBSWY3DPEHPK3PXP";
        let id = PrincipalId::new();
        let id_clone = id.clone();

        let driver = TotpAuthDriver::new(Arc::new(move |pid: &PrincipalId| {
            if pid == &id_clone {
                Some(secret_b32.to_string())
            } else {
                None
            }
        }));

        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "000000".to_string(),
        };
        let pp = make_partial_principal(id);
        let mut ctx = make_ctx_with_partial(pp);

        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Reject(_)));
    }

    #[tokio::test]
    async fn totp_rejects_when_no_secret_configured() {
        let driver = TotpAuthDriver::new(Arc::new(|_id: &PrincipalId| None));

        let creds = Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "123456".to_string(),
        };
        let pp = make_partial_principal(PrincipalId::new());
        let mut ctx = make_ctx_with_partial(pp);

        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(
            matches!(result, AuthResult::Reject(_)),
            "must Reject when user has no TOTP secret configured"
        );
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p ox_security_auth totp 2>&1 | tail -15
```
Expected output:
```
test drivers::totp::tests::totp_continues_for_non_mfa_creds ... ok
test drivers::totp::tests::totp_continues_when_no_partial_principal ... ok
test drivers::totp::tests::totp_rejects_when_no_secret_configured ... ok
test drivers::totp::tests::totp_rejects_wrong_code ... ok
test drivers::totp::tests::totp_validates_correct_code ... ok

test result: ok. 5 passed; 0 failed
```

- [ ] **Step 6: Verify no regressions in full crate**

```bash
cargo test -p ox_security_auth 2>&1 | tail -20
```
Expected: all tests pass (TOTP + TACACS + DB + proto unit tests).

- [ ] **Step 7: Verify clean build**

```bash
cargo build -p ox_security_auth 2>&1 | grep "^error" | head -5
```
Expected: no output.

- [ ] **Step 8: Commit**

```bash
git add crates/security/ox_security_auth
git commit -m "feat(security-auth): implement TotpAuthDriver — RFC 6238 TOTP MFA with injected secret lookup"
```

---

## Task 2: Integration test — full two-step pipeline

This task adds an end-to-end test that wires `DbAuthDriver` → `TotpAuthDriver`
in a pipeline to verify the full MFA flow without mocking either driver.

**Files:**
- Create: `crates/security/ox_security_auth/tests/mfa_pipeline.rs`

- [ ] **Step 1: Write the integration test**

Create `crates/security/ox_security_auth/tests/mfa_pipeline.rs`:

```rust
//! End-to-end test: DbAuthDriver → TotpAuthDriver pipeline.
//!
//! Simulates a login that:
//!   1. Presents UsernamePassword → DbAuthDriver returns MfaRequired.
//!   2. Pipeline sets PartialPrincipal on ctx.
//!   3. Presents MfaPasscode → TotpAuthDriver validates code → Authenticated.

use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::Arc;
use ox_security_auth::{DbAuthDriver, TotpAuthDriver};
use ox_security_auth::pipeline::AuthPipeline;
use ox_security_core::{
    AuthPipelineContext, AuthResult, Credentials,
    MfaChallenge, TenantId, SessionToken,
    PrincipalId, PartialPrincipal, AuthSource, GroupId,
};
use secrecy::SecretString;
use totp_rs::{Algorithm, TOTP, Secret};

const SECRET_B32: &str = "JBSWY3DPEHPK3PXP";

fn current_totp_code() -> String {
    let totp = TOTP::new(
        Algorithm::SHA1, 6, 1, 30,
        Secret::Encoded(SECRET_B32.to_string()).to_bytes().unwrap(),
        None, "test".to_string(),
    ).unwrap();
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    totp.generate(t)
}

/// A stub DbAuthDriver-like driver that returns MfaRequired with a PartialPrincipal.
/// (Real DbAuthDriver in a full integration test would connect to a DB.)
struct StubFirstFactorDriver {
    tenant_id: TenantId,
}

use async_trait::async_trait;
use ox_security_core::drivers::{AuthDriver};

#[async_trait]
impl AuthDriver for StubFirstFactorDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        match credentials {
            Credentials::UsernamePassword { username, .. } => {
                // Store partial principal in context for the TOTP driver.
                ctx.partial_principal = Some(PartialPrincipal {
                    id: PrincipalId::new(),
                    display_name: username.clone(),
                    source: AuthSource::Local,
                    groups: vec![GroupId::new("users")],
                    tenant_id: self.tenant_id.clone(),
                });
                AuthResult::MfaRequired(MfaChallenge::CodeRequired {
                    session_token: SessionToken::new(),
                })
            }
            _ => AuthResult::Continue,
        }
    }
}

#[tokio::test]
async fn full_mfa_pipeline_authenticates_with_correct_totp() {
    let tenant = TenantId::from_str("integration-test").unwrap();

    let first_factor: Arc<dyn ox_security_core::drivers::AuthDriver> =
        Arc::new(StubFirstFactorDriver { tenant_id: tenant.clone() });

    let totp_driver: Arc<dyn ox_security_core::drivers::AuthDriver> =
        Arc::new(TotpAuthDriver::new(Arc::new(|_id| Some(SECRET_B32.to_string()))));

    let pipeline = AuthPipeline::new(vec![first_factor, totp_driver]);

    // Step 1: password
    let mut ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: tenant.clone(),
        source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    };

    let step1 = pipeline.authenticate(
        &Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("pass".to_string()),
        },
        &mut ctx,
    ).await;

    assert!(
        matches!(step1, AuthResult::MfaRequired(_)),
        "step 1 must return MfaRequired"
    );

    // Step 2: TOTP code (partial_principal is still in ctx from step 1)
    let step2 = pipeline.authenticate(
        &Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: current_totp_code(),
        },
        &mut ctx,
    ).await;

    assert!(
        matches!(step2, AuthResult::Authenticated(_)),
        "step 2 must return Authenticated after valid TOTP code"
    );
}

#[tokio::test]
async fn full_mfa_pipeline_rejects_with_wrong_totp() {
    let tenant = TenantId::from_str("integration-test").unwrap();

    let first_factor: Arc<dyn ox_security_core::drivers::AuthDriver> =
        Arc::new(StubFirstFactorDriver { tenant_id: tenant.clone() });

    let totp_driver: Arc<dyn ox_security_core::drivers::AuthDriver> =
        Arc::new(TotpAuthDriver::new(Arc::new(|_id| Some(SECRET_B32.to_string()))));

    let pipeline = AuthPipeline::new(vec![first_factor, totp_driver]);

    let mut ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: tenant.clone(),
        source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    };

    // Step 1
    let _ = pipeline.authenticate(
        &Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("pass".to_string()),
        },
        &mut ctx,
    ).await;

    // Step 2: wrong code
    let step2 = pipeline.authenticate(
        &Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "000000".to_string(),
        },
        &mut ctx,
    ).await;

    assert!(
        matches!(step2, AuthResult::Reject(_)),
        "wrong TOTP code must produce Reject"
    );
}
```

Note: this test requires `AuthPipeline::authenticate` to be `pub` in
`ox_security_auth::pipeline`. Verify the pipeline module exposes this method; if
it is `pub(crate)`, promote it to `pub`.

- [ ] **Step 2: Run the integration tests**

```bash
cargo test -p ox_security_auth --test mfa_pipeline 2>&1 | tail -15
```
Expected output:
```
test full_mfa_pipeline_authenticates_with_correct_totp ... ok
test full_mfa_pipeline_rejects_with_wrong_totp ... ok

test result: ok. 2 passed; 0 failed
```

- [ ] **Step 3: Run complete test suite**

```bash
cargo test -p ox_security_auth 2>&1 | tail -20
```
Expected: all unit + integration tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/security/ox_security_auth
git commit -m "test(security-auth): add end-to-end MFA pipeline integration test for TotpAuthDriver"
```
