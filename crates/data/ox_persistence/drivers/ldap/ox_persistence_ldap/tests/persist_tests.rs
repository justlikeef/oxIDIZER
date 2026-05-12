// Tests for entity.rs — no LDAP server needed.
use ox_persistence_ldap::entity::{
    canonical_map_from_ldap_attrs, ldap_attrs_from_canonical_map,
};
use ox_persistence_ldap::mapping::SchemaMapping;
use std::collections::HashMap;
use ox_type_converter::ValueType;

fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

#[test]
fn ldap_attrs_round_trip_principal_record() {
    let mapping = SchemaMapping::ldap_defaults();
    let mut canon: HashMap<String, (String, ValueType, HashMap<String, String>)> = HashMap::new();
    canon.insert("principal_id".to_string(), str_entry("u001"));
    canon.insert("display_name".to_string(), str_entry("Alice"));
    canon.insert("source".to_string(), str_entry("Ldap"));
    canon.insert("tenant_id".to_string(), str_entry("tenant1"));

    let attrs = ldap_attrs_from_canonical_map(&canon, &mapping, "principals");
    // uid attribute must be set from principal_id
    assert!(attrs.iter().any(|(k, v)| k == "uid" && v.contains(&"u001".to_string())));
    // cn attribute must be set from display_name
    assert!(attrs.iter().any(|(k, v)| k == "cn" && v.contains(&"Alice".to_string())));

    // Round-trip back
    let restored = canonical_map_from_ldap_attrs(&attrs, &mapping, "principals");
    assert_eq!(restored.get("principal_id").unwrap().0, "u001");
    assert_eq!(restored.get("display_name").unwrap().0, "Alice");
}

#[test]
fn ldap_attrs_round_trip_security_group() {
    let mapping = SchemaMapping::ldap_defaults();
    let mut canon: HashMap<String, (String, ValueType, HashMap<String, String>)> = HashMap::new();
    canon.insert("group_id".to_string(), str_entry("grp-ops"));
    canon.insert("name".to_string(), str_entry("Operations"));
    canon.insert("source".to_string(), str_entry("Ldap"));
    canon.insert("tenant_id".to_string(), str_entry("tenant1"));

    let attrs = ldap_attrs_from_canonical_map(&canon, &mapping, "groups");
    assert!(attrs.iter().any(|(k, v)| k == "cn" && v.contains(&"grp-ops".to_string())));

    let restored = canonical_map_from_ldap_attrs(&attrs, &mapping, "groups");
    assert_eq!(restored.get("group_id").unwrap().0, "grp-ops");
    assert_eq!(restored.get("name").unwrap().0, "Operations");
}

#[test]
fn ldap_attrs_round_trip_permission_grant() {
    let mapping = SchemaMapping::ldap_defaults();
    let mut canon: HashMap<String, (String, ValueType, HashMap<String, String>)> = HashMap::new();
    canon.insert("node_path".to_string(), str_entry("com.justlikeef.data"));
    canon.insert("group_id".to_string(), str_entry("grp-ops"));
    canon.insert("operation_name".to_string(), str_entry("read"));
    canon.insert("allow_deny".to_string(), str_entry("Allow"));
    canon.insert("tenant_id".to_string(), str_entry("tenant1"));

    let attrs = ldap_attrs_from_canonical_map(&canon, &mapping, "grants");
    assert!(attrs.iter().any(|(k, _)| k == "oxNodePath"));
    assert!(attrs.iter().any(|(k, _)| k == "oxGroupId"));

    let restored = canonical_map_from_ldap_attrs(&attrs, &mapping, "grants");
    assert_eq!(restored.get("node_path").unwrap().0, "com.justlikeef.data");
    assert_eq!(restored.get("operation_name").unwrap().0, "read");
    assert_eq!(restored.get("allow_deny").unwrap().0, "Allow");
}

mod conn_tests {
    use ox_persistence_ldap::conn_factory::{LdapConn, MockLdapConn};
    use ox_persistence_ldap::entity::LdapAttrList;

    #[tokio::test]
    async fn mock_conn_add_then_search_returns_entry() {
        let mock = MockLdapConn::new();
        let attrs: LdapAttrList = vec![
            ("uid".to_string(), vec!["alice".to_string()]),
            ("cn".to_string(), vec!["Alice Liddell".to_string()]),
            ("objectClass".to_string(), vec!["inetOrgPerson".to_string()]),
        ];

        mock.add("uid=alice,ou=people,dc=example,dc=com", attrs.clone()).await.unwrap();

        let results = mock
            .search("ou=people,dc=example,dc=com", "(uid=alice)")
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].iter().any(|(k, v)| k == "uid" && v.contains(&"alice".to_string())));
    }

    #[tokio::test]
    async fn mock_conn_modify_updates_attribute() {
        let mock = MockLdapConn::new();
        let attrs: LdapAttrList = vec![
            ("uid".to_string(), vec!["bob".to_string()]),
            ("cn".to_string(), vec!["Bob Original".to_string()]),
            ("objectClass".to_string(), vec!["inetOrgPerson".to_string()]),
        ];
        mock.add("uid=bob,ou=people,dc=example,dc=com", attrs).await.unwrap();

        mock.modify(
            "uid=bob,ou=people,dc=example,dc=com",
            vec![("cn".to_string(), vec!["Bob Updated".to_string()])],
        )
        .await
        .unwrap();

        let results = mock
            .search("ou=people,dc=example,dc=com", "(uid=bob)")
            .await
            .unwrap();
        assert!(results[0].iter().any(|(k, v)| k == "cn" && v.contains(&"Bob Updated".to_string())));
    }
}

