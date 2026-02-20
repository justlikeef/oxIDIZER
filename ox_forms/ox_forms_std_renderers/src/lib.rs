use ox_forms::{
    ActionRenderer, ElementRenderer, FormRenderer, RenderContext,
    schema::FieldDefinition, schema::ActionDefinition, schema::FormDefinition, schema::LayoutDefinition,
    registry::TypeRegistry
};
use anyhow::{Result, Error};
use std::sync::Arc;
use serde_json::Value;

// Helper struct to hold common render information
struct FieldRenderInfo {
    class_attr: String,
    styles: String,
    name: String,
    value: String,
    placeholder_attr: String,
}

// Helper to extract common attributes and current value
fn get_field_info(field: &FieldDefinition, ctx: &RenderContext) -> FieldRenderInfo {
    let classes = field.classes.as_deref().unwrap_or("form-control");
    let styles = field.styles.as_deref().map(|s| format!(" style=\"{}\"", s)).unwrap_or_default();
    let class_attr = format!(" class=\"{}\"", classes);
    
    // 1. Check ctx.props
    // 2. Check field.default_value
    let value = ctx.props.get(&field.name)
        .or(field.default_value.as_ref())
        .map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => v.to_string(),
        })
        .unwrap_or_default();

    let placeholder_attr = field.props.get("placeholder")
        .and_then(|v| v.as_str())
        .map(|s| format!(" placeholder=\"{}\"", s))
        .unwrap_or_default();

    FieldRenderInfo {
        class_attr,
        styles,
        name: field.name.clone(),
        value,
        placeholder_attr,
    }
}

pub struct TextInputRenderer;
impl ElementRenderer for TextInputRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["string".to_string(), "text".to_string()] }
    fn render(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error> {
        let info = get_field_info(field, ctx);
        Ok(format!(r#"<div class="form-group"><label for="{0}">{1}</label><input type="text" id="{0}" name="{0}" value="{4}"{2}{3}{5} /></div>"#, 
            info.name, field.label, info.class_attr, info.styles, info.value, info.placeholder_attr))
    }
}

pub struct PasswordRenderer;
impl ElementRenderer for PasswordRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["password".to_string()] }
    fn render(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error> {
        let info = get_field_info(field, ctx);
        Ok(format!(r#"<div class="form-group"><label for="{0}">{1}</label><input type="password" id="{0}" name="{0}" value="{4}"{2}{3}{5} /></div>"#, 
            info.name, field.label, info.class_attr, info.styles, info.value, info.placeholder_attr))
    }
}

pub struct NumberInputRenderer;
impl ElementRenderer for NumberInputRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["integer".to_string(), "float".to_string(), "number".to_string()] }
    fn render(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error> {
        let info = get_field_info(field, ctx);
        Ok(format!(r#"<div class="form-group"><label for="{0}">{1}</label><input type="number" id="{0}" name="{0}" value="{4}"{2}{3}{5} /></div>"#, 
            info.name, field.label, info.class_attr, info.styles, info.value, info.placeholder_attr))
    }
}

pub struct TextAreaRenderer;
impl ElementRenderer for TextAreaRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["longtext".to_string(), "textarea".to_string()] }
    fn render(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error> {
        let info = get_field_info(field, ctx);
        Ok(format!(r#"<div class="form-group"><label for="{0}">{1}</label><textarea id="{0}" name="{0}"{2}{3}{5}>{4}</textarea></div>"#, 
            info.name, field.label, info.class_attr, info.styles, info.value, info.placeholder_attr))
    }
}

pub struct CheckboxRenderer;
impl ElementRenderer for CheckboxRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["boolean".to_string(), "checkbox".to_string()] }
    fn render(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error> {
        let classes = field.classes.as_deref().unwrap_or("form-check-input");
        let styles = field.styles.as_deref().map(|s| format!(" style=\"{}\"", s)).unwrap_or_default();
        let class_attr = format!(" class=\"{}\"", classes);
        let name = field.name.clone();

        let is_checked = ctx.props.get(&name)
            .or(field.default_value.as_ref())
            .map(|v| v.as_bool().unwrap_or(false))
            .unwrap_or(false);
        
        let checked_attr = if is_checked { " checked" } else { "" };

        Ok(format!(r#"<div class="form-check"><input type="checkbox" id="{0}" name="{0}"{2}{3}{4} /><label class="form-check-label" for="{0}">{1}</label></div>"#, 
            name, field.label, class_attr, styles, checked_attr))
    }
}

