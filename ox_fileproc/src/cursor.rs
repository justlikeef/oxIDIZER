use std::path::{Path, PathBuf};
use std::fs;
use anyhow::{Result, Context};
use std::ops::Range;

/// Supported file formats for the Cursor engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    Json,
    Yaml,
    Toml,
    Xml,
    Kdl,
    Unknown,
}

impl Format {
    /// Infers the format from a file path extension.
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase().as_str() {
            "json" | "json5" => Format::Json, // Map json5 to Json generic
            "yaml" | "yml" => Format::Yaml,
            "toml" => Format::Toml,
            "xml" => Format::Xml,
            "kdl" => Format::Kdl,
            _ => Format::Unknown,
        }
    }
}

/// Represents a raw file on disk, maintained in memory for surgical editing.
pub struct RawFile {
    /// Path to the file.
    pub path: PathBuf,
    /// In-memory string representation of the file.
    pub content: String,
    /// The detected format of the file.
    pub format: Format,
}

impl RawFile {
    /// Opens a file from the filesystem.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read file: {:?}", path))?;
        let format = Format::from_path(&path);
        Ok(Self { path, content, format })
    }

    /// Saves the in-memory content back to the disk at `self.path`.
    pub fn save(&self) -> Result<()> {
        fs::write(&self.path, &self.content)
            .with_context(|| format!("Failed to write file: {:?}", self.path))
    }

    /// Update the value at the cursor's position with `new_val`.
    /// 
    /// This performs a surgical string replacement, maintaining all surrounding 
    /// text, comments, and formatting.
    pub fn update(&mut self, span: Range<usize>, new_val: &str) {
        self.content.replace_range(span, new_val);
    }
    
    /// Append a new item as a child of the node at `cursor`.
    /// 
    /// Note: Caller is responsible for formatting (newlines, indentation) correctly.
    pub fn append(&mut self, cursor: &Cursor, item: &str) -> Result<()> {
         self.content.insert_str(cursor.span.end, item);
         Ok(())
    }

    /// Navigates the file structure using a path query.
    /// 
    /// # Query Syntax
    /// - `key`: Selects child with key "key".
    /// - `section/key`: Selects nested child.
    /// - `items[id=val]`: Selects list item where the key `id` matches `val`.
    pub fn find(&self, query: &str) -> impl Iterator<Item = Cursor<'_>> + '_ {
        let mut currents = vec![Cursor {
            span: 0..self.content.len(),
            format: self.format,
            content_ref: &self.content,
        }];
        
        let segments: Vec<&str> = query.split('/').collect();
        
        for seg in segments {
            if currents.is_empty() { break; }
            if seg.is_empty() { continue; }
            
            let mut nexts = Vec::new();
            
            // Format: "key[filter_key=filter_val]" or just "key"
            let (key, filter) = if let Some(bracket_start) = seg.find('[') {
                if let Some(bracket_end) = seg.find(']') {
                    let key = &seg[0..bracket_start];
                    let filter_part = &seg[bracket_start+1..bracket_end];
                    let parts: Vec<&str> = filter_part.split('=').collect();
                    let filter = if parts.len() == 2 {
                        Some((parts[0], parts[1]))
                    } else {
                        None
                    };
                    (key, filter)
                } else {
                    (seg, None)
                }
            } else {
                (seg, None)
            };
            
            for cur in currents {
                let scanner: Box<dyn crate::scanners::Scanner> = match cur.format {
                    Format::Yaml => Box::new(crate::scanners::yaml::YamlScanner),
                    Format::Json => Box::new(crate::scanners::json::JsonScanner),
                    _ => Box::new(crate::scanners::yaml::YamlScanner),
                };
                
                // 1. Find matched child by key
                let target = if !key.is_empty() {
                    scanner.find_child(&cur, key)
                } else {
                    Some(cur) // Empty key matches self (current scope) 
                };
                
                if let Some(t) = target {
                    // 2. Apply filter if present
                    if let Some((f_key, f_val)) = filter {
                        if let Some(filtered) = scanner.find_entry_with_key_value(&t, f_key, f_val) {
                            nexts.push(filtered);
                        }
                    } else {
                        nexts.push(t);
                    }
                }
            }
            currents = nexts;
        }
        
        currents.into_iter()
    }
}

