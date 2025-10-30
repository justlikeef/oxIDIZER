use ox_persistence::{PersistenceDriver, DriverMetadata, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, ModuleCompatibility};
use std::io::BufReader;
use xml::reader::{EventReader, XmlEvent};
use ox_locking::LockStatus;
use ox_type_converter::ValueType;
use std::fs::File;
use std::collections::HashMap;
use std::sync::Arc;
use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use serde_json;
use serde::{Serialize, Deserialize}; // Added for DriverMetadata serialization

pub struct XmlDriver;

impl PersistenceDriver for XmlDriver {
    fn persist(
        &self,
        _serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        _location: &str,
    ) -> Result<(), String> {
        Err("Not implemented".to_string())
    }

    fn restore(
        &self,
        _location: &str,
        _id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        Err("Not implemented".to_string())
    }

    fn fetch(&self, _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, _location: &str) -> Result<Vec<String>, String> {
        Err("Fetch not implemented for XmlDriver".to_string())
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("XmlDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("\n--- Preparing XML Datastore ---");
        println!("Connection Info: {:?}", connection_info);
        println!("--- XML Datastore Prepared ---\n");
        Ok(())
    }

    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        let location = connection_info.get("path").ok_or("Missing 'path' in connection_info")?;
        let file = File::open(location).map_err(|e| e.to_string())?;
        let file = BufReader::new(file);
        let parser = EventReader::new(file);

        let mut depth = 0;
        let mut datasets = std::collections::HashSet::new();

        for e in parser {
            match e {
                Ok(XmlEvent::StartElement { name, .. }) => {
                    depth += 1;
                    if depth == 2 { // Direct children of the root element
                        datasets.insert(name.local_name);
                    }
                }
                Ok(XmlEvent::EndElement { .. }) => {
                    depth -= 1;
                }
                Err(e) => return Err(format!("XML parsing error: {}", e)),
                _ => {}
            }
        }
        Ok(datasets.into_iter().collect())
    }

    fn describe_dataset(&self, connection_info: &HashMap<String, String>, dataset_name: &str) -> Result<DataSet, String> {
        let location = connection_info.get("path").ok_or("Missing 'path' in connection_info")?;
        let file = File::open(location).map_err(|e| e.to_string())?;
        let file = BufReader::new(file);
        let parser = EventReader::new(file);

        let mut depth = 0;
        let mut in_dataset_element = false;
        let mut columns = Vec::new();
        let mut found_dataset = false;

        for e in parser {
            match e {
                Ok(XmlEvent::StartElement { name, .. }) => {
                    depth += 1;
                    if depth == 2 && name.local_name == dataset_name {
                        in_dataset_element = true;
                        found_dataset = true;
                    } else if in_dataset_element && depth == 3 {
                        columns.push(ColumnDefinition {
                            name: name.local_name,
                            data_type: "string".to_string(),
                            metadata: ColumnMetadata::default(),
                        });
                    }
                }
                Ok(XmlEvent::EndElement { name, .. }) => {
                    if depth == 2 && name.local_name == dataset_name {
                        // We have collected all columns from the first item, so we can stop.
                        break;
                    }
                    if in_dataset_element && depth == 3 {
                        // End of a column element
                    }
                    depth -= 1;
                }
                Err(e) => return Err(format!("XML parsing error: {}", e)),
                _ => {}
            }
        }

        if !found_dataset {
            return Err(format!("Dataset '{}' not found in XML file.", dataset_name));
        }

        Ok(DataSet {
            name: dataset_name.to_string(),
            columns,
        })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "path".to_string(),
                description: "The path to the XML data file.".to_string(),
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
            human_name: "XML Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_xml".to_string(),
        description: "A persistence driver for XML files.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

// C-compatible function to create a new driver instance
#[no_mangle]
pub extern "C" fn create_driver() -> *mut c_void {
    let driver = Arc::new(XmlDriver);
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