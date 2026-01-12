use ox_persistence::{PersistenceDriver, DriverMetadata, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, ModuleCompatibility};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use ox_type_converter::ValueType;
use std::collections::HashMap;
use std::sync::Arc;
use libc::{c_char, c_void};
use std::ffi::CStr;
use serde_json;

pub struct FlatfileDriver;

impl PersistenceDriver for FlatfileDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        // location is the directory. We need to find the dataset name from the serializable_map?
        // Or wait, PersistenceDriver::persist takes a map. 
        // The map corresponds to a GenericDataObject. 
        // Usually GDO has a 'table' or 'dataset' attribute?
        // Looking at ox_persistence traits, persist takes just the map and location.
        // Location for flatfile is the directory path (from connection info).
        // But we need to know WHICH file to write to.
        // Assumption: The map has a "_dataset" or similar key? 
        // Or maybe 'location' argument passed to persist() IS the file path?
        // The trait definition says: fn persist(&mut self, driver_name: &str, location: &str) -> ... in Persistent trait.
        // In Drive trait: fn persist(..., location: &str).
        
        // Let's assume 'location' passed here is the full path to the DATASET file for this object.
        // If not, we'd need a way to derive it.
        
        let file_path = Path::new(location);
        
        // We need to read the header first to know column order
        let file = fs::File::open(file_path).map_err(|e| format!("Failed to open file {}: {}", location, e))?;
        let mut reader = csv::ReaderBuilder::new().has_headers(true).from_reader(&file);
        let headers = reader.headers().map_err(|e| e.to_string())?.clone();

        // Re-open in append mode
        let file_append = fs::OpenOptions::new()
            .write(true)
            .append(true)
            .open(file_path)
            .map_err(|e| e.to_string())?;

        let mut writer = csv::WriterBuilder::new().from_writer(file_append);

        // Construct row based on header order
        let mut row = Vec::new();
        for col_name in headers.iter() {
            let val = if let Some((v, _, _)) = serializable_map.get(col_name) {
                v.clone()
            } else {
                String::new() // Check if required?
            };
            row.push(val);
        }

        writer.write_record(&row).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        
        Ok(())
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        let file_path = Path::new(location);
        let file = fs::File::open(file_path).map_err(|e| e.to_string())?;
        let mut reader = csv::ReaderBuilder::new().has_headers(true).from_reader(file);
        let headers = reader.headers().map_err(|e| e.to_string())?.clone();
        
        // We assume the first column is always the ID? Or look for "id" column?
        // Let's assume "id" column exists.
        let id_idx = headers.iter().position(|h| h.eq_ignore_ascii_case("id"))
            .ok_or("Dataset does not have an 'id' column")?;

        for result in reader.records() {
            let record = result.map_err(|e| e.to_string())?;
            if let Some(record_id) = record.get(id_idx) {
                if record_id == id {
                    // Found it
                    let mut map = HashMap::new();
                    for (i, field) in record.iter().enumerate() {
                        let col_name = headers.get(i).unwrap_or("unknown").to_string();
                        // Type detection is weak here, defaulting to String with empty metadata
                        // UPDATED: Use infer_value_type to guess the type (Integer, Float, Boolean, DateTime)
                        let val_type = ox_type_converter::TypeConverter::infer_value_type(&field.to_string());
                        map.insert(col_name, (field.to_string(), val_type, HashMap::new()));
                    }
                    return Ok(map);
                }
            }
        }

        Err(format!("Object with id {} not found in {}", id, location))
    }

    fn fetch(
        &self, 
        filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str
    ) -> Result<Vec<String>, String> {
        let file_path = Path::new(location);
        let file = fs::File::open(file_path).map_err(|e| e.to_string())?;
        let mut reader = csv::ReaderBuilder::new().has_headers(true).from_reader(file);
        let headers = reader.headers().map_err(|e| e.to_string())?.clone();
        
        let id_idx = headers.iter().position(|h| h.eq_ignore_ascii_case("id"))
            .ok_or("Dataset does not have an 'id' column")?;

        let mut matching_ids = Vec::new();

        for result in reader.records() {
            let record = result.map_err(|e| e.to_string())?;
            let mut matches = true;

            // Check all filter conditions
            for (key, (val, _, _)) in filter {
                if let Some(col_idx) = headers.iter().position(|h| h == key) {
                    if let Some(record_val) = record.get(col_idx) {
                        if record_val != val {
                            matches = false;
                            break;
                        }
                    } else {
                         matches = false; 
                         break;
                    }
                } else {
                    // Column not found, assume no match or ignore? Strict: no match.
                    matches = false;
                    break;
                }
            }

            if matches {
                if let Some(id) = record.get(id_idx) {
                    matching_ids.push(id.to_string());
                }
            }
        }
        Ok(matching_ids)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        println!("FlatfileDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
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

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "path".to_string(),
                description: "The path to the directory containing the data files.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "definition_file".to_string(),
                description: "The path to the YAML file that defines the data structure.".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
        ]
    }
}

