use crate::cursor::Cursor;
use crate::scanners::Scanner;

pub struct JsonScanner;

impl Scanner for JsonScanner {
    fn find_child<'a>(&self, parent: &Cursor<'a>, key: &str) -> Option<Cursor<'a>> {
        let block = parent.value().trim();
        let parent_offset = parent.span.start + parent.value().find(block).unwrap_or(0);
        
        // Strip outer braces if present
        let (content, offset_shift) = if block.starts_with('{') && block.ends_with('}') {
            (&block[1..block.len()-1], 1)
        } else {
            (block, 0)
        };
        
        // We need to iterate and find "key":
        let mut depth = 0;
        let mut in_str = false;
        let mut escaped = false;
        
        let chars: Vec<(usize, char)> = content.char_indices().collect();
        let mut i = 0;
        
        while i < chars.len() {
            let (_, c) = chars[i];
            
            if in_str {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_str = false;
                }
                i += 1;
                continue;
            }
            
            if c == '"' {
                in_str = true;
                // Possible key start?
                // Check if we are at depth 0 (top level of THIS object)
                if depth == 0 {
                    // Check if this string matches "key"
                    // We need to look ahead
                    let start_quote_idx = i;
                    // Scan forward to find closing quote to verify content
                    let mut j = i + 1;
                    let mut k_escaped = false;
                    while j < chars.len() {
                        let (_, kc) = chars[j];
                        if k_escaped {
                             k_escaped = false;
                        } else if kc == '\\' {
                             k_escaped = true;
                        } else if kc == '"' {
                             break;
                        }
                        j += 1;
                    }
                    
                    if j < chars.len() {
                        // Found closing quote
                         let key_candidate = &content[chars[start_quote_idx].0 + 1 .. chars[j].0];
                         if key_candidate == key {
                             // Matched key! Now check for colon
                             let mut k = j + 1;
                             while k < chars.len() && chars[k].1.is_whitespace() { k += 1; }
                             if k < chars.len() && chars[k].1 == ':' {
                                 // Found "key": ... Value starts after colon
                                 let val_start_idx = chars[k].0 + 1;
                                 
                                 // Determine Value span
                                 // Scan from val_start_idx to find json value end
                                 let rest = &content[val_start_idx..];
                                 let trimmed_rest = rest.trim_start();
                                 let start_padding = rest.len() - trimmed_rest.len();
                                 let val_actual_start = val_start_idx + start_padding;
                                 
                                 // Use a helper to skip value
                                 // We need to re-use the wrapper logic for skipping value
                                 // Reuse the logic from original implementation for skipping value?
                                 // Or just write a quick skipper.
                                 
                                 // let mut v_depth = 0; // Moved inside
                                 let mut v_in_str = false;
                                 let mut v_escaped = false;
                                 let mut val_len = 0;
                                 
                                 let val_chars: Vec<(usize, char)> = trimmed_rest.char_indices().collect();
                                 if val_chars.is_empty() { break; }
                                 
                                 let first = val_chars[0].1;
                                 if first == '{' || first == '[' {
                                     let close = if first == '{' { '}' } else { ']' };
                                     let mut v_depth = 1;
                                     for (vi, (_, vc)) in val_chars.iter().enumerate().skip(1) {
                                         if v_in_str {
                                             if v_escaped { v_escaped = false; }
                                             else if *vc == '\\' { v_escaped = true; }
                                             else if *vc == '"' { v_in_str = false; }
                                         } else if *vc == '"' { v_in_str = true; }
                                         else if *vc == first { v_depth += 1; }
                                         else if *vc == close {
                                             v_depth -= 1;
                                             if v_depth == 0 {
                                                 val_len = vi + 1;
                                                 break;
                                             }
                                         }
                                     }
                                 } else if first == '"' {
                                     // String
                                    /* v_in_str = true; // Unused */
                                     for (vi, (_, vc)) in val_chars.iter().enumerate().skip(1) {
                                         if v_escaped { v_escaped = false; }
                                         else if *vc == '\\' { v_escaped = true; }
                                         else if *vc == '"' {
                                             val_len = vi + 1;
                                             break;
                                         }
                                     }
                                 } else {
                                     // Primitive (number, bool, null)
                                     // Read until comma or end of block (which is end of content string here)
                                      for (vi, (_, vc)) in val_chars.iter().enumerate() {
                                          if *vc == ',' || *vc == '}' || *vc == ']' || vc.is_whitespace() {
                                              if *vc == ',' { val_len = vi; break; }
                                              if vc.is_whitespace() {
                                                  // check if followed by comma?
                                                  // simplified: just end at whitespace? No, "true "
                                                  // primitives don't have spaces inside.
                                                  val_len = vi; break; 
                                              }
                                              val_len = vi; break;
                                          }
                                          val_len = vi + 1;
                                      }
                                 }
                                 
                                 // Adjust val_len if we stopped early
                                 let final_start = parent_offset + offset_shift + val_actual_start;
                                 let final_end = final_start + val_len;
                                 
                                 return Some(Cursor {
                                     span: final_start..final_end,
                                     format: parent.format,
                                     content_ref: parent.content_ref,
                                 });
                             }
                         }
                         // Move i to j to skip validity check
                         i = j;
                         in_str = false;
                    }
                }
            } else if c == '{' || c == '[' {
                depth += 1;
            } else if (c == '}' || c == ']')
                && depth > 0 { depth -= 1; }
            
            i += 1;
        }
        
        None
    }

    fn find_entry_with_key_value<'a>(&self, parent: &Cursor<'a>, key: &str, value: &str) -> Option<Cursor<'a>> {
        let block = parent.value();
        let parent_offset = parent.span.start;
        
        // Find list entries [{}, {}]
        let trimmed_content = block.trim();
        if !trimmed_content.starts_with('[') { return None; }
        
        // Heuristic: iterate {} blocks
        let mut offset = 0;
        while let Some(start_idx) = block[offset..].find('{') {
            let abs_start = offset + start_idx;
            // Get CURSOR for this object
            let mut depth = 0;
            let mut end_idx = 0;
            let mut in_str = false;
            let mut escaped = false;

            for (i, c) in block[abs_start..].chars().enumerate() {
                if in_str {
                    if escaped { escaped = false; }
                    else if c == '\\' { escaped = true; }
                    else if c == '"' { in_str = false; }
                } else if c == '"' { in_str = true; }
                else if c == '{' { depth += 1; }
                else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        end_idx = abs_start + i + 1;
                        break;
                    }
                }
            }
            
            if end_idx > abs_start {
                let item_cursor = Cursor {
                    span: (parent_offset + abs_start)..(parent_offset + end_idx),
                    format: parent.format,
                    content_ref: parent.content_ref,
                };
                
                // Check if this item matches key=value
                if let Some(child) = self.find_child(&item_cursor, key) {
                    let val = child.value().trim().trim_matches('"');
                    if val == value {
                        return Some(item_cursor);
                    }
                }
                offset = end_idx;
            } else {
                break;
            }
        }
        
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_scanner_find_child() {
         let content = r#"{"key": "value", "nested": {"a": 1}}"#;
         let scanner = JsonScanner;
         let root = Cursor {
            span: 0..content.len(),
            format: crate::cursor::Format::Json,
            content_ref: content,
        };
        
        let child = scanner.find_child(&root, "key").expect("Should find key");
        assert_eq!(child.value().trim().trim_matches('"'), "value");

        let nested = scanner.find_child(&root, "nested").expect("Should find nested");
        assert_eq!(nested.value().trim(), "{\"a\": 1}");
    }

    #[test]
    fn test_json_scanner_find_entry() {
        let content = r#"[{"id": "1", "name": "first"}, {"id": "2", "name": "second"}]"#;
        let scanner = JsonScanner;
        let root = Cursor {
            span: 0..content.len(),
            format: crate::cursor::Format::Json,
            content_ref: content,
        };

        let entry = scanner.find_entry_with_key_value(&root, "id", "2").expect("Should find entry 2");
        assert!(entry.value().contains("\"name\": \"second\""));
    }
}
