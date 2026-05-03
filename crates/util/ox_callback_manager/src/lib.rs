//! ox_callback_manager - A modular callback management system

pub mod registry;
pub mod error;

pub use registry::{CallbackManager, EventType, CallbackFn, CallbackParams};
pub use error::CallbackError;
