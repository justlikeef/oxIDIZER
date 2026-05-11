use std::sync::Arc;
use async_trait::async_trait;
use ox_security_authz::AuthzPipeline;
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

// ─── LocalDbAuthzDriver tests ───────────────────────────────────────────────

use ox_security_authz::{LocalDbAuthzDriver, PermissionGrant};

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
