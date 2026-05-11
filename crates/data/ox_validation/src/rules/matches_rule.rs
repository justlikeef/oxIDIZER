use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Matches {
    pub attribute: String,
    pub other_attribute: String,
    pub message: Option<String>,
}

impl ValidationRule for Matches {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        let other = gdo.get_attribute(&self.other_attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if val != other {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must match '{}'", self.attribute, self.other_attribute)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Value must equal another attribute's value" }
    fn rule_type_name(&self) -> &'static str { "matches" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "other_attribute": self.other_attribute })
    }
}
