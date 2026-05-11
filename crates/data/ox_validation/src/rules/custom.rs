use std::sync::Arc;
use ox_data_object::GenericDataObject;
pub struct Custom {
    pub attribute: String,
    pub description: String,
    pub rule_fn: Arc<dyn Fn(&GenericDataObject) -> Result<(), String> + Send + Sync>,
}
