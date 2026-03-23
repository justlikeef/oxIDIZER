//! ox_callback_manager - A modular callback management system

pub mod registry;

pub use registry::{CallbackManager, CALLBACK_MANAGER, EventType, CallbackFn, CallbackError, CallbackAction, CallbackResult};
pub use std::any::Any;
