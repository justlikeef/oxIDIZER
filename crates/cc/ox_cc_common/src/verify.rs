/// Client-side decryption: Verify-then-Decrypt.
///
/// Wire format: `base64url(envelope_json).base64url(ed25519_signature)`
///
/// Processing order (must not be changed — verify before any JSON parsing):
///   1. Split wire string on `.` → (b64_payload, b64_sig)
///   2. base64-decode b64_sig → raw signature bytes
///   3. Iterate broker_verifying_keys; Ed25519 verify over b64_payload.as_bytes()
///      for each key. Accept on first match; fail with SignatureInvalid if none match.
///   4. base64-decode b64_payload → parse JSON → EncryptedManifestEnvelope
///   5. Check version == "1"
///   6. Check client_id matches expected_client_id
///   7. Check expires_at has not passed
///   8. Decode broker_enc_pubkey, nonce, ciphertext
///   9. X25519 ECDH (client privkey × broker enc pubkey)
///  10. HKDF-SHA256 → symmetric key
///  11. AEAD decrypt → inner manifest JSON; zeroize symmetric key
///  12. Deserialize inner Manifest
///  13. Cross-check manifest_id and client_id (envelope vs manifest)
///  14. Check manifest.issued_at ≤ now (±60s clock skew tolerance)
///  15. Check manifest.expires_at - manifest.issued_at ≤ max_manifest_window_secs
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce as AesNonce};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chacha20poly1305::{ChaCha20Poly1305, Nonce as ChachaNonce};
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signature, VerifyingKey};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::Zeroize;

use crate::encrypt::{self, Cipher};
use crate::error::CryptoError;
use crate::manifest::Manifest;

