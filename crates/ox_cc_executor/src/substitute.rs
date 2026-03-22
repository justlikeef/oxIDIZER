use std::collections::HashMap;
use serde_json::{Map, Value};
use anyhow::Result;

/// Substitute `$key` references in string param values from the state map.
///
/// **Depth:** Only top-level string values are substituted. Nested JSON objects
/// and arrays are passed through unchanged — `$variable` references inside them
/// are not resolved. Callers relying on nested substitution will silently receive
/// the unreplaced literal string.
///
/// Returns an error if a referenced key is not present in the state map.
pub fn substitute_params(
    params: &Map<String, Value>,
    state: &HashMap<String, Value>,
) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for (k, v) in params {
        out.insert(k.clone(), substitute_value(v, state)?);
    }
    Ok(out)
}

fn substitute_value(value: &Value, state: &HashMap<String, Value>) -> Result<Value> {
    match value {
        Value::String(s) if s.starts_with('$') => {
            let key = &s[1..];
            anyhow::ensure!(!key.is_empty(), "empty $variable reference");
            state.get(key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("$variable '{}' not found in state map", key))
        }
        // Nested objects and arrays are intentionally not traversed for substitution.
        // Only top-level string values undergo $variable substitution.
        other => Ok(other.clone()),
    }
}

/// Validate that all `$variable` references are syntactically well-formed
/// (non-empty key name after `$`). Does NOT check that the value is in the state map.
pub fn validate_syntax(params: &Map<String, Value>) -> Result<()> {
    // Nested objects and arrays are intentionally not checked for $variable syntax.
    // Only top-level string values are validated.
    for (_, v) in params {
        if let Value::String(s) = v {
            if s.starts_with('$') {
                anyhow::ensure!(s.len() > 1, "empty $variable reference in params");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn test_no_substitution() {
        let params: Map<String, Value> = serde_json::from_value(json!({"url": "https://example.com"})).unwrap();
        let result = substitute_params(&params, &state(&[])).unwrap();
        assert_eq!(result["url"], json!("https://example.com"));
    }

    #[test]
    fn test_substitution_from_state() {
        let params: Map<String, Value> = serde_json::from_value(json!({"path": "$dest"})).unwrap();
        let s = state(&[("dest", json!("/tmp/foo.deb"))]);
        let result = substitute_params(&params, &s).unwrap();
        assert_eq!(result["path"], json!("/tmp/foo.deb"));
    }

    #[test]
    fn test_missing_key_is_error() {
        let params: Map<String, Value> = serde_json::from_value(json!({"path": "$missing"})).unwrap();
        let err = substitute_params(&params, &state(&[])).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn test_non_string_values_untouched() {
        let params: Map<String, Value> = serde_json::from_value(json!({"count": 5, "flag": true})).unwrap();
        let result = substitute_params(&params, &state(&[])).unwrap();
        assert_eq!(result["count"], json!(5));
        assert_eq!(result["flag"], json!(true));
    }

    #[test]
    fn test_validate_syntax_ok() {
        let params: Map<String, Value> = serde_json::from_value(json!({"path": "$dest", "url": "https://x.com"})).unwrap();
        assert!(validate_syntax(&params).is_ok());
    }

    #[test]
    fn test_validate_syntax_empty_ref_is_error() {
        let params: Map<String, Value> = serde_json::from_value(json!({"bad": "$"})).unwrap();
        assert!(validate_syntax(&params).is_err());
    }

    #[test]
    fn test_substitution_into_non_string_from_state() {
        // State value is a number; it should be returned as-is
        let params: Map<String, Value> = serde_json::from_value(json!({"port": "$port_num"})).unwrap();
        let s = state(&[("port_num", json!(8080))]);
        let result = substitute_params(&params, &s).unwrap();
        assert_eq!(result["port"], json!(8080));
    }
}
