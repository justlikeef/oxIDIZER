use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthzResult,
    drivers::AuthzDriver,
    principal::Principal,
    types::{GroupId, PrincipalId},
};
use crate::grant::PermissionGrant;

/// Given a principal id and the principal's (possibly expanded) group memberships,
/// return all PermissionGrants that apply. Identical contract to LocalDbAuthzDriver.
pub(crate) type GrantLookupFn =
    Arc<dyn Fn(&PrincipalId, &[GroupId]) -> Vec<PermissionGrant> + Send + Sync>;

/// Given the principal's direct group memberships, return the transitive closure
/// (direct groups plus all ancestor groups reachable through nesting).
/// For a flat directory, return a clone of the input unchanged.
pub type GroupResolverFn = Arc<dyn Fn(&[GroupId]) -> Vec<GroupId> + Send + Sync>;

pub struct LdapAuthzDriver {
    lookup: GrantLookupFn,
    resolve_groups: GroupResolverFn,
}

impl LdapAuthzDriver {
    /// Full constructor: supply both a grant lookup function and a group resolver.
    /// The group resolver expands nested group membership before the lookup is called.
    pub fn new(lookup: GrantLookupFn, resolve_groups: GroupResolverFn) -> Self {
        Self { lookup, resolve_groups }
    }

    /// Convenience constructor for deployments where LDAP groups are flat (no nesting).
    /// The resolver is the identity function — groups are passed through unchanged.
    pub fn without_group_resolution(lookup: GrantLookupFn) -> Self {
        Self {
            lookup,
            resolve_groups: Arc::new(|groups: &[GroupId]| groups.to_vec()),
        }
    }
}

#[async_trait]
impl AuthzDriver for LdapAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        // Expand group memberships to transitive closure before lookup.
        let expanded_groups = (self.resolve_groups)(&principal.groups);
        let grants = (self.lookup)(&principal.id, &expanded_groups);

        // Three-pass evaluation: exact match → wildcard → None.
        // Mirrors LocalDbAuthzDriver exactly.

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

        // No matching grant — let the next driver in the pipeline decide.
        AuthzResult::Continue
    }
}
