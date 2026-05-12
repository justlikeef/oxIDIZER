use std::collections::HashMap;
use std::sync::Arc;
use ox_persistence::PersistenceDriver;
use ox_persistence_ad::AdPersistenceDriver;
use ox_persistence_ldap::conn_factory::{LdapConn, MockLdapConn, MockLdapConnFactory};
use ox_type_converter::ValueType;

fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

fn make_ad_driver() -> (AdPersistenceDriver, Arc<MockLdapConn>) {
    let mock_conn = Arc::new(MockLdapConn::new());
    let factory = Arc::new(MockLdapConnFactory::new(mock_conn.clone()));
    let mut conn_info = HashMap::new();
    conn_info.insert("url".to_string(),           "ldap://ad.corp.example.com:389".to_string());
    conn_info.insert("bind_dn".to_string(),       "CN=svc,CN=Users,DC=corp,DC=example,DC=com".to_string());
    conn_info.insert("bind_password".to_string(), "P@ssw0rd".to_string());
    conn_info.insert("base_dn".to_string(),       "DC=corp,DC=example,DC=com".to_string());
    let driver = AdPersistenceDriver::new_with_factory(factory, conn_info);
    (driver, mock_conn)
}

#[test]
fn ad_driver_uses_samaccountname_for_principal_id() {
    let (driver, mock_conn) = make_ad_driver();
    let mut data = HashMap::new();
    data.insert("principal_id".to_string(), str_entry("jsmith"));
    data.insert("display_name".to_string(), str_entry("John Smith"));
    data.insert("source".to_string(),       str_entry("Ad"));
    data.insert("tenant_id".to_string(),    str_entry("corp"));

    driver.persist(&data, "principals").expect("persist failed");

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let results = rt.block_on(async {
        mock_conn.search("DC=corp,DC=example,DC=com", "(sAMAccountName=jsmith)").await.unwrap()
    });
    assert_eq!(results.len(), 1);
    assert!(results[0].iter().any(|(k, v)| k == "sAMAccountName" && v.contains(&"jsmith".to_string())));
}

#[test]
fn ad_driver_restore_principal_by_samaccountname() {
    let (driver, _) = make_ad_driver();
    let mut data = HashMap::new();
    data.insert("principal_id".to_string(), str_entry("jdoe"));
    data.insert("display_name".to_string(), str_entry("Jane Doe"));
    data.insert("source".to_string(),       str_entry("Ad"));
    data.insert("tenant_id".to_string(),    str_entry("corp"));
    driver.persist(&data, "principals").expect("persist failed");

    let restored = driver.restore("principals", "jdoe").expect("restore failed");
    assert_eq!(restored.get("principal_id").unwrap().0, "jdoe");
    assert_eq!(restored.get("display_name").unwrap().0, "Jane Doe");
}

#[test]
fn ad_driver_group_uses_ad_object_class() {
    let (driver, mock_conn) = make_ad_driver();
    let mut data = HashMap::new();
    data.insert("group_id".to_string(),  str_entry("Domain Admins"));
    data.insert("name".to_string(),      str_entry("Domain Administrators"));
    data.insert("source".to_string(),    str_entry("Ad"));
    data.insert("tenant_id".to_string(), str_entry("corp"));
    driver.persist(&data, "groups").expect("persist failed");

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let results = rt.block_on(async {
        mock_conn.search("DC=corp,DC=example,DC=com", "(cn=Domain Admins)").await.unwrap()
    });
    assert_eq!(results.len(), 1);
    // AD groups use objectClass=group
    assert!(results[0].iter().any(|(k, v)| k == "objectClass" && v.contains(&"group".to_string())));
}

#[test]
fn ad_driver_list_datasets_returns_standard_locations() {
    let (driver, _) = make_ad_driver();
    let datasets = driver.list_datasets(&HashMap::new()).expect("list failed");
    assert!(datasets.contains(&"principals".to_string()));
    assert!(datasets.contains(&"groups".to_string()));
}
