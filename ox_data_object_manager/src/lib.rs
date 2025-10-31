use ox_data_object::generic_data_object::{GenericDataObject, AttributeValue};
use ox_type_converter::{ValueType, TypeConverter, CONVERSION_REGISTRY};
use ox_callback_manager::{CallbackManager, CALLBACK_MANAGER, EventType, CallbackError, CallbackAction, CallbackResult, CallbackFn};
use std::collections::HashMap;
use std::any::Any;
use uuid::Uuid;

pub struct DataObjectManager {
    data_object: GenericDataObject,
}

impl DataObjectManager {
    pub fn new(identifier_name: &str, id: Option<Uuid>) -> Self {
        Self {
            data_object: GenericDataObject::new(identifier_name, id),
        }
    }

    pub fn set<T: Any + Send + Sync + Clone + 'static>(&mut self, identifier: &str, value: T) -> Result<Vec<String>, CallbackError> {
        let identifier_owned = identifier.to_string();
        let value_type = TypeConverter::infer_value_type(&value);

        // 1. Trigger BeforeSet callbacks
        let mut messages = match CALLBACK_MANAGER.lock().unwrap().trigger_callbacks(
            &EventType::new("BeforeSet"),
            &mut self.data_object,
            &[&identifier_owned, &value]
        ) {
            Err(e) => return Err(e),
            Ok(msgs) => msgs,
        };

        // 2. Perform the action, saving the old value for potential rollback
        let old_attr_value = self.data_object.set(identifier, value.clone());

        // 3. Trigger AfterSet callbacks
        match CALLBACK_MANAGER.lock().unwrap().trigger_callbacks(
            &EventType::new("AfterSet"),
            &mut self.data_object,
            &[&identifier_owned, &value]
        ) {
            Ok(mut after_messages) => {
                messages.append(&mut after_messages);
                Ok(messages)
            }
            Err(e) => {
                if e.action == CallbackAction::Rollback {
                    // Perform rollback
                    if let Some(prev_attr) = old_attr_value {
                        self.data_object.set_attribute_value(&identifier_owned, prev_attr);
                    } else {
                        // If there was no old value, it means a new attribute was added.
                        // So, remove the newly added attribute.
                        self.data_object.remove_attribute(&identifier_owned);
                    }
                }
                Err(e)
            }
        }
    }

    pub fn get<T: Clone + 'static + Default>(&mut self, identifier: &str) -> Result<Option<T>, CallbackError>
    where
        T: Any + Clone,
    {
        let identifier_owned = identifier.to_string();

        // 1. Trigger BeforeGet callbacks
        match CALLBACK_MANAGER.lock().unwrap().trigger_callbacks(
            &EventType::new("BeforeGet"),
            &mut self.data_object,
            &[&identifier_owned]
        ) {
            Err(e) => return Err(e),
            Ok(_) => { /* Continue */ }
        }

        // 2. Retrieve the value from the underlying GenericDataObject
        let mut current_value = self.data_object.get::<T>(identifier);

        // 3. Trigger AfterGet callbacks
        // Pass a mutable reference to the current_value so callbacks can modify it.
        // Note: This requires the CallbackFn to accept a mutable reference to the value.
        // For now, we'll pass the current_value and let the callback decide how to handle it.
        // If the callback modifies the GDO, we'll re-fetch the value after the callbacks.
        match CALLBACK_MANAGER.lock().unwrap().trigger_callbacks(
            &EventType::new("AfterGet"),
            &mut self.data_object,
            &[&identifier_owned, &current_value]
        ) {
            Err(e) => return Err(e),
            Ok(_) => { /* Continue */ }
        }

        // Re-fetch the value after AfterGet callbacks to ensure we return the most up-to-date value
        current_value = self.data_object.get::<T>(identifier);

        Ok(current_value)
    }

    pub fn register_callback(&mut self, event: EventType, callback: CallbackFn) {
        CALLBACK_MANAGER.lock().unwrap().register_callback(event, callback);
    }

    // Delegate other GenericDataObject methods as needed
    pub fn get_attribute(&self, identifier: &str) -> Option<&AttributeValue> {
        self.data_object.get_attribute(identifier)
    }

    pub fn has_attribute(&self, identifier: &str) -> bool {
        self.data_object.has_attribute(identifier)
    }

    pub fn remove_attribute(&mut self, identifier: &str) -> Option<AttributeValue> {
        self.data_object.remove_attribute(identifier)
    }

    pub fn get_attribute_names(&self) -> Vec<&String> {
        self.data_object.get_attribute_names()
    }

    pub fn len(&self) -> usize {
        self.data_object.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data_object.is_empty()
    }
}
