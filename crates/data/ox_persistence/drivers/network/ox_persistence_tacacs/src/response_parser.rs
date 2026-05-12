//! Parses TACACS+ authorization response AV (Attribute-Value) pairs into canonical IAM maps.
//! TACACS+ AV pairs are defined in RFC 8907 and RFC 1492.

use std::collections::HashMap;
use ox_type_converter::ValueType;

pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// A single TACACS+ attribute-value pair from an Authorization-Response.
#[derive(Debug, Clone)]
pub struct TacacsAvPair {
    pub attribute: String,
    pub value: String,
}

fn s(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

/// Converts a list of TACACS+ AV pairs from an Authorization-Response into a canonical IAM map.
///
/// Mappings:
///   - `user` AV pair → `principal_id`, `display_name`
///   - `priv-lvl` AV pair → `group_id` formatted as `"priv-lvl-<N>"`
///   - `oxgroup` AV pair (custom) → `group_id` (overrides priv-lvl if present)
///   - `source` is hard-coded to "Tacacs"
///   - `tenant_id` is injected from the caller-supplied argument
///
/// The `priv-lvl` convention maps privilege levels (0-15) to group identifiers.
/// Custom AV pairs like `oxgroup=netops` can be used on TACACS+ servers that
/// support custom attributes to carry richer group information.
pub fn parse_av_pairs(pairs: &[TacacsAvPair], tenant_id: &str) -> CanonicalMap {
    let mut map = CanonicalMap::new();
    map.insert("source".to_string(),    s("Tacacs"));
    map.insert("tenant_id".to_string(), s(tenant_id));

    let mut has_explicit_group = false;

    for pair in pairs {
        match pair.attribute.as_str() {
            "user" => {
                map.insert("principal_id".to_string(), s(&pair.value));
                map.insert("display_name".to_string(), s(&pair.value));
            }
            "priv-lvl" => {
                if !has_explicit_group {
                    map.insert("group_id".to_string(), s(&format!("priv-lvl-{}", pair.value)));
                }
            }
            "oxgroup" => {
                // Custom AV pair carrying explicit group name — takes precedence over priv-lvl.
                map.insert("group_id".to_string(), s(&pair.value));
                has_explicit_group = true;
            }
            _ => {
                // Unknown AV pairs are ignored.
            }
        }
    }

    map
}
