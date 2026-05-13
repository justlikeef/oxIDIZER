# ox_security_core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the `ox_security_core` crate containing all shared types, traits, and error types that every other `ox_security_*` crate depends on.

**Architecture:** Pure library crate — no behaviour, no persistence, no I/O. Defines the canonical types (`Principal`, `SecurityContext`, `Credentials`, `OperationDef`, `ContextDefinition`) and the three driver traits (`AuthDriver`, `AuthzDriver`, `AccountingDriver`). All other security crates depend on this one; this crate depends on nothing in the ox_security family.

**Tech Stack:** Rust, `thiserror`, `serde`, `uuid`, `secrecy` (for `SecretString`), `async-trait`, `chrono`

---

## File Structure

```
crates/security/ox_security_core/
  Cargo.toml
  src/
    lib.rs             — re-exports everything; no logic
    error.rs           — SecurityError, AuthzError
    types.rs           — PrincipalId, GroupId, TenantId, SessionId, SessionToken, AuthSource
    credentials.rs     — Credentials enum, MfaChallenge enum
    principal.rs       — Principal, PartialPrincipal
    context.rs         — SecurityContext, AuthPipelineContext
    operations.rs      — OperationDef struct + well-known OP_* constants
    registration.rs    — ContextDefinition, SecurityRegistration trait, ContextRegistrar trait
    drivers.rs         — AuthDriver, AuthzDriver, AccountingDriver traits + result types
    accounting.rs      — AccountingEvent, AuthOutcome, AuthzOutcome
  tests/
    integration.rs     — trait object safety, type construction, round-trip serde
```

---

## Task 1: Crate scaffold + error types

**Files:**
- Create: `crates/security/ox_security_core/Cargo.toml`
- Create: `crates/security/ox_security_core/src/lib.rs`
- Create: `crates/security/ox_security_core/src/error.rs`
- Modify: `Cargo.toml` (workspace root) — add crate to members

- [ ] **Step 1: Write the failing test**

Create `crates/security/ox_security_core/src/error.rs` and `tests/integration.rs`:

```rust
// tests/integration.rs
use ox_security_core::error::{AuthzError, SecurityError};

#[test]
fn authz_error_display() {
    let e = AuthzError::Denied {
        path: "com.justlikeef.data.obj1".to_string(),
        operation: "write".to_string(),
    };
    assert!(e.to_string().contains("write"));
    assert!(e.to_string().contains("com.justlikeef.data.obj1"));
}

#[test]
fn security_error_from_authz() {
    let authz = AuthzError::Unauthenticated;
    let sec: SecurityError = authz.into();
    assert!(matches!(sec, SecurityError::Authz(_)));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /var/repos/oxIDIZER
cargo test -p ox_security_core 2>&1 | head -20
```
Expected: FAIL — crate does not exist yet.

- [ ] **Step 3: Create workspace Cargo.toml entry**

Add to the `[workspace] members` list in `/var/repos/oxIDIZER/Cargo.toml`:
```toml
"crates/security/ox_security_core",
```

- [ ] **Step 4: Create `crates/security/ox_security_core/Cargo.toml`**

```toml
[package]
name = "ox_security_core"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]
thiserror  = "1"
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
uuid       = { version = "1", features = ["v4", "serde"] }
secrecy    = { version = "0.8", features = ["serde"] }
async-trait = "0.1"
chrono     = { version = "0.4", features = ["serde"] }

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt"] }
```

- [ ] **Step 5: Create `src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthzError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("access denied: operation '{operation}' at '{path}'")]
    Denied { path: String, operation: String },
    #[error("context not registered: '{0}'")]
    UnregisteredContext(String),
    #[error("internal authz error: {0}")]
    Internal(String),
}

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("authentication error: {0}")]
    Auth(String),
    #[error("authorization error: {0}")]
    Authz(#[from] AuthzError),
    #[error("accounting error: {0}")]
    Accounting(String),
    #[error("configuration error: {0}")]
    Config(String),
}
```

- [ ] **Step 6: Create `src/lib.rs`**

```rust
pub mod accounting;
pub mod context;
pub mod credentials;
pub mod drivers;
pub mod error;
pub mod operations;
pub mod principal;
pub mod registration;
pub mod types;

pub use error::{AuthzError, SecurityError};
```

