use std::collections::HashMap;
use ox_type_converter::{ValueType, TypeConverter, CONVERSION_REGISTRY};
use std::any::{Any, TypeId};
use ox_callback_manager::{CALLBACK_MANAGER, EventType};
use uuid::Uuid;
use std::cell::RefCell;
use ox_persistence::PERSISTENCE_DRIVER_REGISTRY;

/// Represents the hydration and persistence state of a GenericDataObject.
#[derive(Debug, Clone, PartialEq)]
pub enum DataObjectState {
    /// The object was newly created in memory and does not exist in the datastore.
    New,
    /// The object is a shell, containing only an ID. It represents a full object in the datastore.
    NotHydrated,
    /// The object is fully loaded from the datastore.
    Hydrated,
    /// The object has been modified in memory and is out of sync with the datastore.
    Modified,
    /// The object in memory is in sync with the datastore.
    Consistent,
    /// The object is marked for deletion from the datastore.
    Deleted,
}

/// Holds information required for a GenericDataObject to self-hydrate.
#[derive(Debug, Clone)]
pub struct PersistenceInfo {
    pub driver_name: String,
    pub location: String,
}

/// The main generic data object structure
#[derive(Debug, Clone)]
pub struct GenericDataObject {
    attributes: HashMap<String, AttributeValue>,
    pub state: DataObjectState,
    pub persistence_info: Option<PersistenceInfo>,
    pub identifier_name: String,
}

impl GenericDataObject {
    /// Create a new GenericDataObject with a required unique identifier.
    pub fn new(identifier_name: &str, id: Option<Uuid>) -> Self {
        let mut object = Self {
            attributes: HashMap::new(),
            state: DataObjectState::New,
            persistence_info: None,
            identifier_name: identifier_name.to_string(),
        };

        let guid = id.unwrap_or_else(Uuid::new_v4);
        // Store the UUID as a string for universal compatibility and serialization.
        object.set(identifier_name, guid.to_string()).unwrap();
        // Setting the ID shouldn't change the state from New
        object.state = DataObjectState::New;

        object
    }

    /// Hydrates the object by loading its full data from the datastore.
    fn hydrate(&mut self) -> Result<(), String> {
        if self.state != DataObjectState::NotHydrated {
            return Ok(()); // Already hydrated or new
        }

        let persistence_info = self.persistence_info.as_ref().ok_or("Cannot hydrate: No persistence info available.".to_string())?;
        let id = self.get_raw_value::<String>(&self.identifier_name).ok_or("Cannot hydrate: Object has no ID.".to_string())?;

        let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        let (driver, _) = registry.get_driver(&persistence_info.driver_name)
            .ok_or(format!("Driver '{}' not found for hydration.", persistence_info.driver_name))?;

        let restored_map = driver.restore(&persistence_info.location, &id)?;

        // Clear existing attributes and populate with restored data
        self.attributes.clear();
        let conversion_registry = CONVERSION_REGISTRY.lock().unwrap();

        for (key, (value_str, value_type, parameters)) in restored_map {
            let converted_value = conversion_registry.convert_with_specific_converter(
                value_type.as_str(),
                value_type.as_str(),
                &value_str,
                &parameters,
            );

            if let Ok(converted_box_any) = converted_value {
                let attr_value = AttributeValue {
                    value: converted_box_any,
                    value_type,
                    value_type_parameters: parameters,
                };
                self.attributes.insert(key, attr_value);
            } else {
                let attr_value = AttributeValue {
                    value: Box::new(value_str),
                    value_type: ValueType::String,
                    value_type_parameters: parameters,
                };
                self.attributes.insert(key, attr_value);
            }
        }

        self.state = DataObjectState::Hydrated;
        Ok(())
    }

    /// Get a value from the attributes HashMap
    /// This function calls BeforeGet and AfterGet callbacks.
    pub fn get<T: Clone + 'static + Default>(&mut self, identifier: &str) -> Option<T> 
    where
        T: Any + Clone,
    {
        // Auto-hydrate if the object is just a shell
        if self.state == DataObjectState::NotHydrated {
            self.hydrate().ok()?;
        }

        // Call BeforeGet callbacks
        let identifier_owned = identifier.to_string();
        self.trigger_callbacks_internal(&EventType::new("BeforeGet"), &[&identifier_owned]);

        // Retrieve the value, either directly or through conversion
        let original_value = {
            let attr_value = self.attributes.get(identifier)?;
            
            if let Some(value) = attr_value.get_value::<T>() {
                value
            } else {
                let string_value = attr_value.to_string();
                let target_type = TypeConverter::infer_value_type(&T::default());
                let registry = CONVERSION_REGISTRY.lock().unwrap();
                match registry.convert_with_specific_converter(
                    attr_value.value_type.as_str(),
                    target_type.as_str(),
                    &string_value,
                    &attr_value.value_type_parameters
                ) {
                    Ok(value) => value.downcast_ref::<T>().cloned()?,
                    Err(_) => return None,
                }
            }
        };

        // Wrap the retrieved value in a RefCell to allow mutable access in callbacks
        let mutable_value = RefCell::new(original_value);

        // Trigger AfterGet callbacks, passing the RefCell
        self.trigger_callbacks_internal(
            &EventType::new("AfterGet"), 
            &[&identifier_owned, &mutable_value]
        );

        // Return the potentially modified value from the RefCell
        Some(mutable_value.into_inner())
    }


