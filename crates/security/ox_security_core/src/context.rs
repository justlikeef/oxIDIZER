use std::net::IpAddr;
use std::sync::Arc;
use crate::drivers::AuthzDriver;
use crate::error::AuthzError;
use crate::principal::{PartialPrincipal, Principal};
use crate::types::TenantId;

pub struct SecurityContext {
    pub principal: Option<Principal>,
    pub call_context: String,
    pub tenant_id: TenantId,
    pub source_ip: IpAddr,
    pub authz: Option<Arc<dyn AuthzDriver>>,
}

impl SecurityContext {
    pub fn new(tenant_id: TenantId, source_ip: IpAddr) -> Self {
        Self {
            principal: None,
            call_context: String::new(),
            tenant_id,
            source_ip,
            authz: None,
        }
    }

    pub fn with_authz(mut self, driver: Arc<dyn AuthzDriver>) -> Self {
        self.authz = Some(driver);
        self
    }

    /// Called by objects using only their own fragment.
    /// Resolves: call_context + "." + object_fragment -> full path -> evaluates grants.
    pub async fn check(&self, object_fragment: &str, operation: &str) -> Result<(), AuthzError> {
        let principal = self.principal.as_ref().ok_or(AuthzError::Unauthenticated)?;
        let driver = self.authz.as_ref().ok_or_else(|| {
            AuthzError::Internal("no authz driver configured on SecurityContext".to_string())
        })?;
        let path = if self.call_context.is_empty() {
            object_fragment.to_string()
        } else {
            format!("{}.{}", self.call_context, object_fragment)
        };
        match driver.check(principal, &path, operation).await {
            crate::drivers::AuthzResult::Allow => Ok(()),
            crate::drivers::AuthzResult::Deny(_reason) => Err(AuthzError::Denied {
                path,
                operation: operation.to_string(),
            }),
        }
    }
}

pub struct AuthPipelineContext {
    pub partial_principal: Option<PartialPrincipal>,
    pub tenant_id: TenantId,
    pub source_ip: IpAddr,
}
