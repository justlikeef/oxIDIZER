# ox_security_pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `ox_security_pipeline` — the compositor crate that wires `ox_security_auth`, `ox_security_authz`, and `ox_security_accounting` into a single entry point and implements the `ContextRegistrar` interface for webservice plugin registration.

**Architecture:** `SecurityPipeline` holds one `AuthPipeline`, one `AuthzPipeline`, and one `AccountingPipeline`. `authenticate()` and `authorize()` sequence through those inner pipelines and fire accounting events regardless of outcome. `SecurityPipelineBuilder` constructs the pipeline from lists of typed drivers. `PipelineContextRegistrar` wraps `SecurityPipeline` and implements `ContextRegistrar` so consuming crates can register their context trees at startup.

**Tech Stack:** Rust, `ox_security_core` (all shared types and traits), `ox_security_auth` (AuthPipeline), `ox_security_authz` (AuthzPipeline), `ox_security_accounting` (AccountingPipeline + MemoryAccountingDriver), `async-trait`, `tokio` (dev-dependency)

---

## Background: Inner Pipeline APIs

This plan depends on three sibling crates that must already be implemented. Their public APIs are:

### ox_security_auth — `AuthPipeline`
```rust
// crates/security/ox_security_auth/src/pipeline.rs
pub struct AuthPipeline {
    drivers: Vec<Arc<dyn AuthDriver>>,
}
impl AuthPipeline {
    pub fn new(drivers: Vec<Arc<dyn AuthDriver>>) -> Self;
    pub async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult;
}
```

### ox_security_authz — `AuthzPipeline`
```rust
// crates/security/ox_security_authz/src/pipeline.rs
pub struct AuthzPipeline {
    drivers: Vec<Arc<dyn AuthzDriver>>,
}
impl AuthzPipeline {
    pub fn new(drivers: Vec<Arc<dyn AuthzDriver>>) -> Self;
    pub async fn authorize(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult;
}
```

`AuthzPipeline::authorize` iterates the driver chain. Each driver returns `Allow`, `Deny(reason)`, or `Continue`. If all drivers return `Continue`, the pipeline returns `Deny("no authz driver handled the request")` (fail-closed).

### ox_security_accounting — `AccountingPipeline` + `MemoryAccountingDriver`
```rust
// crates/security/ox_security_accounting/src/pipeline.rs
pub struct AccountingPipeline {
    drivers: Vec<Arc<dyn AccountingDriver>>,
}
impl AccountingPipeline {
    pub fn new(drivers: Vec<Arc<dyn AccountingDriver>>) -> Self;
    pub async fn record(&self, event: &AccountingEvent);
}
```

```rust
// crates/security/ox_security_accounting/src/drivers/memory.rs
pub struct MemoryAccountingDriver {
    events: Arc<Mutex<Vec<AccountingEvent>>>,
}
impl MemoryAccountingDriver {
    pub fn new() -> Self;
    pub fn events(&self) -> Vec<AccountingEvent>;
}
#[async_trait]
impl AccountingDriver for MemoryAccountingDriver { ... }
```

`MemoryAccountingDriver::events()` clones and returns all recorded events.

---

## File Structure

```
crates/security/ox_security_pipeline/
  Cargo.toml
  src/
    lib.rs          — pub mod declarations + top-level re-exports
    error.rs        — SecurityError enum (AuthFailed, MfaRequired, AuthzDenied)
    pipeline.rs     — SecurityPipeline struct + authenticate() + authorize()
    builder.rs      — SecurityPipelineBuilder
    registrar.rs    — PipelineContextRegistrar implements ContextRegistrar
  tests/
    integration.rs  — all integration tests
```

---

## Task 1: Crate scaffold + SecurityError + SecurityPipelineBuilder + skeleton pipeline

**Files:**
- Create: `crates/security/ox_security_pipeline/Cargo.toml`
- Create: `crates/security/ox_security_pipeline/src/lib.rs`
- Create: `crates/security/ox_security_pipeline/src/error.rs`
- Create: `crates/security/ox_security_pipeline/src/pipeline.rs`
- Create: `crates/security/ox_security_pipeline/src/builder.rs`
- Create: `crates/security/ox_security_pipeline/tests/integration.rs`
- Modify: `Cargo.toml` (workspace root) — add crate to `[workspace] members`

