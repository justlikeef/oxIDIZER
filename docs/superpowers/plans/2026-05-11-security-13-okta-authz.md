# OktaAuthzDriver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `OktaAuthzDriver` stub with a real implementation that queries Okta's REST API for the principal's current group memberships, maps those groups to `PermissionGrant` records via an injected mapper, and evaluates the grants using the same three-pass logic as `LocalDbAuthzDriver`.

**Architecture:** `OktaAuthzDriver` is source-gated — it only evaluates principals whose `source == AuthSource::Okta`; all others immediately return `Continue`. For Okta principals, it calls `GET /api/v1/users/{userId}/groups` on the configured Okta domain, extracts the group names, and feeds them to an injected `OktaGrantMapperFn` to obtain `PermissionGrant` records. Grant evaluation is the standard three-pass exact → wildcard → None logic. The Okta API call is injected as `OktaApiFn` so tests never make real HTTP requests. On API error the driver returns `Continue` (fail-open at the driver level; the pipeline is fail-closed at the system level).

**Tech Stack:** Rust, `ox_security_core` (all shared types), `ox_security_authz` (shared `PermissionGrant`), `async-trait`, `reqwest 0.12` (rustls-tls, json features), `futures` (for `BoxFuture`), `tokio` (dev-dependency)

---

## File Structure

```
crates/security/ox_security_authz/
  Cargo.toml             — add reqwest and futures deps
  src/
    drivers/
      okta.rs            — OktaAuthzDriver, OktaConfig, OktaApiFn, OktaGrantMapperFn
      mod.rs             — update re-exports
    lib.rs               — update re-exports
  tests/
    integration.rs       — append Okta test cases
```

---

## Cargo.toml changes

Add to `[dependencies]` in `crates/security/ox_security_authz/Cargo.toml`:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
futures  = "0.3"
```

The full `Cargo.toml` after changes:

```toml
[package]
name = "ox_security_authz"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]
ox_security_core = { path = "../ox_security_core" }
async-trait      = "0.1"
reqwest          = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
futures          = "0.3"

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt"] }
```

---

## Task 1: Define types, write failing tests, implement `OktaAuthzDriver`

**Files:**
- Modify: `crates/security/ox_security_authz/Cargo.toml`
- Modify: `crates/security/ox_security_authz/src/drivers/okta.rs`
- Modify: `crates/security/ox_security_authz/src/drivers/mod.rs`
- Modify: `crates/security/ox_security_authz/src/lib.rs`
- Modify: `crates/security/ox_security_authz/tests/integration.rs`

- [ ] **Step 1: Update `Cargo.toml`**

Apply the changes shown in the Cargo.toml section above.

- [ ] **Step 2: Write the failing tests**

APPEND to `crates/security/ox_security_authz/tests/integration.rs`:

```rust
// ─── OktaAuthzDriver tests ──────────────────────────────────────────────────

use std::future::Future;
use std::pin::Pin;
use ox_security_authz::drivers::{OktaAuthzDriver, OktaConfig, OktaApiFn, OktaGrantMapperFn};
use ox_security_core::types::{AuthSource, TenantId};
use std::str::FromStr;

fn okta_config() -> OktaConfig {
    OktaConfig {
        domain: "dev-123456.okta.com".to_string(),
        api_token: "test-token".to_string(),
        role_claim_attr: "groups".to_string(),
        tenant_id: TenantId::from_str("test").unwrap(),
    }
}

/// Build an OktaApiFn that always returns a fixed list of group names.
fn fixed_groups_api(groups: Vec<String>) -> OktaApiFn {
    Arc::new(move |_user_id: &str| {
        let groups = groups.clone();
        Box::pin(async move { Ok(groups) })
    })
}

/// Build an OktaApiFn that always returns an error.
fn error_api() -> OktaApiFn {
    Arc::new(|_user_id: &str| {
        Box::pin(async move {
            Err("connection refused".to_string())
        })
    })
}

