use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct OneOf {
    pub attribute: String,
    pub values: Vec<String>,
    pub message: Option<String>,
}

impl ValidationRule for OneOf {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if !self.values.contains(&val) {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be one of: {}", self.attribute, self.values.join(", "))
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Value must be one of a fixed set" }
    fn rule_type_name(&self) -> &'static str { "one_of" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "values": self.values })
    }
}

pub struct NotOneOf {
    pub attribute: String,
    pub values: Vec<String>,
    pub message: Option<String>,
}

impl ValidationRule for NotOneOf {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if self.values.contains(&val) {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must not be one of: {}", self.attribute, self.values.join(", "))
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Value must not be one of a fixed set" }
    fn rule_type_name(&self) -> &'static str { "not_one_of" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "values": self.values })
    }
}
