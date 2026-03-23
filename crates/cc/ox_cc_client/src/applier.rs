/// Manifest applier: writes a single manifest.json atomically.
///
/// Combines the decrypted payload with report metadata into one file
/// written via a temp-file + rename. One atomic rename = no
/// mismatched-pair race condition.
use anyhow::Result;
use std::path::Path;

use ox_cc_common::manifest::{ApplierManifest, Manifest};

use crate::config::ClientConfig;

/// Write `manifest.json` atomically to `consumer_dir`.
///
/// The file combines:
///   - manifest_id (for the consuming agent to include in reports)
///   - report_url + mTLS cert paths (so the agent can POST directly)
///   - payload (forwarded verbatim)
pub async fn apply(consumer_dir: &str, cfg: &ClientConfig, manifest: &Manifest) -> Result<()> {
    let applier_manifest = ApplierManifest {
        manifest_id: manifest.manifest_id.clone(),
        report_url: cfg.report_url.clone(),
        client_cert: cfg.tls.client_cert.clone(),
        client_key: cfg.tls.client_key.clone(),
        ca_cert: cfg.tls.ca_cert.clone(),
        payload: manifest.payload.clone(),
    };

    let json = serde_json::to_string_pretty(&applier_manifest)?;

    let dir = Path::new(consumer_dir);
    let final_path = dir.join("manifest.json");
    let tmp_path = dir.join("manifest.json.tmp");

    // Write to temp file, then rename atomically
    tokio::fs::write(&tmp_path, &json).await?;
    tokio::fs::rename(&tmp_path, &final_path).await?;

    tracing::info!(
        path = %final_path.display(),
        manifest_id = %manifest.manifest_id,
        consumer = %manifest.consumer,
        "manifest.json written"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serde_json::json;
    use tempfile::TempDir;
    use std::collections::HashMap;

    use crate::config::{ClientConfig, ClientTlsConfig};

    fn stub_cfg(consumer_dir: &str) -> ClientConfig {
        let mut consumer_dirs = HashMap::new();
        consumer_dirs.insert("test_consumer".to_string(), consumer_dir.to_string());
        ClientConfig {
            client_id: "test-client".to_string(),
            manifest_url: "https://manifest.example.com".to_string(),
            report_url: "https://manifest.example.com/cc/report/test-client".to_string(),
            db_path: ":memory:".to_string(),
            db_encryption_key: "key".to_string(),
            poll_interval_secs: 60,
            max_manifest_window_secs: 90 * 24 * 3600,
            broker_signing_pubkeys_dir: "/tmp".to_string(),
            client_enc_privkey_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            consumer_dirs,
            plugin_dir: None,
            tls: ClientTlsConfig {
                client_cert: "/dev/null".to_string(),
                client_key: "/dev/null".to_string(),
                ca_cert: "/dev/null".to_string(),
            },
        }
    }

    fn make_manifest(id: &str, consumer: &str) -> Manifest {
        let now = Utc::now();
        Manifest {
            version: "1".to_string(),
            manifest_id: id.to_string(),
            client_id: "test-client".to_string(),
            consumer: consumer.to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            issued_at: now.to_rfc3339(),
            expires_at: (now + Duration::hours(24)).to_rfc3339(),
            payload: json!({ "package": "nginx", "version": "1.24" }),
        }
    }

    #[tokio::test]
    async fn test_apply_writes_manifest_json() {
        let dir = TempDir::new().expect("tempdir");
        let cfg = stub_cfg(dir.path().to_str().unwrap());
        let manifest = make_manifest("m1", "test_consumer");

        apply(dir.path().to_str().unwrap(), &cfg, &manifest).await.unwrap();

        let content = std::fs::read_to_string(dir.path().join("manifest.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(v["manifest_id"], "m1");
        assert_eq!(v["payload"]["package"], "nginx");
        assert_eq!(v["report_url"], cfg.report_url.as_str());
        assert_eq!(v["client_cert"], cfg.tls.client_cert.as_str());
    }

    #[tokio::test]
    async fn test_apply_is_atomic_no_partial_file() {
        // Write is temp → rename, so there should never be a .tmp file visible
        // after apply completes. We can only verify the final state here.
        let dir = TempDir::new().expect("tempdir");
        let cfg = stub_cfg(dir.path().to_str().unwrap());
        let manifest = make_manifest("m2", "test_consumer");

        apply(dir.path().to_str().unwrap(), &cfg, &manifest).await.unwrap();

        // Final file exists
        assert!(dir.path().join("manifest.json").exists());
        // Temp file is gone
        assert!(!dir.path().join("manifest.json.tmp").exists());
    }

    #[tokio::test]
    async fn test_apply_overwrites_previous() {
        let dir = TempDir::new().expect("tempdir");
        let cfg = stub_cfg(dir.path().to_str().unwrap());

        apply(dir.path().to_str().unwrap(), &cfg, &make_manifest("m1", "test_consumer")).await.unwrap();
        apply(dir.path().to_str().unwrap(), &cfg, &make_manifest("m2", "test_consumer")).await.unwrap();

        let content = std::fs::read_to_string(dir.path().join("manifest.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["manifest_id"], "m2", "second apply should overwrite first");
    }

    #[tokio::test]
    async fn test_apply_nonexistent_dir_fails() {
        let cfg = stub_cfg("/nonexistent/path/that/does/not/exist");
        let manifest = make_manifest("m1", "test_consumer");
        let result = apply("/nonexistent/path/that/does/not/exist", &cfg, &manifest).await;
        assert!(result.is_err());
    }
}
