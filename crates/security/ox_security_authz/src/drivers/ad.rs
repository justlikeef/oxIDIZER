use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthzResult,
    drivers::AuthzDriver,
    principal::Principal,
    types::GroupId,
};
use crate::grant::PermissionGrant;
use super::ldap::{GrantLookupFn, GroupResolverFn};

/// Authorization driver backed by an Active Directory persistence layer.
///
/// Structurally identical to `LdapAuthzDriver`. The AD-specific behaviour
/// (tokenGroups traversal, SID-based resolution, etc.) is the caller's
/// responsibility via the injected `GroupResolverFn`.
pub struct AdAuthzDriver {
    lookup: GrantLookupFn,
    resolve_groups: GroupResolverFn,
}

impl AdAuthzDriver {
    /// Full constructor: supply a grant lookup function and a group resolver.
    pub fn new(lookup: GrantLookupFn, resolve_groups: GroupResolverFn) -> Self {
        Self { lookup, resolve_groups }
    }

    /// Convenience constructor for deployments where AD groups are flat.
    /// The resolver is the identity function.
    pub fn without_group_resolution(lookup: GrantLookupFn) -> Self {
        Self {
            lookup,
            resolve_groups: Arc::new(|groups: &[GroupId]| groups.to_vec()),
        }
    }
}

#[async_trait]
impl AuthzDriver for AdAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        let expanded_groups = (self.resolve_groups)(&principal.groups);
        let grants = (self.lookup)(&principal.id, &expanded_groups);

        // Pass 1: exact resource match
        for grant in &grants {
            if grant.operation != operation {
                continue;
            }
            if let Some(ref pat) = grant.resource_pattern {
                if !pat.ends_with("/*") && pat.as_str() == path {
                    return AuthzResult::Allow;
                }
            }
        }

        // Pass 2: wildcard resource match
        for grant in &grants {
            if grant.operation != operation {
                continue;
            }
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
