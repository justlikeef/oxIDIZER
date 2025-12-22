use std::ops::Range;
use crate::cursor::{Cursor, Format};

pub trait Scanner {
    fn find_child<'a>(&self, parent: &Cursor<'a>, key: &str) -> Option<Cursor<'a>>;
    fn find_entry_with_key_value<'a>(&self, parent: &Cursor<'a>, key: &str, value: &str) -> Option<Cursor<'a>>;
}

pub mod yaml;
pub mod json;
