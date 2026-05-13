# LDAP + Active Directory Persistence Drivers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `ox_persistence_ldap` and `ox_persistence_ad` — two `PersistenceDriver` cdylib crates that translate the canonical IAM schema (`PrincipalRecord`, `SecurityGroup`, `GroupMember`, `PermissionGrant`, `SessionRecord`) to and from LDAP directory entries using the `ldap3` async client.

**Architecture:** `ox_persistence_ldap` owns all LDAP wire logic through an injected `LdapConnFactory` trait so tests never need a real LDAP server. Attribute name mappings are defined in a `SchemaMapping` struct so `ox_persistence_ad` can override only the AD-specific differences (e.g. `sAMAccountName` instead of `uid`, nested group OID) without duplicating logic. Both crates expose the standard FFI ABI (`ox_driver_init`, `ox_driver_persist`, `ox_driver_restore`, `ox_driver_fetch`, `ox_driver_get_driver_metadata`, etc.) as required by the persistence driver ABI.

**Tech Stack:** Rust, `ldap3 = "0.11"` (async, tls-native feature), `tokio = "1"` (rt-multi-thread), `async-trait = "0.1"`, `serde_json = "1"`, `ox_persistence`, `ox_data_object`, `ox_data_error`, `ox_type_converter`, `libc`

---

## File Structure

```
crates/data/ox_persistence/drivers/ldap/
  ox_persistence_ldap/
    Cargo.toml
    src/
      lib.rs            — PersistenceDriver impl + FFI exports
      mapping.rs        — SchemaMapping struct (attribute name tables)
      ldap_ops.rs       — LDAP search/add/modify helpers (takes &dyn LdapConn)
      conn_factory.rs   — LdapConnFactory trait + RealLdapConnFactory
      entity.rs         — canonical entity structs (PrincipalRecord, SecurityGroup, GroupMember, PermissionGrant, SessionRecord) + ser/de to/from ldap3::SearchEntry
      error.rs          — LdapDriverError -> OxDataError conversion
    tests/
      mock_conn.rs      — MockLdapConn: LdapConn for unit tests
      persist_tests.rs  — unit tests (no real server)
  ox_persistence_ad/
    Cargo.toml
    src/
      lib.rs            — AdPersistenceDriver wrapping LdapPersistenceDriver with AD SchemaMapping + FFI exports
    tests/
      ad_tests.rs       — AD-specific unit tests using MockLdapConn
```

Root workspace file to modify:
```
Cargo.toml   — add two new member entries under "# data"
```

---

## Task 1: Workspace registration and scaffolding

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/Cargo.toml`
- Create: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ad/Cargo.toml`

- [ ] **Step 1: Add both crates to workspace**

In `/var/repos/oxIDIZER/Cargo.toml`, inside the `members = [...]` array, after the line `"crates/data/ox_persistence/drivers/file/ox_persistence_driver_file_delimited",` add:

```toml
    "crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap",
    "crates/data/ox_persistence/drivers/ldap/ox_persistence_ad",
```

- [ ] **Step 2: Create `ox_persistence_ldap/Cargo.toml`**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/Cargo.toml`:

```toml
[package]
name = "ox_persistence_ldap"
version = "0.1.0"
license = "GPL-3.0-only"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_persistence  = { path = "../../../../ox_persistence" }
ox_data_object  = { path = "../../../../ox_data_object" }
ox_data_error   = { path = "../../../../ox_data_error" }
ox_type_converter = { path = "../../../../ox_type_converter" }
ldap3           = { version = "0.11", default-features = false, features = ["tls-native"] }
tokio           = { version = "1", features = ["rt-multi-thread", "macros"] }
async-trait     = "0.1"
serde_json      = "1"
libc            = "0.2"

[dev-dependencies]
tokio           = { version = "1", features = ["rt", "macros"] }
```

- [ ] **Step 3: Create `ox_persistence_ad/Cargo.toml`**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ad/Cargo.toml`:

```toml
[package]
name = "ox_persistence_ad"
version = "0.1.0"
license = "GPL-3.0-only"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_persistence_ldap = { path = "../ox_persistence_ldap" }
ox_persistence      = { path = "../../../../ox_persistence" }
ox_data_error       = { path = "../../../../ox_data_error" }
ox_type_converter   = { path = "../../../../ox_type_converter" }
serde_json          = "1"
libc                = "0.2"

[dev-dependencies]
tokio = { version = "1", features = ["rt", "macros"] }
```

- [ ] **Step 4: Verify workspace parses**

```bash
cd /var/repos/oxIDIZER && cargo metadata --no-deps --format-version 1 \
  | python3 -c "import sys,json; pkgs=[p['name'] for p in json.load(sys.stdin)['packages']]; print('ox_persistence_ldap' in pkgs, 'ox_persistence_ad' in pkgs)"
```

Expected output: `True True`

- [ ] **Step 5: Commit scaffolding**

```bash
git add Cargo.toml \
  crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/Cargo.toml \
  crates/data/ox_persistence/drivers/ldap/ox_persistence_ad/Cargo.toml
git commit -m "chore(data): scaffold ox_persistence_ldap and ox_persistence_ad workspace members"
```

---

## Task 2: Canonical entity structs and LDAP serialization (`entity.rs`)

**Files:**
- Create: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/entity.rs`

The entity module converts between the canonical IAM structs (held in a `HashMap<String, (String, ValueType, HashMap<String, String>)>` — the `serializable_map` used throughout the codebase) and LDAP attribute lists.

- [ ] **Step 1: Write the failing tests**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/tests/persist_tests.rs`:

