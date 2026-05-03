use ox_cert_core::model::{CertStoreConfig, KeyStoreConfig};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CaInitConfig {
    pub tenant_id: String,
    pub keystore: KeyStoreConfig,
    pub ca: CaHierarchyConfig,
    #[serde(default)]
    pub auto_generate: bool,
    #[serde(default)]
    pub extensions: ExtensionsConfig,
    pub store: CertStoreConfig,
}

#[derive(Debug, Deserialize)]
pub struct CaHierarchyConfig {
    pub root: CaCertConfig,
    pub intermediate: CaCertConfig,
}

#[derive(Debug, Deserialize)]
pub struct CaCertConfig {
    /// Used as key_id in KeyStore.
    pub key_path: String,
    /// Filesystem path where the CA cert PEM is written/read.
    pub cert_path: String,
    /// "ecc-p256" | "ecc-p384" | "ecc-p521" | "ed25519" | "rsa-2048" | "rsa-3072" | "rsa-4096"
    pub key_type: String,
    pub validity_years: u32,
    /// Full DN string, e.g. "CN=Root CA, O=Acme, C=US"
    pub subject: String,
    #[serde(default)]
    pub name_constraints: Option<NameConstraintsConfig>,
    pub path_length: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct NameConstraintsConfig {
    #[serde(default)]
    pub permitted_dns: Vec<String>,
    #[serde(default)]
    pub excluded_dns: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ExtensionsConfig {
    pub aia: Option<AiaConfig>,
    pub cdp: Option<CdpConfig>,
}

#[derive(Debug, Deserialize)]
pub struct AiaConfig {
    pub ocsp_url: Option<String>,
    pub ca_issuer_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CdpConfig {
    pub url: Option<String>,
}
