import re

# Edit DESIGN.md
with open("DESIGN.md", "r") as f:
    text = f.read()

# 1. Linux setuid -> systemd path
text = re.sub(
    r"### Linux: Setuid Binary.*?For raw packet capture specifically, full root is not required\. `setcap cap_net_raw\+ep`\s+grants only the capability needed, without setuid\.",
    r"""### Linux: Protected Directory via OS Watcher

Using setuid binaries or persistent root daemons expands the attack surface. Instead, Linux leverages a protected staging directory watched by a systemd `.path` unit (or `incron`) running as `root`.

Communication is purely via the file system:

```
1. ox_cc_client   write signed package to staging dir  (ACL: ox_cc write-only)
2. OS (systemd)   .path unit triggers .service unit on directory change
3. Task (root)    validate path is within staging root
4. Task (root)    re-verify Ed25519 signature on package independently
5. Task (root)    invoke dpkg / rpm / systemctl
6. Task exits — no persistent privileged process remains
```

The staging directory ACL: `ox_cc_client` may write; `root` tasks may read; no other account has access. For specific capabilities like raw packet capture, `setcap cap_net_raw+ep` grants only the capability needed to the relevant binary, without full root.""",
    text, flags=re.DOTALL
)

# 2. Update Update Flow (Both Platforms)
text = text.replace(
    "3. ox_cc_client   invoke setuid binary (Linux) / trigger task (Windows)",
    "3. OS logic       systemd path unit (Linux) or Scheduled Task (Windows) triggers automatically"
)
text = text.replace(
    "The setuid binary and Scheduled Task are installed as part of this",
    "The systemd units and Scheduled Task are installed as part of this"
)

# 3. EncryptedManifestEnvelope Format
text = re.sub(
    r"### EncryptedManifestEnvelope Format.*?covered by the Ed25519 signature\.",
    r"""### EncryptedManifestEnvelope Format

To prevent exposing the JSON parser to unverified data, the outer envelope uses a JWS-like delimited format (header and payload combined, separated from signature).

```text
<base64url(envelope_json)>.<base64url(signature)>
```

The `envelope_json` (when decoded) contains:

```json
{
  "version": "1",
  "manifest_id": "550e8400-e29b-41d4-a716-446655440000",
  "client_id": "hostname-or-uuid",
  "consumer": "arcnition",
  "cipher": "aes256gcm",
  "broker_enc_pubkey": "<base64url(broker X25519 public key)>",
  "expires_at": "2026-04-19T12:00:00Z",
  "nonce": "<base64url(12-byte nonce)>",
  "ciphertext": "<base64url(ciphertext + auth tag)>"
}
```

The client splits the string by the `.` delimiter and verifies the `signature` against the raw `base64url(envelope_json)` bytes *before* performing any JSON deserialization. This mitigates risks associated with parsing maliciously crafted JSON payloads.

Fields covered by the signature.""",
    text, flags=re.DOTALL
)

# 4. Verify-then-Decrypt
text = re.sub(
    r"### Verify-then-Decrypt \(Client\).*?6\. ECDH shared secret:",
    r"""### Verify-then-Decrypt (Client)

```
encrypted envelope string (fetched via mTLS + OCSP)
        │
        ▼
1. Split envelope by `.` into b64_payload and b64_signature
        │
        ▼
2. Ed25519_verify(broker_signing_pubkey, b64_payload bytes, signature)
   → discard if invalid — no JSON parsing or decryption attempted
        │
        ▼
3. Base64url-decode and parse b64_payload to EncryptedManifestEnvelope (JSON)
        │
        ▼
4. Check envelope.client_id == configured client_id → discard if mismatch
        │
        ▼
5. Check envelope.expires_at > now (UTC, strict NTP-synchronized clock required)
   → discard if expired — no decryption attempted
        │
        ▼
6. ECDH shared secret:""",
    text, flags=re.DOTALL
)

