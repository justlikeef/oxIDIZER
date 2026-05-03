# ox_cert_issue

Issues a new X.509 certificate from a submitted CSR or an auto-generated key pair.

---

## Phase

`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/certificates` | Issue a new certificate |

---

## Config Reference

```rust
pub struct CertIssueConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub default_profile: String,
    pub policy: IssuancePolicyConfig,
    pub ct: Option<CtConfig>,
    pub extensions: ExtensionsConfig,
    pub ca_intermediate_key_id: String,
    pub ca_intermediate_cert_path: String,
    pub ca_root_cert_path: String,
}
```

| Field | Default | Description |
|---|---|---|
| `tenant_id` | required | Tenant this instance serves |
| `default_profile` | required | Profile name used when request omits `profile` |
| `policy.domain_allowlist` | `[]` | Regex list; SANs must match at least one |
| `policy.domain_blocklist` | `[]` | Regex list; SANs must not match any |
| `policy.max_san_count` | `100` | Upper bound on number of SANs |
| `policy.wildcard_allowed` | `true` | Whether `*.example.com` SANs are permitted |
| `policy.min_rsa_bits` | `2048` | Minimum RSA key size in bits |
| `policy.require_ra_approval` | `false` | Force all requests through RA approval |
| `ct.enabled` | `false` | Submit to Certificate Transparency logs |
| `ct.min_scts` | `2` | Minimum SCTs required |
| `ct.on_failure` | `warn` | `warn` or `block` on CT failure |
| `ca_intermediate_key_id` | required | Key ID in KeyStore for the signing key |
| `ca_intermediate_cert_path` | required | Path to intermediate CA PEM for chain building |
| `ca_root_cert_path` | required | Path to root CA PEM for chain building |

---

## Request Body

```json
{
  "csr":              "-----BEGIN CERTIFICATE REQUEST-----\n...",
  "profile":          "standard",
  "validity_seconds": 31536000,
  "sans":             ["example.com", "www.example.com"],
  "key_type":         "ecc-p256"
}
```

`csr` is required unless operating in server-keygen mode (supply `key_type` and `sans`
instead). `profile` defaults to `default_profile` from config. `sans` in the body
overrides SANs in the CSR (enrichment use case).

Content-Type `application/pkcs10` is also accepted; the body is treated as a PEM CSR
directly.

---

## Output TaskState Fields

| Field | Value |
|---|---|
| `response.status` | `"201"` |
| `response.body` | JSON: `{ data: { serial, subject_cn, not_before, not_after, profile, certificate, chain, scts[] }, meta: { ... } }` |
| `cert.issued.serial` | UUID string of the issued cert |
| `cert.issued.not_after` | RFC3339 expiry timestamp |

---

## Processing Steps (summary)

1. Parse CSR; verify self-signature.
2. Resolve effective profile; validate against `IssuancePolicy`.
3. If profile requires RA approval and `cert.ra.approved != "true"`: store
   `ApprovalRequest`, return 202 `RA_APPROVAL_REQUIRED`.
4. Apply webhook enrichment from `cert.webhook.enrichment` (additional SANs, custom
   extensions, subject OU).
5. Generate UUID v4 serial.
6. Build and sign certificate via `CertBuilder`.
7. If `ct.enabled`: submit to CT logs; embed SCTs in cert.
8. Build full chain PEM via `ChainBuilder`.
9. Store `CertificateRecord` and `AuditEvent`.
10. Return 201 with certificate and chain.

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| Missing or unparseable CSR | 400 | `INVALID_CSR` |
| CSR self-signature invalid | 400 | `INVALID_CSR` |
| Unknown profile | 400 | `INVALID_REQUEST` |
| Policy violation (domain, key strength, SANs, wildcard) | 403 | `POLICY_VIOLATION` |
| RA approval required | 202 | `RA_APPROVAL_REQUIRED` |
| CA not initialized | 503 | `CA_NOT_READY` |
| CT failure with `on_failure = block` | 502 | `CT_FAILURE` |
| Storage failure | 500 | `INTERNAL_ERROR` |

---

## Server-Keygen Mode

When the request body supplies `key_type` and `sans` without a `csr`, the plugin
generates a key pair:

1. Generates key via `KeyStore::generate_key(tenant_id, serial, key_type, false)`.
2. Builds an internal self-signed CSR from the generated public key.
3. Issues the certificate normally.
4. Encrypts and stores the private key: AES-256-GCM with HKDF-derived key from
   `OX_CA_KEY_PASS`. Stored as `base64(nonce||ciphertext||tag)` in
   `CertificateRecord.private_key_encrypted`.

Server-keygen is required for PKCS#12 export. CSR-based certs cannot be exported as P12
because the CA never holds the private key.

---

## Webhook Integration

If `ox_cert_webhook` runs before `ox_cert_issue` in the pipeline, `ox_cert_issue` reads:
- `cert.webhook.authorized`: must be `"true"` or absent (if webhook not in pipeline).
- `cert.webhook.enrichment`: JSON object with optional `additional_sans`,
  `custom_extensions`, and `subject_ou` fields that are merged into the issuance.
