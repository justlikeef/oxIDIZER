use std::collections::HashMap;
use ox_type_converter::{ValueType, TypeConverter, CONVERSION_REGISTRY};
use std::any::{Any, TypeId};
use uuid::Uuid;

/// Represents a single attribute with its value, type, and conversion parameters.
#[derive(Debug)]
pub struct AttributeValue {
    pub value: Box<dyn Any + Send + Sync>,
    pub value_type: ValueType,
    pub value_type_parameters: HashMap<String, String>,
}

// Manual implementation of Clone for AttributeValue is tricky because of Box<dyn Any>.
// For rollback, we will move the value out and back in, avoiding the need for Clone.

impl AttributeValue {
    pub fn new<T: Any + Send + Sync + Clone + 'static>(value: T, value_type: ValueType) -> Self {
        AttributeValue {
            value: Box::new(value),
            value_type,
            value_type_parameters: HashMap::new(),
        }
    }

    pub fn with_parameters(mut self, parameters: HashMap<String, String>) -> Self {
        self.value_type_parameters = parameters;
        self
    }

    pub fn get_value<T: Clone + 'static>(&self) -> Option<T> {
        self.value.downcast_ref::<T>().cloned()
    }

    pub fn is<T: 'static>(&self) -> bool {
        self.value.is::<T>()
    }

    pub fn type_id(&self) -> TypeId {
        self.value.type_id()
    }

    pub fn to_string(&self) -> String {
        if let Some(s) = self.value.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = self.value.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(i) = self.value.downcast_ref::<i32>() {
            i.to_string()
        } else if let Some(f) = self.value.downcast_ref::<f64>() {
            f.to_string()
        } else if let Some(b) = self.value.downcast_ref::<bool>() {
            b.to_string()
        } else {
            format!("{:?}", self.value)
        }
    }
}

/// The main generic data object structure
#[derive(Debug)]
pub struct GenericDataObject {
    attributes: HashMap<String, AttributeValue>,
    pub identifier_name: String,
}

impl GenericDataObject {
    /// Create a new GenericDataObject with a required unique identifier.
    pub fn new(identifier_name: &str, id: Option<Uuid>) -> Self {
        let mut object = Self {
            attributes: HashMap::new(),
            identifier_name: identifier_name.to_string(),
        };

        let guid = id.unwrap_or_else(Uuid::new_v4);
        // Store the UUID as a string for universal compatibility and serialization.
        // This initial set should not fail, so we can safely expect success.
        object.set(identifier_name, guid.to_string());

        object
    }

    /// Get a value from the attributes HashMap.
    /// This function calls BeforeGet and AfterGet callbacks.
    /// Get a value from the attributes HashMap.
    pub fn get<T: Clone + 'static + Default>(&self, identifier: &str) -> Option<T>
    where
        T: Any + Clone,
    {
        let attr_value = self.attributes.get(identifier)?;

        if let Some(value) = attr_value.get_value::<T>() {
            Some(value)
        } else {
            let string_value = attr_value.to_string();
            let target_type = TypeConverter::infer_value_type(&T::default());
            let registry = CONVERSION_REGISTRY.lock().unwrap();
            match registry.convert_with_specific_converter(
                &attr_value.value_type.as_str(),
                &target_type.as_str(),
                &string_value,
                &attr_value.value_type_parameters
            ) {
                Ok(value) => value.downcast_ref::<T>().cloned(),
                Err(_) => None,
            }
        }
    }

