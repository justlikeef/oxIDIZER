# ox_security_authz Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `ox_security_authz` crate — the authorization pipeline that chains multiple `AuthzDriver` implementations, evaluates permission grants against a hierarchical context tree, and includes a concrete `LocalDbAuthzDriver` backed by an injected lookup function.

**Architecture:** A `Vec<Arc<dyn AuthzDriver>>` is iterated on each check; the first non-`Continue` result wins. If all drivers return `Continue` the pipeline denies (fail-closed). `LocalDbAuthzDriver` holds grants via a pluggable lookup function keyed by `PrincipalId` and `GroupId` slices, supports exact-match and `*`-wildcard-suffix resource patterns, and returns `Continue` (not `Deny`) when no grant matches so downstream drivers can weigh in. `AuthzResult::Continue` is added to `ox_security_core` as a prerequisite step, mirroring `AuthResult::Continue` in the auth pipeline.

**Tech Stack:** Rust, `ox_security_core` (all shared types), `async-trait`, `tokio` (dev-dependency)

---

## File Structure

```
crates/security/ox_security_authz/
  Cargo.toml                    — package + deps
  src/
    lib.rs                      — pub mod declarations + top-level re-exports
    pipeline.rs                 — AuthzPipeline: Vec<Arc<dyn AuthzDriver>>, check()
    grant.rs                    — PermissionGrant struct
    drivers/
      mod.rs                    — pub use of each driver
      local_db.rs               — LocalDbAuthzDriver: injected GrantLookupFn, wildcard eval
      ldap.rs                   — LdapAuthzDriver stub (returns Continue)
      ad.rs                     — AdAuthzDriver stub (returns Continue)
      okta.rs                   — OktaAuthzDriver stub (returns Continue)
  tests/
    integration.rs              — all pipeline + LocalDbAuthzDriver tests

Prerequisite modification:
  crates/security/ox_security_core/src/drivers.rs   — add Continue to AuthzResult
```

---

## Task 1: Add `AuthzResult::Continue` to `ox_security_core`, scaffold crate, implement `AuthzPipeline` and stubs

**Files:**
- Modify: `crates/security/ox_security_core/src/drivers.rs`
- Create: `crates/security/ox_security_authz/Cargo.toml`
- Create: `crates/security/ox_security_authz/src/lib.rs`
- Create: `crates/security/ox_security_authz/src/pipeline.rs`
- Create: `crates/security/ox_security_authz/src/grant.rs`
- Create: `crates/security/ox_security_authz/src/drivers/mod.rs`
- Create: `crates/security/ox_security_authz/src/drivers/ldap.rs`
- Create: `crates/security/ox_security_authz/src/drivers/ad.rs`
- Create: `crates/security/ox_security_authz/src/drivers/okta.rs`
- Create: `crates/security/ox_security_authz/src/drivers/local_db.rs` (skeleton only — Task 2 fills it)
- Create: `crates/security/ox_security_authz/tests/integration.rs`
- Modify: `Cargo.toml` (workspace root) — add crate to members

- [ ] **Step 1: Write the failing tests**

Create `crates/security/ox_security_authz/tests/integration.rs`:

```rust
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_authz::pipeline::AuthzPipeline;
use ox_security_core::{
    AuthzResult,
    drivers::AuthzDriver,
    principal::Principal,
    types::{AuthSource, GroupId, PrincipalId, TenantId},
};
use std::str::FromStr;

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

struct AlwaysAllowDriver;

#[async_trait]
impl AuthzDriver for AlwaysAllowDriver {
    async fn check(&self, _principal: &Principal, _path: &str, _operation: &str) -> AuthzResult {
        AuthzResult::Allow
    }
}

struct AlwaysDenyDriver;

#[async_trait]
impl AuthzDriver for AlwaysDenyDriver {
    async fn check(&self, _principal: &Principal, _path: &str, _operation: &str) -> AuthzResult {
        AuthzResult::Deny("explicit deny".to_string())
    }
}

struct AlwaysContinueDriver;

#[async_trait]
impl AuthzDriver for AlwaysContinueDriver {
    async fn check(&self, _principal: &Principal, _path: &str, _operation: &str) -> AuthzResult {
        AuthzResult::Continue
    }
}

// ─── pipeline tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn empty_pipeline_denies() {
    let pipeline = AuthzPipeline::new(vec![]);
    let principal = test_principal();
    let result = pipeline.check(&principal, "app.resource", "read").await;
    assert!(matches!(result, AuthzResult::Deny(_)));
}

#[tokio::test]
async fn pipeline_stops_at_deny() {
    // Second driver must never be reached — if it were, the test would get Allow.
    let pipeline = AuthzPipeline::new(vec![
        Arc::new(AlwaysDenyDriver),
        Arc::new(AlwaysAllowDriver),
    ]);
    let principal = test_principal();
    let result = pipeline.check(&principal, "app.resource", "read").await;
    assert!(matches!(result, AuthzResult::Deny(_)));
}

#[tokio::test]
async fn pipeline_stops_at_allow() {
    // First driver allows; second driver would deny but must not be reached.
    let pipeline = AuthzPipeline::new(vec![
        Arc::new(AlwaysAllowDriver),
        Arc::new(AlwaysDenyDriver),
    ]);
    let principal = test_principal();
    let result = pipeline.check(&principal, "app.resource", "read").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn pipeline_continues_through_continues() {
    // All drivers return Continue → pipeline must deny (fail-closed).
    let pipeline = AuthzPipeline::new(vec![
        Arc::new(AlwaysContinueDriver),
        Arc::new(AlwaysContinueDriver),
        Arc::new(AlwaysContinueDriver),
    ]);
    let principal = test_principal();
    let result = pipeline.check(&principal, "app.resource", "read").await;
    assert!(matches!(result, AuthzResult::Deny(_)));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p ox_security_authz 2>&1 | head -15
```
Expected: FAIL — crate does not exist yet.

- [ ] **Step 3: Add `AuthzResult::Continue` to `ox_security_core`**

Edit `crates/security/ox_security_core/src/drivers.rs`. Replace the `AuthzResult` enum:

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
    Continue,
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

- [ ] **Step 4: Verify `ox_security_core` and any existing crates still build**

```bash
cargo build -p ox_security_core 2>&1 | grep "^error" | head -10
```

Also check `ox_security_auth` (if it exists in the workspace) is not broken by the `AuthzResult` change — it only uses `AuthResult`:

```bash
cargo build -p ox_security_auth 2>&1 | grep "^error" | head -10
```

Expected: zero errors for both. The `Continue` addition is purely additive — no existing match arms are exhaustive on `AuthzResult`.

If any crate has `match result { AuthzResult::Allow => … | AuthzResult::Deny(_) => … }` without a wildcard arm, the compiler will report a non-exhaustive match. Fix those match arms to add `AuthzResult::Continue => …` with appropriate handling (treat as deny or propagate).

- [ ] **Step 5: Add to workspace**

Add to `[workspace] members` in `/var/repos/oxIDIZER/Cargo.toml`:

```toml
"crates/security/ox_security_authz",
```

- [ ] **Step 6: Create `crates/security/ox_security_authz/Cargo.toml`**

```toml
[package]
name = "ox_security_authz"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]
ox_security_core = { path = "../ox_security_core" }
async-trait      = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt"] }
```

- [ ] **Step 7: Create `src/pipeline.rs`**

```rust
use std::sync::Arc;
use ox_security_core::{AuthzResult, drivers::AuthzDriver, principal::Principal};

pub struct AuthzPipeline {
    drivers: Vec<Arc<dyn AuthzDriver>>,
}

impl AuthzPipeline {
    pub fn new(drivers: Vec<Arc<dyn AuthzDriver>>) -> Self {
        Self { drivers }
    }

    pub async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        for driver in &self.drivers {
            match driver.check(principal, path, operation).await {
                AuthzResult::Continue => continue,
                result => return result,
            }
        }
        AuthzResult::Deny("no authz driver granted access".to_string())
    }
}
```