- [ ] **Step 1: Write the failing tests**

Create `crates/security/ox_security_pipeline/tests/integration.rs`:

```rust
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use async_trait::async_trait;
use ox_security_core::{
    AuthPipelineContext, AuthResult, AuthSource, AuthzResult, Credentials,
    AccountingEvent, AccountingDriver,
    Principal, PrincipalId, TenantId,
};
use ox_security_pipeline::{SecurityError, SecurityPipelineBuilder};

// ---------------------------------------------------------------------------
// Inline stub drivers — used across all tasks in this file
// ---------------------------------------------------------------------------

struct AlwaysContinueAuthDriver;

#[async_trait]
impl ox_security_core::AuthDriver for AlwaysContinueAuthDriver {
    async fn authenticate(
        &self,
        _creds: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        AuthResult::Continue
    }
}

struct AlwaysRejectAuthDriver;

#[async_trait]
impl ox_security_core::AuthDriver for AlwaysRejectAuthDriver {
    async fn authenticate(
        &self,
        _creds: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        AuthResult::Reject("rejected by policy".to_string())
    }
}

struct AcceptsAliceDriver;

#[async_trait]
impl ox_security_core::AuthDriver for AcceptsAliceDriver {
    async fn authenticate(
        &self,
        creds: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        use secrecy::ExposeSecret;
        match creds {
            Credentials::UsernamePassword { username, password }
                if username == "alice" && password.expose_secret() == "pass" =>
            {
                AuthResult::Authenticated(Principal {
                    id: PrincipalId::new(),
                    display_name: "alice".to_string(),
                    source: AuthSource::Local,
                    groups: vec![],
                    tenant_id: "test".parse().unwrap(),
                    session_id: None,
                })
            }
            _ => AuthResult::Reject("bad credentials".to_string()),
        }
    }
}

struct AlwaysContinueAuthzDriver;

#[async_trait]
impl ox_security_core::AuthzDriver for AlwaysContinueAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Allow
    }
}

struct AlwaysDenyAuthzDriver;

#[async_trait]
impl ox_security_core::AuthzDriver for AlwaysDenyAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Deny("deny by policy".to_string())
    }
}

struct NoOpAccountingDriver;

#[async_trait]
impl AccountingDriver for NoOpAccountingDriver {
    async fn record(&self, _event: &AccountingEvent) {}
}

fn test_tenant() -> TenantId {
    "test".parse().unwrap()
}

fn test_source_ip() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
}

fn test_principal() -> Principal {
    Principal {
        id: PrincipalId::new(),
        display_name: "alice".to_string(),
        source: AuthSource::Local,
        groups: vec![],
        tenant_id: test_tenant(),
        session_id: None,
    }
}

fn alice_creds() -> Credentials {
    Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: "pass".to_string().into(),
    }
}

// ---------------------------------------------------------------------------
// Task 1 tests: builder + skeleton authenticate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn builder_creates_pipeline() {
    let _pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AlwaysContinueAuthDriver))
        .authz(Arc::new(AlwaysContinueAuthzDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();
    // Compiles and runs — structural test
}

#[tokio::test]
async fn authenticate_success() {
    let pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AcceptsAliceDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let result = pipeline.authenticate(&creds, &mut auth_ctx).await;
    match result {
        Ok(p) => assert_eq!(p.display_name, "alice"),
        Err(e) => panic!("expected Ok(Principal), got {:?}", e),
    }
}

#[tokio::test]
async fn authenticate_reject() {
    let pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AlwaysRejectAuthDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let result = pipeline.authenticate(&creds, &mut auth_ctx).await;
    assert!(
        matches!(result, Err(SecurityError::AuthFailed(_))),
        "expected AuthFailed, got {:?}",
        result
    );
}

#[tokio::test]
async fn authenticate_empty_pipeline_fails() {
    let pipeline = SecurityPipelineBuilder::new()
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let result = pipeline.authenticate(&creds, &mut auth_ctx).await;
    assert!(
        matches!(result, Err(SecurityError::AuthFailed(_))),
        "expected AuthFailed from empty pipeline, got {:?}",
        result
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_pipeline 2>&1 | head -15
```
Expected: FAIL — crate does not exist.

