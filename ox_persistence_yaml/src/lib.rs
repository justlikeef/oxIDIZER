use ox_data_object::{
    GenericDataObject,
    AttributeValue,
};
use ox_persistence::{DataSet, ColumnDefinition, ColumnMetadata, PersistenceDriver, register_persistence_driver, DriverMetadata};

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

    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        let location = connection_info.get("path").ok_or("Missing 'path' in connection_info")?;
        let mut file = File::open(location).map_err(|e| e.to_string())?;
        let mut yaml_str = String::new();
        file.read_to_string(&mut yaml_str).map_err(|e| e.to_string())?;

        let yaml_value: serde_yaml::Value = serde_yaml::from_str(&yaml_str).map_err(|e| e.to_string())?;

        if let serde_yaml::Value::Mapping(map) = yaml_value {
            Ok(map.keys().filter_map(|k| k.as_str().map(String::from)).collect())
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
                    metadata: ColumnMetadata::default(), // Cannot infer metadata from YAML data
                });
            }
        }

        Ok(DataSet {
            name: dataset_name.to_string(),
            columns,
        })
    }
}
