
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DriverManager, DriverManagerConfig};
    use ox_fileproc::RawFile;
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_driver_toggle_query() {
        // Create a temporary driver file for testing
        let temp_dir = std::env::temp_dir().join("ox_test_drivers");
        if !temp_dir.exists() {
             fs::create_dir(&temp_dir).unwrap();
        }
        let driver_file_path = temp_dir.join("drivers.yaml");
        
        let content = r#"drivers:
  - id: "test_driver"
    name: "Test Driver"
    state: "disabled"
"#;
        fs::write(&driver_file_path, content).unwrap();

        let mut raw = RawFile::open(driver_file_path.to_str().unwrap()).expect("Failed to open file");

        let id = "test_driver";
        // Query must use quotes around ID
        let query = format!("drivers[id=\"{}\"]/state", id);
        
        let match_found = raw.find(&query).next();
        
        assert!(match_found.is_some(), "Should find the driver state");
        
        if let Some(c) = match_found {
            assert_eq!(c.value().trim(), "\"disabled\"", "Initial state should be disabled (quoted)");
            
            // Simulate toggle
            let new_val = "\"enabled\"";
            raw.update(c.span, new_val);
        }
        
        // Verify update in memory (or save and reload)
        // raw.save().unwrap();
        // Here we just verify the query matched correctly.
    }

    #[test]
    fn test_driver_discovery_new_structure() {
        // Setup a mock driver root with new structure
        let temp_dir = std::env::temp_dir().join("ox_test_discovery");
        if temp_dir.exists() {
             fs::remove_dir_all(&temp_dir).unwrap();
        }
        fs::create_dir_all(temp_dir.join("db")).unwrap();
        fs::create_dir_all(temp_dir.join("file")).unwrap();

        // Create dummy driver files
        fs::write(temp_dir.join("db/ox_persistence_driver_db_sqlite.so"), "").unwrap();
        fs::write(temp_dir.join("file/ox_persistence_driver_file_json.so"), "").unwrap();
        fs::write(temp_dir.join("ignored.txt"), "not a driver").unwrap();

        let config = DriverManagerConfig {
            drivers_file: "dummy".to_string(),
            driver_root: temp_dir.to_str().unwrap().to_string(),
            on_content_conflict: None,
        };

        let manager = DriverManager::new(config);
        let files = manager.list_available_driver_files().expect("Failed to list files");

        // Verify we found the drivers
        assert!(files.contains(&"db/ox_persistence_driver_db_sqlite.so".to_string()));
        assert!(files.contains(&"file/ox_persistence_driver_file_json.so".to_string()));
         // Ensure we didn't pick up non-driver files
        assert!(!files.iter().any(|f| f.contains("ignored.txt")));
    }

    #[test]
    fn test_load_configured_drivers_new_ids() {
        let temp_dir = std::env::temp_dir().join("ox_test_config");
        if !temp_dir.exists() {
             fs::create_dir(&temp_dir).unwrap();
        }
        let drivers_yaml = temp_dir.join("drivers.yaml");
        let content = r#"drivers:
  - id: "json"
    name: "ox_persistence_driver_file_json"
    state: "enabled"
  - id: "sqlite"
    name: "ox_persistence_driver_db_sqlite"
    state: "disabled"
"#;
        fs::write(&drivers_yaml, content).unwrap();

         let config = DriverManagerConfig {
            drivers_file: drivers_yaml.to_str().unwrap().to_string(),
            driver_root: "dummy".to_string(),
            on_content_conflict: None,
        };

        let manager = DriverManager::new(config);
        let list = manager.load_configured_drivers().expect("Failed to load config");

        assert_eq!(list.drivers.len(), 2);
        
        let json_driver = list.drivers.iter().find(|d| d.id == "json").unwrap();
        assert_eq!(json_driver.name, "ox_persistence_driver_file_json");
        assert_eq!(json_driver.state, "enabled");

        let sqlite_driver = list.drivers.iter().find(|d| d.id == "sqlite").unwrap();
        assert_eq!(sqlite_driver.name, "ox_persistence_driver_db_sqlite");
    }

    #[test]
    fn test_api_endpoints() {
        use ox_webservice_test_utils::{create_mock_api, create_task_state, drop_task_state, set_mock_field, get_mock_field};
        use std::sync::Arc;

        // Setup
        let temp_dir = std::env::temp_dir().join("ox_test_api");
        if temp_dir.exists() { fs::remove_dir_all(&temp_dir).unwrap(); }
        fs::create_dir_all(&temp_dir).unwrap();
        fs::create_dir_all(temp_dir.join("db")).unwrap();
        fs::write(temp_dir.join("db/test.so"), "").unwrap();

        let drivers_yaml = temp_dir.join("drivers.yaml");
        fs::write(&drivers_yaml, "drivers: []").unwrap();

        let config = DriverManagerConfig {
            drivers_file: drivers_yaml.to_str().unwrap().to_string(),
            driver_root: temp_dir.to_str().unwrap().to_string(),
            on_content_conflict: None,
        };

        let api = create_mock_api();
        let manager = Arc::new(DriverManager::new(config));

        let context = crate::ModuleContext {
            manager: manager.clone(),
            api,
            module_id: "test_driver_manager".to_string(),
        };

        let instance_ptr = Box::into_raw(Box::new(context)) as *mut libc::c_void;

        // 1. Test GET /drivers/available
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/drivers/available");
        set_mock_field(task_ctx, "request.method", "GET");

        unsafe { crate::ox_plugin_process(instance_ptr, task_ctx) };

        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert!(body.contains("db/test.so"), "available drivers should list db/test.so");
        unsafe { drop_task_state(task_ctx); }

        // 2. Test GET /drivers (empty list initially)
        let task_ctx2 = create_task_state();
        set_mock_field(task_ctx2, "request.path", "/drivers");
        set_mock_field(task_ctx2, "request.method", "GET");

        unsafe { crate::ox_plugin_process(instance_ptr, task_ctx2) };

        let body2 = get_mock_field(task_ctx2, "response.body").unwrap_or_default();
        assert!(body2.contains("[]") || body2.contains("drivers\":[]"), "driver list should be empty");
        unsafe { drop_task_state(task_ctx2); }

        // Cleanup
        unsafe { let _ = Box::from_raw(instance_ptr as *mut crate::ModuleContext); }
    }
}
