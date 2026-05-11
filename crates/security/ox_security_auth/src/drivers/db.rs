use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, TenantId,
    Principal, PrincipalId, AuthSource,
    drivers::AuthDriver,
};
use secrecy::ExposeSecret;

pub type CredentialLookupFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
pub type PasswordVerifierFn = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

pub struct DbAuthDriver {
    tenant_id: TenantId,
    lookup: CredentialLookupFn,
    verify: PasswordVerifierFn,
}

impl DbAuthDriver {
    pub fn new(
        tenant_id: TenantId,
        lookup: CredentialLookupFn,
        verify: PasswordVerifierFn,
    ) -> Self {
        Self { tenant_id, lookup, verify }
    }
}

#[async_trait]
impl AuthDriver for DbAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let (username, password) = match credentials {
            Credentials::UsernamePassword { username, password } => {
                (username.as_str(), password.expose_secret())
            }
            _ => return AuthResult::Continue,
        };

        match (self.lookup)(username) {
            None => AuthResult::Continue,
            Some(stored_hash) => {
                if (self.verify)(password, &stored_hash) {
                    AuthResult::Authenticated(Principal {
                        id: PrincipalId::new(),
                        display_name: username.to_string(),
                        source: AuthSource::Local,
                        groups: vec![],
                        tenant_id: self.tenant_id.clone(),
                        session_id: None,
                    })
                } else {
                    AuthResult::Reject(format!("invalid credentials for '{}'", username))
                }
            }
        }
    }
}
