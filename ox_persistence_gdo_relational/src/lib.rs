use ox_persistence::{PersistenceDriver, DriverMetadata, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, ModuleCompatibility, PERSISTENCE_DRIVER_REGISTRY};
use ox_type_converter::ValueType;
use std::collections::HashMap;
use std::sync::Arc;
use libc::{c_char, c_void};
use std::ffi::CString;
use serde_json;
 // Added for DriverMetadata serialization

pub struct GdoRelationalDriver {
    internal_driver_name: String,
    internal_location: String,
}

impl GdoRelationalDriver {
    pub fn new(internal_driver_name: String, internal_location: String) -> Self {
        Self { internal_driver_name, internal_location }
    }

    fn get_internal_driver(&self) -> Result<(Arc<dyn PersistenceDriver + Send + Sync>, DriverMetadata), String> {
        let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        registry.get_driver(&self.internal_driver_name)
            .ok_or_else(|| format!("Internal driver '{}' not found.", self.internal_driver_name))
    }
}

impl PersistenceDriver for GdoRelationalDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        _location: &str, // Location is handled by internal_location
    ) -> Result<(), String> {
        let (driver, _) = self.get_internal_driver()?;
        driver.persist(serializable_map, &self.internal_location)
    }

    fn restore(
        &self,
        _location: &str, // Location is handled by internal_location
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let (driver, _) = self.get_internal_driver()?;
        driver.restore(&self.internal_location, id)
    }

    fn fetch(
        &self,
        filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        _location: &str, // Location is handled by internal_location
    ) -> Result<Vec<String>, String> {
        let (driver, _) = self.get_internal_driver()?;
        driver.fetch(filter, &self.internal_location)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("GdoRelationalDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), String> {
        // The internal driver should be prepared separately
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        Ok(vec!["relationships".to_string()])
    }

    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, _dataset_name: &str) -> Result<DataSet, String> {
        // Define the schema for a relationship GDO
        Ok(DataSet {
            name: "relationships".to_string(),
            columns: vec![
                ColumnDefinition {
                    name: "id".to_string(),
                    data_type: "uuid".to_string(),
                    metadata: ColumnMetadata::default(),
                },
                ColumnDefinition {
                    name: "source_gdo_id".to_string(),
                    data_type: "uuid".to_string(),
                    metadata: ColumnMetadata::default(),
                },
                ColumnDefinition {
                    name: "source_driver_name".to_string(),
                    data_type: "string".to_string(),
                    metadata: ColumnMetadata::default(),
                },
                ColumnDefinition {
                    name: "source_location".to_string(),
                    data_type: "string".to_string(),
                    metadata: ColumnMetadata::default(),
                },
                ColumnDefinition {
                    name: "target_gdo_id".to_string(),
                    data_type: "uuid".to_string(),
                    metadata: ColumnMetadata::default(),
                },
                ColumnDefinition {
                    name: "target_driver_name".to_string(),
                    data_type: "string".to_string(),
                    metadata: ColumnMetadata::default(),
                },
                ColumnDefinition {
                    name: "target_location".to_string(),
                    data_type: "string".to_string(),
                    metadata: ColumnMetadata::default(),
                },
                ColumnDefinition {
                    name: "relationship_type".to_string(),
                    data_type: "string".to_string(),
                    metadata: ColumnMetadata::default(),
                },
                ColumnDefinition {
                    name: "relationship_name".to_string(),
                    data_type: "string".to_string(),
                    metadata: ColumnMetadata::default(),
                },
            ],
        })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "internal_driver_name".to_string(),
                description: "The name of the persistence driver to use for internal storage of relationships (e.g., 'json', 'yaml').".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "internal_location".to_string(),
                description: "The location string for the internal persistence driver (e.g., a file path for 'json').".to_string(),
                data_type: "string".to_string(),
                is_required: true,
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
            human_name: "GDO Relational Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_gdo_relational".to_string(),
        description: "A driver for managing relationships between GDOs across multiple datastores.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

// C-compatible function to create a new driver instance
#[no_mangle]
pub extern "C" fn create_driver() -> *mut c_void {
    // This driver needs configuration, so it cannot be instantiated without parameters.
    // The server will need to call a different function to create it with parameters.
    // For now, we return null to indicate it cannot be created generically.
    std::ptr::null_mut()
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