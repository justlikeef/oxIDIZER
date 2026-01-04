use crate::schema::{FieldDefinition, FormDefinition, LayoutDefinition};
use serde_json::Value;
use anyhow::Error;
use std::collections::HashMap;

/// Context passed to renderers
pub struct RenderContext<'a> {
    pub props: &'a HashMap<String, Value>,
    // Potentially other context like theme, user info, etc.
}

pub trait ElementRenderer: Send + Sync {
    /// Returns list of data types this renderer handles (e.g., ["string", "email"])
    fn handled_data_types(&self) -> Vec<String>;
    
    /// Render the field to a string (HTML)
    fn render(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error>;
}

pub trait FormRenderer: Send + Sync {
    /// Render the overall form container
    fn render(&self, form: &FormDefinition, content: &str) -> Result<String, Error>;
    
    /// Render the layout structure, recursively calling render_layout or delegating field rendering
    fn render_layout(&self, layout: &LayoutDefinition, ctx: &RenderContext) -> Result<String, Error>;
}

pub trait LifecycleHook: Send + Sync {
    fn on_init_element(&self, field: &mut FieldDefinition) -> Result<(), Error>;
    fn on_bind_value(&self, field: &FieldDefinition, value: &mut Value) -> Result<(), Error>;
    fn on_validate(&self, field: &FieldDefinition, value: &Value) -> Result<(), Error>;
}

/// Interface for Client-Side Reactivity
pub trait ClientEventHandler: Send + Sync {
    /// SSR: Returns vanilla JS to attach to the element (e.g., inline onclick)
    fn client_script(&self, field: &FieldDefinition) -> Option<String>;
    
    /// WASM: List of DOM events to listen to for this field
    fn subscribe_events(&self) -> Vec<String>;
    
    // Note: WASM handling logic would likely be in a separate crate/trait 
    // to avoid web-sys dependencies in the core crate if possible, 
    // or this crate needs "wasm" feature.
}

/// Data Source Interface
pub trait DataSource: Send + Sync {
    fn get_value(&self, field_name: &str) -> Option<Value>;
    fn set_value(&mut self, field_name: &str, value: Value) -> Result<(), Error>;
    
    fn get_options(&self, field_name: &str, params: &HashMap<String, Value>) -> Result<Vec<OptionItem>, Error>;
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct OptionItem {
    pub label: String,
    pub value: Value,
}
