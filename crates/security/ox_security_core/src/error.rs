use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthzError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("access denied: operation '{operation}' at '{path}'")]
    Denied { path: String, operation: String },
    #[error("context not registered: '{0}'")]
    UnregisteredContext(String),
    #[error("internal authz error: {0}")]
    Internal(String),
}

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("authentication error: {0}")]
    Auth(String),
    #[error("authorization error: {0}")]
    Authz(#[from] AuthzError),
    #[error("accounting error: {0}")]
    Accounting(String),
    #[error("configuration error: {0}")]
    Config(String),
}
