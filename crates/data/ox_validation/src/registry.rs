use ox_data_object::GenericDataObject;
use crate::error::ValidationResult;
use crate::set::ValidationSet;

pub fn register_validation_set(_set: ValidationSet) {}
pub fn unregister_validation_set(_object_id: &str) {}
pub fn validate(_object_id: &str, _gdo: &GenericDataObject) -> ValidationResult {
    ValidationResult { errors: vec![] }
}
