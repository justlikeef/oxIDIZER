use std::fmt;
use std::error::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OxDataError {
    ConversionError(String),
    TypeMismatch { expected: String, found: String },
    RegistryError(String),
    InternalError(String),
    DriverError(String),
    ValidationError(String),
    TransactionError(String),
    CallbackError(String),
}

impl fmt::Display for OxDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OxDataError::ConversionError(msg) => write!(f, "Conversion Error: {}", msg),
            OxDataError::TypeMismatch { expected, found } => {
                write!(f, "Type Mismatch: expected {}, found {}", expected, found)
            }
            OxDataError::RegistryError(msg) => write!(f, "Registry Error: {}", msg),
            OxDataError::InternalError(msg) => write!(f, "Internal Error: {}", msg),
            OxDataError::DriverError(msg) => write!(f, "Driver Error: {}", msg),
            OxDataError::ValidationError(msg) => write!(f, "Validation Error: {}", msg),
            OxDataError::TransactionError(msg) => write!(f, "Transaction Error: {}", msg),
            OxDataError::CallbackError(msg) => write!(f, "Callback Error: {}", msg),
        }
    }
}

impl Error for OxDataError {}

impl From<std::num::ParseIntError> for OxDataError {
    fn from(e: std::num::ParseIntError) -> Self {
        OxDataError::ConversionError(e.to_string())
    }
}

impl From<std::num::ParseFloatError> for OxDataError {
    fn from(e: std::num::ParseFloatError) -> Self {
        OxDataError::ConversionError(e.to_string())
    }
}

impl From<std::str::ParseBoolError> for OxDataError {
    fn from(e: std::str::ParseBoolError) -> Self {
        OxDataError::ConversionError(e.to_string())
    }
}
