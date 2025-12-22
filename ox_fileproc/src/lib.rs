pub mod substitutor;
pub mod processor;

pub use processor::{process_file, read_raw_file};
pub mod cursor;
pub mod scanners;
pub use cursor::{RawFile, Cursor};

