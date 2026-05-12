//! ox_persistence_radius — read-only RADIUS persistence driver.
//!
//! RADIUS servers do not expose a directory API.  This driver can only:
//! 1. Return principal and group-membership data extracted from a cached
//!    Access-Accept attribute set (populated by the auth driver after a
//!    successful authentication round-trip).
//! 2. Persist is not supported — returns OxDataError::DriverError for all locations.
//!
//! Supported locations (read-only): "principals", "members"
//! Unsupported locations: "groups", "grants", "sessions"

pub mod response_parser;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ox_data_error::OxDataError;
use ox_persistence::{
    PersistenceDriver, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter,
};
#[cfg(feature = "ffi")]
use ox_persistence::{DriverMetadata, ModuleCompatibility, OxBuffer};
use ox_type_converter::ValueType;

pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// In-memory cache of principal data populated from RADIUS Access-Accept responses.
/// Key: principal_id; Value: canonical map for that principal.
type PrincipalCache = HashMap<String, CanonicalMap>;

pub struct RadiusPersistenceDriver {
    cache: Arc<Mutex<PrincipalCache>>,
}

impl RadiusPersistenceDriver {
    /// Constructs a driver backed by an empty cache.
    /// The cache is populated by the auth driver via `insert_cached_principal`.
    pub fn new() -> Self {
        Self { cache: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Constructs a driver with a pre-populated cache (used by tests).
    pub fn new_with_cache(cache: PrincipalCache) -> Self {
        Self { cache: Arc::new(Mutex::new(cache)) }
    }

    /// Called by the RADIUS auth driver after a successful Access-Accept to populate the cache.
    pub fn insert_cached_principal(&self, principal_id: &str, data: CanonicalMap) {
        self.cache.lock().unwrap_or_else(|p| p.into_inner()).insert(principal_id.to_string(), data);
    }
}

impl Default for RadiusPersistenceDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistenceDriver for RadiusPersistenceDriver {
    fn persist(
        &self,
        _serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<(), OxDataError> {
        Err(OxDataError::DriverError(format!(
            "RADIUS driver is read-only: persist not supported for location '{}'",
            location
        )))
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, OxDataError> {
        match location {
            "principals" => {
                let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
                cache.get(id)
                    .cloned()
                    .ok_or_else(|| OxDataError::InternalError(format!("RADIUS: principal '{}' not in cache", id)))
            }
            other => Err(OxDataError::DriverError(format!(
                "RADIUS driver does not support restore for location '{}'", other
            ))),
        }
    }

    fn fetch(
        &self,
        filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<Vec<String>, OxDataError> {
        match location {
            "principals" => {
                let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
                // Return IDs of all cached principals that match every filter field.
                let ids: Vec<String> = cache
                    .iter()
                    .filter(|(_, entry)| {
                        filter.iter().all(|(fk, (fv, _, _))| {
                            entry.get(fk).map_or(false, |(ev, _, _)| ev == fv)
                        })
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                Ok(ids)
            }
            "members" => {
                // Return principal IDs whose group_id matches the filter.
                let gid = match filter.get("group_id").map(|(v, _, _)| v.clone()) {
                    Some(v) => v,
                    None => return Err(OxDataError::DriverError(
                        "RADIUS members fetch requires 'group_id' in filter".to_string(),
                    )),
                };
                let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
                let ids: Vec<String> = cache
                    .iter()
                    .filter(|(_, entry)| {
                        entry.get("group_id").map_or(false, |(v, _, _)| v == &gid)
                    })
                    .filter_map(|(_, entry)| entry.get("principal_id").map(|(v, _, _)| v.clone()))
                    .collect();
                Ok(ids)
            }
            other => Err(OxDataError::DriverError(format!(
                "RADIUS driver does not support fetch for location '{}'", other
            ))),
        }
    }

    fn notify_lock_status_change(&self, _lock_status: &str, _gdo_id: &str) {}

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), OxDataError> {
        // No preparation needed for the in-memory cache driver.
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError> {
        Ok(vec!["principals".to_string(), "members".to_string()])
    }

    fn describe_dataset(
        &self,
        _connection_info: &HashMap<String, String>,
        dataset_name: &str,
    ) -> Result<DataSet, OxDataError> {
        let pk = match dataset_name {
            "principals" => "principal_id",
            "members"    => "principal_id",
            other => return Err(OxDataError::DriverError(format!("Unknown RADIUS location: {}", other))),
        };
        Ok(DataSet {
            name: dataset_name.to_string(),
            columns: vec![ColumnDefinition {
                name: pk.to_string(),
                data_type: "string".to_string(),
                metadata: ColumnMetadata::default(),
            }],
        })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "server".to_string(),
                description: "RADIUS server address (not used for direct persistence — this driver operates on cached Access-Accept data)".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
        ]
    }
}

// ---------------------------------------------------------------------------
// FFI exports
// ---------------------------------------------------------------------------

#[cfg(feature = "ffi")]
use std::ffi::{c_void, CString, CStr};
#[cfg(feature = "ffi")]
use libc::c_char;

#[cfg(feature = "ffi")]
#[no_mangle]
pub extern "C" fn ox_driver_init(_config_json: *const c_char) -> *mut c_void {
    let driver = Box::new(RadiusPersistenceDriver::new());
    Box::into_raw(driver) as *mut c_void
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut RadiusPersistenceDriver);
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    _ctx: *mut c_void,
    _data_json: *const c_char,
    location: *const c_char,
) -> i32 {
    let location_str = CStr::from_ptr(location).to_string_lossy();
    eprintln!("RADIUS driver: persist not supported for location '{}'", location_str);
    -1
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_restore(
    ctx: *mut c_void,
    location: *const c_char,
    id: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut RadiusPersistenceDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();
    match driver.restore(&location_str, &id_str) {
        Ok(map) => OxBuffer::from_str(serde_json::to_string(&map).unwrap_or_default()),
        Err(e) => { eprintln!("RADIUS restore error: {}", e); OxBuffer::empty() }
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_fetch(
    ctx: *mut c_void,
    filter_json: *const c_char,
    location: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut RadiusPersistenceDriver);
    let filter_str = CStr::from_ptr(filter_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&filter_str) {
        Ok(filter) => match driver.fetch(&filter, &location_str) {
            Ok(ids) => OxBuffer::from_str(serde_json::to_string(&ids).unwrap_or_default()),
            Err(e) => { eprintln!("RADIUS fetch error: {}", e); OxBuffer::empty() }
        },
        Err(e) => { eprintln!("RADIUS fetch JSON error: {}", e); OxBuffer::empty() }
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compat = HashMap::new();
    compat.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "RADIUS Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );
    let metadata = DriverMetadata {
        name: "ox_persistence_radius".to_string(),
        friendly_name: Some("RADIUS".to_string()),
        description: "Read-only RADIUS persistence driver. Extracts principal and group membership from cached Access-Accept attributes. Persist not supported — route grants and sessions to overflow store.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules: compat,
    };
    let json = match serde_json::to_string(&metadata) {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    match CString::new(json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = r#"
parameters:
  - name: server
    type: string
    required: false
    description: "RADIUS server address (informational only)"
"#;
    match CString::new(schema) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}
