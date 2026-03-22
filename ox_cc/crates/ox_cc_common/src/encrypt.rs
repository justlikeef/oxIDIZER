/// Broker-side encryption: Encrypt-then-Sign.
///
/// Wire format produced: `base64url(envelope_json).base64url(ed25519_signature)`
///
/// Processing order (must not be changed):
///   1. Serialize inner Manifest to canonical JSON
///   2. X25519 ECDH → shared secret
///   3. HKDF-SHA256 → symmetric key (bound to manifest_id + client_id + consumer)
///   4. AES-256-GCM or ChaCha20-Poly1305 encryption
///   5. Build EncryptedManifestEnvelope (no signature field)
///   6. Serialize envelope to JSON → base64url encode → b64_payload
///   7. Ed25519 sign over b64_payload bytes (raw ASCII, not decoded)
///   8. base64url encode signature → b64_sig
///   9. Return format!("{b64_payload}.{b64_sig}")
///  10. Zeroize symmetric key
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce as AesNonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chacha20poly1305::{ChaCha20Poly1305, Nonce as ChachaNonce};
use ed25519_dalek::{SigningKey, Signer};
use hkdf::Hkdf;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};
use zeroize::Zeroize;

use crate::envelope::EncryptedManifestEnvelope;
use crate::manifest::Manifest;
use crate::error::CryptoError;

/// Cipher selection. Configured per broker instance; sent in the envelope
/// so clients know which algorithm to use during decryption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cipher {
    /// AES-256-GCM. Preferred on platforms with hardware AES acceleration.
    Aes256Gcm,
    /// ChaCha20-Poly1305. Preferred on ARM / platforms without AES-NI.
    ChaCha20Poly1305,
}

impl Cipher {
    pub fn as_str(self) -> &'static str {
        match self {
            Cipher::Aes256Gcm => "aes256gcm",
            Cipher::ChaCha20Poly1305 => "chacha20poly1305",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "aes256gcm" => Some(Cipher::Aes256Gcm),
            "chacha20poly1305" => Some(Cipher::ChaCha20Poly1305),
            _ => None,
        }
    }
}

/// Encrypt a `Manifest` for `client_enc_pubkey` and sign it with
/// `broker_signing_key`.
///
/// Returns the wire string: `base64url(envelope_json).base64url(signature)`
pub fn encrypt_and_sign(
    manifest: &Manifest,
    client_enc_pubkey_bytes: &[u8; 32],
    broker_signing_key: &SigningKey,
    cipher: Cipher,
) -> Result<String, CryptoError> {
    // Step 1: canonical JSON of inner manifest
    let plaintext = serde_json::to_vec(manifest)?;

    // Step 2: ephemeral X25519 ECDH
    // Using an ephemeral secret so each signing produces a unique shared secret.
    // The ephemeral public key is included in the envelope so the client can
    // perform ECDH on its side.
    let broker_enc_ephemeral = EphemeralSecret::random_from_rng(OsRng);
    let broker_enc_pubkey = X25519PublicKey::from(&broker_enc_ephemeral);
    let client_pubkey = X25519PublicKey::from(*client_enc_pubkey_bytes);
    let mut shared = broker_enc_ephemeral.diffie_hellman(&client_pubkey);

    // Step 3: HKDF-SHA256
    // salt   = manifest_id UUID bytes (16 bytes, parsed from the UUID string)
    // info   = domain_sep || client_id || NUL || consumer
    let salt = uuid_to_bytes(&manifest.manifest_id);
    let info = hkdf_info(manifest.client_id.as_bytes(), manifest.consumer.as_bytes());
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared.as_bytes());
    let mut sym_key = [0u8; 32];
    hk.expand(&info, &mut sym_key)
        .expect("HKDF-SHA256 with 32-byte output is always valid");
    shared.zeroize(); // zero the shared secret immediately after HKDF

    // Step 4: generate nonce and encrypt
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);

    let ciphertext = match cipher {
        Cipher::Aes256Gcm => {
            let gcm = Aes256Gcm::new_from_slice(&sym_key)
                .expect("AES-256-GCM key is always 32 bytes");
            let nonce = AesNonce::from_slice(&nonce_bytes);
            gcm.encrypt(nonce, plaintext.as_ref())
                .map_err(|_| CryptoError::DecryptionFailed)?
        }
        Cipher::ChaCha20Poly1305 => {
            let chacha = ChaCha20Poly1305::new_from_slice(&sym_key)
                .expect("ChaCha20-Poly1305 key is always 32 bytes");
            let nonce = ChachaNonce::from_slice(&nonce_bytes);
            chacha
                .encrypt(nonce, plaintext.as_ref())
                .map_err(|_| CryptoError::DecryptionFailed)?
        }
    };
    sym_key.zeroize(); // Step 10

    // Step 5: build envelope (no signature field)
    let envelope = EncryptedManifestEnvelope {
        version: "1".to_string(),
        manifest_id: manifest.manifest_id.clone(),
        client_id: manifest.client_id.clone(),
        consumer: manifest.consumer.clone(),
        cipher: cipher.as_str().to_string(),
        broker_enc_pubkey: URL_SAFE_NO_PAD.encode(broker_enc_pubkey.as_bytes()),
        expires_at: manifest.expires_at.clone(),
        nonce: URL_SAFE_NO_PAD.encode(nonce_bytes),
        ciphertext: URL_SAFE_NO_PAD.encode(&ciphertext),
    };

    // Step 6: serialize envelope JSON → base64url
    let envelope_json = serde_json::to_vec(&envelope)?;
    let b64_payload = URL_SAFE_NO_PAD.encode(&envelope_json);

    // Step 7 + 8: Ed25519 sign over the raw b64_payload bytes, then encode sig
    // Signing over the ASCII b64 string (not the decoded JSON) means the verifier
    // can check the signature before decoding or parsing anything.
    let sig = broker_signing_key.sign(b64_payload.as_bytes());
    let b64_sig = URL_SAFE_NO_PAD.encode(sig.to_bytes());

    // Step 9: return wire string
    Ok(format!("{b64_payload}.{b64_sig}"))
}

// --- helpers (pub(crate) so verify.rs can reuse the same derivation) ---

/// Parse a UUID string to its 16 raw bytes, used as HKDF salt.
/// Falls back to zeroes if the UUID is malformed (should not happen in practice).
pub(crate) fn uuid_to_bytes(uuid_str: &str) -> [u8; 16] {
    let stripped: String = uuid_str.chars().filter(|c| *c != '-').collect();
    let mut out = [0u8; 16];
    if stripped.len() == 32 {
        for (i, chunk) in stripped.as_bytes().chunks(2).enumerate() {
            if let (Some(hi), Some(lo)) = (
                (chunk[0] as char).to_digit(16),
                (chunk[1] as char).to_digit(16),
            ) {
                out[i] = ((hi << 4) | lo) as u8;
            }
        }
    }
    out
}

/// Build the HKDF info field: domain separator + client_id + NUL + consumer.
pub(crate) fn hkdf_info(client_id: &[u8], consumer: &[u8]) -> Vec<u8> {
    let sep = b"ox_cc_encrypt_v1\x00";
    let mut info = Vec::with_capacity(sep.len() + client_id.len() + 1 + consumer.len());
    info.extend_from_slice(sep);
    info.extend_from_slice(client_id);
    info.push(0); // NUL separator prevents length-extension ambiguity
    info.extend_from_slice(consumer);
    info
}