- [ ] **Step 3: Add to workspace**

In `/var/repos/oxIDIZER/Cargo.toml`, add inside `[workspace] members` after `"crates/security/ox_security_core"`:

```toml
"crates/security/ox_security_auth",
"crates/security/ox_security_authz",
"crates/security/ox_security_accounting",
"crates/security/ox_security_pipeline",
```

(These three sibling crates must already be present from their own implementation plans. If they are not, implement their plans first before proceeding.)

- [ ] **Step 4: Create `crates/security/ox_security_pipeline/Cargo.toml`**

```toml
[package]
name = "ox_security_pipeline"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]
ox_security_core       = { path = "../ox_security_core" }
ox_security_auth       = { path = "../ox_security_auth" }
ox_security_authz      = { path = "../ox_security_authz" }
ox_security_accounting = { path = "../ox_security_accounting" }
async-trait            = "0.1"

[dev-dependencies]
tokio   = { version = "1", features = ["macros", "rt"] }
secrecy = { version = "0.8", features = ["serde"] }
```

- [ ] **Step 5: Create `src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("authentication failed: {0}")]
    AuthFailed(String),
    #[error("mfa required: {0}")]
    MfaRequired(String),
    #[error("authorization denied: {0}")]
    AuthzDenied(String),
}
```

- [ ] **Step 6: Create `src/pipeline.rs`**

The `SecurityPipeline` wraps the three inner pipelines. Accounting fires unconditionally on every outcome.

