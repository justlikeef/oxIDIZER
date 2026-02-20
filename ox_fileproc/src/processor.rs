use crate::{substitutor, smart_merge};
use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use log::error;



const INCLUDE_KEY: &str = "include";
const MERGE_KEY: &str = "merge";
const MERGE_RECURSIVE_KEY: &str = "merge_recursive";
const SUBSTITUTIONS_KEY: &str = "substitutions";

#[derive(Clone)]
enum ConfigInput<'a> {
    File(&'a Path),
    Raw {
        content: &'a str,
        extension: &'a str,
        base_path: Option<&'a Path>,
    },
    Value {
        value: Value,
        base_path: Option<&'a Path>,
    },
}

impl ConfigInput<'_> {
    fn path(&self) -> Option<&Path> {
        match self {
            ConfigInput::File(p) => Some(p),
            ConfigInput::Raw { base_path, .. } => *base_path,
            ConfigInput::Value { base_path, .. } => *base_path,
        }
    }
}

/// Builder for configuring the file processing engine.
pub struct Processor {
    max_depth: usize,
    root_dir: Option<PathBuf>,
    strict_dir_includes: bool,
    use_env_vars: bool,
}

impl Processor {
    /// Creates a new `Processor` with default settings:
    /// - `max_depth`: 10
    /// - `root_dir`: `None` (No path restriction)
    /// - `strict_dir_includes`: `true` (Fails on directory IO errors)
    /// - `use_env_vars`: `false` (Environment variables disabled)
    pub fn new() -> Self {
        Self {
            max_depth: 10,
            root_dir: None,
            strict_dir_includes: true,
            use_env_vars: false,
        }
    }

    /// Sets the maximum recursion depth.
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }
    
    /// Alias for max_depth builder style.
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    /// Sets the root directory. Files outside this directory (after canonicalization) will be rejected.
    /// This prevents path traversal attacks.
    pub fn with_root_dir<P: AsRef<Path>>(mut self, root: P) -> Self {
        self.root_dir = Some(fs::canonicalize(root.as_ref()).unwrap_or(root.as_ref().to_path_buf()));
        self
    }

    /// Controls whether directory includes fail on the first error (strict) or skip invalid files.
    pub fn strict_dir_includes(mut self, strict: bool) -> Self {
        self.strict_dir_includes = strict;
        self
    }

    /// Controls whether environment variables are used as fallbacks for substitution. Default: false.
    pub fn use_env_vars(mut self, use_env: bool) -> Self {
        self.use_env_vars = use_env;
        self
    }

    /// Processes a file using the configured settings.
    pub fn process<P: AsRef<Path>>(&self, path: P) -> Result<Value> {
        let mut visited = Vec::new();
        load_recursive(ConfigInput::File(path.as_ref()), &HashMap::new(), &mut visited, 0, self)
    }

    /// Processes raw content as a config, supporting recursion from a base path.
    pub fn process_str(&self, content: &str, extension: &str, base_path: Option<&Path>) -> Result<Value> {
        let mut visited = Vec::new();
        load_recursive(ConfigInput::Raw { content, extension, base_path }, &HashMap::new(), &mut visited, 0, self)
    }

    /// Processes a pre-deserialized Value, supporting recursion from a base path.
    pub fn process_value(&self, value: Value, base_path: Option<&Path>) -> Result<Value> {
        let mut visited = Vec::new();
        load_recursive(ConfigInput::Value { value, base_path }, &HashMap::new(), &mut visited, 0, self)
    }
}

impl Default for Processor {
    fn default() -> Self {
        Self::new()
    }
}

pub fn process_file(path: &Path, max_depth: usize) -> Result<Value> {
    Processor::new().with_max_depth(max_depth).process(path)
}

fn load_recursive(input: ConfigInput, parent_vars: &HashMap<String, String>, visited: &mut Vec<PathBuf>, current_depth: usize, processor: &Processor) -> Result<Value> {
    let canonical_path = if let Some(path) = input.path() {
        let cp = fs::canonicalize(path)
            .with_context(|| format!("Failed to canonicalize path: {:?}", path))?;

        // Security Check: Root Dir
        if let Some(ref root) = processor.root_dir
            && !cp.starts_with(root) {
                 return Err(anyhow!("Security violation: Access denied to {:?} (outside root {:?})", cp, root));
            }

        if visited.iter().any(|v| v == &cp) {
             return Err(anyhow!("Circular dependency detected: {:?}", cp));
        }
        Some(cp)
    } else {
        None
    };

    if processor.max_depth > 0 && current_depth > processor.max_depth {
        return Err(anyhow!("Recursion depth limit reached ({})", processor.max_depth));
    }
    
    if let Some(ref cp) = canonical_path {
        visited.push(cp.clone());
    }
    
    let res = load_recursive_inner(input, parent_vars, visited, current_depth, processor);

    if canonical_path.is_some() {
        visited.pop();
    }
    res
}

/// Simple wrapper to read a file's content as a string.
pub fn read_raw_file(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {:?}", path))?;
    Ok(content)
}

