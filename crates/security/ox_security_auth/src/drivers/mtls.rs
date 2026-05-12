use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, Principal, drivers::AuthDriver,
};

pub type CertValidatorFn = Arc<dyn Fn(&[u8]) -> Result<Principal, String> + Send + Sync>;

pub struct MtlsAuthDriver {
    validator: CertValidatorFn,
}

impl MtlsAuthDriver {
    pub fn new(validator: CertValidatorFn) -> Self {
        Self { validator }
    }
}

#[async_trait]
impl AuthDriver for MtlsAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let der = match credentials {
            Credentials::ClientCert { der } => der.as_slice(),
            _ => return AuthResult::Continue,
        };
        match (self.validator)(der) {
            Ok(principal) => AuthResult::Authenticated(principal),
            Err(reason) => AuthResult::Reject(reason),
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

    fn make_driver() -> MtlsAuthDriver {
        MtlsAuthDriver::new(Arc::new(|der: &[u8]| {
            if der == b"valid-cert-der" {
                Ok(Principal {
                    id: PrincipalId::new(),
                    display_name: "CN=device-001".to_string(),
                    source: AuthSource::Mtls,
                    groups: vec![],
                    tenant_id: TenantId::from_str("test").unwrap(),
                    session_id: None,
                })
            } else {
                Err("certificate validation failed: unknown issuer".to_string())
            }
        }))
    }

    #[tokio::test]
    async fn mtls_continues_for_non_cert_creds() {
        let driver = make_driver();
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: SecretString::new("secret".to_string()),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn mtls_authenticates_valid_cert() {
        let driver = make_driver();
        let creds = Credentials::ClientCert {
            der: b"valid-cert-der".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => assert_eq!(p.display_name, "CN=device-001"),
            _other => panic!("expected Authenticated, got something else"),
        }
    }

    #[tokio::test]
    async fn mtls_rejects_invalid_cert() {
        let driver = make_driver();
        let creds = Credentials::ClientCert {
            der: b"garbage-bytes".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Reject(msg) => assert!(msg.contains("certificate validation failed")),
            _other => panic!("expected Reject, got something else"),
        }
    }
}
