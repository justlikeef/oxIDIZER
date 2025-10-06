use axum::{routing::get, Json, Router};
use serde::Serialize;
use std::net::SocketAddr;
use tokio::net::TcpListener;

// Import all persistence drivers to ensure their init() functions are available
use ox_persistence::{get_registered_drivers, DriverMetadata};
use ox_persistence_api;
use ox_persistence_flatfile;
use ox_persistence_json;
use ox_persistence_mssql;
use ox_persistence_mysql;
use ox_persistence_postgresql;
use ox_persistence_sql;
use ox_persistence_xml;
use ox_persistence_yaml;
use ox_persistence_gdo_relational;

// Function to initialize all persistence drivers
fn init_drivers() {
    ox_persistence_flatfile::init();
    ox_persistence_json::init();
    ox_persistence_mssql::init();
    ox_persistence_mysql::init();
    ox_persistence_postgresql::init();
    ox_persistence_sql::init();
    ox_persistence_xml::init();
    ox_persistence_yaml::init();
    // Note: ox_persistence_gdo_relational::init() is not called here
    // as it requires configuration and explicit registration.
}

#[tokio::main]
async fn main() {
    // Initialize all persistence drivers
    init_drivers();

    // Build our application with a single route
    let app = Router::new().route("/drivers", get(list_drivers_handler));

    // Run it
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("listening on {}", addr);
    let listener = TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// Handler to return a list of registered drivers
async fn list_drivers_handler() -> Json<Vec<DriverMetadata>> {
    let drivers = get_registered_drivers();
    Json(drivers)
}