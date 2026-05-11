use async_trait::async_trait;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, drivers::AuthDriver};

pub struct KerberosAuthDriver;

#[async_trait]
impl AuthDriver for KerberosAuthDriver {
    async fn authenticate(&self, _credentials: &Credentials, _ctx: &mut AuthPipelineContext) -> AuthResult {
        AuthResult::Continue
    }
}
