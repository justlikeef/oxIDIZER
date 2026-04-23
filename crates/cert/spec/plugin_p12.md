# ox_cert_p12

**Purpose:** PKCS#12 / PFX bundle export — packages a server-generated certificate with
its private key and full chain for client download.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/certificates/{serial}.p12` | Download PKCS#12 (password via query param) |
| `POST` | `/api/v1/certificates/{serial}.p12` | Download PKCS#12 (password in request body) |

Route registration: `"GET,POST /api/v1/certificates/*.p12"`.

The `GET` form accepts `?password=...` as a query parameter. The `POST` form accepts
`{ "password": "..." }` in the JSON body. POST is preferred as query parameters appear
in server access logs.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `KeyStore`, `CertStore`, `Pkcs12Builder`, `CertError` |
| `p12` | PKCS#12 bundle construction |
| `ring` | AES-256-GCM decryption of stored private key |
| `hkdf` / `sha2` | Key derivation for decryption |
| `base64` | Decode `private_key_encrypted` from TEXT storage |
| `serde` / `serde_json` | Config and request body deserialization |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct P12Config {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    /// PKCS#12 internal encryption algorithm.
    pub encryption: Pkcs12Encryption,   // Aes256 | TripleDes
}
```

---

## Input TaskState Fields

| Field | Description |
|---|---|
| `request.path.serial` | UUID serial of the certificate |
| `request.query.password` | Password for the P12 bundle (GET form) |
| `request.body` | JSON `{ "password": "..." }` (POST form) |

---

## Processing

1. Extract `serial` from path. Extract `password` from query string (GET) or body (POST).
2. If `password` is missing or empty → 400 `INVALID_REQUEST`.
3. `store.get_cert_by_serial(tenant_id, serial)` → 404 if not found.
4. If `record.private_key_encrypted` is `None` → 409 with message:
   `"Private key not held by CA — only available for server-generated certificates"`.
5. If `record.status == Revoked` → 409 `ALREADY_REVOKED`.
6. Decrypt private key:
   a. base64-decode `private_key_encrypted` → `nonce[12] || ciphertext || tag[16]`.
   b. Derive encryption key via HKDF-SHA-256:
      IKM = `std::env::var("OX_CA_KEY_PASS")`,
      salt = `tenant_id.as_bytes()`,
      info = `b"ox_cert:private_key_enc_v1"`.
   c. Decrypt with `ring::aead::AES_256_GCM`.
   d. Result is PKCS#8 DER of the private key.
7. Load chain: `ChainBuilder::from_paths(root_path, intermediate_path)`.
8. Build full chain PEM from `record.pem`.
9. `Pkcs12Builder::build(private_key_der, full_chain_pem, password, config.encryption)`
   → DER bytes of the `.p12` file.
10. Set `response.body` to the DER bytes (binary; use `set_field_bytes` on task context).
11. Set headers:
    - `Content-Type: application/x-pkcs12`
    - `Content-Disposition: attachment; filename="{serial}.p12"`
12. Set `response.status = "200"`.
13. `store.store_audit_event(tenant_id, AuditEvent { action: P12Export, serial, actor, ... })`.

---

## Output TaskState Fields

| Field | Value |
|---|---|
| `response.status` | `"200"` |
| `response.body` (binary) | DER-encoded PKCS#12 bundle |
| `response.header.Content-Type` | `application/x-pkcs12` |
| `response.header.Content-Disposition` | `attachment; filename="{serial}.p12"` |

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| `serial` not found | 404 | `NOT_FOUND` |
| Private key not held (CSR-based cert) | 409 | `INVALID_REQUEST` |
| Certificate revoked | 409 | `ALREADY_REVOKED` |
| Password missing | 400 | `INVALID_REQUEST` |
| `OX_CA_KEY_PASS` env var not set | 500 | `INTERNAL_ERROR` |
| Decryption failure (wrong passphrase) | 500 | `INTERNAL_ERROR` |
| PKCS#12 build failure | 500 | `INTERNAL_ERROR` |

---

## Notes

- The `OX_CA_KEY_PASS` environment variable must be set on all nodes that serve P12
  export requests, since it is required for decrypting the stored private key.
- P12 export is only available for certificates issued in server-keygen mode (where
  `ox_cert_issue` generated the key pair on behalf of the caller). CSR-based certs
  return 409 with a clear explanation.
- Legacy clients requiring 3DES PKCS#12 are supported via `encryption: TripleDes` in
  config. AES-256 is the default and preferred option.
