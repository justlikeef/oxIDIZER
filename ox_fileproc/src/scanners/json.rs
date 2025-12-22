use crate::cursor::Cursor;
use crate::scanners::Scanner;

pub struct JsonScanner;

impl Scanner for JsonScanner {
    fn find_child<'a>(&self, _parent: &Cursor<'a>, _key: &str) -> Option<Cursor<'a>> {
        None
    }

    fn find_entry_with_key_value<'a>(&self, _parent: &Cursor<'a>, _key: &str, _value: &str) -> Option<Cursor<'a>> {
        None
    }
}
