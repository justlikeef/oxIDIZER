# ox_cert_ssh

SSH Certificate Authority. Issues OpenSSH user and host certificates in the native
OpenSSH binary format (not X.509). Manages separate signing keys for user CAs and host
CAs.

---

## Phase

`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/ssh/sign` | Sign an SSH public key; return an SSH certificate |
| `POST` | `/api/v1/ssh/renew` | Renew an existing SSH certificate |
| `GET` | `/api/v1/ssh/ca/user` | User CA public key (for `TrustedUserCAKeys` on SSH servers) |
| `GET` | `/api/v1/ssh/ca/host` | Host CA public key (for `known_hosts` entries) |
| `GET` | `/api/v1/ssh/config` | Recommended `sshd_config` and `ssh_config` snippets |

Route registration: `"GET,POST /api/v1/ssh/*"`.

---

## Config Reference

```rust
pub struct SshConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub keystore: KeyStoreConfig,
    pub user_ca: SshCaConfig,
    pub host_ca: SshCaConfig,
    pub user: SshPrincipalPolicy,
    pub host: SshPrincipalPolicy,
}

pub struct SshCaConfig {
    pub key_id: String,                // Key ID in KeyStore; generated on first init if missing
    pub key_type: SshCaKeyType,        // Ed25519 | EcdsaP256 | EcdsaP384
    pub default_validity: String,      // Duration string, e.g. "16h", "30d"
}

pub struct SshPrincipalPolicy {
    pub allowed_principals: Vec<String>,     // Glob patterns, e.g. ["*"] or ["*.example.com"]
    pub default_extensions: HashMap<String, String>,
    pub default_critical_options: HashMap<String, String>,
    pub max_validity: Option<String>,        // Cap on requested validity
}
```

| Field | Default | Description |
|---|---|---|
| `user_ca.key_id` | required | KeyStore key ID for user CA signing key |
| `user_ca.key_type` | required | `Ed25519` recommended for SSH CA keys |
| `user_ca.default_validity` | required | e.g. `"16h"` for short-lived user certs |
| `host_ca.default_validity` | required | e.g. `"720h"` (30 days) for host certs |
| `user.allowed_principals` | required | Glob patterns for permitted user principals |
| `host.allowed_principals` | required | Glob patterns, e.g. `["*.example.com"]` |
| `user.default_extensions` | `{}` | Default OpenSSH extensions for user certs |

---

## Request Body (`POST /api/v1/ssh/sign`)

```json
{
  "public_key":       "ssh-ed25519 AAAA...",
  "cert_type":        "user",
  "principals":       ["alice", "admin"],
  "validity":         "16h",
  "key_id":           "alice@example.com",
  "critical_options": {},
  "extensions":       { "permit-pty": "", "permit-port-forwarding": "" }
}
```

`public_key`, `cert_type`, and `principals` are required. `principals` must be non-empty.
`validity` defaults to `default_validity` in config. `extensions` defaults to
`default_extensions` from config if omitted.

---

## SSH Serial Numbers

SSH certificate serials are u64 (per the OpenSSH wire format), not UUID v4. Generated
via an atomic `UPDATE ssh_serial_counter SET next_serial = next_serial + 1 WHERE
tenant_id = $1 RETURNING next_serial` — safe under concurrent writers in active/active
deployments.

---

## Output (`POST /api/v1/ssh/sign`)

```json
{
  "data": {
    "certificate":  "ssh-ed25519-cert-v01@openssh.com AAAA...",
    "serial":       12345,
    "cert_type":    "user",
    "principals":   ["alice"],
    "valid_after":  "2026-04-22T10:00:00Z",
    "valid_before": "2026-04-23T02:00:00Z",
    "key_id":       "alice@example.com"
  },
  "meta": { "tenant_id": "acme-corp", "request_id": "uuid" }
}
```

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| Missing `public_key` or `cert_type` | 400 | `INVALID_REQUEST` |
| Unparseable public key | 400 | `INVALID_REQUEST` |
| Principal not allowed by `allowed_principals` | 403 | `POLICY_VIOLATION` |
| Validity exceeds `max_validity` | 400 | `POLICY_VIOLATION` |
| SSH CA key not initialized | 503 | `CA_NOT_READY` |
| Storage failure | 500 | `INTERNAL_ERROR` |

---

## Operator Integration Notes

**User certificates:** Configure SSH servers with:
```
TrustedUserCAKeys /etc/ssh/trusted_user_cas
```
where the file contains the output of `GET /api/v1/ssh/ca/user`.

**Host certificates:** Sign each server's host public key:
```bash
curl -X POST https://ca.example.com/api/v1/ssh/sign \
  -d '{"public_key": "ssh-ed25519 AAAA...", "cert_type": "host", "principals": ["server.example.com"]}'
```

Then on clients, add to `~/.ssh/known_hosts`:
```
@cert-authority *.example.com ssh-ed25519 AAAA...
```
(The host CA public key is at `GET /api/v1/ssh/ca/host`.)

The `GET /api/v1/ssh/config` endpoint returns pre-formatted snippets for both.
