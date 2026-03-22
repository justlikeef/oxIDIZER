use anyhow::Result;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use std::collections::HashMap;
use x25519_dalek::StaticSecret;

/// Client configuration loaded from YAML on startup.
#[derive(Debug, Deserialize)]
pub struct ClientConfig {
    /// Hostname or identifier for this client.
    /// Must match the client_id used during enrollment.
    pub client_id: String,

    /// URL of the Manifest instance (e.g. "https://manifest.example.com").
    pub manifest_url: String,

    /// mTLS credentials used to authenticate to the Manifest instance.
    pub tls: ClientTlsConfig,

    /// Path to the SQLite state database. Created if absent.
    pub db_path: String,

    /// Encryption key for the SQLite database (SQLCipher PRAGMA key).
    pub db_encryption_key: String,

    /// Poll interval in seconds (how often to check for new manifests).
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,

    /// Maximum allowed manifest validity window in seconds.
    #[serde(default = "default_max_manifest_window_secs")]
    pub max_manifest_window_secs: u64,

    /// Directory containing trusted broker Ed25519 verifying (public) key files.
    /// Each `.pub` file must contain 32 raw bytes. All files are loaded and tried
    /// in sequence; a signature is accepted if any key validates it. This supports
    /// key rotation with overlapping trust windows.
    pub broker_signing_pubkeys_dir: String,

    /// Client X25519 encryption private key, base64url-encoded.
    pub client_enc_privkey_b64: String,

    /// Map of consumer name → directory where manifest.json is written.
    /// E.g. { "arcnition": "/var/lib/arcnition/manifest" }
    #[serde(default)]
    pub consumer_dirs: HashMap<String, String>,

    /// URL to POST the "applied" notification after successful apply.
    /// Usually the manifest instance's /cc/report/{client_id} endpoint.
    pub report_url: String,

    /// Directory to search for external command plugin binaries.
    /// Commands not matching a built-in are looked up as `{plugin_dir}/{command_name}`.
    #[serde(default)]
    pub plugin_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClientTlsConfig {
    /// Path to the client TLS certificate (PEM).
    pub client_cert: String,
    /// Path to the client TLS private key (PEM, mode 440, group ox_cc).
    pub client_key: String,
    /// Path to the server CA certificate for validating the manifest instance.
    pub ca_cert: String,
}

impl ClientConfig {
    pub fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config {}: {}", path, e))?;
        let cfg: Self = serde_yaml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse config {}: {}", path, e))?;
        Ok(cfg)
    }

    /// Load all broker Ed25519 verifying keys from `broker_signing_pubkeys_dir`.
    ///
    /// Each `.pub` file must contain exactly 32 raw bytes (the compressed Ed25519
    /// public key). Files that cannot be parsed are logged and skipped so that a
    /// single corrupt file does not prevent the client from starting.
    /// Returns an error only if the directory cannot be read at all.
    pub fn load_broker_verifying_keys(&self) -> Result<Vec<VerifyingKey>> {
        let dir = std::fs::read_dir(&self.broker_signing_pubkeys_dir).map_err(|e| {
            anyhow::anyhow!(
                "failed to read broker_signing_pubkeys_dir '{}': {}",
                self.broker_signing_pubkeys_dir,
                e
            )
        })?;

        let mut keys = Vec::new();
        for entry in dir {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("pub") {
                continue;
            }
            match std::fs::read(&path) {
                Ok(bytes) => match <[u8; 32]>::try_from(bytes.as_slice()) {
                    Ok(arr) => match VerifyingKey::from_bytes(&arr) {
                        Ok(key) => keys.push(key),
                        Err(e) => log::warn!(
                            "broker pubkey file '{}' is not a valid Ed25519 key: {}",
                            path.display(),
                            e
                        ),
                    },
                    Err(_) => log::warn!(
                        "broker pubkey file '{}' is not exactly 32 bytes, skipping",
                        path.display()
                    ),
                },
                Err(e) => log::warn!(
                    "failed to read broker pubkey file '{}': {}",
                    path.display(),
                    e
                ),
            }
        }

        if keys.is_empty() {
            return Err(anyhow::anyhow!(
                "no valid broker verifying keys found in '{}'",
                self.broker_signing_pubkeys_dir
            ));
        }

        Ok(keys)
    }

    /// Decode and return the client's X25519 static encryption private key.
    pub fn client_enc_privkey(&self) -> Result<StaticSecret> {
        let bytes = URL_SAFE_NO_PAD.decode(&self.client_enc_privkey_b64)?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("client enc privkey must be 32 bytes"))?;
        Ok(StaticSecret::from(arr))
    }
}

fn default_poll_interval() -> u64 {
    300 // 5 minutes
}

