use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use async_trait::async_trait;
use ox_security_core::{
    AuthPipelineContext, AuthResult, AuthSource, AuthzResult, Credentials,
    AccountingEvent, AccountingDriver,
    Principal, PrincipalId, TenantId,
};
use ox_security_pipeline::{SecurityError, SecurityPipelineBuilder};

// ---------------------------------------------------------------------------
// Inline stub drivers — used across all tasks in this file
// ---------------------------------------------------------------------------

struct AlwaysContinueAuthDriver;

#[async_trait]
impl ox_security_core::AuthDriver for AlwaysContinueAuthDriver {
    async fn authenticate(
        &self,
        _creds: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        AuthResult::Continue
    }
}

struct AlwaysRejectAuthDriver;

#[async_trait]
impl ox_security_core::AuthDriver for AlwaysRejectAuthDriver {
    async fn authenticate(
        &self,
        _creds: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        AuthResult::Reject("rejected by policy".to_string())
    }
}

struct AcceptsAliceDriver;

#[async_trait]
impl ox_security_core::AuthDriver for AcceptsAliceDriver {
    async fn authenticate(
        &self,
        creds: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        use secrecy::ExposeSecret;
        match creds {
            Credentials::UsernamePassword { username, password }
                if username == "alice" && password.expose_secret() == "pass" =>
            {
                AuthResult::Authenticated(Principal {
                    id: PrincipalId::new(),
                    display_name: "alice".to_string(),
                    source: AuthSource::Local,
                    groups: vec![],
                    tenant_id: "test".parse().unwrap(),
                    session_id: None,
                })
            }
            _ => AuthResult::Reject("bad credentials".to_string()),
        }
    }
}

struct AlwaysContinueAuthzDriver;

#[async_trait]
impl ox_security_core::AuthzDriver for AlwaysContinueAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Allow
    }
}

struct AlwaysDenyAuthzDriver;

#[async_trait]
impl ox_security_core::AuthzDriver for AlwaysDenyAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Deny("deny by policy".to_string())
    }
}

struct NoOpAccountingDriver;

#[async_trait]
impl AccountingDriver for NoOpAccountingDriver {
    async fn record(&self, _event: &AccountingEvent) {}
}

fn test_tenant() -> TenantId {
    "test".parse().unwrap()
}

fn test_source_ip() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
}

fn test_principal() -> Principal {
    Principal {
        id: PrincipalId::new(),
        display_name: "alice".to_string(),
        source: AuthSource::Local,
        groups: vec![],
        tenant_id: test_tenant(),
        session_id: None,
    }
}

fn alice_creds() -> Credentials {
    Credentials::UsernamePassword {
        username: "alice".to_string(),
        password: "pass".to_string().into(),
    }
}

