pub mod schema;
pub mod traits;
pub mod registry;
#[cfg(not(target_arch = "wasm32"))]
pub mod manager;

pub use schema::*;
pub use traits::*;
pub use registry::*;
#[cfg(not(target_arch = "wasm32"))]
pub use manager::*;
pub mod render;

#[cfg(not(target_arch = "wasm32"))]
pub fn render_standard_form(form: &schema::FormDefinition, props: &std::collections::HashMap<String, serde_json::Value>) -> anyhow::Result<String> {
    use std::sync::{Arc, Mutex};
    let registry = Arc::new(Mutex::new(registry::TypeRegistry::new()));
    
    // Default mappings
    {
        let mut reg = registry.lock().unwrap();
        reg.load_from_config(registry::TypeMappingConfig {
            mappings: vec![
                ("integer".to_string(), registry::DefaultFieldConfig { component: "number-input".to_string(), default_props: serde_json::Value::Null }),
                ("float".to_string(), registry::DefaultFieldConfig { component: "number-input".to_string(), default_props: serde_json::Value::Null }),
                ("string".to_string(), registry::DefaultFieldConfig { component: "text-input".to_string(), default_props: serde_json::Value::Null }),
                ("password".to_string(), registry::DefaultFieldConfig { component: "password-input".to_string(), default_props: serde_json::Value::Null }),
                ("boolean".to_string(), registry::DefaultFieldConfig { component: "checkbox".to_string(), default_props: serde_json::Value::Null }),
                ("date".to_string(), registry::DefaultFieldConfig { component: "date-input".to_string(), default_props: serde_json::Value::Null }),
                ("select".to_string(), registry::DefaultFieldConfig { component: "select-input".to_string(), default_props: serde_json::Value::Null }),
                ("radio".to_string(), registry::DefaultFieldConfig { component: "radio".to_string(), default_props: serde_json::Value::Null }),
                ("hidden".to_string(), registry::DefaultFieldConfig { component: "hidden".to_string(), default_props: serde_json::Value::Null }),
                ("container".to_string(), registry::DefaultFieldConfig { component: "container".to_string(), default_props: serde_json::Value::Null }),
            ].into_iter().collect()
        });
    }

    let mut manager = manager::PluginManager::new(registry.clone());
    let dylib_name = if cfg!(target_os = "linux") {
        "libox_forms_std_renderers.so"
    } else {
        "libox_forms_std_renderers.dylib"  
    };
    
    let lib_path = std::env::current_exe()
        .ok()
        .and_then(|pb| pb.parent().map(|p| p.join(dylib_name)))
        .unwrap_or_else(|| std::path::Path::new(".").join(dylib_name));

    manager.load_plugin(&lib_path)?;

    let registry_lock = registry.lock().unwrap();
    let engine = render::FormEngine::new(&registry_lock);
    let render_ctx = traits::RenderContext { props };
    
    engine.render(form, &render_ctx)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn render_standard_module(module: &schema::ModuleSchema, form_id: &str, props: &std::collections::HashMap<String, serde_json::Value>) -> anyhow::Result<String> {
    use std::sync::{Arc, Mutex};
    let registry = Arc::new(Mutex::new(registry::TypeRegistry::new()));
    
    // Default mappings (copied for simplicity, in real app would be refactored)
    {
        let mut reg = registry.lock().unwrap();
        reg.load_from_config(registry::TypeMappingConfig {
            mappings: vec![
                ("integer".to_string(), registry::DefaultFieldConfig { component: "number-input".to_string(), default_props: serde_json::Value::Null }),
                ("float".to_string(), registry::DefaultFieldConfig { component: "number-input".to_string(), default_props: serde_json::Value::Null }),
                ("string".to_string(), registry::DefaultFieldConfig { component: "text-input".to_string(), default_props: serde_json::Value::Null }),
                ("password".to_string(), registry::DefaultFieldConfig { component: "password-input".to_string(), default_props: serde_json::Value::Null }),
                ("boolean".to_string(), registry::DefaultFieldConfig { component: "checkbox".to_string(), default_props: serde_json::Value::Null }),
                ("date".to_string(), registry::DefaultFieldConfig { component: "date-input".to_string(), default_props: serde_json::Value::Null }),
                ("select".to_string(), registry::DefaultFieldConfig { component: "select-input".to_string(), default_props: serde_json::Value::Null }),
                ("radio".to_string(), registry::DefaultFieldConfig { component: "radio".to_string(), default_props: serde_json::Value::Null }),
                ("hidden".to_string(), registry::DefaultFieldConfig { component: "hidden".to_string(), default_props: serde_json::Value::Null }),
                ("container".to_string(), registry::DefaultFieldConfig { component: "container".to_string(), default_props: serde_json::Value::Null }),
            ].into_iter().collect()
        });
    }

    let mut manager = manager::PluginManager::new(registry.clone());
    let dylib_name = if cfg!(target_os = "linux") {
        "libox_forms_std_renderers.so"
    } else {
        "libox_forms_std_renderers.dylib"  
    };
    
    let lib_path = std::env::current_exe()
        .ok()
        .and_then(|pb| pb.parent().map(|p| p.join(dylib_name)))
        .unwrap_or_else(|| std::path::Path::new(".").join(dylib_name));

    manager.load_plugin(&lib_path)?;

    let registry_lock = registry.lock().unwrap();
    let engine = render::FormEngine::new(&registry_lock).with_module(module);
    
    let form = module.forms.iter().find(|f| f.id == form_id)
        .ok_or_else(|| anyhow::anyhow!("Form '{}' not found in module '{}'", form_id, module.name))?;

    let render_ctx = traits::RenderContext { props };
    engine.render(form, &render_ctx)
}
