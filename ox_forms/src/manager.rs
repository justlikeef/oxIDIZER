use crate::registry::TypeRegistry;
use anyhow::{Context, Result};
use libloading::{Library, Symbol};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Function signature for the plugin initialization.
/// Plugins must export a function `extern "C" fn ox_forms_plugin_init(registry: *mut TypeRegistry) -> i32`
type PluginInitFn = unsafe extern "C" fn(*mut TypeRegistry) -> i32;

pub struct PluginManager {
    registry: Arc<Mutex<TypeRegistry>>,
    loaded_libraries: Vec<Library>, // Keep libs alive
}

impl PluginManager {
    pub fn new(registry: Arc<Mutex<TypeRegistry>>) -> Self {
        Self {
            registry,
            loaded_libraries: Vec::new(),
        }
    }

    /// Load a plugin from a shared object file
    pub fn load_plugin<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref();
        unsafe {
            let lib = Library::new(path).context(format!("Failed to load library {:?}", path))?;
            
            let init_fn: Symbol<PluginInitFn> = lib
                .get(b"ox_forms_plugin_init")
                .context("Failed to find 'ox_forms_plugin_init' symbol")?;

            let mut registry_lock = self.registry.lock().unwrap();
            let registry_ptr = &mut *registry_lock as *mut TypeRegistry;

            let result = init_fn(registry_ptr);
            if result != 0 {
                anyhow::bail!("Plugin initialization failed with code {}", result);
            }

            self.loaded_libraries.push(lib);
        }
        Ok(())
    }
}