```rust
// Tests for entity.rs — no LDAP server needed.
use ox_persistence_ldap::entity::{
    canonical_map_from_ldap_attrs, ldap_attrs_from_canonical_map,
};
use ox_persistence_ldap::mapping::SchemaMapping;
use std::collections::HashMap;
use ox_type_converter::ValueType;

fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

#[test]
fn ldap_attrs_round_trip_principal_record() {
    let mapping = SchemaMapping::ldap_defaults();
    let mut canon: HashMap<String, (String, ValueType, HashMap<String, String>)> = HashMap::new();
    canon.insert("principal_id".to_string(), str_entry("u001"));
    canon.insert("display_name".to_string(), str_entry("Alice"));
    canon.insert("source".to_string(), str_entry("Ldap"));
    canon.insert("tenant_id".to_string(), str_entry("tenant1"));

    let attrs = ldap_attrs_from_canonical_map(&canon, &mapping, "principals");
    // uid attribute must be set from principal_id
    assert!(attrs.iter().any(|(k, v)| k == "uid" && v.contains(&"u001".to_string())));
    // cn attribute must be set from display_name
    assert!(attrs.iter().any(|(k, v)| k == "cn" && v.contains(&"Alice".to_string())));

    // Round-trip back
    let restored = canonical_map_from_ldap_attrs(&attrs, &mapping, "principals");
    assert_eq!(restored.get("principal_id").unwrap().0, "u001");
    assert_eq!(restored.get("display_name").unwrap().0, "Alice");
}

#[test]
fn ldap_attrs_round_trip_security_group() {
    let mapping = SchemaMapping::ldap_defaults();
    let mut canon: HashMap<String, (String, ValueType, HashMap<String, String>)> = HashMap::new();
    canon.insert("group_id".to_string(), str_entry("grp-ops"));
    canon.insert("name".to_string(), str_entry("Operations"));
    canon.insert("source".to_string(), str_entry("Ldap"));
    canon.insert("tenant_id".to_string(), str_entry("tenant1"));

    let attrs = ldap_attrs_from_canonical_map(&canon, &mapping, "groups");
    assert!(attrs.iter().any(|(k, v)| k == "cn" && v.contains(&"grp-ops".to_string())));

    let restored = canonical_map_from_ldap_attrs(&attrs, &mapping, "groups");
    assert_eq!(restored.get("group_id").unwrap().0, "grp-ops");
    assert_eq!(restored.get("name").unwrap().0, "Operations");
}

#[test]
fn ldap_attrs_round_trip_permission_grant() {
    let mapping = SchemaMapping::ldap_defaults();
    let mut canon: HashMap<String, (String, ValueType, HashMap<String, String>)> = HashMap::new();
    canon.insert("node_path".to_string(), str_entry("com.justlikeef.data"));
    canon.insert("group_id".to_string(), str_entry("grp-ops"));
    canon.insert("operation_name".to_string(), str_entry("read"));
    canon.insert("allow_deny".to_string(), str_entry("Allow"));
    canon.insert("tenant_id".to_string(), str_entry("tenant1"));

    let attrs = ldap_attrs_from_canonical_map(&canon, &mapping, "grants");
    assert!(attrs.iter().any(|(k, _)| k == "oxNodePath"));
    assert!(attrs.iter().any(|(k, _)| k == "oxGroupId"));

    let restored = canonical_map_from_ldap_attrs(&attrs, &mapping, "grants");
    assert_eq!(restored.get("node_path").unwrap().0, "com.justlikeef.data");
    assert_eq!(restored.get("operation_name").unwrap().0, "read");
    assert_eq!(restored.get("allow_deny").unwrap().0, "Allow");
}
```

- [ ] **Step 2: Run to confirm test fails**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ldap 2>&1 | head -30
```

Expected: compile error — `entity` module doesn't exist yet.

- [ ] **Step 3: Implement `entity.rs`**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/entity.rs`:

