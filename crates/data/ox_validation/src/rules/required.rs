use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Required {
    pub attribute: String,
    pub message: Option<String>,
}

impl ValidationRule for Required {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if val.is_empty() {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' is required", self.attribute)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Attribute must be present and non-empty" }
    fn rule_type_name(&self) -> &'static str { "required" }
    fn constraint_json(&self) -> serde_json::Value { serde_json::Value::Null }
}
