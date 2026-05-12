//! Translates canonical IAM serializable_maps to/from Okta REST JSON bodies.

use std::collections::HashMap;
use ox_type_converter::ValueType;
use serde_json::{json, Value};

pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// Returns the canonical field value from a map, or empty string.
pub fn canon_get(map: &CanonicalMap, key: &str) -> String {
    map.get(key).map(|(v, _, _)| v.clone()).unwrap_or_default()
}

/// Builds an Okta user creation/update body from a canonical `principals` map.
///
/// Okta user body format:
/// ```json
/// { "profile": { "login": "<principal_id>", "displayName": "<display_name>" } }
/// ```
pub fn principal_to_okta_body(map: &CanonicalMap) -> Value {
    json!({
        "profile": {
            "login": canon_get(map, "principal_id"),
            "displayName": canon_get(map, "display_name"),
            "oxSource": canon_get(map, "source"),
            "oxTenantId": canon_get(map, "tenant_id"),
        }
    })
}

/// Extracts a canonical `principals` map from an Okta user JSON object.
pub fn okta_user_to_canonical(user: &Value) -> CanonicalMap {
    let mut map = CanonicalMap::new();
    let profile = &user["profile"];
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();

    map.insert("principal_id".to_string(), (s(&profile["login"]),      ValueType::String, HashMap::new()));
    map.insert("display_name".to_string(), (s(&profile["displayName"]),ValueType::String, HashMap::new()));
    map.insert("source".to_string(),       (s(&profile["oxSource"]),    ValueType::String, HashMap::new()));
    map.insert("tenant_id".to_string(),    (s(&profile["oxTenantId"]), ValueType::String, HashMap::new()));
    // Store the Okta internal ID as an annotation — callers can use it for group membership calls.
    map.insert("_okta_id".to_string(),     (s(&user["id"]),             ValueType::String, HashMap::new()));
    map
}

/// Builds an Okta group creation body from a canonical `groups` map.
///
/// Okta group body format:
/// ```json
/// { "profile": { "name": "<group_id>", "description": "<name>" } }
/// ```
pub fn group_to_okta_body(map: &CanonicalMap) -> Value {
    json!({
        "profile": {
            "name": canon_get(map, "group_id"),
            "description": canon_get(map, "name"),
            "oxSource": canon_get(map, "source"),
            "oxTenantId": canon_get(map, "tenant_id"),
        }
    })
}

/// Extracts a canonical `groups` map from an Okta group JSON object.
pub fn okta_group_to_canonical(group: &Value) -> CanonicalMap {
    let mut map = CanonicalMap::new();
    let profile = &group["profile"];
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();

    map.insert("group_id".to_string(),  (s(&profile["name"]),        ValueType::String, HashMap::new()));
    map.insert("name".to_string(),      (s(&profile["description"]), ValueType::String, HashMap::new()));
    map.insert("source".to_string(),    (s(&profile["oxSource"]),    ValueType::String, HashMap::new()));
    map.insert("tenant_id".to_string(), (s(&profile["oxTenantId"]), ValueType::String, HashMap::new()));
    map.insert("_okta_id".to_string(),  (s(&group["id"]),            ValueType::String, HashMap::new()));
    map
}

/// Returns the Okta REST path for adding a user to a group.
/// `group_okta_id` is the Okta internal group id (not the canonical group_id).
/// `user_okta_id` is the Okta internal user id.
pub fn group_membership_put_path(group_okta_id: &str, user_okta_id: &str) -> String {
    format!("/api/v1/groups/{}/users/{}", group_okta_id, user_okta_id)
}

/// Returns the Okta REST path for listing a user's groups.
pub fn user_groups_path(user_okta_id: &str) -> String {
    format!("/api/v1/users/{}/groups", user_okta_id)
}

/// Returns the list of locations this driver supports natively (no overflow needed).
pub fn supported_locations() -> Vec<String> {
    vec![
        "principals".to_string(),
        "groups".to_string(),
        "members".to_string(),
    ]
}
