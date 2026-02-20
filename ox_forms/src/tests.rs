#[cfg(test)]
mod tests {
    use crate::schema::{FormDefinition, FieldDefinition, ValidationRule};
    use crate::binding::{GenericDataObjectBinder, Binder};
    use crate::validation::{Validator, ValidationError};
    use ox_data_object::GenericDataObject;
    use serde_json::json;

    fn create_test_form() -> FormDefinition {
        FormDefinition {
            id: "test_form".to_string(),
            title: "Test Form".to_string(),
            fields: vec![
                FieldDefinition {
                    name: "name".to_string(),
                    label: "Name".to_string(),
                    data_type: "string".to_string(),
                    validation: vec![ValidationRule {
                        rule_type: "required".to_string(),
                        parameters: json!(null),
                        message: None,
                    }],
                    ..Default::default()
                },
                FieldDefinition {
                    name: "age".to_string(),
                    label: "Age".to_string(),
                    data_type: "integer".to_string(),
                    validation: vec![ValidationRule {
                        rule_type: "min".to_string(),
                        parameters: json!(18),
                        message: Some("Must be an adult".to_string()),
                    }],
                    ..Default::default()
                },
            ],
            layout: None,
            actions: vec![],
            data_source_binding: None,
            style: None,
            classes: None,
            styles: None,
            condition: None,
        }
    }

    #[test]
    fn test_binding_hydration() {
        let mut form = create_test_form();
        let mut gdo = GenericDataObject::new("id", None);
        gdo.set("name", "Alice".to_string());
        gdo.set("age", 30);

        let binder = GenericDataObjectBinder;
        binder.hydrate(&mut form, &gdo).unwrap();

        assert_eq!(form.fields[0].default_value, Some(json!("Alice")));
        assert_eq!(form.fields[1].default_value, Some(json!("30")));
    }

    #[test]
    fn test_binding_extraction() {
        let mut gdo = GenericDataObject::new("id", None);
        let mut data = serde_json::Map::new();
        data.insert("name".to_string(), json!("Bob"));
        data.insert("age".to_string(), json!(25));

        let binder = GenericDataObjectBinder;
        binder.extract(&mut gdo, &data).unwrap();

        assert_eq!(gdo.get::<String>("name").unwrap(), "Bob");
        // GDO stores age as string when set from Value::Number currently in my impl
        assert_eq!(gdo.get::<String>("age").unwrap(), "25");
    }

    #[test]
    fn test_validation_success() {
        let form = create_test_form();
        let mut gdo = GenericDataObject::new("id", None);
        gdo.set("name", "Charlie".to_string());
        gdo.set("age", 20);

        let validator = Validator;
        let errors = validator.validate(&form, &gdo);

        assert!(errors.is_empty());
    }

    #[test]
    fn test_validation_failure() {
        let form = create_test_form();
        let mut gdo = GenericDataObject::new("id", None);
        // name missing (required)
        gdo.set("age", 15); // too young (min 18)

        let validator = Validator;
        let errors = validator.validate(&form, &gdo);

        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.field == "name" && e.message.contains("required")));
        assert!(errors.iter().any(|e| e.field == "age" && e.message == "Must be an adult"));
    }

    #[test]
    fn test_form_generator() {
        use ox_data_object_manager::{DataObjectDefinition, DataObjectAttribute, AttributeMapping, AttributeValidation};
        use ox_type_converter::ValueType;
        use crate::FormGenerator;
        use std::collections::HashMap;

        // Mock Definition
        let mut params = HashMap::new();
        params.insert("min_len".to_string(), "5".to_string());

        let def = DataObjectDefinition {
            id: "User".to_string(),
            name: "User Profile".to_string(),
            description: None,
            attributes: vec![
                DataObjectAttribute {
                    name: "username".to_string(),
                    data_type: ValueType::String,
                    mapping: AttributeMapping::Direct { container_id: "c1".to_string(), field_name: "u".to_string() },
                    description: Some("Username".to_string()),
                    validation: Some(vec![
                        AttributeValidation {
                            rule_type: "min".to_string(),
                            parameters: params,
                            message: Some("Too short".to_string()),
                        }
                    ]),
                },
                DataObjectAttribute {
                    name: "age".to_string(),
                    data_type: ValueType::Integer,
                    mapping: AttributeMapping::Direct { container_id: "c1".to_string(), field_name: "a".to_string() },
                    description: None,
                    validation: None,
                },
            ],
            relationships: vec![],
        };

        // Generate
        let form = FormGenerator::from_dictionary_definition(&def);

        assert_eq!(form.fields.len(), 2);
        
        let username = form.fields.iter().find(|f| f.name == "username").unwrap();
        assert_eq!(username.component.as_deref(), Some("input_text"));
        assert_eq!(username.validation.len(), 1);
        assert_eq!(username.validation[0].rule_type, "min");

        let age = form.fields.iter().find(|f| f.name == "age").unwrap();
        assert_eq!(age.component.as_deref(), Some("input_number"));
        assert_eq!(age.data_type, "integer");
    }
}
