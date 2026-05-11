use std::sync::Arc;
use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Custom {
    pub attribute: String,
    pub description: String,
    pub rule_fn: Arc<dyn Fn(&GenericDataObject) -> Result<(), String> + Send + Sync>,
}

impl ValidationRule for Custom {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        (self.rule_fn)(gdo).map_err(|msg| ValidationError {
            attribute: self.attribute.clone(),
            rule: self.rule_type_name().to_string(),
            message: msg,
        })
    }

    fn description(&self) -> &str { &self.description }
    fn rule_type_name(&self) -> &'static str { "custom" }
    fn constraint_json(&self) -> serde_json::Value { serde_json::Value::Null }
}
