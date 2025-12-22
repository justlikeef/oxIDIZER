
#[cfg(test)]
mod tests {
    use crate::JsonPersistenceDriver;
    use ox_persistence::PersistenceDriver;
    use tempfile::NamedTempFile;
    use std::collections::HashMap;
    use ox_type_converter::ValueType;

    #[test]
    fn test_json_persist_restore_fetch() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_str().unwrap().to_string();
        let driver = JsonPersistenceDriver;

        // 1. Persist
        let mut data = HashMap::new();
        // ID is crucial for restore/fetch logic in this driver
        data.insert("id".to_string(), ("123".to_string(), ValueType::String, HashMap::new()));
        data.insert("name".to_string(), ("Alice".to_string(), ValueType::String, HashMap::new()));
        data.insert("age".to_string(), ("30".to_string(), ValueType::Integer, HashMap::new()));

        driver.persist(&data, &path).expect("Persist failed");

        // 2. Restore
        let restored = driver.restore(&path, "123").expect("Restore failed");
        assert_eq!(restored.get("name").unwrap().0, "Alice");
        assert_eq!(restored.get("age").unwrap().0, "30");

        // 3. Fetch
        let mut filter = HashMap::new();
        filter.insert("name".to_string(), ("Alice".to_string(), ValueType::String, HashMap::new()));
        
        let fetched_ids = driver.fetch(&filter, &path).expect("Fetch failed");
        assert_eq!(fetched_ids.len(), 1);
        assert_eq!(fetched_ids[0], "123");

        // Fetch non-existent
        let mut filter_none = HashMap::new();
        filter_none.insert("name".to_string(), ("Bob".to_string(), ValueType::String, HashMap::new()));
        let fetched_none = driver.fetch(&filter_none, &path).expect("Fetch failed");
        assert!(fetched_none.is_empty());
    }
}
