//! Parses RADIUS Access-Accept attribute-value pairs into canonical IAM maps.
//! RADIUS attribute type numbers follow RFC 2865.

use std::collections::HashMap;
use ox_type_converter::ValueType;

pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// A single RADIUS attribute from an Access-Accept packet.
#[derive(Debug, Clone)]
pub struct RadiusAttribute {
    /// RADIUS attribute type number (e.g. 1 = User-Name, 25 = Class).
    pub attr_type: u8,
    /// Raw value bytes.
    pub value: Vec<u8>,
}

impl RadiusAttribute {
    /// Decodes the value as a UTF-8 string, replacing invalid bytes with the replacement char.
    pub fn value_str(&self) -> String {
        String::from_utf8_lossy(&self.value).into_owned()
    }
}

fn s(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

/// Converts a list of RADIUS Access-Accept attributes into a canonical IAM map.
///
/// Mappings:
///   - Attribute type 1 (User-Name) → `principal_id`
///   - Attribute type 6 (Service-Type) → `source` annotation
///   - Attribute type 25 (Class) → `group_id` (first Class attribute wins for group_id)
///   - `tenant_id` is injected from the caller-supplied argument
///   - `source` is hard-coded to "Radius"
///
/// The Class attribute (type 25) contains group or role names depending on the
/// RADIUS server configuration.  If multiple Class attributes are present, the
/// first is used for `group_id` and the rest are stored as `group_id_2`, `group_id_3`, …
pub fn parse_access_accept(attrs: &[RadiusAttribute], tenant_id: &str) -> CanonicalMap {
    let mut map = CanonicalMap::new();
    map.insert("source".to_string(),    s("Radius"));
    map.insert("tenant_id".to_string(), s(tenant_id));

    let mut group_counter: u32 = 0;

    for attr in attrs {
        match attr.attr_type {
            1 => {
                // User-Name
                map.insert("principal_id".to_string(), s(&attr.value_str()));
                map.insert("display_name".to_string(), s(&attr.value_str()));
            }
            25 => {
                // Class — first occurrence becomes group_id, subsequent become group_id_N
                if group_counter == 0 {
                    map.insert("group_id".to_string(), s(&attr.value_str()));
                } else {
                    map.insert(format!("group_id_{}", group_counter + 1), s(&attr.value_str()));
                }
                group_counter += 1;
            }
            _ => {
                // Unknown attributes are ignored — the driver only extracts what it understands.
            }
        }
    }

    map
}
