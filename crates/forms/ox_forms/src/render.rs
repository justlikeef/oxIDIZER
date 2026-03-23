use crate::registry::TypeRegistry;
use crate::schema::{FormDefinition, LayoutDefinition, LayoutItem, FieldDefinition};
use crate::traits::{RenderContext, FormRenderer};
use anyhow::{Result, Context};
use std::collections::HashMap;

pub struct FormEngine<'a> {
    registry: &'a TypeRegistry,
    forms: HashMap<String, &'a FormDefinition>,
    fields: HashMap<String, &'a FieldDefinition>,
    actions: HashMap<String, &'a crate::schema::ActionDefinition>,
}

impl<'a> FormEngine<'a> {
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self { 
            registry, 
            forms: HashMap::new(),
            fields: HashMap::new(),
            actions: HashMap::new(),
        }
    }

    pub fn with_module(mut self, module: &'a crate::schema::ModuleSchema) -> Self {
        for form in &module.forms {
            self.forms.insert(form.id.clone(), form);
            for field in &form.fields {
                self.fields.insert(field.name.clone(), field);
            }
            for action in &form.actions {
                self.actions.insert(action.name.clone(), action);
            }
        }
        self
    }

    pub fn render(&self, form: &FormDefinition, ctx: &RenderContext) -> Result<String> {
        // Build a local field map
        let mut local_fields = self.fields.clone();
        for field in &form.fields {
            local_fields.insert(field.name.clone(), field);
        }
        
        // Build local action map
        let mut local_actions = self.actions.clone();
        for action in &form.actions {
            local_actions.insert(action.name.clone(), action);
        }
        
        // Render content
        let content = if let Some(layout) = &form.layout {
            // If layout is present, it is responsible for EVERYTHING including actions.
            self.render_layout(layout, &local_fields, &local_actions, ctx)?
        } else {
            // Default behavior: Fields then Actions Footer
            let mut c = self.render_fields(&form.fields, ctx)?;
            c.push_str(&self.render_actions(&form.actions, ctx)?);
            c
        };

        // Get Renderer from registry
        let renderer = self.registry.get_form_renderer("html")
            .context("No form renderer named 'html' found in registry")?;
        
        renderer.render(form, &content)
    }

    pub fn render_form_content(&self, form: &FormDefinition, ctx: &RenderContext) -> Result<String> {
        let mut local_fields = self.fields.clone();
        for field in &form.fields {
            local_fields.insert(field.name.clone(), field);
        }
        let mut local_actions = self.actions.clone();
        for action in &form.actions {
            local_actions.insert(action.name.clone(), action);
        }

        if let Some(layout) = &form.layout {
             self.render_layout(layout, &local_fields, &local_actions, ctx)
        } else {
            let mut content = self.render_fields(&form.fields, ctx)?;
            content.push_str(&self.render_actions(&form.actions, ctx)?);
            Ok(content)
        }
    }

    pub fn render_actions(&self, actions: &[crate::schema::ActionDefinition], ctx: &RenderContext) -> Result<String> {
        if actions.is_empty() {
            return Ok(String::new());
        }
        let mut output = String::new();
        output.push_str("<div class=\"form-actions\">");
        for action in actions {
            output.push_str(&self.render_action(action, ctx)?);
        }
        output.push_str("</div>");
        Ok(output)
    }

    //Removed render_actions_filtered and collect_layout_actions as they are no longer needed.

    pub fn render_action(&self, action: &crate::schema::ActionDefinition, ctx: &RenderContext) -> Result<String> {
        let component_name = action.component.as_deref().unwrap_or("action-button");
        
        let renderer = self.registry.get_action_renderer(component_name)
            .context(format!("No action renderer found for component '{}'", component_name))?;

        renderer.render(action, ctx).map_err(|e| anyhow::anyhow!(e.to_string()))
    }

    fn render_layout(&self, layout: &LayoutDefinition, fields: &HashMap<String, &'a FieldDefinition>, actions: &HashMap<String, &'a crate::schema::ActionDefinition>, ctx: &RenderContext) -> Result<String> {
        let mut output = String::new();
        for item in &layout.items {
            output.push_str(&self.render_layout_item(item, fields, actions, ctx)?);
        }
        Ok(output)
    }

    fn render_layout_item(&self, item: &LayoutItem, fields: &HashMap<String, &'a FieldDefinition>, actions: &HashMap<String, &'a crate::schema::ActionDefinition>, ctx: &RenderContext) -> Result<String> {
        match item {
            LayoutItem::Row { items, classes } => {
                let inner = items.iter().map(|i| self.render_layout_item(i, fields, actions, ctx)).collect::<Result<Vec<_>>>()?.join("");
                let cls = classes.clone().unwrap_or_default();
                Ok(format!(r#"<div class="row {}">{}</div>"#, cls, inner))
            }
            LayoutItem::Column { items, width } => {
                let inner = items.iter().map(|i| self.render_layout_item(i, fields, actions, ctx)).collect::<Result<Vec<_>>>()?.join("");
                let w = width.unwrap_or(12);
                Ok(format!(r#"<div class="col-{}">{}</div>"#, w, inner))
            }
            LayoutItem::Field { name } => {
                // Try to find in form specific fields first (passed in arg), then global
                if let Some(field) = fields.get(name) {
                    self.render_field(field, ctx)
                } else if let Some(field) = self.fields.get(name) {
                     self.render_field(field, ctx)
                } else {
                    Ok(format!("<!-- Field {} not found -->", name))
                }
            }
            LayoutItem::Action { name } => {
                 if let Some(action) = actions.get(name) {
                     self.render_action(action, ctx)
                 } else {
                      Ok(format!("<!-- Action {} not found -->", name))
                 }
            }
            LayoutItem::HTML { content } => Ok(content.clone()),
            LayoutItem::Tabs { tabs: _ } => {
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

        let mut output = renderer.render(field, ctx)?;

        // Render subfields if any
        if let Some(subfields) = &field.subfields {
            output.push_str("<div class=\"form-subfields\">");
            output.push_str(&self.render_fields(subfields, ctx)?);
            output.push_str("</div>");
        }

        // Render subforms if any
        if let Some(subform_ids) = &field.subforms {
            for subform_id in subform_ids {
                if let Some(subform) = self.forms.get(subform_id) {
                    let content = self.render_form_content(subform, ctx)?;
                    let wrapped = if let Some(cond) = &subform.condition {
                        format!(r#"<div class="conditional-form" data-condition="{}">{}</div>"#, cond, content)
                    } else {
                        content
                    };
                    output.push_str(&wrapped);
                }
            }
        }

        // Apply conditional wrapper if condition exists
        if let Some(condition) = &field.condition {
            output = format!(r#"<div class="conditional-field" data-condition="{}">{}</div>"#, condition, output);
        }

        Ok(output)
    }
}
