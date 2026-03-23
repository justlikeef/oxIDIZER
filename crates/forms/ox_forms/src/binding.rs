use crate::schema::{FormDefinition, FieldDefinition};
use ox_data_object::GenericDataObject;
use serde_json::Value;
use anyhow::Result;

/// Trait for binding data objects to forms.
pub trait Binder<T> {
    /// Populates the form fields from the data object.
    fn hydrate(&self, form: &mut FormDefinition, obj: &T) -> Result<()>;
    
    /// Extracts data from form values (e.g. from a POST request) into the data object.
    fn extract(&self, obj: &mut T, data: &serde_json::Map<String, Value>) -> Result<()>;
}

/// A binder implementation for GenericDataObject.
pub struct GenericDataObjectBinder;

impl Binder<GenericDataObject> for GenericDataObjectBinder {
    fn hydrate(&self, form: &mut FormDefinition, obj: &GenericDataObject) -> Result<()> {
        for field in &mut form.fields {
            self.hydrate_field(field, obj)?;
        }
        Ok(())
    }

    fn extract(&self, obj: &mut GenericDataObject, data: &serde_json::Map<String, Value>) -> Result<()> {
        for (key, value) in data {
            // GDO.set handles trait-based conversion if we pass it the right types.
            // But we have a Value. Let's use string-based coercion for now as GDO supports it.
            let val_str = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => String::new(),
                _ => value.to_string(),
            };
            
            obj.set(key, val_str);
        }
        Ok(())
    }
}

impl GenericDataObjectBinder {
    fn hydrate_field(&self, field: &mut FieldDefinition, obj: &GenericDataObject) -> Result<()> {
        // Try to get value by name
        if let Some(val) = obj.get_attribute(&field.name) {
            field.default_value = Some(serde_json::to_value(val.to_string())?);
        }

        // Recursively hydrate subfields
        if let Some(subfields) = &mut field.subfields {
            for subfield in subfields {
                self.hydrate_field(subfield, obj)?;
            }
        }

        Ok(())
    }

    /// Helper to create a GDO from a map of values.
    pub fn create_from_data(id_name: &str, data: &serde_json::Map<String, Value>) -> Result<GenericDataObject> {
        let mut obj = GenericDataObject::new(id_name, None);
        let binder = GenericDataObjectBinder;
        binder.extract(&mut obj, data)?;
        Ok(obj)
    }
}
