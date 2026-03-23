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

/// Defines the action to be taken upon a callback error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackAction {
    /// Continue processing, but return the error to the caller.
    Continue,
    /// Attempt to roll back the action that triggered the callback.
    Rollback,
}

/// Represents an error returned by a callback.
#[derive(Debug, Clone)]
pub struct CallbackError {
    pub message: String,
    pub action: CallbackAction,
}

/// The result type for a callback function.
/// On success, it can optionally return a message (`Ok(Some(String))`).
/// On failure, it returns a `CallbackError`.
pub type CallbackResult = Result<Option<String>, CallbackError>;

/// Type alias for a callback function.
/// It receives a mutable context object and additional parameters.
pub type CallbackFn = Box<dyn FnMut(&mut dyn Any, &[&dyn Any]) -> CallbackResult + Send + Sync + 'static>;

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
            .or_default()
            .push(callback);
    }

    /// Triggers all registered callback functions for a given event type.
    ///
    /// # Arguments
    /// * `event_type` - The event to trigger callbacks for.
    /// * `context` - A mutable reference to a context object (e.g., `GenericDataObject`) that the callback can operate on.
    /// * `params` - A slice of additional parameters for the callback.
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - A vector of success messages from all callbacks that returned one.
    /// * `Err(CallbackError)` - The error from the first callback that failed.
    pub fn trigger_callbacks(
        &mut self,
        event_type: &EventType,
        context: &mut dyn Any,
        params: &[&dyn Any],
    ) -> Result<Vec<String>, CallbackError> {
        let mut messages = Vec::new();

        if let Some(callbacks) = self.callbacks.get_mut(event_type) {
            for callback in callbacks {
                match callback(context, params) {
                    Ok(Some(message)) => messages.push(message),
                    Ok(None) => { /* Continue processing */ }
                    Err(e) => return Err(e), // Stop on first error
                }
            }
        }

        Ok(messages)
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