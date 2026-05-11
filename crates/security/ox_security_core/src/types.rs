use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrincipalId(Uuid);

impl PrincipalId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for PrincipalId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupId(String);

impl GroupId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantId(String);

impl TenantId {
    pub fn from_str(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            return Err("tenant id must not be empty".to_string());
        }
        Ok(Self(s.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SessionToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, strum::Display, strum::EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum AuthSource {
    Local,
    Ldap,
    Ad,
    Kerberos,
    Radius,
    Tacacs,
    Okta,
    Saml,
    Oidc,
    ApiKey,
    Mtls,
}