```rust
use std::net::{IpAddr, Ipv4Addr};
use chrono::Utc;
use ox_security_auth::pipeline::AuthPipeline;
use ox_security_authz::pipeline::AuthzPipeline;
use ox_security_accounting::pipeline::AccountingPipeline;
use ox_security_core::{
    AccountingEvent, AuthOutcome, AuthPipelineContext, AuthResult, AuthzOutcome, AuthzResult,
    Credentials, Principal, TenantId,
};
use crate::error::SecurityError;

pub struct SecurityPipeline {
    pub(crate) auth: AuthPipeline,
    pub(crate) authz: AuthzPipeline,
    pub(crate) accounting: AccountingPipeline,
}

impl SecurityPipeline {
    /// Authenticate a set of credentials.
    ///
    /// On success records an `AuthSuccess` accounting event and returns `Ok(Principal)`.
    /// On failure records an `AuthFailure` event and returns `Err(SecurityError::AuthFailed)`.
    /// On `MfaRequired` returns `Err(SecurityError::MfaRequired)` — no accounting event is
    /// recorded because the authentication attempt is incomplete.
    pub async fn authenticate(
        &self,
        credentials: &Credentials,
        auth_ctx: &mut AuthPipelineContext,
    ) -> Result<Principal, SecurityError> {
        let result = self.auth.authenticate(credentials, auth_ctx).await;
        match result {
            AuthResult::Authenticated(principal) => {
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: Some(principal.id.clone()),
                        auth_outcome: AuthOutcome::Authenticated,
                        authz_outcome: None,
                        call_context: String::new(),
                        object_fragment: None,
                        operation_name: None,
                        timestamp: Utc::now(),
                        source_ip: auth_ctx.source_ip,
                        session_id: principal.session_id.clone(),
                        tenant_id: auth_ctx.tenant_id.clone(),
                    })
                    .await;
                Ok(principal)
            }
            AuthResult::Reject(reason) => {
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: None,
                        auth_outcome: AuthOutcome::Failed(reason.clone()),
                        authz_outcome: None,
                        call_context: String::new(),
                        object_fragment: None,
                        operation_name: None,
                        timestamp: Utc::now(),
                        source_ip: auth_ctx.source_ip,
                        session_id: None,
                        tenant_id: auth_ctx.tenant_id.clone(),
                    })
                    .await;
                Err(SecurityError::AuthFailed(reason))
            }
            AuthResult::MfaRequired(challenge) => {
                let description = match &challenge {
                    ox_security_core::MfaChallenge::PushSent { .. } => "push sent".to_string(),
                    ox_security_core::MfaChallenge::CodeRequired { .. } => {
                        "code required".to_string()
                    }
                };
                Err(SecurityError::MfaRequired(description))
            }
            AuthResult::Continue => {
                let reason = "no auth driver handled credentials".to_string();
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: None,
                        auth_outcome: AuthOutcome::Failed(reason.clone()),
                        authz_outcome: None,
                        call_context: String::new(),
                        object_fragment: None,
                        operation_name: None,
                        timestamp: Utc::now(),
                        source_ip: auth_ctx.source_ip,
                        session_id: None,
                        tenant_id: auth_ctx.tenant_id.clone(),
                    })
                    .await;
                Err(SecurityError::AuthFailed(reason))
            }
        }
    }

    /// Authorize a principal for an operation on a path.
    ///
    /// `path` is the fully-resolved permission path (call_context + "." + object_fragment).
    /// Returns `Ok(())` on allow, `Err(SecurityError::AuthzDenied)` on deny or empty pipeline.
    pub async fn authorize(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> Result<(), SecurityError> {
        let result = self.authz.authorize(principal, path, operation).await;
        match result {
            AuthzResult::Allow => {
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: Some(principal.id.clone()),
                        auth_outcome: AuthOutcome::Authenticated,
                        authz_outcome: Some(AuthzOutcome::Allowed),
                        call_context: path.to_string(),
                        object_fragment: None,
                        operation_name: Some(operation.to_string()),
                        timestamp: Utc::now(),
                        source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                        session_id: principal.session_id.clone(),
                        tenant_id: principal.tenant_id.clone(),
                    })
                    .await;
                Ok(())
            }
            AuthzResult::Deny(reason) => {
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: Some(principal.id.clone()),
                        auth_outcome: AuthOutcome::Authenticated,
                        authz_outcome: Some(AuthzOutcome::Denied {
                            path: path.to_string(),
                            operation_name: operation.to_string(),
                        }),
                        call_context: path.to_string(),
                        object_fragment: None,
                        operation_name: Some(operation.to_string()),
                        timestamp: Utc::now(),
                        source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                        session_id: principal.session_id.clone(),
                        tenant_id: principal.tenant_id.clone(),
                    })
                    .await;
                Err(SecurityError::AuthzDenied(reason))
            }
        }
    }
}
```

- [ ] **Step 7: Create `src/builder.rs`**

```rust
use std::sync::Arc;
use ox_security_auth::pipeline::AuthPipeline;
use ox_security_authz::pipeline::AuthzPipeline;
use ox_security_accounting::pipeline::AccountingPipeline;
use ox_security_core::{AuthDriver, AuthzDriver, AccountingDriver};
use crate::pipeline::SecurityPipeline;

pub struct SecurityPipelineBuilder {
    auth_drivers: Vec<Arc<dyn AuthDriver>>,
    authz_drivers: Vec<Arc<dyn AuthzDriver>>,
    accounting_drivers: Vec<Arc<dyn AccountingDriver>>,
}

impl SecurityPipelineBuilder {
    pub fn new() -> Self {
        Self {
            auth_drivers: Vec::new(),
            authz_drivers: Vec::new(),
            accounting_drivers: Vec::new(),
        }
    }

    pub fn auth(mut self, driver: Arc<dyn AuthDriver>) -> Self {
        self.auth_drivers.push(driver);
        self
    }

    pub fn authz(mut self, driver: Arc<dyn AuthzDriver>) -> Self {
        self.authz_drivers.push(driver);
        self
    }

    pub fn accounting(mut self, driver: Arc<dyn AccountingDriver>) -> Self {
        self.accounting_drivers.push(driver);
        self
    }

    pub fn build(self) -> SecurityPipeline {
        SecurityPipeline {
            auth: AuthPipeline::new(self.auth_drivers),
            authz: AuthzPipeline::new(self.authz_drivers),
            accounting: AccountingPipeline::new(self.accounting_drivers),
        }
    }
}

impl Default for SecurityPipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 8: Create `src/lib.rs`**

```rust
pub mod builder;
pub mod error;
pub mod pipeline;
pub mod registrar;