// --- FFI Exports ---

use ox_persistence::OxBuffer;
use std::ffi::CString;

#[no_mangle]
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compatible_modules = HashMap::new();
    compatible_modules.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "Flatfile Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    let metadata = DriverMetadata {
        name: "ox_persistence_driver_flatfile".to_string(),
        description: "A flatfile (CSV) persistence driver.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub extern "C" fn ox_driver_init(_config_json: *const c_char) -> *mut c_void {
    // Config parsing can happen here if needed. For now, just create the driver.
    let driver = Box::new(FlatfileDriver);
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut FlatfileDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void, 
    data_json: *const c_char, 
    location: *const c_char
) -> i32 {
    let driver = &*(ctx as *mut FlatfileDriver);
    let data_str = CStr::from_ptr(data_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();

    // Deserialize JSON to HashMap needed by persist
    // Note: The internal persist trait expects a complex HashMap structure.
    // For simplicity in this FFI adaptation, we'll try to deserialize to that structure.
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&data_str) {
        Ok(map) => {
            match driver.persist(&map, &location_str) {
                Ok(_) => 0, // Success
                Err(e) => {
                    eprintln!("Persist error: {}", e);
                    -1
                }
            }
        },
        Err(e) => {
            eprintln!("JSON parse error: {}", e);
            -2
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_restore(
    ctx: *mut c_void, 
    location: *const c_char, 
    id: *const c_char
) -> OxBuffer {
    let driver = &*(ctx as *mut FlatfileDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();

    match driver.restore(&location_str, &id_str) {
        Ok(map) => {
            let json = serde_json::to_string(&map).unwrap_or_default();
            OxBuffer::from_str(json)
        },
        Err(e) => {
            // Return error as a special JSON or empty buffer?
            // For now, empty buffer implies failure or not found.
            // Ideally should have status code return.
            eprintln!("Restore error: {}", e);
            OxBuffer::empty()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_fetch(
    ctx: *mut c_void, 
    filter_json: *const c_char, 
    location: *const c_char
) -> OxBuffer {
    let driver = &*(ctx as *mut FlatfileDriver);
    let filter_str = CStr::from_ptr(filter_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();

    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&filter_str) {
        Ok(filter) => {
            match driver.fetch(&filter, &location_str) {
                Ok(ids) => {
                    let json = serde_json::to_string(&ids).unwrap_or_default();
                    OxBuffer::from_str(json)
                },
                Err(e) => {
                    eprintln!("Fetch error: {}", e);
                    OxBuffer::empty()
                }
            }
        },
        Err(e) => {
            eprintln!("JSON parse error: {}", e);
            OxBuffer::empty()
        }
    }
}

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = include_str!("../ox_persistence_driver_file_delimited_config_schema.yaml");
    CString::new(schema).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}