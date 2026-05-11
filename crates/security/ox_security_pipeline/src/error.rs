use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("authentication failed: {0}")]
    AuthFailed(String),
    #[error("mfa required: {0}")]
    MfaRequired(String),
    #[error("authorization denied: {0}")]
    AuthzDenied(String),
}
