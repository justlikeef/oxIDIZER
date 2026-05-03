use serde::Deserialize;

/// Configuration for the manifest plugin, loaded from YAML at startup.
#[derive(Debug, Deserialize)]
pub struct ManifestPluginConfig {
    /// Path to the shared SQLite database (manifest_instance.db).
    pub db_path: String,

    /// Encryption key for the SQLite database (SQLCipher PRAGMA key).
    pub db_encryption_key: String,

    /// Maximum valid window for manifests (expires_at - issued_at), in seconds.
    #[serde(default = "default_max_manifest_window_secs")]
    pub max_manifest_window_secs: u64,

    /// List of broker public keys (base64url) to distribute to clients during bootstrap.
    pub broker_pubkeys: Vec<String>,

    /// Public URL of the manifest service.
    pub manifest_url: String,

    /// Public URL of the report service.
    pub report_url: String,
}

impl ManifestPluginConfig {
    pub fn load(path: &str) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config {}: {}", path, e))?;
        let cfg: Self = serde_yaml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse config {}: {}", path, e))?;
        Ok(cfg)
    }
}

fn default_max_manifest_window_secs() -> u64 {
    90 * 24 * 3600
}