/// Parses raw content into a `serde_json::Value` based on the file extension.
/// 
/// Supported extensions: `json`, `yaml`, `yml`, `toml`, `xml`, `json5`, `kdl`.
pub fn parse_content(content: &str, extension: &str) -> Result<Value> {
    match extension {
        "json" => serde_json::from_str(content).map_err(Into::into),
        "yaml" | "yml" => serde_yaml_ng::from_str(content).map_err(Into::into),
        "toml" => {
            let toml_val: toml::Value = toml::from_str(content)?;
            Ok(serde_json::to_value(toml_val)?)
        },
        "xml" => {
            // Security Check: Detect DTDs/Entities.
            // Scan until the first root element. Any DTD/Entity declaration before the root is an error.
            // This is safer than a fixed size check and avoids bypassing via massive whitespace/comments.
            
            // Re-implementing using `find` loop for simplicity and correctness
            let mut cursor = 0;
            while let Some(idx) = content[cursor..].find('<') {
                let start = cursor + idx;
                let remainder = &content[start..];
                
                if remainder.starts_with("<!--") {
                    // Comment, skip to -->
                    if let Some(end) = remainder.find("-->") {
                        cursor = start + end + 3;
                        continue;
                    } else {
                        break; // EOF inside comment
                    }
                }
                
                if remainder.starts_with("<?") {
                    // PI, skip to ?>
                    if let Some(end) = remainder.find("?>") {
                        cursor = start + end + 2;
                        continue;
                    } else {
                         break;
                    }
                }
                
                if remainder.starts_with("<!") {
                    // Some other declaration? 
                    // CDATA (<![CDATA[) shouldn't be in prolog.
                    // Unknown DTD types? 
                    // To be safe/strict in the prolog, we should reject unknown text starting with <!.
                    let lower_rem = remainder.get(..20).unwrap_or(remainder).to_ascii_lowercase(); // check next 20 chars
                    if lower_rem.starts_with("<!doctype") || lower_rem.starts_with("<!entity") {
                        return Err(anyhow!("XML DOCTYPE/ENTITY declarations are not supported"));
                    } else {
                        return Err(anyhow!("Found unknown or malformed XML declaration in prolog"));
                    }
                }
                
                // If we reach here, it's a '<' that isn't a comment or PI or DTD.
                // Check if it looks like a valid start tag to be the root element.
                // Valid start chars: : or [a-zA-Z] or _ or [xC0-xD6] ... (Unicode)
                // We peek the char after '<'
                if let Some(next_char) = remainder.chars().nth(1) {
                    // Use is_alphabetic() to support Unicode start characters
                    if next_char.is_alphabetic() || next_char == '_' || next_char == ':' {
                        // Found root element start. Stop scanning.
                        break;
                    }
                    // If it's not a start char (e.g. <1 or < ), it's technically malformed XML 
                    // but not a DTD injection vector. We continue scanning to be safe 
                    // in case the real DOCTYPE is hiding later?
                    // Or we stop? 
                    // If we continue, we might find a malicious DOCTYPE later.
                    // So we continue.
                } else {
                     break; // EOF
                }
            }
             
            quick_xml::de::from_str(content).map_err(Into::into)
        },
        "json5" => json5::from_str(content).map_err(Into::into),
        "kdl" => {
            let doc: kdl::KdlDocument = content.parse()?;
            kdl_to_json_value(&doc)
        },
        _ => Err(anyhow!("Unsupported file extension: {}", extension)),
    }
}

fn load_recursive_inner(input: ConfigInput, parent_vars: &HashMap<String, String>, visited: &mut Vec<PathBuf>, current_depth: usize, processor: &Processor) -> Result<Value> {
    let mut value = match input {
        ConfigInput::File(path) => {
            let content = fs::read_to_string(path)
                .with_context(|| format!("Failed to read file: {:?}", path))?;

            let extension = path.extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();

            parse_content(&content, &extension)
                .with_context(|| format!("Error parsing file: {:?}", path))?
        }
        ConfigInput::Raw { content, extension, .. } => {
            parse_content(content, &extension.to_lowercase())
                .with_context(|| format!("Error parsing raw content (ext: {})", extension))?
        }
        ConfigInput::Value { ref value, .. } => value.clone(),
    };

    // 1. Extract and Process Substitutions
    let mut current_vars = parent_vars.clone();
    
    // We need to check if the root is an object to have "substitutions"
    if let Value::Object(ref mut map) = value
        && let Some(sub_val) = map.remove(SUBSTITUTIONS_KEY) {
            match sub_val {
                Value::String(ref s) => {
                    // Load substitutions from file
                    let base_path = input.path().unwrap_or(Path::new("."));
                    let sub_path = base_path.parent().unwrap_or(Path::new(".")).join(s);
                    let sub_vars = load_substitutions_from_file(&sub_path, &current_vars, visited, current_depth + 1, processor)?;
                    current_vars.extend(sub_vars);
                }
                Value::Object(m) => {
                    // Inline map
                    for (k, v) in m {
                        if let Value::String(vs) = v {
                            current_vars.insert(k, vs);
                        } else if let Value::Number(vn) = v {
                            current_vars.insert(k, vn.to_string());
                        } else if let Value::Bool(vb) = v {
                            current_vars.insert(k, vb.to_string());
                        }
                    }
                }
                _ => {
                    return Err(anyhow!("Invalid 'substitutions' format in config. Expected string (path) or object (map)."));
                }
            }
        }

    // Resolve references between substitution variables (bounded to avoid cycles).
    // Note: We intentionally disable environment variable lookup here (`allow_env = false`)
    // to prevents env values from "bleeding" into internal alias definitions.
    // Env vars are only resolved during the final value substitution pass.
    resolve_vars(&mut current_vars, false)?;

    // 2. Perform Variable Substitution on the entire structure
    substitute_value(&mut value, &current_vars, processor.use_env_vars);

    // 3. Process Includes
    let base_path = input.path().unwrap_or(Path::new("."));
    process_includes(&mut value, base_path, &current_vars, visited, current_depth, processor)
        .with_context(|| format!("Error processing includes in config"))?;

    // 4. Final Security Check (Unresolved Tokens)
    // Ensure no placeholders remain in the verified output.
    scan_for_unresolved_tokens(&value)?;

    Ok(value)
}

