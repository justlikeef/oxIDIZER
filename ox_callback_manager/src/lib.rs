//! ox_callback_manager - A modular callback management system

pub mod registry;

pub use registry::{CallbackRegistry, CALLBACK_REGISTRY};
pub use std::any::Any;
