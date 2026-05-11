use async_trait::async_trait;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, drivers::AuthDriver};

pub struct ApiKeyAuthDriver;

#[async_trait]
impl AuthDriver for ApiKeyAuthDriver {
    async fn authenticate(&self, _credentials: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Continue
    }
}
