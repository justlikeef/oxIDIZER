//! Converts between the persistence layer's canonical serializable_map and
//! LDAP attribute lists.  This module is pure data transformation — no network I/O.

use std::collections::HashMap;
use ox_type_converter::ValueType;
use crate::mapping::SchemaMapping;

/// Canonical serializable map type (mirrors ox_persistence convention).
pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// LDAP attribute list: each entry is (attribute_name, Vec<value_string>).
pub type LdapAttrList = Vec<(String, Vec<String>)>;

/// Converts a canonical serializable_map into an LDAP attribute list for the
/// given `location` (e.g. "principals", "groups", "grants", "sessions").
/// Canonical field names are translated to LDAP attribute names via `mapping`.
pub fn ldap_attrs_from_canonical_map(
    map: &CanonicalMap,
    mapping: &SchemaMapping,
    location: &str,
) -> LdapAttrList {
    let mut attrs: LdapAttrList = Vec::new();
    for (canon_key, (value, _vtype, _meta)) in map {
        let ldap_attr = mapping.canonical_to_ldap(location, canon_key);
        // Skip internal oxid metadata key
        if ldap_attr == "__skip__" {
            continue;
        }
        attrs.push((ldap_attr, vec![value.clone()]));
    }
    // Inject required objectClass
    let object_class = mapping.object_class_for(location);
    if !object_class.is_empty() {
        attrs.push(("objectClass".to_string(), object_class));
    }
    attrs
}

/// Converts an LDAP attribute list back into a canonical serializable_map for
/// the given `location`.
pub fn canonical_map_from_ldap_attrs(
    attrs: &LdapAttrList,
    mapping: &SchemaMapping,
    location: &str,
) -> CanonicalMap {
    let mut map: CanonicalMap = HashMap::new();
    for (ldap_attr, values) in attrs {
        if ldap_attr == "objectClass" {
            continue;
        }
        if let Some(canon_key) = mapping.ldap_to_canonical(location, ldap_attr) {
            let value = values.first().cloned().unwrap_or_default();
            map.insert(canon_key, (value, ValueType::String, HashMap::new()));
        }
    }
    map
}

/// Extracts the primary key value from a canonical map for the given location.
/// Returns an empty string if not found.
pub fn primary_key_value(map: &CanonicalMap, mapping: &SchemaMapping, location: &str) -> String {
    let pk_field = mapping.primary_key_field(location);
    map.get(&pk_field)
        .map(|(v, _, _)| v.clone())
        .unwrap_or_default()
}

/// Returns the LDAP search filter that matches the primary key.
/// e.g. "(uid=alice)" for principals.
pub fn primary_key_filter(id: &str, mapping: &SchemaMapping, location: &str) -> String {
    let pk_attr = mapping.canonical_to_ldap(location, &mapping.primary_key_field(location));
    format!("({}={})", pk_attr, ldap_escape(id))
}

/// Returns an LDAP search filter built from all key/value pairs in `filter_map`.
/// Multiple filters are ANDed: "(&(uid=alice)(oxTenantId=tenant1))".
pub fn build_fetch_filter(
    filter_map: &CanonicalMap,
    mapping: &SchemaMapping,
    location: &str,
) -> String {
    let conditions: Vec<String> = filter_map
        .iter()
        .filter_map(|(canon_key, (value, _, _))| {
            let ldap_attr = mapping.canonical_to_ldap(location, canon_key);
            if ldap_attr == "__skip__" {
                return None;
            }
            Some(format!("({}={})", ldap_attr, ldap_escape(value)))
        })
        .collect();

    if conditions.is_empty() {
        "(objectClass=*)".to_string()
    } else if conditions.len() == 1 {
        conditions[0].clone()
    } else {
        format!("(&{})", conditions.concat())
    }
}

/// Escapes special characters in LDAP filter values per RFC 4515.
pub fn ldap_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '*'  => out.push_str(r"\2a"),
            '('  => out.push_str(r"\28"),
            ')'  => out.push_str(r"\29"),
            '\\' => out.push_str(r"\5c"),
            '\0' => out.push_str(r"\00"),
            _    => out.push(c),
        }
    }
    out
}
