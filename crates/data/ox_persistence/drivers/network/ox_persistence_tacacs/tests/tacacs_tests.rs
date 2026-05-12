use ox_persistence_tacacs::response_parser::{parse_av_pairs, TacacsAvPair};
use ox_persistence_tacacs::TacacsPersistenceDriver;
use ox_persistence::PersistenceDriver;
use std::collections::HashMap;
use ox_type_converter::ValueType;

fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

// ---------------------------------------------------------------------------
// Parser tests
// ---------------------------------------------------------------------------

#[test]
fn parse_av_pairs_extracts_user_from_service_pair() {
    let pairs = vec![
        TacacsAvPair { attribute: "service".to_string(), value: "shell".to_string() },
        TacacsAvPair { attribute: "priv-lvl".to_string(), value: "15".to_string() },
        TacacsAvPair { attribute: "user".to_string(), value: "alice".to_string() },
    ];
    let canonical = parse_av_pairs(&pairs, "t1");
    assert_eq!(canonical.get("principal_id").unwrap().0, "alice");
    assert_eq!(canonical.get("tenant_id").unwrap().0, "t1");
}

#[test]
fn parse_av_pairs_maps_priv_level_to_group() {
    let pairs = vec![
        TacacsAvPair { attribute: "user".to_string(),     value: "bob".to_string() },
        TacacsAvPair { attribute: "priv-lvl".to_string(), value: "15".to_string() },
    ];
    let canonical = parse_av_pairs(&pairs, "t1");
    // priv-lvl 15 maps to the "network-admin" role by convention (configurable).
    // The parser maps priv-lvl value to group_id as "priv-lvl-<N>".
    assert_eq!(canonical.get("group_id").unwrap().0, "priv-lvl-15");
}

#[test]
fn parse_av_pairs_custom_group_av() {
    let pairs = vec![
        TacacsAvPair { attribute: "user".to_string(),   value: "carol".to_string() },
        TacacsAvPair { attribute: "oxgroup".to_string(), value: "netops".to_string() },
    ];
    let canonical = parse_av_pairs(&pairs, "t1");
    assert_eq!(canonical.get("group_id").unwrap().0, "netops");
}

// ---------------------------------------------------------------------------
// Driver tests
// ---------------------------------------------------------------------------

#[test]
fn tacacs_driver_restore_returns_cached_principal() {
    let mut cache = HashMap::new();
    let mut entry = HashMap::new();
    entry.insert("principal_id".to_string(), str_entry("alice"));
    entry.insert("source".to_string(),       str_entry("Tacacs"));
    entry.insert("tenant_id".to_string(),    str_entry("t1"));
    cache.insert("alice".to_string(), entry);

    let driver = TacacsPersistenceDriver::new_with_cache(cache);
    let restored = driver.restore("principals", "alice").expect("restore failed");
    assert_eq!(restored.get("principal_id").unwrap().0, "alice");
}

#[test]
fn tacacs_driver_persist_returns_not_supported() {
    let driver = TacacsPersistenceDriver::new_with_cache(HashMap::new());
    let mut data = HashMap::new();
    data.insert("principal_id".to_string(), str_entry("alice"));
    let result = driver.persist(&data, "principals");
    assert!(result.is_err());
}

#[test]
fn tacacs_driver_list_datasets_does_not_include_grants() {
    let driver = TacacsPersistenceDriver::new_with_cache(HashMap::new());
    let datasets = driver.list_datasets(&HashMap::new()).expect("list failed");
    assert!(datasets.contains(&"principals".to_string()));
    assert!(!datasets.contains(&"grants".to_string()));
    assert!(!datasets.contains(&"sessions".to_string()));
}
