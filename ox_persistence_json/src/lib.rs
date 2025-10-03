use ox_data_object::{
    GenericDataObject,
    AttributeValue,
};
use ox_persistence::{PersistenceDriver, register_persistence_driver, DriverMetadata};
use ox_locking::LockStatus;
use ox_type_converter::ValueType;
use std::fs::File;
use std::io::{Read, Write};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
struct SerializableAttributeValue {
    value: String,
    value_type: ValueType,
    value_type_parameters: HashMap<String, String>,
}

pub struct JsonDriver;

impl PersistenceDriver for JsonDriver {
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

        let json = serde_json::to_string_pretty(&serializable_data).map_err(|e| e.to_string())?;
        let mut file = File::create(location).map_err(|e| e.to_string())?;
        file.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn restore(
        &self,
        location: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let mut file = File::open(location).map_err(|e| e.to_string())?;
        let mut json = String::new();
        file.read_to_string(&mut json).map_err(|e| e.to_string())?;
        let deserialized_data: HashMap<String, SerializableAttributeValue> = 
            serde_json::from_str(&json).map_err(|e| e.to_string())?;

        let serializable_map: HashMap<String, (String, ValueType, HashMap<String, String>)> = 
            deserialized_data
                .into_iter()
                .map(|(key, serializable_attr)| {
                    (
                        key,
                        (
                            serializable_attr.value,
                            serializable_attr.value_type,
                            serializable_attr.value_type_parameters,
                        ),
                    )
                })
                .collect();
        Ok(serializable_map)
    }

    fn fetch(
        &self,
        _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        _location: &str,
    ) -> Result<Vec<HashMap<String, (String, ValueType, HashMap<String, String>)>>, String> {
        unimplemented!()
    }

    fn notify_lock_status_change(&self, lock_status: LockStatus, gdo_id: usize) {
        println!("JsonDriver: GDO {} lock status changed to {:?}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("\n--- Preparing JSON Datastore ---");
        println!("Connection Info: {:?}", connection_info);
        println!("--- JSON Datastore Prepared ---\n");
        Ok(())
    }
}

pub fn init() {
    let metadata = DriverMetadata {
        name: "json".to_string(),
        description: "A driver for JSON files.".to_string(),
        version: "0.1.0".to_string(),
    };
    register_persistence_driver(Arc::new(JsonDriver), metadata);
}
