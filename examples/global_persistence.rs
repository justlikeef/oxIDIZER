use ox_data_object::GenericDataObject;
use ox_locking::{Lockable, LockableGenericDataObject};
use ox_persistence::{get_registered_drivers, Persistent};
use ox_persistence_driver_db_mysql;
use ox_persistence_driver_file_json;
// use ox_persistence_driver_db_sql;
// use ox_persistence_driver_file_delimited;
// use ox_persistence_driver_db_mssql;
// use ox_persistence_driver_db_postgres;
// use ox_persistence_driver_file_xml;
// use ox_persistence_driver_file_yaml;

fn main() {
    // Initialize available drivers
    ox_persistence_driver_db_mysql::MysqlPersistenceDriver::register();
    ox_persistence_driver_file_json::JsonPersistenceDriver::register();
    
    // Other drivers not yet updated for static registration:
    // ox_persistence_driver_db_sql::init(); 
    // ox_persistence_driver_file_delimited::init();
    // ox_persistence_driver_db_mssql::init();
    // ox_persistence_driver_db_postgres::init();
    // ox_persistence_driver_file_xml::init();
    // ox_persistence_driver_file_yaml::init();

    // Get and print the list of registered drivers
    println!("Registered persistence drivers:");
    let drivers = get_registered_drivers();
    for driver in drivers {
        println!("  - Name: {}, Version: {}, Description: {}", driver.name, driver.version, driver.description);
    }

    // Now, use one of the drivers to persist an object
    let gdo = GenericDataObject::new("id", None);
    let mut locked_gdo = LockableGenericDataObject::new(gdo);
    
    // Lockable set logic now requires accessing the inner GDO or explicit status setting.
    // Assuming simple set for this example.
    locked_gdo.get_gdo_mut().set("name", "John Doe".to_string());
    locked_gdo.get_gdo_mut().set("age", 30);
    locked_gdo.get_gdo_mut().set("city", "New York".to_string());

    println!("\nPersisting object to JSON...");
    match locked_gdo.get_gdo_mut().persist("json", "my_data.json") {
        Ok(_) => println!("Successfully persisted data."),
        Err(e) => println!("Error: {}", e),
    }

    println!("\nPersisting object to SQL...");
    match locked_gdo.get_gdo_mut().persist("sql", "my_sql_data") {
        Ok(_) => println!("Successfully persisted data."),
        Err(e) => println!("Error: {}", e),
    }
}