fn load_substitutions_from_file(path: &Path, parent_vars: &HashMap<String, String>, visited: &mut Vec<PathBuf>, current_depth: usize, processor: &Processor) -> Result<HashMap<String, String>> {
    // We pass visited to prevent cycles in substitution files too
    let val = load_recursive(ConfigInput::File(path), parent_vars, visited, current_depth, processor)?; 
    
    let mut vars = HashMap::new();
    if let Value::Object(map) = val {
        for (k, v) in map {
             if let Value::String(vs) = v {
                vars.insert(k, vs);
            } else if let Value::Number(vn) = v {
                vars.insert(k, vn.to_string());
            } else if let Value::Bool(vb) = v {
                vars.insert(k, vb.to_string());
            }
        }
    }
    Ok(vars)
}

fn resolve_vars(vars: &mut HashMap<String, String>, allow_env: bool) -> Result<()> {
    if vars.is_empty() {
        return Ok(());
    }

    let mut changed = true;
    let max_iters = vars.len().saturating_add(1);
    for _ in 0..max_iters {
        if !changed {
            break;
        }
        changed = false;
        // SORT KEYS for deterministic resolution order
        let mut keys: Vec<String> = vars.keys().cloned().collect();
        keys.sort();
        
        for key in keys {
            if let Some(val) = vars.get(&key).cloned() {
                let resolved = substitutor::substitute(&val, vars, allow_env);
                if resolved != val {
                    vars.insert(key, resolved);
                    changed = true;
                }
            }
        }
    }
    
    // Check for unresolved variables or cycles
    // If we hit max_iters and things were still changing, or if placeholders remain
    if changed {
         return Err(anyhow!("Circular dependency detected in variable substitution"));
    }
    
    // Final check for remaining placeholders
    for (k, v) in vars.iter() {
        if substitutor::has_unresolved_tokens(v) {
             return Err(anyhow!("Unresolved variable in '{}': {}", k, v));
        }
    }
    Ok(())
}

fn substitute_value(value: &mut Value, vars: &HashMap<String, String>, allow_env: bool) {
    match value {
        Value::String(s) => {
            *s = substitutor::substitute(s, vars, allow_env);
        }
        Value::Array(arr) => {
            for v in arr {
                substitute_value(v, vars, allow_env);
            }
        }
        Value::Object(map) => {
            for (_, v) in map {
                substitute_value(v, vars, allow_env);
            }
        }
        _ => {}
    }
}

fn process_includes(value: &mut Value, base_path: &Path, vars: &HashMap<String, String>, visited: &mut Vec<PathBuf>, current_depth: usize, processor: &Processor) -> Result<()> {
    match value {
        Value::Object(map) => {
            // Check for include OR merge OR mergerecursive key
            // Start with mergerecursive (longest specific), then merge/include
            
            let mut include_target = None;
            let mut is_recursive = false;
            
            if let Some(val) = map.remove(MERGE_RECURSIVE_KEY) {
                include_target = Some(val);
                is_recursive = true;
            } else if let Some(val) = map.remove(INCLUDE_KEY) {
                include_target = Some(val);
            } else if let Some(val) = map.remove(MERGE_KEY) {
                include_target = Some(val);
            }
            
            if let Some(path_val) = include_target {
                if let Value::String(path_str) = path_val {
                    let mut included_val = resolve_include(base_path, &path_str, vars, visited, current_depth, processor, is_recursive)?;
                    
                    // Handle scalar replacement if needed
                    if !included_val.is_object() && included_val != Value::Null {
                        if map.is_empty() {
                            *value = included_val;
                            return Ok(());
                        } else {
                            return Err(anyhow!("Included content is not an object, cannot merge into object with existing keys"));
                        }
                    }

                    merge_overlay_into_base(map, &mut included_val, base_path, vars, visited, current_depth, processor)?;
                    return Ok(());

                } else if let Value::Array(paths) = path_val {
                    let mut combined_base = Value::Null;
                    for p in paths {
                        if let Value::String(path_str) = p {
                            // Recursion inheritance? If mergerecursive is array, apply recursive to all
                            let val = resolve_include(base_path, &path_str, vars, visited, current_depth, processor, is_recursive)?;
                            if combined_base == Value::Null {
                                combined_base = val;
                            } else {
                                smart_merge::smart_merge_values(&mut combined_base, val);
                            }
                        }
                    }
                    
                    // Handle scalar replacement if combined result is scalar
                    if !combined_base.is_object() && combined_base != Value::Null {
                         if map.is_empty() {
                            *value = combined_base;
                            return Ok(());
                        } else {
                            return Err(anyhow!("Combined included content is not an object, cannot merge into object with existing keys"));
                        }
                    }

                    merge_overlay_into_base(map, &mut combined_base, base_path, vars, visited, current_depth, processor)?;
                    return Ok(());
                }
            }
            
            // Recurse for children (only if no include was found/processed above)
            for (_, v) in map {
                process_includes(v, base_path, vars, visited, current_depth, processor)?;
            }
        }
        Value::Array(arr) => {
             for v in arr {
                 process_includes(v, base_path, vars, visited, current_depth, processor)?;
             }
        }
        _ => {}
    }
    Ok(())
}

