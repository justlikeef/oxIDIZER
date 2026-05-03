use std::sync::Arc;
use crate::error::CallbackError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventType(pub String);

impl EventType {
    pub fn new(s: &str) -> Self {
        EventType(s.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct CallbackParams {
    pub event_type: EventType,
    pub attribute: Option<String>,
    pub value: Option<String>,
    pub error: Option<String>,
}

pub type CallbackFn<T> = Arc<dyn Fn(&mut T, &CallbackParams) -> Result<(), CallbackError> + Send + Sync>;

pub struct CallbackManager<T> {
    callbacks: Vec<(EventType, CallbackFn<T>)>,
}

impl<T> CallbackManager<T> {
    pub fn new() -> Self {
        Self {
            callbacks: Vec::new(),
        }
    }

    pub fn register(&mut self, event_type: EventType, callback: CallbackFn<T>) {
        self.callbacks.push((event_type, callback));
    }

    pub fn trigger(&self, context: &mut T, params: &CallbackParams) -> Result<(), CallbackError> {
        for (evt, cb) in &self.callbacks {
            if evt == &params.event_type {
                cb(context, params)?;
            }
        }
        Ok(())
    }
}

impl<T> Default for CallbackManager<T> {
    fn default() -> Self {
        Self::new()
    }
}
