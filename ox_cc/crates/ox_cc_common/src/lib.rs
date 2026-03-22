/// ox_cc_common — shared types and cryptography for all ox_cc crates.
///
/// No I/O is performed here. All functions are pure or depend only on
/// system entropy (rand::rngs::OsRng). Key material is zeroized on drop
/// wherever possible.
pub mod envelope;
pub mod manifest;
pub mod encrypt;
pub mod verify;
pub mod error;

pub use envelope::EncryptedManifestEnvelope;
pub use manifest::Manifest;
pub use manifest::{CommandEntry, OnFailure};
pub use error::CryptoError;

#[cfg(test)]
mod tests;
