//! String conversion functions
//! 
//! This module contains all conversion functions that involve strings.

use crate::HashMap;

/// Convert string to integer
pub fn string_to_integer(value: &str, _parameters: &HashMap<String, String>) -> Result<i64, String> {
    value.parse::<i64>().map_err(|e| format!("Failed to parse integer: {:?}", e))
}

/// Convert string to float
pub fn string_to_float(value: &str, _parameters: &HashMap<String, String>) -> Result<f64, String> {
    value.parse::<f64>().map_err(|e| format!("Failed to parse float: {:?}", e))
}

/// Convert string to boolean
pub fn string_to_boolean(value: &str, _parameters: &HashMap<String, String>) -> Result<bool, String> {
    value.parse::<bool>().map_err(|e| format!("Failed to parse boolean: {:?}", e))
}

/// Convert string to string (identity conversion)
pub fn string_to_string(value: &str, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(value.to_string())
}

/// Convert string to unsigned integer
pub fn string_to_uinteger(value: &str, _parameters: &HashMap<String, String>) -> Result<u64, String> {
    value.parse::<u64>().map_err(|e| format!("Failed to parse unsigned integer: {:?}", e))
}

/// Convert string to 32-bit integer
pub fn string_to_i32(value: &str, _parameters: &HashMap<String, String>) -> Result<i32, String> {
    value.parse::<i32>().map_err(|e| format!("Failed to parse i32: {:?}", e))
}

/// Convert string to 64-bit integer
pub fn string_to_i64(value: &str, _parameters: &HashMap<String, String>) -> Result<i64, String> {
    value.parse::<i64>().map_err(|e| format!("Failed to parse i64: {:?}", e))
}

/// Convert string to 32-bit float
pub fn string_to_f32(value: &str, _parameters: &HashMap<String, String>) -> Result<f32, String> {
    value.parse::<f32>().map_err(|e| format!("Failed to parse f32: {:?}", e))
}

/// Convert string to 64-bit float
pub fn string_to_f64(value: &str, _parameters: &HashMap<String, String>) -> Result<f64, String> {
    value.parse::<f64>().map_err(|e| format!("Failed to parse f64: {:?}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_to_integer() {
        let result = string_to_integer("123", &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_string_to_float() {
        let result = string_to_float("123.45", &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123.45);
    }

    #[test]
    fn test_string_to_boolean() {
        let result = string_to_boolean("true", &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    #[test]
    fn test_string_to_string() {
        let result = string_to_string("hello", &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn test_string_to_uinteger() {
        let result = string_to_uinteger("123", &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_string_to_i32() {
        let result = string_to_i32("123", &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_string_to_i64() {
        let result = string_to_i64("123", &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_string_to_f32() {
        let result = string_to_f32("123.45", &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123.45);
    }

    #[test]
    fn test_string_to_f64() {
        let result = string_to_f64("123.45", &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123.45);
    }

    #[test]
    fn test_invalid_conversions() {
        assert!(string_to_integer("not_a_number", &HashMap::new()).is_err());
        assert!(string_to_float("not_a_float", &HashMap::new()).is_err());
        assert!(string_to_boolean("not_a_bool", &HashMap::new()).is_err());
    }
}
