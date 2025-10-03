//! Numeric conversion functions
//! 
//! This module contains all conversion functions that involve numeric types.

use crate::HashMap;

/// Convert integer to string
pub fn integer_to_string(value: i64, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(value.to_string())
}

/// Convert float to string
pub fn float_to_string(value: f64, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(value.to_string())
}

/// Convert float to integer (truncates)
pub fn float_to_integer(value: f64, _parameters: &HashMap<String, String>) -> Result<i64, String> {
    Ok(value as i64)
}

/// Convert integer to float
pub fn integer_to_float(value: i64, _parameters: &HashMap<String, String>) -> Result<f64, String> {
    Ok(value as f64)
}

/// Convert unsigned integer to string
pub fn uinteger_to_string(value: u64, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(value.to_string())
}

/// Convert 32-bit integer to string
pub fn i32_to_string(value: i32, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(value.to_string())
}

/// Convert 64-bit integer to string
pub fn i64_to_string(value: i64, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(value.to_string())
}

/// Convert 32-bit float to string
pub fn f32_to_string(value: f32, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(value.to_string())
}

/// Convert 64-bit float to string
pub fn f64_to_string(value: f64, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(value.to_string())
}

/// Convert 32-bit float to integer
pub fn f32_to_integer(value: f32, _parameters: &HashMap<String, String>) -> Result<i64, String> {
    Ok(value as i64)
}

/// Convert 64-bit float to integer
pub fn f64_to_integer(value: f64, _parameters: &HashMap<String, String>) -> Result<i64, String> {
    Ok(value as i64)
}

/// Convert 32-bit integer to float
pub fn i32_to_float(value: i32, _parameters: &HashMap<String, String>) -> Result<f64, String> {
    Ok(value as f64)
}

/// Convert 64-bit integer to float
pub fn i64_to_float(value: i64, _parameters: &HashMap<String, String>) -> Result<f64, String> {
    Ok(value as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_integer_to_string() {
        let result = integer_to_string(123, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "123");
    }

    #[test]
    fn test_float_to_string() {
        let result = float_to_string(123.45, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "123.45");
    }

    #[test]
    fn test_float_to_integer() {
        let result = float_to_integer(123.7, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_integer_to_float() {
        let result = integer_to_float(123, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123.0);
    }

    #[test]
    fn test_uinteger_to_string() {
        let result = uinteger_to_string(123, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "123");
    }

    #[test]
    fn test_i32_to_string() {
        let result = i32_to_string(123, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "123");
    }

    #[test]
    fn test_i64_to_string() {
        let result = i64_to_string(123, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "123");
    }

    #[test]
    fn test_f32_to_string() {
        let result = f32_to_string(123.45, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "123.45");
    }

    #[test]
    fn test_f64_to_string() {
        let result = f64_to_string(123.45, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "123.45");
    }

    #[test]
    fn test_f32_to_integer() {
        let result = f32_to_integer(123.7, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_f64_to_integer() {
        let result = f64_to_integer(123.7, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123);
    }

    #[test]
    fn test_i32_to_float() {
        let result = i32_to_float(123, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123.0);
    }

    #[test]
    fn test_i64_to_float() {
        let result = i64_to_float(123, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123.0);
    }
}
