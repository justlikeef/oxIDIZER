use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TypeMappingConfig {
    pub mappings: HashMap<String, DefaultFieldConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DefaultFieldConfig {
    pub component: String, 
    #[serde(default)]
    pub default_props: Value,
}

use crate::traits::{ElementRenderer, FormRenderer};
use std::sync::Arc;

pub struct TypeRegistry {
    /// Maps data types (e.g. "email") to default configs (e.g. component="email-input")
    mapping: HashMap<String, DefaultFieldConfig>,
    /// Maps component names (e.g. "email-input") to actual renderers
    element_renderers: HashMap<String, Arc<dyn ElementRenderer>>,
    /// Maps form renderer names to implementations
    form_renderers: HashMap<String, Arc<dyn FormRenderer>>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self {
            mapping: HashMap::new(),
            element_renderers: HashMap::new(),
            form_renderers: HashMap::new(),
        }
    }

    pub fn load_from_config(&mut self, config: TypeMappingConfig) {
        self.mapping.extend(config.mappings);
    }

    pub fn resolve_component_config(&self, data_type: &str) -> Option<&DefaultFieldConfig> {
        self.mapping.get(data_type)
    }

    pub fn register_element_renderer(&mut self, name: &str, renderer: Arc<dyn ElementRenderer>) {
        self.element_renderers.insert(name.to_string(), renderer);
    }

    pub fn get_element_renderer(&self, component_name: &str) -> Option<Arc<dyn ElementRenderer>> {
        self.element_renderers.get(component_name).cloned()
    }
}
