use std::collections::HashMap;
use lazy_static::lazy_static;
use std::sync::Mutex;
use ox_data_object::GenericDataObject;
use crate::error::ValidationResult;
use crate::set::ValidationSet;

lazy_static! {
    static ref VALIDATION_REGISTRY: Mutex<HashMap<String, ValidationSet>> =
        Mutex::new(HashMap::new());
}

pub fn register_validation_set(set: ValidationSet) {
    VALIDATION_REGISTRY.lock().unwrap().insert(set.object_id.clone(), set);
}

pub fn unregister_validation_set(object_id: &str) {
    VALIDATION_REGISTRY.lock().unwrap().remove(object_id);
}

pub fn validate(object_id: &str, gdo: &GenericDataObject) -> ValidationResult {
    let registry = VALIDATION_REGISTRY.lock().unwrap();
    match registry.get(object_id) {
        Some(set) => set.validate(gdo),
        None => ValidationResult { errors: vec![] },
    }
}
