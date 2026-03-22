use serde::{Deserialize, Serialize};

/// The JSON payload embedded in the wire format.
///
/// Wire format: `base64url(envelope_json).base64url(ed25519_signature)`
///
/// The Ed25519 signature covers the raw base64url-encoded bytes of the
/// envelope JSON (i.e., the first segment of the `.`-delimited string),
/// not the decoded JSON. This means the client verifies the signature
/// before performing any JSON parsing, minimising the pre-verification
/// attack surface.
///
/// Binary fields (nonce, ciphertext, broker_enc_pubkey) are
/// base64url-encoded without padding inside the JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedManifestEnvelope {
    /// Protocol version. Currently "1".
    pub version: String,

    /// UUIDv4 assigned by the broker per client. Unique across all clients
    /// for a given template, used as HKDF salt.
    pub manifest_id: String,

    /// Hostname or identifier of the intended recipient client.
    pub client_id: String,

    /// Consumer tag (e.g. "arcnition"). Bound into the HKDF info field.
    pub consumer: String,

    /// Cipher used for the payload. One of: "aes256gcm", "chacha20poly1305".
    pub cipher: String,

    /// base64url-encoded X25519 ephemeral public key used by the broker
    /// during encryption. Allows future key rotation without client config
    /// changes — the client always re-derives the shared secret from this
    /// field rather than assuming a single broker key.
    pub broker_enc_pubkey: String,

    /// ISO 8601 UTC timestamp. Client rejects envelope after this time.
    pub expires_at: String,

    /// base64url-encoded 12-byte random nonce for the AEAD cipher.
    pub nonce: String,

    /// base64url-encoded ciphertext with AEAD tag appended.
    /// Covers the canonical JSON of the inner `Manifest`.
    pub ciphertext: String,
}
