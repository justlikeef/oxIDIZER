use ox_security_auth::AuthPipeline;
use ox_security_core::{
    Credentials, AuthResult, Principal, AuthSource, PrincipalId, TenantId,
    AuthPipelineContext,
};

use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::drivers::AuthDriver;

struct AlwaysRejectDriver;

#[async_trait]
impl AuthDriver for AlwaysRejectDriver {
    async fn authenticate(&self, _creds: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Reject("rejected".to_string())
    }
}

struct AlwaysContinueDriver;

#[async_trait]
impl AuthDriver for AlwaysContinueDriver {
    async fn authenticate(&self, _creds: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Continue
    }
}

fn test_ctx() -> AuthPipelineContext {
    AuthPipelineContext {
        partial_principal: None,
        tenant_id: TenantId::from_str("test").unwrap(),
        source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    }
}

#[tokio::test]
async fn empty_pipeline_rejects() {
    let pipeline = AuthPipeline::new(vec![]);
    let creds = Credentials::UsernamePassword {
        username: "user".to_string(),
        password: "pass".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = pipeline.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}

#[tokio::test]
async fn pipeline_stops_at_reject() {
    let pipeline = AuthPipeline::new(vec![
        Arc::new(AlwaysRejectDriver),
        Arc::new(AlwaysContinueDriver),
    ]);
    let creds = Credentials::UsernamePassword {
        username: "u".to_string(),
        password: "p".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = pipeline.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}

#[tokio::test]
async fn pipeline_continues_through_misses() {
    struct AlwaysAuthDriver;
    #[async_trait]
    impl AuthDriver for AlwaysAuthDriver {
        async fn authenticate(&self, _c: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
            AuthResult::Authenticated(Principal {
                id: PrincipalId::new(),
                display_name: "auto".to_string(),
                source: AuthSource::Local,
                groups: vec![],
                tenant_id: TenantId::from_str("test").unwrap(),
                session_id: None,
            })
        }
    }

    let pipeline = AuthPipeline::new(vec![
        Arc::new(AlwaysContinueDriver),
        Arc::new(AlwaysContinueDriver),
        Arc::new(AlwaysAuthDriver),
    ]);
    let creds = Credentials::UsernamePassword {
        username: "u".to_string(),
        password: "p".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = pipeline.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Authenticated(_)));
}

use ox_security_auth::DbAuthDriver;
use secrecy::SecretString;

fn make_db_driver(username: &str, password: &str) -> DbAuthDriver {
    let u = username.to_string();
    let stored = password.to_string();
    DbAuthDriver::new(
        TenantId::from_str("test").unwrap(),
        Arc::new(move |user: &str| {
            if user == u { Some(stored.clone()) } else { None }
        }),
        Arc::new(|plaintext: &str, hash: &str| plaintext == hash),
    )
}

#[tokio::test]
async fn db_driver_authenticates_valid_user() {
    let driver = make_db_driver("alice", "secret");
    let creds = Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: "secret".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    match result {
        AuthResult::Authenticated(p) => assert_eq!(p.display_name, "alice"),
        _ => panic!("expected Authenticated"),
    }
}

#[tokio::test]
async fn db_driver_rejects_wrong_password() {
    let driver = make_db_driver("alice", "secret");
    let creds = Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: "wrong".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}

#[tokio::test]
async fn db_driver_continues_for_unknown_user() {
    let driver = make_db_driver("alice", "secret");
    let creds = Credentials::UsernamePassword {
        username: "nobody".to_string(),
        password: "anything".to_string().into(),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Continue));
}

#[tokio::test]
async fn db_driver_continues_for_non_password_credentials() {
    let driver = make_db_driver("alice", "secret");
    let creds = Credentials::ApiKey { key: "somekey".to_string().into() };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Continue));
}

// ── LDAP / AD tests ────────────────────────────────────────────────────────

use ox_security_auth::{LdapAuthDriver, LdapConfig, AdAuthDriver, AdConfig};
use ox_security_auth::drivers::ldap::{LdapBindResult, MockLdapAdapter};
use ox_security_auth::drivers::ad::BindDnCapture;

fn ldap_config() -> LdapConfig {
    LdapConfig {
        url: "ldap://localhost:389".to_string(),
        bind_dn_template: "uid={},ou=users,dc=example,dc=com".to_string(),
        base_dn: "dc=example,dc=com".to_string(),
        group_attr: "memberOf".to_string(),
        tenant_id: TenantId::from_str("test").unwrap(),
    }
}

