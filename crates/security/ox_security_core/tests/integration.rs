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

use ox_security_core::principal::{PartialPrincipal, Principal};
use ox_security_core::types::GroupId;

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

use ox_security_core::context::{AuthPipelineContext, SecurityContext};
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
