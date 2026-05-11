use std::net::{IpAddr, Ipv4Addr};
use chrono::Utc;
use ox_security_auth::AuthPipeline;
use ox_security_authz::AuthzPipeline;
use ox_security_accounting::AccountingPipeline;
use ox_security_core::{
    AccountingEvent, AuthOutcome, AuthPipelineContext, AuthResult, AuthzOutcome, AuthzResult,
    Credentials, Principal,
};
use crate::error::SecurityError;

pub struct SecurityPipeline {
    pub(crate) auth: AuthPipeline,
    pub(crate) authz: AuthzPipeline,
    pub(crate) accounting: AccountingPipeline,
}

impl SecurityPipeline {
    /// Authenticate a set of credentials.
    ///
    /// On success records an `AuthSuccess` accounting event and returns `Ok(Principal)`.
    /// On failure records an `AuthFailure` event and returns `Err(SecurityError::AuthFailed)`.
    /// On `MfaRequired` returns `Err(SecurityError::MfaRequired)` — no accounting event is
    /// recorded because the authentication attempt is incomplete.
    pub async fn authenticate(
        &self,
        credentials: &Credentials,
        auth_ctx: &mut AuthPipelineContext,
    ) -> Result<Principal, SecurityError> {
        let result = self.auth.authenticate(credentials, auth_ctx).await;
        match result {
            AuthResult::Authenticated(principal) => {
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: Some(principal.id.clone()),
                        auth_outcome: AuthOutcome::Authenticated,
                        authz_outcome: None,
                        call_context: String::new(),
                        object_fragment: None,
                        operation_name: None,
                        timestamp: Utc::now(),
                        source_ip: auth_ctx.source_ip,
                        session_id: principal.session_id.clone(),
                        tenant_id: auth_ctx.tenant_id.clone(),
                    })
                    .await;
                Ok(principal)
            }
            AuthResult::Reject(reason) => {
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: None,
                        auth_outcome: AuthOutcome::Failed(reason.clone()),
                        authz_outcome: None,
                        call_context: String::new(),
                        object_fragment: None,
                        operation_name: None,
                        timestamp: Utc::now(),
                        source_ip: auth_ctx.source_ip,
                        session_id: None,
                        tenant_id: auth_ctx.tenant_id.clone(),
                    })
                    .await;
                Err(SecurityError::AuthFailed(reason))
            }
            AuthResult::MfaRequired(challenge) => {
                let description = match &challenge {
                    ox_security_core::MfaChallenge::PushSent { .. } => "push sent".to_string(),
                    ox_security_core::MfaChallenge::CodeRequired { .. } => {
                        "code required".to_string()
                    }
                };
                Err(SecurityError::MfaRequired(description))
            }
            AuthResult::Continue => {
                let reason = "no auth driver handled credentials".to_string();
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: None,
                        auth_outcome: AuthOutcome::Failed(reason.clone()),
                        authz_outcome: None,
                        call_context: String::new(),
                        object_fragment: None,
                        operation_name: None,
                        timestamp: Utc::now(),
                        source_ip: auth_ctx.source_ip,
                        session_id: None,
                        tenant_id: auth_ctx.tenant_id.clone(),
                    })
                    .await;
                Err(SecurityError::AuthFailed(reason))
            }
        }
    }

    /// Authorize a principal for an operation on a path.
    ///
    /// `path` is the fully-resolved permission path (call_context + "." + object_fragment).
    /// Returns `Ok(())` on allow, `Err(SecurityError::AuthzDenied)` on deny or empty pipeline.
    pub async fn authorize(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> Result<(), SecurityError> {
        // NOTE: AuthzPipeline method is named `check`, not `authorize`
        let result = self.authz.check(principal, path, operation).await;
        match result {
            AuthzResult::Allow => {
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: Some(principal.id.clone()),
                        auth_outcome: AuthOutcome::Authenticated,
                        authz_outcome: Some(AuthzOutcome::Allowed),
                        call_context: path.to_string(),
                        object_fragment: None,
                        operation_name: Some(operation.to_string()),
                        timestamp: Utc::now(),
                        source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                        session_id: principal.session_id.clone(),
                        tenant_id: principal.tenant_id.clone(),
                    })
                    .await;
                Ok(())
            }
            AuthzResult::Deny(reason) => {
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: Some(principal.id.clone()),
                        auth_outcome: AuthOutcome::Authenticated,
                        authz_outcome: Some(AuthzOutcome::Denied {
                            path: path.to_string(),
                            operation_name: operation.to_string(),
                        }),
                        call_context: path.to_string(),
                        object_fragment: None,
                        operation_name: Some(operation.to_string()),
                        timestamp: Utc::now(),
                        source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                        session_id: principal.session_id.clone(),
                        tenant_id: principal.tenant_id.clone(),
                    })
                    .await;
                Err(SecurityError::AuthzDenied(reason))
            }
            AuthzResult::Continue => {
                // Continue with no further drivers means fail-closed
                let reason = "no authz driver granted access".to_string();
                self.accounting
                    .record(&AccountingEvent {
                        principal_id: Some(principal.id.clone()),
                        auth_outcome: AuthOutcome::Authenticated,
                        authz_outcome: Some(AuthzOutcome::Denied {
                            path: path.to_string(),
                            operation_name: operation.to_string(),
                        }),
                        call_context: path.to_string(),
                        object_fragment: None,
                        operation_name: Some(operation.to_string()),
                        timestamp: Utc::now(),
                        source_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                        session_id: principal.session_id.clone(),
                        tenant_id: principal.tenant_id.clone(),
                    })
                    .await;
                Err(SecurityError::AuthzDenied(reason))
            }
        }
    }
}