mod driver_tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use ox_persistence::PersistenceDriver;
    use ox_persistence_ldap::LdapPersistenceDriver;
    use ox_persistence_ldap::conn_factory::{LdapConn, MockLdapConn, MockLdapConnFactory};
    use ox_persistence_ldap::mapping::SchemaMapping;
    use ox_type_converter::ValueType;

    fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
        (v.to_string(), ValueType::String, HashMap::new())
    }

    fn make_driver() -> (LdapPersistenceDriver, Arc<MockLdapConn>) {
        let mock_conn = Arc::new(MockLdapConn::new());
        let factory = Arc::new(MockLdapConnFactory::new(mock_conn.clone()));
        let mut conn_info = HashMap::new();
        conn_info.insert("url".to_string(),           "ldap://localhost:389".to_string());
        conn_info.insert("bind_dn".to_string(),       "cn=admin,dc=example,dc=com".to_string());
        conn_info.insert("bind_password".to_string(), "secret".to_string());
        conn_info.insert("base_dn".to_string(),       "dc=example,dc=com".to_string());
        let driver = LdapPersistenceDriver::new_with_factory(factory, SchemaMapping::ldap_defaults(), conn_info);
        (driver, mock_conn)
    }

    #[test]
    fn persist_principal_record_creates_ldap_entry() {
        let (driver, mock_conn) = make_driver();
        let mut data = HashMap::new();
        data.insert("principal_id".to_string(), str_entry("alice"));
        data.insert("display_name".to_string(), str_entry("Alice Liddell"));
        data.insert("source".to_string(),       str_entry("Ldap"));
        data.insert("tenant_id".to_string(),    str_entry("t1"));

        driver.persist(&data, "principals").expect("persist failed");

        // The mock should now contain an entry for alice
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let results = rt.block_on(async {
            mock_conn.search("dc=example,dc=com", "(uid=alice)").await.unwrap()
        });
        assert_eq!(results.len(), 1);
        assert!(results[0].iter().any(|(k, v)| k == "uid" && v.contains(&"alice".to_string())));
    }

    #[test]
    fn restore_principal_by_id_returns_entry() {
        let (driver, _) = make_driver();
        let mut data = HashMap::new();
        data.insert("principal_id".to_string(), str_entry("bob"));
        data.insert("display_name".to_string(), str_entry("Bob Builder"));
        data.insert("source".to_string(),       str_entry("Ldap"));
        data.insert("tenant_id".to_string(),    str_entry("t1"));
        driver.persist(&data, "principals").expect("persist failed");

        let restored = driver.restore("principals", "bob").expect("restore failed");
        assert_eq!(restored.get("principal_id").unwrap().0, "bob");
        assert_eq!(restored.get("display_name").unwrap().0, "Bob Builder");
    }

    #[test]
    fn fetch_principals_by_tenant_returns_ids() {
        let (driver, _) = make_driver();
        for name in &["carol", "dave"] {
            let mut data = HashMap::new();
            data.insert("principal_id".to_string(), str_entry(name));
            data.insert("display_name".to_string(), str_entry(name));
            data.insert("source".to_string(),       str_entry("Ldap"));
            data.insert("tenant_id".to_string(),    str_entry("t1"));
            driver.persist(&data, "principals").expect("persist failed");
        }

        let mut filter = HashMap::new();
        filter.insert("tenant_id".to_string(), str_entry("t1"));
        let ids = driver.fetch(&filter, "principals").expect("fetch failed");
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"carol".to_string()));
        assert!(ids.contains(&"dave".to_string()));
    }

    #[test]
    fn persist_permission_grant_creates_grant_entry() {
        let (driver, mock_conn) = make_driver();
        let mut data = HashMap::new();
        data.insert("node_path".to_string(),     str_entry("com.justlikeef.data"));
        data.insert("group_id".to_string(),       str_entry("grp-ops"));
        data.insert("operation_name".to_string(), str_entry("read"));
        data.insert("allow_deny".to_string(),     str_entry("Allow"));
        data.insert("tenant_id".to_string(),      str_entry("t1"));

        driver.persist(&data, "grants").expect("persist failed");

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let results = rt.block_on(async {
            mock_conn.search("dc=example,dc=com", "(oxNodePath=com.justlikeef.data)").await.unwrap()
        });
        assert_eq!(results.len(), 1);
        assert!(results[0].iter().any(|(k, v)| k == "oxGroupId" && v.contains(&"grp-ops".to_string())));
    }

    #[test]
    fn list_datasets_returns_supported_locations() {
        let (driver, _) = make_driver();
        let conn_info = HashMap::new();
        let datasets = driver.list_datasets(&conn_info).expect("list failed");
        assert!(datasets.contains(&"principals".to_string()));
        assert!(datasets.contains(&"groups".to_string()));
        assert!(datasets.contains(&"grants".to_string()));
        assert!(datasets.contains(&"sessions".to_string()));
    }
}
