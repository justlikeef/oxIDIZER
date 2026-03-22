use serde::Deserialize;
use std::collections::HashMap; // used by policy field

/// Configuration for the broker plugin, loaded from a YAML file at startup.
#[derive(Debug, Deserialize)]
pub struct BrokerPluginConfig {
    /// Path to the SQLite broker database. Created if absent.
    pub db_path: String,

    /// Encryption key for the SQLite database (SQLCipher PRAGMA key).
    pub db_encryption_key: String,

    /// Path to the broker's Ed25519 signing private key (PEM or raw hex).
    pub signing_key_path: String,

    /// Path to the broker's X25519 encryption private key.
    pub enc_key_path: String,

    /// Cipher to use for payload encryption. One of: "aes256gcm", "chacha20poly1305".
    #[serde(default = "default_cipher")]
    pub cipher: String,

    /// Maximum age of a pending template before it auto-expires (seconds).
    #[serde(default = "default_pending_ttl_secs")]
    pub pending_ttl_secs: u64,

    /// Maximum allowed manifest validity window (expires_at - issued_at).
    /// Default: 90 days.
    #[serde(default = "default_max_manifest_window_secs")]
    pub max_manifest_window_secs: u64,

    /// Directory where encrypted payload files are stored.
    /// MUST be on an encrypted-at-rest filesystem. Only base filenames are
    /// stored in the DB; all paths are resolved relative to this directory.
    pub payload_dir: String,

    /// Per-consumer payload policy allowlists.
    /// Key: consumer name (e.g. "arcnition").
    /// Value: list of allowed top-level payload keys.
    #[serde(default)]
    pub policy: HashMap<String, ConsumerPolicy>,

}

#[derive(Debug, Deserialize, Default)]
pub struct ConsumerPolicy {
    /// Allowed top-level keys in the payload object for this consumer.
    #[serde(default)]
    pub allowed_payload_keys: Vec<String>,
}

impl BrokerPluginConfig {
    pub fn load(path: &str) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config {}: {}", path, e))?;
        let cfg: Self = serde_yaml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse config {}: {}", path, e))?;
        Ok(cfg)
    }
}

fn default_cipher() -> String {
    "aes256gcm".to_string()
}

fn default_pending_ttl_secs() -> u64 {
    86_400 // 24 hours
}

fn default_max_manifest_window_secs() -> u64 {
    90 * 24 * 3600 // 90 days
}
