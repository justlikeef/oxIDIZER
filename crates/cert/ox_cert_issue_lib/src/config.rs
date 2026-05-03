use ox_cert_core::model::{CertStoreConfig, CtConfig, IssuancePolicyConfig, KeyStoreConfig};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CertIssueConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub default_profile: String,
    pub policy: IssuancePolicyConfig,
    #[serde(default)]
    pub ct: Option<CtConfig>,
    #[serde(default)]
    pub extensions: ExtensionsConfig,
    /// Key ID of the intermediate CA signing key in the KeyStore.
    pub ca_intermediate_key_id: String,
    /// Filesystem path to the intermediate CA cert PEM (for chain building and issuer DN).
    pub ca_intermediate_cert_path: String,
    /// Filesystem path to the root CA cert PEM (for chain building).
    pub ca_root_cert_path: String,
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
