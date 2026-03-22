use chrono::{Duration, Utc};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

use crate::encrypt::{encrypt_and_sign, Cipher};
use crate::error::CryptoError;
use crate::manifest::Manifest;
use crate::verify::verify_and_decrypt;

// ── Test fixtures ────────────────────────────────────────────────────────────

fn make_signing_keypair() -> (SigningKey, VerifyingKey) {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();
    (signing_key, verifying_key)
}

fn make_enc_keypair() -> (StaticSecret, X25519PublicKey) {
    let privkey = StaticSecret::random_from_rng(OsRng);
    let pubkey = X25519PublicKey::from(&privkey);
    (privkey, pubkey)
}

fn make_manifest(client_id: &str) -> Manifest {
    let now = Utc::now();
    Manifest {
        version: "1".to_string(),
        manifest_id: uuid::Uuid::new_v4().to_string(),
        client_id: client_id.to_string(),
        consumer: "test_consumer".to_string(),
        name: "Test manifest".to_string(),
        description: "Integration test manifest".to_string(),
        issued_at: now.to_rfc3339(),
        expires_at: (now + Duration::hours(24)).to_rfc3339(),
        payload: json!({ "key": "value" }),
    }
}

const MAX_WINDOW: u64 = 90 * 24 * 3600;

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn test_encrypt_decrypt_roundtrip() {
    let (signing_key, verifying_key) = make_signing_keypair();
    let (client_privkey, client_pubkey) = make_enc_keypair();
    let manifest = make_manifest("client-a.example.com");

    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    let decrypted = verify_and_decrypt(
        &wire,
        "client-a.example.com",
        &[verifying_key],
        &client_privkey,
        MAX_WINDOW,
    )
    .expect("verify_and_decrypt should succeed");

    assert_eq!(decrypted.manifest_id, manifest.manifest_id);
    assert_eq!(decrypted.payload, manifest.payload);
    assert_eq!(decrypted.consumer, "test_consumer");
}

#[test]
fn test_encrypt_decrypt_roundtrip_chacha() {
    let (signing_key, verifying_key) = make_signing_keypair();
    let (client_privkey, client_pubkey) = make_enc_keypair();
    let manifest = make_manifest("client-b.example.com");

    let wire = encrypt_and_sign(
        &manifest,
        client_pubkey.as_bytes(),
        &signing_key,
        Cipher::ChaCha20Poly1305,
    )
    .expect("chacha encrypt_and_sign should succeed");

    let decrypted = verify_and_decrypt(
        &wire,
        "client-b.example.com",
        &[verifying_key],
        &client_privkey,
        MAX_WINDOW,
    )
    .expect("chacha verify_and_decrypt should succeed");

    assert_eq!(decrypted.manifest_id, manifest.manifest_id);
}

#[test]
fn test_signature_verification_fails_on_tamper() {
    let (signing_key, verifying_key) = make_signing_keypair();
    let (client_privkey, client_pubkey) = make_enc_keypair();
    let manifest = make_manifest("client-a.example.com");

    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    // Tamper: flip a byte in the payload segment (before the dot)
    let dot = wire.find('.').unwrap();
    let mut tampered = wire.into_bytes();
    tampered[dot / 2] ^= 0x01;
    let tampered = String::from_utf8(tampered).unwrap();

    let result = verify_and_decrypt(
        &tampered,
        "client-a.example.com",
        &[verifying_key],
        &client_privkey,
        MAX_WINDOW,
    );

    assert!(matches!(result, Err(CryptoError::SignatureInvalid)));
}

#[test]
fn test_wrong_key_rejected() {
    let (signing_key, _) = make_signing_keypair();
    let (wrong_signing_key, wrong_verifying_key) = make_signing_keypair();
    let _ = wrong_signing_key; // suppress warning
    let (client_privkey, client_pubkey) = make_enc_keypair();
    let manifest = make_manifest("client-a.example.com");

    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    let result = verify_and_decrypt(
        &wire,
        "client-a.example.com",
        &[wrong_verifying_key], // wrong key
        &client_privkey,
        MAX_WINDOW,
    );

    assert!(matches!(result, Err(CryptoError::SignatureInvalid)));
}

#[test]
fn test_multi_key_rotation_accepted() {
    let (old_signing_key, old_verifying_key) = make_signing_keypair();
    let (_, new_verifying_key) = make_signing_keypair();
    let (client_privkey, client_pubkey) = make_enc_keypair();
    let manifest = make_manifest("client-a.example.com");

    // Envelope signed with old key; client trusts both old and new (rotation window)
    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &old_signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    let result = verify_and_decrypt(
        &wire,
        "client-a.example.com",
        &[new_verifying_key, old_verifying_key], // old key is in the list
        &client_privkey,
        MAX_WINDOW,
    );

    assert!(result.is_ok(), "should accept with old key still in trusted list");
}

