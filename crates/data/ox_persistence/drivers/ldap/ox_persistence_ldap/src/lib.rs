//! ox_persistence_ldap — LDAP persistence driver for the canonical IAM schema.
//! Implements the PersistenceDriver trait from ox_persistence.

pub mod conn_factory;
pub mod entity;
pub mod error;
pub mod mapping;

use std::collections::HashMap;
use std::sync::Arc;
use ox_data_error::OxDataError;
use ox_persistence::{PersistenceDriver, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, OxBuffer};
#[cfg(feature = "ffi")]
use ox_persistence::{DriverMetadata, ModuleCompatibility};
use ox_type_converter::ValueType;

use conn_factory::{LdapConnFactory, LdapConfig, RealLdapConnFactory};
use entity::{ldap_attrs_from_canonical_map, canonical_map_from_ldap_attrs,
             primary_key_value, primary_key_filter, build_fetch_filter};
use mapping::SchemaMapping;

// Re-export MockLdapConn so integration tests can import it from the crate root in tests.
pub use conn_factory::{MockLdapConn, MockLdapConnFactory};

/// The LDAP persistence driver.  Holds a `LdapConnFactory` — in production this is a
/// `RealLdapConnFactory`; in tests it is a `MockLdapConnFactory`.
pub struct LdapPersistenceDriver {
    factory: Arc<dyn LdapConnFactory>,
    mapping: SchemaMapping,
    base_dn: String,
}

impl LdapPersistenceDriver {
    pub fn new(config: LdapConfig, mapping: SchemaMapping) -> Self {
        let base_dn = config.base_dn.clone();
        let factory = Arc::new(RealLdapConnFactory::new(config));
        Self { factory, mapping, base_dn }
    }

    /// Constructor that accepts a pre-built factory (used by tests and ox_persistence_ad).
    pub fn new_with_factory(
        factory: Arc<dyn LdapConnFactory>,
        mapping: SchemaMapping,
        connection_info: HashMap<String, String>,
    ) -> Self {
        let base_dn = connection_info.get("base_dn").cloned().unwrap_or_default();
        Self { factory, mapping, base_dn }
    }

    /// Build the DN for a new entry under the appropriate sub-tree.
    /// e.g. "uid=alice,ou=principals,dc=example,dc=com"
    fn entry_dn(&self, location: &str, pk_value: &str) -> String {
        let pk_attr = self.mapping.canonical_to_ldap(location, &self.mapping.primary_key_field(location));
        format!("{}={},ou={},{}", pk_attr, pk_value, location, self.base_dn)
    }
}

impl PersistenceDriver for LdapPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<(), OxDataError> {
        let pk_val = primary_key_value(serializable_map, &self.mapping, location);
        if pk_val.is_empty() {
            return Err(OxDataError::DriverError(
                format!("LDAP persist: missing primary key for location '{}'", location),
            ));
        }
        let dn = self.entry_dn(location, &pk_val);
        let attrs = ldap_attrs_from_canonical_map(serializable_map, &self.mapping, location);
        let conn = self.factory.create();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| OxDataError::InternalError(e.to_string()))?;

        // Try add; if entry exists, fall back to modify.
        rt.block_on(async {
            let add_result = conn.add(&dn, attrs.clone()).await;
            if add_result.is_err() {
                // Entry may already exist — replace attribute values.
                let mods: Vec<(String, Vec<String>)> = attrs
                    .into_iter()
                    .filter(|(k, _)| k != "objectClass")
                    .collect();
                conn.modify(&dn, mods).await
            } else {
                add_result
            }
        })
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, OxDataError> {
        let filter = primary_key_filter(id, &self.mapping, location);
        let search_base = format!("ou={},{}", location, self.base_dn);
        let conn = self.factory.create();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| OxDataError::InternalError(e.to_string()))?;

        let entries = rt.block_on(conn.search(&search_base, &filter))?;
        let entry = entries.into_iter().next()
            .ok_or_else(|| OxDataError::InternalError(format!("LDAP entry not found: id={} location={}", id, location)))?;

        Ok(canonical_map_from_ldap_attrs(&entry, &self.mapping, location))
    }

    fn fetch(
        &self,
        filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<Vec<String>, OxDataError> {
        let ldap_filter = build_fetch_filter(filter, &self.mapping, location);
        let search_base = format!("ou={},{}", location, self.base_dn);
        let conn = self.factory.create();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| OxDataError::InternalError(e.to_string()))?;

        let entries = rt.block_on(conn.search(&search_base, &ldap_filter))?;
        let pk_field = self.mapping.primary_key_field(location);

        let ids: Vec<String> = entries
            .into_iter()
            .map(|attrs| canonical_map_from_ldap_attrs(&attrs, &self.mapping, location))
            .filter_map(|canon| canon.get(&pk_field).map(|(v, _, _)| v.clone()))
            .collect();

        Ok(ids)
    }

    fn notify_lock_status_change(&self, _lock_status: &str, _gdo_id: &str) {
        // No-op: LDAP has no native lock notification concept.
    }

    fn prepare_datastore(&self, connection_info: &HashMap<String, String>) -> Result<(), OxDataError> {
        // Verify connection by doing a simple root DSE search.
        let config = LdapConfig::from_map(connection_info)
            .map_err(|e| OxDataError::DriverError(format!("{:?}", e)))?;
        let driver = LdapPersistenceDriver::new(config, self.mapping.clone());
        let conn = driver.factory.create();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| OxDataError::InternalError(e.to_string()))?;
        let _ = rt.block_on(conn.search("", "(objectClass=*)"))?;
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError> {
        Ok(vec![
            "principals".to_string(),
            "groups".to_string(),
            "members".to_string(),
            "grants".to_string(),
            "sessions".to_string(),
        ])
    }

    fn describe_dataset(
        &self,
        _connection_info: &HashMap<String, String>,
        dataset_name: &str,
    ) -> Result<DataSet, OxDataError> {
        let pk = self.mapping.primary_key_field(dataset_name);
        let columns = vec![ColumnDefinition {
            name: pk,
            data_type: "string".to_string(),
            metadata: ColumnMetadata::default(),
        }];
        Ok(DataSet { name: dataset_name.to_string(), columns })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "url".to_string(),
                description: "LDAP server URL (e.g. ldap://ldap.example.com:389 or ldaps://...)".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "bind_dn".to_string(),
                description: "DN of the service account used for bind (e.g. cn=svc,dc=example,dc=com)".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "bind_password".to_string(),
                description: "Password for the bind DN service account".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "base_dn".to_string(),
                description: "LDAP search base DN (e.g. dc=example,dc=com)".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
        ]
    }
}

