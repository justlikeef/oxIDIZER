use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use crate::types::{PrincipalId, SessionId, TenantId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthOutcome {
    Authenticated,
    Failed(String),
    MfaRequired,
    MfaFailed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthzOutcome {
    Allowed,
    Denied { path: String, operation_name: String },
}

#[derive(Debug)]
pub struct AccountingEvent {
    pub principal_id: Option<PrincipalId>,
    pub auth_outcome: AuthOutcome,
    pub authz_outcome: Option<AuthzOutcome>,
    pub call_context: String,
    pub object_fragment: Option<String>,
    pub operation_name: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub source_ip: IpAddr,
    pub session_id: Option<SessionId>,
    pub tenant_id: TenantId,
}
