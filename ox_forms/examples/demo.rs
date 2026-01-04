use ox_forms::registry::TypeRegistry;
use ox_forms::manager::PluginManager;
use ox_forms::render::FormEngine;
use ox_forms::schema::{FormDefinition, FieldDefinition};
use ox_forms::traits::RenderContext;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    // 1. Initialize Registry
    let registry = Arc::new(Mutex::new(TypeRegistry::new()));
    
    // 2. Load Plugin
    let mut manager = PluginManager::new(registry.clone());
    
    // Assume we are running from workspace root or crate root
    // Try to find the built library
    let dylib_name = if cfg!(target_os = "linux") {
        "libox_forms_std_renderers.so"
    } else if cfg!(target_os = "macos") {
        "libox_forms_std_renderers.dylib"
    } else {
        "ox_forms_std_renderers.dll"
    };

    // We are in oxIDIZER/ox_forms, workspace target is oxIDIZER/target
    let target_dir = PathBuf::from("../target/debug");
    let lib_path = target_dir.join(dylib_name);
    
    println!("Current Dir: {:?}", std::env::current_dir()?);
    
    println!("Loading plugin from: {:?}", lib_path);
    manager.load_plugin(&lib_path)?;
    println!("Plugin loaded successfully!");

    // 3. Create Sample Form
    let form = FormDefinition {
        id: "login_form".to_string(),
        title: "Login".to_string(),
        fields: vec![
            FieldDefinition {
                name: "username".to_string(),
                label: "Username".to_string(),
                data_type: "string".to_string(),
                component: Some("text-input".to_string()),
                plugins: vec![],
                validation: vec![],
                dependencies: vec![],
                props: serde_json::Value::Null,
            },
            FieldDefinition {
                name: "age".to_string(),
                label: "Age".to_string(),
                data_type: "integer".to_string(),
                component: Some("number-input".to_string()),
                plugins: vec![],
                validation: vec![],
                dependencies: vec![],
                props: serde_json::Value::Null,
            },
        ],
        layout: None,
        actions: vec![],
        data_source_binding: None,
    };

    // 4. Render
    let registry_lock = registry.lock().unwrap();
    let engine = FormEngine::new(&registry_lock);
    let ctx = RenderContext {
        props: &HashMap::new(),
    };

    println!("Rendering Form...");
    let html = engine.render(&form, &ctx)?;
    
    println!("--- Rendered Output ---");
    println!("{}", html);
    println!("-----------------------");

    if html.contains("<form") && html.contains("username") && html.contains("input type=\"text\"") {
        println!("VERIFICATION PASSED");
    } else {
        println!("VERIFICATION FAILED: Output did not contain expected tags.");
        std::process::exit(1);
    }

    Ok(())
}
