use ox_validation::error::{ValidationError, ValidationResult};
use ox_validation::rule::ValidationRule;
use ox_data_object::GenericDataObject;

struct AlwaysPass;

impl ValidationRule for AlwaysPass {
    fn attribute(&self) -> &str { "any" }
    fn validate(&self, _gdo: &GenericDataObject) -> Result<(), ValidationError> { Ok(()) }
    fn description(&self) -> &str { "Always passes" }
    fn rule_type_name(&self) -> &'static str { "always_pass" }
    fn constraint_json(&self) -> serde_json::Value { serde_json::Value::Null }
}

struct AlwaysFail { pub attr: String }

impl ValidationRule for AlwaysFail {
    fn attribute(&self) -> &str { &self.attr }
    fn validate(&self, _gdo: &GenericDataObject) -> Result<(), ValidationError> {
        Err(ValidationError {
            attribute: self.attr.clone(),
            rule: "always_fail".to_string(),
            message: "forced failure".to_string(),
        })
    }
    fn description(&self) -> &str { "Always fails" }
    fn rule_type_name(&self) -> &'static str { "always_fail" }
    fn constraint_json(&self) -> serde_json::Value { serde_json::Value::Null }
}

#[test]
fn validation_result_is_valid_when_empty() {
    let result = ValidationResult { errors: vec![] };
    assert!(result.is_valid());
}

#[test]
fn validation_result_invalid_when_has_errors() {
    let result = ValidationResult {
        errors: vec![ValidationError {
            attribute: "name".to_string(),
            rule: "required".to_string(),
            message: "required".to_string(),
        }],
    };
    assert!(!result.is_valid());
}

#[test]
fn validation_rule_trait_object_safe() {
    let rules: Vec<Box<dyn ValidationRule>> = vec![
        Box::new(AlwaysPass),
        Box::new(AlwaysFail { attr: "x".to_string() }),
    ];
    assert_eq!(rules[0].rule_type_name(), "always_pass");
    assert_eq!(rules[1].rule_type_name(), "always_fail");
}
