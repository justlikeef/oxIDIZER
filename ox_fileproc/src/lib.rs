pub mod cursor;
pub mod processor;
pub mod scanners;
pub mod substitutor;
mod repro_test;

pub use cursor::{Cursor, Format, RawFile};
pub use processor::process_file;
// Wait, previous file had ReadRawFile and RawFile.
// In `cursor.rs` I see `RawFile` is defined.
// In `processor.rs`, `process_file` is defined.
// I need these consistent.