pub use builder::SecurityPipelineBuilder;
pub use error::SecurityError;
pub use pipeline::SecurityPipeline;
pub use registrar::PipelineContextRegistrar;
```

- [ ] **Step 9: Create `src/registrar.rs` (skeleton — full implementation in Task 3)**

```rust
use std::sync::Mutex;
use ox_security_core::registration::{ContextDefinition, ContextRegistrar};
use crate::pipeline::SecurityPipeline;

pub struct PipelineContextRegistrar {
    _pipeline: SecurityPipeline,
    context_def: ContextDefinition,
    registrations: Mutex<Vec<ContextDefinition>>,
}

impl PipelineContextRegistrar {
    pub fn new(pipeline: SecurityPipeline, context_def: ContextDefinition) -> Self {
        Self {
            _pipeline: pipeline,
            context_def,
            registrations: Mutex::new(Vec::new()),
        }
    }
}

impl ContextRegistrar for PipelineContextRegistrar {
    fn register_context(&self, def: ContextDefinition) {
        self.registrations.lock().unwrap().push(def);
    }
}
```

- [ ] **Step 10: Run test to verify it passes**

```bash
cargo test -p ox_security_pipeline 2>&1 | tail -15
```
Expected: 4 tests pass — `builder_creates_pipeline`, `authenticate_success`, `authenticate_reject`, `authenticate_empty_pipeline_fails`.

- [ ] **Step 11: Commit**

```bash
git add crates/security/ox_security_pipeline Cargo.toml
git commit -m "feat(security-pipeline): scaffold SecurityPipeline with builder, error types, and auth wiring"
```

---

## Task 2: Authorize integration

**Files:**
- Modify: `crates/security/ox_security_pipeline/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `tests/integration.rs`:

```rust
// ---------------------------------------------------------------------------
// Task 2 tests: authorize
// ---------------------------------------------------------------------------

#[tokio::test]
async fn authorize_allow() {
    let pipeline = SecurityPipelineBuilder::new()
        .authz(Arc::new(AlwaysContinueAuthzDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let principal = test_principal();
    let result = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "read")
        .await;
    assert!(result.is_ok(), "expected Ok(()), got {:?}", result);
}

#[tokio::test]
async fn authorize_deny() {
    let pipeline = SecurityPipelineBuilder::new()
        .authz(Arc::new(AlwaysDenyAuthzDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let principal = test_principal();
    let result = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "write")
        .await;
    assert!(
        matches!(result, Err(SecurityError::AuthzDenied(_))),
        "expected AuthzDenied, got {:?}",
        result
    );
}

#[tokio::test]
async fn authorize_no_drivers_fails() {
    // Empty authz pipeline is fail-closed: AuthzPipeline returns Deny when all drivers Continue.
    let pipeline = SecurityPipelineBuilder::new()
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let principal = test_principal();
    let result = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "read")
        .await;
    assert!(
        matches!(result, Err(SecurityError::AuthzDenied(_))),
        "expected AuthzDenied from empty pipeline, got {:?}",
        result
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_pipeline authorize 2>&1 | head -20
```
Expected: FAIL — `authorize` method may not compile if `AuthzResult` handling is incomplete.

- [ ] **Step 3: Verify `pipeline.rs` already handles all `AuthzResult` variants**

The `authorize()` method written in Task 1 Step 6 handles `Allow` and `Deny`. The empty-pipeline test requires `AuthzPipeline::authorize` to return `Deny` when no drivers are registered. Confirm this is the case in `ox_security_authz`'s `AuthzPipeline::authorize` implementation (it must return `AuthzResult::Deny("no authz driver handled the request")` when the driver list is exhausted). If not, add that behaviour to `ox_security_authz` first.

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_security_pipeline 2>&1 | tail -15
```
Expected: 7 tests pass — 4 from Task 1 + `authorize_allow`, `authorize_deny`, `authorize_no_drivers_fails`.

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_pipeline/tests/integration.rs
git commit -m "feat(security-pipeline): add authorize integration tests — allow, deny, fail-closed empty"
```

