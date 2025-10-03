use std::fs;
use std::collections::HashMap;
use std::sync::Arc;
use serde::Deserialize;

use ox_persistence::{register_persistence_driver, get_registered_drivers, DriverMetadata, Persistent, PersistenceDriver};
use ox_persistence_mysql;
use ox_persistence_json;
use ox_persistence_sql;
use ox_persistence_flatfile;
use ox_persistence_mssql;
use ox_persistence_postgresql;
use ox_persistence_xml;
use ox_persistence_yaml;

// Struct to represent the datastore configuration from YAML
#[derive(Debug, Deserialize)]
struct DatastoreConfig {
    uuid: String,
    name: String,
    description: String,
    driver: String,
    connection_info: HashMap<String, String>,
}

fn main() {
    // Initialize all the drivers, which will register themselves
    ox_persistence_mysql::init();
    ox_persistence_json::init();
    ox_persistence_sql::init();
    ox_persistence_flatfile::init();
    ox_persistence_mssql::init();
    ox_persistence_postgresql::init();
    ox_persistence_xml::init();
    ox_persistence_yaml::init();

    println!("Registered persistence drivers:");
    let drivers = get_registered_drivers();
    for driver in drivers {
        println!("  - Name: {}, Version: {}, Description: {}", driver.name, driver.version, driver.description);
    }

    println!("\nReading datastore configurations...");

    let datastores_dir = "./examples/datastores";
    for entry in fs::read_dir(datastores_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.extension().map_or(false, |ext| ext == "yaml") {
            println!("\nProcessing config file: {}", path.display());
            let config_content = fs::read_to_string(&path).unwrap();
            let config: DatastoreConfig = serde_yaml::from_str(&config_content).unwrap();

            println!("  Datastore Name: {}", config.name);
            println!("  Driver: {}", config.driver);

            let registry = ox_persistence::PERSISTENCE_DRIVER_REGISTRY.lock().unwrap();
            if let Some((driver_arc, _)) = registry.get_driver(&config.driver) {
                match driver_arc.prepare_datastore(&config.connection_info) {
                    Ok(_) => println!("  Datastore prepared successfully."),
                    Err(e) => println!("  Error preparing datastore: {}", e),
                }
            } else {
                println!("  Error: Driver '{}' not found in registry.", config.driver);
            }
        }
    }
}
