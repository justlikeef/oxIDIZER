use std::fmt;

use serde::{Serialize, Deserialize};

/// Represents the type of a value stored in the data object
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ValueType {
    String,
    Integer,
    Float,
    Boolean,
    Binary,
    /// A list of items, where all items share the same type
    List(Box<ValueType>),
    /// A map (key-value pairs), representing a nested object structure
    Map,
    /// A timestamp or date-time value
    DateTime,
    /// Fallback for custom or unknown types, storing the type name as a string
    Custom(String),
}

impl ValueType {
    /// Create a new ValueType from a string representation
    /// This is for backward compatibility and parsing
    pub fn new(type_name: &str) -> Self {
        match type_name.to_lowercase().as_str() {
            "string" => ValueType::String,
            "integer" | "int" => ValueType::Integer,
            "float" | "double" => ValueType::Float,
            "boolean" | "bool" => ValueType::Boolean,
            "binary" | "blob" => ValueType::Binary,
            "map" | "object" => ValueType::Map,
            "datetime" | "date" | "timestamp" => ValueType::DateTime,
            _ => ValueType::Custom(type_name.to_string()),
        }
    }
    
    /// Get the type name as a string slice (for internal logic that still expects strings)
    pub fn as_str(&self) -> String {
        self.to_string()
    }
}

impl From<&str> for ValueType {
    fn from(s: &str) -> Self {
        ValueType::new(s)
    }
}

impl From<String> for ValueType {
    fn from(s: String) -> Self {
        ValueType::new(&s)
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueType::String => write!(f, "string"),
            ValueType::Integer => write!(f, "integer"),
            ValueType::Float => write!(f, "float"),
            ValueType::Boolean => write!(f, "boolean"),
            ValueType::Binary => write!(f, "binary"),
            ValueType::List(inner) => write!(f, "list<{}>", inner),
            ValueType::Map => write!(f, "map"),
            ValueType::DateTime => write!(f, "datetime"),
            ValueType::Custom(s) => write!(f, "{}", s),
        }
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
