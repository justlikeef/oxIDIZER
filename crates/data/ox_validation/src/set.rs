use ox_data_object::GenericDataObject;
use crate::error::ValidationResult;
use crate::rule::ValidationRule;

pub struct ValidationSet {
    pub object_id: String,
    pub rules: Vec<Box<dyn ValidationRule>>,
}

impl ValidationSet {
    pub fn new(object_id: &str) -> Self {
        Self { object_id: object_id.to_string(), rules: vec![] }
    }
    pub fn add_rule(&mut self, rule: Box<dyn ValidationRule>) -> &mut Self {
        self.rules.push(rule);
        self
    }
    pub fn validate(&self, _gdo: &GenericDataObject) -> ValidationResult {
        ValidationResult { errors: vec![] }
    }
}
