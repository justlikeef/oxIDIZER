use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AdminPluginConfig {
    /// Path to the admin SQLite database.
    pub db_path: String,

    /// Encryption key for the SQLite database (SQLCipher PRAGMA key).
    pub db_encryption_key: String,

    /// URL of the Broker instance (e.g. "https://broker.internal").
    pub broker_url: String,

    /// URL of the Manifest instance (e.g. "https://manifest.example.com").
    pub manifest_instance_url: String,

    /// mTLS credentials used by the admin plugin when calling the broker
    /// and manifest instance. These are the operator-level admin role certs.
    pub tls: AdminTlsConfig,
}

#[derive(Debug, Deserialize)]
pub struct AdminTlsConfig {
    /// Path to the admin role TLS certificate (PEM).
    pub client_cert: String,
    /// Path to the admin role TLS private key (PEM).
    pub client_key: String,
    /// Path to the CA certificate for validating the broker/manifest servers.
    pub ca_cert: String,
}

impl AdminPluginConfig {
    pub fn load(path: &str) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config {}: {}", path, e))?;
        let cfg: Self = serde_yaml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse config {}: {}", path, e))?;
        Ok(cfg)
    }
}