fn resolve_include(base_path: &Path, path_str: &str, vars: &HashMap<String, String>, visited: &mut Vec<PathBuf>, current_depth: usize, processor: &Processor, recursive: bool) -> Result<Value> {
    let include_path = base_path.parent().unwrap_or(Path::new(".")).join(path_str);
    
    if include_path.is_dir() {
        // Directory merging logic
        // Use recursive walker if recursive is true
        
        let mut files_to_process = Vec::new();
        
        if recursive {
            let mut stack = vec![include_path.clone()];
            while let Some(path) = stack.pop() {
                 if path.is_file() {
                     files_to_process.push(path);
                 } else if path.is_dir() {
                     // Security Check: Root Dir before reading subtree
                     let canonical_dir = fs::canonicalize(&path)
                        .with_context(|| format!("Failed to canonicalize directory: {:?}", path))?;

                     if let Some(ref root) = processor.root_dir
                         && !canonical_dir.starts_with(root) {
                              return Err(anyhow!("Security violation: Access denied to list {:?} (outside root {:?})", canonical_dir, root));
                         }

                     let entries = fs::read_dir(&canonical_dir)
                        .with_context(|| format!("Failed to read directory: {:?}", canonical_dir))?;
                     for entry in entries {
                        let entry = entry.with_context(|| format!("Failed to read directory entry in {:?}", canonical_dir))?;
                        stack.push(entry.path());
                     }
                 }
            }
        } else {
            // Flat (only immediate keys)
             let canonical_dir = fs::canonicalize(&include_path)
                .with_context(|| format!("Failed to canonicalize directory: {:?}", include_path))?;

             if let Some(ref root) = processor.root_dir
                 && !canonical_dir.starts_with(root) {
                      return Err(anyhow!("Security violation: Access denied to list {:?} (outside root {:?})", canonical_dir, root));
                 }
             
             let entries = fs::read_dir(&canonical_dir)
                .with_context(|| format!("Failed to read directory: {:?}", canonical_dir))?;
             for entry in entries {
                 let entry = entry.with_context(|| format!("Failed to read directory entry in {:?}", canonical_dir))?;
                 let p = entry.path();
                 if p.is_file() {
                     files_to_process.push(p);
                 }
             }
        }

        files_to_process.sort(); // Deterministic order

        let mut combined_base = Value::Null;

        for entry in files_to_process {
            // Check extension
            let ext = entry.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            match ext.as_str() {
                "json" | "yaml" | "yml" | "toml" | "xml" | "json5" | "kdl" => {
                    match load_recursive(ConfigInput::File(&entry), vars, visited, current_depth + 1, processor) {
                        Ok(val) => {
                            if combined_base == Value::Null {
                                combined_base = val;
                            } else {
                                smart_merge::smart_merge_values(&mut combined_base, val);
                            }
                        }
                        Err(e) => {
                            if processor.strict_dir_includes {
                                return Err(anyhow!("Failed to process included file {:?}: {}", entry, e));
                            }
                            error!("Failed to process included file {:?}: {}. Skipping.", entry, e);
                            // Continue to next file
                        }
                    }
                }
                _ => {} // Skip unknown
            }
        }
        
        Ok(combined_base)
        
    } else {
        // Single file logic
        load_recursive(ConfigInput::File(&include_path), vars, visited, current_depth + 1, processor)
    }
}

fn scan_for_unresolved_tokens(value: &Value) -> Result<()> {
    match value {
        Value::String(s) => {
             if substitutor::has_unresolved_tokens(s) {
                 return Err(anyhow!("Found unresolved variable placeholder in output: {}", s));
             }
        },
        Value::Array(arr) => {
            for v in arr {
                scan_for_unresolved_tokens(v)?;
            }
        },
        Value::Object(map) => {
            for (_, v) in map {
                 scan_for_unresolved_tokens(v)?;
            }
        },
        _ => {}
    }
    Ok(())
}

