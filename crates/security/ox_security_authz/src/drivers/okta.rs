use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthzResult,
    drivers::AuthzDriver,
    principal::Principal,
    types::{AuthSource, TenantId},
};
use crate::grant::PermissionGrant;

#[derive(Clone)]
pub struct OktaConfig {
    pub domain: String,
    pub api_token: String,
    pub role_claim_attr: String,
    pub tenant_id: TenantId,
}

pub type OktaApiFn = Arc<
    dyn Fn(&str) -> Pin<Box<dyn Future<Output = Result<Vec<String>, String>> + Send>>
        + Send
        + Sync,
>;

pub type OktaGrantMapperFn =
    Arc<dyn Fn(&[String]) -> Vec<PermissionGrant> + Send + Sync>;

pub struct OktaAuthzDriver {
    config: OktaConfig,
    api: OktaApiFn,
    mapper: OktaGrantMapperFn,
}

impl OktaAuthzDriver {
    pub fn new(config: OktaConfig, api: OktaApiFn, mapper: OktaGrantMapperFn) -> Self {
        Self { config, api, mapper }
    }
}

#[async_trait]
impl AuthzDriver for OktaAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        if principal.source != AuthSource::Okta {
            return AuthzResult::Continue;
        }

        let user_id = principal.id.as_uuid().to_string();
        let group_names = match (self.api)(&user_id).await {
            Ok(names) => names,
            Err(_) => return AuthzResult::Continue,
        };

        let grants = (self.mapper)(&group_names);

        // Pass 1: exact resource match
        for grant in &grants {
            if grant.operation != operation { continue; }
            if let Some(ref pat) = grant.resource_pattern {
                if !pat.ends_with("/*") && pat.as_str() == path {
                    return AuthzResult::Allow;
                }
            }
        }

        // Pass 2: wildcard resource match
        for grant in &grants {
            if grant.operation != operation { continue; }
            if let Some(ref pat) = grant.resource_pattern {
                if let Some(prefix) = pat.strip_suffix("/*") {
                    if path.len() > prefix.len()
                        && path.starts_with(prefix)
                        && path.as_bytes()[prefix.len()] == b'/'
                    {
                        return AuthzResult::Allow;
                    }
                }
            }
        }

        // Pass 3: operation-only grant (resource_pattern = None → all resources)
        for grant in &grants {
            if grant.operation == operation && grant.resource_pattern.is_none() {
                return AuthzResult::Allow;
            }
        }

        AuthzResult::Continue
    }
}