fn ad_config() -> AdConfig {
    AdConfig {
        ldap: ldap_config(),
        domain: "EXAMPLE".to_string(),
        upn_suffix: "example.com".to_string(),
    }
}

#[tokio::test]
async fn ldap_driver_continues_for_non_password_creds() {
    let driver = LdapAuthDriver::with_mock(
        ldap_config(),
        MockLdapAdapter::new(LdapBindResult::Success { groups: vec![] }),
    );
    let creds = Credentials::ApiKey { key: SecretString::new("key123".to_string()) };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Continue));
}

#[tokio::test]
async fn ldap_driver_authenticates_valid_user() {
    let groups = vec!["cn=admins,dc=example,dc=com".to_string()];
    let driver = LdapAuthDriver::with_mock(
        ldap_config(),
        MockLdapAdapter::new(LdapBindResult::Success { groups: groups.clone() }),
    );
    let creds = Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: SecretString::new("correct".to_string()),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    match result {
        AuthResult::Authenticated(p) => {
            assert_eq!(p.display_name, "alice");
            assert_eq!(p.groups, vec![ox_security_core::GroupId::new("cn=admins,dc=example,dc=com")]);
            assert_eq!(p.source, AuthSource::Ldap);
            assert_eq!(p.tenant_id.as_str(), "test");
        }
        _ => panic!("expected Authenticated"),
    }
}

#[tokio::test]
async fn ldap_driver_rejects_bad_password() {
    let driver = LdapAuthDriver::with_mock(
        ldap_config(),
        MockLdapAdapter::new(LdapBindResult::InvalidCredentials),
    );
    let creds = Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: SecretString::new("wrong".to_string()),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}

#[tokio::test]
async fn ldap_driver_rejects_unknown_user() {
    let driver = LdapAuthDriver::with_mock(
        ldap_config(),
        MockLdapAdapter::new(LdapBindResult::NoSuchEntry),
    );
    let creds = Credentials::UsernamePassword {
        username: "ghost".to_string(),
        password: SecretString::new("anything".to_string()),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}

#[tokio::test]
async fn ad_driver_tries_upn_format() {
    let capture = BindDnCapture::new_sequence(vec![
        LdapBindResult::InvalidCredentials,
        LdapBindResult::InvalidCredentials,
        LdapBindResult::Success { groups: vec![] },
    ]);
    let driver = AdAuthDriver::with_mock(ad_config(), capture.clone());
    let creds = Credentials::UsernamePassword {
        username: "bob".to_string(),
        password: SecretString::new("pass".to_string()),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Authenticated(_)));
    let attempted = capture.last_bind_dn();
    assert_eq!(attempted.len(), 3, "expected all three DN forms to be tried");
    assert!(!attempted.is_empty(), "expected at least one bind attempt");
    assert!(attempted.iter().any(|dn: &String| dn.ends_with("@example.com")), "expected UPN form to be tried");
}

#[tokio::test]
async fn ad_driver_authenticates_via_domain_prefix() {
    let capture = BindDnCapture::new_sequence(vec![
        LdapBindResult::InvalidCredentials,
        LdapBindResult::Success { groups: vec![] },
    ]);
    let driver = AdAuthDriver::with_mock(ad_config(), capture.clone());
    let creds = Credentials::UsernamePassword {
        username: "carol".to_string(),
        password: SecretString::new("pass".to_string()),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Authenticated(_)));
    let attempted = capture.last_bind_dn();
    assert!(
        attempted.iter().any(|dn| dn.starts_with("EXAMPLE\\")),
        "expected DOMAIN\\user attempt, got: {:?}", attempted
    );
}

#[tokio::test]
async fn ldap_driver_continues_on_adapter_error() {
    let mock = MockLdapAdapter::new(LdapBindResult::Error("connection refused".to_string()));
    let driver = LdapAuthDriver::with_mock(ldap_config(), mock);
    let creds = Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: SecretString::new("pass".to_string()),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Continue));
}

#[tokio::test]
async fn ad_driver_returns_last_reject_when_all_forms_fail() {
    use ox_security_auth::drivers::ad::BindDnCapture;
    use ox_security_auth::drivers::ldap::LdapBindResult;
    let mock = BindDnCapture::new(LdapBindResult::InvalidCredentials);
    let driver = AdAuthDriver::with_mock(ad_config(), mock);
    let creds = Credentials::UsernamePassword {
        username: "bob".to_string(),
        password: SecretString::new("wrong".to_string()),
    };
    let mut ctx = test_ctx();
    let result = driver.authenticate(&creds, &mut ctx).await;
    assert!(matches!(result, AuthResult::Reject(_)));
}
