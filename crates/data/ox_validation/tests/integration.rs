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

use ox_validation::rules::{Required, MinLength, MaxLength, OneOf, NotOneOf, Matches};

fn gdo_with(attr: &str, val: &str) -> GenericDataObject {
    let mut gdo = GenericDataObject::new("test", None);
    gdo.set(attr, val.to_string()).unwrap();
    gdo
}

#[test]
fn required_passes_when_present() {
    let gdo = gdo_with("name", "Alice");
    let rule = Required { attribute: "name".to_string(), message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn required_fails_when_absent() {
    let gdo = GenericDataObject::new("test", None);
    let rule = Required { attribute: "name".to_string(), message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "required");
    assert_eq!(err.attribute, "name");
}

#[test]
fn required_fails_when_empty_string() {
    let gdo = gdo_with("name", "");
    let rule = Required { attribute: "name".to_string(), message: None };
    assert!(rule.validate(&gdo).is_err());
}

#[test]
fn required_custom_message() {
    let gdo = GenericDataObject::new("test", None);
    let rule = Required { attribute: "email".to_string(), message: Some("Email is required".to_string()) };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.message, "Email is required");
}

#[test]
fn min_length_passes() {
    let gdo = gdo_with("pwd", "password123");
    let rule = MinLength { attribute: "pwd".to_string(), min: 8, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn min_length_fails() {
    let gdo = gdo_with("pwd", "short");
    let rule = MinLength { attribute: "pwd".to_string(), min: 8, message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "min_length");
}

#[test]
fn max_length_passes() {
    let gdo = gdo_with("code", "ABC");
    let rule = MaxLength { attribute: "code".to_string(), max: 5, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn max_length_fails() {
    let gdo = gdo_with("code", "TOOLONG");
    let rule = MaxLength { attribute: "code".to_string(), max: 5, message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "max_length");
}

#[test]
fn one_of_passes() {
    let gdo = gdo_with("status", "active");
    let rule = OneOf {
        attribute: "status".to_string(),
        values: vec!["active".to_string(), "inactive".to_string()],
        message: None,
    };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn one_of_fails() {
    let gdo = gdo_with("status", "unknown");
    let rule = OneOf {
        attribute: "status".to_string(),
        values: vec!["active".to_string(), "inactive".to_string()],
        message: None,
    };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "one_of");
}

#[test]
fn not_one_of_passes() {
    let gdo = gdo_with("word", "hello");
    let rule = NotOneOf {
        attribute: "word".to_string(),
        values: vec!["admin".to_string(), "root".to_string()],
        message: None,
    };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn not_one_of_fails() {
    let gdo = gdo_with("word", "admin");
    let rule = NotOneOf {
        attribute: "word".to_string(),
        values: vec!["admin".to_string(), "root".to_string()],
        message: None,
    };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "not_one_of");
}

#[test]
fn matches_passes() {
    let mut gdo = GenericDataObject::new("test", None);
    gdo.set("pwd", "secret".to_string()).unwrap();
    gdo.set("pwd_confirm", "secret".to_string()).unwrap();
    let rule = Matches {
        attribute: "pwd_confirm".to_string(),
        other_attribute: "pwd".to_string(),
        message: None,
    };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn matches_fails() {
    let mut gdo = GenericDataObject::new("test", None);
    gdo.set("pwd", "secret".to_string()).unwrap();
    gdo.set("pwd_confirm", "different".to_string()).unwrap();
    let rule = Matches {
        attribute: "pwd_confirm".to_string(),
        other_attribute: "pwd".to_string(),
        message: None,
    };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "matches");
}
