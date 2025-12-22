use std::path::{Path, PathBuf};
use std::fs;
use anyhow::{Result, Context, anyhow};
use std::ops::Range;

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

pub struct RawFile {
    pub path: PathBuf,
    pub content: String,
    pub format: Format,
}

impl RawFile {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read file: {:?}", path))?;
        let format = Format::from_path(&path);
        Ok(Self { path, content, format })
    }

    pub fn save(&self) -> Result<()> {
        fs::write(&self.path, &self.content)
            .with_context(|| format!("Failed to write file: {:?}", self.path))
    }

    /// Update the value at the cursor's position with new_val.
    /// This performs a surgical string replacement.
    pub fn update(&mut self, span: Range<usize>, new_val: &str) {
        self.content.replace_range(span, new_val);
    }
    
    /// Append a new item as a child of the node at cursor.
    /// Note: Caller is responsible for formatting (newlines, indentation) correctly.
    pub fn append(&mut self, cursor: &Cursor, item: &str) -> Result<()> {
         self.content.insert_str(cursor.span.end, item);
         Ok(())
    }

    pub fn find(&self, query: &str) -> impl Iterator<Item = Cursor> + '_ {
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

#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    pub span: Range<usize>,
    pub format: Format,
    pub content_ref: &'a str,
}

impl<'a> Cursor<'a> {
    pub fn value(&self) -> &'a str {
        &self.content_ref[self.span.clone()]
    }
}
