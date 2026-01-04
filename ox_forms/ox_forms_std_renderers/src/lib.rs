use ox_forms::registry::TypeRegistry;
use ox_forms::traits::{ElementRenderer, RenderContext};
use ox_forms::schema::FieldDefinition;
use anyhow::{Result, Error};
use std::sync::Arc;

pub struct TextInputRenderer;

impl ElementRenderer for TextInputRenderer {
    fn handled_data_types(&self) -> Vec<String> {
        vec!["string".to_string(), "text".to_string()]
    }

    fn render(&self, field: &FieldDefinition, _ctx: &RenderContext) -> Result<String, Error> {
        let label = &field.label;
        let name = &field.name;
        // Simple HTML rendering for now
        Ok(format!(
            r#"<div class="form-group">
                <label for="{name}">{label}</label>
                <input type="text" id="{name}" name="{name}" class="form-control" />
            </div>"#
        ))
    }
}

pub struct NumberInputRenderer;

impl ElementRenderer for NumberInputRenderer {
    fn handled_data_types(&self) -> Vec<String> {
        vec!["integer".to_string(), "float".to_string()]
    }

    fn render(&self, field: &FieldDefinition, _ctx: &RenderContext) -> Result<String, Error> {
        let label = &field.label;
        let name = &field.name;
        Ok(format!(
            r#"<div class="form-group">
                <label for="{name}">{label}</label>
                <input type="number" id="{name}" name="{name}" class="form-control" />
            </div>"#
        ))
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_forms_plugin_init(registry: *mut TypeRegistry) -> i32 {
    if registry.is_null() {
        return 1;
    }
    
    let registry = &mut *registry;
    
    // Register renderers
    registry.register_element_renderer("text-input", Arc::new(TextInputRenderer));
    registry.register_element_renderer("number-input", Arc::new(NumberInputRenderer));
    
    0
}