```rust
//! Converts between the persistence layer's canonical serializable_map and
//! LDAP attribute lists.  This module is pure data transformation — no network I/O.

use std::collections::HashMap;
use ox_type_converter::ValueType;
use crate::mapping::SchemaMapping;

/// Canonical serializable map type (mirrors ox_persistence convention).
pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// LDAP attribute list: each entry is (attribute_name, Vec<value_string>).
pub type LdapAttrList = Vec<(String, Vec<String>)>;

/// Converts a canonical serializable_map into an LDAP attribute list for the
/// given `location` (e.g. "principals", "groups", "grants", "sessions").
/// Canonical field names are translated to LDAP attribute names via `mapping`.
pub fn ldap_attrs_from_canonical_map(
    map: &CanonicalMap,
    mapping: &SchemaMapping,
    location: &str,
) -> LdapAttrList {
    let mut attrs: LdapAttrList = Vec::new();
    for (canon_key, (value, _vtype, _meta)) in map {
        let ldap_attr = mapping.canonical_to_ldap(location, canon_key);
        // Skip internal oxid metadata key
        if ldap_attr == "__skip__" {
            continue;
        }
        attrs.push((ldap_attr, vec![value.clone()]));
    }
    // Inject required objectClass
    let object_class = mapping.object_class_for(location);
    if !object_class.is_empty() {
        attrs.push(("objectClass".to_string(), object_class));
    }
    attrs
}

/// Converts an LDAP attribute list back into a canonical serializable_map for
/// the given `location`.
pub fn canonical_map_from_ldap_attrs(
    attrs: &LdapAttrList,
    mapping: &SchemaMapping,
    location: &str,
) -> CanonicalMap {
    let mut map: CanonicalMap = HashMap::new();
    for (ldap_attr, values) in attrs {
        if ldap_attr == "objectClass" {
            continue;
        }
        if let Some(canon_key) = mapping.ldap_to_canonical(location, ldap_attr) {
            let value = values.first().cloned().unwrap_or_default();
            map.insert(canon_key, (value, ValueType::String, HashMap::new()));
        }
    }
    map
}

/// Extracts the primary key value from a canonical map for the given location.
/// Returns an empty string if not found.
pub fn primary_key_value(map: &CanonicalMap, mapping: &SchemaMapping, location: &str) -> String {
    let pk_field = mapping.primary_key_field(location);
    map.get(&pk_field)
        .map(|(v, _, _)| v.clone())
        .unwrap_or_default()
}

/// Returns the LDAP search filter that matches the primary key.
/// e.g. "(uid=alice)" for principals.
pub fn primary_key_filter(id: &str, mapping: &SchemaMapping, location: &str) -> String {
    let pk_attr = mapping.canonical_to_ldap(location, &mapping.primary_key_field(location));
    format!("({}={})", pk_attr, ldap_escape(id))
}

/// Returns an LDAP search filter built from all key/value pairs in `filter_map`.
/// Multiple filters are ANDed: "(&(uid=alice)(oxTenantId=tenant1))".
pub fn build_fetch_filter(
    filter_map: &CanonicalMap,
    mapping: &SchemaMapping,
    location: &str,
) -> String {
    let conditions: Vec<String> = filter_map
        .iter()
        .map(|(canon_key, (value, _, _))| {
            let ldap_attr = mapping.canonical_to_ldap(location, canon_key);
            format!("({}={})", ldap_attr, ldap_escape(value))
        })
        .collect();

    if conditions.is_empty() {
        "(objectClass=*)".to_string()
    } else if conditions.len() == 1 {
        conditions[0].clone()
    } else {
        format!("(&{})", conditions.concat())
    }
}

/// Escapes special characters in LDAP filter values per RFC 4515.
pub fn ldap_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '\\' => vec!['\\', '5', 'c'],
            '*'  => vec!['\\', '2', 'a'],
            '('  => vec!['\\', '2', '8'],
            ')'  => vec!['\\', '2', '9'],
            '\0' => vec!['\\', '0', '0'],
            c    => vec![c],
        })
        .collect()
}
```

- [ ] **Step 4: Run failing tests again — now only mapping module missing**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ldap 2>&1 | grep "error\|not found" | head -20
```

Expected: compile errors referencing `mapping` module not found (entity compiles fine once mapping exists).

---

## Task 3: Schema mapping (`mapping.rs`)

**Files:**
- Create: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/mapping.rs`

`SchemaMapping` holds tables that translate canonical field names ↔ LDAP attribute names per location. `ox_persistence_ad` builds its own `SchemaMapping::ad_defaults()` that overrides the handful of AD-specific attribute names.

- [ ] **Step 1: No new test — Task 2 tests already cover this via `SchemaMapping::ldap_defaults()`**

- [ ] **Step 2: Implement `mapping.rs`**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/mapping.rs`:

```rust
//! Attribute name mapping tables between canonical IAM fields and LDAP attributes.
//! `SchemaMapping::ldap_defaults()` covers standard LDAP (RFC 2798/4519).
//! `SchemaMapping::ad_defaults()` in ox_persistence_ad overrides AD-specific names.

use std::collections::HashMap;

/// Maps canonical field names to/from LDAP attribute names for each location.
#[derive(Clone, Debug)]
pub struct SchemaMapping {
    /// location -> (canonical_field -> ldap_attribute)
    canonical_to_ldap: HashMap<String, HashMap<String, String>>,
    /// location -> (ldap_attribute -> canonical_field)
    ldap_to_canonical: HashMap<String, HashMap<String, String>>,
    /// location -> objectClass values
    object_classes: HashMap<String, Vec<String>>,
    /// location -> canonical primary key field name
    primary_keys: HashMap<String, String>,
}

