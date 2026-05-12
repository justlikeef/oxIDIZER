//! End-to-end test: StubFirstFactorDriver → TotpAuthDriver pipeline.
//!
//! Simulates a login that:
//!   1. Presents UsernamePassword → stub driver returns MfaRequired and sets partial_principal.
//!   2. Presents MfaPasscode → TotpAuthDriver validates code → Authenticated.

use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::Arc;
use ox_security_auth::{TotpAuthDriver, AuthPipeline};
use ox_security_core::{
    AuthPipelineContext, AuthResult, Credentials,
    MfaChallenge, TenantId, SessionToken,
    PrincipalId, PartialPrincipal, AuthSource, GroupId,
};
use secrecy::SecretString;
use totp_rs::{Algorithm, TOTP, Secret};

const SECRET_B32: &str = "JBSWY3DPEHPK3PXP2FASXCHVKN7G65Z";

fn current_totp_code() -> String {
    let totp = TOTP::new(
        Algorithm::SHA1, 6, 1, 30,
        Secret::Encoded(SECRET_B32.to_string()).to_bytes().unwrap(),
    ).expect("test secret must be RFC-compliant");
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    totp.generate(t)
}

/// A stub first-factor driver that returns MfaRequired with a PartialPrincipal.
struct StubFirstFactorDriver {
    tenant_id: TenantId,
}

use async_trait::async_trait;
use ox_security_core::drivers::AuthDriver;

#[async_trait]
impl AuthDriver for StubFirstFactorDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        match credentials {
            Credentials::UsernamePassword { username, .. } => {
                // Store partial principal in context for the TOTP driver.
                ctx.partial_principal = Some(PartialPrincipal {
                    id: PrincipalId::new(),
                    display_name: username.clone(),
                    source: AuthSource::Local,
                    groups: vec![GroupId::new("users")],
                    tenant_id: self.tenant_id.clone(),
                });
                AuthResult::MfaRequired(MfaChallenge::CodeRequired {
                    session_token: SessionToken::new(),
                })
            }
            _ => AuthResult::Continue,
        }
    }
}

#[tokio::test]
async fn full_mfa_pipeline_authenticates_with_correct_totp() {
    let tenant = TenantId::from_str("integration-test").unwrap();

    let first_factor: Arc<dyn ox_security_core::drivers::AuthDriver> =
        Arc::new(StubFirstFactorDriver { tenant_id: tenant.clone() });

    let totp_driver: Arc<dyn ox_security_core::drivers::AuthDriver> =
        Arc::new(TotpAuthDriver::new(Arc::new(|_id| Some(SECRET_B32.to_string()))));

    let pipeline = AuthPipeline::new(vec![first_factor, totp_driver]);

    // Step 1: password
    let mut ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: tenant.clone(),
        source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    };

    let step1 = pipeline.authenticate(
        &Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("pass".to_string()),
        },
        &mut ctx,
    ).await;

    assert!(
        matches!(step1, AuthResult::MfaRequired(_)),
        "step 1 must return MfaRequired"
    );

    // Step 2: TOTP code (partial_principal is still in ctx from step 1)
    let step2 = pipeline.authenticate(
        &Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: current_totp_code(),
        },
        &mut ctx,
    ).await;

    assert!(
        matches!(step2, AuthResult::Authenticated(_)),
        "step 2 must return Authenticated after valid TOTP code"
    );
}

#[tokio::test]
async fn full_mfa_pipeline_rejects_with_wrong_totp() {
    let tenant = TenantId::from_str("integration-test").unwrap();

    let first_factor: Arc<dyn ox_security_core::drivers::AuthDriver> =
        Arc::new(StubFirstFactorDriver { tenant_id: tenant.clone() });

    let totp_driver: Arc<dyn ox_security_core::drivers::AuthDriver> =
        Arc::new(TotpAuthDriver::new(Arc::new(|_id| Some(SECRET_B32.to_string()))));

    let pipeline = AuthPipeline::new(vec![first_factor, totp_driver]);

    let mut ctx = AuthPipelineContext {
        partial_principal: None,
        tenant_id: tenant.clone(),
        source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    };

    // Step 1
    let _ = pipeline.authenticate(
        &Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("pass".to_string()),
        },
        &mut ctx,
    ).await;

    // Step 2: wrong code
    let step2 = pipeline.authenticate(
        &Credentials::MfaPasscode {
            session_token: SessionToken::new(),
            code: "000000".to_string(),
        },
        &mut ctx,
    ).await;

    assert!(
        matches!(step2, AuthResult::Reject(_)),
        "wrong TOTP code must produce Reject"
    );
}
