use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthResult, AuthPipelineContext, Credentials, TenantId,
    Principal, PrincipalId, AuthSource,
    drivers::AuthDriver,
};

/// Pluggable ticket validator.
///
/// Given the raw Kerberos ticket bytes, returns the client principal name
/// (e.g. `"alice@EXAMPLE.COM"`) on success, or an error message on failure.
///
/// In production, wire this to a `cross_krb5::ServerCtx` that accepts the token
/// against the configured service principal and keytab:
///
/// ```rust,ignore
/// use cross_krb5::{ServerCtx, Step};
/// let validator: TicketValidatorFn = Arc::new(move |ticket: &[u8]| {
///     let mut ctx = ServerCtx::new(Some(&service_principal), Some(&keytab_path))
///         .map_err(|e| e.to_string())?;
///     match ctx.step(ticket).map_err(|e| e.to_string())? {
///         Step::Finished(_tok) => ctx
///             .client()
///             .map(|c| c.to_string())
///             .map_err(|e| e.to_string()),
///         Step::Continue(_) => Err("unexpected continuation token from server ctx".to_string()),
///     }
/// });
/// ```
pub type TicketValidatorFn = Arc<dyn Fn(&[u8]) -> Result<String, String> + Send + Sync>;

/// Configuration for `KerberosAuthDriver`.
pub struct KerberosConfig {
    /// Fully-qualified service principal, e.g. `"HTTP/myservice.example.com@EXAMPLE.COM"`.
    pub service_principal: String,
    /// Path to the keytab file for service credential, e.g. `"/etc/krb5.keytab"`.
    pub keytab_path: String,
    /// Kerberos realm, e.g. `"EXAMPLE.COM"`.
    pub realm: String,
    /// Tenant this driver is scoped to.
    pub tenant_id: TenantId,
}

/// Authentication driver that validates Kerberos service tickets.
///
/// Accepts only `Credentials::KerberosTicket` — passes `Continue` for all other variants.
/// On validation success, returns `AuthResult::Authenticated` with `source: AuthSource::Kerberos`.
/// On validation failure, returns `AuthResult::Reject` with the validator's error message.
pub struct KerberosAuthDriver {
    config: KerberosConfig,
    validator: TicketValidatorFn,
}

impl KerberosAuthDriver {
    /// Construct a driver with an explicit ticket validator.
    ///
    /// For production use, pass a validator wrapping `cross_krb5::ServerCtx`.
    /// For tests, pass a closure that mocks success or failure.
    pub fn new(config: KerberosConfig, validator: TicketValidatorFn) -> Self {
        Self { config, validator }
    }
}

#[async_trait]
impl AuthDriver for KerberosAuthDriver {
    async fn authenticate(
        &self,
        credentials: &Credentials,
        _ctx: &mut AuthPipelineContext,
    ) -> AuthResult {
        let ticket = match credentials {
            Credentials::KerberosTicket { ticket } => ticket.as_slice(),
            _ => return AuthResult::Continue,
        };

        match (self.validator)(ticket) {
            Ok(client_principal) => AuthResult::Authenticated(Principal {
                id: PrincipalId::new(),
                display_name: client_principal,
                source: AuthSource::Kerberos,
                groups: vec![],
                tenant_id: self.config.tenant_id.clone(),
                session_id: None,
            }),
            Err(reason) => AuthResult::Reject(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use ox_security_core::AuthPipelineContext;

    fn test_config() -> KerberosConfig {
        KerberosConfig {
            service_principal: "HTTP/myservice.example.com@EXAMPLE.COM".to_string(),
            keytab_path: "/etc/krb5.keytab".to_string(),
            realm: "EXAMPLE.COM".to_string(),
            tenant_id: "test".parse().unwrap(),
        }
    }

    fn test_ctx() -> AuthPipelineContext {
        AuthPipelineContext {
            partial_principal: None,
            tenant_id: "test".parse().unwrap(),
            source_ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        }
    }

    #[tokio::test]
    async fn kerberos_continues_for_non_ticket_creds() {
        let driver = KerberosAuthDriver::new(
            test_config(),
            Arc::new(|_ticket: &[u8]| Ok("principal@EXAMPLE.COM".to_string())),
        );
        let creds = Credentials::UsernamePassword {
            username: "alice".to_string(),
            password: "pass".to_string().into(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        assert!(matches!(result, AuthResult::Continue));
    }

    #[tokio::test]
    async fn kerberos_authenticates_valid_ticket() {
        let driver = KerberosAuthDriver::new(
            test_config(),
            Arc::new(|_ticket: &[u8]| Ok("alice@EXAMPLE.COM".to_string())),
        );
        let creds = Credentials::KerberosTicket {
            ticket: b"fake-ticket-bytes".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Authenticated(p) => {
                assert_eq!(p.display_name, "alice@EXAMPLE.COM");
                assert_eq!(p.source, AuthSource::Kerberos);
                assert!(p.groups.is_empty());
            }
            _ => panic!("expected Authenticated, got something else"),
        }
    }

    #[tokio::test]
    async fn kerberos_rejects_invalid_ticket() {
        let driver = KerberosAuthDriver::new(
            test_config(),
            Arc::new(|_ticket: &[u8]| {
                Err("ticket validation failed: clock skew too great".to_string())
            }),
        );
        let creds = Credentials::KerberosTicket {
            ticket: b"invalid-ticket".to_vec(),
        };
        let mut ctx = test_ctx();
        let result = driver.authenticate(&creds, &mut ctx).await;
        match result {
            AuthResult::Reject(msg) => {
                assert!(
                    msg.contains("ticket validation failed"),
                    "unexpected message: {}",
                    msg
                );
            }
            _ => panic!("expected Reject, got something else"),
        }
    }
}
