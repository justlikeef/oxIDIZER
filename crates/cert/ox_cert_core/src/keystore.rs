use crate::model::{KeyInfo, KeyStoreConfig, KeyStoreType, KeyType, SigningAlgorithm};
use crate::CertError;
use std::io::Write as IoWrite;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// KeyStore trait
// ---------------------------------------------------------------------------

/// All CA signing operations go through this trait.
/// The Software implementation stores PKCS#8 PEM under `{key_dir}/{tenant_id}/{key_id}.key.pem`.
/// The PKCS#11 implementation (HSM) is a stub — add the `cryptoki` dependency to enable it.
pub trait KeyStore: Send + Sync {
    fn sign(
        &self,
        tenant_id: &str,
        key_id: &str,
        algorithm: SigningAlgorithm,
        data: &[u8],
    ) -> Result<Vec<u8>, CertError>;

    fn public_key(&self, tenant_id: &str, key_id: &str) -> Result<Vec<u8>, CertError>;

    fn generate_key(
        &self,
        tenant_id: &str,
        key_id: &str,
        key_type: KeyType,
        overwrite: bool,
    ) -> Result<(), CertError>;

    fn key_exists(&self, tenant_id: &str, key_id: &str) -> Result<bool, CertError>;
    fn key_info(&self, tenant_id: &str, key_id: &str) -> Result<KeyInfo, CertError>;
    fn delete_key(&self, tenant_id: &str, key_id: &str) -> Result<(), CertError>;

    /// Export the raw PKCS#8 PEM for the key. Returns `CertError::Crypto` for HSM keystores
    /// where key material cannot be exported.
    fn load_key_pem(&self, tenant_id: &str, key_id: &str) -> Result<String, CertError>;
}

