//! Type conversion modules
//! 
//! This module automatically includes all individual conversion files
//! making it easy to add new conversion routines.

// Automatically include all conversion modules
pub mod string_conversions;
pub mod numeric_conversions;
pub mod boolean_conversions;
pub mod generic_conversions;

pub use string_conversions::*;
pub use numeric_conversions::*;
pub use boolean_conversions::*;
pub use generic_conversions::*;

use crate::value_type::ValueType;

/// Main type converter that provides access to all conversion functions
pub struct TypeConverter;

impl TypeConverter {
    /// Infer the value type from the input value
    /// Infer the value type from the input value
    pub fn infer_value_type<T: 'static>(value: &T) -> ValueType {
        use std::any::{TypeId, Any};
        
        let type_id = TypeId::of::<T>();
        
        if type_id == TypeId::of::<String>() {
             if let Some(s) = (value as &dyn Any).downcast_ref::<String>() {
                 return Self::infer_from_string(s);
             }
        } else if type_id == TypeId::of::<&str>() {
             if let Some(s) = (value as &dyn Any).downcast_ref::<&str>() {
                 return Self::infer_from_string(s);
             }
        }
        
        if type_id == TypeId::of::<i32>() || type_id == TypeId::of::<i64>() || type_id == TypeId::of::<u32>() || type_id == TypeId::of::<u64>() {
            ValueType::Integer
        } else if type_id == TypeId::of::<f32>() || type_id == TypeId::of::<f64>() {
            ValueType::Float
        } else if type_id == TypeId::of::<bool>() {
            ValueType::Boolean
        } else {
            // For custom types, use the type name
            let type_name = std::any::type_name::<T>();
            // Avoid marking String as Custom if downcast failed (unlikely)
            if type_name == "alloc::string::String" || type_name == "str" || type_name == "&str" {
                ValueType::String
            } else {
                ValueType::Custom(type_name.to_string())
            }
        }
    }

    fn infer_from_string(value: &str) -> ValueType {
        if value.parse::<i64>().is_ok() {
            ValueType::Integer
        } else if value.parse::<f64>().is_ok() {
            ValueType::Float
        } else if value.parse::<bool>().is_ok() {
            ValueType::Boolean
        } else if chrono::DateTime::parse_from_rfc3339(value).is_ok() {
            ValueType::DateTime
        } else {
            ValueType::String
        }
    }

    /// Convert a value to string representation
    pub fn to_string<T: ToString>(value: &T) -> String {
        value.to_string()
    }

    /// Check if a value can be converted to the target type
    pub fn can_convert_to(value: &str, value_type: &ValueType, target_type: &ValueType) -> bool {
        // If types are the same, conversion is always possible (identity)
        if value_type == target_type {
            return true;
        }

        match target_type {
            ValueType::String => true, // Everything can be a string
            ValueType::Integer => value.parse::<i64>().is_ok(),
            ValueType::Float => value.parse::<f64>().is_ok(),
            ValueType::Boolean => value.parse::<bool>().is_ok(),
            ValueType::DateTime => chrono::DateTime::parse_from_rfc3339(value).is_ok(),
            ValueType::Binary => false, // Cannot easily check binary validness from string without context
            ValueType::List(_) => false, // Complex types handled separately
            ValueType::Map => false,
            ValueType::Custom(_) => true, // Custom types assume flexible conversion for now
        }
    }

    /// Coerce a string value to the target type's string representation if possible
    pub fn coerce_string(value: &str, target_type: &ValueType) -> String {
        match target_type {
            ValueType::Integer => {
                 if value.parse::<i64>().is_ok() {
                     value.to_string()
                 } else if let Ok(f) = value.parse::<f64>() {
                     (f as i64).to_string()
                 } else {
                     value.to_string()
                 }
            },
            ValueType::Float => {
                if value.parse::<f64>().is_ok() {
                     value.to_string()
                } else {
                     value.to_string()
                }
            },
            ValueType::Boolean => {
                 if let Ok(b) = value.parse::<bool>() {
                     b.to_string()
                 } else {
                     match value.to_lowercase().as_str() {
                         "1" | "yes" | "on" => "true".to_string(),
                         "0" | "no" | "off" => "false".to_string(),
                         _ => value.to_string()
                     }
                 }
            },
            _ => value.to_string()
        }
    }
    /// Get supported type names
    pub fn supported_types() -> Vec<&'static str> {
        vec![
            "string", "String",
            "integer", "Integer", "int", "i32", "i64",
            "float", "Float", "f32", "f64", "double",
            "boolean", "Boolean", "bool",
            "datetime", "DateTime",
        ]
    }

    /// Check if a type is supported
    pub fn is_supported_type(type_name: &str) -> bool {
        Self::supported_types().contains(&type_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_value_type() {
        assert_eq!(TypeConverter::infer_value_type(&"hello"), ValueType::String);
        assert_eq!(TypeConverter::infer_value_type(&42), ValueType::Integer);
        assert_eq!(TypeConverter::infer_value_type(&3.14), ValueType::Float);
        assert_eq!(TypeConverter::infer_value_type(&true), ValueType::Boolean);
    }

    #[test]
    fn test_can_convert_to() {
        assert!(TypeConverter::can_convert_to("123", &ValueType::Integer));
        assert!(TypeConverter::can_convert_to("123.45", &ValueType::Float));
        assert!(TypeConverter::can_convert_to("true", &ValueType::Boolean));
        assert!(!TypeConverter::can_convert_to("not_a_number", &ValueType::Integer));
    }

    #[test]
    fn test_supported_types() {
        let types = TypeConverter::supported_types();
        assert!(types.contains(&"integer"));
        assert!(types.contains(&"float"));
        assert!(types.contains(&"boolean"));
        assert!(types.contains(&"string"));
    }

    #[test]
    fn test_is_supported_type() {
        assert!(TypeConverter::is_supported_type("integer"));
        assert!(TypeConverter::is_supported_type("float"));
        assert!(TypeConverter::is_supported_type("boolean"));
        assert!(TypeConverter::is_supported_type("string"));
        assert!(!TypeConverter::is_supported_type("unsupported_type"));
    }
}
