use ox_data_object::GenericDataObject;
use crate::error::ValidationError;

pub trait ValidationRule: Send + Sync {
    fn attribute(&self) -> &str;
    fn validate(&self, gdo: &GenericDataObject) -> Result<(), ValidationError>;
    fn description(&self) -> &str;
    fn rule_type_name(&self) -> &'static str;
    fn constraint_json(&self) -> serde_json::Value;
}