/// Open a `KeyStore` from config.
pub fn open_keystore(config: &KeyStoreConfig) -> Result<Box<dyn KeyStore>, CertError> {
    match config.store_type {
        KeyStoreType::Software => Ok(Box::new(SoftwareKeyStore::open(config)?)),
        KeyStoreType::Pkcs11 => Err(CertError::Crypto(
            "PKCS#11 keystore not compiled in; add the cryptoki feature".to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// SoftwareKeyStore
// ---------------------------------------------------------------------------

pub struct SoftwareKeyStore {
    key_dir: PathBuf,
    passphrase: Option<String>,
}

impl SoftwareKeyStore {
    pub fn open(config: &KeyStoreConfig) -> Result<Self, CertError> {
        let key_dir = config
            .key_dir
            .clone()
            .ok_or_else(|| CertError::Crypto("key_dir not configured for software keystore".to_string()))?;
        let passphrase =
            config.passphrase_env.as_ref().and_then(|env| std::env::var(env).ok());
        Ok(Self { key_dir, passphrase })
    }

    fn key_path(&self, tenant_id: &str, key_id: &str) -> PathBuf {
        self.key_dir.join(tenant_id).join(format!("{}.key.pem", key_id))
    }

    fn ensure_tenant_dir(&self, tenant_id: &str) -> Result<(), CertError> {
        let dir = self.key_dir.join(tenant_id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| CertError::Crypto(format!("failed to create key dir: {}", e)))
    }

    fn load_key_der(&self, tenant_id: &str, key_id: &str) -> Result<(Vec<u8>, KeyType), CertError> {
        let path = self.key_path(tenant_id, key_id);
        let pem_str = std::fs::read_to_string(&path).map_err(|e| {
            CertError::Crypto(format!("failed to read key {}: {}", path.display(), e))
        })?;

        let pem_item = pem::parse(pem_str.as_bytes())
            .map_err(|e| CertError::Crypto(format!("invalid PEM: {}", e)))?;

        let raw_der = if pem_item.tag() == "ENCRYPTED PRIVATE KEY" {
            // Custom AES-256-GCM format written by this keystore's generate_key.
            // Keys generated externally (e.g. by generate-root-ca.sh) are stored
            // unencrypted ("PRIVATE KEY") and handled by the else branch below.
            let pass = self.passphrase.as_deref().ok_or_else(|| {
                CertError::Crypto(
                    "key is encrypted but passphrase_env is not set or empty".to_string(),
                )
            })?;
            decrypt_key(pem_item.contents(), pass, tenant_id)?
        } else {
            // Unencrypted PKCS#8 DER ("PRIVATE KEY" PEM header).
            pem_item.contents().to_vec()
        };

        let key_type = detect_key_type(&raw_der)?;
        Ok((raw_der, key_type))
    }

    fn write_key_pem(
        &self,
        tenant_id: &str,
        key_id: &str,
        der: &[u8],
    ) -> Result<(), CertError> {
        let pem_header = if self.passphrase.is_some() {
            "ENCRYPTED PRIVATE KEY"
        } else {
            "PRIVATE KEY"
        };

        let contents = if let Some(pass) = &self.passphrase {
            encrypt_key(der, pass, tenant_id)?
        } else {
            der.to_vec()
        };

        let pem_obj = pem::Pem::new(pem_header, contents);
        let pem_str = pem::encode(&pem_obj);

        let path = self.key_path(tenant_id, key_id);
        let mut file = std::fs::File::create(&path)
            .map_err(|e| CertError::Crypto(format!("failed to create key file: {}", e)))?;
        file.write_all(pem_str.as_bytes())
            .map_err(|e| CertError::Crypto(format!("failed to write key: {}", e)))?;
        Ok(())
    }
}

impl KeyStore for SoftwareKeyStore {
    fn generate_key(
        &self,
        tenant_id: &str,
        key_id: &str,
        key_type: KeyType,
        overwrite: bool,
    ) -> Result<(), CertError> {
        self.ensure_tenant_dir(tenant_id)?;
        let path = self.key_path(tenant_id, key_id);
        if path.exists() && !overwrite {
            return Ok(());
        }

        let alg = match key_type {
            KeyType::EcP256 => &rcgen::PKCS_ECDSA_P256_SHA256,
            KeyType::EcP384 => &rcgen::PKCS_ECDSA_P384_SHA384,
            KeyType::EcP521 => {
                return Err(CertError::Crypto(
                    "P-521 key generation requires the aws_lc_rs feature in rcgen".to_string(),
                ));
            }
            KeyType::Ed25519 => &rcgen::PKCS_ED25519,
            KeyType::Rsa2048 | KeyType::Rsa3072 | KeyType::Rsa4096 => {
                return Err(CertError::Crypto(
                    "RSA key generation requires the aws-lc-rs feature; use EcP256/EcP384/Ed25519 or supply a pre-generated RSA PKCS#8 key".to_string(),
                ));
            }
        };

        let key_pair = rcgen::KeyPair::generate_for(alg)
            .map_err(|e| CertError::Crypto(format!("key gen failed: {}", e)))?;
        let der = key_pair.serialize_der();
        self.write_key_pem(tenant_id, key_id, &der)
    }

    fn sign(
        &self,
        tenant_id: &str,
        key_id: &str,
        algorithm: SigningAlgorithm,
        data: &[u8],
    ) -> Result<Vec<u8>, CertError> {
        use ring::signature::{self};

        let (der, _key_type) = self.load_key_der(tenant_id, key_id)?;
        let rng = ring::rand::SystemRandom::new();

        let sig = match algorithm {
            SigningAlgorithm::EcdsaWithSha256 => {
                let kp = signature::EcdsaKeyPair::from_pkcs8(
                    &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                    &der,
                    &rng,
                )
                .map_err(|e| CertError::Crypto(format!("ECDSA P-256 load failed: {}", e)))?;
                kp.sign(&rng, data)
                    .map_err(|e| CertError::Crypto(format!("ECDSA sign failed: {}", e)))?
                    .as_ref()
                    .to_vec()
            }
            SigningAlgorithm::EcdsaWithSha384 => {
                let kp = signature::EcdsaKeyPair::from_pkcs8(
                    &signature::ECDSA_P384_SHA384_FIXED_SIGNING,
                    &der,
                    &rng,
                )
                .map_err(|e| CertError::Crypto(format!("ECDSA P-384 load failed: {}", e)))?;
                kp.sign(&rng, data)
                    .map_err(|e| CertError::Crypto(format!("ECDSA sign failed: {}", e)))?
                    .as_ref()
                    .to_vec()
            }
            SigningAlgorithm::EcdsaWithSha512 => {
                // ring doesn't support P-521; fall through to error
                return Err(CertError::Crypto(
                    "P-521 signing not supported in ring; use aws-lc-rs backend".to_string(),
                ));
            }
            SigningAlgorithm::Ed25519 => {
                let kp = signature::Ed25519KeyPair::from_pkcs8(&der)
                    .map_err(|e| CertError::Crypto(format!("Ed25519 load failed: {}", e)))?;
                kp.sign(data).as_ref().to_vec()
            }
            SigningAlgorithm::Sha256WithRsa
            | SigningAlgorithm::Sha384WithRsa
            | SigningAlgorithm::Sha512WithRsa => {
                use ring::signature::RsaEncoding;
                let kp = signature::RsaKeyPair::from_pkcs8(&der)
                    .map_err(|e| CertError::Crypto(format!("RSA load failed: {}", e)))?;
                let padding: &dyn RsaEncoding = match algorithm {
                    SigningAlgorithm::Sha256WithRsa => &signature::RSA_PKCS1_SHA256,
                    SigningAlgorithm::Sha384WithRsa => &signature::RSA_PKCS1_SHA384,
                    _ => &signature::RSA_PKCS1_SHA512,
                };
                let mut sig = vec![0u8; kp.public().modulus_len()];
                kp.sign(padding, &rng, data, &mut sig)
                    .map_err(|e| CertError::Crypto(format!("RSA sign failed: {}", e)))?;
                sig
            }
        };
        Ok(sig)
    }

    fn public_key(&self, tenant_id: &str, key_id: &str) -> Result<Vec<u8>, CertError> {
        use ring::signature::KeyPair as _;

        let (der, key_type) = self.load_key_der(tenant_id, key_id)?;
        let rng = ring::rand::SystemRandom::new();

        let spki = match key_type {
            KeyType::EcP256 => {
                let kp = ring::signature::EcdsaKeyPair::from_pkcs8(
                    &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                    &der,
                    &rng,
                )
                .map_err(|e| CertError::Crypto(e.to_string()))?;
                kp.public_key().as_ref().to_vec()
            }
            KeyType::EcP384 => {
                let kp = ring::signature::EcdsaKeyPair::from_pkcs8(
                    &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING,
                    &der,
                    &rng,
                )
                .map_err(|e| CertError::Crypto(e.to_string()))?;
                kp.public_key().as_ref().to_vec()
            }
            KeyType::Ed25519 => {
                let kp = ring::signature::Ed25519KeyPair::from_pkcs8(&der)
                    .map_err(|e| CertError::Crypto(e.to_string()))?;
                kp.public_key().as_ref().to_vec()
            }
            KeyType::Rsa2048 | KeyType::Rsa3072 | KeyType::Rsa4096 => {
                let kp = ring::signature::RsaKeyPair::from_pkcs8(&der)
                    .map_err(|e| CertError::Crypto(e.to_string()))?;
                kp.public_key().as_ref().to_vec()
            }
            KeyType::EcP521 => {
                return Err(CertError::Crypto(
                    "P-521 public key extraction not supported in ring".to_string(),
                ));
            }
        };
        Ok(spki)
    }

    fn key_exists(&self, tenant_id: &str, key_id: &str) -> Result<bool, CertError> {
        Ok(self.key_path(tenant_id, key_id).exists())
    }

    fn key_info(&self, tenant_id: &str, key_id: &str) -> Result<KeyInfo, CertError> {
        let path = self.key_path(tenant_id, key_id);
        if !path.exists() {
            return Err(CertError::NotFound(format!("key {}/{}", tenant_id, key_id)));
        }
        let (der, key_type) = self.load_key_der(tenant_id, key_id)?;
        let _ = der;
        let meta = std::fs::metadata(&path)
            .map_err(|e| CertError::Crypto(format!("stat failed: {}", e)))?;
        let created = meta
            .created()
            .map(|t| {
                use std::time::UNIX_EPOCH;
                let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
                time::OffsetDateTime::from_unix_timestamp(d.as_secs() as i64)
                    .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
            })
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
        Ok(KeyInfo {
            key_id: key_id.to_string(),
            tenant_id: tenant_id.to_string(),
            key_type,
            created_at: created,
        })
    }

    fn delete_key(&self, tenant_id: &str, key_id: &str) -> Result<(), CertError> {
        let path = self.key_path(tenant_id, key_id);
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| CertError::Crypto(format!("delete failed: {}", e)))?;
        }
        Ok(())
    }

    fn load_key_pem(&self, tenant_id: &str, key_id: &str) -> Result<String, CertError> {
        let (der, _) = self.load_key_der(tenant_id, key_id)?;
        let pem_obj = pem::Pem::new("PRIVATE KEY", der);
        Ok(pem::encode(&pem_obj))
    }
}

// ---------------------------------------------------------------------------
// Key-type detection from PKCS#8 DER (OID sniffing)
// ---------------------------------------------------------------------------

fn detect_key_type(der: &[u8]) -> Result<KeyType, CertError> {
    // PKCS#8 PrivateKeyInfo: SEQUENCE { AlgorithmIdentifier { OID, ... }, BIT STRING }
    // We look for the algorithm OID in the first ~30 bytes.
    // EC keys: id-ecPublicKey = 1.2.840.10045.2.1 (06 07 2a 86 48 ce 3d 02 01)
    // RSA:     rsaEncryption  = 1.2.840.113549.1.1.1 (06 09 2a 86 48 86 f7 0d 01 01 01)
    // Ed25519: id-EdDSA       = 1.3.101.112 (06 03 2b 65 70)
    const EC_OID: &[u8] = &[0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
    const RSA_OID: &[u8] = &[0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
    const ED25519_OID: &[u8] = &[0x06, 0x03, 0x2b, 0x65, 0x70];

    if contains_bytes(der, ED25519_OID) {
        return Ok(KeyType::Ed25519);
    }
    if contains_bytes(der, RSA_OID) {
        // Distinguish RSA sizes by modulus length — parse public key to get modulus bits
        let kp = ring::signature::RsaKeyPair::from_pkcs8(der)
            .map_err(|e| CertError::Crypto(e.to_string()))?;
        let bits = kp.public().modulus_len() * 8;
        return Ok(match bits {
            2048 => KeyType::Rsa2048,
            3072 => KeyType::Rsa3072,
            4096 => KeyType::Rsa4096,
            _ => KeyType::Rsa4096,
        });
    }
    if contains_bytes(der, EC_OID) {
        // Distinguish curve by named curve OID
        const P256_OID: &[u8] = &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
        const P384_OID: &[u8] = &[0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x22];
        const P521_OID: &[u8] = &[0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x23];
        if contains_bytes(der, P256_OID) {
            return Ok(KeyType::EcP256);
        }
        if contains_bytes(der, P384_OID) {
            return Ok(KeyType::EcP384);
        }
        if contains_bytes(der, P521_OID) {
            return Ok(KeyType::EcP521);
        }
        return Ok(KeyType::EcP384); // fallback
    }

    Err(CertError::Crypto("unrecognized PKCS#8 algorithm OID".to_string()))
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------
// AES-256-GCM key encryption/decryption (same format as private_key_encrypted)
// Wire format: nonce[12] || ciphertext || tag[16]
// ---------------------------------------------------------------------------

fn derive_key(passphrase: &str, tenant_id: &str) -> [u8; 32] {
    use ring::{hkdf, hmac};
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, tenant_id.as_bytes());
    let prk = salt.extract(passphrase.as_bytes());
    let info: &[&[u8]] = &[b"ox_cert:private_key_enc_v1"];
    let mut key = [0u8; 32];
    prk.expand(info, MyLen(32))
        .expect("HKDF expand")
        .fill(&mut key)
        .expect("HKDF fill");
    let _ = hmac::Key::new(hmac::HMAC_SHA256, &[]); // silence unused import
    key
}

struct MyLen(usize);
impl ring::hkdf::KeyType for MyLen {
    fn len(&self) -> usize { self.0 }
}

fn encrypt_key(plaintext: &[u8], passphrase: &str, tenant_id: &str) -> Result<Vec<u8>, CertError> {
    use ring::{aead, rand::SecureRandom};
    let rng = ring::rand::SystemRandom::new();
    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| CertError::Crypto("RNG failed".to_string()))?;

    let key_bytes = derive_key(passphrase, tenant_id);
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
        .map_err(|_| CertError::Crypto("AES key init failed".to_string()))?;
    let aead_key = aead::LessSafeKey::new(unbound);
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);

    let mut buf = plaintext.to_vec();
    aead_key
        .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut buf)
        .map_err(|_| CertError::Crypto("AES-GCM encrypt failed".to_string()))?;

    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&buf);
    Ok(out)
}

