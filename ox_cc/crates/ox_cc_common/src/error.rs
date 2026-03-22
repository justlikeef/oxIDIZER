use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("signature verification failed")]
    SignatureInvalid,

    #[error("decryption failed")]
    DecryptionFailed,

    #[error("envelope version unsupported: {0}")]
    UnsupportedVersion(String),

    #[error("unsupported cipher: {0}")]
    UnsupportedCipher(String),

    #[error("envelope field missing: {0}")]
    MissingField(String),

    #[error("manifest expired")]
    ManifestExpired,

    #[error("manifest issued_at in the future")]
    ManifestNotYetValid,

    #[error("manifest validity window exceeds maximum ({max_secs}s)")]
    ValidityWindowTooLarge { max_secs: u64 },

    #[error("client_id mismatch: envelope says {envelope}, expected {expected}")]
    ClientIdMismatch { envelope: String, expected: String },

    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ed25519 error: {0}")]
    Ed25519(#[from] ed25519_dalek::SignatureError),
}
