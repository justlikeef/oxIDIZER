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

use ox_validation::rules::{Min, Max, Range};

#[test]
fn min_passes() {
    let gdo = gdo_with("age", "25");
    let rule = Min { attribute: "age".to_string(), min: 18.0, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn min_fails() {
    let gdo = gdo_with("age", "15");
    let rule = Min { attribute: "age".to_string(), min: 18.0, message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "min");
}

#[test]
fn min_fails_non_numeric() {
    let gdo = gdo_with("age", "not-a-number");
    let rule = Min { attribute: "age".to_string(), min: 18.0, message: None };
    assert!(rule.validate(&gdo).is_err());
}

#[test]
fn max_passes() {
    let gdo = gdo_with("score", "95");
    let rule = Max { attribute: "score".to_string(), max: 100.0, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn max_fails() {
    let gdo = gdo_with("score", "110");
    let rule = Max { attribute: "score".to_string(), max: 100.0, message: None };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "max");
}

#[test]
fn range_passes() {
    let gdo = gdo_with("pct", "50");
    let rule = Range { attribute: "pct".to_string(), min: 0.0, max: 100.0, message: None };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn range_fails_below() {
    let gdo = gdo_with("pct", "-5");
    let rule = Range { attribute: "pct".to_string(), min: 0.0, max: 100.0, message: None };
    assert!(rule.validate(&gdo).is_err());
}

#[test]
fn range_fails_above() {
    let gdo = gdo_with("pct", "105");
    let rule = Range { attribute: "pct".to_string(), min: 0.0, max: 100.0, message: None };
    assert!(rule.validate(&gdo).is_err());
}

use ox_validation::rules::{Regex as RegexRule, Custom};
use std::sync::Arc;

#[test]
fn regex_passes() {
    let gdo = gdo_with("email", "user@example.com");
    let rule = RegexRule::new("email", r"^[^@]+@[^@]+\.[^@]+$", None).unwrap();
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn regex_fails() {
    let gdo = gdo_with("email", "not-an-email");
    let rule = RegexRule::new("email", r"^[^@]+@[^@]+\.[^@]+$", None).unwrap();
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "regex");
}

#[test]
fn regex_invalid_pattern_returns_err() {
    let result = RegexRule::new("f", r"[invalid", None);
    assert!(result.is_err());
}

#[test]
fn custom_passes() {
    let gdo = gdo_with("score", "42");
    let rule = Custom {
        attribute: "score".to_string(),
        description: "must be 42".to_string(),
        rule_fn: Arc::new(|gdo| {
            let v = gdo.get_attribute("score").map(|a| a.to_string()).unwrap_or_default();
            if v == "42" { Ok(()) } else { Err("not 42".to_string()) }
        }),
    };
    assert!(rule.validate(&gdo).is_ok());
}

#[test]
fn custom_fails() {
    let gdo = gdo_with("score", "99");
    let rule = Custom {
        attribute: "score".to_string(),
        description: "must be 42".to_string(),
        rule_fn: Arc::new(|gdo| {
            let v = gdo.get_attribute("score").map(|a| a.to_string()).unwrap_or_default();
            if v == "42" { Ok(()) } else { Err("not 42".to_string()) }
        }),
    };
    let err = rule.validate(&gdo).unwrap_err();
    assert_eq!(err.rule, "custom");
    assert!(err.message.contains("not 42"));
}

use ox_validation::ValidationSet;

#[test]
fn validation_set_collects_all_errors() {
    let mut set = ValidationSet::new("user");
    set.add_rule(Box::new(Required { attribute: "email".to_string(), message: None }))
       .add_rule(Box::new(Required { attribute: "name".to_string(), message: None }));
    let gdo = GenericDataObject::new("user", None);
    let result = set.validate(&gdo);
    assert!(!result.is_valid());
    assert_eq!(result.errors.len(), 2);
}

#[test]
fn validation_set_passes_when_all_rules_pass() {
    let mut set = ValidationSet::new("user");
    set.add_rule(Box::new(Required { attribute: "email".to_string(), message: None }));
    let gdo = gdo_with("email", "user@example.com");
    let result = set.validate(&gdo);
    assert!(result.is_valid());
}

#[test]
fn validation_set_object_id() {
    let set = ValidationSet::new("my_object");
    assert_eq!(set.object_id, "my_object");
}
