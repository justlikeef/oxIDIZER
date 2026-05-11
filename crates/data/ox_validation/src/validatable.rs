pub trait Validatable {
    fn validate(&mut self, object_id: &str) -> crate::error::ValidationResult;
}
