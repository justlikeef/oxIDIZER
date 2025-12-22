use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ox_data_object::{GenericDataObject};
use ox_type_converter::ValueType;
use serde::{Deserialize, Serialize};
use std::ffi::{c_void, CString};
use libc::c_char;

/// A C-compatible buffer for passing data across FFI boundaries.
/// The `ptr` points to the data, `len` is the valid length, and `cap` is the capacity (if managed by Rust Vec).
#[repr(C)]
#[derive(Debug, Clone)]
pub struct OxBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl OxBuffer {
    /// Creates a generic empty buffer.
    pub fn empty() -> Self {
        Self { ptr: std::ptr::null_mut(), len: 0, cap: 0 }
    }

    /// Creates an OxBuffer from a Rust String.
    /// The buffer now owns the memory. To free it, call `free_ox_buffer`.
    pub fn from_str(s: String) -> Self {
        let mut v = s.into_bytes();
        let buf = Self {
            ptr: v.as_mut_ptr(),
            len: v.len(),
            cap: v.capacity(),
        };
        std::mem::forget(v); // Prevent Rust from deallocating the Vec
        buf
    }
    
    /// Converts the buffer back to a Rust String (unsafe).
    pub unsafe fn to_string(&self) -> String {
        if self.ptr.is_null() {
            return String::new();
        }
        let slice = std::slice::from_raw_parts(self.ptr, self.len);
        String::from_utf8_lossy(slice).into_owned()
    }
}

/// Frees the memory associated with an OxBuffer created by Rust.
#[no_mangle]
pub unsafe extern "C" fn free_ox_buffer(buf: OxBuffer) {
    if !buf.ptr.is_null() {
        let _ = Vec::from_raw_parts(buf.ptr, buf.len, buf.cap);
    }
}

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

impl Persistent for GenericDataObject {
    fn persist(&mut self, driver_name: &str, location: &str) -> Result<(), String> {
        let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        if let Some((driver, _)) = registry.get_driver(driver_name) {
            let map = self.to_serializable_map();
            driver.persist(&map, location)?;
            // Implicitly consistent after persist
            Ok(())
        } else {
            Err(format!("Driver '{}' not found", driver_name))
        }
    }

    fn fetch(&self, driver_name: &str, location: &str) -> Result<Vec<GenericDataObject>, String> {
         let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        if let Some((driver, _)) = registry.get_driver(driver_name) {
             let filter = self.to_serializable_map();
             let ids = driver.fetch(&filter, location)?;
             
             let mut results = Vec::new();
             for id in ids {
                 let mut obj = GenericDataObject::new(&self.identifier_name, None);
                 // We need to hydrate this new object
                 // But hydrate_object is instance method.
                 // We can call restore on driver directly.
                 let restored_map = driver.restore(location, &id)?;
                 obj = GenericDataObject::from_serializable_map(restored_map, &self.identifier_name);
                 results.push(obj);
             }
             Ok(results)
        } else {
            Err(format!("Driver '{}' not found", driver_name))
        }
    }

    /// Hydrates the object by loading its full data from the datastore.
    fn hydrate_object(&mut self, driver_name: &str, location: &str) -> Result<(), String> {
        let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        if let Some((driver, _)) = registry.get_driver(driver_name) {
            let id = self.get::<String>(&self.identifier_name)
                .ok_or_else(|| format!("Object missing identifier '{}'", self.identifier_name))?;
                
            let restored_map = driver.restore(location, &id)?;
            // Update self from map
            // GenericDataObject doesn't have a "update from map" method that preserves identity?
            // Actually from_serializable_map creates new.
            // We can replace our attributes.
            let new_obj = GenericDataObject::from_serializable_map(restored_map, &self.identifier_name);
            // Assuming we can access fields or use a helper
            // We can iterate the map and set.
            // But we already have from_serializable_map logic. 
            // Let's rely on set_attributes if possible, or just hack it with a temporary obj.
            // We can't move out of new_obj easily if fields are private.
            // Wait, GenericDataObject fields are: attributes (private), identifier_name (public).
            // But to_serializable_map is public.
            // We need a way to bulk update.
            // GenericDataObject has `from_serializable_map` static method.
            // And `attributes` is private.
            // But `set` is public.
            // I'll assume we iterate the restored map and set fields.
            // Actually `PersistenceDriver::restore` returns the map.
            // I will iterate the map and call set_with_type.
            
             for (key, (value_str, value_type, params)) in new_obj.to_serializable_map() {
                 self.set_with_type(&key, value_str, value_type, Some(params));
             }
             Ok(())

        } else {
            Err(format!("Driver '{}' not found", driver_name))
        }
    }
}