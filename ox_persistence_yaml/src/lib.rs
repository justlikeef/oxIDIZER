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
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let mut file = File::open(location).map_err(|e| e.to_string())?;
        let mut yaml = String::new();
        file.read_to_string(&mut yaml).map_err(|e| e.to_string())?;
        let deserialized_data: HashMap<String, SerializableAttributeValue> = 
            serde_yaml::from_str(&yaml).map_err(|e| e.to_string())?;

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
        println!("YamlDriver: GDO {} lock status changed to {:?}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("\n--- Preparing YAML Datastore ---");
        println!("Connection Info: {:?}", connection_info);
        println!("--- YAML Datastore Prepared ---\n");
        Ok(())
    }
}
