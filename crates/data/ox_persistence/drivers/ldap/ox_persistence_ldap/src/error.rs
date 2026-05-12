//! Maps ldap3 and driver-specific errors to OxDataError.

use ox_data_error::OxDataError;

#[derive(Debug)]
pub enum LdapDriverError {
    ConnectionFailed(String),
    BindFailed(String),
    SearchFailed(String),
    AddFailed(String),
    ModifyFailed(String),
    NotFound(String),
    InvalidConfig(String),
}

impl From<LdapDriverError> for OxDataError {
    fn from(e: LdapDriverError) -> Self {
        match e {
            LdapDriverError::ConnectionFailed(m) => OxDataError::DriverError(format!("LDAP connection failed: {}", m)),
            LdapDriverError::BindFailed(m)       => OxDataError::DriverError(format!("LDAP bind failed: {}", m)),
            LdapDriverError::SearchFailed(m)     => OxDataError::DriverError(format!("LDAP search failed: {}", m)),
            LdapDriverError::AddFailed(m)        => OxDataError::DriverError(format!("LDAP add failed: {}", m)),
            LdapDriverError::ModifyFailed(m)     => OxDataError::DriverError(format!("LDAP modify failed: {}", m)),
            LdapDriverError::NotFound(id)        => OxDataError::InternalError(format!("LDAP entry not found: {}", id)),
            LdapDriverError::InvalidConfig(m)    => OxDataError::DriverError(format!("LDAP driver config error: {}", m)),
        }
    }
}
