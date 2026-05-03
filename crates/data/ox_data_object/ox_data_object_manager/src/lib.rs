pub mod dictionary;
pub mod introspection;
pub mod query;

pub use dictionary::*;
pub use introspection::*;
pub use query::*;

use ox_data_object::GenericDataObject;
use std::collections::HashMap;
use ox_data_error::OxDataError;

pub struct DataObjectManager {
    pub dictionary: DataDictionary,
}

impl DataObjectManager {
    pub fn new() -> Self {
        DataObjectManager {
            dictionary: DataDictionary::new(),
        }
    }

    pub fn with_dictionary(dictionary: DataDictionary) -> Self {
        DataObjectManager {
            dictionary,
        }
    }

    pub fn create_data_object(&self, identifier_name: &str) -> GenericDataObject {
        GenericDataObject::new(identifier_name, None)
    }

    pub fn save_data_object(&self, _data_object: &GenericDataObject) -> Result<(), OxDataError> {
        // Placeholder for saving logic
        Ok(())
    }

    pub fn load_data_object(&self, identifier_name: &str, id: &str) -> Result<GenericDataObject, OxDataError> {
        let def = self.dictionary.objects.get(identifier_name)
            .ok_or_else(|| OxDataError::InternalError(format!("Object definition '{}' not found", identifier_name)))?;

        // 1. Find primary container via "id" attribute or first attribute
        let primary_mapping = def.attributes.iter()
            .find(|a| a.name == "id")
            .or_else(|| def.attributes.first())
            .ok_or_else(|| OxDataError::InternalError("No attributes defined in object".to_string()))?
            .mapping.clone();

        let (container_id, field_name) = match primary_mapping {
            AttributeMapping::Direct { container_id, field_name } => (container_id, field_name),
            _ => return Err(OxDataError::InternalError("Complex mapping not supported for primary key".to_string())),
        };

        let container = self.dictionary.containers.get(&container_id)
            .ok_or_else(|| OxDataError::InternalError(format!("Container '{}' not found", container_id)))?;

        let mut filters = HashMap::new();
        filters.insert(field_name, id.to_string());

        let root = QueryNode::Fetch {
            container_id: container.id.clone(),
            datasource_id: container.datasource_id.clone(),
            location: container.name.clone(),
            filters,
        };

        // TODO: Handle Joins if other attributes come from different containers.

        let plan = QueryPlan { root };
        let engine = QueryEngine::new();
        let results = engine.execute_plan(&plan)?;

        let row = results.first().ok_or_else(|| OxDataError::InternalError("Object not found".to_string()))?;

        // Hydrate GDO with remapping
        use ox_data_object::AttributeValue;
        use ox_type_converter::{ValueType, CONVERSION_REGISTRY};

        let mut gdo = GenericDataObject::new(identifier_name, None);
        let registry = CONVERSION_REGISTRY.lock().unwrap();

        for attr in &def.attributes {
            match &attr.mapping {
                 AttributeMapping::Direct { field_name, .. } => {
                     if let Some((val_str, val_type, params)) = row.get(field_name) {
                         let converted = registry.convert_with_specific_converter(
                             &val_type.as_str(),
                             &attr.data_type.as_str(),
                             val_str,
                             params
                         );
                         
                         let final_val = match converted {
                             Ok(v) => AttributeValue { value: v, value_type: attr.data_type.clone(), value_type_parameters: params.clone() },
                             Err(_) => {
                                 AttributeValue { value: Box::new(val_str.clone()), value_type: ValueType::String, value_type_parameters: params.clone() }
                             }
                         };
                         gdo.set_attribute_value(&attr.name, final_val)?;
                     }
                 }
                 _ => {}
            }
        }
        
        Ok(gdo)
    }
}

#[cfg(test)]
mod tests;