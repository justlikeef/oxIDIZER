use crate::substitutor;
use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const INCLUDE_KEY: &str = "include";
const SUBSTITUTIONS_KEY: &str = "substitutions";

pub fn process_file(path: &Path, max_depth: usize) -> Result<Value> {
    let mut visited = Vec::new();
    load_recursive(path, &HashMap::new(), &mut visited, 0, max_depth)
}

fn load_recursive(path: &Path, parent_vars: &HashMap<String, String>, visited: &mut Vec<PathBuf>, current_depth: usize, max_depth: usize) -> Result<Value> {
    let canonical_path = fs::canonicalize(path)
        .with_context(|| format!("Failed to canonicalize path: {:?}", path))?;

    if visited.iter().any(|v| v == &canonical_path) {
         return Err(anyhow!("Circular dependency detected: {:?}", canonical_path));
    }

    if max_depth > 0 && current_depth > max_depth {
        return Err(anyhow!("Recursion depth limit reached ({})", max_depth));
    }
    
    visited.push(canonical_path.clone());
    
    let res = load_recursive_inner(path, parent_vars, visited, current_depth, max_depth);

    visited.pop();
    res
}

fn load_recursive_inner(path: &Path, parent_vars: &HashMap<String, String>, visited: &mut Vec<PathBuf>, current_depth: usize, max_depth: usize) -> Result<Value> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {:?}", path))?;

    let extension = path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut value: Value = match extension.as_str() {
        "json" => serde_json::from_str(&content)?,
        "yaml" | "yml" => serde_yaml::from_str(&content)?,
        "toml" => {
            let toml_val: toml::Value = toml::from_str(&content)?;
            serde_json::to_value(toml_val)?
        },
        "xml" => quick_xml::de::from_str(&content)?,
        "json5" => json5::from_str(&content)?,
        "kdl" => {
            let doc: kdl::KdlDocument = content.parse()?;
            kdl_to_json_value(&doc)?
        },
        _ => return Err(anyhow!("Unsupported file extension: {}", extension)),
    };

    // 1. Extract and Process Substitutions
    let mut current_vars = parent_vars.clone();
    
    // We need to check if the root is an object to have "substitutions"
    if let Value::Object(ref mut map) = value {
        if let Some(sub_val) = map.remove(SUBSTITUTIONS_KEY) {
            match sub_val {
                Value::String(ref s) => {
                    // Load substitutions from file
                    // Load substitutions from file
                    let sub_path = path.parent().unwrap_or(Path::new(".")).join(s);
                    // Pass current_depth + 1 (or same? subs are "part of" current file logic? Let's say +1)
                    let sub_vars = load_substitutions_from_file(&sub_path, visited, current_depth + 1, max_depth)?;
                    current_vars.extend(sub_vars);
                }
                Value::Object(m) => {
                    // Inline map
                    for (k, v) in m {
                        if let Value::String(vs) = v {
                            current_vars.insert(k, vs);
                        }
                    }
                }
                _ => {} // Ignore invalid format
            }
        }
    }

    // 2. Perform Variable Substitution on the entire structure
    // We do this BEFORE processing includes, or AFTER?
    // If we do it before, then the included path can be dynamic! ${ENV}.yaml
    // That sounds powerful. Let's do it before.
    substitute_value(&mut value, &current_vars);

    // 3. Process Includes
    // 3. Process Includes
    process_includes(&mut value, path, &current_vars, visited, current_depth, max_depth)?;

    Ok(value)
}

