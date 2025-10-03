#[cfg(test)]
mod tests {
    use ox_type_converter::registry::{CONVERSION_REGISTRY, ConversionRegistry};
    use std::collections::HashMap;

    #[test]
    fn test_registry_creation() {
        let registry = CONVERSION_REGISTRY.lock().unwrap();
        assert!(registry.has_conversion("string", "integer"));
        assert!(registry.has_conversion("string", "float"));
        assert!(registry.has_conversion("string", "boolean"));
    }

    #[test]
    fn test_get_converter() {
        let registry = CONVERSION_REGISTRY.lock().unwrap();
        assert!(registry.get_converter("string", "integer").is_some());
        assert!(registry.get_converter("string", "invalid").is_none());
    }

    #[test]
    fn test_convert_with_specific_converter() {
        let registry = CONVERSION_REGISTRY.lock().unwrap();
        let result = registry.convert_with_specific_converter("string", "integer", "123", &HashMap::new());
        assert!(result.is_ok());
        
        let result = registry.convert_with_specific_converter("string", "invalid", "123", &HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_available_conversions() {
        let registry = CONVERSION_REGISTRY.lock().unwrap();
        let conversions = registry.get_available_conversions();
        assert!(!conversions.is_empty());
        assert!(conversions.contains(&("string".to_string(), "integer".to_string())));
    }

    #[test]
    fn test_register_custom_conversion() {
        let mut registry = CONVERSION_REGISTRY.lock().unwrap();
        
        // Register a custom conversion
        registry.register_conversion("custom", "string", |v, _p| {
            Ok(Box::new(format!("custom_{}", v)) as Box<dyn std::any::Any + Send + Sync>)
        });
        
        assert!(registry.has_conversion("custom", "string"));
        
        let result = registry.convert_with_specific_converter("custom", "string", "test", &HashMap::new());
        assert!(result.is_ok());
    }
}