---

## Task 3: Accounting events are recorded + PipelineContextRegistrar

**Files:**
- Modify: `crates/security/ox_security_pipeline/src/registrar.rs`
- Modify: `crates/security/ox_security_pipeline/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `tests/integration.rs`:

```rust
use ox_security_accounting::drivers::MemoryAccountingDriver;
use ox_security_core::{AccountingEvent, AuthOutcome, AuthzOutcome};
use ox_security_core::registration::{ContextDefinition, ContextRegistrar};
use ox_security_pipeline::PipelineContextRegistrar;
use ox_security_core::operations::OP_READ;

// ---------------------------------------------------------------------------
// Task 3 tests: accounting events + PipelineContextRegistrar
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_success_records_event() {
    let accounting_driver = Arc::new(MemoryAccountingDriver::new());
    let pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AcceptsAliceDriver))
        .accounting(Arc::clone(&accounting_driver) as Arc<dyn AccountingDriver>)
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let _ = pipeline.authenticate(&creds, &mut auth_ctx).await;

    let events = accounting_driver.events();
    assert_eq!(events.len(), 1);
    assert!(
        matches!(events[0].auth_outcome, AuthOutcome::Authenticated),
        "expected Authenticated outcome, got {:?}",
        events[0].auth_outcome
    );
}

#[tokio::test]
async fn auth_failure_records_event() {
    let accounting_driver = Arc::new(MemoryAccountingDriver::new());
    let pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AlwaysRejectAuthDriver))
        .accounting(Arc::clone(&accounting_driver) as Arc<dyn AccountingDriver>)
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let _ = pipeline.authenticate(&creds, &mut auth_ctx).await;

    let events = accounting_driver.events();
    assert_eq!(events.len(), 1);
    assert!(
        matches!(events[0].auth_outcome, AuthOutcome::Failed(_)),
        "expected Failed outcome, got {:?}",
        events[0].auth_outcome
    );
}

#[tokio::test]
async fn authz_allow_records_event() {
    let accounting_driver = Arc::new(MemoryAccountingDriver::new());
    let pipeline = SecurityPipelineBuilder::new()
        .authz(Arc::new(AlwaysContinueAuthzDriver))
        .accounting(Arc::clone(&accounting_driver) as Arc<dyn AccountingDriver>)
        .build();

    let principal = test_principal();
    let _ = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "read")
        .await;

    let events = accounting_driver.events();
    assert_eq!(events.len(), 1);
    assert!(
        matches!(events[0].authz_outcome, Some(AuthzOutcome::Allowed)),
        "expected Allowed authz outcome, got {:?}",
        events[0].authz_outcome
    );
}

#[tokio::test]
async fn authz_deny_records_event() {
    let accounting_driver = Arc::new(MemoryAccountingDriver::new());
    let pipeline = SecurityPipelineBuilder::new()
        .authz(Arc::new(AlwaysDenyAuthzDriver))
        .accounting(Arc::clone(&accounting_driver) as Arc<dyn AccountingDriver>)
        .build();

    let principal = test_principal();
    let _ = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "write")
        .await;

    let events = accounting_driver.events();
    assert_eq!(events.len(), 1);
    let authz_outcome = events[0].authz_outcome.as_ref().expect("authz_outcome should be Some");
    assert!(
        matches!(authz_outcome, AuthzOutcome::Denied { .. }),
        "expected Denied authz outcome, got {:?}",
        authz_outcome
    );
    if let AuthzOutcome::Denied { path, operation_name } = authz_outcome {
        assert_eq!(path, "com.justlikeef.app.obj");
        assert_eq!(operation_name, "write");
    }
}

static TEST_CONTEXT_DEF: ContextDefinition = ContextDefinition {
    root: "com.justlikeef.test",
    operations: &[OP_READ],
    children: &[],
};

