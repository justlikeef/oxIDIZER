//! ox_callback_manager - A modular callback management system

pub mod registry;

pub use registry::{CallbackManager, CALLBACK_MANAGER, EventType};
pub use std::any::Any;