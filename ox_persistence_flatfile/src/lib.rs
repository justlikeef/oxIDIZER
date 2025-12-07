use ox_persistence::{PersistenceDriver, DriverMetadata, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, ModuleCompatibility};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use ox_type_converter::ValueType;
use std::collections::HashMap;
use std::sync::Arc;
use libc::{c_char, c_void};
use std::ffi::CString;
use serde_json;

pub struct FlatfileDriver;

impl PersistenceDriver for FlatfileDriver {
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
        Err("Fetch not implemented for FlatfileDriver".to_string())
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("FlatfileDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("\n--- Preparing Flatfile Datastore ---");
        println!("Connection Info: {:?}", connection_info);
        println!("--- Flatfile Datastore Prepared ---\n");
        Ok(())
    }

    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        let path = connection_info.get("path").ok_or("Missing 'path' in connection_info")?;
        let entries = fs::read_dir(path).map_err(|e| e.to_string())?;
        
        let mut datasets = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                    datasets.push(filename.to_string());
                }
            }
        }
        Ok(datasets)
    }

    fn describe_dataset(&self, connection_info: &HashMap<String, String>, dataset_name: &str) -> Result<DataSet, String> {
        let dir_path = connection_info.get("path").ok_or("Missing 'path' in connection_info")?;
        let file_path = Path::new(dir_path).join(dataset_name);

        let file = fs::File::open(file_path).map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(file);
        
        let mut first_line = String::new();
        reader.read_line(&mut first_line).map_err(|e| e.to_string())?;

        let columns: Vec<ColumnDefinition> = first_line.trim()
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|name| ColumnDefinition {
                name: name.to_string(),
                data_type: "string".to_string(), // Cannot infer type from header, default to string
                metadata: ColumnMetadata::default(),
            })
            .collect();

        if columns.is_empty() {
            return Err(format!("Could not parse headers from file '{}'. It might be empty or not comma-delimited.", dataset_name));
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
                description: "The path to the directory containing the data files.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "definition_file".to_string(),
                description: "The path to the YAML file that defines the data structure.".to_string(),
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
            human_name: "Flatfile Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_flatfile".to_string(),
        description: "A persistence driver for flat files.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

// C-compatible function to create a new driver instance
#[no_mangle]
pub extern "C" fn create_driver() -> *mut c_void {
    let driver = Arc::new(FlatfileDriver);
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