impl SchemaMapping {
    /// Standard LDAP defaults (RFC 2798 inetOrgPerson, groupOfNames, custom oxPermissionGrant, oxSession).
    pub fn ldap_defaults() -> Self {
        let mut c2l: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut l2c: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut oc: HashMap<String, Vec<String>> = HashMap::new();
        let mut pk: HashMap<String, String> = HashMap::new();

        // ---- principals (PrincipalRecord) ----
        let mut p_c2l = HashMap::new();
        p_c2l.insert("principal_id".to_string(),   "uid".to_string());
        p_c2l.insert("display_name".to_string(),   "cn".to_string());
        p_c2l.insert("source".to_string(),         "oxSource".to_string());
        p_c2l.insert("tenant_id".to_string(),      "oxTenantId".to_string());
        let mut p_l2c = HashMap::new();
        p_l2c.insert("uid".to_string(),           "principal_id".to_string());
        p_l2c.insert("cn".to_string(),            "display_name".to_string());
        p_l2c.insert("oxSource".to_string(),      "source".to_string());
        p_l2c.insert("oxTenantId".to_string(),    "tenant_id".to_string());
        c2l.insert("principals".to_string(), p_c2l);
        l2c.insert("principals".to_string(), p_l2c);
        oc.insert("principals".to_string(), vec!["inetOrgPerson".to_string(), "oxIAMPrincipal".to_string()]);
        pk.insert("principals".to_string(), "principal_id".to_string());

        // ---- groups (SecurityGroup) ----
        let mut g_c2l = HashMap::new();
        g_c2l.insert("group_id".to_string(),    "cn".to_string());
        g_c2l.insert("name".to_string(),        "description".to_string());
        g_c2l.insert("source".to_string(),      "oxSource".to_string());
        g_c2l.insert("tenant_id".to_string(),   "oxTenantId".to_string());
        let mut g_l2c = HashMap::new();
        g_l2c.insert("cn".to_string(),          "group_id".to_string());
        g_l2c.insert("description".to_string(), "name".to_string());
        g_l2c.insert("oxSource".to_string(),    "source".to_string());
        g_l2c.insert("oxTenantId".to_string(),  "tenant_id".to_string());
        c2l.insert("groups".to_string(), g_c2l);
        l2c.insert("groups".to_string(), g_l2c);
        oc.insert("groups".to_string(), vec!["groupOfNames".to_string(), "oxIAMGroup".to_string()]);
        pk.insert("groups".to_string(), "group_id".to_string());

        // ---- members (GroupMember) — stored as 'member' attributes on the group entry ----
        // GroupMember persists as a DN on the group; principal_id maps to member value.
        let mut m_c2l = HashMap::new();
        m_c2l.insert("principal_id".to_string(),  "member".to_string());
        m_c2l.insert("group_id".to_string(),      "__skip__".to_string()); // encoded in the group DN
        m_c2l.insert("tenant_id".to_string(),     "oxTenantId".to_string());
        let mut m_l2c = HashMap::new();
        m_l2c.insert("member".to_string(),        "principal_id".to_string());
        m_l2c.insert("oxTenantId".to_string(),    "tenant_id".to_string());
        c2l.insert("members".to_string(), m_c2l);
        l2c.insert("members".to_string(), m_l2c);
        oc.insert("members".to_string(), vec![]);
        pk.insert("members".to_string(), "principal_id".to_string());

        // ---- grants (PermissionGrant) — custom objectClass oxPermissionGrant ----
        let mut gr_c2l = HashMap::new();
        gr_c2l.insert("node_path".to_string(),      "oxNodePath".to_string());
        gr_c2l.insert("group_id".to_string(),        "oxGroupId".to_string());
        gr_c2l.insert("operation_name".to_string(),  "oxOperation".to_string());
        gr_c2l.insert("allow_deny".to_string(),      "oxAllowDeny".to_string());
        gr_c2l.insert("tenant_id".to_string(),       "oxTenantId".to_string());
        let mut gr_l2c = HashMap::new();
        gr_l2c.insert("oxNodePath".to_string(),     "node_path".to_string());
        gr_l2c.insert("oxGroupId".to_string(),      "group_id".to_string());
        gr_l2c.insert("oxOperation".to_string(),    "operation_name".to_string());
        gr_l2c.insert("oxAllowDeny".to_string(),    "allow_deny".to_string());
        gr_l2c.insert("oxTenantId".to_string(),     "tenant_id".to_string());
        c2l.insert("grants".to_string(), gr_c2l);
        l2c.insert("grants".to_string(), gr_l2c);
        oc.insert("grants".to_string(), vec!["oxPermissionGrant".to_string()]);
        pk.insert("grants".to_string(), "node_path".to_string());

        // ---- sessions (SessionRecord) — custom objectClass oxSession ----
        let mut s_c2l = HashMap::new();
        s_c2l.insert("session_id".to_string(),    "oxSessionId".to_string());
        s_c2l.insert("principal_id".to_string(),  "oxPrincipalId".to_string());
        s_c2l.insert("expires_at".to_string(),    "oxExpiresAt".to_string());
        s_c2l.insert("tenant_id".to_string(),     "oxTenantId".to_string());
        let mut s_l2c = HashMap::new();
        s_l2c.insert("oxSessionId".to_string(),   "session_id".to_string());
        s_l2c.insert("oxPrincipalId".to_string(), "principal_id".to_string());
        s_l2c.insert("oxExpiresAt".to_string(),   "expires_at".to_string());
        s_l2c.insert("oxTenantId".to_string(),    "tenant_id".to_string());
        c2l.insert("sessions".to_string(), s_c2l);
        l2c.insert("sessions".to_string(), s_l2c);
        oc.insert("sessions".to_string(), vec!["oxSession".to_string()]);
        pk.insert("sessions".to_string(), "session_id".to_string());

        Self { canonical_to_ldap: c2l, ldap_to_canonical: l2c, object_classes: oc, primary_keys: pk }
    }

    /// Translate a canonical field to its LDAP attribute name for the given location.
    /// Returns the canonical key unchanged if no mapping found (passthrough).
    /// Returns "__skip__" for fields that should be excluded from LDAP attributes.
    pub fn canonical_to_ldap(&self, location: &str, canonical_field: &str) -> String {
        self.canonical_to_ldap
            .get(location)
            .and_then(|m| m.get(canonical_field))
            .cloned()
            .unwrap_or_else(|| canonical_field.to_string())
    }

    /// Translate an LDAP attribute name back to the canonical field name for the given location.
    /// Returns None if the LDAP attribute is not mapped (it should be ignored).
    pub fn ldap_to_canonical(&self, location: &str, ldap_attr: &str) -> Option<String> {
        self.ldap_to_canonical
            .get(location)
            .and_then(|m| m.get(ldap_attr))
            .cloned()
    }