fn okta_principal() -> Principal {
    Principal {
        id: PrincipalId::new(),
        display_name: "Okta User".to_string(),
        source: AuthSource::Okta,
        groups: vec![],
        tenant_id: TenantId::from_str("test").unwrap(),
        session_id: None,
    }
}

fn local_principal() -> Principal {
    Principal {
        id: PrincipalId::new(),
        display_name: "Local User".to_string(),
        source: AuthSource::Local,
        groups: vec![],
        tenant_id: TenantId::from_str("test").unwrap(),
        session_id: None,
    }
}

#[tokio::test]
async fn okta_authz_continues_for_non_okta_principal() {
    // Non-Okta principals must be skipped immediately (source gate).
    let driver = OktaAuthzDriver::new(
        okta_config(),
        fixed_groups_api(vec!["admins".to_string()]),
        Arc::new(|_groups: &[String]| {
            vec![PermissionGrant {
                operation: "read".to_string(),
                resource_pattern: None,
            }]
        }),
    );
    let result = driver.check(&local_principal(), "any/resource", "read").await;
    assert!(matches!(result, AuthzResult::Continue));
}

#[tokio::test]
async fn okta_authz_allows_when_group_grants_permission() {
    // Okta API returns ["editors"]. Mapper maps "editors" → write grant. Principal is Okta.
    let driver = OktaAuthzDriver::new(
        okta_config(),
        fixed_groups_api(vec!["editors".to_string()]),
        Arc::new(|groups: &[String]| {
            if groups.iter().any(|g| g == "editors") {
                vec![PermissionGrant {
                    operation: "write".to_string(),
                    resource_pattern: Some("docs/*".to_string()),
                }]
            } else {
                vec![]
            }
        }),
    );
    let result = driver.check(&okta_principal(), "docs/spec.md", "write").await;
    assert!(matches!(result, AuthzResult::Allow));
}

#[tokio::test]
async fn okta_authz_continues_when_no_matching_grant() {
    // API returns groups, mapper returns grants, but none match the operation.
    let driver = OktaAuthzDriver::new(
        okta_config(),
        fixed_groups_api(vec!["viewers".to_string()]),
        Arc::new(|_groups: &[String]| {
            vec![PermissionGrant {
                operation: "read".to_string(),
                resource_pattern: None,
            }]
        }),
    );
    let result = driver.check(&okta_principal(), "any/resource", "delete").await;
    assert!(matches!(result, AuthzResult::Continue));
}

#[tokio::test]
async fn okta_authz_continues_on_api_error() {
    // Driver fails open: API error → Continue (pipeline is fail-closed at system level).
    let driver = OktaAuthzDriver::new(
        okta_config(),
        error_api(),
        Arc::new(|_groups: &[String]| {
            vec![PermissionGrant {
                operation: "read".to_string(),
                resource_pattern: None,
            }]
        }),
    );
    let result = driver.check(&okta_principal(), "any/resource", "read").await;
    assert!(matches!(result, AuthzResult::Continue));
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test -p ox_security_authz okta_authz 2>&1 | head -20
```

Expected: FAIL — `OktaAuthzDriver::new` does not accept arguments, `OktaConfig` / `OktaApiFn` / `OktaGrantMapperFn` not defined.

- [ ] **Step 4: Implement `src/drivers/okta.rs`**

```rust
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthzResult,
    drivers::AuthzDriver,
    principal::Principal,
    types::{AuthSource, TenantId},
};
use crate::grant::PermissionGrant;

/// Okta tenant configuration injected into the driver.
#[derive(Clone)]
pub struct OktaConfig {
    /// Okta domain, e.g. `"dev-123456.okta.com"`.
    pub domain: String,
    /// Okta API token (SSWS token). Stored as plain `String` here; callers
    /// should source this from a secrets store, not hardcode it.
    pub api_token: String,
    /// Okta profile attribute whose value contains group/role membership.
    /// Defaults to `"groups"` in most deployments.
    pub role_claim_attr: String,
    /// Tenant this driver is scoped to. Principals from other tenants are skipped.
    pub tenant_id: TenantId,
}