static REGISTERED_DEF: ContextDefinition = ContextDefinition {
    root: "com.justlikeef.test.objects",
    operations: &[OP_READ],
    children: &[],
};

#[test]
fn registrar_stores_registration() {
    let pipeline = SecurityPipelineBuilder::new().build();
    let registrar = PipelineContextRegistrar::new(pipeline, TEST_CONTEXT_DEF);

    registrar.register_context(REGISTERED_DEF);

    let stored = registrar.stored_registrations();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].root, "com.justlikeef.test.objects");

    assert_eq!(registrar.context_definition().root, "com.justlikeef.test");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_pipeline 2>&1 | head -20
```
Expected: FAIL — `MemoryAccountingDriver` not imported, `stored_registrations()` and `context_definition()` not implemented on `PipelineContextRegistrar`.

- [ ] **Step 3: Implement `src/registrar.rs` with `stored_registrations()` and `context_definition()`**

Replace `src/registrar.rs` entirely:

```rust
use std::sync::Mutex;
use ox_security_core::registration::{ContextDefinition, ContextRegistrar};
use crate::pipeline::SecurityPipeline;

pub struct PipelineContextRegistrar {
    _pipeline: SecurityPipeline,
    context_def: ContextDefinition,
    registrations: Mutex<Vec<ContextDefinition>>,
}

impl PipelineContextRegistrar {
    pub fn new(pipeline: SecurityPipeline, context_def: ContextDefinition) -> Self {
        Self {
            _pipeline: pipeline,
            context_def,
            registrations: Mutex::new(Vec::new()),
        }
    }

    /// Returns the `ContextDefinition` this registrar was constructed with.
    /// This is the application-level root node passed to consuming crates.
    pub fn context_definition(&self) -> ContextDefinition {
        self.context_def
    }

    /// Returns a snapshot of all registrations stored so far.
    /// Used in tests; also callable by admin code that needs to enumerate registered contexts.
    pub fn stored_registrations(&self) -> Vec<ContextDefinition> {
        self.registrations.lock().unwrap().clone()
    }
}

impl ContextRegistrar for PipelineContextRegistrar {
    fn register_context(&self, def: ContextDefinition) {
        self.registrations.lock().unwrap().push(def);
    }
}
```

Note: `ContextDefinition` derives `Clone` and `Copy` in `ox_security_core::registration`. Confirm this before running tests — if it only derives `Copy` (not `Clone` explicitly), the `clone()` call on a `Vec<ContextDefinition>` will work because `Copy` implies `Clone`.

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_security_pipeline 2>&1 | tail -20
```
Expected: 12 tests pass — 4 (Task 1) + 3 (Task 2) + 5 (Task 3).

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_pipeline/src/registrar.rs \
        crates/security/ox_security_pipeline/tests/integration.rs
git commit -m "feat(security-pipeline): accounting event recording + PipelineContextRegistrar"
```

---

## Task 4: Wire lib.rs, clean build, final verification

**Files:**
- Modify: `crates/security/ox_security_pipeline/src/lib.rs`

- [ ] **Step 1: Confirm final `src/lib.rs` exports**

Verify `src/lib.rs` reads exactly:

```rust
pub mod builder;
pub mod error;
pub mod pipeline;
pub mod registrar;

pub use builder::SecurityPipelineBuilder;
pub use error::SecurityError;
pub use pipeline::SecurityPipeline;
pub use registrar::PipelineContextRegistrar;
```

- [ ] **Step 2: Build and run all tests with zero warnings**

```bash
cargo build -p ox_security_pipeline 2>&1 | grep "^error"
cargo test -p ox_security_pipeline 2>&1 | tail -20
```
Expected: zero `error:` lines; 12 tests pass.

- [ ] **Step 3: Run the full workspace build to catch any integration regressions**

```bash
cargo build 2>&1 | grep "^error" | head -10
```
Expected: zero errors.

- [ ] **Step 4: Commit**

```bash
git add crates/security/ox_security_pipeline/src/lib.rs
git commit -m "feat(security-pipeline): complete ox_security_pipeline — pipeline, builder, registrar, accounting"
```
