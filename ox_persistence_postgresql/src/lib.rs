use ox_persistence::{PersistenceDriver, DriverMetadata, DataSet, ConnectionParameter, ModuleCompatibility};
use ox_locking::LockStatus;
use ox_type_converter::ValueType;
use std::collections::HashMap;
use std::sync::Arc;
use ox_persistence_sql::SqlPersistenceDriver;
use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use serde_json;
use serde::{Serialize, Deserialize}; // Added for DriverMetadata serialization

pub struct PostgresqlDriver {
    sql_driver: SqlPersistenceDriver,
}

impl PersistenceDriver for PostgresqlDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        // Delegate to SqlPersistenceDriver
        self.sql_driver.persist(serializable_map, location)
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        // Delegate to SqlPersistenceDriver
        self.sql_driver.restore(location, id)
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
        // Delegate to SqlPersistenceDriver
        self.sql_driver.fetch(filter, location)
    }



    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("PostgresqlDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("\n--- Preparing PostgreSQL Datastore ---");
        println!("Connection Info: {:?}", connection_info);
        println!("--- PostgreSQL Datastore Prepared ---\n");
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        // TODO: Implement by querying pg_catalog.pg_tables
        Err("Not implemented for PostgreSQL driver yet.".to_string())
    }

    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, _dataset_name: &str) -> Result<DataSet, String> {
        // TODO: Implement by querying information_schema.columns
        Err("Not implemented for PostgreSQL driver yet.".to_string())
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "host".to_string(),
                description: "The PostgreSQL server host address.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: Some("localhost".to_string()),
            },
            ConnectionParameter {
                name: "port".to_string(),
                description: "The PostgreSQL server port.".to_string(),
                data_type: "integer".to_string(),
                is_required: false,
                default_value: Some("5432".to_string()),
            },
            ConnectionParameter {
                name: "database".to_string(),
                description: "The name of the PostgreSQL database.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "username".to_string(),
                description: "The username for PostgreSQL database access.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "password".to_string(),
                description: "The password for PostgreSQL database access.".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
            ConnectionParameter {
                name: "service_name".to_string(),
                description: "The PostgreSQL service name (alternative to host/port/db). If provided, other connection details might be ignored.".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
        ]
    }
}



// C-compatible function to get driver metadata
#[no_mangle]
pub extern "C" fn get_driver_metadata_json() -> *mut c_char {
    let mut compatible_modules = HashMap::new();
    compatible_modules.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "PostgreSQL Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_postgresql".to_string(),
        description: "A persistence driver for PostgreSQL databases.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

// C-compatible function to create a new driver instance
#[no_mangle]
pub extern "C" fn create_driver() -> *mut c_void {
    let driver = Arc::new(PostgresqlDriver {
        sql_driver: SqlPersistenceDriver,
    });
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