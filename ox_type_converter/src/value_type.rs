use std::fmt;

use serde::{Serialize, Deserialize};

/// Represents the type of a value stored in the data object
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueType(pub String);

impl ValueType {
    /// Create a new ValueType with the given type name
    pub fn new(type_name: &str) -> Self {
        Self(type_name.to_string())
    }
    
    /// Get the type name as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ValueType {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ValueType {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_type_creation() {
        let value_type = ValueType::new("integer");
        assert_eq!(value_type.as_str(), "integer");
    }

    #[test]
    fn test_value_type_from_string() {
        let value_type: ValueType = "float".into();
        assert_eq!(value_type.as_str(), "float");
    }

    #[test]
    fn test_value_type_display() {
        let value_type = ValueType::new("boolean");
        assert_eq!(value_type.to_string(), "boolean");
    }
}