fn default_max_manifest_window_secs() -> u64 {
    90 * 24 * 3600 // 90 days
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use ed25519_dalek::SigningKey;
    use rand::{rngs::OsRng, RngCore};
    use std::io::Write;
    use tempfile::TempDir;

    fn gen_ed25519_pubkey_bytes() -> [u8; 32] {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        sk.verifying_key().to_bytes()
    }

    fn write_pub_file(dir: &TempDir, filename: &str, bytes: &[u8]) {
        let path = dir.path().join(filename);
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(bytes).unwrap();
    }

    // ── load_broker_verifying_keys ────────────────────────────────────────────

    #[test]
    fn test_load_verifying_keys_single_valid_file() {
        let dir = TempDir::new().unwrap();
        write_pub_file(&dir, "broker.pub", &gen_ed25519_pubkey_bytes());

        let cfg = ClientConfig {
            broker_signing_pubkeys_dir: dir.path().to_str().unwrap().to_string(),
            ..stub_cfg()
        };
        let keys = cfg.load_broker_verifying_keys().unwrap();
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn test_load_verifying_keys_multiple_pub_files() {
        let dir = TempDir::new().unwrap();
        write_pub_file(&dir, "key1.pub", &gen_ed25519_pubkey_bytes());
        write_pub_file(&dir, "key2.pub", &gen_ed25519_pubkey_bytes());

        let cfg = ClientConfig {
            broker_signing_pubkeys_dir: dir.path().to_str().unwrap().to_string(),
            ..stub_cfg()
        };
        let keys = cfg.load_broker_verifying_keys().unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_load_verifying_keys_ignores_non_pub_files() {
        let dir = TempDir::new().unwrap();
        write_pub_file(&dir, "broker.pub", &gen_ed25519_pubkey_bytes());
        write_pub_file(&dir, "README.txt", b"not a key");
        write_pub_file(&dir, "broker.key", &[0u8; 32]);

        let cfg = ClientConfig {
            broker_signing_pubkeys_dir: dir.path().to_str().unwrap().to_string(),
            ..stub_cfg()
        };
        let keys = cfg.load_broker_verifying_keys().unwrap();
        assert_eq!(keys.len(), 1, "only .pub files should be loaded");
    }

    #[test]
    fn test_load_verifying_keys_skips_wrong_size_pub_file() {
        let dir = TempDir::new().unwrap();
        write_pub_file(&dir, "good.pub", &gen_ed25519_pubkey_bytes());
        write_pub_file(&dir, "bad.pub", &[0u8; 16]); // wrong size

        let cfg = ClientConfig {
            broker_signing_pubkeys_dir: dir.path().to_str().unwrap().to_string(),
            ..stub_cfg()
        };
        // Should not error — bad file is skipped; good file is loaded
        let keys = cfg.load_broker_verifying_keys().unwrap();
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn test_load_verifying_keys_empty_dir_returns_error() {
        let dir = TempDir::new().unwrap();
        let cfg = ClientConfig {
            broker_signing_pubkeys_dir: dir.path().to_str().unwrap().to_string(),
            ..stub_cfg()
        };
        let result = cfg.load_broker_verifying_keys();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no valid broker verifying keys"));
    }

    #[test]
    fn test_load_verifying_keys_missing_dir_returns_error() {
        let cfg = ClientConfig {
            broker_signing_pubkeys_dir: "/nonexistent/dir/that/does/not/exist".to_string(),
            ..stub_cfg()
        };
        let result = cfg.load_broker_verifying_keys();
        assert!(result.is_err());
    }

    // ── client_enc_privkey ────────────────────────────────────────────────────

    #[test]
    fn test_client_enc_privkey_valid_base64() {
        let raw = [0x11u8; 32];
        let b64 = URL_SAFE_NO_PAD.encode(raw);
        let cfg = ClientConfig {
            client_enc_privkey_b64: b64,
            ..stub_cfg()
        };
        let key = cfg.client_enc_privkey().unwrap();
        assert_eq!(key.to_bytes(), raw);
    }

    #[test]
    fn test_client_enc_privkey_invalid_base64() {
        let cfg = ClientConfig {
            client_enc_privkey_b64: "!!!not-base64!!!".to_string(),
            ..stub_cfg()
        };
        assert!(cfg.client_enc_privkey().is_err());
    }

    #[test]
    fn test_client_enc_privkey_wrong_length() {
        // Valid base64 but only 16 bytes
        let b64 = URL_SAFE_NO_PAD.encode([0u8; 16]);
        let cfg = ClientConfig {
            client_enc_privkey_b64: b64,
            ..stub_cfg()
        };
        assert!(cfg.client_enc_privkey().is_err());
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn stub_cfg() -> ClientConfig {
        use std::collections::HashMap;
        ClientConfig {
            client_id: "test".to_string(),
            manifest_url: "https://x.example.com".to_string(),
            report_url: "https://x.example.com/report".to_string(),
            db_path: ":memory:".to_string(),
            db_encryption_key: "k".to_string(),
            poll_interval_secs: 60,
            max_manifest_window_secs: 90 * 24 * 3600,
            broker_signing_pubkeys_dir: "/tmp".to_string(),
            client_enc_privkey_b64: URL_SAFE_NO_PAD.encode([0u8; 32]),
            consumer_dirs: HashMap::new(),
            plugin_dir: None,
            tls: ClientTlsConfig {
                client_cert: "/dev/null".to_string(),
                client_key: "/dev/null".to_string(),
                ca_cert: "/dev/null".to_string(),
            },
        }
    }
}
