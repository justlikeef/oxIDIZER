
#[cfg(test)]
mod tests {
    use crate::SqlitePersistenceDriver;
    use ox_persistence::PersistenceDriver;
    use std::collections::HashMap;
    use ox_type_converter::ValueType;
    use std::sync::Mutex;
    use rusqlite::Connection;

    #[test]
    fn test_sqlite_persist_restore_fetch() {
        // Use a file-based DB for persistence across calls if needed, 
        // but since we re-instantiate connection per request in FFI usage, 
        // passing :memory: might not persist data between 'persist' and 'restore' calls 
        // if they were separate connections.
        // However, the test here uses the struct directly, which holds the connection.
        
        let driver = SqlitePersistenceDriver {
            conn: Mutex::new(Some(Connection::open_in_memory().unwrap())),
            connection_string: Mutex::new(":memory:".to_string())
        };

        // Prepare table
        {
             let mut guard = driver.conn.lock().unwrap();
             let conn = guard.as_mut().unwrap();
             conn.execute("CREATE TABLE test_table (id TEXT PRIMARY KEY, name TEXT, age INTEGER)", []).unwrap();
        }

        let location = "test_table"; // Used as table name in this driver implementation assumption

        // 1. Persist
        let mut data = HashMap::new();
        data.insert("id".to_string(), ("1".to_string(), ValueType::String, HashMap::new()));
        data.insert("name".to_string(), ("Bob".to_string(), ValueType::String, HashMap::new()));
        data.insert("age".to_string(), ("40".to_string(), ValueType::Integer, HashMap::new()));

        driver.persist(&data, location).expect("Persist failed");

        // 2. Restore
        let restored = driver.restore(location, "1").expect("Restore failed");
        assert_eq!(restored.get("name").unwrap().0, "Bob");
        assert_eq!(restored.get("age").unwrap().0, "40");

        // 3. Fetch
        let mut filter = HashMap::new();
        filter.insert("name".to_string(), ("Bob".to_string(), ValueType::String, HashMap::new()));
        
        // Note: fetch logic in current driver might need 'id' column mapping
        // implementation assumes standard SQL.
        let fetched_ids = driver.fetch(&filter, location).expect("Fetch failed");
        assert_eq!(fetched_ids.len(), 1);
        assert_eq!(fetched_ids[0], "1");
    }
}