// ---------------------------------------------------------------------------
// FFI exports — mirrors ox_persistence_api pattern exactly
// Only compiled when the "ffi" feature is enabled (default for standalone builds;
// disabled when linked as a library dependency by ox_persistence_ad to avoid
// duplicate symbol errors).
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
    let config = match LdapConfig::from_map(&info) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ox_persistence_ldap init error: {:?}", e);
            return std::ptr::null_mut();
        }
    };
    let driver = Box::new(LdapPersistenceDriver::new(config, SchemaMapping::ldap_defaults()));
    Box::into_raw(driver) as *mut c_void
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut LdapPersistenceDriver);
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void,
    data_json: *const c_char,
    location: *const c_char,
) -> i32 {
    let driver = &*(ctx as *mut LdapPersistenceDriver);
    let data_str = CStr::from_ptr(data_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&data_str) {
        Ok(map) => match driver.persist(&map, &location_str) {
            Ok(_) => 0,
            Err(e) => { eprintln!("LDAP persist error: {}", e); -1 }
        },
        Err(e) => { eprintln!("LDAP persist JSON error: {}", e); -2 }
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_restore(
    ctx: *mut c_void,
    location: *const c_char,
    id: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut LdapPersistenceDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();
    match driver.restore(&location_str, &id_str) {
        Ok(map) => OxBuffer::from_str(serde_json::to_string(&map).unwrap_or_default()),
        Err(e) => { eprintln!("LDAP restore error: {}", e); OxBuffer::empty() }
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_fetch(
    ctx: *mut c_void,
    filter_json: *const c_char,
    location: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut LdapPersistenceDriver);
    let filter_str = CStr::from_ptr(filter_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&filter_str) {
        Ok(filter) => match driver.fetch(&filter, &location_str) {
            Ok(ids) => OxBuffer::from_str(serde_json::to_string(&ids).unwrap_or_default()),
            Err(e) => { eprintln!("LDAP fetch error: {}", e); OxBuffer::empty() }
        },
        Err(e) => { eprintln!("LDAP fetch JSON error: {}", e); OxBuffer::empty() }
    }
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compat = HashMap::new();
    compat.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "LDAP Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );
    let metadata = DriverMetadata {
        name: "ox_persistence_ldap".to_string(),
        friendly_name: Some("LDAP Directory".to_string()),
        description: "Persists canonical IAM entities (principals, groups, grants, sessions) to an LDAP directory.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules: compat,
    };
    let json = serde_json::to_string(&metadata).expect("serialize metadata");
    CString::new(json).expect("CString").into_raw()
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = r#"
parameters:
  - name: url
    type: string
    required: true
    description: "LDAP server URL"
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
    description: "Search base DN"
"#;
    CString::new(schema).expect("CString").into_raw()
}

#[cfg(feature = "ffi")]
#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}