/// A virtual "view" into a segment of a [`RawFile`].
/// 
/// A `Cursor` defines a range (span) within the raw text that corresponds to 
/// a logical element (a key, a value, or a block).
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    /// The byte range within the raw text.
    pub span: Range<usize>,
    /// The format of the file this cursor belongs to.
    pub format: Format,
    /// Reference to the underlying raw content.
    pub content_ref: &'a str,
}

impl<'a> Cursor<'a> {
    /// Returns the text slice associated with this cursor's span.
    pub fn value(&self) -> &'a str {
        &self.content_ref[self.span.clone()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_path() {
        assert_eq!(Format::from_path(Path::new("file.json")), Format::Json);
        assert_eq!(Format::from_path(Path::new("file.yaml")), Format::Yaml);
        assert_eq!(Format::from_path(Path::new("file.yml")), Format::Yaml);
        assert_eq!(Format::from_path(Path::new("file.toml")), Format::Toml);
        assert_eq!(Format::from_path(Path::new("file.unknown")), Format::Unknown);
    }

    #[test]
    fn test_rawfile_open_find_update() -> Result<()> {
        // We can't easily test open() without a real file or mocking fs.
        // But we can test the struct logic if we construct it manually.
        let content = r#"key: value
section:
  foo: bar
"#;
        let mut raw = RawFile {
            path: PathBuf::from("test.yaml"),
            content: content.to_string(),
            format: Format::Yaml,
        };

        // Test find
        let cursors: Vec<_> = raw.find("key").collect();
        assert_eq!(cursors.len(), 1);
        assert_eq!(cursors[0].value(), "value");

        let cursors: Vec<_> = raw.find("section/foo").collect();
        assert_eq!(cursors.len(), 1);
        assert_eq!(cursors[0].value(), "bar");

        // Test update
        let span = cursors[0].span.clone();
        raw.update(span, "baz");
        
        assert!(raw.content.contains("foo: baz"));
        assert!(!raw.content.contains("foo: bar"));
        
        Ok(())
    }
    
    #[test]
    fn test_rawfile_append() {
         let content = "items:\n";
         // We construct raw with the string content
         let mut raw = RawFile {
            path: PathBuf::from("test.yaml"),
            content: content.to_string(),
            format: Format::Yaml,
        };
        
        // We construct a cursor that points to the SAME logical content, but physically borrows the local `content` str,
        // (or we can just use an empty string if append doesn't read it, but let's be safe and point to something valid).
        // Since `append` only uses `cursor.span.end`, the content_ref doesn't strictly matter for logic, 
        // but for type safety it must be a &str.
        // We use the local `content` variable which `raw.content` was cloned from.
        let root = Cursor {
            span: 0..content.len(),
            format: Format::Yaml,
            content_ref: content, 
        };
        
        raw.append(&root, "  - item1\n").unwrap();
        assert_eq!(raw.content, "items:\n  - item1\n");
    }

    #[test]
    fn test_find_deep_nesting() {
        let content = r#"
a:
  b:
    c:
      d: 100
"#;
        let raw = RawFile {
            path: PathBuf::from("nest.yaml"),
            content: content.to_string(),
            format: Format::Yaml,
        };
        
        let res: Vec<_> = raw.find("a/b/c/d").collect();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].value().trim(), "100");
    }

    #[test]
    fn test_json_surgical_update() -> Result<()> {
        let content = r#"{
  "server": {
    "port": 8080
  }
}"#;
        let mut raw = RawFile {
            path: PathBuf::from("test.json"),
            content: content.to_string(),
            format: Format::Json,
        };
        
        let cursor = raw.find("server/port").next().expect("Should find port");
        raw.update(cursor.span, "9090");
        
        assert!(raw.content.contains("\"port\": 9090"));
        Ok(())
    }
}