    /// Returns the objectClass list for the given location.
    pub fn object_class_for(&self, location: &str) -> Vec<String> {
        self.object_classes
            .get(location)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns the canonical field name that is the primary key for the given location.
    pub fn primary_key_field(&self, location: &str) -> String {
        self.primary_keys
            .get(location)
            .cloned()
            .unwrap_or_else(|| "id".to_string())
    }

    /// Override individual canonical->ldap mappings.  Used by ox_persistence_ad to
    /// swap in AD-specific attribute names without rebuilding the full table.
    pub fn with_override(mut self, location: &str, canonical_field: &str, ldap_attr: &str) -> Self {
        self.canonical_to_ldap
            .entry(location.to_string())
            .or_default()
            .insert(canonical_field.to_string(), ldap_attr.to_string());
        self.ldap_to_canonical
            .entry(location.to_string())
            .or_default()
            .insert(ldap_attr.to_string(), canonical_field.to_string());
        self
    }
}
```

- [ ] **Step 3: Run Task 2 tests — all should pass now**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ldap -- entity 2>&1 | tail -15
```

Expected: `test ldap_attrs_round_trip_principal_record ... ok`, `test ldap_attrs_round_trip_security_group ... ok`, `test ldap_attrs_round_trip_permission_grant ... ok`

- [ ] **Step 4: Commit entity + mapping**

```bash
git add crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/entity.rs \
        crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/mapping.rs \
        crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/tests/persist_tests.rs
git commit -m "feat(ldap-driver): add entity serialization and schema mapping for canonical IAM types"
```

---

## Task 4: Connection abstraction (`conn_factory.rs` and `error.rs`)

**Files:**
- Create: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/conn_factory.rs`
- Create: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/error.rs`

- [ ] **Step 1: Write failing tests for the mock factory**

Add to `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/tests/persist_tests.rs`:

```rust
mod conn_tests {
    use ox_persistence_ldap::conn_factory::{LdapConn, MockLdapConn};
    use ox_persistence_ldap::entity::LdapAttrList;

    #[tokio::test]
    async fn mock_conn_add_then_search_returns_entry() {
        let mock = MockLdapConn::new();
        let attrs: LdapAttrList = vec![
            ("uid".to_string(), vec!["alice".to_string()]),
            ("cn".to_string(), vec!["Alice Liddell".to_string()]),
            ("objectClass".to_string(), vec!["inetOrgPerson".to_string()]),
        ];

        mock.add("uid=alice,ou=people,dc=example,dc=com", attrs.clone()).await.unwrap();

        let results = mock
            .search("ou=people,dc=example,dc=com", "(uid=alice)")
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].iter().any(|(k, v)| k == "uid" && v.contains(&"alice".to_string())));
    }

    #[tokio::test]
    async fn mock_conn_modify_updates_attribute() {
        let mock = MockLdapConn::new();
        let attrs: LdapAttrList = vec![
            ("uid".to_string(), vec!["bob".to_string()]),
            ("cn".to_string(), vec!["Bob Original".to_string()]),
            ("objectClass".to_string(), vec!["inetOrgPerson".to_string()]),
        ];
        mock.add("uid=bob,ou=people,dc=example,dc=com", attrs).await.unwrap();

        mock.modify(
            "uid=bob,ou=people,dc=example,dc=com",
            vec![("cn".to_string(), vec!["Bob Updated".to_string()])],
        )
        .await
        .unwrap();

        let results = mock
            .search("ou=people,dc=example,dc=com", "(uid=bob)")
            .await
            .unwrap();
        assert!(results[0].iter().any(|(k, v)| k == "cn" && v.contains(&"Bob Updated".to_string())));
    }
}
```

- [ ] **Step 2: Run to confirm fail**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ldap -- conn_tests 2>&1 | head -20
```

Expected: compile error — `conn_factory` module not found.

- [ ] **Step 3: Implement `error.rs`**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/error.rs`:

```rust
//! Maps ldap3 and driver-specific errors to OxDataError.

use ox_data_error::OxDataError;

#[derive(Debug)]
pub enum LdapDriverError {
    ConnectionFailed(String),
    BindFailed(String),
    SearchFailed(String),
    AddFailed(String),
    ModifyFailed(String),
    NotFound(String),
    InvalidConfig(String),
}

impl From<LdapDriverError> for OxDataError {
    fn from(e: LdapDriverError) -> Self {
        match e {
            LdapDriverError::ConnectionFailed(m) => OxDataError::DriverError(format!("LDAP connection failed: {}", m)),
            LdapDriverError::BindFailed(m)       => OxDataError::DriverError(format!("LDAP bind failed: {}", m)),
            LdapDriverError::SearchFailed(m)     => OxDataError::DriverError(format!("LDAP search failed: {}", m)),
            LdapDriverError::AddFailed(m)        => OxDataError::DriverError(format!("LDAP add failed: {}", m)),
            LdapDriverError::ModifyFailed(m)     => OxDataError::DriverError(format!("LDAP modify failed: {}", m)),
            LdapDriverError::NotFound(id)        => OxDataError::InternalError(format!("LDAP entry not found: {}", id)),
            LdapDriverError::InvalidConfig(m)    => OxDataError::DriverError(format!("LDAP driver config error: {}", m)),
        }
    }
}
```

- [ ] **Step 4: Implement `conn_factory.rs`**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/conn_factory.rs`:

```rust
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
                    .map(|(k, v)| (k, v))
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

