use std::collections::HashMap;
use ox_type_converter::{ValueType, TypeConverter, CONVERSION_REGISTRY};
use ox_callback_manager::{ CallbackManager, EventType, CallbackFn, CallbackParams };
use ox_data_error::OxDataError;
use std::any::{Any, TypeId};
use std::sync::Mutex;
use uuid::Uuid;
use lazy_static::lazy_static;
use serde_json::Value;

lazy_static! {
    pub static ref CALLBACK_MANAGER: Mutex<CallbackManager<GenericDataObject>> = Mutex::new(CallbackManager::new());
}

/// Represents a single attribute with its value, type, and conversion parameters.
#[derive(Debug)]
pub struct AttributeValue {
    pub value: Box<dyn Any + Send + Sync>,
    pub value_type: ValueType,
    pub value_type_parameters: HashMap<String, String>,
}

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

pub trait Introspectable {
    fn attribute_names(&self) -> Vec<String>;
    fn attribute_type(&self, name: &str) -> Option<ValueType>;
    fn attribute_parameters(&self, name: &str) -> Option<HashMap<String, String>>;
    fn attribute_value_string(&self, name: &str) -> Option<String>;
}

/// The main generic data object structure
pub struct GenericDataObject {
    attributes: HashMap<String, AttributeValue>,
    extensions: HashMap<String, Value>,
    callbacks: CallbackManager<GenericDataObject>,
    pub identifier_name: String,
}

impl std::fmt::Debug for GenericDataObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericDataObject")
            .field("attributes", &self.attributes)
            .field("extensions", &self.extensions)
            // Skip callbacks as it cannot be trivially debug-formatted
            .field("identifier_name", &self.identifier_name)
            .finish()
    }
}

impl Introspectable for GenericDataObject {
    fn attribute_names(&self) -> Vec<String> {
        self.get_attribute_names()
    }

    fn attribute_type(&self, name: &str) -> Option<ValueType> {
        self.attributes.get(name).map(|v| v.value_type.clone())
    }

    fn attribute_parameters(&self, name: &str) -> Option<HashMap<String, String>> {
        self.attributes.get(name).map(|v| v.value_type_parameters.clone())
    }

    fn attribute_value_string(&self, name: &str) -> Option<String> {
        self.attributes.get(name).map(|v| v.to_string())
    }
}

impl GenericDataObject {
    /// Create a new GenericDataObject with a required unique identifier.
    pub fn new(identifier_name: &str, id: Option<Uuid>) -> Self {
        let mut object = Self {
            attributes: HashMap::new(),
            extensions: HashMap::new(),
            callbacks: CallbackManager::new(),
            identifier_name: identifier_name.to_string(),
        };

        let guid = id.unwrap_or_else(Uuid::new_v4);
        // Note: this set does not trigger callbacks as it is part of construction
        let value_type = TypeConverter::infer_value_type(&guid.to_string());
        object.attributes.insert(
            identifier_name.to_string(),
            AttributeValue::new(guid.to_string(), value_type)
        );

        object
    }

    pub fn register_callback(&mut self, event_type: EventType, callback: CallbackFn<GenericDataObject>) {
        self.callbacks.register(event_type, callback);
    }

    pub fn trigger_callbacks(&mut self, event_type: &str, attribute: Option<&str>, value: Option<&str>, error: Option<&str>) -> Result<(), OxDataError> {
        let params = CallbackParams {
            event_type: EventType::new(event_type),
            attribute: attribute.map(|s| s.to_string()),
            value: value.map(|s| s.to_string()),
            error: error.map(|s| s.to_string()),
        };

        // Two-level dispatch: per-object, then global
        let local_callbacks = std::mem::take(&mut self.callbacks);
        let res_local = local_callbacks.trigger(self, &params).map_err(|e| OxDataError::CallbackError(e.to_string()));
        self.callbacks = local_callbacks;
        res_local?;

        let global_manager = CALLBACK_MANAGER.lock().unwrap();
        // Trigger global callbacks
        global_manager.trigger(self, &params).map_err(|e| OxDataError::CallbackError(e.to_string()))
    }

