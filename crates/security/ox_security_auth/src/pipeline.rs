use std::sync::Arc;
use ox_security_core::drivers::AuthDriver;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials};

pub struct AuthPipeline {
    drivers: Vec<Arc<dyn AuthDriver>>,
}

impl AuthPipeline {
    pub fn new(drivers: Vec<Arc<dyn AuthDriver>>) -> Self {
        Self { drivers }
    }

    pub async fn authenticate(
        &self,
        credentials: &Credentials,
        ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        for driver in &self.drivers {
            match driver.authenticate(credentials, ctx).await {
                AuthResult::Continue => continue,
                result => return result,
            }
        }
        AuthResult::Reject("no driver handled the credentials".to_string())
    }
}
