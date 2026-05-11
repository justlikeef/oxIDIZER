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

use ox_security_core::types::{AuthSource, PrincipalId, TenantId};

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
