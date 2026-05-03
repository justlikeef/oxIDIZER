use rand::rngs::OsRng;
use rand::RngCore;
use ed25519_dalek::SigningKey;
use x25519_dalek::StaticSecret;

/// Generate a new Ed25519 signing keypair.
pub fn generate_signing_key() -> SigningKey {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    SigningKey::from_bytes(&seed)
}

/// Generate a new X25519 encryption keypair.
pub fn generate_encryption_key() -> StaticSecret {
    StaticSecret::random_from_rng(OsRng)
}
