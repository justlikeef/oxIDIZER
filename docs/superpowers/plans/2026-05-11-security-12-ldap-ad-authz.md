# LdapAuthzDriver and AdAuthzDriver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `LdapAuthzDriver` and `AdAuthzDriver` stubs with full implementations that use the same injected `GrantLookupFn` pattern as `LocalDbAuthzDriver`, plus a `GroupResolverFn` that expands nested/transitive group membership before grant evaluation.

**Architecture:** Both drivers are datasource-agnostic. The "LDAP"/"AD" naming reflects the intended backing persistence driver (injected via `GrantLookupFn`), not any protocol implementation in this crate. The key difference from `LocalDbAuthzDriver` is the addition of a `GroupResolverFn` that takes the principal's direct group memberships and returns the transitive closure (all ancestor groups). Grant evaluation after expansion is identical to `LocalDbAuthzDriver`'s three-pass logic (exact → wildcard → None). `AdAuthzDriver` is structurally identical to `LdapAuthzDriver`; the injected `GroupResolverFn` handles any AD-specific traversal in the calling code.

**Tech Stack:** Rust, `ox_security_core` (all shared types), `ox_security_authz` (shared `GrantLookupFn`, `PermissionGrant`), `async-trait`, `tokio` (dev-dependency)

---

## File Structure

```
crates/security/ox_security_authz/
  src/
    drivers/
      ldap.rs          — LdapAuthzDriver: GrantLookupFn + GroupResolverFn, three-pass eval
      ad.rs            — AdAuthzDriver: identical to LdapAuthzDriver
      mod.rs           — no change needed (already exports both)
    grant.rs           — no change (PermissionGrant already defined)
  tests/
    integration.rs     — append LDAP and AD test cases
```

No new crates. No `Cargo.toml` changes (no new deps).

---

## Shared type: `GroupResolverFn`

This type will be defined once in `src/drivers/ldap.rs` and re-exported from `src/drivers/mod.rs`. `AdAuthzDriver` imports it from `ldap.rs` (or from the crate root re-export — whichever is cleaner).

```rust
pub type GroupResolverFn = Arc<dyn Fn(&[GroupId]) -> Vec<GroupId> + Send + Sync>;
```

The function receives the principal's direct group memberships and returns the transitive closure — i.e., direct groups plus all ancestor groups. A no-op resolver simply returns a clone of the input.

---

## Task 1: `LdapAuthzDriver` — tests, then implementation

**Files:**
- Modify: `crates/security/ox_security_authz/src/drivers/ldap.rs`
- Modify: `crates/security/ox_security_authz/src/drivers/mod.rs`
- Modify: `crates/security/ox_security_authz/tests/integration.rs`

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/security/ox_security_authz/tests/integration.rs`:

```rust
// ─── LdapAuthzDriver tests ──────────────────────────────────────────────────

use ox_security_authz::drivers::{LdapAuthzDriver, GroupResolverFn};

fn identity_resolver() -> GroupResolverFn {
    Arc::new(|groups: &[GroupId]| groups.to_vec())
}

