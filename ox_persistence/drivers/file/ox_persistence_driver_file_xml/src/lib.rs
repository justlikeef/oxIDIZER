use ox_persistence::{PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use std::collections::HashMap;
use std::ffi::{CString, CStr};
use libc::{c_char, c_void};
use std::sync::Arc;
use ox_type_converter::ValueType;
use ox_fileproc::serde_json;


pub struct XmlPersistenceDriver;

impl PersistenceDriver for XmlPersistenceDriver {

    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>, 
        location: &str,
    ) -> Result<(), String> {
        use std::fs;
        use std::path::Path;

        let file_path = Path::new(location);
        let mut wrapper : RecordsWrapper = if file_path.exists() {
            let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
            if content.trim().is_empty() {
                RecordsWrapper { record: Vec::new() }
            } else {
                 quick_xml::de::from_str(&content).map_err(|e| format!("XML parse error: {}", e))?
            }
        } else {
            RecordsWrapper { record: Vec::new() }
        };

        let mut fields = HashMap::new();
        for (k, (v, vt, meta)) in serializable_map {
            fields.insert(k.clone(), FieldData {
                value: v.clone(),
                value_type: vt.as_str().to_string(),
                metadata: meta.clone(),
            });
        }

        wrapper.record.push(Record { fields });

        let new_content = quick_xml::se::to_string(&wrapper).map_err(|e| e.to_string())?;
        fs::write(file_path, new_content).map_err(|e| e.to_string())?;

        Ok(())
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, String> {
        use std::fs;
        use std::path::Path;

        let file_path = Path::new(location);
        if !file_path.exists() {
            return Err(format!("File {} not found", location));
        }
        let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
        let wrapper : RecordsWrapper = quick_xml::de::from_str(&content).map_err(|e| format!("XML parse error: {}", e))?;

        for record in wrapper.record {
             if let Some(id_field) = record.fields.get("id") {
                 if id_field.value == id {
                     let mut map = HashMap::new();
                     for (k, fd) in record.fields {
                         map.insert(k, (fd.value, ValueType::from(fd.value_type), fd.metadata));
                     }
                     return Ok(map);
                 }
             }
        }

        Err(format!("Object with id {} not found in {}", id, location))
    }

    fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
        use std::fs;
        use std::path::Path;

        let file_path = Path::new(location);
        if !file_path.exists() {
             return Ok(Vec::new());
        }
        let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
        let wrapper : RecordsWrapper = quick_xml::de::from_str(&content).map_err(|e| format!("XML parse error: {}", e))?;

        let mut matching_ids = Vec::new();

        for record in wrapper.record {
            let mut matches = true;
             for (key, (val, _, _)) in filter {
                 if let Some(fd) = record.fields.get(key) {
                     if &fd.value != val {
                         matches = false;
                         break;
                     }
                 } else {
                     matches = false;
                     break;
                 }
            }

            if matches {
                if let Some(id_field) = record.fields.get("id") {
                    matching_ids.push(id_field.value.clone());
                }
            }
        }
        Ok(matching_ids)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
         println!("XmlDriver: GDO {} lock status changed to {}", gdo_id, lock_status);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), String> {
        println!("Preparing XML datastore: {:?}", connection_info);
        Ok(())
    }

    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, String> {
        Ok(vec!["default".to_string()])
    }
    
    fn describe_dataset(&self, _connection_info: &HashMap<String, String>, dataset_name: &str) -> Result<DataSet, String> {
        Ok(DataSet { name: dataset_name.to_string(), columns: Vec::new() })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
         vec![ConnectionParameter {
            name: "path".to_string(),
            description: "Path to xml file".to_string(),
            data_type: "string".to_string(),
            is_required: true,
            default_value: None,
        }]
    }
}


// --- FFI Exports ---

#[no_mangle]
pub extern "C" fn ox_driver_init(_config_json: *const c_char) -> *mut c_void {
    let driver = Box::new(XmlPersistenceDriver);
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut XmlPersistenceDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void, 
    data_json: *const c_char, 
    location: *const c_char
) -> i32 {
    let driver = &*(ctx as *mut XmlPersistenceDriver);
    let data_str = CStr::from_ptr(data_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();

    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&data_str) {
        Ok(map) => {
            match driver.persist(&map, &location_str) {
                Ok(_) => 0,
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
    let driver = &*(ctx as *mut XmlPersistenceDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();

    match driver.restore(&location_str, &id_str) {
        Ok(map) => {
            let json = serde_json::to_string(&map).unwrap_or_default();
            OxBuffer::from_str(json)
        },
        Err(e) => {
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
    let driver = &*(ctx as *mut XmlPersistenceDriver);
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
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compatible_modules = HashMap::new();
    compatible_modules.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "XML Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );

    // Parse friendly_name from schema using ox_fileproc
    let schema = include_str!("../ox_persistence_driver_file_xml_config_schema.yaml");
    let friendly_name = match ox_fileproc::processor::parse_content(schema, "yaml") {
        Ok(ox_fileproc::serde_json::Value::Object(map)) => {
            map.get("friendly_name")
               .and_then(|v| v.as_str())
               .map(|s| s.to_string())
               .unwrap_or("XML File".to_string())
        },
        Ok(_) => {
            eprintln!("Schema parsed but not an object!");
            "XML File".to_string()
        },
        Err(e) => {
            eprintln!("Schema parse error: {}", e);
            "XML File".to_string()
        }
    };

    let metadata = DriverMetadata {
        name: "ox_persistence_driver_xml".to_string(),
        friendly_name: Some(friendly_name),
        description: "An XML persistence driver.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules,
    };

    let json_string = ox_fileproc::serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    CString::new(json_string).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = include_str!("../ox_persistence_driver_file_xml_config_schema.yaml");
    CString::new(schema).expect("Failed to create CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}

// Wrapper struct for serialization
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename = "records")]
struct RecordsWrapper {
    #[serde(rename = "record", default)]
    record: Vec<Record>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    fields: HashMap<String, FieldData>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FieldData {
    value: String,
    value_type: String, // String representation of ValueType
    metadata: HashMap<String, String>,
}