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
