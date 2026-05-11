use async_trait::async_trait;
use crate::accounting::AccountingEvent;
use crate::context::AuthPipelineContext;
use crate::credentials::{Credentials, MfaChallenge};
use crate::principal::Principal;

pub enum AuthResult {
    Authenticated(Principal),
    MfaRequired(MfaChallenge),
    Continue,
    Reject(String),
}

#[derive(Debug)]
pub enum AuthzResult {
    Allow,
    Deny(String),
}

#[async_trait]
pub trait AuthDriver: Send + Sync {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult;
}

#[async_trait]
pub trait AuthzDriver: Send + Sync {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult;
}

#[async_trait]
pub trait AccountingDriver: Send + Sync {
    async fn record(&self, event: &AccountingEvent);
}
