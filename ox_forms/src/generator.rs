use crate::schema::*;
use ox_data_object_manager::{DataObjectDefinition, DataObjectAttribute, AttributeValidation};
use ox_type_converter::ValueType;
use serde_json::Value;

pub struct FormGenerator;

impl FormGenerator {
    pub fn from_dictionary_definition(def: &DataObjectDefinition) -> FormDefinition {
        let mut fields = Vec::new();

        for attr in &def.attributes {
            fields.push(Self::generate_field(attr));
        }

        FormDefinition {
            id: format!("form_{}", def.id),
            title: def.name.clone(),
            fields,
            layout: None, // Can implement auto-layout later
            actions: vec![
                ActionDefinition {
                    name: "submit".to_string(),
                    label: "Save".to_string(),
                    action_type: "submit".to_string(),
                    ..Default::default()
                }
            ],
            data_source_binding: Some(def.id.clone()),
            ..Default::default()
        }
    }

    fn generate_field(attr: &DataObjectAttribute) -> FieldDefinition {
        let (data_type, component) = match &attr.data_type {
            ValueType::String => ("string".to_string(), Some("input_text".to_string())),
            ValueType::Integer => ("integer".to_string(), Some("input_number".to_string())),
            ValueType::Float => ("float".to_string(), Some("input_number".to_string())),
            ValueType::Boolean => ("boolean".to_string(), Some("checkbox".to_string())),
            // Fallback
            _ => ("string".to_string(), Some("input_text".to_string())),
        };

        let mut validation_rules = Vec::new();
        if let Some(validations) = &attr.validation {
            for v in validations {
                validation_rules.push(ValidationRule {
                    rule_type: v.rule_type.clone(),
                    parameters: serde_json::to_value(&v.parameters).unwrap_or(Value::Null),
                    message: v.message.clone(),
                });
            }
        }

        FieldDefinition {
            name: attr.name.clone(),
            label: attr.description.clone().unwrap_or_else(|| attr.name.clone()),
            data_type,
            component,
            validation: validation_rules,
            ..Default::default()
        }
    }
}