/// Verifies the wire-format envelope and decrypts the inner manifest.
///
/// # Parameters
/// - `wire` — the `.`-delimited wire string produced by `encrypt_and_sign`
/// - `expected_client_id` — the local client's own identifier; checked against
///   the envelope and manifest to prevent cross-client replay
/// - `broker_verifying_keys` — slice of trusted Ed25519 broker signing keys;
///   all are tried in order and the first match wins. Supports key rotation with
///   overlapping trust windows.
/// - `client_enc_privkey` — the client's X25519 static private key
/// - `max_manifest_window_secs` — maximum allowed `expires_at - issued_at`
pub fn verify_and_decrypt(
    wire: &str,
    expected_client_id: &str,
    broker_verifying_keys: &[VerifyingKey],
    client_enc_privkey: &StaticSecret,
    max_manifest_window_secs: u64,
) -> Result<Manifest, CryptoError> {
    // Step 1: split on `.` — exactly one dot expected
    let dot = wire.find('.').ok_or(CryptoError::SignatureInvalid)?;
    let b64_payload = &wire[..dot];
    let b64_sig = &wire[dot + 1..];

    // Reject malformed wires with a second dot (two-segment format only)
    if b64_sig.contains('.') {
        return Err(CryptoError::SignatureInvalid);
    }

    // Step 2: decode signature
    let sig_bytes = URL_SAFE_NO_PAD.decode(b64_sig)?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| CryptoError::SignatureInvalid)?;
    let signature = Signature::from_bytes(&sig_array);

    // Step 3: verify over the raw b64_payload bytes (before any JSON parsing)
    // Iterating all trusted keys allows key rotation without client downtime.
    if broker_verifying_keys.is_empty() {
        return Err(CryptoError::SignatureInvalid);
    }
    let verified = broker_verifying_keys
        .iter()
        .any(|key| key.verify_strict(b64_payload.as_bytes(), &signature).is_ok());
    if !verified {
        return Err(CryptoError::SignatureInvalid);
    }

    // Step 4: decode payload and parse envelope (only after signature is valid)
    let envelope_bytes = URL_SAFE_NO_PAD.decode(b64_payload)?;
    let envelope: crate::envelope::EncryptedManifestEnvelope =
        serde_json::from_slice(&envelope_bytes)?;

    // Step 5: version check
    if envelope.version != "1" {
        return Err(CryptoError::UnsupportedVersion(envelope.version.clone()));
    }

    // Step 6: client_id check (pre-decryption, prevents processing another
    // client's envelope which would waste resources and leak timing)
    if envelope.client_id != expected_client_id {
        return Err(CryptoError::ClientIdMismatch {
            envelope: envelope.client_id.clone(),
            expected: expected_client_id.to_string(),
        });
    }

    // Step 7: outer expires_at check
    let outer_expires = parse_timestamp(&envelope.expires_at)?;
    if Utc::now() > outer_expires {
        return Err(CryptoError::ManifestExpired);
    }

    // Step 8: decode binary fields
    let broker_pubkey_bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(&envelope.broker_enc_pubkey)?
        .try_into()
        .map_err(|_| CryptoError::MissingField("broker_enc_pubkey length".to_string()))?;
    let nonce_bytes: [u8; 12] = URL_SAFE_NO_PAD
        .decode(&envelope.nonce)?
        .try_into()
        .map_err(|_| CryptoError::MissingField("nonce length".to_string()))?;
    let ciphertext = URL_SAFE_NO_PAD.decode(&envelope.ciphertext)?;

    // Step 9: X25519 ECDH
    let broker_pubkey = X25519PublicKey::from(broker_pubkey_bytes);
    let mut shared = client_enc_privkey.diffie_hellman(&broker_pubkey);

    // Step 10: HKDF-SHA256
    let salt = encrypt::uuid_to_bytes(&envelope.manifest_id);
    let info = encrypt::hkdf_info(
        envelope.client_id.as_bytes(),
        envelope.consumer.as_bytes(),
    );
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared.as_bytes());
    let mut sym_key = [0u8; 32];
    hk.expand(&info, &mut sym_key)
        .expect("HKDF-SHA256 with 32-byte output is always valid");
    shared.zeroize();

    // Step 11: AEAD decrypt
    let cipher = Cipher::from_str(&envelope.cipher)
        .ok_or_else(|| CryptoError::UnsupportedCipher(envelope.cipher.clone()))?;
    let plaintext = match cipher {
        Cipher::Aes256Gcm => {
            let gcm = Aes256Gcm::new_from_slice(&sym_key)
                .expect("key is always 32 bytes");
            let nonce = AesNonce::from_slice(&nonce_bytes);
            gcm.decrypt(nonce, ciphertext.as_ref())
                .map_err(|_| CryptoError::DecryptionFailed)?
        }
        Cipher::ChaCha20Poly1305 => {
            let chacha = ChaCha20Poly1305::new_from_slice(&sym_key)
                .expect("key is always 32 bytes");
            let nonce = ChachaNonce::from_slice(&nonce_bytes);
            chacha
                .decrypt(nonce, ciphertext.as_ref())
                .map_err(|_| CryptoError::DecryptionFailed)?
        }
    };
    sym_key.zeroize();

    // Step 12: deserialize inner manifest
    let manifest: Manifest = serde_json::from_slice(&plaintext)?;

    // Step 13: cross-check manifest_id and client_id (envelope vs manifest)
    if manifest.manifest_id != envelope.manifest_id {
        return Err(CryptoError::ClientIdMismatch {
            envelope: envelope.manifest_id.clone(),
            expected: manifest.manifest_id.clone(),
        });
    }
    if manifest.client_id != expected_client_id {
        return Err(CryptoError::ClientIdMismatch {
            envelope: manifest.client_id.clone(),
            expected: expected_client_id.to_string(),
        });
    }

    // Step 14: issued_at ≤ now (±60s clock skew tolerance)
    let issued_at = parse_timestamp(&manifest.issued_at)?;
    let now = Utc::now();
    if issued_at > now + Duration::seconds(60) {
        return Err(CryptoError::ManifestNotYetValid);
    }

    // Step 15: validity window check
    let inner_expires = parse_timestamp(&manifest.expires_at)?;
    let window_secs = (inner_expires - issued_at).num_seconds().max(0) as u64;
    if window_secs > max_manifest_window_secs {
        return Err(CryptoError::ValidityWindowTooLarge {
            max_secs: max_manifest_window_secs,
        });
    }

    Ok(manifest)
}

fn parse_timestamp(s: &str) -> Result<DateTime<Utc>, CryptoError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| CryptoError::MissingField(format!("invalid timestamp: {s}")))
}
