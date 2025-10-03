use ox_data_object::{
    GenericDataObject,
    AttributeValue,
};
use ox_persistence::{PersistenceDriver, register_persistence_driver, DriverMetadata, DataSet, ColumnDefinition, ColumnMetadata};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use ox_locking::LockStatus;
use ox_type_converter::ValueType;
use std::collections::HashMap;
use std::sync::Arc;

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
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        Err("Not implemented".to_string())
    }

    fn fetch(
        &self,
        _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        _location: &str,
    ) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>, String> {
        unimplemented!()
    }

    fn notify_lock_status_change(&self, lock_status: LockStatus, gdo_id: usize) {
        println!("FlatfileDriver: GDO {} lock status changed to {:?}", gdo_id, lock_status);
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
}

pub fn init() {
    let metadata = DriverMetadata {
        name: "flatfile".to_string(),
        description: "A driver for flat files.".to_string(),
        version: "0.1.0".to_string(),
    };
    register_persistence_driver(Arc::new(FlatfileDriver), metadata);
}
