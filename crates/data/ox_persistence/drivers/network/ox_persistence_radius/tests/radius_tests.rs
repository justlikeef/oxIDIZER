use ox_persistence_radius::response_parser::{
    parse_access_accept, RadiusAttribute,
};
use ox_persistence_radius::RadiusPersistenceDriver;
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
fn parse_access_accept_extracts_user_name() {
    // Simulate an Access-Accept with User-Name and Class attributes.
    let attrs = vec![
        RadiusAttribute { attr_type: 1,   value: b"alice".to_vec() }, // User-Name = 1
        RadiusAttribute { attr_type: 25,  value: b"grp-ops".to_vec() }, // Class = 25
    ];
    let canonical = parse_access_accept(&attrs, "t1");
    assert_eq!(canonical.get("principal_id").unwrap().0, "alice");
    assert_eq!(canonical.get("tenant_id").unwrap().0, "t1");
}

#[test]
fn parse_access_accept_extracts_class_as_group() {
    let attrs = vec![
        RadiusAttribute { attr_type: 1,  value: b"bob".to_vec() },
        RadiusAttribute { attr_type: 25, value: b"network-admins".to_vec() },
    ];
    let canonical = parse_access_accept(&attrs, "tenant1");
    assert_eq!(canonical.get("group_id").unwrap().0, "network-admins");
    assert_eq!(canonical.get("principal_id").unwrap().0, "bob");
}

// ---------------------------------------------------------------------------
// Driver tests
// ---------------------------------------------------------------------------

#[test]
fn radius_driver_restore_returns_cached_principal() {
    let mut cache = HashMap::new();
    let mut entry = HashMap::new();
    entry.insert("principal_id".to_string(), str_entry("alice"));
    entry.insert("display_name".to_string(), str_entry("Alice"));
    entry.insert("source".to_string(),       str_entry("Radius"));
    entry.insert("tenant_id".to_string(),    str_entry("t1"));
    cache.insert("alice".to_string(), entry);

    let driver = RadiusPersistenceDriver::new_with_cache(cache);
    let restored = driver.restore("principals", "alice").expect("restore failed");
    assert_eq!(restored.get("principal_id").unwrap().0, "alice");
}

#[test]
fn radius_driver_persist_returns_not_supported() {
    let driver = RadiusPersistenceDriver::new_with_cache(HashMap::new());
    let mut data = HashMap::new();
    data.insert("principal_id".to_string(), str_entry("alice"));
    let result = driver.persist(&data, "principals");
    assert!(result.is_err(), "RADIUS persist should return not-supported error");
}

#[test]
fn radius_driver_list_datasets_returns_principals_and_members() {
    let driver = RadiusPersistenceDriver::new_with_cache(HashMap::new());
    let datasets = driver.list_datasets(&HashMap::new()).expect("list failed");
    assert!(datasets.contains(&"principals".to_string()));
    assert!(datasets.contains(&"members".to_string()));
    assert!(!datasets.contains(&"grants".to_string()));
    assert!(!datasets.contains(&"sessions".to_string()));
}

#[test]
fn radius_driver_fetch_members_returns_by_group() {
    let driver = RadiusPersistenceDriver::new();
    let mut entry1 = HashMap::new();
    entry1.insert("principal_id".to_string(), str_entry("u1"));
    entry1.insert("group_id".to_string(),     str_entry("g1"));
    entry1.insert("tenant_id".to_string(),    str_entry("t1"));
    driver.insert_cached_principal("u1", entry1);

    let mut entry2 = HashMap::new();
    entry2.insert("principal_id".to_string(), str_entry("u2"));
    entry2.insert("group_id".to_string(),     str_entry("g2"));
    entry2.insert("tenant_id".to_string(),    str_entry("t1"));
    driver.insert_cached_principal("u2", entry2);

    let mut filter = HashMap::new();
    filter.insert("group_id".to_string(), str_entry("g1"));
    let result = driver.fetch(&filter, "members").expect("fetch failed");
    assert_eq!(result.len(), 1);
    assert!(result.contains(&"u1".to_string()));
}

#[test]
fn radius_driver_fetch_members_missing_group_id_returns_error() {
    let driver = RadiusPersistenceDriver::new();
    let filter = HashMap::new();
    let result = driver.fetch(&filter, "members");
    assert!(result.is_err());
}