// ---------------------------------------------------------------------------
// Task 1 tests: builder + skeleton authenticate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn builder_creates_pipeline() {
    let _pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AlwaysContinueAuthDriver))
        .authz(Arc::new(AlwaysContinueAuthzDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();
    // Compiles and runs — structural test
}

#[tokio::test]
async fn authenticate_success() {
    let pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AcceptsAliceDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let result = pipeline.authenticate(&creds, &mut auth_ctx).await;
    match result {
        Ok(p) => assert_eq!(p.display_name, "alice"),
        Err(e) => panic!("expected Ok(Principal), got {:?}", e),
    }
}

#[tokio::test]
async fn authenticate_reject() {
    let pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AlwaysRejectAuthDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let result = pipeline.authenticate(&creds, &mut auth_ctx).await;
    assert!(
        matches!(result, Err(SecurityError::AuthFailed(_))),
        "expected AuthFailed, got {:?}",
        result
    );
}

#[tokio::test]
async fn authenticate_empty_pipeline_fails() {
    let pipeline = SecurityPipelineBuilder::new()
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let result = pipeline.authenticate(&creds, &mut auth_ctx).await;
    assert!(
        matches!(result, Err(SecurityError::AuthFailed(_))),
        "expected AuthFailed from empty pipeline, got {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// Task 2 tests: authorize
// ---------------------------------------------------------------------------

#[tokio::test]
async fn authorize_allow() {
    let pipeline = SecurityPipelineBuilder::new()
        .authz(Arc::new(AlwaysContinueAuthzDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let principal = test_principal();
    let result = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "read")
        .await;
    assert!(result.is_ok(), "expected Ok(()), got {:?}", result);
}

#[tokio::test]
async fn authorize_deny() {
    let pipeline = SecurityPipelineBuilder::new()
        .authz(Arc::new(AlwaysDenyAuthzDriver))
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let principal = test_principal();
    let result = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "write")
        .await;
    assert!(
        matches!(result, Err(SecurityError::AuthzDenied(_))),
        "expected AuthzDenied, got {:?}",
        result
    );
}

#[tokio::test]
async fn authorize_no_drivers_fails() {
    // Empty authz pipeline is fail-closed: AuthzPipeline returns Deny when all drivers Continue.
    let pipeline = SecurityPipelineBuilder::new()
        .accounting(Arc::new(NoOpAccountingDriver))
        .build();

    let principal = test_principal();
    let result = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "read")
        .await;
    assert!(
        matches!(result, Err(SecurityError::AuthzDenied(_))),
        "expected AuthzDenied from empty pipeline, got {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// Task 3 tests: accounting events + PipelineContextRegistrar
//
// NOTE: MemoryAccountingDriver::events() returns Vec<String> (JSON strings),
// not Vec<AccountingEvent>. Tests use JSON string inspection instead of
// struct field access.
// ---------------------------------------------------------------------------

use ox_security_accounting::MemoryAccountingDriver;
use ox_security_core::registration::{ContextDefinition, ContextRegistrar};
use ox_security_pipeline::PipelineContextRegistrar;
use ox_security_core::operations::OP_READ;

#[tokio::test]
async fn auth_success_records_event() {
    let accounting_driver = Arc::new(MemoryAccountingDriver::new());
    let pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AcceptsAliceDriver))
        .accounting(Arc::clone(&accounting_driver) as Arc<dyn AccountingDriver>)
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let _ = pipeline.authenticate(&creds, &mut auth_ctx).await;

    let events = accounting_driver.events();
    assert_eq!(events.len(), 1, "expected exactly 1 accounting event");
    assert!(
        events[0].contains("Authenticated"),
        "expected Authenticated in event JSON, got: {}",
        events[0]
    );
}

#[tokio::test]
async fn auth_failure_records_event() {
    let accounting_driver = Arc::new(MemoryAccountingDriver::new());
    let pipeline = SecurityPipelineBuilder::new()
        .auth(Arc::new(AlwaysRejectAuthDriver))
        .accounting(Arc::clone(&accounting_driver) as Arc<dyn AccountingDriver>)
        .build();

    let creds = alice_creds();
    let mut auth_ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: test_tenant(),
        source_ip: test_source_ip(),
    };
    let _ = pipeline.authenticate(&creds, &mut auth_ctx).await;

    let events = accounting_driver.events();
    assert_eq!(events.len(), 1, "expected exactly 1 accounting event");
    assert!(
        events[0].contains("Failed"),
        "expected Failed in event JSON, got: {}",
        events[0]
    );
}

#[tokio::test]
async fn authz_allow_records_event() {
    let accounting_driver = Arc::new(MemoryAccountingDriver::new());
    let pipeline = SecurityPipelineBuilder::new()
        .authz(Arc::new(AlwaysContinueAuthzDriver))
        .accounting(Arc::clone(&accounting_driver) as Arc<dyn AccountingDriver>)
        .build();

    let principal = test_principal();
    let _ = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "read")
        .await;

    let events = accounting_driver.events();
    assert_eq!(events.len(), 1, "expected exactly 1 accounting event");
    assert!(
        events[0].contains("Allowed"),
        "expected Allowed in event JSON, got: {}",
        events[0]
    );
}

#[tokio::test]
async fn authz_deny_records_event() {
    let accounting_driver = Arc::new(MemoryAccountingDriver::new());
    let pipeline = SecurityPipelineBuilder::new()
        .authz(Arc::new(AlwaysDenyAuthzDriver))
        .accounting(Arc::clone(&accounting_driver) as Arc<dyn AccountingDriver>)
        .build();

    let principal = test_principal();
    let _ = pipeline
        .authorize(&principal, "com.justlikeef.app.obj", "write")
        .await;

    let events = accounting_driver.events();
    assert_eq!(events.len(), 1, "expected exactly 1 accounting event");
    assert!(
        events[0].contains("Denied"),
        "expected Denied in event JSON, got: {}",
        events[0]
    );
    assert!(
        events[0].contains("com.justlikeef.app.obj"),
        "expected path in event JSON, got: {}",
        events[0]
    );
    assert!(
        events[0].contains("write"),
        "expected operation in event JSON, got: {}",
        events[0]
    );
}

static TEST_CONTEXT_DEF: ContextDefinition = ContextDefinition {
    root: "com.justlikeef.test",
    operations: &[OP_READ],
    children: &[],
};

static REGISTERED_DEF: ContextDefinition = ContextDefinition {
    root: "com.justlikeef.test.objects",
    operations: &[OP_READ],
    children: &[],
};

#[test]
fn registrar_stores_registration() {
    let pipeline = SecurityPipelineBuilder::new().build();
    let registrar = PipelineContextRegistrar::new(pipeline, TEST_CONTEXT_DEF);

    registrar.register_context(REGISTERED_DEF);

    let stored = registrar.stored_registrations();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].root, "com.justlikeef.test.objects");

    assert_eq!(registrar.context_definition().root, "com.justlikeef.test");
}