    // --- Extension Slot ---
    pub fn get_extension(&self, key: &str) -> Option<&Value> {
        self.extensions.get(key)
    }

    pub fn set_extension(&mut self, key: &str, value: Value) {
        self.extensions.insert(key.to_string(), value);
    }

    pub fn remove_extension(&mut self, key: &str) -> Option<Value> {
        self.extensions.remove(key)
    }

    pub fn extension_keys(&self) -> Vec<String> {
        self.extensions.keys().cloned().collect()
    }

    pub fn get<T: Clone + 'static + Default>(&self, identifier: &str) -> Option<T>
    where
        T: Any + Clone,
    {
        self.get_value_internal(identifier)
    }

    fn get_value_internal<T: Clone + 'static + Default>(&self, identifier: &str) -> Option<T>
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

    pub fn get_mut<T: Clone + 'static + Default>(&mut self, identifier: &str) -> Result<Option<T>, OxDataError>
    where
        T: Any + Clone,
    {
        if let Err(e) = self.trigger_callbacks("before_get", Some(identifier), None, None) {
            let err_msg = e.to_string();
            let _ = self.trigger_callbacks("on_error_get", Some(identifier), None, Some(&err_msg));
            return Err(e);
        }

        let result = self.get_value_internal::<T>(identifier);

        if result.is_some() {
            let string_val = self.attributes.get(identifier).map(|a| a.to_string());
            let _ = self.trigger_callbacks("after_get", Some(identifier), string_val.as_deref(), None);
        } else {
            let _ = self.trigger_callbacks("on_error_get", Some(identifier), None, Some("Value not found or conversion failed"));
        }

        Ok(result)
    }

