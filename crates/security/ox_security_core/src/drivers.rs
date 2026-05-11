use async_trait::async_trait;
use crate::principal::Principal;

#[derive(Debug)]
pub enum AuthzResult {
    Allow,
    Deny(String),
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
