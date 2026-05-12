use std::sync::Arc;
use async_trait::async_trait;
use secrecy::ExposeSecret;
use ox_security_core::{AuthResult, AuthPipelineContext, Credentials, Principal, drivers::AuthDriver};

pub type ApiKeyLookupFn = Arc<dyn Fn(&str) -> Option<Principal> + Send + Sync>;

pub struct ApiKeyAuthDriver {
    lookup: ApiKeyLookupFn,
}

impl ApiKeyAuthDriver {
    pub fn new(lookup: ApiKeyLookupFn) -> Self {
        Self { lookup }
    }
}

#[async_trait]
impl AuthDriver for ApiKeyAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let key = match credentials {
            Credentials::ApiKey { key } => key.expose_secret(),
            _ => return AuthResult::Continue,
        };
        match (self.lookup)(key) {
            Some(principal) => AuthResult::Authenticated(principal),
            // Unknown key => Continue, not Reject; another driver may recognise it
            None => AuthResult::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use ox_security_core::{
        AuthResult, AuthPipelineContext, Credentials, Principal, PrincipalId,
        AuthSource, TenantId,
    };
    use secrecy::SecretString;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: TenantId::from_str("test").unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    fn make_driver() -> ApiKeyAuthDriver {
        ApiKeyAuthDriver::new(Arc::new(|key: &str| {
            if key == "valid-api-key" {
                Some(Principal {
                    id: PrincipalId::new(),
                    display_name: "service-account".to_string(),
                    source: AuthSource::ApiKey,
                    groups: vec![],
                    tenant_id: TenantId::from_str("test").unwrap(),
                    session_id: None,
                })
            } else {
                None
            }
        }))
    }

    #[tokio::test]
    async fn api_key_continues_for_non_api_key_creds() {
        let driver = make_driver();
        let creds = Credentials::UsernamePassword {
            username: "user".to_string(),
            password: SecretString::new("pass".to_string()),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn api_key_authenticates_known_key() {
        let driver = make_driver();
        let creds = Credentials::ApiKey {
            key: SecretString::new("valid-api-key".to_string()),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => assert_eq!(p.display_name, "service-account"),
            _other => panic!("expected Authenticated, got something else"),
        }
    }

    #[tokio::test]
    async fn api_key_continues_for_unknown_key() {
        let driver = make_driver();
        let creds = Credentials::ApiKey {
            key: SecretString::new("unknown-key".to_string()),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }
}
