//! OktaDriverError to OxDataError conversion.

use ox_data_error::OxDataError;

#[derive(Debug)]
pub enum OktaDriverError {
    HttpError(String),
    NotFound(String),
    InvalidConfig(String),
    DeserializationError(String),
}

impl From<OktaDriverError> for OxDataError {
    fn from(e: OktaDriverError) -> Self {
        match e {
            OktaDriverError::HttpError(m)           => OxDataError::DriverError(format!("Okta HTTP error: {}", m)),
            OktaDriverError::NotFound(id)           => OxDataError::InternalError(format!("Okta entity not found: {}", id)),
            OktaDriverError::InvalidConfig(m)       => OxDataError::DriverError(format!("Okta config error: {}", m)),
            OktaDriverError::DeserializationError(m) => OxDataError::InternalError(format!("Okta deserialize error: {}", m)),
        }
    }
}
