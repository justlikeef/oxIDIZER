//! Boolean conversion functions
//! 
//! This module contains all conversion functions that involve boolean types.

use crate::HashMap;

/// Convert boolean to string
pub fn boolean_to_string(value: bool, _parameters: &HashMap<String, String>) -> Result<String, String> {
    Ok(value.to_string())
}

/// Convert boolean to integer
pub fn boolean_to_integer(value: bool, _parameters: &HashMap<String, String>) -> Result<i64, String> {
    Ok(if value { 1 } else { 0 })
}

/// Convert integer to boolean
pub fn integer_to_boolean(value: i64, _parameters: &HashMap<String, String>) -> Result<bool, String> {
    Ok(value != 0)
}

/// Convert boolean to unsigned integer
pub fn boolean_to_uinteger(value: bool, _parameters: &HashMap<String, String>) -> Result<u64, String> {
    Ok(if value { 1 } else { 0 })
}

/// Convert unsigned integer to boolean
pub fn uinteger_to_boolean(value: u64, _parameters: &HashMap<String, String>) -> Result<bool, String> {
    Ok(value != 0)
}

/// Convert boolean to float
pub fn boolean_to_float(value: bool, _parameters: &HashMap<String, String>) -> Result<f64, String> {
    Ok(if value { 1.0 } else { 0.0 })
}

/// Convert float to boolean
pub fn float_to_boolean(value: f64, _parameters: &HashMap<String, String>) -> Result<bool, String> {
    Ok(value != 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boolean_to_string() {
        let result = boolean_to_string(true, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "true");

        let result = boolean_to_string(false, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "false");
    }

    #[test]
    fn test_boolean_to_integer() {
        let result = boolean_to_integer(true, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);

        let result = boolean_to_integer(false, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_integer_to_boolean() {
        let result = integer_to_boolean(5, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);

        let result = integer_to_boolean(0, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);

        let result = integer_to_boolean(-5, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    #[test]
    fn test_boolean_to_uinteger() {
        let result = boolean_to_uinteger(true, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);

        let result = boolean_to_uinteger(false, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_uinteger_to_boolean() {
        let result = uinteger_to_boolean(5, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);

        let result = uinteger_to_boolean(0, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);
    }

    #[test]
    fn test_boolean_to_float() {
        let result = boolean_to_float(true, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1.0);

        let result = boolean_to_float(false, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0.0);
    }

    #[test]
    fn test_float_to_boolean() {
        let result = float_to_boolean(5.5, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);

        let result = float_to_boolean(0.0, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);

        let result = float_to_boolean(-5.5, &HashMap::new());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }
}
