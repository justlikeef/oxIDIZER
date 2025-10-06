use std::collections::HashMap;
use std::sync::Mutex;
use lazy_static::lazy_static;
use std::any::Any;

/// Represents a type of event that callbacks can be registered for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventType(pub String);

impl EventType {
    pub fn new(name: &str) -> Self {
        EventType(name.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Type alias for a callback function that can receive a context object and additional parameters.
pub type CallbackFn = Box<dyn Fn(&dyn Any, &[&dyn Any]) + Send + Sync + 'static>;

/// Registry for managing callback functions.
pub struct CallbackManager {
    callbacks: HashMap<EventType, Vec<CallbackFn>>,
}

lazy_static! {
    /// The global callback manager.
    pub static ref CALLBACK_MANAGER: Mutex<CallbackManager> = Mutex::new(CallbackManager::new());
}

impl CallbackManager {
    /// Creates a new empty callback manager.
    fn new() -> Self {
        Self {
            callbacks: HashMap::new(),
        }
    }

    /// Registers a callback function for a given event type.
    pub fn register_callback(&mut self, event_type: EventType, callback: CallbackFn) {
        self.callbacks
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(callback);
    }

    /// Triggers all registered callback functions for a given event type.
    /// The `context` parameter is a generic object that can be downcast by the callback.
    /// The `params` slice contains additional generic parameters for the callback.
    pub fn trigger_callbacks_immutable(&self, event_type: &EventType, params: &[&dyn Any], context: &dyn Any) {
        if let Some(callbacks) = self.callbacks.get(event_type) {
            for callback in callbacks {
                callback(context, params);
            }
        }
    }

    /// Checks if any callbacks are registered for a given event type.
    pub fn has_callbacks(&self, event_type: &EventType) -> bool {
        self.callbacks.contains_key(event_type)
    }

    /// Returns a list of all registered event names.
    pub fn get_registered_events(&self) -> Vec<EventType> {
        self.callbacks.keys().cloned().collect()
    }
}