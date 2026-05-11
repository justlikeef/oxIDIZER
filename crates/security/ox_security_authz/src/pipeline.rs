use std::sync::Arc;
use ox_security_core::{AuthzResult, drivers::AuthzDriver, principal::Principal};

pub struct AuthzPipeline {
    drivers: Vec<Arc<dyn AuthzDriver>>,
}

impl AuthzPipeline {
    pub fn new(drivers: Vec<Arc<dyn AuthzDriver>>) -> Self {
        Self { drivers }
    }

    pub async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        for driver in &self.drivers {
            match driver.check(principal, path, operation).await {
                AuthzResult::Continue => continue,
                result => return result,
            }
        }
        AuthzResult::Deny("no authz driver granted access".to_string())
    }
}
