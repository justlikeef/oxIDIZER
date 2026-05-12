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
    // An empty grants list means the driver has nothing to match against,
    // so it returns Continue to let the next pipeline driver decide.
    let driver = make_driver(vec![]);
    let principal = test_principal();
    let result = driver.check(&principal, "any/resource", "read").await;
    assert!(matches!(result, AuthzResult::Continue));
}

// ─── LdapAuthzDriver tests ──────────────────────────────────────────────────

use ox_security_authz::{LdapAuthzDriver, GroupResolverFn};

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

// ─── AdAuthzDriver tests ────────────────────────────────────────────────────

use ox_security_authz::AdAuthzDriver;

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

// ─── OktaAuthzDriver tests ──────────────────────────────────────────────────

use ox_security_authz::{OktaAuthzDriver, OktaConfig, OktaApiFn, OktaGrantMapperFn};

fn okta_config() -> OktaConfig {
    OktaConfig {
        domain: "dev-123456.okta.com".to_string(),
        api_token: "test-token".to_string(),
        role_claim_attr: "groups".to_string(),
        tenant_id: TenantId::from_str("test").unwrap(),
    }
}

fn fixed_groups_api(groups: Vec<String>) -> OktaApiFn {
    Arc::new(move |_user_id: &str| {
        let groups = groups.clone();
        Box::pin(async move { Ok(groups) })
    })
}

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

fn local_principal_okta() -> Principal {
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
    let result = driver.check(&local_principal_okta(), "any/resource", "read").await;
    assert!(matches!(result, AuthzResult::Continue));
}

#[tokio::test]
async fn okta_authz_allows_when_group_grants_permission() {
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
