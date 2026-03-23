/// Per-client envelope construction: builds the inner Manifest, calls
/// ox_cc_common::encrypt::encrypt_and_sign, returns the wire string.
use ed25519_dalek::SigningKey;
use uuid::Uuid;
use x25519_dalek::StaticSecret;

use ox_cc_common::encrypt::{encrypt_and_sign, Cipher};
use ox_cc_common::manifest::Manifest;

use crate::config::BrokerPluginConfig;

/// Encrypt and sign a manifest for a single client.
///
/// Returns the wire string: `base64url(envelope_json).base64url(signature)`
pub fn broker_encrypt(
    config: &BrokerPluginConfig,
    signing_key: &SigningKey,
    _enc_key: &StaticSecret, // reserved; actual ECDH uses an ephemeral key per encrypt_and_sign
    client_id: &str,
    consumer: &str,
    name: &str,
    description: &str,
    issued_at: &str,
    expires_at: &str,
    payload_json: &str,
    client_enc_pubkey: &[u8; 32],
    _request_id: &str,
) -> Result<String, anyhow::Error> {
    let payload: serde_json::Value = serde_json::from_str(payload_json)
        .map_err(|e| anyhow::anyhow!("invalid payload JSON: {}", e))?;

    let manifest = Manifest {
        version: "1".to_string(),
        manifest_id: Uuid::new_v4().to_string(),
        client_id: client_id.to_string(),
        consumer: consumer.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        issued_at: issued_at.to_string(),
        expires_at: expires_at.to_string(),
        payload,
    };

    let cipher = Cipher::from_str(&config.cipher)
        .ok_or_else(|| anyhow::anyhow!("unsupported cipher: {}", config.cipher))?;

    encrypt_and_sign(&manifest, client_enc_pubkey, signing_key, cipher)
        .map_err(|e| anyhow::anyhow!("encrypt_and_sign: {}", e))
}
