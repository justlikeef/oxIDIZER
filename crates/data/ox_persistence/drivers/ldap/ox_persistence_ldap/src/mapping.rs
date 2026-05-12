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
        // Remove the old reverse mapping if present
        if let Some(old_ldap) = self.canonical_to_ldap
            .get(location)
            .and_then(|m| m.get(canonical_field))
            .cloned()
        {
            self.ldap_to_canonical
                .entry(location.to_string())
                .or_default()
                .remove(&old_ldap);
        }
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

    /// Override objectClass list for a location.  Used by ox_persistence_ad.
    pub fn with_object_classes(mut self, location: &str, classes: Vec<String>) -> Self {
        self.object_classes.insert(location.to_string(), classes);
        self
    }
}
