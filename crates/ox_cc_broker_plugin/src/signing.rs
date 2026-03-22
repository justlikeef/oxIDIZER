/// Signing orchestration: load keys, sign per-client envelopes on approval.
///
/// Signing occurs only after an approver explicitly approves a template.
/// Each client's envelope is signed independently; failures are logged and
/// tracked in failed_client_ids without blocking successful clients.
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use ed25519_dalek::SigningKey;
use rusqlite::{params, Connection};
use x25519_dalek::StaticSecret;

use crate::config::BrokerPluginConfig;
use crate::encrypt::broker_encrypt as encrypt_for_client;

/// Load the Ed25519 signing key from the configured path.
/// The key file is expected to contain 32 raw bytes (the seed/private scalar).
pub fn load_signing_key(path: &str) -> Result<SigningKey, anyhow::Error> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("read signing key {}: {}", path, e))?;
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("signing key must be exactly 32 bytes"))?;
    Ok(SigningKey::from_bytes(&array))
}

/// Load the X25519 encryption key from the configured path.
pub fn load_enc_key(path: &str) -> Result<StaticSecret, anyhow::Error> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("read enc key {}: {}", path, e))?;
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("enc key must be exactly 32 bytes"))?;
    Ok(StaticSecret::from(array))
}

/// Sign all pending signing_requests for `template_id`.
///
/// Returns (signed_count, failed_client_ids).
/// Signing failures for individual clients do not abort the batch —
/// all successful clients are signed and their envelopes stored.
pub fn sign_batch(
    conn: &Connection,
    config: &BrokerPluginConfig,
    template_id: &str,
    payload_json: &str,
    consumer: &str,
    name: &str,
    description: &str,
    expires_in_secs: i64,
) -> Result<(usize, Vec<String>), anyhow::Error> {
    let signing_key = load_signing_key(&config.signing_key_path)?;
    let enc_key = load_enc_key(&config.enc_key_path)?;

    // Fetch all pending signing_requests for this template
    let mut stmt = conn.prepare(
        "SELECT request_id, client_id FROM signing_requests
         WHERE template_id = ?1 AND status = 'pending'",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![template_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut signed_count = 0usize;
    let mut failed_client_ids: Vec<String> = Vec::new();

    let issued_at = Utc::now();
    let expires_at = issued_at + chrono::Duration::seconds(expires_in_secs);

    for (request_id, client_id) in rows {
        // Look up client's X25519 pubkey from the clients table
        let pubkey_b64: String = match conn.query_row(
            "SELECT enc_pubkey_b64 FROM clients WHERE client_id = ?1",
            params![client_id],
            |row| row.get(0),
        ) {
            Ok(k) => k,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                log::warn!("client_id '{}' not in clients table, skipping", client_id);
                failed_client_ids.push(client_id.clone());
                conn.execute(
                    "UPDATE signing_requests SET status = 'failed' WHERE request_id = ?1",
                    params![request_id],
                )?;
                continue;
            }
            Err(e) => {
                log::warn!("db error looking up pubkey for '{}': {}", client_id, e);
                failed_client_ids.push(client_id.clone());
                conn.execute(
                    "UPDATE signing_requests SET status = 'failed' WHERE request_id = ?1",
                    params![request_id],
                )?;
                continue;
            }
        };

        let pubkey_bytes = match URL_SAFE_NO_PAD.decode(&pubkey_b64) {
            Ok(b) if b.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&b);
                arr
            }
            _ => {
                log::warn!("invalid pubkey for client '{}', skipping", client_id);
                failed_client_ids.push(client_id.clone());
                conn.execute(
                    "UPDATE signing_requests SET status = 'failed' WHERE request_id = ?1",
                    params![request_id],
                )?;
                continue;
            }
        };

        match encrypt_for_client(
            config,
            &signing_key,
            &enc_key,
            &client_id,
            consumer,
            name,
            description,
            &issued_at.to_rfc3339(),
            &expires_at.to_rfc3339(),
            payload_json,
            &pubkey_bytes,
            &request_id,
        ) {
            Ok(envelope_json) => {
                conn.execute(
                    "UPDATE signing_requests
                     SET status = 'approved', envelope_json = ?1
                     WHERE request_id = ?2",
                    params![envelope_json, request_id],
                )?;
                signed_count += 1;
            }
            Err(e) => {
                log::error!(
                    "signing failed for client '{}' (request {}): {}",
                    client_id, request_id, e
                );
                failed_client_ids.push(client_id.clone());
                conn.execute(
                    "UPDATE signing_requests SET status = 'failed' WHERE request_id = ?1",
                    params![request_id],
                )?;
            }
        }
    }

    Ok((signed_count, failed_client_ids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_bytes(bytes: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_load_signing_key_ok() {
        let seed = [0x42u8; 32];
        let f = write_bytes(&seed);
        let key = load_signing_key(f.path().to_str().unwrap()).unwrap();
        // Round-trip: from_bytes then to_bytes should match the seed
        assert_eq!(key.to_bytes(), seed);
    }

    #[test]
    fn test_load_signing_key_wrong_size() {
        let f = write_bytes(&[0u8; 16]); // only 16 bytes
        let result = load_signing_key(f.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("32 bytes"));
    }

    #[test]
    fn test_load_signing_key_empty_file() {
        let f = write_bytes(&[]);
        let result = load_signing_key(f.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_signing_key_missing_file() {
        let result = load_signing_key("/nonexistent/path/broker.key");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_enc_key_ok() {
        let raw = [0xABu8; 32];
        let f = write_bytes(&raw);
        let key = load_enc_key(f.path().to_str().unwrap()).unwrap();
        assert_eq!(key.to_bytes(), raw);
    }

    #[test]
    fn test_load_enc_key_wrong_size() {
        let f = write_bytes(&[0u8; 64]); // 64 bytes, not 32
        let result = load_enc_key(f.path().to_str().unwrap());
        assert!(result.is_err());
        let e = result.err().unwrap();
        assert!(e.to_string().contains("32 bytes"), "got: {}", e);
    }

    #[test]
    fn test_load_enc_key_missing_file() {
        let result = load_enc_key("/nonexistent/broker.enc");
        assert!(result.is_err());
    }
}
