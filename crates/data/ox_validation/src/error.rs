#[derive(Debug, Clone)]
pub struct ValidationError {
    pub attribute: String,
    pub rule: String,
    pub message: String,
}

#[derive(Debug)]
pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}