    /// Set a value in the attributes HashMap
    /// Set a value in the attributes HashMap.
    /// Returns the old `AttributeValue` if an attribute was replaced, or `None` otherwise.
    pub fn set<T: Any + Send + Sync + Clone + 'static>(&mut self, identifier: &str, value: T) -> Option<AttributeValue> {
        let value_type = TypeConverter::infer_value_type(&value);
        self.set_with_type(identifier, value, value_type, None)
    }

    /// Set a value with explicit type and parameters.
    /// Returns the old `AttributeValue` if an attribute was replaced, or `None` otherwise.
    pub fn set_with_type<T: Any + Send + Sync + Clone + 'static>(
        &mut self, 
        identifier: &str, 
        value: T, 
        value_type: ValueType,
        parameters: Option<HashMap<String, String>>
    ) -> Option<AttributeValue> {
        let mut new_attr_value = AttributeValue::new(value, value_type);
        if let Some(params) = parameters {
            new_attr_value = new_attr_value.with_parameters(params);
        }
        self.attributes.insert(identifier.to_string(), new_attr_value)
    }



    // --- Other methods ---

    pub fn get_attribute(&self, identifier: &str) -> Option<&AttributeValue> {
        self.attributes.get(identifier)
    }

    pub fn get_raw_value<T: Any + Clone>(&self, identifier: &str) -> Option<T> {
        self.attributes.get(identifier)?.get_value::<T>()
    }

    pub fn has_attribute(&self, identifier: &str) -> bool {
        self.attributes.contains_key(identifier)
    }

    pub fn remove_attribute(&mut self, identifier: &str) -> Option<AttributeValue> {
        self.attributes.remove(identifier)
    }

    pub fn get_attribute_names(&self) -> Vec<&String> {
        self.attributes.keys().collect()
    }

    pub fn len(&self) -> usize {
        self.attributes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }
    
    pub fn to_serializable_map(&self) -> HashMap<String, (String, ValueType, HashMap<String, String>)> {
        self.attributes.iter().map(|(k, v)| {
            let coerced_value = TypeConverter::coerce_string(&v.to_string(), &v.value_type);
            (k.clone(), (coerced_value, v.value_type.clone(), v.value_type_parameters.clone()))
        }).collect()
    }

    pub fn set_attribute_value(&mut self, identifier: &str, attribute_value: AttributeValue) -> Option<AttributeValue> {
        self.attributes.insert(identifier.to_string(), attribute_value)
    }

    pub fn clear_attributes(&mut self) {
        self.attributes.clear();
    }

    pub fn set_attributes(&mut self, attributes: HashMap<String, AttributeValue>) {
        self.attributes = attributes;
    }

    pub fn from_serializable_map(
        serializable_map: HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        identifier_name: &str
    ) -> Self {
        let mut gdo = GenericDataObject {
            attributes: HashMap::new(),
            identifier_name: identifier_name.to_string(),
        };
        let registry = CONVERSION_REGISTRY.lock().unwrap();

        for (key, (value_str, value_type, parameters)) in serializable_map {
            let converted_value = registry.convert_with_specific_converter(
                &value_type.as_str(),
                &value_type.as_str(),
                &value_str,
                &parameters,
            );

            let attr_value = if let Ok(converted_box_any) = converted_value {
                AttributeValue {
                    value: converted_box_any,
                    value_type,
                    value_type_parameters: parameters,
                }
            } else {
                AttributeValue {
                    value: Box::new(value_str),
                    value_type: ValueType::new("string"),
                    value_type_parameters: parameters,
                }
            };
            gdo.attributes.insert(key, attr_value);
        }
        gdo
    }
}

impl Default for GenericDataObject {
    fn default() -> Self {
        Self::new("id", None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_data_object() {
        let data_object = GenericDataObject::new("id", None);
        assert!(!data_object.is_empty());
        assert_eq!(data_object.len(), 1);
        assert!(data_object.has_attribute("id"));
    }

    #[test]
    fn test_set_and_get_string() {
        let mut data_object = GenericDataObject::new("id", None);
        data_object.set("name", "John Doe".to_string());
        assert!(data_object.has_attribute("name"));
        let value: String = data_object.get("name").unwrap();
        assert_eq!(value, "John Doe");
    }

    #[test]
    fn test_to_serializable_map_coercion() {
        let mut data_object = GenericDataObject::new("id", None);
        // Set a float value but claim it is an Integer
        data_object.set_with_type("age", 25.5_f64, ValueType::Integer, None);
        
        // Also set a boolean from number
        data_object.set_with_type("is_active", "1".to_string(), ValueType::Boolean, None);

        let map = data_object.to_serializable_map();
        
        // Check age: should be "25" (coerced from 25.5)
        let (age_str, age_type, _) = map.get("age").unwrap();
        assert_eq!(age_str, "25");
        assert_eq!(*age_type, ValueType::Integer);

        // Check is_active: should be "true" (coerced from "1")
        let (active_str, active_type, _) = map.get("is_active").unwrap();
        assert_eq!(active_str, "true");
        assert_eq!(*active_type, ValueType::Boolean);
    }
}