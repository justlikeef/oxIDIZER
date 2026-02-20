use crate::schema::{FormDefinition, FieldDefinition, ValidationRule};
use ox_data_object::GenericDataObject;

/// Represents a validation error for a specific field.
#[derive(Debug, serde::Serialize)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

/// Service for validating data objects against form definitions.
pub struct Validator;

impl Validator {
    /// Validates a GenericDataObject against a FormDefinition.
    pub fn validate(&self, form: &FormDefinition, obj: &GenericDataObject) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        for field in &form.fields {
            self.validate_field(field, obj, &mut errors);
        }
        errors
    }

    fn validate_field(&self, field: &FieldDefinition, obj: &GenericDataObject, errors: &mut Vec<ValidationError>) {
        let value_opt = obj.get_attribute(&field.name);
        
        for rule in &field.validation {
            if let Err(msg) = self.check_rule(rule, value_opt, field) {
                errors.push(ValidationError {
                    field: field.name.clone(),
                    message: msg,
                });
                // If a field fails one rule, we might want to skip others, but for now we report all.
            }
        }

        // Validate subfields
        if let Some(subfields) = &field.subfields {
            for subfield in subfields {
                self.validate_field(subfield, obj, errors);
            }
        }
    }

    fn check_rule(&self, rule: &ValidationRule, value_opt: Option<&ox_data_object::AttributeValue>, field: &FieldDefinition) -> Result<(), String> {
        match rule.rule_type.as_str() {
            "required" => {
                if value_opt.is_none() || value_opt.unwrap().to_string().trim().is_empty() {
                    return Err(rule.message.clone().unwrap_or_else(|| format!("{} is required", field.label)));
                }
            }
            "min" => {
                let min_val = rule.parameters.as_f64().ok_or("Invalid min parameter")?;
                if let Some(attr) = value_opt {
                    let val = attr.to_string().parse::<f64>().map_err(|_| "Not a number")?;
                    if val < min_val {
                        return Err(rule.message.clone().unwrap_or_else(|| format!("{} must be at least {}", field.label, min_val)));
                    }
                }
            }
            "max" => {
                let max_val = rule.parameters.as_f64().ok_or("Invalid max parameter")?;
                if let Some(attr) = value_opt {
                    let val = attr.to_string().parse::<f64>().map_err(|_| "Not a number")?;
                    if val > max_val {
                        return Err(rule.message.clone().unwrap_or_else(|| format!("{} must be at most {}", field.label, max_val)));
                    }
                }
            }
            "regex" => {
                let pattern = rule.parameters.as_str().ok_or("Invalid regex parameter")?;
                if let Some(attr) = value_opt {
                    let re = regex::Regex::new(pattern).map_err(|_| "Invalid regex pattern")?;
                    if !re.is_match(&attr.to_string()) {
                        return Err(rule.message.clone().unwrap_or_else(|| format!("{} has invalid format", field.label)));
                    }
                }
            }
            _ => {
                // Unknown rule, skip for now
            }
        }
        Ok(())
    }
}