- [ ] **Step 7: Create stub files so the crate compiles**

Create each of the remaining `src/*.rs` files with just `// placeholder` so `lib.rs` compiles:
- `src/accounting.rs`
- `src/context.rs`
- `src/credentials.rs`
- `src/drivers.rs`
- `src/operations.rs`
- `src/principal.rs`
- `src/registration.rs`
- `src/types.rs`

- [ ] **Step 8: Run test to verify it passes**

```bash
cargo test -p ox_security_core 2>&1 | tail -10
```
Expected: `test authz_error_display ... ok`, `test security_error_from_authz ... ok`

- [ ] **Step 9: Commit**

```bash
git add crates/security/ox_security_core Cargo.toml
git commit -m "feat: scaffold ox_security_core crate with error types"
```

---

## Task 2: Identity types

**Files:**
- Modify: `crates/security/ox_security_core/src/types.rs`
- Modify: `crates/security/ox_security_core/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tests/integration.rs`:

```rust
use ox_security_core::types::{
    AuthSource, GroupId, PrincipalId, SessionId, SessionToken, TenantId,
};

#[test]
fn principal_id_roundtrip_serde() {
    let id = PrincipalId::new();
    let json = serde_json::to_string(&id).unwrap();
    let back: PrincipalId = serde_json::from_str(&json).unwrap();
    assert_eq!(id, back);
}

#[test]
fn auth_source_display() {
    assert_eq!(AuthSource::Ldap.to_string(), "ldap");
    assert_eq!(AuthSource::Local.to_string(), "local");
    assert_eq!(AuthSource::Oidc.to_string(), "oidc");
}

#[test]
fn tenant_id_from_str() {
    let id = TenantId::from_str("acme").unwrap();
    assert_eq!(id.as_str(), "acme");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_core types 2>&1 | head -15
```
Expected: FAIL — types not defined.

- [ ] **Step 3: Implement `src/types.rs`**

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrincipalId(Uuid);

impl PrincipalId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
    pub fn as_uuid(&self) -> &Uuid { &self.0 }
}

