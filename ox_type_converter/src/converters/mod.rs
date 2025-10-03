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
    pub fn infer_value_type<T: 'static>(_value: &T) -> ValueType {
        use std::any::TypeId;
        
        let type_id = TypeId::of::<T>();
        
        if type_id == TypeId::of::<String>() || type_id == TypeId::of::<&str>() {
            ValueType::new("string")
        } else if type_id == TypeId::of::<i32>() || type_id == TypeId::of::<i64>() || type_id == TypeId::of::<u32>() || type_id == TypeId::of::<u64>() {
            ValueType::new("integer")
        } else if type_id == TypeId::of::<f32>() || type_id == TypeId::of::<f64>() {
            ValueType::new("float")
        } else if type_id == TypeId::of::<bool>() {
            ValueType::new("boolean")
        } else {
            // For custom types, use the type name
            let type_name = std::any::type_name::<T>();
            ValueType::new(type_name)
        }
    }

    /// Convert a value to string representation
    pub fn to_string<T: ToString>(value: &T) -> String {
        value.to_string()
    }

    /// Validate if a value can be converted to the specified type
    pub fn can_convert_to(value: &str, value_type: &ValueType) -> bool {
        match value_type.as_str() {
            "string" | "String" => true,
            "integer" | "Integer" | "int" | "i32" | "i64" => {
                value.parse::<i64>().is_ok()
            },
            "float" | "Float" | "f32" | "f64" | "double" => {
                value.parse::<f64>().is_ok()
            },
            "boolean" | "Boolean" | "bool" => {
                value.parse::<bool>().is_ok()
            },
            _ => {
                // For custom types, we assume they can be converted
                // This could be enhanced with a registry of custom converters
                true
            },
        }
    }

    /// Get supported type names
    pub fn supported_types() -> Vec<&'static str> {
        vec![
            "string", "String",
            "integer", "Integer", "int", "i32", "i64",
            "float", "Float", "f32", "f64", "double",
            "boolean", "Boolean", "bool",
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
        assert_eq!(TypeConverter::infer_value_type(&"hello"), ValueType::new("string"));
        assert_eq!(TypeConverter::infer_value_type(&42), ValueType::new("integer"));
        assert_eq!(TypeConverter::infer_value_type(&3.14), ValueType::new("float"));
        assert_eq!(TypeConverter::infer_value_type(&true), ValueType::new("boolean"));
    }

    #[test]
    fn test_can_convert_to() {
        assert!(TypeConverter::can_convert_to("123", &ValueType::new("integer")));
        assert!(TypeConverter::can_convert_to("123.45", &ValueType::new("float")));
        assert!(TypeConverter::can_convert_to("true", &ValueType::new("boolean")));
        assert!(!TypeConverter::can_convert_to("not_a_number", &ValueType::new("integer")));
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