/// Given an Okta user ID, return the list of Okta group names the user belongs to.
/// The future resolves to `Ok(Vec<String>)` on success or `Err(String)` on failure.
///
/// In production, implement this by calling
/// `GET https://{domain}/api/v1/users/{userId}/groups`
/// with the SSWS token in the `Authorization` header and extracting
/// each group's `profile.name` field from the JSON response.
///
/// In tests, inject a closure that returns a fixed list without HTTP.
pub type OktaApiFn = Arc<
    dyn Fn(&str) -> Pin<Box<dyn Future<Output = Result<Vec<String>, String>> + Send>>
        + Send
        + Sync,
>;

/// Given a list of Okta group names, return the `PermissionGrant`s they confer.
/// Implement by mapping Okta group names to operations and resource patterns
/// using whatever policy store the deployment uses (config file, database, etc.).
pub type OktaGrantMapperFn =
    Arc<dyn Fn(&[String]) -> Vec<PermissionGrant> + Send + Sync>;

pub struct OktaAuthzDriver {
    config: OktaConfig,
    api: OktaApiFn,
    mapper: OktaGrantMapperFn,
}

impl OktaAuthzDriver {
    pub fn new(config: OktaConfig, api: OktaApiFn, mapper: OktaGrantMapperFn) -> Self {
        Self { config, api, mapper }
    }
}

#[async_trait]
impl AuthzDriver for OktaAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        // Source gate: only process Okta-authenticated principals.
        if principal.source != AuthSource::Okta {
            return AuthzResult::Continue;
        }

        // Fetch current group memberships from Okta.
        // Use the string representation of the principal's UUID as the Okta user ID.
        let user_id = principal.id.as_uuid().to_string();
        let group_names = match (self.api)(&user_id).await {
            Ok(names) => names,
            Err(_) => {
                // API error → fail open at driver level.
                // The pipeline's fail-closed default catches this if no other driver allows.
                return AuthzResult::Continue;
            }
        };

        // Map Okta group names to PermissionGrant records.
        let grants = (self.mapper)(&group_names);

        // Three-pass evaluation: exact match → wildcard → None.
        // Identical logic to LocalDbAuthzDriver.

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

- [ ] **Step 5: Update `src/drivers/mod.rs`**

```rust
pub(crate) mod ad;
pub(crate) mod ldap;
pub(crate) mod local_db;
pub(crate) mod okta;

pub use ad::AdAuthzDriver;
pub use ldap::{LdapAuthzDriver, GroupResolverFn};
pub use local_db::LocalDbAuthzDriver;
pub use okta::{OktaAuthzDriver, OktaApiFn, OktaConfig, OktaGrantMapperFn};
```

- [ ] **Step 6: Update `src/lib.rs`**

```rust
pub(crate) mod drivers;
pub(crate) mod grant;
pub(crate) mod pipeline;

pub use drivers::{
    AdAuthzDriver, GroupResolverFn, LdapAuthzDriver, LocalDbAuthzDriver,
    OktaAuthzDriver, OktaApiFn, OktaConfig, OktaGrantMapperFn,
};
pub use grant::PermissionGrant;
pub use pipeline::AuthzPipeline;
```

- [ ] **Step 7: Run Okta tests to verify they pass**

```bash
cargo test -p ox_security_authz okta_authz 2>&1 | tail -15
```

Expected output:

```
running 4 tests
test okta_authz_allows_when_group_grants_permission ... ok
test okta_authz_continues_for_non_okta_principal ... ok
test okta_authz_continues_on_api_error ... ok
test okta_authz_continues_when_no_matching_grant ... ok

test result: ok. 4 passed; 0 failed; 0 ignored
```

- [ ] **Step 8: Run all authz tests to verify no regressions**

```bash
cargo test -p ox_security_authz 2>&1 | tail -30
```