#[tokio::test]
async fn ldap_authz_allows_direct_grant() {
    // Principal has a direct PermissionGrant for the requested operation + resource.
    let grants = Arc::new(vec![PermissionGrant {
        operation: "read".to_string(),
        resource_pattern: Some("docs/spec.md".to_string()),
    }]);
    let driver = LdapAuthzDriver::new(
        Arc::new(move |_id, _groups| grants.as_ref().clone()),
        identity_resolver(),
    );
    let principal = test_principal();
    let result = driver.check(&principal, "docs/spec.md", "read").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn ldap_authz_allows_via_group() {
    // The lookup fn returns grants when the principal belongs to "readers".
    let driver = LdapAuthzDriver::new(
        Arc::new(|_id, groups: &[GroupId]| {
            if groups.iter().any(|g| g.as_str() == "readers") {
                vec![PermissionGrant {
                    operation: "read".to_string(),
                    resource_pattern: None,
                }]
            } else {
                vec![]
            }
        }),
        identity_resolver(),
    );
    let principal = principal_with_groups(vec!["readers"]);
    let result = driver.check(&principal, "any/resource", "read").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn ldap_authz_allows_via_nested_group() {
    // Principal is a direct member of group B only.
    // GroupResolverFn expands B → [B, A].
    // The grant is attached to group A.
    // The lookup fn is called with expanded groups [B, A] and returns the grant.
    let driver = LdapAuthzDriver::new(
        Arc::new(|_id, groups: &[GroupId]| {
            // Grant is on group A
            if groups.iter().any(|g| g.as_str() == "group-a") {
                vec![PermissionGrant {
                    operation: "write".to_string(),
                    resource_pattern: None,
                }]
            } else {
                vec![]
            }
        }),
        // Resolver: group-b is nested under group-a
        Arc::new(|groups: &[GroupId]| {
            let mut expanded = groups.to_vec();
            if groups.iter().any(|g| g.as_str() == "group-b") {
                expanded.push(GroupId::new("group-a"));
            }
            expanded
        }),
    );
    // Principal is only in group-b; resolver adds group-a
    let principal = principal_with_groups(vec!["group-b"]);
    let result = driver.check(&principal, "any/resource", "write").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn ldap_authz_continues_for_no_match() {
    // No grant matches — driver returns Continue, not Deny.
    let driver = LdapAuthzDriver::new(
        Arc::new(|_id, _groups| vec![]),
        identity_resolver(),
    );
    let principal = test_principal();
    let result = driver.check(&principal, "any/resource", "delete").await;
    assert!(matches!(result, AuthzResult::Continue));
}

#[tokio::test]
async fn ldap_authz_without_group_resolution_uses_direct_groups() {
    // without_group_resolution convenience constructor passes groups through unchanged.
    let driver = LdapAuthzDriver::without_group_resolution(Arc::new(|_id, groups: &[GroupId]| {
        if groups.iter().any(|g| g.as_str() == "admins") {
            vec![PermissionGrant {
                operation: "delete".to_string(),
                resource_pattern: None,
            }]
        } else {
            vec![]
        }
    }));
    let principal = principal_with_groups(vec!["admins"]);
    let result = driver.check(&principal, "files/readme.txt", "delete").await;
    assert!(matches!(result, AuthzResult::Allow));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p ox_security_authz ldap_authz 2>&1 | head -20
```

Expected: FAIL — `LdapAuthzDriver::new` does not accept arguments, `GroupResolverFn` not defined.

- [ ] **Step 3: Implement `src/drivers/ldap.rs`**

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

/// Given a principal id and the principal's (possibly expanded) group memberships,
/// return all PermissionGrants that apply. Identical contract to LocalDbAuthzDriver.
pub(crate) type GrantLookupFn =
    Arc<dyn Fn(&PrincipalId, &[GroupId]) -> Vec<PermissionGrant> + Send + Sync>;

/// Given the principal's direct group memberships, return the transitive closure
/// (direct groups plus all ancestor groups reachable through nesting).
/// For a flat directory, return a clone of the input unchanged.
pub type GroupResolverFn = Arc<dyn Fn(&[GroupId]) -> Vec<GroupId> + Send + Sync>;

pub struct LdapAuthzDriver {
    lookup: GrantLookupFn,
    resolve_groups: GroupResolverFn,
}

impl LdapAuthzDriver {
    /// Full constructor: supply both a grant lookup function and a group resolver.
    /// The group resolver expands nested group membership before the lookup is called.
    pub fn new(lookup: GrantLookupFn, resolve_groups: GroupResolverFn) -> Self {
        Self { lookup, resolve_groups }
    }

    /// Convenience constructor for deployments where LDAP groups are flat (no nesting).
    /// The resolver is the identity function — groups are passed through unchanged.
    pub fn without_group_resolution(lookup: GrantLookupFn) -> Self {
        Self {
            lookup,
            resolve_groups: Arc::new(|groups: &[GroupId]| groups.to_vec()),
        }
    }
}

#[async_trait]
impl AuthzDriver for LdapAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        // Expand group memberships to transitive closure before lookup.
        let expanded_groups = (self.resolve_groups)(&principal.groups);
        let grants = (self.lookup)(&principal.id, &expanded_groups);

        // Three-pass evaluation: exact match → wildcard → None.
        // Mirrors LocalDbAuthzDriver exactly.

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
                if let Some(prefix) = pat.strip_suffix("/*") {
                    if path.len() > prefix.len()
                        && path.starts_with(prefix)
                        && path.as_bytes()[prefix.len()] == b'/'
                    {
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

        // No matching grant — let the next driver in the pipeline decide.
        AuthzResult::Continue
    }
}
```

- [ ] **Step 4: Export `GroupResolverFn` from `src/drivers/mod.rs`**

Edit `crates/security/ox_security_authz/src/drivers/mod.rs` to add the re-export:

```rust
pub(crate) mod ad;
pub(crate) mod ldap;
pub(crate) mod local_db;
pub(crate) mod okta;

pub use ad::AdAuthzDriver;
pub use ldap::{LdapAuthzDriver, GroupResolverFn};
pub use local_db::LocalDbAuthzDriver;
pub use okta::OktaAuthzDriver;
```

Also update `crates/security/ox_security_authz/src/lib.rs` to re-export `GroupResolverFn` at the crate root:

```rust
pub(crate) mod drivers;
pub(crate) mod grant;
pub(crate) mod pipeline;

pub use drivers::{AdAuthzDriver, GroupResolverFn, LdapAuthzDriver, LocalDbAuthzDriver, OktaAuthzDriver};
pub use grant::PermissionGrant;
pub use pipeline::AuthzPipeline;
```

- [ ] **Step 5: Run LDAP tests to verify they pass**

```bash
cargo test -p ox_security_authz ldap_authz 2>&1 | tail -15
```

Expected output:

```
running 5 tests
test ldap_authz_allows_direct_grant ... ok
test ldap_authz_allows_via_group ... ok
test ldap_authz_allows_via_nested_group ... ok
test ldap_authz_continues_for_no_match ... ok
test ldap_authz_without_group_resolution_uses_direct_groups ... ok

test result: ok. 5 passed; 0 failed; 0 ignored
```

- [ ] **Step 6: Commit**

```bash
git add crates/security/ox_security_authz/src/drivers/ldap.rs \
        crates/security/ox_security_authz/src/drivers/mod.rs \
        crates/security/ox_security_authz/src/lib.rs \
        crates/security/ox_security_authz/tests/integration.rs
git commit -m "feat(security-authz): implement LdapAuthzDriver with GrantLookupFn and GroupResolverFn"
```

---

## Task 2: `AdAuthzDriver` — tests, then implementation

**Files:**
- Modify: `crates/security/ox_security_authz/src/drivers/ad.rs`
- Modify: `crates/security/ox_security_authz/tests/integration.rs`

`AdAuthzDriver` is structurally identical to `LdapAuthzDriver`. The distinction (AD vs LDAP directory semantics, tokenGroups attribute, SID-based nesting) lives entirely in the injected `GroupResolverFn` supplied by the caller, not in the driver itself.

- [ ] **Step 1: Write the failing tests**

APPEND to `crates/security/ox_security_authz/tests/integration.rs`:

```rust
// ─── AdAuthzDriver tests ────────────────────────────────────────────────────

use ox_security_authz::drivers::AdAuthzDriver;

#[tokio::test]
async fn ad_authz_allows_direct_grant() {
    let grants = Arc::new(vec![PermissionGrant {
        operation: "read".to_string(),
        resource_pattern: Some("shares/finance".to_string()),
    }]);
    let driver = AdAuthzDriver::new(
        Arc::new(move |_id, _groups| grants.as_ref().clone()),
        identity_resolver(),
    );
    let principal = test_principal();
    let result = driver.check(&principal, "shares/finance", "read").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn ad_authz_allows_via_nested_group() {
    // Principal belongs to "Domain Users"; resolver adds "Finance-Readers" as a parent.
    let driver = AdAuthzDriver::new(
        Arc::new(|_id, groups: &[GroupId]| {
            if groups.iter().any(|g| g.as_str() == "Finance-Readers") {
                vec![PermissionGrant {
                    operation: "read".to_string(),
                    resource_pattern: Some("shares/finance/*".to_string()),
                }]
            } else {
                vec![]
            }
        }),
        Arc::new(|groups: &[GroupId]| {
            let mut expanded = groups.to_vec();
            if groups.iter().any(|g| g.as_str() == "Domain Users") {
                expanded.push(GroupId::new("Finance-Readers"));
            }
            expanded
        }),
    );
    let principal = principal_with_groups(vec!["Domain Users"]);
    let result = driver.check(&principal, "shares/finance/q1.xlsx", "read").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn ad_authz_continues_for_no_match() {
    let driver = AdAuthzDriver::new(
        Arc::new(|_id, _groups| vec![]),
        identity_resolver(),
    );
    let principal = test_principal();
    let result = driver.check(&principal, "any/resource", "write").await;
    assert!(matches!(result, AuthzResult::Continue));
}

#[tokio::test]
async fn ad_authz_without_group_resolution_uses_direct_groups() {
    let driver = AdAuthzDriver::without_group_resolution(Arc::new(|_id, groups: &[GroupId]| {
        if groups.iter().any(|g| g.as_str() == "Operators") {
            vec![PermissionGrant {
                operation: "execute".to_string(),
                resource_pattern: None,
            }]
        } else {
            vec![]
        }
    }));
    let principal = principal_with_groups(vec!["Operators"]);
    let result = driver.check(&principal, "scripts/deploy.sh", "execute").await;
    assert!(matches!(result, AuthzResult::Allow));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p ox_security_authz ad_authz 2>&1 | head -20
```

Expected: FAIL — `AdAuthzDriver::new` does not accept arguments.

- [ ] **Step 3: Implement `src/drivers/ad.rs`**

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
use super::ldap::{GrantLookupFn, GroupResolverFn};

/// Authorization driver backed by an Active Directory persistence layer.
///
/// Structurally identical to `LdapAuthzDriver`. The AD-specific behaviour
/// (tokenGroups traversal, SID-based resolution, etc.) is the caller's
/// responsibility via the injected `GroupResolverFn`.
pub struct AdAuthzDriver {
    lookup: GrantLookupFn,
    resolve_groups: GroupResolverFn,
}

impl AdAuthzDriver {
    /// Full constructor: supply a grant lookup function and a group resolver.
    pub fn new(lookup: GrantLookupFn, resolve_groups: GroupResolverFn) -> Self {
        Self { lookup, resolve_groups }
    }

    /// Convenience constructor for deployments where AD groups are flat.
    /// The resolver is the identity function.
    pub fn without_group_resolution(lookup: GrantLookupFn) -> Self {
        Self {
            lookup,
            resolve_groups: Arc::new(|groups: &[GroupId]| groups.to_vec()),
        }
    }
}

#[async_trait]
impl AuthzDriver for AdAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        let expanded_groups = (self.resolve_groups)(&principal.groups);
        let grants = (self.lookup)(&principal.id, &expanded_groups);

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
                if let Some(prefix) = pat.strip_suffix("/*") {
                    if path.len() > prefix.len()
                        && path.starts_with(prefix)
                        && path.as_bytes()[prefix.len()] == b'/'
                    {
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

        AuthzResult::Continue
    }
}
```

- [ ] **Step 4: Export `AdAuthzDriver` with updated `mod.rs`**

`crates/security/ox_security_authz/src/drivers/mod.rs` already exports `AdAuthzDriver`. Verify it still compiles cleanly — no change needed unless the visibility of `GrantLookupFn`/`GroupResolverFn` requires adjustment. The `super::ldap::` import in `ad.rs` accesses those types through the sibling module without requiring them to be `pub` at the `mod.rs` level.

If the compiler rejects `super::ldap::GrantLookupFn` due to `pub(crate)` visibility, add explicit `pub(super)` to both type aliases in `ldap.rs`:

```rust
pub(super) type GrantLookupFn = ...;
pub type GroupResolverFn = ...;
```

- [ ] **Step 5: Run all authz tests to verify they pass**

```bash
cargo test -p ox_security_authz 2>&1 | tail -25
```

Expected output (all tests including Task 1 LDAP tests):

```
running 16 tests
test ad_authz_allows_direct_grant ... ok
test ad_authz_allows_via_nested_group ... ok
test ad_authz_continues_for_no_match ... ok
test ad_authz_without_group_resolution_uses_direct_groups ... ok
test empty_pipeline_denies ... ok
test ldap_authz_allows_direct_grant ... ok
test ldap_authz_allows_via_group ... ok
test ldap_authz_allows_via_nested_group ... ok
test ldap_authz_continues_for_no_match ... ok
test ldap_authz_without_group_resolution_uses_direct_groups ... ok
test local_db_allows_direct_grant ... ok
test local_db_allows_via_group ... ok
test local_db_denies_missing_grant ... ok
test local_db_exact_beats_wildcard ... ok
test local_db_no_principal_continues ... ok
test local_db_wildcard_does_not_match_outside_prefix ... ok

test result: ok. 16 passed; 0 failed; 0 ignored
```

- [ ] **Step 6: Build the entire workspace**

```bash
cargo build 2>&1 | grep "^error" | head -10
```

Expected: zero errors.

- [ ] **Step 7: Commit**

```bash
git add crates/security/ox_security_authz/src/drivers/ad.rs \
        crates/security/ox_security_authz/tests/integration.rs
git commit -m "feat(security-authz): implement AdAuthzDriver — identical to LdapAuthzDriver, injected GroupResolverFn"
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task covering it |
|---|---|
| LDAP/AD drivers use same `GrantLookupFn` pattern as `LocalDbAuthzDriver` | Task 1 + Task 2 |
| Backing store is datasource-agnostic (LDAP naming = persistence driver selection only) | Design note in Architecture section; no protocol code in driver |
| Nested/hierarchical group expansion via `GroupResolverFn` | Task 1 — `GroupResolverFn` type; `ldap_authz_allows_via_nested_group` test |
| `without_group_resolution` convenience constructor | Task 1 — `LdapAuthzDriver::without_group_resolution`; Task 2 — `AdAuthzDriver::without_group_resolution` |
| AD driver code identical to LDAP driver code | Task 2 — `AdAuthzDriver` delegates to same three-pass logic |
| No match → `Continue` (not Deny) | Both drivers — fallback `AuthzResult::Continue` |
| Group resolver receives direct memberships, returns transitive closure | `GroupResolverFn` contract + `ldap_authz_allows_via_nested_group` test |

**Placeholder scan:** No TBDs. All code blocks are complete and compilable. Test helper functions (`test_principal`, `principal_with_groups`, `identity_resolver`) referenced in new tests are either defined in the existing `integration.rs` (first two) or added inline above (third).

**Type consistency:**
- `GrantLookupFn`: defined in `ldap.rs`, imported in `ad.rs` via `super::ldap::GrantLookupFn` — same `Arc<dyn Fn(&PrincipalId, &[GroupId]) -> Vec<PermissionGrant> + Send + Sync>` signature throughout.
- `GroupResolverFn`: defined in `ldap.rs`, exported from `mod.rs` and crate root — `Arc<dyn Fn(&[GroupId]) -> Vec<GroupId> + Send + Sync>`.
- Both drivers pass `&expanded_groups` (the resolved `Vec<GroupId>`) to the `GrantLookupFn` — the lookup therefore sees the full transitive group set, not just direct memberships.
