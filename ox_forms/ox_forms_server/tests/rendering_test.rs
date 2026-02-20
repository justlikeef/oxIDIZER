use ox_forms::{
    registry::TypeRegistry,
    render::FormEngine,
    schema::{FormDefinition, FieldDefinition},
    traits::RenderContext,
};
use std::collections::HashMap;

#[test]
fn test_render_hardcoded_form() {
    let mut registry = TypeRegistry::new();
    ox_forms_std_renderers::register_standard_renderers(&mut registry);
    let engine = FormEngine::new(&registry);
    
    // This test verifies FormEngine independently of server loading logic
    let form = FormDefinition {
        id: "server_test_form".to_string(),
        title: "Auto-Generated Form".to_string(),
        fields: vec![
            FieldDefinition {
                name: "full_name".to_string(),
                label: "Full Name".to_string(),
                data_type: "string".to_string(),
                ..Default::default()
            },
            FieldDefinition {
                name: "quantity".to_string(),
                label: "Quantity".to_string(),
                data_type: "integer".to_string(),
                component: Some("number-input".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let render_ctx = RenderContext {
        props: &HashMap::new(),
    };

    let result = engine.render(&form, &render_ctx);
    assert!(result.is_ok(), "Form should render successfully");
    
    let html = result.unwrap();
    assert!(html.contains("Full Name"), "HTML should contain field label");
    assert!(html.contains("Quantity"), "HTML should contain field label");
}
