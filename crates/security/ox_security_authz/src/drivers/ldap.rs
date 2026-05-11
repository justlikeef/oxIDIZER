use async_trait::async_trait;
use ox_security_core::{AuthzResult, drivers::AuthzDriver, principal::Principal};

pub struct LdapAuthzDriver;

#[async_trait]
impl AuthzDriver for LdapAuthzDriver {
    async fn check(
        &self,
        _principal: &Principal,
        _path: &str,
        _operation: &str,
    ) -> AuthzResult {
        AuthzResult::Continue
    }
}
