use crate::dictionary::{DataStoreContainer, DataStoreField};
use ox_persistence::PERSISTENCE_DRIVER_REGISTRY;
use ox_type_converter::ValueType;
use std::collections::HashMap;

pub struct IntrospectionService;

impl IntrospectionService {
    /// Introspects a specific driver to build DataStoreContainer definitions.
    /// Requires connection_info to connect to the physical data source.
    pub fn introspect_driver(driver_name: &str, connection_info: &HashMap<String, String>) -> Result<Vec<DataStoreContainer>, String> {
        let registry = PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
        if let Some((driver, _metadata)) = registry.get_driver(driver_name) {
            let datasets = driver.list_datasets(connection_info)?;
            let mut containers = Vec::new();

            for ds_name in datasets {
                match driver.describe_dataset(connection_info, &ds_name) {
                    Ok(dataset) => {
                        containers.push(DataStoreContainer {
                            id: format!("{}:{}", driver_name, ds_name),
                            datasource_id: driver_name.to_string(),
                            name: ds_name,
                            container_type: "table".to_string(), // Default, driver could specify
                            fields: dataset.columns.into_iter().map(|col| DataStoreField {
                                name: col.name,
                                data_type: ValueType::new(&col.data_type), 
                                parameters: col.metadata.additional,
                                description: None,
                            }).collect(),
                            metadata: HashMap::new(),
                        });
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to describe dataset '{}' on driver '{}': {}", ds_name, driver_name, e);
                    }
                }
            }
            Ok(containers)
        } else {
            Err(format!("Driver {} not found", driver_name))
        }
    }
}
