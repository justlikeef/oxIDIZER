//! ox_persistence_ad — Active Directory persistence driver.
//!
//! A thin wrapper over `ox_persistence_ldap::LdapPersistenceDriver` that substitutes
//! AD-specific attribute names and object classes.  All LDAP wire work is delegated
//! to the LDAP driver.

use std::collections::HashMap;
use std::sync::Arc;
use ox_data_error::OxDataError;
use ox_persistence::{
    PersistenceDriver, DataSet, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer,
};
use ox_persistence_ldap::{LdapPersistenceDriver, mapping::SchemaMapping};
use ox_type_converter::ValueType;

/// Returns the AD-specific schema mapping by starting with LDAP defaults and
/// applying AD attribute overrides.
pub fn ad_schema_mapping() -> SchemaMapping {
    SchemaMapping::ldap_defaults()
        // AD uses sAMAccountName as the login identifier instead of uid
        .with_override("principals", "principal_id", "sAMAccountName")
        // AD user objectClass is "user" instead of "inetOrgPerson"
        .with_object_classes("principals", vec!["user".to_string(), "oxIAMPrincipal".to_string()])
        // AD group objectClass is "group" instead of "groupOfNames"
        .with_object_classes("groups", vec!["group".to_string(), "oxIAMGroup".to_string()])
}

/// The Active Directory persistence driver.
pub struct AdPersistenceDriver {
    inner: LdapPersistenceDriver,
}

impl AdPersistenceDriver {
    pub fn new(connection_info: HashMap<String, String>) -> Result<Self, OxDataError> {
        use ox_persistence_ldap::conn_factory::LdapConfig;
        let config = LdapConfig::from_map(&connection_info)
            .map_err(|e| OxDataError::DriverError(format!("{:?}", e)))?;
        Ok(Self {
            inner: LdapPersistenceDriver::new(config, ad_schema_mapping()),
        })
    }

    /// Construct with an injected factory (used by tests).
    pub fn new_with_factory(
        factory: Arc<dyn ox_persistence_ldap::conn_factory::LdapConnFactory>,
        connection_info: HashMap<String, String>,
    ) -> Self {
        Self {
            inner: LdapPersistenceDriver::new_with_factory(factory, ad_schema_mapping(), connection_info),
        }
    }
}

impl PersistenceDriver for AdPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<(), OxDataError> {
        self.inner.persist(serializable_map, location)
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, OxDataError> {
        self.inner.restore(location, id)
    }

    fn fetch(
        &self,
        filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<Vec<String>, OxDataError> {
        self.inner.fetch(filter, location)
    }

    fn notify_lock_status_change(&self, lock_status: &str, gdo_id: &str) {
        self.inner.notify_lock_status_change(lock_status, gdo_id);
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), OxDataError> {
        self.inner.prepare_datastore(connection_info)
    }

    fn list_datasets(&self, connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError> {
        self.inner.list_datasets(connection_info)
    }

    fn describe_dataset(
        &self,
        connection_info: &HashMap<String, String>,
        dataset_name: &str,
    ) -> Result<DataSet, OxDataError> {
        self.inner.describe_dataset(connection_info, dataset_name)
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "url".to_string(),
                description: "Active Directory LDAP URL (e.g. ldap://ad.corp.com:389)".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "bind_dn".to_string(),
                description: "DN of the service account (e.g. CN=svc,CN=Users,DC=corp,DC=com)".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "bind_password".to_string(),
                description: "Password for the service account".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "base_dn".to_string(),
                description: "AD search base DN (e.g. DC=corp,DC=com)".to_string(),
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

use std::ffi::{c_void, CString, CStr};
use libc::c_char;

#[no_mangle]
pub extern "C" fn ox_driver_init(config_json: *const c_char) -> *mut c_void {
    let config_str = unsafe { CStr::from_ptr(config_json).to_string_lossy() };
    let info: HashMap<String, String> = serde_json::from_str(&config_str).unwrap_or_default();
    match AdPersistenceDriver::new(info) {
        Ok(driver) => Box::into_raw(Box::new(driver)) as *mut c_void,
        Err(e) => {
            eprintln!("ox_persistence_ad init error: {}", e);
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut AdPersistenceDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void,
    data_json: *const c_char,
    location: *const c_char,
) -> i32 {
    let driver = &*(ctx as *mut AdPersistenceDriver);
    let data_str = CStr::from_ptr(data_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&data_str) {
        Ok(map) => match driver.persist(&map, &location_str) {
            Ok(_) => 0,
            Err(e) => { eprintln!("AD persist error: {}", e); -1 }
        },
        Err(e) => { eprintln!("AD persist JSON error: {}", e); -2 }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_restore(
    ctx: *mut c_void,
    location: *const c_char,
    id: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut AdPersistenceDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();
    match driver.restore(&location_str, &id_str) {
        Ok(map) => OxBuffer::from_str(serde_json::to_string(&map).unwrap_or_default()),
        Err(e) => { eprintln!("AD restore error: {}", e); OxBuffer::empty() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_fetch(
    ctx: *mut c_void,
    filter_json: *const c_char,
    location: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut AdPersistenceDriver);
    let filter_str = CStr::from_ptr(filter_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&filter_str) {
        Ok(filter) => match driver.fetch(&filter, &location_str) {
            Ok(ids) => OxBuffer::from_str(serde_json::to_string(&ids).unwrap_or_default()),
            Err(e) => { eprintln!("AD fetch error: {}", e); OxBuffer::empty() }
        },
        Err(e) => { eprintln!("AD fetch JSON error: {}", e); OxBuffer::empty() }
    }
}

#[no_mangle]
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compat = HashMap::new();
    compat.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "Active Directory Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );
    let metadata = DriverMetadata {
        name: "ox_persistence_ad".to_string(),
        friendly_name: Some("Active Directory".to_string()),
        description: "Persists canonical IAM entities to Active Directory using LDAP with AD-specific attribute mappings.".to_string(),
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

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = r#"
parameters:
  - name: url
    type: string
    required: true
    description: "Active Directory LDAP URL"
  - name: bind_dn
    type: string
    required: true
    description: "Service account DN"
  - name: bind_password
    type: string
    required: true
    description: "Service account password"
  - name: base_dn
    type: string
    required: true
    description: "AD search base DN"
"#;
    match CString::new(schema) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}
