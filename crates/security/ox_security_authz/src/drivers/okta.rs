use async_trait::async_trait;
use ox_security_core::{AuthzResult, drivers::AuthzDriver, principal::Principal};

pub struct OktaAuthzDriver;

#[async_trait]
impl AuthzDriver for OktaAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Continue
    }
}
