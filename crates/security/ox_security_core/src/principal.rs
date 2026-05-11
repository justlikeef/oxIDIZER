use serde::{Deserialize, Serialize};
use crate::types::{AuthSource, GroupId, PrincipalId, SessionId, TenantId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub id: PrincipalId,
    pub display_name: String,
    pub source: AuthSource,
    pub groups: Vec<GroupId>,
    pub tenant_id: TenantId,
    pub session_id: Option<SessionId>,
}

/// Produced by credential drivers before MFA is complete.
/// Promotes to Principal after all auth steps pass.
#[derive(Debug, Clone)]
pub struct PartialPrincipal {
    pub id: PrincipalId,
    pub display_name: String,
    pub source: AuthSource,
    pub groups: Vec<GroupId>,
    pub tenant_id: TenantId,
}

impl PartialPrincipal {
    pub fn into_principal(self, session_id: Option<SessionId>) -> Principal {
        Principal {
            id: self.id,
            display_name: self.display_name,
            source: self.source,
            groups: self.groups,
            tenant_id: self.tenant_id,
            session_id,
        }
    }
}
