//! # ox_fileproc
//! 
//! `ox_fileproc` is a technical library for recursive configuration loading, 
//! variable substitution, and surgical structure-aware file editing.
//!
//! ## Core Components
//! - [`RawFile`] & [`Cursor`]: The "Surgical Editing" engine.
//! - [`process_file`]: The recursive configuration loader.

pub mod cursor;
pub mod processor;
pub mod scanners;
pub mod smart_merge;
pub mod substitutor;
mod repro_test;

pub use cursor::{Cursor, Format, RawFile};
pub use processor::process_file;
pub use serde_json;
pub use serde_yaml_ng as serde_yaml;