        let ldap_attrs: Vec<(String, Vec<String>)> = attrs;
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
```

- [ ] **Step 5: Run Task 4 tests**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ldap -- conn_tests 2>&1 | tail -15
```

Expected: `test conn_tests::mock_conn_add_then_search_returns_entry ... ok`, `test conn_tests::mock_conn_modify_updates_attribute ... ok`

- [ ] **Step 6: Commit connection abstraction**

```bash
git add crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/error.rs \
        crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/conn_factory.rs
git commit -m "feat(ldap-driver): add LdapConn trait, MockLdapConn, RealLdapConn, and error types"
```

---

## Task 5: `PersistenceDriver` implementation (`lib.rs`)

**Files:**
- Create: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/lib.rs`
- Extend: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/tests/persist_tests.rs`

- [ ] **Step 1: Write the failing PersistenceDriver tests**

Add to `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/tests/persist_tests.rs`:

```rust
mod driver_tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use ox_persistence::PersistenceDriver;
    use ox_persistence_ldap::LdapPersistenceDriver;
    use ox_persistence_ldap::conn_factory::{MockLdapConn, MockLdapConnFactory};
    use ox_persistence_ldap::mapping::SchemaMapping;
    use ox_type_converter::ValueType;

    fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
        (v.to_string(), ValueType::String, HashMap::new())
    }

    fn make_driver() -> (LdapPersistenceDriver, Arc<MockLdapConn>) {
        let mock_conn = Arc::new(MockLdapConn::new());
        let factory = Arc::new(MockLdapConnFactory::new(mock_conn.clone()));
        let mut conn_info = HashMap::new();
        conn_info.insert("url".to_string(),           "ldap://localhost:389".to_string());
        conn_info.insert("bind_dn".to_string(),       "cn=admin,dc=example,dc=com".to_string());
        conn_info.insert("bind_password".to_string(), "secret".to_string());
        conn_info.insert("base_dn".to_string(),       "dc=example,dc=com".to_string());
        let driver = LdapPersistenceDriver::new_with_factory(factory, SchemaMapping::ldap_defaults(), conn_info);
        (driver, mock_conn)
    }

    #[test]
    fn persist_principal_record_creates_ldap_entry() {
        let (driver, mock_conn) = make_driver();
        let mut data = HashMap::new();
        data.insert("principal_id".to_string(), str_entry("alice"));
        data.insert("display_name".to_string(), str_entry("Alice Liddell"));
        data.insert("source".to_string(),       str_entry("Ldap"));
        data.insert("tenant_id".to_string(),    str_entry("t1"));

        driver.persist(&data, "principals").expect("persist failed");

        // The mock should now contain an entry for alice
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let results = rt.block_on(async {
            mock_conn.search("dc=example,dc=com", "(uid=alice)").await.unwrap()
        });
        assert_eq!(results.len(), 1);
        assert!(results[0].iter().any(|(k, v)| k == "uid" && v.contains(&"alice".to_string())));
    }

    #[test]
    fn restore_principal_by_id_returns_entry() {
        let (driver, _) = make_driver();
        let mut data = HashMap::new();
        data.insert("principal_id".to_string(), str_entry("bob"));
        data.insert("display_name".to_string(), str_entry("Bob Builder"));
        data.insert("source".to_string(),       str_entry("Ldap"));
        data.insert("tenant_id".to_string(),    str_entry("t1"));
        driver.persist(&data, "principals").expect("persist failed");

        let restored = driver.restore("principals", "bob").expect("restore failed");
        assert_eq!(restored.get("principal_id").unwrap().0, "bob");
        assert_eq!(restored.get("display_name").unwrap().0, "Bob Builder");
    }

    #[test]
    fn fetch_principals_by_tenant_returns_ids() {
        let (driver, _) = make_driver();
        for name in &["carol", "dave"] {
            let mut data = HashMap::new();
            data.insert("principal_id".to_string(), str_entry(name));
            data.insert("display_name".to_string(), str_entry(name));
            data.insert("source".to_string(),       str_entry("Ldap"));
            data.insert("tenant_id".to_string(),    str_entry("t1"));
            driver.persist(&data, "principals").expect("persist failed");
        }

        let mut filter = HashMap::new();
        filter.insert("tenant_id".to_string(), str_entry("t1"));
        let ids = driver.fetch(&filter, "principals").expect("fetch failed");
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"carol".to_string()));
        assert!(ids.contains(&"dave".to_string()));
    }

    #[test]
    fn persist_permission_grant_creates_grant_entry() {
        let (driver, mock_conn) = make_driver();
        let mut data = HashMap::new();
        data.insert("node_path".to_string(),     str_entry("com.justlikeef.data"));
        data.insert("group_id".to_string(),       str_entry("grp-ops"));
        data.insert("operation_name".to_string(), str_entry("read"));
        data.insert("allow_deny".to_string(),     str_entry("Allow"));
        data.insert("tenant_id".to_string(),      str_entry("t1"));

        driver.persist(&data, "grants").expect("persist failed");

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let results = rt.block_on(async {
            mock_conn.search("dc=example,dc=com", "(oxNodePath=com.justlikeef.data)").await.unwrap()
        });
        assert_eq!(results.len(), 1);
        assert!(results[0].iter().any(|(k, v)| k == "oxGroupId" && v.contains(&"grp-ops".to_string())));
    }

    #[test]
    fn list_datasets_returns_supported_locations() {
        let (driver, _) = make_driver();
        let conn_info = HashMap::new();
        let datasets = driver.list_datasets(&conn_info).expect("list failed");
        assert!(datasets.contains(&"principals".to_string()));
        assert!(datasets.contains(&"groups".to_string()));
        assert!(datasets.contains(&"grants".to_string()));
        assert!(datasets.contains(&"sessions".to_string()));
    }
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ldap -- driver_tests 2>&1 | head -25
```

Expected: compile errors — `LdapPersistenceDriver` not defined.

- [ ] **Step 3: Implement `lib.rs`**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/lib.rs`:

```rust
//! ox_persistence_ldap — LDAP persistence driver for the canonical IAM schema.
//! Implements the PersistenceDriver trait from ox_persistence.

pub mod conn_factory;
pub mod entity;
pub mod error;
pub mod mapping;

use std::collections::HashMap;
use std::sync::Arc;
use ox_data_error::OxDataError;
use ox_persistence::{PersistenceDriver, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer};
use ox_type_converter::ValueType;

use conn_factory::{LdapConnFactory, LdapConfig, RealLdapConnFactory};
use entity::{ldap_attrs_from_canonical_map, canonical_map_from_ldap_attrs,
             primary_key_value, primary_key_filter, build_fetch_filter};
use mapping::SchemaMapping;

// Re-export MockLdapConn so integration tests can import it from the crate root in tests.
#[cfg(any(test, feature = "test-support"))]
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
// ---------------------------------------------------------------------------

use std::ffi::{c_void, CString, CStr};
use libc::c_char;

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

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut LdapPersistenceDriver);
    }
}

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

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}
```

- [ ] **Step 4: Run all LDAP tests**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ldap 2>&1 | tail -20
```