- [ ] **Step 8: Create `src/grant.rs`**

```rust
/// A single permission grant stored against a principal or group.
///
/// `resource_pattern` is `None` to match all resources for the operation,
/// or `Some(pattern)` where `pattern` may be:
///   - an exact string, e.g. `"files/readme.txt"`
///   - a wildcard-suffix string ending with `/*`, e.g. `"files/*"` — matches any
///     resource whose path starts with `"files/"`.
#[derive(Debug, Clone)]
pub struct PermissionGrant {
    pub operation: String,
    pub resource_pattern: Option<String>,
}
```

- [ ] **Step 9: Create stub drivers**

Create `src/drivers/ldap.rs`:

```rust
use async_trait::async_trait;
use ox_security_core::{AuthzResult, drivers::AuthzDriver, principal::Principal};

pub struct LdapAuthzDriver;

#[async_trait]
impl AuthzDriver for LdapAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Continue
    }
}
```

Create `src/drivers/ad.rs`:

```rust
use async_trait::async_trait;
use ox_security_core::{AuthzResult, drivers::AuthzDriver, principal::Principal};

pub struct AdAuthzDriver;

#[async_trait]
impl AuthzDriver for AdAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Continue
    }
}
```

Create `src/drivers/okta.rs`:

```rust
use async_trait::async_trait;
use ox_security_core::{AuthzResult, drivers::AuthzDriver, principal::Principal};

pub struct OktaAuthzDriver;

#[async_trait]
impl AuthzDriver for OktaAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Continue
    }
}
```

Create `src/drivers/local_db.rs` (skeleton only — Task 2 replaces this entirely):

```rust
use async_trait::async_trait;
use ox_security_core::{AuthzResult, drivers::AuthzDriver, principal::Principal};

pub struct LocalDbAuthzDriver;

#[async_trait]
impl AuthzDriver for LocalDbAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Continue
    }
}
```

Create `src/drivers/mod.rs`:

```rust
pub mod ad;
pub mod ldap;
pub mod local_db;
pub mod okta;

pub use ad::AdAuthzDriver;
pub use ldap::LdapAuthzDriver;
pub use local_db::LocalDbAuthzDriver;
pub use okta::OktaAuthzDriver;
```

- [ ] **Step 10: Create `src/lib.rs`**

```rust
pub mod drivers;
pub mod grant;
pub mod pipeline;

pub use drivers::{AdAuthzDriver, LdapAuthzDriver, LocalDbAuthzDriver, OktaAuthzDriver};
pub use grant::PermissionGrant;
pub use pipeline::AuthzPipeline;
```

- [ ] **Step 11: Run tests to verify they pass**

```bash
cargo test -p ox_security_authz 2>&1 | tail -15
```

Expected output:

```
running 4 tests
test empty_pipeline_denies ... ok
test pipeline_continues_through_continues ... ok
test pipeline_stops_at_allow ... ok
test pipeline_stops_at_deny ... ok

test result: ok. 4 passed; 0 failed; 0 ignored
```

- [ ] **Step 12: Commit**

```bash
git add crates/security/ox_security_core/src/drivers.rs \
        crates/security/ox_security_authz \
        Cargo.toml
git commit -m "feat(security-authz): scaffold AuthzPipeline, PermissionGrant, stub drivers; add AuthzResult::Continue to core"
```

---

## Task 2: `LocalDbAuthzDriver` — grant lookup with wildcard resource matching

**Files:**
- Modify: `crates/security/ox_security_authz/src/drivers/local_db.rs`
- Modify: `crates/security/ox_security_authz/tests/integration.rs`

The driver takes an injected lookup function so it can be tested without a database. Given a `PrincipalId` and the principal's `GroupId` slice, the function returns all `PermissionGrant`s that apply to that principal (direct + via groups). The driver evaluates them against the requested `(operation, resource)` pair.

**Evaluation rules (applied in this order):**
1. If any matching grant is an explicit `Deny` — **not applicable here** because `PermissionGrant` only stores `Allow` grants in this crate (deny is handled by returning `AuthzResult::Deny` from a dedicated deny driver, or by the pipeline's fail-closed default). A grant match → `Allow`. No match → `Continue`.
2. Wildcard suffix: a `resource_pattern` of `"files/*"` matches any resource string starting with `"files/"`. Strip the `/*` suffix and check `resource.starts_with(prefix)`.
3. `None` resource pattern matches all resources.
4. Exact match takes precedence over wildcard for the purpose of returning a deterministic result, but since all matches here are `Allow`, both return `Allow`. The specificity rule matters when a deny layer is added in a future driver — for now, first match wins in this order: exact → wildcard → None.

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/security/ox_security_authz/tests/integration.rs`:

```rust
use ox_security_authz::{
    drivers::LocalDbAuthzDriver,
    grant::PermissionGrant,
};
use ox_security_core::types::GroupId;
use std::sync::Arc;

fn make_driver(grants: Vec<PermissionGrant>) -> LocalDbAuthzDriver {
    let grants = Arc::new(grants);
    LocalDbAuthzDriver::new(Arc::new(move |_principal_id, _groups| {
        grants.as_ref().clone()
    }))
}

fn principal_with_groups(groups: Vec<&str>) -> Principal {
    Principal {
        id: PrincipalId::new(),
        display_name: "Test".to_string(),
        source: AuthSource::Local,
        groups: groups.into_iter().map(|g| GroupId::new(g)).collect(),
        tenant_id: TenantId::from_str("test").unwrap(),
        session_id: None,
    }
}

// ─── LocalDbAuthzDriver tests ───────────────────────────────────────────────

#[tokio::test]
async fn local_db_allows_direct_grant() {
    let driver = make_driver(vec![
        PermissionGrant {
            operation: "read".to_string(),
            resource_pattern: Some("docs/spec.md".to_string()),
        },
    ]);
    let principal = test_principal();
    let result = driver.check(&principal, "docs/spec.md", "read").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn local_db_allows_via_group() {
    // The lookup fn receives the principal's groups; here we simulate that
    // the injected fn returns grants for the group by inspecting the groups slice.
    let driver = {
        LocalDbAuthzDriver::new(Arc::new(|_principal_id, groups: &[GroupId]| {
            if groups.iter().any(|g| g.as_str() == "admins") {
                vec![PermissionGrant {
                    operation: "write".to_string(),
                    resource_pattern: None,
                }]
            } else {
                vec![]
            }
        }))
    };
    let principal = principal_with_groups(vec!["admins"]);
    let result = driver.check(&principal, "any/resource", "write").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn local_db_denies_missing_grant() {
    // No grant for this operation → Continue (not Deny), so the next driver can decide.
    let driver = make_driver(vec![
        PermissionGrant {
            operation: "read".to_string(),
            resource_pattern: None,
        },
    ]);
    let principal = test_principal();
    let result = driver.check(&principal, "any/resource", "delete").await;
    assert!(matches!(result, AuthzResult::Continue));
}

#[tokio::test]
async fn local_db_wildcard_resource_match() {
    let driver = make_driver(vec![
        PermissionGrant {
            operation: "read".to_string(),
            resource_pattern: Some("files/*".to_string()),
        },
    ]);
    let principal = test_principal();
    let result = driver.check(&principal, "files/readme.txt", "read").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn local_db_wildcard_does_not_match_outside_prefix() {
    let driver = make_driver(vec![
        PermissionGrant {
            operation: "read".to_string(),
            resource_pattern: Some("files/*".to_string()),
        },
    ]);
    let principal = test_principal();
    // "other/readme.txt" does NOT start with "files/" → no match → Continue
    let result = driver.check(&principal, "other/readme.txt", "read").await;
    assert!(matches!(result, AuthzResult::Continue));
}

#[tokio::test]
async fn local_db_exact_beats_wildcard() {
    // Both an exact grant and a wildcard grant exist for the same operation.
    // The driver should return Allow (both match; exact is checked first,
    // but either would return Allow — this test verifies the driver doesn't panic
    // or return wrong result when multiple grants match).
    let driver = make_driver(vec![
        PermissionGrant {
            operation: "read".to_string(),
            resource_pattern: Some("files/*".to_string()),
        },
        PermissionGrant {
            operation: "read".to_string(),
            resource_pattern: Some("files/readme.txt".to_string()),
        },
    ]);
    let principal = test_principal();
    let result = driver.check(&principal, "files/readme.txt", "read").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn local_db_no_principal_continues() {
    // SecurityContext.principal is None — the driver cannot check, so Continue.
    // The LocalDbAuthzDriver.check() receives a &Principal so it always has one;
    // this test verifies that an empty grants list → Continue (not Deny).
    let driver = make_driver(vec![]);
    let principal = test_principal();
    let result = driver.check(&principal, "any/resource", "read").await;
    assert!(matches!(result, AuthzResult::Continue));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p ox_security_authz local_db 2>&1 | head -20
```

Expected: FAIL — `LocalDbAuthzDriver::new` does not exist, `GrantLookupFn` not defined.

- [ ] **Step 3: Implement `src/drivers/local_db.rs`**

```rust
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthzResult,
    drivers::AuthzDriver,
    principal::Principal,
    types::{GroupId, PrincipalId},
};
use crate::grant::PermissionGrant;

/// Given a principal id and the principal's group memberships, return all
/// PermissionGrants that apply (direct grants + group grants combined).
/// The injected function is responsible for querying whatever backing store
/// (database, in-memory map, etc.) is appropriate for the deployment.
pub type GrantLookupFn =
    Arc<dyn Fn(&PrincipalId, &[GroupId]) -> Vec<PermissionGrant> + Send + Sync>;

pub struct LocalDbAuthzDriver {
    lookup: GrantLookupFn,
}

impl LocalDbAuthzDriver {
    pub fn new(lookup: GrantLookupFn) -> Self {
        Self { lookup }
    }
}

/// Returns true if `resource_pattern` matches `resource`.
///
/// Matching rules:
///   - `None`                 → matches any resource
///   - `Some("files/*")`      → matches any resource starting with `"files/"`
///   - `Some("files/a.txt")`  → matches only the exact string `"files/a.txt"`
fn pattern_matches(resource_pattern: &Option<String>, resource: &str) -> bool {
    match resource_pattern {
        None => true,
        Some(pattern) => {
            if let Some(prefix) = pattern.strip_suffix("/*") {
                // wildcard: resource must start with "<prefix>/"
                resource.starts_with(&format!("{}/", prefix))
            } else {
                // exact match
                resource == pattern.as_str()
            }
        }
    }
}

#[async_trait]
impl AuthzDriver for LocalDbAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        let grants = (self.lookup)(&principal.id, &principal.groups);

        // Evaluate in specificity order: exact match first, then wildcard, then None.
        // Return Allow on the first matching grant for the requested operation.
        // If no grant matches at all, return Continue so the next pipeline driver
        // gets a chance to evaluate.

        // Pass 1: exact resource match
        for grant in &grants {
            if grant.operation != operation {
                continue;
            }
            if let Some(ref pat) = grant.resource_pattern {
                if !pat.ends_with("/*") && pat.as_str() == path {
                    return AuthzResult::Allow;
                }
            }
        }

        // Pass 2: wildcard resource match
        for grant in &grants {
            if grant.operation != operation {
                continue;
            }
            if let Some(ref pat) = grant.resource_pattern {
                if pat.ends_with("/*") {
                    if pattern_matches(&grant.resource_pattern, path) {
                        return AuthzResult::Allow;
                    }
                }
            }
        }

        // Pass 3: operation-only grant (resource_pattern = None → all resources)
        for grant in &grants {
            if grant.operation == operation && grant.resource_pattern.is_none() {
                return AuthzResult::Allow;
            }
        }

        // No matching grant found — let the next driver in the pipeline decide.
        AuthzResult::Continue
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p ox_security_authz 2>&1 | tail -20
```

Expected output:

```
running 11 tests
test empty_pipeline_denies ... ok
test local_db_allows_direct_grant ... ok
test local_db_allows_via_group ... ok
test local_db_denies_missing_grant ... ok
test local_db_exact_beats_wildcard ... ok
test local_db_no_principal_continues ... ok
test local_db_wildcard_does_not_match_outside_prefix ... ok
test local_db_wildcard_resource_match ... ok
test pipeline_continues_through_continues ... ok
test pipeline_stops_at_allow ... ok
test pipeline_stops_at_deny ... ok

test result: ok. 11 passed; 0 failed; 0 ignored
```

- [ ] **Step 5: Commit**

```bash
git add crates/security/ox_security_authz/src/drivers/local_db.rs \
        crates/security/ox_security_authz/tests/integration.rs
git commit -m "feat(security-authz): implement LocalDbAuthzDriver with wildcard resource matching"
```

---

## Task 3: Wire `lib.rs` and verify clean workspace build

**Files:**
- Modify: `crates/security/ox_security_authz/src/lib.rs`

- [ ] **Step 1: Confirm `src/lib.rs` re-exports everything callers need**

The file should already contain this from Task 1. Verify it reads exactly:

```rust
pub mod drivers;
pub mod grant;
pub mod pipeline;

pub use drivers::{AdAuthzDriver, LdapAuthzDriver, LocalDbAuthzDriver, OktaAuthzDriver};
pub use grant::PermissionGrant;
pub use pipeline::AuthzPipeline;
```

If it does not, update it now.

- [ ] **Step 2: Build the entire workspace**

```bash
cargo build 2>&1 | grep "^error" | head -10
```

Expected: zero errors.

- [ ] **Step 3: Run all tests in the workspace**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass with no failures.

- [ ] **Step 4: Commit**

```bash
git add crates/security/ox_security_authz/src/lib.rs
git commit -m "feat(security-authz): complete ox_security_authz — pipeline, LocalDbAuthzDriver, all driver stubs"
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task covering it |
|---|---|
| `AuthzPipeline` chains `Vec<Arc<dyn AuthzDriver>>` | Task 1 — `pipeline.rs` |
| First non-Continue wins | Task 1 — pipeline loop |
| All Continue → Deny (fail-closed) | Task 1 — pipeline fallback |
| `PermissionGrant` struct with `operation` + `resource_pattern` | Task 1 — `grant.rs` |
| `GrantLookupFn` type alias | Task 2 — `local_db.rs` |
| Exact match and `*` wildcard suffix | Task 2 — `pattern_matches` + three-pass eval |
| Group grants via `GroupId` slice | Task 2 — injected fn receives `&[GroupId]` |
| No match → `Continue` (not Deny) | Task 2 — fallback `AuthzResult::Continue` |
| `LdapAuthzDriver` stub | Task 1 |
| `AdAuthzDriver` stub | Task 1 |
| `OktaAuthzDriver` stub | Task 1 |
| `AuthzResult::Continue` needed for pipeline chaining | Task 1 — prerequisite mod to `ox_security_core` |

**Placeholder scan:** No TBDs, no "same pattern as above", no missing code blocks. All stubs written out in full.

**Type consistency:**
- `PermissionGrant` defined in Task 1 (`grant.rs`) and imported in Task 2 tests and `local_db.rs` — field names `operation: String` and `resource_pattern: Option<String>` consistent throughout.
- `GrantLookupFn` defined in Task 2 `local_db.rs` as `Arc<dyn Fn(&PrincipalId, &[GroupId]) -> Vec<PermissionGrant> + Send + Sync>` — matches usage in test helper `make_driver` and `LocalDbAuthzDriver::new`.
- `AuthzResult::Continue` added in Task 1 to `ox_security_core` — used in pipeline, stubs, and `LocalDbAuthzDriver` consistently.
- `AuthzPipeline::check` signature: `(&self, principal: &Principal, path: &str, operation: &str) -> AuthzResult` — matches `AuthzDriver::check` minus the `&self` receiver, consistent with test calls.
- Test helper `test_principal()` defined at top of `integration.rs` and reused in both Task 1 and Task 2 test blocks — correct.
