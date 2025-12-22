use crate::cursor::Cursor;
use crate::scanners::Scanner;

pub struct YamlScanner;

impl Scanner for YamlScanner {
    fn find_child<'a>(&self, parent: &Cursor<'a>, key: &str) -> Option<Cursor<'a>> {
        let block = parent.value();
        let parent_offset = parent.span.start;
        
        // YAML is tricky with just regex, need indentation tracking.
        // Assuming reasonably standard YAML (pretty printed).
        // Find "key:"
        
        // Simple heuristic for now: Find "key:" at the apparent "current indentation".
        // If parent is root, indent is 0.
        // If parent is a block, we need to determine its indent?
        // Actually, if we just search for "\n<indent>key:" we might find it.
        
        // Let's iterate lines.
        for (idx, line) in block.lines().enumerate() {
            // Check if line looks like "  key: value" or "- key: value" or just "key:"
            // We need to be careful about not matching inside comments or strings.
             let trimmed = line.trim_start();
             if trimmed.starts_with("#") { continue; }
             
             if let Some(rest) = trimmed.strip_prefix(key) {
                 if rest.trim_start().starts_with(':') {
                     // Check inline value
                     let val_part = rest.trim_start().strip_prefix(':').unwrap_or("").trim();
                     if !val_part.is_empty() && !val_part.starts_with('#') {
                         // Inline value logic (same as before)
                         let val_part_raw = rest.trim_start().strip_prefix(':').unwrap_or("");
                         let val_start_idx_in_line = line.len() - val_part_raw.len();
                         let val_trim = val_part_raw.trim_start();
                         let line_start_offset = find_line_start(block, idx).unwrap_or(0);
                         let val_absolute_start = parent_offset + line_start_offset + val_start_idx_in_line + (val_part_raw.len() - val_trim.len());
                         let value_end = if let Some(comment_idx) = val_trim.find('#') { comment_idx } else { val_trim.trim_end().len() };
                         
                         return Some(Cursor {
                             span: val_absolute_start..(val_absolute_start + value_end),
                             format: parent.format,
                             content_ref: parent.content_ref,
                         });
                     } else {
                         // Block value logic
                         // Scan forward to capture children content
                         // Identify key indent
                         let key_indent = line.len() - line.trim_start().len();
                         let mut block_start: Option<usize> = None;
                         let mut block_end = block.len();
                         
                         // Start checking lines after this key
                         let start_line_idx = idx + 1;
                         let line_start_offset = find_line_start(block, start_line_idx).unwrap_or(block.len());
                         
                         let mut relative_offset = line_start_offset;
                         // Iterate remaining lines
                         // We need a way to look at subsequent lines efficiently from 'block'
                         // block[line_start_offset..]
                         
                         for sub_line in block[line_start_offset..].lines() {
                             if sub_line.trim().is_empty() || sub_line.trim_start().starts_with('#') {
                                 // Skip empty/comments without breaking block
                                 relative_offset += sub_line.len() + 1; // +1 for newline approximation (imperfect for CRLF but find_line_start used split_inclusive)
                                 // Actually split_inclusive includes the newline in strict mode, lines() strips it.
                                 // Using lines() logic here is mismatched with byte offsets if newlines vary. 
                                 // Let's rely on `find_line_start` logic or byte iteration correctly.
                                 continue;
                             }
                             
                             let sub_indent = sub_line.len() - sub_line.trim_start().len();
                             if sub_indent <= key_indent {
                                 // End of block
                                 block_end = relative_offset;
                                 break;
                             }
                             
                             if block_start.is_none() {
                                 block_start = Some(relative_offset);
                             }
                             
                             relative_offset += sub_line.len() + 1; // approximation, unsafe for utf8 strings unless we know line break?
                             // Safe option: Use find_line_start logic.
                         }
                         
                         // Re-calc end properly without approximation loop:
                         // We iterate lines from idx+1. If line indent <= key indent, that line is STOP.
                         // The end of our block is the start of that line.
                         
                         let stop_idx = block.lines().skip(idx+1).position(|l| {
                             let t = l.trim();
                             if t.is_empty() || t.starts_with('#') { return false; }
                             let ind = l.len() - l.trim_start().len();
                             ind <= key_indent
                         });
                         
                         let end_byte_offset = if let Some(pos) = stop_idx {
                             find_line_start(block, idx + 1 + pos).unwrap_or(block.len())
                         } else {
                             block.len()
                         };
                         
                         // Start byte offset is line after key? Or first indented line?
                         // Usually we just want the whole block starting after the key line newline.
                         let content_start = find_line_start(block, idx + 1).unwrap_or(block.len()); // Start of next line
                         
                         return Some(Cursor {
                             span: (parent_offset + content_start)..(parent_offset + end_byte_offset),
                             format: parent.format,
                             content_ref: parent.content_ref,
                         });
                     }
                 }
             }
        }
        None
    }

    fn find_entry_with_key_value<'a>(&self, parent: &Cursor<'a>, key: &str, value: &str) -> Option<Cursor<'a>> {
        // Iterate list items (lines starting with "-")
        // Check if they contain "key: value"
        let block = parent.value();
        let parent_offset = parent.span.start;

        let mut current_item_start: Option<usize> = None;
        
        // Iterate lines to find "- ". 
        // When found, scan forward until next "- " or end indent loop.
        
        // We need a more robust line iterator that gives us byte offsets.
        let mut offset = 0;
        for line in block.split_inclusive('\n') {
            let line_len = line.len();
            let trimmed = line.trim_start();
            
            if trimmed.starts_with("- ") {
                // New item starts
                // Check previous item?
                // Actually, we want to return the CURSOR to the ITEM BLOCK (so we can find children in it).
                
                // If we were scanning an item, did it match?
                // Realistically, to support "Find item with key=val", we check the lines associated with this item immediately?
                
                // Let's assume the key:val is on a line inside this item block.
                // We start a "sub-scan" or just check lines until next item.
                current_item_start = Some(offset);
            }
            
            // Check for match in current scope
            // Handle compact syntax: "- key: value"
            let content_to_check = if trimmed.starts_with("- ") {
                trimmed.strip_prefix("- ").unwrap().trim_start()
            } else {
                trimmed
            };

            if content_to_check.starts_with(key) {
                 if let Some(rest) = content_to_check.strip_prefix(key) {
                     if rest.trim_start().starts_with(':') {
                         let val_part = rest.trim_start().strip_prefix(':').unwrap_or("").trim();
                         if val_part == value {
                             // Found it! 
                             // We need to return the Cursor covering the whole ITEM BLOCK.
                             // Start is current_item_start.
                             // End is ... start of next item?
                             if let Some(start) = current_item_start {
                                  // Find end of this block (next line starting with - at same indent?)
                                  // For now, return start..end_of_block
                                  // Hack: return start..end_of_string (rest of parent). 
                                  // Ideally we bound it.
                                  return Some(Cursor {
                                      span: (parent_offset + start)..(parent_offset + block.len()), // TODO: tighten end
                                      format: parent.format,
                                      content_ref: parent.content_ref,
                                  });
                             }
                         }
                     }
                 }
            }
            
            offset += line_len;
        }
        
        None
    }
}

// Helper to find byte offset of Nth line
fn find_line_start(s: &str, line_idx: usize) -> Option<usize> {
    let mut offset = 0;
    for (i, line) in s.split_inclusive('\n').enumerate() {
        if i == line_idx {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}