Expected: all 8 tests pass (3 entity tests + 2 conn tests + 5 driver tests including list_datasets).

- [ ] **Step 5: Commit LDAP driver**

```bash
git add crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/lib.rs \
        crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/tests/persist_tests.rs
git commit -m "feat(ldap-driver): implement LdapPersistenceDriver with full PersistenceDriver trait and FFI exports (8 tests)"
```

---

## Task 6: Active Directory driver (`ox_persistence_ad`)

**Files:**
- Create: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ad/src/lib.rs`
- Create: `crates/data/ox_persistence/drivers/ldap/ox_persistence_ad/tests/ad_tests.rs`

The AD driver wraps `LdapPersistenceDriver` with an AD-specific `SchemaMapping` and overrides the FFI symbols. Key AD differences:
- `sAMAccountName` instead of `uid` for `principal_id`
- `userPrincipalName` carries the UPN (stored as `oxSource` annotation)
- Groups use `member` attribute with full DNs; nested group traversal uses `memberOf:1.2.840.113556.1.4.1941:=<dn>` (LDAP_MATCHING_RULE_IN_CHAIN) for deep membership
- `objectClass` for users is `user`; for groups is `group`

- [ ] **Step 1: Write failing AD tests**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ad/tests/ad_tests.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;
use ox_persistence::PersistenceDriver;
use ox_persistence_ad::AdPersistenceDriver;
use ox_persistence_ldap::conn_factory::{MockLdapConn, MockLdapConnFactory};
use ox_type_converter::ValueType;

fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

fn make_ad_driver() -> (AdPersistenceDriver, Arc<MockLdapConn>) {
    let mock_conn = Arc::new(MockLdapConn::new());
    let factory = Arc::new(MockLdapConnFactory::new(mock_conn.clone()));
    let mut conn_info = HashMap::new();
    conn_info.insert("url".to_string(),           "ldap://ad.corp.example.com:389".to_string());
    conn_info.insert("bind_dn".to_string(),       "CN=svc,CN=Users,DC=corp,DC=example,DC=com".to_string());
    conn_info.insert("bind_password".to_string(), "P@ssw0rd".to_string());
    conn_info.insert("base_dn".to_string(),       "DC=corp,DC=example,DC=com".to_string());
    let driver = AdPersistenceDriver::new_with_factory(factory, conn_info);
    (driver, mock_conn)
}

#[test]
fn ad_driver_uses_samaccountname_for_principal_id() {
    let (driver, mock_conn) = make_ad_driver();
    let mut data = HashMap::new();
    data.insert("principal_id".to_string(), str_entry("jsmith"));
    data.insert("display_name".to_string(), str_entry("John Smith"));
    data.insert("source".to_string(),       str_entry("Ad"));
    data.insert("tenant_id".to_string(),    str_entry("corp"));

    driver.persist(&data, "principals").expect("persist failed");

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let results = rt.block_on(async {
        mock_conn.search("DC=corp,DC=example,DC=com", "(sAMAccountName=jsmith)").await.unwrap()
    });
    assert_eq!(results.len(), 1);
    assert!(results[0].iter().any(|(k, v)| k == "sAMAccountName" && v.contains(&"jsmith".to_string())));
}

#[test]
fn ad_driver_restore_principal_by_samaccountname() {
    let (driver, _) = make_ad_driver();
    let mut data = HashMap::new();
    data.insert("principal_id".to_string(), str_entry("jdoe"));
    data.insert("display_name".to_string(), str_entry("Jane Doe"));
    data.insert("source".to_string(),       str_entry("Ad"));
    data.insert("tenant_id".to_string(),    str_entry("corp"));
    driver.persist(&data, "principals").expect("persist failed");

    let restored = driver.restore("principals", "jdoe").expect("restore failed");
    assert_eq!(restored.get("principal_id").unwrap().0, "jdoe");
    assert_eq!(restored.get("display_name").unwrap().0, "Jane Doe");
}

#[test]
fn ad_driver_group_uses_ad_object_class() {
    let (driver, mock_conn) = make_ad_driver();
    let mut data = HashMap::new();
    data.insert("group_id".to_string(),  str_entry("Domain Admins"));
    data.insert("name".to_string(),      str_entry("Domain Administrators"));
    data.insert("source".to_string(),    str_entry("Ad"));
    data.insert("tenant_id".to_string(), str_entry("corp"));
    driver.persist(&data, "groups").expect("persist failed");

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let results = rt.block_on(async {
        mock_conn.search("DC=corp,DC=example,DC=com", "(cn=Domain Admins)").await.unwrap()
    });
    assert_eq!(results.len(), 1);
    // AD groups use objectClass=group
    assert!(results[0].iter().any(|(k, v)| k == "objectClass" && v.contains(&"group".to_string())));
}

#[test]
fn ad_driver_list_datasets_returns_standard_locations() {
    let (driver, _) = make_ad_driver();
    let datasets = driver.list_datasets(&HashMap::new()).expect("list failed");
    assert!(datasets.contains(&"principals".to_string()));
    assert!(datasets.contains(&"groups".to_string()));
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ad 2>&1 | head -25
```

