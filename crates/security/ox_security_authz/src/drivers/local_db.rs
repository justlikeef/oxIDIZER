use std::sync::Arc;
use async_trait::async_trait;
use ox_security_core::{
    AuthzResult,
    drivers::AuthzDriver,
    principal::Principal,
    types::{GroupId, PrincipalId},
};
use crate::grant::PermissionGrant;

/// Given a principal id and the principal's group memberships, return all
/// PermissionGrants that apply (direct grants + group grants combined).
/// The injected function is responsible for querying whatever backing store
/// (database, in-memory map, etc.) is appropriate for the deployment.
pub(crate) type GrantLookupFn =
    Arc<dyn Fn(&PrincipalId, &[GroupId]) -> Vec<PermissionGrant> + Send + Sync>;

pub struct LocalDbAuthzDriver {
    lookup: GrantLookupFn,
}

impl LocalDbAuthzDriver {
    pub fn new(lookup: GrantLookupFn) -> Self {
        Self { lookup }
    }
}

/// Returns true if `resource_pattern` matches `resource`.
///
/// Matching rules:
///   - `None`                 → matches any resource
///   - `Some("files/*")`      → matches any resource starting with `"files/"`
///   - `Some("files/a.txt")`  → matches only the exact string `"files/a.txt"`
fn pattern_matches(resource_pattern: &Option<String>, resource: &str) -> bool {
    match resource_pattern {
        None => true,
        Some(pattern) => {
            if let Some(prefix) = pattern.strip_suffix("/*") {
                // wildcard: resource must start with "<prefix>/"
                resource.len() > prefix.len()
                    && resource.starts_with(prefix)
                    && resource.as_bytes()[prefix.len()] == b'/'
            } else {
                // exact match
                resource == pattern.as_str()
            }
        }
    }
}

#[async_trait]
impl AuthzDriver for LocalDbAuthzDriver {
    async fn check(
        &self,
        principal: &Principal,
        path: &str,
        operation: &str,
    ) -> AuthzResult {
        let grants = (self.lookup)(&principal.id, &principal.groups);

        // Evaluate in specificity order: exact match first, then wildcard, then None.
        // Return Allow on the first matching grant for the requested operation.
        // If no grant matches at all, return Continue so the next pipeline driver
        // gets a chance to evaluate.

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

        // No matching grant found — let the next driver in the pipeline decide.
        AuthzResult::Continue
    }
}
