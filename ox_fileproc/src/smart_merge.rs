use serde_json::{Map, Value};
use std::collections::HashMap;

/// Smartly merges `overlay` into `base`.
pub fn smart_merge_values(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            smart_merge_objects(base_map, overlay_map);
        }
        (Value::Array(base_arr), Value::Array(overlay_arr)) => {
            smart_merge_arrays(base_arr, overlay_arr);
        }
        (base_val, overlay_val) => {
            // Default: Overlay replaces Base
            *base_val = overlay_val;
        }
    }
}

pub fn smart_merge_objects(base: &mut Map<String, Value>, overlay: Map<String, Value>) {
    for (k, v) in overlay {
        if let Some(base_val) = base.get_mut(&k) {
            smart_merge_values(base_val, v);
        } else {
            base.insert(k, v);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum IdKey {
    Str(String),
    I64(i64),
    U64(u64),
    F64(u64),
    Bool(bool),
}

fn id_key(val: &Value) -> Option<IdKey> {
    match val {
        Value::String(s) => Some(IdKey::Str(s.clone())),
        Value::Bool(b) => Some(IdKey::Bool(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(IdKey::I64(i))
            } else if let Some(u) = n.as_u64() {
                Some(IdKey::U64(u))
            } else if let Some(f) = n.as_f64() {
                if f.is_finite() {
                    Some(IdKey::F64(f.to_bits()))
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn number_f64_bits(val: &Value) -> Option<u64> {
    if let Value::Number(n) = val
        && let Some(f) = n.as_f64()
            && f.is_finite() {
                return Some(f.to_bits());
            }
    None
}

pub fn smart_merge_arrays(base: &mut Vec<Value>, overlay: Vec<Value>) {
    // Build an index for base items by id for efficient lookups.
    let mut base_index: HashMap<IdKey, usize> = HashMap::new();
    let mut base_index_f64: HashMap<u64, usize> = HashMap::new();

    for (idx, item) in base.iter().enumerate() {
        if let Value::Object(map) = item
            && let Some(id_val) = map.get("id") {
                if let Some(key) = id_key(id_val) {
                    base_index.entry(key).or_insert(idx);
                }
                if let Some(bits) = number_f64_bits(id_val) {
                    base_index_f64.entry(bits).or_insert(idx);
                }
            }
    }

    for overlay_item in overlay {
        let mut merged = false;

        if let Value::Object(ref overlay_map) = overlay_item
            && let Some(overlay_id_val) = overlay_map.get("id") {
                let mut match_idx = None;
                if let Some(key) = id_key(overlay_id_val) {
                    match_idx = base_index.get(&key).cloned();
                }
                if match_idx.is_none()
                    && let Some(bits) = number_f64_bits(overlay_id_val) {
                        match_idx = base_index_f64.get(&bits).cloned();
                    }

                if let Some(idx) = match_idx
                    && let Some(base_item) = base.get_mut(idx) {
                        smart_merge_values(base_item, overlay_item.clone());
                        merged = true;
                    }
            }

        if !merged {
            let new_index = base.len();
            if let Value::Object(ref map) = overlay_item
                && let Some(id_val) = map.get("id") {
                    if let Some(key) = id_key(id_val) {
                        base_index.entry(key).or_insert(new_index);
                    }
                    if let Some(bits) = number_f64_bits(id_val) {
                        base_index_f64.entry(bits).or_insert(new_index);
                    }
                }
            base.push(overlay_item);
        }
    }
}