Expected: compile error — `AdPersistenceDriver` not defined.

- [ ] **Step 3: Implement `ox_persistence_ad/src/lib.rs`**

Create `crates/data/ox_persistence/drivers/ldap/ox_persistence_ad/src/lib.rs`:

```rust
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

// Re-export MockLdapConn from ox_persistence_ldap for test convenience.
#[cfg(any(test, feature = "test-support"))]
pub use ox_persistence_ldap::conn_factory::{MockLdapConn, MockLdapConnFactory};

/// Returns the AD-specific schema mapping by starting with LDAP defaults and
/// applying AD attribute overrides.
pub fn ad_schema_mapping() -> SchemaMapping {
    SchemaMapping::ldap_defaults()
        // AD uses sAMAccountName as the login identifier instead of uid
        .with_override("principals", "principal_id", "sAMAccountName")
        // AD user objectClass is "user" not "inetOrgPerson"
        // Note: objectClass list is set via with_object_class_override (not needed here
        // because the mock test checks for the correct objectClass set by AD mapping).
        // The inner LdapPersistenceDriver will still emit "objectClass" from the mapping's
        // object_class_for("principals") entry. We accept that the ldap_defaults entry
        // carries inetOrgPerson; to fully correct this the SchemaMapping::with_override
        // would need a with_object_class_override helper — add that if the AD object class
        // test fails and adjust accordingly.
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
    CString::new(serde_json::to_string(&metadata).expect("serialize")).expect("CString").into_raw()
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
    CString::new(schema).expect("CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}
```

- [ ] **Step 4: Run all AD tests**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ad 2>&1 | tail -15
```

Expected: 4 tests pass (`ad_driver_uses_samaccountname_for_principal_id`, `ad_driver_restore_principal_by_samaccountname`, `ad_driver_group_uses_ad_object_class`, `ad_driver_list_datasets_returns_standard_locations`).

If `ad_driver_group_uses_ad_object_class` fails because objectClass still shows `groupOfNames` instead of `group`, extend `SchemaMapping` with a `with_object_classes` method:

```rust
// In mapping.rs — add this method to SchemaMapping impl:
pub fn with_object_classes(mut self, location: &str, classes: Vec<String>) -> Self {
    self.object_classes.insert(location.to_string(), classes);
    self
}
```

Then in `ad_schema_mapping()`:
```rust
pub fn ad_schema_mapping() -> SchemaMapping {
    SchemaMapping::ldap_defaults()
        .with_override("principals", "principal_id", "sAMAccountName")
        .with_object_classes("principals", vec!["user".to_string(), "oxIAMPrincipal".to_string()])
        .with_object_classes("groups", vec!["group".to_string(), "oxIAMGroup".to_string()])
}
```

- [ ] **Step 5: Run full test suite for both crates**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ldap -p ox_persistence_ad 2>&1 | tail -20
```

Expected: all 12 tests pass (8 LDAP + 4 AD).

- [ ] **Step 6: Commit AD driver**

```bash
git add crates/data/ox_persistence/drivers/ldap/ox_persistence_ad/src/lib.rs \
        crates/data/ox_persistence/drivers/ldap/ox_persistence_ad/tests/ad_tests.rs \
        crates/data/ox_persistence/drivers/ldap/ox_persistence_ldap/src/mapping.rs
git commit -m "feat(ad-driver): implement AdPersistenceDriver wrapping LdapPersistenceDriver with AD attribute overrides (4 tests)"
```

---

## Task 7: Final integration check

- [ ] **Step 1: Build all workspace crates to catch any ripple**

```bash
cd /var/repos/oxIDIZER && cargo build --workspace 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 2: Run all tests in both new crates**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_ldap -p ox_persistence_ad -- --test-threads=4 2>&1 | tail -25
```

Expected: 12 tests, 0 failures.

- [ ] **Step 3: Commit final state if anything was adjusted**

```bash
git add -p   # stage only relevant changes
git commit -m "fix(ldap-ad-drivers): post-integration adjustments"
```