impl Default for PrincipalId {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn new(name: impl Into<String>) -> Self { Self(name.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantId(String);

impl TenantId {
    pub fn from_str(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            return Err("tenant id must not be empty".to_string());
        }
        Ok(Self(s.to_string()))
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(Uuid);

impl SessionId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

impl Default for SessionId {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn new() -> Self { Self(Uuid::new_v4().to_string()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl Default for SessionToken {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, strum::Display, strum::EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum AuthSource {
    Local,
    Ldap,
    Ad,
    Kerberos,
    Radius,
    Tacacs,
    Okta,
    Saml,
    Oidc,
    ApiKey,
    Mtls,
}
```

- [ ] **Step 4: Add `strum` dependency to `Cargo.toml`**

```toml
strum = { version = "0.26", features = ["derive"] }
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p ox_security_core types 2>&1 | tail -10
```
Expected: all three type tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/security/ox_security_core
git commit -m "feat(security-core): add identity types (PrincipalId, GroupId, TenantId, SessionId, AuthSource)"
```

---

## Task 3: Credentials and MFA challenge types

**Files:**
- Modify: `crates/security/ox_security_core/src/credentials.rs`
- Modify: `crates/security/ox_security_core/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tests/integration.rs`:

```rust
use ox_security_core::credentials::{Credentials, MfaChallenge};
use ox_security_core::types::SessionToken;

#[test]
fn credentials_username_password_constructed() {
    let c = Credentials::UsernamePassword {
        username: "john".to_string(),
        password: "secret".to_string().into(),
    };
    assert!(matches!(c, Credentials::UsernamePassword { .. }));
}

#[test]
fn credentials_mfa_passcode_constructed() {
    let token = SessionToken::new();
    let c = Credentials::MfaPasscode {
        session_token: token,
        code: "123456".to_string(),
    };
    assert!(matches!(c, Credentials::MfaPasscode { .. }));
}

#[test]
fn mfa_challenge_push_contains_token() {
    let token = SessionToken::new();
    let challenge = MfaChallenge::PushSent { session_token: token.clone() };
    if let MfaChallenge::PushSent { session_token } = challenge {
        assert_eq!(session_token.as_str(), token.as_str());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_core credentials 2>&1 | head -15
```
Expected: FAIL — credentials not defined.

- [ ] **Step 3: Implement `src/credentials.rs`**

```rust
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use crate::types::SessionToken;

#[derive(Debug)]
pub enum Credentials {
    UsernamePassword {
        username: String,
        password: SecretString,
    },
    MfaPasscode {
        session_token: SessionToken,
        code: String,
    },
    MfaPush {
        session_token: SessionToken,
    },
    BearerToken {
        token: String,
    },
    SamlAssertion {
        xml: String,
    },
    ApiKey {
        key: SecretString,
    },
    ClientCert {
        der: Vec<u8>,
    },
    KerberosTicket {
        ticket: Vec<u8>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MfaChallenge {
    PushSent { session_token: SessionToken },
    CodeRequired { session_token: SessionToken },
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_security_core credentials 2>&1 | tail -10
```
Expected: all three credential tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_core
git commit -m "feat(security-core): add Credentials and MfaChallenge types"
```

---

## Task 4: Principal and auth pipeline context

**Files:**
- Modify: `crates/security/ox_security_core/src/principal.rs`
- Modify: `crates/security/ox_security_core/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tests/integration.rs`:

```rust
use ox_security_core::principal::{PartialPrincipal, Principal};
use ox_security_core::types::{AuthSource, GroupId, PrincipalId, TenantId};

#[test]
fn principal_constructed_with_groups() {
    let p = Principal {
        id: PrincipalId::new(),
        display_name: "John Smith".to_string(),
        source: AuthSource::Ldap,
        groups: vec![GroupId::new("it"), GroupId::new("dataadmins")],
        tenant_id: TenantId::from_str("acme").unwrap(),
        session_id: None,
    };
    assert_eq!(p.groups.len(), 2);
    assert_eq!(p.display_name, "John Smith");
}

#[test]
fn partial_principal_promotes_to_principal() {
    let partial = PartialPrincipal {
        id: PrincipalId::new(),
        display_name: "Jane".to_string(),
        source: AuthSource::Local,
        groups: vec![GroupId::new("finance")],
        tenant_id: TenantId::from_str("acme").unwrap(),
    };
    let principal: Principal = partial.into_principal(None);
    assert_eq!(principal.display_name, "Jane");
    assert!(principal.session_id.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_core principal 2>&1 | head -15
```
Expected: FAIL — principal types not defined.

- [ ] **Step 3: Implement `src/principal.rs`**

```rust
use serde::{Deserialize, Serialize};
use crate::types::{AuthSource, GroupId, PrincipalId, SessionId, TenantId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub id: PrincipalId,
    pub display_name: String,
    pub source: AuthSource,
    pub groups: Vec<GroupId>,
    pub tenant_id: TenantId,
    pub session_id: Option<SessionId>,
}

/// Produced by credential drivers before MFA is complete.
/// Promotes to Principal after all auth steps pass.
#[derive(Debug, Clone)]
pub struct PartialPrincipal {
    pub id: PrincipalId,
    pub display_name: String,
    pub source: AuthSource,
    pub groups: Vec<GroupId>,
    pub tenant_id: TenantId,
}

impl PartialPrincipal {
    pub fn into_principal(self, session_id: Option<SessionId>) -> Principal {
        Principal {
            id: self.id,
            display_name: self.display_name,
            source: self.source,
            groups: self.groups,
            tenant_id: self.tenant_id,
            session_id,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_security_core principal 2>&1 | tail -10
```
Expected: both principal tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_core
git commit -m "feat(security-core): add Principal, PartialPrincipal types"
```

---

## Task 5: SecurityContext and AuthPipelineContext

**Files:**
- Modify: `crates/security/ox_security_core/src/context.rs`
- Modify: `crates/security/ox_security_core/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tests/integration.rs`:

```rust
use ox_security_core::context::{AuthPipelineContext, SecurityContext};
use ox_security_core::types::TenantId;
use std::net::{IpAddr, Ipv4Addr};

#[test]
fn security_context_default_state() {
    let ctx = SecurityContext::new(
        TenantId::from_str("acme").unwrap(),
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    );
    assert!(ctx.principal.is_none());
    assert!(ctx.call_context.is_empty());
}

#[test]
fn security_context_with_call_context() {
    let mut ctx = SecurityContext::new(
        TenantId::from_str("acme").unwrap(),
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
    );
    ctx.call_context = "com.justlikeef.application1".to_string();
    assert_eq!(ctx.call_context, "com.justlikeef.application1");
}

#[test]
fn auth_pipeline_context_constructed() {
    let ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: TenantId::from_str("acme").unwrap(),
        source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    };
    assert!(ctx.partial_principal.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_core context 2>&1 | head -15
```
Expected: FAIL — context types not defined.

- [ ] **Step 3: Add `AuthzDriver` forward stub to `src/drivers.rs`**

`context.rs` imports `AuthzDriver` — add it to `src/drivers.rs` now so the crate compiles. Task 8 will expand this file with `AuthDriver`, result types, and `AccountingDriver`.

```rust
// src/drivers.rs — forward stub; expanded fully in Task 8
use async_trait::async_trait;
use crate::principal::Principal;

#[derive(Debug)]
pub enum AuthzResult {
    Allow,
    Deny(String),
}

#[async_trait]
pub trait AuthzDriver: Send + Sync {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult;
}
```

- [ ] **Step 4: Implement `src/context.rs`**

```rust
use std::net::IpAddr;
use std::sync::Arc;
use crate::drivers::AuthzDriver;
use crate::error::AuthzError;
use crate::principal::{PartialPrincipal, Principal};
use crate::types::TenantId;

pub struct SecurityContext {
    pub principal: Option<Principal>,
    pub call_context: String,
    pub tenant_id: TenantId,
    pub(crate) source_ip: IpAddr,
    pub(crate) authz: Option<Arc<dyn AuthzDriver>>,
}

impl SecurityContext {
    pub fn new(tenant_id: TenantId, source_ip: IpAddr) -> Self {
        Self {
            principal: None,
            call_context: String::new(),
            tenant_id,
            source_ip,
            authz: None,
        }
    }

    pub fn with_authz(mut self, driver: Arc<dyn AuthzDriver>) -> Self {
        self.authz = Some(driver);
        self
    }

    /// Called by objects using only their own fragment.
    /// Resolves: call_context + "." + object_fragment -> full path -> evaluates grants.
    pub async fn check(&self, object_fragment: &str, operation: &str) -> Result<(), AuthzError> {
        let principal = self.principal.as_ref().ok_or(AuthzError::Unauthenticated)?;
        let driver = self.authz.as_ref().ok_or_else(|| {
            AuthzError::Internal("no authz driver configured on SecurityContext".to_string())
        })?;
        let path = if self.call_context.is_empty() {
            object_fragment.to_string()
        } else {
            format!("{}.{}", self.call_context, object_fragment)
        };
        driver.check(principal, &path, operation).await
    }
}

pub struct AuthPipelineContext {
    pub partial_principal: Option<PartialPrincipal>,
    pub tenant_id: TenantId,
    pub source_ip: IpAddr,
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p ox_security_core context 2>&1 | tail -10
```
Expected: all three context tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/security/ox_security_core
git commit -m "feat(security-core): add SecurityContext and AuthPipelineContext"
```

---

## Task 6: OperationDef and well-known constants

**Files:**
- Modify: `crates/security/ox_security_core/src/operations.rs`
- Modify: `crates/security/ox_security_core/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tests/integration.rs`:

```rust
use ox_security_core::operations::{
    OP_CHANGE, OP_CREATE, OP_DDL, OP_DELETE, OP_EXECUTE, OP_LIST, OP_READ, OP_WRITE,
    OperationDef,
};

#[test]
fn well_known_operations_have_names() {
    assert_eq!(OP_READ.name, "read");
    assert_eq!(OP_WRITE.name, "write");
    assert_eq!(OP_CREATE.name, "create");
    assert_eq!(OP_CHANGE.name, "change");
    assert_eq!(OP_DELETE.name, "delete");
    assert_eq!(OP_LIST.name, "list");
    assert_eq!(OP_EXECUTE.name, "execute");
    assert_eq!(OP_DDL.name, "ddl");
}

#[test]
fn operation_def_custom_name() {
    const OP_ISSUE: OperationDef = OperationDef {
        name: "issue",
        description: "Issue a new certificate",
    };
    assert_eq!(OP_ISSUE.name, "issue");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_core operations 2>&1 | head -15
```
Expected: FAIL — operations not defined.

- [ ] **Step 3: Implement `src/operations.rs`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationDef {
    pub name: &'static str,
    pub description: &'static str,
}

pub const OP_READ:    OperationDef = OperationDef { name: "read",    description: "Read a value or record" };
pub const OP_WRITE:   OperationDef = OperationDef { name: "write",   description: "Write a value or record" };
pub const OP_CREATE:  OperationDef = OperationDef { name: "create",  description: "Create a new record" };
pub const OP_CHANGE:  OperationDef = OperationDef { name: "change",  description: "Modify an existing record" };
pub const OP_DELETE:  OperationDef = OperationDef { name: "delete",  description: "Delete a record" };
pub const OP_LIST:    OperationDef = OperationDef { name: "list",    description: "List or enumerate records" };
pub const OP_EXECUTE: OperationDef = OperationDef { name: "execute", description: "Execute a function or procedure" };
pub const OP_DDL:     OperationDef = OperationDef { name: "ddl",     description: "Modify schema or structure" };
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_security_core operations 2>&1 | tail -10
```
Expected: both operation tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_core
git commit -m "feat(security-core): add OperationDef and well-known OP_* constants"
```

---

## Task 7: ContextDefinition and registration traits

**Files:**
- Modify: `crates/security/ox_security_core/src/registration.rs`
- Modify: `crates/security/ox_security_core/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tests/integration.rs`:

```rust
use ox_security_core::operations::{OP_READ, OP_WRITE, OP_CHANGE, OP_DELETE, OP_DDL};
use ox_security_core::registration::{ContextDefinition, SecurityRegistration};

struct FakeDataObject;

impl SecurityRegistration for FakeDataObject {
    fn context_definition(&self) -> ContextDefinition {
        ContextDefinition {
            root: "dataobject1",
            operations: &[],
            children: &[
                ContextDefinition {
                    root: "field1",
                    operations: &[OP_READ, OP_WRITE, OP_CHANGE, OP_DELETE],
                    children: &[],
                },
            ],
        }
    }
}

#[test]
fn context_definition_root() {
    let obj = FakeDataObject;
    let def = obj.context_definition();
    assert_eq!(def.root, "dataobject1");
    assert_eq!(def.children.len(), 1);
    assert_eq!(def.children[0].root, "field1");
}

#[test]
fn context_definition_operations_at_leaf() {
    let obj = FakeDataObject;
    let def = obj.context_definition();
    let field1 = &def.children[0];
    assert_eq!(field1.operations.len(), 4);
    assert!(field1.operations.iter().any(|op| op.name == "read"));
    assert!(field1.operations.iter().any(|op| op.name == "write"));
}

#[test]
fn context_definition_subtree_operations() {
    let obj = FakeDataObject;
    let def = obj.context_definition();
    let all_ops = def.all_operations();
    assert!(all_ops.iter().any(|op| op.name == "read"));
    assert!(all_ops.iter().any(|op| op.name == "write"));
    // root has no ops of its own — they come from children
    assert_eq!(def.operations.len(), 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_core registration 2>&1 | head -15
```
Expected: FAIL — registration not defined.

- [ ] **Step 3: Implement `src/registration.rs`**

```rust
use crate::operations::OperationDef;

#[derive(Debug, Clone)]
pub struct ContextDefinition {
    pub root: &'static str,
    pub operations: &'static [OperationDef],
    pub children: &'static [ContextDefinition],
}

impl ContextDefinition {
    /// Returns the union of all operations in this node and its entire subtree.
    /// This is the set of operations that can be granted at this node.
    pub fn all_operations(&self) -> Vec<OperationDef> {
        let mut ops: Vec<OperationDef> = self.operations.to_vec();
        for child in self.children {
            for op in child.all_operations() {
                if !ops.iter().any(|o| o.name == op.name) {
                    ops.push(op);
                }
            }
        }
        ops
    }
}

/// Implemented by objects that participate in the permission model.
/// Objects describe only their own fragment — they have no knowledge of
/// which application or call context uses them.
pub trait SecurityRegistration {
    fn context_definition(&self) -> ContextDefinition;
}

/// Implemented by SecurityPipeline. Consuming crates call this at startup
/// to register their context tree fragment.
pub trait ContextRegistrar {
    fn register_context(&self, def: ContextDefinition);
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_security_core registration 2>&1 | tail -10
```
Expected: all three registration tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_core
git commit -m "feat(security-core): add ContextDefinition, SecurityRegistration, ContextRegistrar"
```

---

## Task 8: Driver traits and result types

**Files:**
- Modify: `crates/security/ox_security_core/src/drivers.rs`
- Modify: `crates/security/ox_security_core/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tests/integration.rs`:

```rust
use std::sync::Arc;
use ox_security_core::drivers::{AuthDriver, AuthResult, AuthzDriver, AuthzResult};
use ox_security_core::context::AuthPipelineContext;
use ox_security_core::credentials::Credentials;
use ox_security_core::principal::Principal;
use ox_security_core::types::{AuthSource, TenantId};
use std::net::{IpAddr, Ipv4Addr};

struct AlwaysAllowAuthz;

#[async_trait::async_trait]
impl AuthzDriver for AlwaysAllowAuthz {
    async fn check(&self, _p: &Principal, _path: &str, _op: &str) -> AuthzResult {
        AuthzResult::Allow
    }
}

#[tokio::test]
async fn authz_driver_trait_object() {
    let driver: Arc<dyn AuthzDriver> = Arc::new(AlwaysAllowAuthz);
    let principal = Principal {
        id: ox_security_core::types::PrincipalId::new(),
        display_name: "test".to_string(),
        source: AuthSource::Local,
        groups: vec![],
        tenant_id: TenantId::from_str("acme").unwrap(),
        session_id: None,
    };
    let result = driver.check(&principal, "com.justlikeef.data.obj1", "read").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[test]
fn authz_result_deny_carries_reason() {
    let r = AuthzResult::Deny("no grant found".to_string());
    if let AuthzResult::Deny(reason) = r {
        assert!(reason.contains("no grant"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_core drivers 2>&1 | head -15
```
Expected: FAIL — drivers not defined.

- [ ] **Step 3: Expand `src/drivers.rs`** — `AuthzDriver` and `AuthzResult` are already defined (Task 5 stub). Add `AuthDriver` and `AuthResult` now.

Replace the full contents of `src/drivers.rs` with:

```rust
use async_trait::async_trait;
use crate::accounting::AccountingEvent;
use crate::context::AuthPipelineContext;
use crate::credentials::{Credentials, MfaChallenge};
use crate::principal::Principal;

pub enum AuthResult {
    Authenticated(Principal),
    MfaRequired(MfaChallenge),
    Continue,
    Reject(String),
}

#[derive(Debug)]
pub enum AuthzResult {
    Allow,
    Deny(String),
}

#[async_trait]
pub trait AuthDriver: Send + Sync {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult;
}

#[async_trait]
pub trait AuthzDriver: Send + Sync {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult;
}

#[async_trait]
pub trait AccountingDriver: Send + Sync {
    async fn record(&self, event: &AccountingEvent);
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_security_core drivers 2>&1 | tail -10
```
Expected: both driver tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_core
git commit -m "feat(security-core): add AuthDriver, AuthzDriver traits and result types"
```

---

## Task 9: AccountingEvent and AccountingDriver

**Files:**
- Modify: `crates/security/ox_security_core/src/accounting.rs`
- Modify: `crates/security/ox_security_core/src/drivers.rs`
- Modify: `crates/security/ox_security_core/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tests/integration.rs`:

```rust
use chrono::Utc;
use ox_security_core::accounting::{AccountingEvent, AuthOutcome, AuthzOutcome};
use ox_security_core::types::{SessionId, TenantId};
use std::net::{IpAddr, Ipv4Addr};

#[test]
fn accounting_event_constructed() {
    let event = AccountingEvent {
        principal_id: None,
        auth_outcome: AuthOutcome::Failed("bad password".to_string()),
        authz_outcome: None,
        call_context: "com.justlikeef.application1".to_string(),
        object_fragment: None,
        operation_name: None,
        timestamp: Utc::now(),
        source_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        session_id: None,
        tenant_id: TenantId::from_str("acme").unwrap(),
    };
    assert!(matches!(event.auth_outcome, AuthOutcome::Failed(_)));
    assert!(event.authz_outcome.is_none());
}

#[test]
fn authz_outcome_denied_carries_path_and_op() {
    let outcome = AuthzOutcome::Denied {
        path: "com.justlikeef.data.obj1".to_string(),
        operation_name: "write".to_string(),
    };
    if let AuthzOutcome::Denied { path, operation_name } = outcome {
        assert_eq!(path, "com.justlikeef.data.obj1");
        assert_eq!(operation_name, "write");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_core accounting 2>&1 | head -15
```
Expected: FAIL — accounting types not defined.

- [ ] **Step 3: Implement `src/accounting.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use crate::types::{PrincipalId, SessionId, TenantId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthOutcome {
    Authenticated,
    Failed(String),
    MfaRequired,
    MfaFailed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthzOutcome {
    Allowed,
    Denied { path: String, operation_name: String },
}

#[derive(Debug)]
pub struct AccountingEvent {
    pub principal_id: Option<PrincipalId>,
    pub auth_outcome: AuthOutcome,
    pub authz_outcome: Option<AuthzOutcome>,
    pub call_context: String,
    pub object_fragment: Option<String>,
    pub operation_name: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub source_ip: IpAddr,
    pub session_id: Option<SessionId>,
    pub tenant_id: TenantId,
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p ox_security_core accounting 2>&1 | tail -10
```
Expected: both accounting tests pass.

- [ ] **Step 6: Run full test suite**

```bash
cargo test -p ox_security_core 2>&1 | tail -15
```
Expected: all tests pass, zero failures.

- [ ] **Step 7: Commit**

```bash
git add crates/security/ox_security_core
git commit -m "feat(security-core): add AccountingEvent, AuthOutcome, AuthzOutcome, AccountingDriver"
```

---

## Task 10: Wire lib.rs exports and verify clean build

**Files:**
- Modify: `crates/security/ox_security_core/src/lib.rs`

- [ ] **Step 1: Update `src/lib.rs` with full re-exports**

```rust
pub mod accounting;
pub mod context;
pub mod credentials;
pub mod drivers;
pub mod error;
pub mod operations;
pub mod principal;
pub mod registration;
pub mod types;

// Top-level re-exports for convenience
pub use accounting::{AccountingEvent, AuthOutcome, AuthzOutcome};
pub use context::{AuthPipelineContext, SecurityContext};
pub use credentials::{Credentials, MfaChallenge};
pub use drivers::{AccountingDriver, AuthDriver, AuthResult, AuthzDriver, AuthzResult};
pub use error::{AuthzError, SecurityError};
pub use operations::{
    OperationDef, OP_CHANGE, OP_CREATE, OP_DDL, OP_DELETE, OP_EXECUTE, OP_LIST, OP_READ, OP_WRITE,
};
pub use principal::{PartialPrincipal, Principal};
pub use registration::{ContextDefinition, ContextRegistrar, SecurityRegistration};
pub use types::{AuthSource, GroupId, PrincipalId, SessionId, SessionToken, TenantId};
```

- [ ] **Step 2: Run full build and test suite**

```bash
cargo build -p ox_security_core 2>&1 | tail -5
cargo test -p ox_security_core 2>&1 | tail -15
```
Expected: zero warnings on the security crate itself, all tests pass.

- [ ] **Step 3: Verify trait objects are Send + Sync**

```bash
cargo test -p ox_security_core 2>&1 | grep -E "FAILED|ok|error"
```
Expected: all tests `ok`, no `FAILED`.

- [ ] **Step 4: Commit**

```bash
git add crates/security/ox_security_core
git commit -m "feat(security-core): complete ox_security_core — all types, traits, and re-exports"
```
