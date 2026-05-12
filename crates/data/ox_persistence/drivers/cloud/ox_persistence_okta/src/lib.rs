//! ox_persistence_okta — Okta REST API persistence driver for canonical IAM entities.
//!
//! Natively supports: principals (Okta users), groups (Okta groups), members (group membership).
//! Does NOT support: grants, sessions — callers must route those to an overflow store.

pub mod error;
pub mod http_client;
pub mod mapping;

use std::collections::HashMap;
use std::sync::Arc;
use ox_data_error::OxDataError;
use ox_persistence::{
    PersistenceDriver, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter,
};
#[cfg(feature = "ffi")]
use ox_persistence::{DriverMetadata, ModuleCompatibility, OxBuffer};
use ox_type_converter::ValueType;

use http_client::{OktaHttpClient, RealOktaHttpClient};
use mapping::{
    principal_to_okta_body, okta_user_to_canonical,
    group_to_okta_body, okta_group_to_canonical,
    supported_locations, canon_get,
};

// Re-export MockOktaHttpClient for downstream test consumers
#[cfg(any(test, feature = "test-support"))]
pub use http_client::MockOktaHttpClient;

/// The Okta persistence driver.
pub struct OktaPersistenceDriver {
    client: Arc<dyn OktaHttpClient>,
}

impl OktaPersistenceDriver {
    pub fn new(domain: &str, api_token: &str) -> Self {
        Self {
            client: Arc::new(RealOktaHttpClient::new(domain, api_token)),
        }
    }

    /// Constructor for tests — accepts a pre-built mock client.
    pub fn new_with_client(client: Arc<dyn OktaHttpClient>) -> Self {
        Self { client }
    }
}

impl PersistenceDriver for OktaPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<(), OxDataError> {
        match location {
            "principals" => {
                let body = principal_to_okta_body(serializable_map);
                // Use ?activate=true to immediately activate the user.
                self.client.post("/api/v1/users?activate=true", &body)?;
                Ok(())
            }
            "groups" => {
                let body = group_to_okta_body(serializable_map);
                self.client.post("/api/v1/groups", &body)?;
                Ok(())
            }
            "members" => {
                // Requires both _okta_group_id and _okta_user_id to be provided as annotations.
                // These must be resolved by the caller (or via a fetch before persist).
                let group_id = canon_get(serializable_map, "_okta_group_id");
                let user_id  = canon_get(serializable_map, "_okta_user_id");
                if group_id.is_empty() || user_id.is_empty() {
                    return Err(OxDataError::DriverError(
                        "Okta members persist requires '_okta_group_id' and '_okta_user_id' in the map".to_string(),
                    ));
                }
                let path = mapping::group_membership_put_path(&group_id, &user_id);
                self.client.put(&path, &serde_json::json!({}))?;
                Ok(())
            }
            other => Err(OxDataError::DriverError(format!(
                "Okta driver does not support persist for location '{}'. Route to overflow store.",
                other
            ))),
        }
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, OxDataError> {
        match location {
            "principals" => {
                let path = format!("/api/v1/users/{}", id);
                let user = self.client.get(&path)?;
                Ok(okta_user_to_canonical(&user))
            }
            "groups" => {
                // Okta group lookup by name requires a search.
                let encoded_id = id.replace(' ', "%20").replace('&', "%26").replace('=', "%3D").replace('+', "%2B");
                let path = format!("/api/v1/groups?q={}", encoded_id);
                let groups = self.client.get(&path)?;
                let arr = groups.as_array()
                    .and_then(|a| a.first())
                    .ok_or_else(|| OxDataError::InternalError(format!("Okta group not found: {}", id)))?;
                Ok(okta_group_to_canonical(arr))
            }
            other => Err(OxDataError::DriverError(format!(
                "Okta driver does not support restore for location '{}'", other
            ))),
        }
    }