fn decrypt_key(blob: &[u8], passphrase: &str, tenant_id: &str) -> Result<Vec<u8>, CertError> {
    use ring::aead;
    if blob.len() < 12 + 16 {
        return Err(CertError::Crypto("encrypted key blob too short".to_string()));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let mut nonce_arr = [0u8; 12];
    nonce_arr.copy_from_slice(nonce_bytes);

    let key_bytes = derive_key(passphrase, tenant_id);
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
        .map_err(|_| CertError::Crypto("AES key init failed".to_string()))?;
    let aead_key = aead::LessSafeKey::new(unbound);
    let nonce = aead::Nonce::assume_unique_for_key(nonce_arr);

    let mut buf = ciphertext.to_vec();
    let plaintext = aead_key
        .open_in_place(nonce, aead::Aad::empty(), &mut buf)
        .map_err(|_| CertError::Crypto("AES-GCM decrypt failed".to_string()))?;
    Ok(plaintext.to_vec())
}

// ---------------------------------------------------------------------------
// Private key encryption for CertStore (certificates.private_key_encrypted)
// ---------------------------------------------------------------------------

/// Encrypt a private key DER for storage in `certificates.private_key_encrypted`.
/// Returns base64(nonce[12] || ciphertext || tag[16]).
pub fn encrypt_private_key(
    der: &[u8],
    passphrase: &str,
    tenant_id: &str,
) -> Result<String, CertError> {
    use base64::Engine;
    let blob = encrypt_key(der, passphrase, tenant_id)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&blob))
}

/// Decrypt a private key from `certificates.private_key_encrypted`.
pub fn decrypt_private_key(
    b64: &str,
    passphrase: &str,
    tenant_id: &str,
) -> Result<Vec<u8>, CertError> {
    use base64::Engine;
    let blob = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| CertError::Crypto(format!("base64 decode: {}", e)))?;
    decrypt_key(&blob, passphrase, tenant_id)
}