    /// Set a value in the attributes HashMap
    pub fn set<T: Any + Send + Sync + Clone + 'static>(&mut self, identifier: &str, value: T) -> Result<(), String> {
        // Auto-hydrate if the object is just a shell
        if self.state == DataObjectState::NotHydrated {
            self.hydrate()?;
        }

        // Call BeforeSet callbacks
        let identifier_owned = identifier.to_string();
        self.trigger_callbacks_internal(&EventType::new("BeforeSet"), &[&identifier_owned, &value]);

        // Update state if the object is not new
        if self.state != DataObjectState::New {
            self.state = DataObjectState::Modified;
        }

        // Determine the value type from the input value
        let value_type = TypeConverter::infer_value_type(&value);
        
        // Create the attribute value with the original type
        let attr_value = AttributeValue::new(value.clone(), value_type);
        
        // Store the attribute
        self.attributes.insert(identifier.to_string(), attr_value);
        
        // Call AfterSet callbacks
        self.trigger_callbacks_internal(&EventType::new("AfterSet"), &[&identifier_owned, &value]);
        
        Ok(())
    }

    /// Set a value with explicit type and parameters
    pub fn set_with_type<T: Any + Send + Sync + Clone + 'static>(
        &mut self, 
        identifier: &str, 
        value: T, 
        value_type: ValueType,
        parameters: Option<HashMap<String, String>>
    ) -> Result<(), String> {
        // Auto-hydrate if the object is just a shell
        if self.state == DataObjectState::NotHydrated {
            self.hydrate()?;
        }

        // Call BeforeSet callbacks
        let identifier_owned = identifier.to_string();
        self.trigger_callbacks_internal(&EventType::new("BeforeSet"), &[&identifier_owned, &value]);

        if self.state != DataObjectState::New {
            self.state = DataObjectState::Modified;
        }

        let mut attr_value = AttributeValue::new(value.clone(), value_type);
        
        if let Some(params) = parameters {
            attr_value = attr_value.with_parameters(params);
        }
        
        self.attributes.insert(identifier.to_string(), attr_value);
        
        // Call AfterSet callbacks
        self.trigger_callbacks_internal(&EventType::new("AfterSet"), &[&identifier_owned, &value]);
        
        Ok(())
    }

    /// Register a callback function for a specific event
    pub fn register_callback<F>(&mut self, event: EventType, callback: F)
    where
        F: Fn(&dyn Any, &[&dyn Any]) + Send + Sync + 'static,
    {
        CALLBACK_MANAGER.lock().unwrap().register_callback(event, callback);
    }



    /// Get the raw attribute value (for debugging or advanced usage)
    pub fn get_attribute(&self, identifier: &str) -> Option<&AttributeValue> {
        self.attributes.get(identifier)
    }

    /// Get the raw value as a specific type (for advanced usage)
    pub fn get_raw_value<T: Any + Clone>(&self, identifier: &str) -> Option<T> {
        self.attributes.get(identifier)?.get_value::<T>()
    }

    /// Internal method to trigger callbacks without borrowing conflicts
    fn trigger_callbacks_internal(&mut self, event: &EventType, params: &[&dyn std::any::Any]) {
        // Use the immutable version to avoid borrowing conflicts
        CALLBACK_MANAGER.lock().unwrap().trigger_callbacks_immutable(event, params, self);
    }

    /// Check if an attribute exists
    pub fn has_attribute(&self, identifier: &str) -> bool {
        self.attributes.contains_key(identifier)
    }

    /// Remove an attribute
    pub fn remove_attribute(&mut self, identifier: &str) -> Option<AttributeValue> {
        self.attributes.remove(identifier)
    }

    /// Get all attribute identifiers
    pub fn get_attribute_names(&self) -> Vec<&String> {
        self.attributes.keys().collect()
    }

    /// Get the number of attributes
    pub fn len(&self) -> usize {
        self.attributes.len()
    }

    /// Check if the object is empty
    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }

    /// Converts the GenericDataObject into a serializable map representation.
    pub fn to_serializable_map(&self) -> HashMap<String, (String, ValueType, HashMap<String, String>)> {
        let mut serializable_map = HashMap::new();
        for (key, attr_value) in &self.attributes {
            serializable_map.insert(
                key.clone(),
                (
                    attr_value.to_string(),
                    attr_value.value_type.clone(),
                    attr_value.value_type_parameters.clone(),
                ),
            );
        }
        serializable_map
    }

    // Helper method to get a clone of the attributes map
    pub(crate) fn get_attributes_map(&self) -> &HashMap<String, AttributeValue> {
        &self.attributes
    }

    // Helper method to set attributes from a map (used for unlocking)
    pub(crate) fn _set_attributes_from_map(&mut self, attributes: HashMap<String, AttributeValue>) {
        self.attributes = attributes;
    }

    /// Restores the GenericDataObject from a serializable map representation.
    pub fn from_serializable_map(
        serializable_map: HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        identifier_name: &str
    ) -> Self {
        let mut gdo = GenericDataObject {
            attributes: HashMap::new(),
            state: DataObjectState::Hydrated, // This object is considered fully hydrated
            persistence_info: None, // We don't know the origin from just a map
            identifier_name: identifier_name.to_string(),
        };
        let registry = CONVERSION_REGISTRY.lock().unwrap();

        for (key, (value_str, value_type, parameters)) in serializable_map {
            // Attempt to convert the string back to its original type
            // This is a simplified approach and might need more robust type handling
            let converted_value = registry.convert_with_specific_converter(
                value_type.as_str(), // Source type
                value_type.as_str(), // Target type (attempt to restore original)
                &value_str,
                &parameters,
            );

            if let Ok(converted_box_any) = converted_value {
                let attr_value = AttributeValue {
                    value: converted_box_any,
                    value_type,
                    value_type_parameters: parameters,
                };
                self.attributes.insert(key, attr_value);
            } else {
                // Fallback if conversion fails, store as string
                let attr_value = AttributeValue {
                    value: Box::new(value_str),
                    value_type: ValueType::String,
                    value_type_parameters: parameters,
                };
                self.attributes.insert(key, attr_value);
            }
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
        assert_eq!(data_object.state, DataObjectState::New);
    }

    #[test]
    fn test_set_and_get_string() {
        let mut data_object = GenericDataObject::new("id", None);
        
        data_object.set("name", "John Doe").unwrap();
        assert!(data_object.has_attribute("name"));
        
        let value: String = data_object.get("name").unwrap();
        assert_eq!(value, "John Doe");
    }

    #[test]
    fn test_set_and_get_integer() {
        let mut data_object = GenericDataObject::new("id", None);
        
        data_object.set("age", 25).unwrap();
        
        let value: i32 = data_object.get("age").unwrap();
        assert_eq!(value, 25);
    }

    #[test]
    fn test_set_and_get_float() {
        let mut data_object = GenericDataObject::new("id", None);
        
        data_object.set("price", 19.99).unwrap();
        
        let value: f64 = data_object.get("price").unwrap();
        assert_eq!(value, 19.99);
    }

    #[test]
    fn test_callback_system() {
        let mut data_object = GenericDataObject::new("id", None);
        let callback_called = std::sync::Arc::new(std::sync::Mutex::new(false));
        let callback_called_clone = callback_called.clone();
        
        data_object.register_callback(EventType::new("BeforeGet"), move |_obj, _params| {
            *callback_called_clone.lock().unwrap() = true;
        });
        
        data_object.set("test", "value").unwrap();
        let _: String = data_object.get("test").unwrap();
        
        assert!(*callback_called.lock().unwrap());
    }

    #[test]
    fn test_get_nonexistent_attribute() {
        let mut data_object = GenericDataObject::new("id", None);
        let value: Option<String> = data_object.get("nonexistent");
        assert!(value.is_none());
    }

    #[test]
    fn test_raw_value_access() {
        let mut data_object = GenericDataObject::new("id", None);
        
        data_object.set("age", 25).unwrap();
        
        // Get raw value as the original type
        let raw_age: Option<i32> = data_object.get_raw_value("age");
        assert_eq!(raw_age, Some(25));
        
        // Get as string (converted)
        let age_string: Option<String> = data_object.get("age");
        assert_eq!(age_string, Some("25".to_string()));
        
        // Check attribute properties
        if let Some(attr) = data_object.get_attribute("age") {
            assert!(attr.is::<i32>());
            assert!(!attr.is::<String>());
            assert_eq!(attr.to_string(), "25");
        }
    }

    #[test]
    fn test_after_get_callback_modification() {
        let mut data_object = GenericDataObject::new("id", None);
        
        // Register an AfterGet callback to modify the string value
        data_object.register_callback(EventType::new("AfterGet"), move |_obj, params| {
            // Param 0: identifier (String)
            // Param 1: value (RefCell<T>)
            if let Some(identifier) = params.get(0).and_then(|p| p.downcast_ref::<String>()) {
                if identifier == "name" {
                    if let Some(value_cell) = params.get(1).and_then(|p| p.downcast_ref::<RefCell<String>>()) {
                        let mut value = value_cell.borrow_mut();
                        *value = format!("Modified {}", *value);
                    }
                }
            }
        });
        
        data_object.set("name", "Original").unwrap();
        
        // Get the value, which should trigger the AfterGet callback
        let value: String = data_object.get("name").unwrap();
        
        // Check that the value was modified by the callback
        assert_eq!(value, "Modified Original");
    }
}
