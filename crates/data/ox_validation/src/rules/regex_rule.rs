use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Regex {
    attribute: String,
    pattern: String,
    compiled: regex::Regex,
    message: Option<String>,
}

impl Regex {
    pub fn new(attribute: &str, pattern: &str, message: Option<String>) -> Result<Self, String> {
        let compiled = regex::Regex::new(pattern)
            .map_err(|e| format!("invalid regex pattern: {}", e))?;
        Ok(Self {
            attribute: attribute.to_string(),
            pattern: pattern.to_string(),
            compiled,
            message,
        })
    }
}

impl ValidationRule for Regex {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let val = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        if !self.compiled.is_match(&val) {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' does not match required pattern", self.attribute)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Value must match a regular expression" }
    fn rule_type_name(&self) -> &'static str { "regex" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "pattern": self.pattern })
    }
}
