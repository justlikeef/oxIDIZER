//! Abstracts the LDAP connection so production code uses ldap3 and tests use MockLdapConn.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use ox_data_error::OxDataError;
use crate::entity::LdapAttrList;
use crate::error::LdapDriverError;

/// Minimal LDAP operations needed by this driver.
/// The `search` return type is a Vec of attribute lists — one per matching entry.
#[async_trait]
pub trait LdapConn: Send + Sync {
    /// Perform a one-level or subtree search from `base_dn` with `filter`.
    /// Returns a list of entries; each entry is a `LdapAttrList`.
    async fn search(&self, base_dn: &str, filter: &str) -> Result<Vec<LdapAttrList>, OxDataError>;

    /// Add a new entry at `dn` with the given attributes.
    async fn add(&self, dn: &str, attrs: LdapAttrList) -> Result<(), OxDataError>;

    /// Replace attribute values on an existing entry.
    async fn modify(&self, dn: &str, mods: Vec<(String, Vec<String>)>) -> Result<(), OxDataError>;

    /// Delete the entry at `dn`.
    async fn delete(&self, dn: &str) -> Result<(), OxDataError>;
}

/// Factory that creates (or reuses) an `LdapConn`.  Injected into the driver at construction
/// time so tests can supply a `MockLdapConnFactory` without touching the ldap3 crate.
pub trait LdapConnFactory: Send + Sync {
    fn create(&self) -> Arc<dyn LdapConn>;
}

// ---------------------------------------------------------------------------
// Real ldap3-backed connection (used in production)
// ---------------------------------------------------------------------------

/// Connection config extracted from the driver's `connection_info` HashMap.
#[derive(Clone)]
pub struct LdapConfig {
    pub url: String,
    pub bind_dn: String,
    pub bind_password: String,
    pub base_dn: String,
}

impl LdapConfig {
    pub fn from_map(info: &HashMap<String, String>) -> Result<Self, LdapDriverError> {
        Ok(Self {
            url:           info.get("url")          .cloned().ok_or_else(|| LdapDriverError::InvalidConfig("missing 'url'".to_string()))?,
            bind_dn:       info.get("bind_dn")      .cloned().ok_or_else(|| LdapDriverError::InvalidConfig("missing 'bind_dn'".to_string()))?,
            bind_password: info.get("bind_password").cloned().ok_or_else(|| LdapDriverError::InvalidConfig("missing 'bind_password'".to_string()))?,
            base_dn:       info.get("base_dn")      .cloned().ok_or_else(|| LdapDriverError::InvalidConfig("missing 'base_dn'".to_string()))?,
        })
    }
}

/// Real ldap3 connection, created on demand.
pub struct RealLdapConn {
    config: LdapConfig,
}