fn merge_overlay_into_base(overlay_map: &mut Map<String, Value>, base_val: &mut Value, base_path: &Path, vars: &HashMap<String, String>, visited: &mut Vec<PathBuf>, current_depth: usize, processor: &Processor) -> Result<()> {
    // Use exported smart_merge functions
    use crate::smart_merge;
    
    if let Value::Object(base_map) = base_val {
        
        let mut overlay_val = Value::Object(std::mem::take(overlay_map));
        process_includes(&mut overlay_val, base_path, vars, visited, current_depth, processor)?;
        
        // Merge Overlay into Base
        if let Value::Object(overlay_map_processed) = overlay_val {
            smart_merge::smart_merge_objects(base_map, overlay_map_processed);
            *overlay_map = std::mem::take(base_map); // Result is the merged object put back into 'map'
        }
    } else if *base_val == Value::Null {
            // Included nothing (e.g. empty dir).
            let mut overlay_val = Value::Object(std::mem::take(overlay_map));
            process_includes(&mut overlay_val, base_path, vars, visited, current_depth, processor)?;
            if let Value::Object(overlay_map_processed) = overlay_val {
                *overlay_map = overlay_map_processed;
            }
    } else if overlay_map.is_empty() {
         return Err(anyhow!("Included content is not an object, cannot merge into object with existing keys. (Scalar replacement not supported in this helper flow)"));
    } else {
         return Err(anyhow!("Included content is not an object, cannot merge into object with existing keys"));
    }
    Ok(())
}

fn kdl_to_json_value(doc: &kdl::KdlDocument) -> Result<Value> {
    let mut map = Map::new();
    for node in doc.nodes() {
        let key = node.name().value().to_string();
        // KDL node mapping logic:
        // 1. If node has children, it's an object: key { ... } -> "key": { ... }
        // 2. If node has arguments:
        //    - One argument: key "val" -> "key": "val"
        //    - Multiple arguments: key "v1" "v2" -> "key": ["v1", "v2"]
        // 3. Properties: key prop="val" -> "key": { "prop": "val" } (Merged with children?)
        // Simple heuristic:
        // - If children exist, it's a map.
        // - If args exist, they might be the value (if no children).
        
        let value = if let Some(children) = node.children() {
            kdl_to_json_value(children)?
        } else if !node.entries().is_empty() {
            // Process entries (arguments and properties)
            // If only positional args:
            let args: Vec<&kdl::KdlEntry> = node.entries().iter().filter(|e| e.name().is_none()).collect();
            let props: Vec<&kdl::KdlEntry> = node.entries().iter().filter(|e| e.name().is_some()).collect();
            
            if !props.is_empty() {
                // Treat as object with properties
                let mut p_map = Map::new();
                for prop in props {
                    let name = prop.name().map(|id| id.value()).unwrap_or("").to_string();
                    let v = kdl_entry_to_value(prop.value());
                    p_map.insert(name, v);
                }
                // Determine what to do with args if mixed? Ignore or use "_" key?
                if !args.is_empty() {
                     let arg_vals: Vec<Value> = args.iter().map(|e| kdl_entry_to_value(e.value())).collect();
                     p_map.insert("_args".to_string(), Value::Array(arg_vals));
                }
                Value::Object(p_map)
            } else if args.len() == 1 {
                kdl_entry_to_value(args[0].value())
            } else {
                let arg_vals: Vec<Value> = args.iter().map(|e| kdl_entry_to_value(e.value())).collect();
                Value::Array(arg_vals)
            }
        } else {
            Value::Null
        };
        
        map.insert(key, value);
    }
    Ok(Value::Object(map))
}