#[test]
fn test_client_id_mismatch_rejected() {
    let (signing_key, verifying_key) = make_signing_keypair();
    let (client_privkey, client_pubkey) = make_enc_keypair();
    let manifest = make_manifest("client-a.example.com");

    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    // Try to decrypt as a different client
    let result = verify_and_decrypt(
        &wire,
        "client-b.example.com", // wrong client
        &[verifying_key],
        &client_privkey,
        MAX_WINDOW,
    );

    assert!(matches!(result, Err(CryptoError::ClientIdMismatch { .. })));
}

#[test]
fn test_expired_envelope_rejected() {
    let (signing_key, verifying_key) = make_signing_keypair();
    let (client_privkey, client_pubkey) = make_enc_keypair();

    // Create a manifest that expired in the past
    let past = Utc::now() - Duration::hours(2);
    let manifest = Manifest {
        version: "1".to_string(),
        manifest_id: uuid::Uuid::new_v4().to_string(),
        client_id: "client-a.example.com".to_string(),
        consumer: "test_consumer".to_string(),
        name: "Expired manifest".to_string(),
        description: "This manifest has expired".to_string(),
        issued_at: (past - Duration::hours(1)).to_rfc3339(),
        expires_at: past.to_rfc3339(), // expired
        payload: json!({}),
    };

    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    let result = verify_and_decrypt(
        &wire,
        "client-a.example.com",
        &[verifying_key],
        &client_privkey,
        MAX_WINDOW,
    );

    assert!(matches!(result, Err(CryptoError::ManifestExpired)));
}

#[test]
fn test_validity_window_too_large_rejected() {
    let (signing_key, verifying_key) = make_signing_keypair();
    let (client_privkey, client_pubkey) = make_enc_keypair();

    let now = Utc::now();
    // Window of 200 days — larger than 90-day max
    let manifest = Manifest {
        version: "1".to_string(),
        manifest_id: uuid::Uuid::new_v4().to_string(),
        client_id: "client-a.example.com".to_string(),
        consumer: "test_consumer".to_string(),
        name: "Wide-window manifest".to_string(),
        description: "Window too large".to_string(),
        issued_at: now.to_rfc3339(),
        expires_at: (now + Duration::days(200)).to_rfc3339(),
        payload: json!({}),
    };

    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    let result = verify_and_decrypt(
        &wire,
        "client-a.example.com",
        &[verifying_key],
        &client_privkey,
        MAX_WINDOW, // 90 days
    );

    assert!(matches!(result, Err(CryptoError::ValidityWindowTooLarge { .. })));
}

#[test]
fn test_wire_format_is_two_segments() {
    let (signing_key, _) = make_signing_keypair();
    let (_, client_pubkey) = make_enc_keypair();
    let manifest = make_manifest("client-a.example.com");

    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    let segments: Vec<&str> = wire.splitn(3, '.').collect();
    assert_eq!(segments.len(), 2, "wire format must be exactly two dot-separated segments");
    assert!(!segments[0].is_empty(), "payload segment must not be empty");
    assert!(!segments[1].is_empty(), "signature segment must not be empty");
}

#[test]
fn test_no_trusted_keys_rejected() {
    let (signing_key, _) = make_signing_keypair();
    let (client_privkey, client_pubkey) = make_enc_keypair();
    let manifest = make_manifest("client-a.example.com");

    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    let result = verify_and_decrypt(
        &wire,
        "client-a.example.com",
        &[], // empty key list
        &client_privkey,
        MAX_WINDOW,
    );

    assert!(matches!(result, Err(CryptoError::SignatureInvalid)));
}

#[test]
fn test_wrong_client_privkey_fails_decryption() {
    let (signing_key, verifying_key) = make_signing_keypair();
    let (_, client_pubkey) = make_enc_keypair();
    let (wrong_privkey, _) = make_enc_keypair(); // different key pair
    let manifest = make_manifest("client-a.example.com");

    let wire = encrypt_and_sign(&manifest, client_pubkey.as_bytes(), &signing_key, Cipher::Aes256Gcm)
        .expect("encrypt_and_sign should succeed");

    // Signature verification passes (the wire is valid), but ECDH produces wrong key
    let result = verify_and_decrypt(
        &wire,
        "client-a.example.com",
        &[verifying_key],
        &wrong_privkey, // wrong encryption private key
        MAX_WINDOW,
    );

    assert!(matches!(result, Err(CryptoError::DecryptionFailed)));
}