Expected: all tests pass (pipeline, LocalDbAuthzDriver, LdapAuthzDriver, AdAuthzDriver, OktaAuthzDriver).

- [ ] **Step 9: Build the entire workspace**

```bash
cargo build 2>&1 | grep "^error" | head -10
```

Expected: zero errors.

- [ ] **Step 10: Commit**

```bash
git add crates/security/ox_security_authz/Cargo.toml \
        crates/security/ox_security_authz/src/drivers/okta.rs \
        crates/security/ox_security_authz/src/drivers/mod.rs \
        crates/security/ox_security_authz/src/lib.rs \
        crates/security/ox_security_authz/tests/integration.rs
git commit -m "feat(security-authz): implement OktaAuthzDriver with source gate, injected API fn, group-to-grant mapper"
```

---

## Production integration notes (not part of this plan — reference only)

When wiring `OktaAuthzDriver` in a real deployment, the `OktaApiFn` should be built using `reqwest`:

```rust
use reqwest::Client;
use std::sync::Arc;

fn production_okta_api(domain: String, api_token: String) -> OktaApiFn {
    let client = Client::new();
    Arc::new(move |user_id: &str| {
        let client = client.clone();
        let url = format!("https://{}/api/v1/users/{}/groups", domain, user_id);
        let token = api_token.clone();
        Box::pin(async move {
            let resp = client
                .get(&url)
                .header("Authorization", format!("SSWS {}", token))
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if !resp.status().is_success() {
                return Err(format!("Okta API returned {}", resp.status()));
            }

            // Okta groups response: array of objects with profile.name
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let names = body
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|g| g["profile"]["name"].as_str().map(|s| s.to_string()))
                .collect();
            Ok(names)
        })
    })
}
```

This function is intentionally kept out of the driver crate itself — it belongs in the integration/wiring layer that knows about HTTP clients and secret management.

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task covering it |
|---|---|
| `OktaAuthzDriver` queries Okta REST API for group membership | Task 1 — `OktaApiFn` injected fn, `GET /api/v1/users/{userId}/groups` documented |
| Source gate: only evaluate Okta principals | Task 1 — `if principal.source != AuthSource::Okta { return Continue }` |
| Map group names to grants via injected mapper | Task 1 — `OktaGrantMapperFn` |
| Same three-pass eval as LocalDbAuthzDriver | Task 1 — identical pass 1/2/3 logic |
| `reqwest` dep with rustls-tls | Cargo.toml changes |
| `OktaApiFn` injected for testability | Task 1 — `fixed_groups_api`, `error_api` test helpers |
| Fail-open on API error at driver level | Task 1 — `Err(_) => return AuthzResult::Continue` |
| `okta_authz_continues_for_non_okta_principal` | Task 1 — source gate test |
| `okta_authz_allows_when_group_grants_permission` | Task 1 |
| `okta_authz_continues_when_no_matching_grant` | Task 1 |
| `okta_authz_continues_on_api_error` | Task 1 |

**Placeholder scan:** No TBDs. `OktaConfig.api_token` is `String` (not a secret wrapper type) to avoid pulling in a secrets crate dependency — the production integration note advises callers to source it from a secrets store.

**Type consistency:**
- `OktaApiFn`: `Arc<dyn Fn(&str) -> Pin<Box<dyn Future<Output = Result<Vec<String>, String>> + Send>> + Send + Sync>` — matches both test helpers (`fixed_groups_api`, `error_api`) and the production snippet.
- `OktaGrantMapperFn`: `Arc<dyn Fn(&[String]) -> Vec<PermissionGrant> + Send + Sync>` — matches test closures.
- `PermissionGrant` imported from `crate::grant` — same struct used in `LocalDbAuthzDriver`, `LdapAuthzDriver`, `AdAuthzDriver`.
- `AuthSource::Okta` variant exists in `ox_security_core::types::AuthSource` — confirmed in `types.rs`.