fn kdl_entry_to_value(val: &kdl::KdlValue) -> Value {
    match val {
        kdl::KdlValue::String(s) => Value::String(s.clone()),
        kdl::KdlValue::Integer(i) => {
            // i is i128. serde_json::Number supports up to u64/i64.
            // Try explicit casting. If it fits in i64, nice.
            // If i128 is too large, it might be an issue.
            // For config, i64 is usually sufficient.
            let v = i64::try_from(*i).ok();
            if let Some(val) = v {
                 Value::Number(val.into())
            } else {
                 // Fallback to string or null? Or f64?
                 // KDL allows very large ints.
                 Value::String(i.to_string())
            }
        },
        kdl::KdlValue::Float(f) => serde_json::Number::from_f64(*f).map(Value::Number).unwrap_or(Value::Null),
        kdl::KdlValue::Bool(b) => Value::Bool(*b),
        kdl::KdlValue::Null => Value::Null,
        // All variants covered, no need for catch-all
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_json() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, r#"{{"key": "value"}}"#).unwrap();
        let _path = file.path().to_path_buf().with_extension("json"); // NamedTempFile usually has random extension or none. 
        // We need to persist it with correct extension or rename? 
        // tempfile::Builder allows suffix.
        let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(file, r#"{{"key": "value"}}"#).unwrap();
        
        let val = process_file(file.path(), 5).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn test_load_yaml() {
        let mut file = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        write!(file, "key: value").unwrap();
        
        let val = process_file(file.path(), 5).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn test_load_xml() {
        let mut file = tempfile::Builder::new().suffix(".xml").tempfile().unwrap();
        write!(file, r#"<root><key>value</key></root>"#).unwrap();
        // quick-xml deserializes text content of a node as $text if it's mixed or standard behavior
        // Based on failure: Object {"$text": String("value")}
        let val = process_file(file.path(), 5).unwrap();
        assert_eq!(val["key"]["$text"], "value");
    }

    #[test]
    fn test_load_json5() {
        let mut file = tempfile::Builder::new().suffix(".json5").tempfile().unwrap();
        write!(file, r#"{{
            // Comment
            key: 'value',
        }}"#).unwrap();
        
        let val = process_file(file.path(), 5).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn test_load_kdl() {
        let mut file = tempfile::Builder::new().suffix(".kdl").tempfile().unwrap();
        write!(file, r#"
            key "value"
            section {{
                inner "data"
            }}
        "#).unwrap();
        
        let val = process_file(file.path(), 5).unwrap();
        assert_eq!(val["key"], "value");
        assert_eq!(val["section"]["inner"], "data");
    }

    #[test]
    fn test_substitution_inline() {
        let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        file.write_all(r#"{
            "substitutions": { "VAR": "World" },
            "greeting": "Hello ${{VAR}}"
        }"#.as_bytes()).unwrap();
        
        let val = process_file(file.path(), 5).unwrap();
        assert_eq!(val["greeting"], "Hello World");
    }

    #[test]
    fn test_include_merge() {
        // Included file
        let mut inc_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(inc_file, r#"{{"included": "true", "shared": "included_value"}}"#).unwrap();
        let inc_path = inc_file.path().file_name().unwrap().to_str().unwrap();

        // Main file
        let mut main_file = tempfile::Builder::new().suffix(".json").tempfile_in(inc_file.path().parent().unwrap()).unwrap();
        write!(main_file, r#"{{
            "include": "{}",
            "main": "true",
            "shared": "main_value"
        }}"#, inc_path).unwrap();

        let val = process_file(main_file.path(), 5).unwrap();
        assert_eq!(val["included"], "true");
        assert_eq!(val["main"], "true");
        // Main should override included
        assert_eq!(val["shared"], "main_value"); 
    }

    #[test]
    fn test_substitution_from_file() {
         // Vars file
        let mut vars_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        vars_file.write_all(r#"{ "VAR": "FileWorld" }"#.as_bytes()).unwrap();
        let vars_name = vars_file.path().file_name().unwrap().to_str().unwrap();

        // Main file
        let mut main_file = tempfile::Builder::new().suffix(".json").tempfile_in(vars_file.path().parent().unwrap()).unwrap();
        write!(main_file, r#"{{
            "substitutions": "{}",
            "greeting": "Hello ${{{{VAR}}}}"
        }}"#, vars_name).unwrap();

        let val = process_file(main_file.path(), 5).unwrap();
        assert_eq!(val["greeting"], "Hello FileWorld");
    }

    #[test]
    fn test_circular_dependency() {
        let mut file_a = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        let mut file_b = tempfile::Builder::new().suffix(".json").tempfile_in(file_a.path().parent().unwrap()).unwrap();
        
        let path_a = file_a.path().file_name().unwrap().to_str().unwrap().to_string();
        let path_b = file_b.path().file_name().unwrap().to_str().unwrap().to_string();

        write!(file_a, r#"{{"include": "{}"}}"#, path_b).unwrap();
        write!(file_b, r#"{{"include": "{}"}}"#, path_a).unwrap();

        let res = process_file(file_a.path(), 5);
        assert!(res.is_err());
        let err = res.err().unwrap();
        // The error is wrapped: "Error processing includes in file...: Circular dependency..."
        println!("Circular Dependency Error: {:?}", err);
        assert!(format!("{:?}", err).contains("Circular dependency detected"));
    }

    #[test]
    fn test_max_depth() {
        // Create chain a -> b -> c
        let mut file_a = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        let mut file_b = tempfile::Builder::new().suffix(".json").tempfile_in(file_a.path().parent().unwrap()).unwrap();
        let mut file_c = tempfile::Builder::new().suffix(".json").tempfile_in(file_a.path().parent().unwrap()).unwrap();
        
        let path_b = file_b.path().file_name().unwrap().to_str().unwrap().to_string();
        let path_c = file_c.path().file_name().unwrap().to_str().unwrap().to_string();

        write!(file_a, r#"{{"include": "{}"}}"#, path_b).unwrap();
        write!(file_b, r#"{{"include": "{}"}}"#, path_c).unwrap();
        write!(file_c, r#"{{"val": "leaf"}}"#).unwrap();

        // Max depth 1: A (0) -> B (1) -> C (2) should fail if limit is 1?
        // 0 <= 1 OK. 1 <= 1 OK. 2 > 1 Fail?
        // check logic: if current_depth > max_depth.
        // A=0. B=1. C=2.
        
        // Test depth 1
        let res = process_file(file_a.path(), 1);
        assert!(res.is_err());
        // Exact message depends on recursion logic. Current impl: 
        // 0 (A) -> 1 (B) -> 2 (C) -- if max=1.
        // C check: if 1 > 1 (false). hmm.
        // Wait, "load_recursive" (A) depth=0. 
        // Calls "process_includes" -> "process_file_recursive(B, depth=1)".
        // B depth=1. max=1. 1 > 1 is false.
        // B calls C, depth=2. 2 > 1 is true. Error.
        
        let err_str = format!("{:?}", res.err().unwrap());
        assert!(err_str.contains("Recursion depth limit reached") || err_str.contains("Error processing includes"));
        
        // Test depth 2 (should pass)
        let res2 = process_file(file_a.path(), 2);
        assert!(res2.is_ok());
        
        // Test depth 0 (no limit)
        let res3 = process_file(file_a.path(), 0);
        assert!(res3.is_ok());
    }

    #[test]
    fn test_malformed_files() {
        let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(file, r#"{{"key": "value""#).unwrap(); // Missing closing brace
        let res = process_file(file.path(), 5);
        assert!(res.is_err());

        let mut file_xml = tempfile::Builder::new().suffix(".xml").tempfile().unwrap();
        write!(file_xml, r#"<root><key>val</root>"#).unwrap(); // Mismatched tag
        let res_xml = process_file(file_xml.path(), 5);
        assert!(res_xml.is_err());
    }

    #[test]
    fn test_missing_files() {
        let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(file, r#"{{"include": "non_existent.json"}}"#).unwrap();
        let res = process_file(file.path(), 5);
        assert!(res.is_err());
        let err = res.err().unwrap();
        let err_msg = format!("{:?}", err);
        // The include fails because canonicalize fails for missing file
        // OR it fails to read. 
        // Context added: "Error processing includes in file ..."
        // Inner context: "Failed to canonicalize path" (from load_recursive)
        println!("Missing Files Error: {}", err_msg);
        assert!(err_msg.contains("Failed to canonicalize path") || err_msg.contains("No such file") || err_msg.contains("cannot find the file"));
    }

    #[test]
    fn test_mixed_formats() {
        // JSON including YAML
        let mut file_json = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        let mut file_yaml = tempfile::Builder::new().suffix(".yaml").tempfile_in(file_json.path().parent().unwrap()).unwrap();
        
        write!(file_yaml, "yaml_key: yaml_value").unwrap();
        let yaml_name = file_yaml.path().file_name().unwrap().to_str().unwrap();

        write!(file_json, r#"{{"include": "{}", "json_key": "json_value"}}"#, yaml_name).unwrap();

        let val = process_file(file_json.path(), 5).unwrap();
        assert_eq!(val["yaml_key"], "yaml_value");
        assert_eq!(val["json_key"], "json_value");
    }

    #[test]
    fn test_complex_substitution() {
        let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        file.write_all(r#"{
            "substitutions": {
                "A": "PartA",
                "B": "PartB",
                "COMBINED": "Val${{A}}_${{B}}"
            },
            "result": "${{COMBINED}}",
            "nested": {
                "sub": "Deep ${{A}}"
            },
            "list": ["Item ${{B}}"]
        }"#.as_bytes()).unwrap();
        
        let val = process_file(file.path(), 5).unwrap();
        assert_eq!(val["nested"]["sub"], "Deep PartA");
        assert_eq!(val["list"][0], "Item PartB");
    }

    #[test]
    fn test_kdl_edge_cases() {
        let mut file = tempfile::Builder::new().suffix(".kdl").tempfile().unwrap();
        write!(file, r#"
            node "arg1" "arg2" prop="val" {{
                child "child_val"
            }}
            mixed_args "pos" key="val"
        "#).unwrap();
        
        let val = process_file(file.path(), 5).unwrap();
        
        // Check node structure (children override args/props in current impl)
        assert!(val.get("node").is_some()); 
        
        // Check mixed args/props preservation
        assert_eq!(val["mixed_args"]["key"], "val");
        assert_eq!(val["mixed_args"]["_args"][0], "pos");
    }

    #[test]
    fn test_empty_file() {
        let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(file, "").unwrap(); 
        let res = process_file(file.path(), 5);
        assert!(res.is_err());
    }
    #[test]
    fn test_merge_alias() {
        // Included file
        let mut inc_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(inc_file, r#"{{"included": "true"}}"#).unwrap();
        let inc_path = inc_file.path().file_name().unwrap().to_str().unwrap();

        // Main file using "merge" instead of "include"
        let mut main_file = tempfile::Builder::new().suffix(".json").tempfile_in(inc_file.path().parent().unwrap()).unwrap();
        write!(main_file, r#"{{"merge": "{}"}}"#, inc_path).unwrap();

        let val = process_file(main_file.path(), 5).unwrap();
        assert_eq!(val["included"], "true");
    }

    #[test]
    fn test_smart_list_merge() {
        // Base file with list of objects with IDs
        let mut base_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(base_file, r#"{{
            "items": [
                {{ "id": "1", "val": "base1" }},
                {{ "id": "2", "val": "base2" }}
            ]
        }}"#).unwrap();
        let base_path = base_file.path().file_name().unwrap().to_str().unwrap();

        // Overlay file merging base and overriding item 1, adding item 3
        let mut overlay_file = tempfile::Builder::new().suffix(".json").tempfile_in(base_file.path().parent().unwrap()).unwrap();
        write!(overlay_file, r#"{{
            "merge": "{}",
            "items": [
                {{ "id": "1", "val": "overlay1" }},
                {{ "val": "new" }}
            ]
        }}"#, base_path).unwrap();

        let val = process_file(overlay_file.path(), 5).unwrap();
        
        let items = val["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        
        // Item 1 should be merged (overlay overrides base)
        assert_eq!(items[0]["id"], "1");
        assert_eq!(items[0]["val"], "overlay1");
        
        // Item 2 should remain
        assert_eq!(items[1]["id"], "2");
        assert_eq!(items[1]["val"], "base2");
        
        // Item 3 should be appended
        assert_eq!(items[2]["val"], "new");
    }

    #[test]
    fn test_merge_priority() {
        // Grandchild A (Base)
        let mut file_a = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(file_a, r#"{{"id": "1", "val": "A", "other": "A"}}"#).unwrap();
        let path_a = file_a.path().file_name().unwrap().to_str().unwrap();

        // Grandchild B (Overlay)
        let mut file_b = tempfile::Builder::new().suffix(".json").tempfile_in(file_a.path().parent().unwrap()).unwrap();
        write!(file_b, r#"{{"val": "B"}}"#).unwrap(); // B overrides val
        let path_b = file_b.path().file_name().unwrap().to_str().unwrap();

        // Parent (Defines list with A)
        let mut file_parent = tempfile::Builder::new().suffix(".json").tempfile_in(file_a.path().parent().unwrap()).unwrap();
        write!(file_parent, r#"{{
            "items": [
                {{ "include": "{}" }} 
            ]
        }}"#, path_a).unwrap();
        let path_parent = file_parent.path().file_name().unwrap().to_str().unwrap();

        // Main (Merges Parent, Overrides item with B)
        let mut file_main = tempfile::Builder::new().suffix(".json").tempfile_in(file_a.path().parent().unwrap()).unwrap();
        // Here, the Main file wants to override the item with ID 1.
        // It does so by providing an item with ID 1 that INCLUDES B.
        // Expectation: Result has ID 1, val B (from Overlay/B), other A (from Base/A).
        write!(file_main, r#"{{
            "merge": "{}",
            "items": [
                {{ "id": "1", "include": "{}" }}
            ]
        }}"#, path_parent, path_b).unwrap();

        let val = process_file(file_main.path(), 5).unwrap();
        let item = &val["items"][0];
        
        // Debug output if fails
        // println!("Merged Item: {:?}", item);

        assert_eq!(item["id"], "1");
        assert_eq!(item["other"], "A"); // Preserved from Base
        assert_eq!(item["val"], "B");   // Overridden by Overlay (which included B)
    }

    #[test]
    fn test_directory_merge() {
        // Create a directory
        let dir = tempfile::Builder::new().prefix("test_dir").tempdir().unwrap();
        let dir_path = dir.path();

        // Create file A: { "list": [{"id":1, "v":"A"}], "base": "A" }
        let file_a = dir_path.join("a.json");
        fs::write(&file_a, r#"{
            "list": [{"id":"1", "v":"A"}],
            "base": "A"
        }"#).unwrap();

        // Create file B: { "list": [{"id":1, "v":"B"}, {"id":2, "v":"B"}], "overlay": "B" }
        // B comes after A, so it should override id:1 with v:B
        let file_b = dir_path.join("b.json");
        fs::write(&file_b, r#"{
            "list": [{"id":"1", "v":"B"}, {"id":"2", "v":"B"}],
            "overlay": "B"
        }"#).unwrap();

        // Main file including the directory
        let mut main_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        // Point to the directory
        write!(main_file, r#"{{"merge": "{}"}}"#, dir_path.to_str().unwrap()).unwrap();

        let val = process_file(main_file.path(), 5).unwrap();
        
        // Assertions
        assert_eq!(val["base"], "A");
        assert_eq!(val["overlay"], "B");
        
        let list = val["list"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        
        // Item 1: ID 1 should be B
        let item1 = list.iter().find(|i| i["id"] == "1").unwrap();
        assert_eq!(item1["v"], "B");

        // Item 2: ID 2 should be B
        let item2 = list.iter().find(|i| i["id"] == "2").unwrap();
        assert_eq!(item2["v"], "B");
    }

    #[test]
    fn test_directory_merge_absolute() {
        let dir = tempfile::Builder::new().prefix("test_dir_abs").tempdir().unwrap();
        let dir_path = dir.path().canonicalize().unwrap(); // ensure absolute
        
        // Create file inside
        fs::write(dir_path.join("a.json"), r#"{"a":1}"#).unwrap();

        let mut main_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        // Use the absolute path as string
        // Note: tempfile paths are usually absolute, but we force verify.
        write!(main_file, r#"{{"merge": "{}"}}"#, dir_path.to_str().unwrap()).unwrap();
        
        let val = process_file(main_file.path(), 5).unwrap();
        assert_eq!(val["a"], 1);
    }
    #[test]
    fn test_read_raw_file() {
        let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        let content = r#"{
            "include": "other.json",
            "val": "raw_value",
            "sub": "${{VAR}}"
        }"#;
        write!(file, "{}", content).unwrap();
        
        // read_raw_file should return the raw string content
        let val = read_raw_file(file.path()).unwrap();
        
        assert!(val.contains(r#""val": "raw_value""#));
        assert!(val.contains(r#""include": "other.json""#));
        assert!(val.contains(r#""sub": "${{VAR}}""#));
    }

    #[test]
    fn test_merge_recursive_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        
        let subdir = root.join("subdir");
        fs::create_dir(&subdir).unwrap();
        
        fs::write(subdir.join("a.yaml"), "a: 1").unwrap();
        fs::write(subdir.join("b.yaml"), "b: 2").unwrap();
        
        let main_file = root.join("main.yaml");
        fs::write(&main_file, "merge_recursive: subdir").unwrap();
        
        let val = process_file(&main_file, 5).unwrap();
        assert_eq!(val["a"], 1);
        assert_eq!(val["b"], 2);
    }
}