# 5. Encrypt-then-Sign
text = re.sub(
    r"5\. Build EncryptedManifestEnvelope JSON \(without signature field\).*?9\. Zero sym_key from memory \(ZeroizeOnDrop\)",
    r"""5. Build EncryptedManifestEnvelope JSON
        │
        ▼
6. Encode b64_payload = base64url(canonical_json(envelope_json))
        │
        ▼
7. signature = Ed25519_sign(broker_signing_privkey, b64_payload bytes)
        │
        ▼
8. Attach signature: final_envelope_string = b64_payload + "." + base64url(signature)
   → place in [approved] queue for admin to poll
        │
        ▼
9. Zero sym_key from memory (ZeroizeOnDrop)""",
    text, flags=re.DOTALL
)

# 6. Database Encryption
text = text.replace(
    "SQLite database (`broker.db`, `chmod 600`, WAL mode):",
    "SQLite database (`broker.db`, encrypted via **SQLCipher**, `chmod 600`, WAL mode with `PRAGMA busy_timeout=5000`):"
)
text = text.replace(
    "manifest_instance.db",
    "manifest_instance.db (encrypted via SQLCipher, WAL mode with PRAGMA busy_timeout=5000)"
)
text = text.replace(
    "Admin State Database (`admin.db`, `chmod 600`)",
    "Admin State Database (`admin.db`, encrypted via **SQLCipher**, `chmod 600`, WAL mode with `PRAGMA busy_timeout=5000`)"
)
text = text.replace(
    "State Database (`client.db`, `chmod 600`, owned by `ox_cc`)",
    "State Database (`client.db`, encrypted via **SQLCipher**, `chmod 600`, owned by `ox_cc`, WAL mode with `PRAGMA busy_timeout=5000`)"
)
text = text.replace("encrypted at rest.", "encrypted at rest. (The SQLite databases themselves are encrypted using SQLCipher to protect sensitive metadata).")

# 7. Cancellation API
text = text.replace(
    "| `POST` | `/broker/approved/{template_id}/acknowledge` | Confirm receipt; marks batch delivered |",
    "| `POST` | `/broker/approved/{template_id}/acknowledge` | Confirm receipt; marks batch delivered |\n| `POST` | `/broker/approved/{template_id}/cancel` | Cancel an approved batch before delivery |"
)
text = text.replace(
    "| `POST` | `/admin/api/approved/{template_id}/deploy` | Deploy approved batch to Manifest instance |",
    "| `POST` | `/admin/api/approved/{template_id}/deploy` | Deploy approved batch moving it to clients |\n| `POST` | `/admin/api/templates/{template_id}/cancel` | Cancel an undelivered broker batch OR issue a cancel directive for running manifests |"
)
text = text.replace(
    "| `PATCH` | `/cc/manifest/{client_id}/expire` | mTLS (admin role cert) | Set expires_at to past (effective revocation) |",
    "| `PATCH` | `/cc/manifest/{client_id}/expire` | mTLS (admin role cert) | Set expires_at to past (effective revocation) |\n| `POST` | `/cc/manifest/{client_id}/cancel` | mTLS (admin role cert) | Issue a cancel directive to actively stop a running manifest |"
)

# 8. NTP wording
text = text.replace(
    "(UTC, NTP-synchronized clock)",
    "(UTC, strict NTP-synchronized clock required)"
)

with open("DESIGN.md", "w") as f:
    f.write(text)


import re

# Edit IMPLEMENTATION_PLAN.md
with open("IMPLEMENTATION_PLAN.md", "r") as f:
    plan = f.read()

plan = plan.replace(
    "- Add a `Mutex<HashMap<String, (Instant, u32)>>` to `OxModule` for per-client\n     rate tracking (client_id → (window_start, count)).",
    "- Add a `Mutex<LruCache<String, (Instant, u32)>>` (e.g. via `lru` crate) to `OxModule` for per-client\n     rate tracking to prevent unbounded memory growth from millions of unique IPs."
)
plan = plan.replace(
    "- `reqwest::blocking::Client` cannot be used directly in async context.\n     Use `tokio::task::spawn_blocking` (already done in scaffold) or switch to\n     `reqwest::Client` (async) throughout.",
    "- `reqwest::blocking::Client` spins up its own thread pools and can cause fragmentation in an async context.\n     Migrate completely to `reqwest::Client` (async) to avoid blocking the executor."
)

with open("IMPLEMENTATION_PLAN.md", "w") as f:
    f.write(plan)
