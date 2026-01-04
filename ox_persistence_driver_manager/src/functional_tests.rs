
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
         use ox_webservice_api::{CoreHostApi, ModuleInterface, PipelineState};
         use ox_webservice_test_utils::{create_mock_api, create_stub_pipeline_state};
         use std::sync::{Arc, Mutex};
         
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
        };
        
        // Construct Context manually since we can't easily use initialize_module from within test without raw pointer gymnastics
        let api = Box::leak(Box::new(create_mock_api())); // Leak for static lifetime
        let manager = Arc::new(DriverManager::new(config));
        
        let context = crate::ModuleContext {
            manager: manager.clone(),
            api: api,
            module_id: "test_driver_manager".to_string(),
        };
        
        // We need to cast it to c_void
        let context_box = Box::new(context);
        let instance_ptr = Box::into_raw(context_box) as *mut libc::c_void;
        
        // 1. Test GET /drivers/available
        let mut ps = create_stub_pipeline_state();
        ps.request_path = "/drivers/available".to_string();
        ps.request_method = "GET".to_string();
        
        let result = unsafe { crate::process_request(instance_ptr, &mut ps as *mut _, api.log_callback, api.alloc_raw, std::ptr::null()) };
        
        assert_eq!(result.status, ox_webservice_api::ModuleStatus::Modified);
        let body = String::from_utf8_lossy(&ps.response_body);
        assert!(body.contains("db/test.so"));
        
        // 2. Test GET /drivers (empty list initially)
        let mut ps2 = create_stub_pipeline_state();
        ps2.request_path = "/drivers".to_string();
        ps2.request_method = "GET".to_string();
        
        let result2 = unsafe { crate::process_request(instance_ptr, &mut ps2 as *mut _, api.log_callback, api.alloc_raw, std::ptr::null()) };
         assert_eq!(result2.status, ox_webservice_api::ModuleStatus::Modified);
        let body2 = String::from_utf8_lossy(&ps2.response_body);
        assert!(body2.contains("[]") || body2.contains("drivers\":[]"));

        // Cleanup: reconstruct box to drop
        unsafe { let _ = Box::from_raw(instance_ptr as *mut crate::ModuleContext); }
    }
}
