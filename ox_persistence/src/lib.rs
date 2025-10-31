use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ox_data_object::generic_data_object::{GenericDataObject};
use ox_type_converter::ValueType;
use serde::{Deserialize, Serialize};

/// Represents the hydration and persistence state of a GenericDataObject.
#[derive(Debug, Clone, PartialEq)]
pub enum DataObjectState {
    /// The object was newly created in memory and does not exist in the datastore.
    New,
    /// The object is a shell, containing only an ID. It represents a full object in the datastore.
    NotHydrated,
    /// The object is fully loaded from the datastore.
    Hydrated,
    /// The object has been modified in memory and is out of sync with the datastore.
    Modified,
    /// The object in memory is in sync with the datastore.
    Consistent,
    /// The object is marked for deletion from the datastore.
    Deleted,
}

/// Holds information required for a GenericDataObject to self-hydrate.
#[derive(Debug, Clone)]
pub struct PersistenceInfo {
    pub driver_name: String,
    pub location: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSet {
    pub name: String,
    pub columns: Vec<ColumnDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDefinition {
    pub name: String,
    pub data_type: String, // Using String for flexibility, could be an enum
    pub metadata: ColumnMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ColumnMetadata {
    pub is_unique: Option<bool>,
    pub max_length: Option<u32>,
    pub precision: Option<u8>,
    pub scale: Option<u8>, // For decimal types
    // Using a HashMap for any other driver-specific metadata
    pub additional: HashMap<String, String>,
}

/// Describes a single connection parameter required by a persistence driver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionParameter {
    pub name: String,
    pub description: String,
    pub data_type: String, // e.g., "string", "integer", "boolean"
    pub is_required: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleCompatibility {
    pub human_name: String,
    pub crate_type: String,
    pub version: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct DriverMetadata {
    pub name: String, // The crate name, e.g., "ox_persistence_flatfile"
    pub version: String, // The crate version
    pub description: String,
    pub compatible_modules: HashMap<String, ModuleCompatibility>,
}

pub struct PersistenceDriverRegistry {
    drivers: HashMap<String, (Arc<dyn PersistenceDriver + Send + Sync>, DriverMetadata)>,
}

impl PersistenceDriverRegistry {
    fn new() -> Self {
        Self { drivers: HashMap::new() }
    }

    fn register_driver(&mut self, driver: Arc<dyn PersistenceDriver + Send + Sync>, metadata: DriverMetadata) {
        self.drivers.insert(metadata.name.clone(), (driver, metadata));
    }

    pub fn get_driver(&self, name: &str) -> Option<(Arc<dyn PersistenceDriver + Send + Sync>, DriverMetadata)> {
        self.drivers.get(name).cloned()
    }

    fn get_all_drivers(&self) -> Vec<DriverMetadata> {
        self.drivers.values().map(|(_, meta)| meta.clone()).collect()
    }
}

lazy_static::lazy_static! {
    pub static ref PERSISTENCE_DRIVER_REGISTRY: Mutex<PersistenceDriverRegistry> = Mutex::new(PersistenceDriverRegistry::new());
}

pub fn register_persistence_driver(driver: Arc<dyn PersistenceDriver + Send + Sync>, metadata: DriverMetadata) {
    PERSISTENCE_DRIVER_REGISTRY.lock().unwrap().register_driver(driver, metadata);
}

pub fn get_registered_drivers() -> Vec<DriverMetadata> {
    PERSISTENCE_DRIVER_REGISTRY.lock().unwrap().get_all_drivers()
}

pub fn unregister_persistence_driver(driver_name: &str) {
    PERSISTENCE_DRIVER_REGISTRY.lock().unwrap().drivers.remove(driver_name);
}


/// A trait for objects that can be persisted.
pub trait Persistent {
    fn persist(&mut self, driver_name: &str, location: &str) -> Result<(), String>;
    fn fetch(&self, driver_name: &str, location: &str) -> Result<Vec<GenericDataObject>, String>;

    /// Hydrates the object by loading its full data from the datastore.
    fn hydrate_object(&mut self, driver_name: &str, location: &str) -> Result<(), String>;
}

/// A trait for drivers that can persist and restore a GenericDataObject
pub trait PersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String>;

    /// Restores a single object by its unique ID.
    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String>;

    /// Fetches the unique IDs of objects matching a filter.
    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String>;

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str);

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String>;

    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, String>;
    fn describe_dataset(&self, connection_info: &HashMap<String, String>, dataset_name: &str) -> Result<DataSet, String>;

/// Gets the definition of connection parameters required by the driver.
    fn get_connection_parameters(&self) -> Vec<ConnectionParameter>;
}