//! ox_type_converter - A modular type conversion system
//! 
//! This crate provides a flexible and extensible type conversion system
//! with individual conversion functions organized into separate modules.

pub mod value_type;
pub mod converters;
pub mod registry;

pub use value_type::ValueType;
pub use converters::TypeConverter;
pub use converters::generic_conversions::convert_value;
pub use registry::{ConversionRegistry, CONVERSION_REGISTRY};

// Re-export commonly used types
pub use std::collections::HashMap;
pub use std::any::Any;