pub struct DateRenderer;
impl ElementRenderer for DateRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["date".to_string(), "datetime".to_string()] }
    fn render(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error> {
        let info = get_field_info(field, ctx);
        Ok(format!(r#"<div class="form-group"><label for="{0}">{1}</label><input type="date" id="{0}" name="{0}" value="{4}"{2}{3}{5} /></div>"#, 
            info.name, field.label, info.class_attr, info.styles, info.value, info.placeholder_attr))
    }
}

pub struct SelectRenderer;
impl ElementRenderer for SelectRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["select".to_string(), "enum".to_string()] }
    fn render(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error> {
        let info = get_field_info(field, ctx);
        let mut options_html = String::new();
        
        let empty_vec = vec![];
        let options = field.props.get("options")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_vec);

        for opt in options {
            let (val, label) = if let Some(s) = opt.as_str() {
                (s.to_string(), s.to_string())
            } else if let Some(obj) = opt.as_object() {
                let val = obj.get("value").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_default();
                let label = obj.get("label").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or(val.clone());
                (val, label)
            } else {
                continue;
            };

            let selected_attr = if val == info.value { " selected" } else { "" };
            options_html.push_str(&format!("<option value=\"{}\"{}>{}</option>", val, selected_attr, label));
        }
        
        Ok(format!(r#"<div class="form-group"><label for="{0}">{1}</label><select id="{0}" name="{0}"{2}{3}>{4}</select></div>"#, 
            info.name, field.label, info.class_attr, info.styles, options_html))
    }
}

pub struct RadioButtonRenderer;
impl ElementRenderer for RadioButtonRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["radio".to_string()] }
    fn render(&self, field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error> {
        let info = get_field_info(field, ctx);
        let mut options_html = String::new();
        
        let empty_vec = vec![];
        let options = field.props.get("options")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_vec);

        options_html.push_str("<div class=\"radio-group\">");
        for (idx, opt) in options.iter().enumerate() {
            let (val, label) = if let Some(s) = opt.as_str() {
                (s.to_string(), s.to_string())
            } else if let Some(obj) = opt.as_object() {
                let val = obj.get("value").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_default();
                let label = obj.get("label").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or(val.clone());
                (val, label)
            } else {
                continue;
            };

            let checked_attr = if val == info.value { " checked" } else { "" };
            let option_id = format!("{}_{}", info.name, idx);
            
            options_html.push_str(&format!(
                r#"<div class="form-check form-check-inline">
                    <input class="form-check-input" type="radio" name="{0}" id="{2}" value="{1}"{3}>
                    <label class="form-check-label" for="{2}">{4}</label>
                </div>"#, 
                info.name, val, option_id, checked_attr, label
            ));
        }
        options_html.push_str("</div>");
        
        Ok(format!(r#"<div class="form-group"><label>{0}</label>{1}</div>"#, 
            field.label, options_html))
    }
}

pub struct HiddenRenderer;
impl ElementRenderer for HiddenRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["hidden".to_string()] }
    fn render(&self, _field: &FieldDefinition, ctx: &RenderContext) -> Result<String, Error> {
        let field = _field;
        let info = get_field_info(field, ctx);
        Ok(format!(r#"<input type="hidden" id="{0}" name="{0}" value="{1}" />"#, 
            info.name, info.value))
    }
}

pub struct ContainerRenderer;
impl ElementRenderer for ContainerRenderer {
    fn handled_data_types(&self) -> Vec<String> { vec!["container".to_string()] }
    fn render(&self, _field: &FieldDefinition, _ctx: &RenderContext) -> Result<String, Error> {
        Ok(String::new())
    }
}

pub struct ActionButtonRenderer;

impl ActionRenderer for ActionButtonRenderer {
    fn render(&self, action: &ActionDefinition, _ctx: &RenderContext) -> Result<String, Error> {
        let btn_type = match action.action_type.as_str() {
            "submit" => "submit",
            "reset" => "reset",
            _ => "button",
        };
        
        // Use props for extra classes or styles
        let mut classes = "btn".to_string();
        if action.action_type == "submit" {
            classes.push_str(" btn-primary");
        } else {
            classes.push_str(" btn-secondary");
        }
        
        if let Some(extra_classes) = action.props.get("classes").and_then(|v| v.as_str()) {
            classes.push_str(" ");
            classes.push_str(extra_classes);
        }

        let style_attr = action.props.get("styles").and_then(|v| v.as_str())
            .map(|s| format!(" style=\"{}\"", s))
            .unwrap_or_default();

        Ok(format!(
            r#"<button type="{}" name="{}" class="{}"{}>{}</button>"#,
            btn_type, action.name, classes, style_attr, action.label
        ))
    }
}

