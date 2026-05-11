use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct MinLength {
    pub attribute: String,
    pub min: usize,
    pub message: Option<String>,
}

impl ValidationRule for MinLength {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if val.chars().count() < self.min {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be at least {} characters", self.attribute, self.min)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Minimum character length" }
    fn rule_type_name(&self) -> &'static str { "min_length" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "min": self.min })
    }
}

pub struct MaxLength {
    pub attribute: String,
    pub max: usize,
    pub message: Option<String>,
}

impl ValidationRule for MaxLength {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if val.chars().count() > self.max {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be at most {} characters", self.attribute, self.max)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Maximum character length" }
    fn rule_type_name(&self) -> &'static str { "max_length" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "max": self.max })
    }
}
