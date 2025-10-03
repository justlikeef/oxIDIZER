use crate::HashMap;
use crate::converters::*;
use lazy_static::lazy_static;
use std::sync::Mutex;

/// Conversion function type that can convert from string to any type
pub type ConversionFn = fn(&str, &HashMap<String, String>) -> Result<Box<dyn crate::Any + Send + Sync>, String>;

/// Registry for managing conversion functions
pub struct ConversionRegistry {
    conversions: HashMap<String, HashMap<String, ConversionFn>>,
}

lazy_static! {
    /// The global conversion registry
    pub static ref CONVERSION_REGISTRY: Mutex<ConversionRegistry> = Mutex::new(ConversionRegistry::new());
}

impl ConversionRegistry {
    /// Create a new empty conversion registry
    fn new() -> Self {
        let mut registry = Self {
            conversions: HashMap::new(),
        };
        
        // Register all the built-in conversions
        registry.register_builtin_conversions();
        
        registry
    }

    /// Register a conversion function
    pub fn register_conversion(&mut self, from_type: &str, to_type: &str, converter: ConversionFn) {
        self.conversions
            .entry(from_type.to_string())
            .or_insert_with(HashMap::new)
            .insert(to_type.to_string(), converter);
    }

    /// Get a conversion function for a specific conversion
    pub fn get_converter(&self, from_type: &str, to_type: &str) -> Option<&ConversionFn> {
        self.conversions
            .get(from_type)
            .and_then(|to_types| to_types.get(to_type))
    }

    /// Convert using the appropriate specific converter
    pub fn convert_with_specific_converter(
        &self, 
        from_type: &str, 
        to_type: &str, 
        value: &str, 
        parameters: &HashMap<String, String>
    ) -> Result<Box<dyn crate::Any + Send + Sync>, String> {
        if let Some(converter) = self.get_converter(from_type, to_type) {
            converter(value, parameters)
        } else {
            Err(format!("No converter available from '{}' to '{}'", from_type, to_type))
        }
    }

    /// Get all available conversion types
    pub fn get_available_conversions(&self) -> Vec<(String, String)> {
        let mut conversions = Vec::new();
        for (from_type, to_types) in &self.conversions {
            for to_type in to_types.keys() {
                conversions.push((from_type.clone(), to_type.clone()));
            }
        }
        conversions
    }

    /// Check if a conversion is available
    pub fn has_conversion(&self, from_type: &str, to_type: &str) -> bool {
        self.get_converter(from_type, to_type).is_some()
    }

    /// Register all built-in conversion functions
    fn register_builtin_conversions(&mut self) {
        // String conversions
        self.register_conversion("string", "integer", |v, p| {
            string_to_integer(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("string", "float", |v, p| {
            string_to_float(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("string", "boolean", |v, p| {
            string_to_boolean(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("string", "string", |v, p| {
            string_to_string(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("string", "uinteger", |v, p| {
            string_to_uinteger(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("string", "i32", |v, p| {
            string_to_i32(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("string", "i64", |v, p| {
            string_to_i64(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("string", "f32", |v, p| {
            string_to_f32(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("string", "f64", |v, p| {
            string_to_f64(v, p).map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });

        // Numeric conversions
        self.register_conversion("integer", "string", |v, p| {
            v.parse::<i64>().map_err(|e| format!("Failed to parse integer: {:?}", e))
                .and_then(|val| integer_to_string(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("float", "string", |v, p| {
            v.parse::<f64>().map_err(|e| format!("Failed to parse float: {:?}", e))
                .and_then(|val| float_to_string(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("uinteger", "string", |v, p| {
            v.parse::<u64>().map_err(|e| format!("Failed to parse unsigned integer: {:?}", e))
                .and_then(|val| uinteger_to_string(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("i32", "string", |v, p| {
            v.parse::<i32>().map_err(|e| format!("Failed to parse i32: {:?}", e))
                .and_then(|val| i32_to_string(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("i64", "string", |v, p| {
            v.parse::<i64>().map_err(|e| format!("Failed to parse i64: {:?}", e))
                .and_then(|val| i64_to_string(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("f32", "string", |v, p| {
            v.parse::<f32>().map_err(|e| format!("Failed to parse f32: {:?}", e))
                .and_then(|val| f32_to_string(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("f64", "string", |v, p| {
            v.parse::<f64>().map_err(|e| format!("Failed to parse f64: {:?}", e))
                .and_then(|val| f64_to_string(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });

        // Boolean conversions
        self.register_conversion("boolean", "string", |v, p| {
            v.parse::<bool>().map_err(|e| format!("Failed to parse boolean: {:?}", e))
                .and_then(|val| boolean_to_string(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });

        // Cross-type conversions
        self.register_conversion("float", "integer", |v, p| {
            v.parse::<f64>().map_err(|e| format!("Failed to parse float: {:?}", e))
                .and_then(|val| float_to_integer(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("integer", "float", |v, p| {
            v.parse::<i64>().map_err(|e| format!("Failed to parse integer: {:?}", e))
                .and_then(|val| integer_to_float(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("boolean", "integer", |v, p| {
            v.parse::<bool>().map_err(|e| format!("Failed to parse boolean: {:?}", e))
                .and_then(|val| boolean_to_integer(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
        self.register_conversion("integer", "boolean", |v, p| {
            v.parse::<i64>().map_err(|e| format!("Failed to parse integer: {:?}", e))
                .and_then(|val| integer_to_boolean(val, p))
                .map(|val| Box::new(val) as Box<dyn crate::Any + Send + Sync>)
        });
    }
}
