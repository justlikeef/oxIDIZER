//! Generic conversion functions
//! 
//! This module contains generic conversion functions that can work with any type.

use crate::value_type::ValueType;
use crate::HashMap;
use std::fmt;

/// Generic conversion function that routes to appropriate specific converter
pub fn convert_value<T>(value: &str, value_type: &ValueType, _parameters: &HashMap<String, String>) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: fmt::Debug,
{
    match value_type.as_str() {
        "string" | "String" => {
            // For string type, we need to handle this specially since we're already storing as string
            // This is a bit of a limitation of the current design
            Err("String type conversion not implemented for generic get".to_string())
        },
        "integer" | "Integer" | "int" | "i32" | "i64" => {
            value.parse::<T>().map_err(|e| format!("Failed to parse integer: {:?}", e))
        },
        "float" | "Float" | "f32" | "f64" | "double" => {
            value.parse::<T>().map_err(|e| format!("Failed to parse float: {:?}", e))
        },
        "boolean" | "Boolean" | "bool" => {
            value.parse::<T>().map_err(|e| format!("Failed to parse boolean: {:?}", e))
        },
        _ => {
            // For custom types, we might need special handling
            value.parse::<T>().map_err(|e| format!("Failed to parse custom type '{}': {:?}", value_type.as_str(), e))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_integer() {
        let result = convert_value::<i32>("123", &ValueType::new("integer"), &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_convert_float() {
        let result = convert_value::<f64>("123.45", &ValueType::new("float"), &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123.45);
    }

    #[test]
    fn test_convert_boolean() {
        let result = convert_value::<bool>("true", &ValueType::new("boolean"), &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    #[test]
    fn test_convert_invalid_integer() {
        let result = convert_value::<i32>("not_a_number", &ValueType::new("integer"), &HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_convert_string_type() {
        let result = convert_value::<String>("hello", &ValueType::new("string"), &HashMap::new());
        assert!(result.is_err()); // String type conversion not implemented
    }
}