    fn fetch(
        &self,
        _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<Vec<String>, OxDataError> {
        match location {
            "principals" => {
                // List all users (no filter applied — caller should post-filter or extend
                // with Okta search query parameters via call_action).
                let users = self.client.get("/api/v1/users")?;
                let arr = users.as_array().ok_or_else(|| {
                    OxDataError::InternalError("Okta users response was not an array".to_string())
                })?;
                Ok(arr.iter()
                    .map(|u| okta_user_to_canonical(u))
                    .filter_map(|m| m.get("principal_id").map(|(v, _, _)| v.clone()))
                    .collect())
            }
            "groups" => {
                let groups = self.client.get("/api/v1/groups")?;
                let arr = groups.as_array().ok_or_else(|| {
                    OxDataError::InternalError("Okta groups response was not an array".to_string())
                })?;
                Ok(arr.iter()
                    .map(|g| okta_group_to_canonical(g))
                    .filter_map(|m| m.get("group_id").map(|(v, _, _)| v.clone()))
                    .collect())
            }
            other => Err(OxDataError::DriverError(format!(
                "Okta driver does not support fetch for location '{}'", other
            ))),
        }
    }

    fn notify_lock_status_change(&self, _lock_status: &str, _gdo_id: &str) {
        // No-op: Okta has no native lock notification.
    }

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), OxDataError> {
        // Verify connectivity by fetching the current user.
        self.client.get("/api/v1/users/me")?;
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError> {
        Ok(supported_locations())
    }

    fn describe_dataset(
        &self,
        _connection_info: &HashMap<String, String>,
        dataset_name: &str,
    ) -> Result<DataSet, OxDataError> {
        let pk = match dataset_name {
            "principals" => "principal_id",
            "groups"     => "group_id",
            "members"    => "principal_id",
            other => return Err(OxDataError::DriverError(format!("Unknown Okta location: {}", other))),
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
                name: "domain".to_string(),
                description: "Okta organization domain (e.g. yourorg.okta.com)".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "api_token".to_string(),
                description: "Okta API token (SSWS token from Okta admin console)".to_string(),
                data_type: "string".to_string(),
                is_required: true,
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
pub extern "C" fn ox_driver_init(config_json: *const c_char) -> *mut c_void {
    let config_str = unsafe { CStr::from_ptr(config_json).to_string_lossy() };
    let info: HashMap<String, String> = serde_json::from_str(&config_str).unwrap_or_default();
    let domain    = info.get("domain").cloned().unwrap_or_default();
    let api_token = info.get("api_token").cloned().unwrap_or_default();
    if domain.is_empty() {
        eprintln!("ox_persistence_okta: missing 'domain' in config");
        return std::ptr::null_mut();
    }
    let driver = Box::new(OktaPersistenceDriver::new(&domain, &api_token));
    Box::into_raw(driver) as *mut c_void
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut OktaPersistenceDriver);
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void,
    data_json: *const c_char,
    location: *const c_char,
) -> i32 {
    let driver = &*(ctx as *mut OktaPersistenceDriver);
    let data_str = CStr::from_ptr(data_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&data_str) {
        Ok(map) => match driver.persist(&map, &location_str) {
            Ok(_) => 0,
            Err(e) => { eprintln!("Okta persist error: {}", e); -1 }
        },
        Err(e) => { eprintln!("Okta persist JSON error: {}", e); -2 }
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_restore(
    ctx: *mut c_void,
    location: *const c_char,
    id: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut OktaPersistenceDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();
    match driver.restore(&location_str, &id_str) {
        Ok(map) => OxBuffer::from_str(serde_json::to_string(&map).unwrap_or_default()),
        Err(e) => { eprintln!("Okta restore error: {}", e); OxBuffer::empty() }
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_fetch(
    ctx: *mut c_void,
    filter_json: *const c_char,
    location: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut OktaPersistenceDriver);
    let filter_str = CStr::from_ptr(filter_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&filter_str) {
        Ok(filter) => match driver.fetch(&filter, &location_str) {
            Ok(ids) => OxBuffer::from_str(serde_json::to_string(&ids).unwrap_or_default()),
            Err(e) => { eprintln!("Okta fetch error: {}", e); OxBuffer::empty() }
        },
        Err(e) => { eprintln!("Okta fetch JSON error: {}", e); OxBuffer::empty() }
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compat = HashMap::new();
    compat.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "Okta Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );
    let metadata = DriverMetadata {
        name: "ox_persistence_okta".to_string(),
        friendly_name: Some("Okta".to_string()),
        description: "Persists canonical IAM principals, groups, and group membership to Okta via REST API. Grants and sessions require an overflow store.".to_string(),
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
  - name: domain
    type: string
    required: true
    description: "Okta organization domain"
  - name: api_token
    type: string
    required: true
    description: "Okta SSWS API token"
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
