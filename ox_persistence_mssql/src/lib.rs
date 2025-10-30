use ox_persistence::{PersistenceDriver, DriverMetadata, DataSet, ConnectionParameter, ModuleCompatibility};
use ox_locking::LockStatus;
use ox_type_converter::ValueType;
use std::collections::HashMap;
use std::sync::Arc;
use ox_persistence_sql::{SqlPersistenceDriver, GenericSqlDriver};
use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use serde_json;
use serde::{Serialize, Deserialize}; // Added for DriverMetadata serialization

pub struct MssqlDriver;

impl PersistenceDriver for MssqlDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        // Delegate to GenericSqlDriver
        GenericSqlDriver.persist(serializable_map, location)
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        // Delegate to GenericSqlDriver
        GenericSqlDriver.restore(location, id)
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
        // Delegate to GenericSqlDriver
        GenericSqlDriver.fetch(filter, location)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("MssqlDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("\n--- Preparing MSSQL Datastore ---");
        println!("Connection Info: {:?}", connection_info);
        println!("--- MSSQL Datastore Prepared ---\\n");
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        // TODO: Implement by querying INFORMATION_SCHEMA.TABLES
        Err("Not implemented for MSSQL driver yet.".to_string())
    }

    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, _dataset_name: &str) -> Result<DataSet, String> {
        // TODO: Implement by querying INFORMATION_SCHEMA.COLUMNS
        Err("Not implemented for MSSQL driver yet.".to_string())
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "host".to_string(),
                description: "The MSSQL server host address.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: Some("localhost".to_string()),
            },
            ConnectionParameter {
                name: "port".to_string(),
                description: "The MSSQL server port.".to_string(),
                data_type: "integer".to_string(),
                is_required: false,
                default_value: Some("1433".to_string()),
            },
            ConnectionParameter {
                name: "database".to_string(),
                description: "The name of the MSSQL database.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "username".to_string(),
                description: "The username for MSSQL database access.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "password".to_string(),
                description: "The password for MSSQL database access.".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
            ConnectionParameter {
                name: "instance_name".to_string(),
                description: "The MSSQL instance name (if not using default port/instance).".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
        ]
    }
}

impl SqlPersistenceDriver for MssqlDriver {
    fn execute_query(&self, query: &str, params: &HashMap<String, String>) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>, String> {
        // This would be the actual MSSQL specific query execution
        println!("Executing MSSQL query: {}", query);
        println!("With params: {:?}", params);
        // Dummy implementation
        Ok(vec![])
    }

    fn build_where_clause(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>) -> (String, HashMap<String, String>) {
        GenericSqlDriver.build_where_clause(filter)
    }
}

// C-compatible function to get driver metadata
#[no_mangle]
pub extern "C" fn get_driver_metadata_json() -> *mut c_char {
    let mut compatible_modules = HashMap::new();
    compatible_modules.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "MS SQL Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_mssql".to_string(),
        description: "A persistence driver for Microsoft SQL Server.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

// C-compatible function to create a new driver instance
#[no_mangle]
pub extern "C" fn create_driver() -> *mut c_void {
    let driver = Arc::new(MssqlDriver);
    let trait_object: Arc<dyn PersistenceDriver + Send + Sync> = driver;
    Box::into_raw(Box::new(trait_object)) as *mut c_void
}

// C-compatible function to destroy a driver instance
#[no_mangle]
pub extern "C" fn destroy_driver(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        // Reconstruct the Box and let it drop
        let _ = Box::from_raw(ptr as *mut Arc<dyn PersistenceDriver + Send + Sync>);
    }
}

// The init function is no longer responsible for registering the driver directly.
// It can be used for any other initialization logic if needed.
#[no_mangle]
pub extern "C" fn init() {
    // No direct registration here, as the server will handle it after loading
    // the driver dynamically.
}