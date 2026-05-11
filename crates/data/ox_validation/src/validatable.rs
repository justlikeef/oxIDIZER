use ox_data_object::GenericDataObject;
use crate::registry;
use crate::error::ValidationResult;

pub trait Validatable {
    fn validate(&mut self, object_id: &str) -> ValidationResult;
}

impl Validatable for GenericDataObject {
    fn validate(&mut self, object_id: &str) -> ValidationResult {
        let _ = self.trigger_callbacks("before_validate", None, None, None);
        let result = registry::validate(object_id, self);
        if result.is_valid() {
            let _ = self.trigger_callbacks("after_validate", None, None, None);
        } else {
            let error_summary = result.errors.iter()
                .map(|e| format!("{}: {}", e.attribute, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            let _ = self.trigger_callbacks("on_error_validate", None, None, Some(&error_summary));
        }
        result
    }
}
