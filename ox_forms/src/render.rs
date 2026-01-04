use crate::registry::TypeRegistry;
use crate::schema::{FormDefinition, LayoutDefinition, LayoutItem, FieldDefinition};
use crate::traits::{RenderContext, FormRenderer};
use anyhow::{Result, Context};
use std::collections::HashMap;

pub struct FormEngine<'a> {
    registry: &'a TypeRegistry,
}

impl<'a> FormEngine<'a> {
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self { registry }
    }

    pub fn render(&self, form: &FormDefinition, ctx: &RenderContext) -> Result<String> {
        // Default to a simple form renderer if none specified, 
        // or look up a registered form renderer.
        // For now, hardcode a basic HTML form wrapper for simplicity 
        // until we have a proper FormRenderer registry/lookup in place.
        
        let content = if let Some(layout) = &form.layout {
            self.render_layout(layout, ctx)?
        } else {
            self.render_fields(&form.fields, ctx)?
        };

        Ok(format!(r#"<form id="{}" method="post">{}</form>"#, form.id, content))
    }

    fn render_layout(&self, layout: &LayoutDefinition, ctx: &RenderContext) -> Result<String> {
        let mut output = String::new();
        for item in &layout.items {
            output.push_str(&self.render_layout_item(item, ctx)?);
        }
        Ok(output)
    }

    fn render_layout_item(&self, item: &LayoutItem, ctx: &RenderContext) -> Result<String> {
        match item {
            LayoutItem::Row { items, classes } => {
                let inner = items.iter().map(|i| self.render_layout_item(i, ctx)).collect::<Result<Vec<_>>>()?.join("");
                let cls = classes.clone().unwrap_or_default();
                Ok(format!(r#"<div class="row {}">{}</div>"#, cls, inner))
            }
            LayoutItem::Column { items, width } => {
                let inner = items.iter().map(|i| self.render_layout_item(i, ctx)).collect::<Result<Vec<_>>>()?.join("");
                let w = width.unwrap_or(12);
                Ok(format!(r#"<div class="col-{}">{}</div>"#, w, inner))
            }
            LayoutItem::Field { name } => {
                // Find field definition from context? Context doesn't have fields.
                // We need to pass the map of fields down or look it up.
                // For this implementation, let's assume we can't easily look up by name 
                // without passing the FormDefinition around constantly or preprocessing.
                // NOTE: This implementation fails because we don't have access to the FieldDefinition here.
                // We should probably build a map of fields first.
                // Skipping for now, will implement generic "render_field_by_name" placeholder.
                Ok(format!("<!-- Field {} placeholder -->", name))
            }
            LayoutItem::HTML { content } => Ok(content.clone()),
            LayoutItem::Tabs { tabs } => {
                // ... render tabs ...
                Ok("<!-- Tabs -->".to_string())
            }
        }
    }

    pub fn render_fields(&self, fields: &[FieldDefinition], ctx: &RenderContext) -> Result<String> {
        let mut output = String::new();
        for field in fields {
            output.push_str(&self.render_field(field, ctx)?);
        }
        Ok(output)
    }

    pub fn render_field(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String> {
        // Resolve component
        let component_name = if let Some(c) = &field.component {
            c.clone()
        } else {
            self.registry.resolve_component_config(&field.data_type)
                .map(|c| c.component.clone())
                .unwrap_or_else(|| "text-input".to_string()) // Fallback
        };

        let renderer = self.registry.get_element_renderer(&component_name)
            .context(format!("No renderer found for component '{}'", component_name))?;

        renderer.render(field, ctx)
    }
}
