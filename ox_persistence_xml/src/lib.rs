use ox_data_object::{
    GenericDataObject,
    AttributeValue,
};
use ox_persistence::{PersistenceDriver, register_persistence_driver, DriverMetadata, DataSet, ColumnDefinition, ColumnMetadata};
use std::io::BufReader;
use xml::reader::{EventReader, XmlEvent};
use ox_locking::LockStatus;
use ox_type_converter::ValueType;
use std::fs::File;
use std::io::{Read, Write};
use std::collections::HashMap;
use std::sync::Arc;

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
        println!("XmlDriver: GDO {} lock status changed to {:?}", gdo_id, lock_status);
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
}

pub fn init() {
    let metadata = DriverMetadata {
        name: "xml".to_string(),
        description: "A driver for XML files.".to_string(),
        version: "0.1.0".to_string(),
    };
    register_persistence_driver(Arc::new(XmlDriver), metadata);
}
