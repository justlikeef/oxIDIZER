use ox_data_object::GenericDataObject;
use crate::error::ValidationError;
use crate::rule::ValidationRule;

pub struct Min {
    pub attribute: String,
    pub min: f64,
    pub message: Option<String>,
}

impl ValidationRule for Min {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let raw = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        let val: f64 = raw.parse().map_err(|_| ValidationError {
            attribute: self.attribute.clone(),
            rule: self.rule_type_name().to_string(),
            message: format!("'{}' must be a number", self.attribute),
        })?;
        if val < self.min {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be at least {}", self.attribute, self.min)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Minimum numeric value" }
    fn rule_type_name(&self) -> &'static str { "min" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "min": self.min })
    }
}

pub struct Max {
    pub attribute: String,
    pub max: f64,
    pub message: Option<String>,
}

impl ValidationRule for Max {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let raw = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        let val: f64 = raw.parse().map_err(|_| ValidationError {
            attribute: self.attribute.clone(),
            rule: self.rule_type_name().to_string(),
            message: format!("'{}' must be a number", self.attribute),
        })?;
        if val > self.max {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be at most {}", self.attribute, self.max)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Maximum numeric value" }
    fn rule_type_name(&self) -> &'static str { "max" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "max": self.max })
    }
}

pub struct Range {
    pub attribute: String,
    pub min: f64,
    pub max: f64,
    pub message: Option<String>,
}

impl ValidationRule for Range {
    fn attribute(&self) -> &str { &self.attribute }

    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError> {
        let raw = gdo.get_attribute(&self.attribute)
            .map(|a| a.to_string())
            .unwrap_or_default();
        let val: f64 = raw.parse().map_err(|_| ValidationError {
            attribute: self.attribute.clone(),
            rule: self.rule_type_name().to_string(),
            message: format!("'{}' must be a number", self.attribute),
        })?;
        if val < self.min || val > self.max {
            return Err(ValidationError {
                attribute: self.attribute.clone(),
                rule: self.rule_type_name().to_string(),
                message: self.message.clone().unwrap_or_else(|| {
                    format!("'{}' must be between {} and {}", self.attribute, self.min, self.max)
                }),
            });
        }
        Ok(())
    }

    fn description(&self) -> &str { "Numeric value within a range" }
    fn rule_type_name(&self) -> &'static str { "range" }
    fn constraint_json(&self) -> serde_json::Value {
        serde_json::json!({ "min": self.min, "max": self.max })
    }
}
