use std::fmt;
use std::error::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackError {
    ExecutionError(String),
    ValidationError(String),
    InternalError(String),
}

impl fmt::Display for CallbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallbackError::ExecutionError(msg) => write!(f, "Callback Execution Error: {}", msg),
            CallbackError::ValidationError(msg) => write!(f, "Callback Validation Error: {}", msg),
            CallbackError::InternalError(msg) => write!(f, "Callback Internal Error: {}", msg),
        }
    }
}

impl Error for CallbackError {}

impl From<String> for CallbackError {
    fn from(s: String) -> Self {
        CallbackError::ExecutionError(s)
    }
}

impl From<&str> for CallbackError {
    fn from(s: &str) -> Self {
        CallbackError::ExecutionError(s.to_string())
    }
}