pub struct HtmlFormRenderer;

impl FormRenderer for HtmlFormRenderer {
    fn render(&self, form: &FormDefinition, content: &str) -> Result<String, Error> {
        let wasm_script = format!(r#"
<script type="module">
    import init, {{ attach_listeners }} from '/ox_forms_client/ox_forms_client.js';
    async function run() {{
        await init();
        try {{
            attach_listeners('{}');
        }} catch (e) {{
            console.error("Failed to attach listeners for form {}", e);
        }}
    }}
    run();
</script>
"#, form.id, form.id);

        let mut classes = form.classes.clone().unwrap_or_default();
        if let Some(legacy_style) = &form.style {
            if !classes.contains(legacy_style) {
                if !classes.is_empty() { classes.push(' '); }
                classes.push_str(legacy_style);
            }
        }
        
        let class_attr = if !classes.is_empty() { format!(" class=\"{}\"", classes) } else { String::new() };
        let style_attr = form.styles.as_ref().map(|s| format!(" style=\"{}\"", s)).unwrap_or_default();

        Ok(format!(r#"<form id="{}" method="post"{}{}>{}{}</form>"#, form.id, class_attr, style_attr, content, wasm_script))
    }

    fn render_layout(&self, _layout: &LayoutDefinition, _ctx: &RenderContext) -> Result<String, Error> {
        Ok(String::new())
    }
}

pub fn register_standard_renderers(registry: &mut TypeRegistry) {
    registry.register_element_renderer("text-input", Arc::new(TextInputRenderer));
    registry.register_element_renderer("password-input", Arc::new(PasswordRenderer));
    registry.register_element_renderer("number-input", Arc::new(NumberInputRenderer));
    registry.register_element_renderer("textarea", Arc::new(TextAreaRenderer));
    registry.register_element_renderer("checkbox", Arc::new(CheckboxRenderer));
    registry.register_element_renderer("date-input", Arc::new(DateRenderer));
    registry.register_element_renderer("select-input", Arc::new(SelectRenderer));
    registry.register_element_renderer("radio", Arc::new(RadioButtonRenderer));
    registry.register_element_renderer("hidden", Arc::new(HiddenRenderer));
    registry.register_element_renderer("container", Arc::new(ContainerRenderer));
    
    registry.register_action_renderer("action-button", Arc::new(ActionButtonRenderer));
    registry.register_form_renderer("html", Arc::new(HtmlFormRenderer));
}

#[no_mangle]
pub unsafe extern "C" fn ox_forms_plugin_init(registry: *mut TypeRegistry) -> i32 {
    if registry.is_null() { return 1; }
    let registry = &mut *registry;
    
    register_standard_renderers(registry);
    
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_hidden_renderer_no_wrapper() {
        let renderer = HiddenRenderer;
        let field = FieldDefinition {
            name: "test_hidden".to_string(),
            label: "Test Hidden".to_string(),
            data_type: "hidden".to_string(),
            component: Some("hidden".to_string()),
            plugins: vec![],
            validation: vec![],
            dependencies: vec![],
            props: serde_json::Value::Null,
            classes: None,
            styles: None,
            default_value: None,
            condition: None,
            subfields: None,
            subforms: None,
        };
        let props = HashMap::new();
        let ctx = RenderContext { props: &props };
        
        let result = renderer.render(&field, &ctx).unwrap();
        
        // Assert it's just the input tag
        assert_eq!(result, r#"<input type="hidden" id="test_hidden" name="test_hidden" value="" />"#);
        assert!(!result.contains("form-group"));
        assert!(!result.contains("<label"));
    }

    #[test]
    fn test_text_renderer_has_wrapper() {
        let renderer = TextInputRenderer;
        let field = FieldDefinition {
            name: "test_text".to_string(),
            label: "Test Text".to_string(),
            data_type: "string".to_string(),
            component: Some("text-input".to_string()),
            plugins: vec![],
            validation: vec![],
            dependencies: vec![],
            props: serde_json::Value::Null,
            classes: None,
            styles: None,
            default_value: None,
            condition: None,
            subfields: None,
            subforms: None,
        };
        let props = HashMap::new();
        let ctx = RenderContext { props: &props };
        
        let result = renderer.render(&field, &ctx).unwrap();
        
        // Assert it HAS the wrapper
        assert!(result.contains("form-group"));
        assert!(result.contains("<label"));
        assert!(result.contains("<input type=\"text\""));
    }
}
