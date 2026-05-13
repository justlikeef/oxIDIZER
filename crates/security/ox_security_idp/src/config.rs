use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OAuthClientDef {
    pub client_id: String,
    pub client_secret_hash: Option<String>,
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub allowed_scopes: Vec<String>,
    #[serde(default)]
    pub allowed_grants: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SamlSpDef {
    pub entity_id: String,
    pub acs_url: String,
    pub slo_url: Option<String>,
    pub name_id_format: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct IdpConfig {
    pub tenant_id: String,
    pub issuer: String,
    pub rsa_private_key_pem: String,
    #[serde(default = "default_token_ttl")]
    pub access_token_ttl_secs: u64,
    #[serde(default = "default_refresh_ttl")]
    pub refresh_token_ttl_secs: u64,
    #[serde(default)]
    pub clients: Vec<OAuthClientDef>,
    #[serde(default)]
    pub saml_sps: Vec<SamlSpDef>,
}

fn default_token_ttl() -> u64 { 3600 }
fn default_refresh_ttl() -> u64 { 86400 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_config_deserializes() {
        let json = r#"{"tenant_id":"t1","issuer":"https://auth.example.com","rsa_private_key_pem":"PLACEHOLDER","clients":[],"saml_sps":[]}"#;
        let cfg: IdpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.tenant_id, "t1");
        assert_eq!(cfg.access_token_ttl_secs, 3600);
    }
}