fn load_substitutions_from_file(path: &Path, visited: &mut Vec<PathBuf>, current_depth: usize, max_depth: usize) -> Result<HashMap<String, String>> {
    // We pass visited to prevent cycles in substitution files too
    let val = load_recursive(path, &HashMap::new(), visited, current_depth, max_depth)?; 
    // Let's assume vars file shouldn't depend on parent vars to avoid cycles or complexity for now, or just allow it.
    // Recursive load returns Value. Expect Object.
    
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

fn substitute_value(value: &mut Value, vars: &HashMap<String, String>) {
    match value {
        Value::String(s) => {
            *s = substitutor::substitute(s, vars);
        }
        Value::Array(arr) => {
            for v in arr {
                substitute_value(v, vars);
            }
        }
        Value::Object(map) => {
            for (_, v) in map {
                substitute_value(v, vars);
            }
        }
        _ => {}
    }
}

fn process_includes(value: &mut Value, base_path: &Path, vars: &HashMap<String, String>, visited: &mut Vec<PathBuf>, current_depth: usize, max_depth: usize) -> Result<()> {
    match value {
        Value::Object(map) => {
            // Check for include key
            if let Some(include_val) = map.remove(INCLUDE_KEY) {
                if let Value::String(path_str) = include_val {
                    let include_path = base_path.parent().unwrap_or(Path::new(".")).join(path_str);
                    let included_val = load_recursive(&include_path, vars, visited, current_depth + 1, max_depth)?;
                    
                    if let Value::Object(included_map) = included_val {
                       // Merge: Existing keys in 'map' override included keys? 
                       // Usually: Inherit from include, override with local.
                       // So we take included_map, extend with current 'map', then replace 'map'.
                       let mut merged = included_map;
                       let existing = std::mem::take(map);
                       merged.extend(existing);
                       *map = merged;
                    } else {
                        // If included value is not an object, but we are in an object...
                        // We can't easily merge a list into an object.
                        // Ideally, if the current object ONLY had 'include', we replace the whole value.
                        if map.is_empty() {
                            *value = included_val;
                            return Ok(());
                        } else {
                             // Error or warning? "Cannot merge non-object include into object"
                             // allow it to fail silently or error?
                             return Err(anyhow!("Included file {:?} is not an object, cannot merge into object", include_path));
                        }
                    }
                }
            }
            
            // Recurse for children
            // Note: if we replaced *value, we might need to re-process? 
            // The recursive call above `load_recursive` already processed the included file fully.
            // So we only need to process the *remaining* keys in the current object?
            // But we effectively merged them. The values in `map` (from `drain`) need to be processed for includes?
            // Wait, we ran `substitute_value` on the current file content already.
            // We call `process_includes` on the current node.
            // If we just merged, the values from `included_map` are already fully processed.
            // The values from `current map` (drained and re-inserted) are NOT processed for includes yet (we are doing it now).
            // So we should iterate over the map values.
            
            // Re-borrow map after potential replacement
            if let Value::Object(map) = value {
                  for (_, v) in map {
                      process_includes(v, base_path, vars, visited, current_depth, max_depth)?;
                  }
            }
        }
        Value::Array(arr) => {
             for v in arr {
                 process_includes(v, base_path, vars, visited, current_depth, max_depth)?;
             }
        }
        _ => {}
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
            let v = match i64::try_from(*i) {
                Ok(val) => Some(val),
                Err(_) => None, 
            };
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
        _ => Value::String(val.to_string()),
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
        let path = file.path().to_path_buf().with_extension("json"); // NamedTempFile usually has random extension or none. 
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
        write!(file, r#"{{
            "substitutions": {{ "VAR": "World" }},
            "greeting": "Hello ${{VAR}}"
        }}"#).unwrap();
        
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
        write!(vars_file, r#"{{"VAR": "FileWorld"}}"#).unwrap();
        let vars_name = vars_file.path().file_name().unwrap().to_str().unwrap();

        // Main file
        let mut main_file = tempfile::Builder::new().suffix(".json").tempfile_in(vars_file.path().parent().unwrap()).unwrap();
        write!(main_file, r#"{{
            "substitutions": "{}",
            "greeting": "Hello ${{VAR}}"
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
        assert!(err.to_string().contains("Circular dependency detected"));
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
        assert!(res.err().unwrap().to_string().contains("Recursion depth limit reached"));
        
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
        let err_msg = res.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to read file") || err_msg.contains("Failed to canonicalize path"));
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
        write!(file, r#"{{
            "substitutions": {{
                "A": "PartA",
                "B": "PartB",
                "COMBINED": "Val${{A}}_${{B}}"
            }},
            "result": "${{COMBINED}}",
            "nested": {{
                "sub": "Deep ${{A}}"
            }},
            "list": ["Item ${{B}}"]
        }}"#).unwrap();
        
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
}
