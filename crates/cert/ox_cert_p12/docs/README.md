# ox_cert_p12

PKCS#12 / PFX bundle export. Packages a server-generated certificate with its private
key and full chain into a password-protected `.p12` file for client download.

---

## Phase

`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/certificates/{serial}.p12` | Download P12 (password via query param) |
| `POST` | `/api/v1/certificates/{serial}.p12` | Download P12 (password in request body) |

Route registration: `"GET,POST /api/v1/certificates/*.p12"`.

**Use the POST form** — the GET form accepts `?password=...` as a query parameter, which
will appear in access logs.

---

## Config Reference

```rust
pub struct P12Config {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
    pub encryption: Pkcs12Encryption,    // Aes256 | TripleDes
}
```

| Field | Default | Description |
|---|---|---|
| `encryption` | `Aes256` | PKCS#12 internal encryption. Use `TripleDes` only for legacy client compatibility. |
| `ca_intermediate_cert_path` | required | Path to intermediate CA PEM for chain building |
| `ca_root_cert_path` | required | Path to root CA PEM for chain building |

---

## Request / Response

**GET:** `GET /api/v1/certificates/{serial}.p12?password=secret`

**POST body:** `{ "password": "secret" }`

**Response:** Binary DER-encoded PKCS#12 file with headers:
- `Content-Type: application/x-pkcs12`
- `Content-Disposition: attachment; filename="{serial}.p12"`

---

## Private Key Decryption

The plugin decrypts the stored private key using:
1. base64-decode `private_key_encrypted` → `nonce[12] || ciphertext || tag[16]`
2. Derive encryption key: HKDF-SHA-256 with IKM=`OX_CA_KEY_PASS`, salt=`tenant_id`,
   info=`"ox_cert:private_key_enc_v1"`, output 32 bytes
3. Decrypt with AES-256-GCM

This requires `OX_CA_KEY_PASS` to be set on nodes serving P12 export requests.

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| Serial not found | 404 | `NOT_FOUND` |
| Certificate revoked | 409 | `ALREADY_REVOKED` |
| Private key not held (CSR-based cert) | 409 | `INVALID_REQUEST` |
| Password missing or empty | 400 | `INVALID_REQUEST` |
| `OX_CA_KEY_PASS` env var not set | 500 | `INTERNAL_ERROR` |
| Decryption failure | 500 | `INTERNAL_ERROR` |
| PKCS#12 build failure | 500 | `INTERNAL_ERROR` |

---

## Implementation Notes

- P12 export is only available for **server-keygen** certificates — where `ox_cert_issue`
  generated the key pair on behalf of the caller. CSR-based certificates return 409
  because the CA never holds the requester's private key.
- An `AuditEvent` with `action = P12Export` is stored on every successful download.
- AES-256 is the default and preferred encryption algorithm. 3DES support (`TripleDes`)
  exists for compatibility with legacy Java keystores and Windows Certificate Manager.
