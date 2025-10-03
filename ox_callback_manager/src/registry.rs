use std::collections::HashMap;
use std::sync::Mutex;
use lazy_static::lazy_static;
use std::any::Any;

/// Type alias for a callback function.
/// Callbacks take a generic context object and return a Result indicating success or failure.
pub type CallbackFn = fn(&dyn Any) -> Result<(), String>;

/// Registry for managing callback functions.
pub struct CallbackRegistry {
    callbacks: HashMap<String, Vec<CallbackFn>>,
}

lazy_static! {
    /// The global callback registry.
    pub static ref CALLBACK_REGISTRY: Mutex<CallbackRegistry> = Mutex::new(CallbackRegistry::new());
}

impl CallbackRegistry {
    /// Creates a new empty callback registry.
    fn new() -> Self {
        Self {
            callbacks: HashMap::new(),
        }
    }

    /// Registers a callback function for a given event name.
    pub fn register_callback(&mut self, event_name: &str, callback: CallbackFn) {
        self.callbacks
            .entry(event_name.to_string())
            .or_insert_with(Vec::new)
            .push(callback);
    }

    /// Triggers all registered callback functions for a given event name.
    /// The `context` parameter is a generic object that can be downcast by the callback.
    pub fn trigger_callbacks(&self, event_name: &str, context: &dyn Any) -> Result<(), String> {
        if let Some(callbacks) = self.callbacks.get(event_name) {
            for callback in callbacks {
                callback(context)?;
            }
        }
        Ok(())
    }

    /// Checks if any callbacks are registered for a given event name.
    pub fn has_callbacks(&self, event_name: &str) -> bool {
        self.callbacks.contains_key(event_name)
    }

    /// Returns a list of all registered event names.
    pub fn get_registered_events(&self) -> Vec<String> {
        self.callbacks.keys().cloned().collect()
    }
}