impl RealLdapConn {
    pub fn new(config: LdapConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl LdapConn for RealLdapConn {
    async fn search(&self, base_dn: &str, filter: &str) -> Result<Vec<LdapAttrList>, OxDataError> {
        use ldap3::{LdapConnAsync, Scope, SearchEntry};

        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP connect: {}", e)))?;
        ldap3::drive!(conn);

        ldap.simple_bind(&self.config.bind_dn, &self.config.bind_password)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP bind: {}", e)))?
            .success()
            .map_err(|e| OxDataError::DriverError(format!("LDAP bind rejected: {}", e)))?;

        let (rs, _res) = ldap
            .search(base_dn, Scope::Subtree, filter, vec!["*"])
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP search: {}", e)))?
            .success()
            .map_err(|e| OxDataError::DriverError(format!("LDAP search result: {}", e)))?;

        let entries: Vec<LdapAttrList> = rs
            .into_iter()
            .map(|entry| {
                let se = SearchEntry::construct(entry);
                se.attrs
                    .into_iter()
                    .collect()
            })
            .collect();

        ldap.unbind()
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP unbind: {}", e)))?;

        Ok(entries)
    }

    async fn add(&self, dn: &str, attrs: LdapAttrList) -> Result<(), OxDataError> {
        use ldap3::LdapConnAsync;

        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP connect: {}", e)))?;
        ldap3::drive!(conn);

        ldap.simple_bind(&self.config.bind_dn, &self.config.bind_password)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP bind: {}", e)))?
            .success()
            .map_err(|e| OxDataError::DriverError(format!("LDAP bind rejected: {}", e)))?;

        let ldap_attrs: Vec<(String, std::collections::HashSet<String>)> = attrs
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect();
        ldap.add(dn, ldap_attrs)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP add: {}", e)))?
            .success()
            .map_err(|e| OxDataError::DriverError(format!("LDAP add result: {}", e)))?;

        ldap.unbind()
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP unbind: {}", e)))?;

        Ok(())
    }

    async fn modify(&self, dn: &str, mods: Vec<(String, Vec<String>)>) -> Result<(), OxDataError> {
        use ldap3::{LdapConnAsync, Mod};

        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP connect: {}", e)))?;
        ldap3::drive!(conn);

        ldap.simple_bind(&self.config.bind_dn, &self.config.bind_password)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP bind: {}", e)))?
            .success()
            .map_err(|e| OxDataError::DriverError(format!("LDAP bind rejected: {}", e)))?;

        let ldap_mods: Vec<Mod<String>> = mods
            .into_iter()
            .map(|(attr, vals)| Mod::Replace(attr, vals.into_iter().collect()))
            .collect();

        ldap.modify(dn, ldap_mods)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP modify: {}", e)))?
            .success()
            .map_err(|e| OxDataError::DriverError(format!("LDAP modify result: {}", e)))?;

        ldap.unbind()
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP unbind: {}", e)))?;

        Ok(())
    }

    async fn delete(&self, dn: &str) -> Result<(), OxDataError> {
        use ldap3::LdapConnAsync;

        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP connect: {}", e)))?;
        ldap3::drive!(conn);

        ldap.simple_bind(&self.config.bind_dn, &self.config.bind_password)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP bind: {}", e)))?
            .success()
            .map_err(|e| OxDataError::DriverError(format!("LDAP bind rejected: {}", e)))?;

        ldap.delete(dn)
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP delete: {}", e)))?
            .success()
            .map_err(|e| OxDataError::DriverError(format!("LDAP delete result: {}", e)))?;

        ldap.unbind()
            .await
            .map_err(|e| OxDataError::DriverError(format!("LDAP unbind: {}", e)))?;

        Ok(())
    }
}

pub struct RealLdapConnFactory {
    config: LdapConfig,
}

impl RealLdapConnFactory {
    pub fn new(config: LdapConfig) -> Self {
        Self { config }
    }
}

impl LdapConnFactory for RealLdapConnFactory {
    fn create(&self) -> Arc<dyn LdapConn> {
        Arc::new(RealLdapConn::new(self.config.clone()))
    }
}

// ---------------------------------------------------------------------------
// Mock connection (tests only)
// ---------------------------------------------------------------------------

/// In-memory LDAP store backed by a HashMap<dn, LdapAttrList>.
#[derive(Clone, Default)]
pub struct MockLdapConn {
    store: Arc<Mutex<HashMap<String, LdapAttrList>>>,
}

impl MockLdapConn {
    pub fn new() -> Self {
        Self { store: Arc::new(Mutex::new(HashMap::new())) }
    }
}

#[async_trait]
impl LdapConn for MockLdapConn {
    async fn search(&self, base_dn: &str, filter: &str) -> Result<Vec<LdapAttrList>, OxDataError> {
        let store = self.store.lock().unwrap();
        // Filter: only entries whose DN ends with base_dn (subtree simulation).
        // For the mock we do a simple substring filter parse: "(attr=value)".
        let (filter_attr, filter_val) = parse_simple_filter(filter);
        let results: Vec<LdapAttrList> = store
            .iter()
            .filter(|(dn, _)| dn.ends_with(base_dn))
            .filter(|(_, attrs)| {
                if filter_attr == "objectClass" && filter_val == "*" {
                    return true;
                }
                attrs.iter().any(|(k, v)| {
                    k == &filter_attr && (filter_val == "*" || v.contains(&filter_val.to_string()))
                })
            })
            .map(|(_, attrs)| attrs.clone())
            .collect();
        Ok(results)
    }

    async fn add(&self, dn: &str, attrs: LdapAttrList) -> Result<(), OxDataError> {
        let mut store = self.store.lock().unwrap();
        store.insert(dn.to_string(), attrs);
        Ok(())
    }

    async fn modify(&self, dn: &str, mods: Vec<(String, Vec<String>)>) -> Result<(), OxDataError> {
        let mut store = self.store.lock().unwrap();
        let entry = store
            .get_mut(dn)
            .ok_or_else(|| OxDataError::DriverError(format!("MockLdapConn: DN not found: {}", dn)))?;
        for (attr, new_vals) in mods {
            // Replace existing attribute or add it.
            if let Some(existing) = entry.iter_mut().find(|(k, _)| k == &attr) {
                *existing = (attr, new_vals);
            } else {
                entry.push((attr, new_vals));
            }
        }
        Ok(())
    }

    async fn delete(&self, dn: &str) -> Result<(), OxDataError> {
        let mut store = self.store.lock().unwrap();
        store.remove(dn);
        Ok(())
    }
}

pub struct MockLdapConnFactory {
    conn: Arc<MockLdapConn>,
}

impl MockLdapConnFactory {
    pub fn new(conn: Arc<MockLdapConn>) -> Self {
        Self { conn }
    }
}

impl LdapConnFactory for MockLdapConnFactory {
    fn create(&self) -> Arc<dyn LdapConn> {
        self.conn.clone()
    }
}

// ---------------------------------------------------------------------------
// Minimal filter parser for mock
// ---------------------------------------------------------------------------

/// Parses "(attr=value)" into ("attr", "value").  Handles "(objectClass=*)" and
/// simple equality filters only — sufficient for the mock.
fn parse_simple_filter(filter: &str) -> (String, String) {
    let inner = filter.trim_start_matches('(').trim_end_matches(')');
    if let Some(pos) = inner.find('=') {
        let attr = inner[..pos].to_string();
        let val = inner[pos + 1..].to_string();
        (attr, val)
    } else {
        ("objectClass".to_string(), "*".to_string())
    }
}
