use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ReportPluginConfig {
    /// Path to the shared manifest_instance.db (same as ManifestPluginConfig.db_path).
    pub db_path: String,

    /// Encryption key for the SQLite database (SQLCipher PRAGMA key).
    /// Must match the key configured in ox_cc_manifest_plugin since both share the file.
    pub db_encryption_key: String,

    /// Per-client rate limits.
    #[serde(default)]
    pub rate_limits: RateLimits,
}

#[derive(Debug, Deserialize, Default)]
pub struct RateLimits {
    /// Maximum reports accepted from a single client per minute.
    #[serde(default = "default_reports_per_client_per_minute")]
    pub reports_per_client_per_minute: u32,
}

impl ReportPluginConfig {
    pub fn load(path: &str) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config {}: {}", path, e))?;
        let cfg: Self = serde_yaml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse config {}: {}", path, e))?;
        Ok(cfg)
    }
}

fn default_reports_per_client_per_minute() -> u32 {
    60
}
