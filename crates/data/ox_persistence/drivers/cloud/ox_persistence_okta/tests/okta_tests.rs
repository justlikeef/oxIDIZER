mod http_client_tests {
    use ox_persistence_okta::http_client::{MockOktaHttpClient, OktaHttpClient, OktaRequest, OktaResponse};
    use std::collections::HashMap;

    #[test]
    fn mock_get_returns_canned_response() {
        let mock = MockOktaHttpClient::new();
        mock.expect_get(
            "/api/v1/users/alice",
            serde_json::json!({
                "id": "00uABCD1234",
                "profile": { "login": "alice", "displayName": "Alice" }
            }),
        );

        let resp = mock.get("/api/v1/users/alice").expect("get failed");
        assert_eq!(resp["id"], "00uABCD1234");
        assert_eq!(resp["profile"]["login"], "alice");
    }

    #[test]
    fn mock_post_records_request_body() {
        let mock = MockOktaHttpClient::new();
        mock.expect_post(
            "/api/v1/users",
            serde_json::json!({ "id": "00uNEW0001", "profile": { "login": "newuser" } }),
        );

        let body = serde_json::json!({ "profile": { "login": "newuser", "displayName": "New User" } });
        let resp = mock.post("/api/v1/users", &body).expect("post failed");
        assert_eq!(resp["id"], "00uNEW0001");

        let recorded = mock.recorded_posts();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["profile"]["login"], "newuser");
    }

    #[test]
    fn mock_put_records_call() {
        let mock = MockOktaHttpClient::new();
        mock.expect_put("/api/v1/groups/grp001/users/usr001", serde_json::json!({}));
        mock.put("/api/v1/groups/grp001/users/usr001", &serde_json::json!({}))
            .expect("put failed");
        assert_eq!(mock.recorded_puts().len(), 1);
    }
}

mod driver_tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use ox_persistence::PersistenceDriver;
    use ox_persistence_okta::OktaPersistenceDriver;
    use ox_persistence_okta::http_client::MockOktaHttpClient;
    use ox_type_converter::ValueType;

    fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
        (v.to_string(), ValueType::String, HashMap::new())
    }

    fn make_driver() -> (OktaPersistenceDriver, Arc<MockOktaHttpClient>) {
        let mock = Arc::new(MockOktaHttpClient::new());
        let driver = OktaPersistenceDriver::new_with_client(mock.clone());
        (driver, mock)
    }

    #[test]
    fn persist_principal_posts_to_okta_users() {
        let (driver, mock) = make_driver();
        mock.expect_post("/api/v1/users?activate=true", serde_json::json!({
            "id": "00u001",
            "profile": { "login": "alice" }
        }));

        let mut data = HashMap::new();
        data.insert("principal_id".to_string(), str_entry("alice"));
        data.insert("display_name".to_string(), str_entry("Alice"));
        data.insert("source".to_string(),       str_entry("Okta"));
        data.insert("tenant_id".to_string(),    str_entry("t1"));

        driver.persist(&data, "principals").expect("persist failed");
        let posts = mock.recorded_posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0]["profile"]["login"], "alice");
    }

    #[test]
    fn restore_principal_by_id_calls_get_user() {
        let (driver, mock) = make_driver();
        mock.expect_get("/api/v1/users/alice", serde_json::json!({
            "id": "00u001",
            "profile": {
                "login": "alice",
                "displayName": "Alice Liddell",
                "oxSource": "Okta",
                "oxTenantId": "t1"
            }
        }));

        let restored = driver.restore("principals", "alice").expect("restore failed");
        assert_eq!(restored.get("principal_id").unwrap().0, "alice");
        assert_eq!(restored.get("display_name").unwrap().0, "Alice Liddell");
    }

    #[test]
    fn fetch_principals_calls_list_users() {
        let (driver, mock) = make_driver();
        mock.expect_get("/api/v1/users", serde_json::json!([
            { "id": "00u001", "profile": { "login": "alice", "displayName": "Alice", "oxSource": "Okta", "oxTenantId": "t1" } },
            { "id": "00u002", "profile": { "login": "bob",   "displayName": "Bob",   "oxSource": "Okta", "oxTenantId": "t1" } }
        ]));

        let filter = HashMap::new();
        let ids = driver.fetch(&filter, "principals").expect("fetch failed");
        assert!(ids.contains(&"alice".to_string()));
        assert!(ids.contains(&"bob".to_string()));
    }

    #[test]
    fn persist_group_posts_to_okta_groups() {
        let (driver, mock) = make_driver();
        mock.expect_post("/api/v1/groups", serde_json::json!({
            "id": "grp001",
            "profile": { "name": "ops", "description": "Operations" }
        }));

        let mut data = HashMap::new();
        data.insert("group_id".to_string(),  str_entry("ops"));
        data.insert("name".to_string(),      str_entry("Operations"));
        data.insert("source".to_string(),    str_entry("Okta"));
        data.insert("tenant_id".to_string(), str_entry("t1"));

        driver.persist(&data, "groups").expect("persist failed");
        assert_eq!(mock.recorded_posts().len(), 1);
    }

    #[test]
    fn persist_grant_returns_not_supported() {
        let (driver, _) = make_driver();
        let mut data = HashMap::new();
        data.insert("node_path".to_string(),     str_entry("com.justlikeef.data"));
        data.insert("group_id".to_string(),       str_entry("ops"));
        data.insert("operation_name".to_string(), str_entry("read"));
        data.insert("allow_deny".to_string(),     str_entry("Allow"));
        data.insert("tenant_id".to_string(),      str_entry("t1"));

        let result = driver.persist(&data, "grants");
        assert!(result.is_err(), "grants should not be supported natively");
    }

    #[test]
    fn list_datasets_returns_supported_locations() {
        let (driver, _) = make_driver();
        let datasets = driver.list_datasets(&HashMap::new()).expect("list failed");
        assert!(datasets.contains(&"principals".to_string()));
        assert!(datasets.contains(&"groups".to_string()));
        assert!(datasets.contains(&"members".to_string()));
        // grants and sessions must NOT be in the list (overflow responsibility of caller)
        assert!(!datasets.contains(&"grants".to_string()));
        assert!(!datasets.contains(&"sessions".to_string()));
    }
}