    pub fn set<T: Any + Send + Sync + Clone + 'static>(&mut self, identifier: &str, value: T) -> Result<Option<AttributeValue>, OxDataError> {
        let value_type = TypeConverter::infer_value_type(&value);
        self.set_with_type(identifier, value, value_type, None)
    }

    pub fn set_with_type<T: Any + Send + Sync + Clone + 'static>(
        &mut self, 
        identifier: &str, 
        value: T, 
        value_type: ValueType,
        parameters: Option<HashMap<String, String>>
    ) -> Result<Option<AttributeValue>, OxDataError> {
        let mut new_attr_value = AttributeValue::new(value, value_type);
        if let Some(params) = parameters {
            new_attr_value = new_attr_value.with_parameters(params);
        }

        let new_str = new_attr_value.to_string();
        if let Err(e) = self.trigger_callbacks("before_set", Some(identifier), Some(&new_str), None) {
            let err_msg = e.to_string();
            let _ = self.trigger_callbacks("on_error_set", Some(identifier), Some(&new_str), Some(&err_msg));
            return Err(e);
        }

        let result = self.attributes.insert(identifier.to_string(), new_attr_value);

        self.trigger_callbacks("after_set", Some(identifier), None, None)?;
        Ok(result)
    }


    // Introspection (no callbacks)
    pub fn get_attribute(&self, identifier: &str) -> Option<&AttributeValue> {
        self.attributes.get(identifier)
    }

    pub fn get_raw_value<T: Any + Clone>(&self, identifier: &str) -> Option<T> {
        self.attributes.get(identifier)?.get_value::<T>()
    }

    pub fn has_attribute(&self, identifier: &str) -> bool {
        self.attributes.contains_key(identifier)
    }

    pub fn remove_attribute(&mut self, identifier: &str) -> Result<Option<AttributeValue>, OxDataError> {
        if let Err(e) = self.trigger_callbacks("before_remove", Some(identifier), None, None) {
            let err_msg = e.to_string();
            let _ = self.trigger_callbacks("on_error_remove", Some(identifier), None, Some(&err_msg));
            return Err(e);
        }

        let result = self.attributes.remove(identifier);

        if result.is_some() {
            self.trigger_callbacks("after_remove", Some(identifier), None, None)?;
        } else {
            let _ = self.trigger_callbacks("on_error_remove", Some(identifier), None, Some("Attribute not found"));
        }

        Ok(result)
    }

    pub fn get_attribute_names(&self) -> Vec<String> {
        self.attributes.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.attributes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }
    
    pub fn to_serializable_map(&self) -> HashMap<String, (String, ValueType, HashMap<String, String>)> {
        let mut map: HashMap<String, (String, ValueType, HashMap<String, String>)> = self.attributes.iter().map(|(k, v)| {
            let coerced_value = TypeConverter::coerce_string(&v.to_string(), &v.value_type);
            (k.clone(), (coerced_value, v.value_type.clone(), v.value_type_parameters.clone()))
        }).collect();

        if !self.extensions.is_empty() {
            if let Ok(json_str) = serde_json::to_string(&self.extensions) {
                map.insert("__extensions__".to_string(), (json_str, ValueType::new("string"), HashMap::new()));
            }
        }
        
        map
    }

    pub fn set_attribute_value(&mut self, identifier: &str, attribute_value: AttributeValue) -> Result<Option<AttributeValue>, OxDataError> {
        let new_str = attribute_value.to_string();
        if let Err(e) = self.trigger_callbacks("before_set", Some(identifier), Some(&new_str), None) {
            let err_msg = e.to_string();
            let _ = self.trigger_callbacks("on_error_set", Some(identifier), Some(&new_str), Some(&err_msg));
            return Err(e);
        }

        let result = self.attributes.insert(identifier.to_string(), attribute_value);
        self.trigger_callbacks("after_set", Some(identifier), None, None)?;
        Ok(result)
    }

    pub fn clear_attributes(&mut self) -> Result<(), OxDataError> {
        if let Err(e) = self.trigger_callbacks("before_clear", None, None, None) {
            let err_msg = e.to_string();
            let _ = self.trigger_callbacks("on_error_clear", None, None, Some(&err_msg));
            return Err(e);
        }
        self.attributes.clear();
        self.trigger_callbacks("after_clear", None, None, None)?;
        Ok(())
    }

    pub fn set_attributes(&mut self, attributes: HashMap<String, AttributeValue>) -> Result<(), OxDataError> {
        if let Err(e) = self.trigger_callbacks("before_set", None, None, None) {
            let err_msg = e.to_string();
            let _ = self.trigger_callbacks("on_error_set", None, None, Some(&err_msg));
            return Err(e);
        }
        self.attributes = attributes;
        self.trigger_callbacks("after_set", None, None, None)?;
        Ok(())
    }

    pub fn from_serializable_map(
        serializable_map: HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        identifier_name: &str
    ) -> Result<Self, OxDataError> {
        let mut gdo = GenericDataObject {
            attributes: HashMap::new(),
            extensions: HashMap::new(),
            callbacks: CallbackManager::new(),
            identifier_name: identifier_name.to_string(),
        };
        let registry = CONVERSION_REGISTRY.lock().unwrap();

        for (key, (value_str, value_type, parameters)) in serializable_map {
            if key == "__extensions__" {
                if let Ok(ext_map) = serde_json::from_str::<HashMap<String, Value>>(&value_str) {
                    gdo.extensions = ext_map;
                }
                continue;
            }

            let converted_value = registry.convert_with_specific_converter(
                &value_type.as_str(),
                &value_type.as_str(),
                &value_str,
                &parameters,
            );

            let attr_value = match converted_value {
                Ok(converted_box_any) => AttributeValue {
                    value: converted_box_any,
                    value_type,
                    value_type_parameters: parameters,
                },
                Err(e) => return Err(OxDataError::ConversionError(e.to_string())),
            };
            gdo.attributes.insert(key, attr_value);
        }
        Ok(gdo)
    }
}

impl Default for GenericDataObject {
    fn default() -> Self {
        Self::new("id", None)
    }
}