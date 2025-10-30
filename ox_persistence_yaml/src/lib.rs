use ox_data_object::generic_data_object::AttributeValue;
use ox_persistence::{DataSet, ColumnDefinition, ColumnMetadata, PersistenceDriver, DriverMetadata, ConnectionParameter, ModuleCompatibility};
use ox_locking::LockStatus;
use ox_type_converter::ValueType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::Arc;
use libc::{c_char, c_void};
use std::ffi::{CStr, CString};
use serde_json; // For serializing metadata to JSON

#[derive(Serialize, Deserialize)]
struct SerializableAttributeValue {
    value: String,
    value_type: ValueType,
    value_type_parameters: HashMap<String, String>,
}

pub struct YamlDriver;

impl PersistenceDriver for YamlDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        let serializable_data: HashMap<String, SerializableAttributeValue> = serializable_map
            .iter()
            .map(|(key, (value, value_type, params))| {
                (
                    key.clone(),
                    SerializableAttributeValue {
                        value: value.clone(),
                        value_type: value_type.clone(),
                        value_type_parameters: params.clone(),
                    },
                )
            })
            .collect();

        let yaml = serde_yaml::to_string(&serializable_data).map_err(|e| e.to_string())?;
        let mut file = File::create(location).map_err(|e| e.to_string())?;
        file.write_all(yaml.as_bytes()).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let parts: Vec<&str> = location.splitn(2, ':').collect();
        let (file_path, dataset_name) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            return Err("Location for restore must be in 'filepath:dataset' format".to_string());
        };

        let file = File::open(file_path).map_err(|e| e.to_string())?;
        let yaml_value: serde_yaml::Value = serde_yaml::from_reader(file).map_err(|e| e.to_string())?;

        let root_map = yaml_value.as_mapping().ok_or("YAML root is not a mapping")?;
        let dataset_seq = root_map
            .get(&serde_yaml::Value::String(dataset_name.to_string()))
            .and_then(|v| v.as_sequence())
            .ok_or(format!("Dataset '{}' not found or is not a sequence", dataset_name))?;

        for item in dataset_seq {
            if let Some(map) = item.as_mapping() {
                if let Some(id_val) = map.get(&serde_yaml::Value::String("id".to_string())) {
                    if id_val.as_str() == Some(id) {
                        let mut serializable_map = HashMap::new();
                        for (key, value) in map {
                            if let Some(key_str) = key.as_str() {
                                let value_str = value.as_str().unwrap_or("").to_string();
                                let value_type = match value {
                                    serde_yaml::Value::Number(_) => ValueType::new("float"),
                                    serde_yaml::Value::Bool(_) => ValueType::new("boolean"),
                                    _ => ValueType::new("string"),
                                };
                                serializable_map.insert(key_str.to_string(), (value_str, value_type, HashMap::new()));
                            }
                        }
                        return Ok(serializable_map);
                    }
                }
            }
        }

        Err(format!("Object with id '{}' not found in dataset '{}'", id, dataset_name))
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
        let dataset_name = filter.keys().next().ok_or("Filter must contain a dataset name".to_string())?;
        
        let file = File::open(location).map_err(|e| e.to_string())?;
        let yaml_value: serde_yaml::Value = serde_yaml::from_reader(file).map_err(|e| e.to_string())?;
        
        let root_map = yaml_value.as_mapping().ok_or("YAML root is not a mapping")?;
        let dataset_seq = root_map
            .get(&serde_yaml::Value::String(dataset_name.clone()))
            .and_then(|v| v.as_sequence())
            .ok_or(format!("Dataset '{}' not found or is not a sequence", dataset_name))?;

        let mut ids = Vec::new();
        for item in dataset_seq {
            if let Some(map) = item.as_mapping() {
                if let Some(id_val) = map.get(&serde_yaml::Value::String("id".to_string())) {
                    if let Some(id_str) = id_val.as_str() {
                        ids.push(id_str.to_string());
                    }
                }
            }
        }
        Ok(ids)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("YamlDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("\n--- Preparing YAML Datastore ---");
        println!("Connection Info: {:?}", connection_info);
        println!("--- YAML Datastore Prepared ---\n");
        Ok(())
    }

    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        let location = connection_info.get("path").ok_or("Missing 'path' in connection_info")?;
        let mut file = File::open(location).map_err(|e| e.to_string())?;
        let mut yaml_str = String::new();
        file.read_to_string(&mut yaml_str).map_err(|e| e.to_string())?;

        let yaml_value: serde_yaml::Value = serde_yaml::from_str(&yaml_str).map_err(|e| e.to_string())?;

        if let serde_yaml::Value::Mapping(map) = yaml_value {
            Ok(map.iter().map(|(k, _v)| k).filter_map(|k| k.as_str().map(String::from)).collect())
        } else {
            Err("YAML root is not a mapping (object)".to_string())
        }
    }

    fn describe_dataset(&self, connection_info: &HashMap<String, String>, dataset_name: &str) -> Result<DataSet, String> {
        let location = connection_info.get("path").ok_or("Missing 'path' in connection_info")?;
        let mut file = File::open(location).map_err(|e| e.to_string())?;
        let mut yaml_str = String::new();
        file.read_to_string(&mut yaml_str).map_err(|e| e.to_string())?;

        let yaml_value: serde_yaml::Value = serde_yaml::from_str(&yaml_str).map_err(|e| e.to_string())?;
        
        let root_map = yaml_value.as_mapping().ok_or("YAML root is not a mapping")?;
        let dataset_value = root_map.get(&serde_yaml::Value::String(dataset_name.to_string()))
            .ok_or(format!("Dataset '{}' not found in YAML file", dataset_name))?;

        let dataset_seq = dataset_value.as_sequence().ok_or(format!("Dataset '{}' is not a sequence (array)", dataset_name))?;
        let first_item = dataset_seq.get(0).ok_or(format!("Dataset '{}' is empty", dataset_name))?;
        let item_map = first_item.as_mapping().ok_or(format!("Items in dataset '{}' are not mappings (objects)", dataset_name))?;

        let mut columns = Vec::new();
        for (key, value) in item_map {
            if let Some(name) = key.as_str() {
                let data_type = match value {
                    serde_yaml::Value::Null => "null",
                    serde_yaml::Value::Bool(_) => "boolean",
                    serde_yaml::Value::Number(_) => "numeric",
                    serde_yaml::Value::String(_) => "string",
                    serde_yaml::Value::Sequence(_) => "sequence",
                    serde_yaml::Value::Mapping(_) => "mapping",
                    _ => "unknown",
                }.to_string();

                columns.push(ColumnDefinition {
                    name: name.to_string(),
                    data_type,
                    metadata: ColumnMetadata::default(),
                });
            }
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
                description: "The path to the YAML data file.".to_string(),
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
            human_name: "YAML Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_yaml".to_string(),
        description: "A persistence driver for YAML files.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

// C-compatible function to create a new driver instance
#[no_mangle]
pub extern "C" fn create_driver() -> *mut c_void {
    let driver = Arc::new(YamlDriver);
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
