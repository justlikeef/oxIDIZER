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
    match value_type {
        ValueType::String => {
            // For string type, we need to handle this specially since we're already storing as string
            // This is a bit of a limitation of the current design
            Err("String type conversion not implemented for generic get".to_string())
        },
        ValueType::Integer => {
            value.parse::<T>().map_err(|e| format!("Failed to parse integer: {:?}", e))
        },
        ValueType::Float => {
            value.parse::<T>().map_err(|e| format!("Failed to parse float: {:?}", e))
        },
        ValueType::Boolean => {
            value.parse::<T>().map_err(|e| format!("Failed to parse boolean: {:?}", e))
        },
        ValueType::DateTime => {
             if chrono::DateTime::parse_from_rfc3339(&value).is_ok() {
                 // If the target type is String, we can return the original value directly.
                 // Otherwise, we rely on FromStr for T.
                 // This assumes T can be constructed from a valid RFC3339 string.
                 value.parse::<T>().map_err(|e| format!("Failed to parse DateTime: {:?}", e))
             } else {
                 Err(format!("Value '{}' is not a valid ISO8601 DateTime", value))
             }
        },
        ValueType::List(_) => Err(format!("Cannot safely convert List '{}' blindly", value)),
        ValueType::Map => Err(format!("Cannot safely convert Map '{}' blindly", value)),
        ValueType::Binary => Err(format!("Cannot safely convert Binary '{}' blindly", value)),
        ValueType::Custom(_) => Ok(value.parse::<T>().map_err(|e| format!("Failed to parse custom type: {:?}", e))?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_integer() {
        let result = convert_value::<i32>("123", &ValueType::Integer, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_convert_float() {
        let result = convert_value::<f64>("123.45", &ValueType::Float, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123.45);
    }

    #[test]
    fn test_convert_boolean() {
        let result = convert_value::<bool>("true", &ValueType::Boolean, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    #[test]
    fn test_convert_invalid_integer() {
        let result = convert_value::<i32>("not_a_number", &ValueType::Integer, &HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_convert_string_type() {
        let result = convert_value::<String>("hello", &ValueType::String, &HashMap::new());
        assert!(result.is_err()); // String type conversion not implemented
    }
}
