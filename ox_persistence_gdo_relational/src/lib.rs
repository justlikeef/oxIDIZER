use ox_persistence::{
    PersistenceDriver, DriverMetadata, DataObjectState, PersistenceInfo,
    DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, PERSISTENCE_DRIVER_REGISTRY
};
use ox_type_converter::ValueType;
use ox_locking::LockStatus;
use std::collections::HashMap;
use std::sync::Arc;

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

pub fn init() {
    let metadata = DriverMetadata {
        name: "gdo_relational".to_string(),
        description: "A driver for managing relationships between GDOs across multiple datastores.".to_string(),
        version: "0.1.0".to_string(),
    };
    // This driver needs configuration, so it cannot be registered with a default instance.
    // It should be instantiated and registered by the application with its specific internal_driver_name and internal_location.
    // For demonstration, we'll register a dummy one.
    // register_persistence_driver(Arc::new(GdoRelationalDriver::new("json".to_string(), "relationships.json".to_string())), metadata);
}
