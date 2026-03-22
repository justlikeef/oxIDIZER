# ox_c_c_client — Detailed Design

## Deployment Topology

Four distinct ox_webservice instances, each with a different trust level and network
exposure:

```
┌──────────────────────────── OPERATOR NETWORK ──────────────────────────────┐
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │  ox_webservice (Admin Instance)                                     │   │
│  │                                                                     │   │
│  │  [ox_cc_admin_plugin (cdylib)]  — API, follows status plugin pattern│   │
│  │   /admin/api/*   — JSON API for template management + reporting     │   │
│  │   mTLS required (admin or approver role cert per endpoint group)    │   │
│  │                                                                     │   │
│  │  [ox_webservice_template_jinja2 (existing)]  — HTML UI              │   │
│  │   /admin/*       — manifest submission, client selection, approvals │   │
│  └──────────────────────────┬──────────────────────────────────────────┘   │
│                             │  mTLS (operator cert)                      │
│                             ▼                                               │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │  ox_webservice (Broker Instance)  — INTERNAL INTERFACE ONLY         │   │
│  │  [ox_cc_broker_plugin (cdylib)]                                     │   │
│  │                                                                     │   │
│  │  Admin API (Access enforced by ox_webservice):                      │   │
│  │   POST /broker/request             — submit template for signing    │   │
│  │   GET  /broker/approved            — poll for approved envelopes    │   │
│  │   GET  /broker/approved/{template_id}                               │   │
│  │   POST /broker/approved/{template_id}/acknowledge                   │   │
│  │   GET  /broker/audit               — query signing audit log        │   │
│  │                                                                     │   │
│  │  Approver API (Access enforced by ox_webservice):                   │   │
│  │   GET  /broker/pending             — list templates awaiting approval│  │
│  │   GET  /broker/pending/{template_id} — decoded view for review      │   │
│  │   POST /broker/pending/{template_id}/approve                        │   │
│  │   POST /broker/pending/{template_id}/reject                         │   │
│  │                                                                     │   │
│  │  Internal:                                                          │   │
│  │   1. validate policy (all clients in batch)                         │   │
│  │   2. ENCRYPT with X25519 ECDH per client (on approval)              │   │
│  │   3. SIGN ciphertext with Ed25519 per client (on approval)          │   │
│  │   GET  /broker/healthz             — liveness (no auth)             │   │
│  └──────────────────────────┬──────────────────────────────────────────┘   │
│                             │                                               │
│  Admin deploys approved envelopes via authenticated POST ──────────────►   │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │  ox_webservice (Manifest Instance)                                  │   │
│  │                                                                     │   │
│  │  [ox_cc_manifest_plugin (cdylib)]  — SQLite-backed                  │   │
│  │   GET   /cc/manifest/{client_id}/latest   — mTLS + OCSP required    │   │
│  │   POST  /cc/manifest/{client_id}          — admin role cert         │   │
│  │   GET   /cc/manifest/{client_id}/history  — admin role cert         │   │
│  │   PATCH /cc/manifest/{client_id}/expire   — admin role cert         │   │
│  │   GET   /cc/clients                       — admin role cert         │   │
│  │   GET   /cc/clients/{client_id}/status    — admin role cert         │   │
│  │                                                                     │   │
│  │  [ox_cc_report_plugin (cdylib)]  — shares same SQLite DB            │   │
│  │   POST /cc/report/{client_id}             — mTLS + OCSP required    │   │
│  │   GET  /cc/report/{client_id}             — admin role cert         │   │
│  │   GET  /cc/report/{client_id}/{manifest_id} — admin role cert       │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
└────────────────────────────────────┬───────────────────────────────────────┘
                    mTLS (client machine cert, OCSP required)
             GET /cc/manifest/{client_id}/latest
             POST /cc/report/{client_id}
                                     │
┌────────────────────────────────────┼───────────────────────────────────────┐
│  CLIENT HOST                       ▼                                       │
│                    [ox_cc_client daemon]  — SQLite state DB               │
│                    presents machine cert (mTLS)                            │
│                    1. VERIFY Ed25519 sig over ciphertext                   │
│                    2. DECRYPT with X25519 ECDH                             │
│                    3. write manifest.json (payload + report metadata)      │
│                    4. POST "applied" notification                          │
│                                                                            │
│    [consuming agent] ──reads──► payload_dir/manifest.json                  │
│    [consuming agent] ──POST reports directly──► Manifest Instance          │
└────────────────────────────────────────────────────────────────────────────┘
```

**Security properties**:
- The Manifest instance holds no signing or encryption keys. A fully-compromised
  Manifest instance cannot forge signatures or read manifest contents.
- Every manifest requires **two separate authenticated actions**: an admin submits, an
  approver approves. Neither can complete the signing alone.
- The admin node never receives an unsolicited push from the broker. It polls. A
  compromised broker cannot push malicious data back to the admin node.
- mTLS + OCSP on all client connections: a revoked machine cert is rejected immediately.
- Signing over ciphertext ensures the signature leaks nothing about payload contents.

---

## Signing Request Lifecycle

An admin prepares one **manifest template** — a single payload with a name, description,
and a list of target client IDs. The admin server assigns a `template_id` (UUIDv4) at
the moment the template is created locally, before anything is sent to the broker. This
ID is the stable reference used throughout: in the broker's queue, in the per-client
manifests, and in the admin server's own records. The broker receives the `template_id`
from the admin and uses it as-is.

The approver reviews and approves the **template** once, seeing both the payload content
and the full list of target clients. On approval the broker generates all N per-client
envelopes in a single batch.

```
Admin submits template (template_id, name, description, payload, [client_id × N])
        │
        ▼
   broker: validate policy for ALL client_ids
   if any client unenrolled or fails policy → reject entire batch (422)
   create N signing_requests linked to template_id
        │
        ▼
   [pending_approval]  ← template sitting in broker queue
        │
        ├── Approver rejects → [rejected]  (all N cancelled; admin notified on next poll)
        │
        └── Approver approves template (sees payload + full client list)
                │
                ▼
           broker: for each client_id independently:
             encrypt with client's X25519 pubkey → sign with Ed25519
             if signing fails for a client → log failure, continue others
                │
                ▼
           [approved] or [partially_approved]
           ← signed envelopes held in broker queue, linked to template_id
           ← failed_client_ids listed in response
                │
                └── Admin polls GET /broker/approved/{template_id}
                    retrieves batch, acknowledges receipt
                            │
                            ▼
                       [delivered]  (envelopes in admin's hands)
                            │
                            └── Admin deploys each via
                                POST /cc/manifest/{client_id} per client
```

Pending templates not actioned within a configurable TTL (default: 24 hours) expire
automatically and are logged. The broker never pushes data to the admin node — all
delivery is pull-based.

---

## Privilege Separation *(for further consideration)*

> This section describes the intended model but is not yet finalized. Package format
> and platform-specific details require further design work before development begins.

The client runs as a non-privileged service account on both Windows and Linux, but
certain operations (raw packet capture/injection, software installation) require
elevated rights.

A persistent privileged helper service with an IPC channel was considered but
rejected: a listening socket or named pipe is itself an attack surface — another
process could use it to invoke privileged operations. The preferred approach
eliminates the channel entirely using short-lived privileged executables invoked
directly by `ox_cc_client` as child processes.

### Linux: Setuid Binary

A small compiled binary (Rust) installed with the setuid bit and restricted execute
permissions (mode `4750`, group `ox_cc`). Only the `ox_cc` service account's group can
invoke it. No socket, no listener, nothing for another process to connect to.

```
ox_cc_client (non-privileged, group ox_cc)
    │
    └── fork/exec  ox_cc_installer  (setuid root, mode 4750, group ox_cc)
                        │
                        validate path is within configured staging root
                        re-verify Ed25519 signature on package
                        invoke dpkg / rpm / systemctl
                        exit
```

**Setuid does not work on scripts.** The binary must be compiled. The binary validates
that the supplied path is within the expected staging root (no path traversal) and
re-verifies the broker's Ed25519 signature independently before invoking the installer.

For raw packet capture specifically, full root is not required. `setcap cap_net_raw+ep`
grants only the capability needed, without setuid.

### Windows: Pre-Configured Scheduled Task

Windows has no setuid equivalent. The preferred approach is a **Scheduled Task**
configured at initial deployment to run as SYSTEM with on-demand triggering.

Communication is through a **staging directory**, not a pipe:

```
1. ox_cc_client   write signed package to staging dir  (ACL: ox_cc write-only)
2. ox_cc_client   trigger Scheduled Task via Task Scheduler COM API
3. Task (SYSTEM)  validate path is within staging root
4. Task (SYSTEM)  re-verify Ed25519 signature on package
5. Task (SYSTEM)  invoke msiexec /i <pkg> /quiet  (max execution time configured)
6. Task exits — no persistent privileged process remains
```

The staging directory ACL: `ox_cc_client` may write; SYSTEM task may read; no other
account has write access. No named pipe or socket exists.

### Update Flow (Both Platforms)

```
1. ox_cc_client   download signed package to staging dir     (no privilege)
2. ox_cc_client   verify Ed25519 signature against broker pubkey
3. ox_cc_client   invoke setuid binary (Linux) / trigger task (Windows)
4. privileged step re-verifies Ed25519 signature independently
5. OS-native installer applies update and restarts service
```

### Initial Deployment

The first installation must be performed remotely via a deployment tool (Ansible,
SCCM, Intune, Group Policy, etc.) or an OS package. One-time, no interactive local
session required. The setuid binary and Scheduled Task are installed as part of this
initial package.

### Package Format

Packages must be proper OS packages (`.deb`/`.rpm` on Linux, `.msi` on Windows) so
the OS installer handles privileged file replacement and service lifecycle correctly.

---

## Repository Structure (Cargo Workspace)

```
ox_c_c_client/
├── Cargo.toml                      # workspace definition
├── PROJECT_INFO.md
├── DESIGN.md
├── crates/
│   ├── ox_cc_common/               # lib — shared crypto, types, no I/O
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── manifest.rs         # plaintext Manifest struct + serde (JSON)
│   │       ├── envelope.rs         # EncryptedManifestEnvelope struct + serde (JSON)
│   │       ├── encrypt.rs          # X25519 ECDH + AES-256-GCM / ChaCha20-Poly1305
│   │       └── verify.rs           # Ed25519 sign/verify over envelope
│   │
│   ├── ox_cc_client/               # bin — daemon on managed hosts
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── config.rs           # ClientConfig loading
│   │       ├── db.rs               # SQLite state DB (manifests table)
│   │       ├── fetcher.rs          # mTLS HTTPS polling of Manifest instance
│   │       └── applier.rs          # write manifest.json (payload + meta) atomically; POST "applied"
│   │
│   ├── ox_cc_broker_plugin/        # cdylib — ox_webservice plugin (Broker instance)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              # C-FFI plugin entry point
│   │       ├── config.rs           # BrokerPluginConfig + client key registry
│   │       ├── db.rs               # SQLite broker DB (templates + signing_requests + audit)
│   │       ├── queue.rs            # signing request lifecycle state machine
│   │       ├── encrypt.rs          # per-client ECDH envelope construction
│   │       ├── signing.rs          # Ed25519 sign over encrypted envelope
│   │       ├── policy.rs           # consumer-scoped allowlist + name/description validation
│   │       └── handlers.rs         # all HTTP handlers (admin + approver APIs)
│   │
│   ├── ox_cc_manifest_plugin/      # cdylib — ox_webservice plugin (Manifest instance)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              # C-FFI plugin entry point
│   │       ├── config.rs           # ManifestPluginConfig (shared db path, admin cert CN)
│   │       ├── db.rs               # shared SQLite manifest_instance.db (encrypted via SQLCipher, WAL mode with PRAGMA busy_timeout=5000)
│   │       └── handlers.rs         # GET/POST/PATCH /cc/manifest/*, GET /cc/clients/*
│   │
│   ├── ox_cc_report_plugin/        # cdylib — ox_webservice plugin (Manifest instance)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              # C-FFI plugin entry point
│   │       ├── config.rs           # ReportPluginConfig (shared db path, rate limits)
│   │       ├── db.rs               # uses shared manifest_instance.db (encrypted via SQLCipher, WAL mode with PRAGMA busy_timeout=5000)
│   │       └── handlers.rs         # POST/GET /cc/report/*
│   │
│   ├── ox_cc_admin_plugin/         # cdylib — ox_webservice plugin (Admin instance)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              # C-FFI entry point, follows ox_webservice_status pattern
│   │       ├── config.rs           # AdminPluginConfig (admin db, broker URL, manifest URL)
│   │       ├── db.rs               # SQLite admin DB (templates + manifest_deployments)
│   │       ├── http_client.rs      # mTLS blocking reqwest client + HttpClient trait for testing
│   │       └── handlers.rs         # all /admin/api/* endpoints
│   │   templates/                  # jinja2 templates served by ox_webservice_template_jinja2
│   │       ├── index.html          # dashboard: client list with status
│   │       ├── template_new.html   # submission form with client multi-select
│   │       ├── template_detail.html
│   │       ├── pending.html        # approver queue
│   │       └── client_status.html
│   │
│   └── ox_cc_keygen/               # bin — keypair generation utility
│       ├── Cargo.toml
│       └── src/
│           └── main.rs             # `broker` subcommand (Ed25519) + `client` subcommand (X25519)
│
└── conf/
    ├── client.example.yaml
    ├── ox_cc_client.service         # systemd unit file
    ├── broker_plugin.example.yaml
    ├── manifest_plugin.example.yaml
    ├── report_plugin.example.yaml
    └── admin_plugin.example.yaml
```

### Plugin Crate Type

All five plugin crates are `cdylib` implementing the `ox_workflow_abi` C-FFI contract
(`ox_plugin_init`, `ox_plugin_process`, `ox_plugin_error`, `ox_plugin_destroy` exports):

```toml
[lib]
crate-type = ["cdylib", "rlib"]
```

Each registers its routes via a module YAML in the respective instance's
`conf/modules/active/` directory and receives a `CoreHostApi` from the host.

The Admin instance additionally loads the existing `ox_webservice_template_jinja2`
plugin (from oxIDIZER) to serve the HTML UI. No changes to that plugin are required.

**Single-instance deployment**: For environments where running four separate
ox_webservice processes is not practical, the Admin, Manifest, and Report plugins can
be co-loaded on a single instance using `conf/modules/ox_cc_combined.yaml`. The Broker
plugin must still run isolated (it holds signing keys).

---

## Cryptographic Design

### Key Material

| Key                           | Type    | Held by                                     | Purpose                                       |
| ----------------------------- | ------- | ------------------------------------------- | --------------------------------------------- |
| Broker signing keypair        | Ed25519 | Broker plugin                               | Sign encrypted envelopes                      |
| Broker encryption keypair     | X25519  | Broker plugin                               | ECDH per-client encryption                    |
| Broker server TLS certificate | X.509   | Broker instance                             | HTTPS server identity for admin/approver mTLS |
| Client encryption keypair     | X25519  | Client (privkey) + Broker registry (pubkey) | ECDH per-client decryption                    |
| Operator Session/JWT          | JWT     | Authorized operator                         | ox_webservice enforces endpoint access        |
| Client TLS certificate        | X.509   | Client                                      | mTLS identity to Manifest instance            |
| Enrollment CA certificate     | X.509   | Manifest instance (trust anchor)            | Validate client TLS certs                     |

Endpoint routing and bearer token security for operators are handled entirely by `ox_webservice`. The plugin solely pulls the user identity from the JWT populated on the request context to enforce separation of duties. Specifically, the `POST /broker/pending/{template_id}/approve` handler ensures the approving `operator_id` is **strictly not equal** to the `submitted_by` identity recorded when the template was created. This ensures no single user can authorize their own manifest.

### Operator Authentication (JWT)

Operator authentication — for human users accessing the broker and admin APIs — is delegated entirely to `ox_webservice`. The ox_cc plugins do **not** validate tokens; they read a pre-validated identity from the pipeline request context.

#### Token Flow

```
Operator browser/client
    │  POST credentials (outside scope of this system)
    ▼
ox_webservice authentication layer
    │  issues signed JWT on successful login
    ▼
Operator receives JWT (stored in browser session or CLI credential store)
    │  includes JWT in Authorization: Bearer <token> header
    ▼
ox_webservice validates JWT signature + expiry on every request
    │  populates request.operator_id and request.operator_roles in context
    ▼
ox_cc plugin reads request.operator_id from context (never the raw token)
```

#### JWT Claims (minimum required)

| Claim | Type | Description |
|-------|------|-------------|
| `sub` | string | Unique operator identifier (`operator_id`). Stable across sessions. |
| `roles` | string[] | List of granted roles. Recognised values: `"admin"`, `"approver"`. |
| `exp` | integer | Expiry (Unix timestamp). ox_webservice rejects expired tokens. |
| `iat` | integer | Issued-at timestamp. |

An operator may hold both roles. Role membership is enforced by ox_webservice at the route level before the plugin receives the request.

#### Two-Person Integrity

The broker plugin enforces at the application layer that no single operator can both submit and approve the same template:

- `POST /broker/request`: records `submitted_by = request.operator_id`
- `POST /broker/pending/{template_id}/approve`: reads `actioned_by = request.operator_id`, then asserts `actioned_by != submitted_by`; returns `403 Forbidden` if equal

This check is redundant with role separation when the admin and approver roles are assigned to different individuals, but remains as a defence-in-depth guard against misconfigured role grants.

#### Context Keys (set by ox_webservice, read by plugins)

| Key | Type | Value |
|-----|------|-------|
| `request.operator_id` | string | `sub` claim from validated JWT |
| `request.operator_roles` | JSON array string | `roles` claim from validated JWT |

#### Dependency

JWT issuance and validation are implemented in `ox_webservice` (oxIDIZER repo). The specific algorithm (HS256/RS256/EdDSA) and issuer configuration are determined by the ox_webservice auth module. This system imposes no requirement on the algorithm — only that `request.operator_id` is populated with a verified, stable identifier before any ox_cc plugin handler is invoked.

---

### Encrypt-then-Sign (Broker, on approval)

Processing order — encryption and signing occur only after an approver has
explicitly approved the request:

```
plaintext manifest JSON
        │
        ▼
1. Serialize to canonical JSON
        │
        ▼
2. ECDH shared secret:
   shared = X25519(broker_enc_privkey, client_enc_pubkey)
        │
        ▼
3. Derive symmetric key:
   sym_key = HKDF-SHA256(
     ikm    = shared,
     salt   = manifest_id (UUID bytes),
     info   = b"ox_cc_encrypt_v1" || client_id || consumer
   )
        │
        ▼
4. Encrypt:
   (ciphertext, tag) = AES-256-GCM(sym_key, random_nonce_12B, canonical_json)
   — or ChaCha20-Poly1305 if configured (see Cipher Selection)
        │
        ▼
5. Build EncryptedManifestEnvelope JSON
        │
        ▼
6. Encode b64_payload = base64url(canonical_json(envelope_json))
        │
        ▼
7. signature = Ed25519_sign(broker_signing_privkey, b64_payload)
        │
        ▼
8. Attach signature: final_envelope_string = b64_payload + "." + base64url(signature)
   → place in [approved] queue for admin to poll
        │
        ▼
9. Zero sym_key from memory (ZeroizeOnDrop)
```

The HKDF `info` field includes both `client_id` and `consumer`, binding the derived
key tightly to its intended recipient and payload type.

### Verify-then-Decrypt (Client)

```
encrypted envelope string (fetched via mTLS + OCSP)
        │
        ▼
1. Split envelope string by `.` into b64_payload and b64_signature
        │
        ▼
2. Iterate through `.pub` files in `broker_signing_pubkeys_dir` and Ed25519_verify(pubkey, b64_payload, signature)
   → discard if no valid signature matches — no JSON parsing or decryption attempted
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
6. ECDH shared secret:
   shared = X25519(client_enc_privkey, broker_enc_pubkey_from_envelope)
        │
        ▼
7. Derive symmetric key (same HKDF params)
        │
        ▼
8. Decrypt (AES-256-GCM or ChaCha20-Poly1305 per envelope.cipher field)
   → canonical JSON of plaintext manifest
        │
        ▼
9. Zero sym_key from memory (ZeroizeOnDrop)
        │
        ▼
10. Deserialize to Manifest; confirm manifest_id, expires_at, consumer match envelope
        │
        ▼
10a. Check manifest.issued_at ≤ now (within 60s clock-skew tolerance)
     Check manifest.expires_at − manifest.issued_at ≤ max_manifest_window_secs
     → discard if window exceeds configured maximum
        │
        ▼
11. Check SQLite manifests table: skip if manifest_id already applied
        │
        ▼
12. Look up manifest.consumer in consumers config → get payload_dir
    → discard if unknown consumer
        │
        ▼
13. Call applier::apply(payload_dir, manifest)
    → writes single manifest.json (payload + report metadata) atomically
        │
        ▼
14. Insert row into manifests table (SQLite, WAL mode)
        │
        ▼
15. POST "applied" notification to Manifest instance; retry on failure
    (consuming agent handles all subsequent progress reports directly)
```

Verification and expiry checks are performed before decryption. An invalid signature or
expired envelope terminates processing without decryption work, preventing oracle attacks.

### Cipher Selection

The envelope includes a `cipher` field (`"aes256gcm"` or `"chacha20poly1305"`),
covered by the Ed25519 signature. Both provide equivalent security. ChaCha20-Poly1305
is preferred on hosts without AES hardware acceleration (ARM, older hardware).
The broker selects the cipher per-client based on the client registry configuration.

### EncryptedManifestEnvelope Format

To prevent exposing the JSON parser to unverified data, the outer envelope uses a JWS-like 
delimited format (header and payload combined, separated from signature).

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

The client splits the string by the `.` delimiter and verifies the `signature` against the 
raw `b64_payload` before performing any JSON deserialization. This mitigates risks 
associated with parsing maliciously crafted JSON payloads.

Fields covered by the signature:
`version`, `manifest_id`, `client_id`, `consumer`, `cipher`, `broker_enc_pubkey`,
`expires_at`, `nonce`, `ciphertext`.

`expires_at` in the outer envelope allows the client to reject expired envelopes before
decryption; the signature prevents an attacker from extending it.

### Plaintext Manifest Format (inner, after decryption)

The inner payload is also **JSON**. Using JSON for both layers minimizes parser attack surface and allows combining the payload and report metadata into one file.

**Payload Scope Restriction**: The `payload` object contains *commands, orchestrations, URIs, and configuration data only*. It should never embed raw bulk binary contents. If the client (`arcnition`) must run a software installer or large executable, the payload directs the client to download the binary dynamically from a static content server (e.g. object storage), verify it against a SHA256 hash included in the payload, uncompress it, and execute it.

```json
{
  "version": "1",
  "manifest_id": "550e8400-e29b-41d4-a716-446655440000",
  "client_id": "hostname-or-uuid",
  "consumer": "arcnition",
  "name": "Enable project scan for acme-corp",
  "description": "Adds get_customer and run_scan stages for acme-corp project.",
  "issued_at": "2026-03-19T12:00:00Z",
  "expires_at": "2026-04-19T12:00:00Z",
  "payload": { "pypeline": [ ... ] }
}
```

After decryption, the client confirms `manifest_id`, `expires_at`, and `consumer` match
the outer envelope values (detects field tampering missed by the signature check).

### Expiry Semantics

`expires_at` is an anti-replay guard: the client refuses to **apply** a manifest whose
expiry has passed. It does not invalidate an already-applied manifest. Once a manifest
is applied, its effects persist until a new manifest overwrites them.

The broker enforces `max_expires_in_secs` (default 90 days, configurable). The client
enforces the same maximum independently via `max_manifest_window_secs` and rejects
envelopes exceeding it even if the signature is valid.

---

## Client Enrollment

Two enrollment steps are required before a client can receive manifests:

### 1. TLS Machine Certificate (mTLS access to Manifest instance)

1. Generate keypair and CSR on the client machine.
2. Sign with the enrollment CA (operator-managed).
3. Install the signed cert + private key on the client.
4. Install the enrollment CA cert on the Manifest instance's ox_webservice TLS config.

**OCSP is a hard requirement.** The enrollment CA must have an OCSP responder
configured. Both the Manifest instance (verifying client certs) and the client
(verifying the server cert) must require a valid OCSP response. Soft-fail OCSP
is not acceptable — a revoked cert must be rejected, not silently accepted.

```yaml
# manifest instance ox_webservice.yaml (TLS section)
tls:
  require_client_cert: true
  client_ca_cert: "/etc/ox_cc/enrollment_ca.crt"
  ocsp_require: true
```

The `client_id` must match the CN of the enrolled certificate.

**Consuming agent cert access:** The consuming agent POSTs progress reports directly
to the Manifest instance using the same client TLS certificate. The client key file
must be readable by the consuming agent's OS process. The key file should be mode
`440` (owner + group readable), group `ox_cc`. The consuming agent must run as the
`ox_cc` user or as a member of the `ox_cc` group. This is a deployment constraint;
the consuming agent does not require elevated rights beyond group membership.

> **CA security**: Follow current best practices for CA key protection (offline storage,
> HSM, or equivalent). This is out of scope for this repository.

### 2. X25519 Encryption Public Key (per-client encryption by broker)

1. Generate an X25519 keypair on the client machine.
2. Register the client's X25519 public key in the broker's client registry. To scale securely, `ox_webservice` maintains this registry inside the broker's SQLite database (`broker.db`), and administrators register new keys dynamically via the `POST /broker/clients` API.

3. Install the client's X25519 private key at the configured path.

---

## Component: `ox_cc_broker_plugin`

### API Surface

**Broker Admin API** (Endpoint access enforced by `ox_webservice`; plugin extracts identity from JWT):

| Method | Path                                         | Description                                |
| ------ | -------------------------------------------- | ------------------------------------------ |
| `POST` | `/broker/request`                            | Submit a manifest template for signing     |
| `GET`  | `/broker/approved`                           | Poll for all approved, undelivered batches |
| `GET`  | `/broker/approved/{template_id}`             | Retrieve a specific approved batch         |
| `POST` | `/broker/approved/{template_id}/acknowledge` | Confirm receipt; marks batch delivered     |
| `POST` | `/broker/approved/{template_id}/cancel`      | Cancel an approved batch before delivery   |
| `GET`  | `/broker/audit`                              | Query the signing audit log                |
| `POST` | `/broker/clients`                            | Register or rotate a client X25519 pubkey  |

**Broker Approver API** (Endpoint access enforced by `ox_webservice`; plugin extracts identity from JWT — must not be submitter):

| Method | Path                                    | Description                                    |
| ------ | --------------------------------------- | ---------------------------------------------- |
| `GET`  | `/broker/pending`                       | List templates awaiting approval               |
| `GET`  | `/broker/pending/{template_id}`         | View decoded human-readable payload for review |
| `POST` | `/broker/pending/{template_id}/approve` | Approve and enqueue signing asynchronously     |
| `POST` | `/broker/pending/{template_id}/reject`  | Reject with optional reason                    |

**Unauthenticated**:

| Method | Path              | Description                              |
| ------ | ----------------- | ---------------------------------------- |
| `GET`  | `/broker/healthz` | Liveness probe (internal interface only) |

### `POST /broker/request` — Request

```json
{
  "template_id": "admin-assigned-uuidv4",
  "consumer": "arcnition",
  "name": "Enable project scan for acme-corp",
  "description": "Adds get_customer and run_scan stages for acme-corp project.",
  "expires_in_secs": 2592000,
  "client_ids": ["host-a", "host-b", "host-c"],
  "payload": { "pypeline": [ ... ] }
}
```

The broker validates policy for **every** `client_id` in the list atomically. If any
client is unenrolled or policy fails for any client, the **entire request is rejected
with `422`** — no partial batches at submission time. The `template_id` (admin-assigned)
is echoed back in the response so the admin can poll by it.

### `GET /broker/pending/{template_id}` — Approver Review

Returns a human-readable decoded representation so the approver can read exactly what
they are authorising:

```json
{
  "template_id": "...",
  "submitted_at": "2026-03-19T12:00:00Z",
  "submitted_by": "<admin cert CN>",
  "consumer": "arcnition",
  "name": "Enable project scan for acme-corp",
  "description": "Adds get_customer and run_scan stages for acme-corp project.",
  "expires_in_secs": 2592000,
  "client_ids": ["host-a", "host-b", "...97 more..."],
  "client_count": 100,
  "payload_decoded": { "pypeline": [ ... ] }
}
```

The approver sees the full client list alongside the decoded payload. Approval
authorises signing for all clients in the list simultaneously.

### Partial Batch Signing

On approval, the broker signs per-client envelopes **independently**. If signing fails
for one or more clients after approval (e.g., a client's key was removed from the
registry between submission and approval), the broker:

1. Signs all clients whose keys are present and valid.
2. Logs a failure entry per failed `client_id` in the audit log.
3. Sets the template status to `partially_approved`.
4. Includes a `failed_client_ids` array in the `GET /broker/approved/{template_id}`
   response so the admin can see which clients were not signed.

The admin can resubmit a new template for the failed clients only.

### Policy Engine

```yaml
# broker_plugin.example.yaml
policy:
  max_expires_in_secs: 7776000    # 90 days (configurable; client enforces same)
  pending_request_ttl_secs: 86400 # 24h — unactioned templates expire
  consumers:
    arcnition:
      allowed_stages:
        - "arcnition.stages.get_customer"
        - "arcnition.stages.get_project"
        - "arcnition.stages.select_interface"
        - "arcnition.stages.run_scan"
      max_stages: 20
```

Validation checks on every request:
- `consumer` must exist in policy config.
- `client_id` must exist in client registry; `allowed_consumers` list enforced if set.
- Each stage value must be in the consumer's `allowed_stages`.
- Stage count ≤ `max_stages`.
- `expires_in_secs` ≤ `max_expires_in_secs`.
- `name`: required, max 200 characters, no HTML tags, no control characters.
- `description`: required, max 2000 characters, no HTML tags, no control characters.
- No shell metacharacters, absolute paths, or template directives in stage values.

### Rate Limiting

```yaml
rate_limits:
  admin_requests_per_minute: 10
  approver_actions_per_minute: 30
  audit_queries_per_minute: 60
```

### Audit Log

| Event                       | Fields logged                                                                 |
| --------------------------- | ----------------------------------------------------------------------------- |
| Template submitted          | timestamp, admin cert CN, template_id, consumer, client_count, policy outcome |
| Template approved           | timestamp, approver cert CN, template_id                                      |
| Template rejected           | timestamp, approver cert CN, template_id, reason                              |
| Template expired            | timestamp, template_id                                                        |
| Signing failed (per client) | timestamp, template_id, client_id, reason                                     |
| Batch acknowledged          | timestamp, admin cert CN, template_id                                         |

Append-only, queryable via `GET /broker/audit` (admin role). The admin interface
maintains its own local copy of retrieved audit events.

### Key Management

| Key                           | File                                | Usage                                   |
| ----------------------------- | ----------------------------------- | --------------------------------------- |
| Ed25519 signing private key   | `broker_signing.pem`                | Sign envelopes (ZeroizeOnDrop)          |
| X25519 encryption private key | `broker_enc.pem`                    | ECDH encrypt per-client (ZeroizeOnDrop) |
| Server TLS certificate + key  | `broker_tls.crt` / `broker_tls.key` | HTTPS server identity                   |

Key files: `chmod 400`, owned by the process user. Loaded once at plugin init.
Key material is `ZeroizeOnDrop` — memory is zeroed when the key struct is dropped.

### Broker State Database

SQLite database (`broker.db`, encrypted via **SQLCipher**, `chmod 600`, WAL mode with `PRAGMA busy_timeout=5000`):

```sql
CREATE TABLE manifest_templates (
  template_id       TEXT PRIMARY KEY,
  submitted_at      TEXT NOT NULL,
  submitted_by      TEXT NOT NULL,
  consumer          TEXT NOT NULL,
  name              TEXT NOT NULL,
  description       TEXT NOT NULL,
  payload_path      TEXT NOT NULL,    -- path to encrypted payload file; NOT stored inline
  expires_in_secs   INTEGER NOT NULL,
  status            TEXT NOT NULL,    -- pending | approved | partially_approved | rejected | expired
  actioned_at       TEXT,
  actioned_by       TEXT,
  failed_client_ids TEXT              -- JSON array; populated on partial signing failure
);

CREATE TABLE signing_requests (
  request_id    TEXT PRIMARY KEY,
  template_id   TEXT NOT NULL REFERENCES manifest_templates(template_id),
  client_id     TEXT NOT NULL,
  status        TEXT NOT NULL,    -- pending | signed | delivered
  envelope_json TEXT,             -- populated on signing
  delivered_at  TEXT
);

CREATE TABLE audit_log (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  timestamp     TEXT NOT NULL,
  event_type    TEXT NOT NULL,
  actor         TEXT NOT NULL,
  template_id   TEXT,
  client_id     TEXT,
  detail        TEXT
);
```

**Payload storage**: The payload is stored as a file on disk (`payload_path`), not
inline in the database. The payload storage directory **must be encrypted at rest**.
`payload_path` is relative to a configured base directory and is never derived from
user-supplied input (prevents path traversal).

---

## Component: `ox_cc_manifest_plugin`

ox_webservice plugin loaded by the Manifest instance. Backed by a SQLite database
shared with `ox_cc_report_plugin` — both plugins are configured with the same
`db_path` (`manifest_instance.db (encrypted via SQLCipher, WAL mode with PRAGMA busy_timeout=5000)`). This shared database enables cross-plugin queries
(e.g., "last report received for client X").

### API Surface

| Method  | Path                               | Auth                              | Description                                                  |
| ------- | ---------------------------------- | --------------------------------- | ------------------------------------------------------------ |
| `GET`   | `/cc/manifest/{client_id}/latest`  | mTLS + OCSP (client machine cert) | Serve latest signed envelope                                 |
| `POST`  | `/cc/manifest/{client_id}`         | mTLS (Admin backend cert)         | Store new signed envelope                                    |
| `GET`   | `/cc/manifest/{client_id}/history` | mTLS (Admin backend cert)         | List historical envelopes                                    |
| `PATCH` | `/cc/manifest/{client_id}/expire`  | mTLS (Admin backend cert)         | Set expires_at to past (effective revocation)                |
| `POST`  | `/cc/manifest/{client_id}/cancel`  | mTLS (Admin backend cert)         | Issue a cancel directive to actively stop a running manifest |
| `GET`   | `/cc/clients`                      | mTLS (Admin backend cert)         | List all clients with last-polled timestamp                  |
| `GET`   | `/cc/clients/{client_id}/status`   | mTLS (Admin backend cert)         | Client status summary                                        |

`GET /cc/manifest/{client_id}/latest` supports `ETag` / `If-None-Match` for efficient
polling. Every call (including 304 responses) updates `last_polled_at` for that client.

`PATCH /cc/manifest/{client_id}/expire` sets the current envelope's `expires_at` to a
past timestamp, making it immediately stale to any client that fetches it. History is
never deleted — all previous envelopes are retained for audit.

`GET /cc/clients/{client_id}/status` returns:
- `latest_manifest_id`, `stored_at`, `stored_by`
- `last_polled_at`
- `last_report_received_at` (joined from reports table in shared DB)

### Manifest Instance Database (`manifest_instance.db (encrypted via SQLCipher, WAL mode with PRAGMA busy_timeout=5000)`, shared with report plugin)

```sql
CREATE TABLE envelopes (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  client_id       TEXT NOT NULL,
  manifest_id     TEXT NOT NULL UNIQUE,
  stored_at       TEXT NOT NULL,
  stored_by       TEXT NOT NULL,
  envelope_json   TEXT NOT NULL,
  is_latest       INTEGER NOT NULL,  -- 1 for current, 0 for historical
  last_polled_at  TEXT               -- updated on every client GET, even 304
);
CREATE INDEX idx_envelopes_client ON envelopes(client_id, is_latest);
```

---

## Component: `ox_cc_report_plugin`

Handles inbound progress reports from mTLS-authenticated clients and read queries from
the admin. Shares `manifest_instance.db (encrypted via SQLCipher, WAL mode with PRAGMA busy_timeout=5000)` with `ox_cc_manifest_plugin`.

Multiple reports per manifest are expected; multiple manifests may be in-flight
simultaneously on a single client.

### API Surface

| Method | Path                                   | Auth                                         | Description                                          |
| ------ | -------------------------------------- | -------------------------------------------- | ---------------------------------------------------- |
| `POST` | `/cc/report/{client_id}`               | mTLS + OCSP (cert CN must match `client_id`) | Store a progress report                              |
| `GET`  | `/cc/report/{client_id}`               | mTLS (Admin backend cert)                    | List reports for client (newest first)               |
| `GET`  | `/cc/report/{client_id}/{manifest_id}` | mTLS (Admin backend cert)                    | Reports for a specific manifest, ordered by sequence |

### Rate Limiting

```yaml
# report_plugin.example.yaml
rate_limits:
  reports_per_client_per_minute: 60   # configurable
max_body_bytes: 65536
```

### Report Format

```json
{
  "manifest_id": "550e8400-e29b-41d4-a716-446655440000",
  "report_id": "uuidv4-unique-per-report",
  "sequence": 3,
  "timestamp": "2026-03-19T12:05:00Z",
  "status": "in_progress",
  "detail": "stage get_project complete"
}
```

`status` values: `in_progress`, `complete`, `failed`.

On duplicate `report_id` the plugin returns `200 OK` without re-inserting — the
`UNIQUE` constraint on `report_id` silently discards the duplicate, allowing the
consuming agent to safely retry failed POSTs.

### Report Database (in shared `manifest_instance.db (encrypted via SQLCipher, WAL mode with PRAGMA busy_timeout=5000)`)

```sql
CREATE TABLE reports (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  client_id     TEXT NOT NULL,
  manifest_id   TEXT NOT NULL,
  report_id     TEXT NOT NULL UNIQUE,
  sequence      INTEGER NOT NULL,
  received_at   TEXT NOT NULL,
  status        TEXT NOT NULL,
  detail        TEXT
);
CREATE INDEX idx_reports_manifest ON reports(client_id, manifest_id, sequence);
```

---

## Component: `ox_cc_admin_plugin`

The admin interface runs on a dedicated ox_webservice instance. Two plugins are loaded:

1. **`ox_cc_admin_plugin` (this crate)** — all JSON API endpoints, follows the
   `ox_webservice_status` plugin pattern (request dispatch via `request.verb` and
   `request.path`, responses via `response.body` + `response.type`).

2. **`ox_webservice_template_jinja2` (existing plugin from oxIDIZER, unmodified)** —
   serves jinja2 HTML templates from the `templates/` directory. The UI is a thin
   client that calls the API plugin endpoints for all data.

### API Surface (`/admin/api/*`)

**Admin Dashboard API** (Access enforced by `ox_webservice` via JWT):

| Method  | Path                                              | Description                                                                          |
| ------- | ------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `GET`   | `/admin/api/clients`                              | List enrolled clients from broker registry (for client selection UI)                 |
| `POST`  | `/admin/api/templates`                            | Create and submit new template to broker                                             |
| `GET`   | `/admin/api/templates`                            | List all templates with status                                                       |
| `GET`   | `/admin/api/templates/{template_id}`              | Template detail + per-client signing status                                          |
| `GET`   | `/admin/api/approved`                             | Poll broker for approved batches                                                     |
| `POST`  | `/admin/api/approved/{template_id}/deploy`        | Deploy approved batch moving it to clients                                           |
| `POST`  | `/admin/api/templates/{template_id}/cancel`       | Cancel an undelivered broker batch OR issue a cancel directive for running manifests |
| `GET`   | `/admin/api/audit`                                | Query broker audit log                                                               |
| `GET`   | `/admin/api/manifest-clients`                     | Client list from Manifest instance                                                   |
| `GET`   | `/admin/api/manifest-clients/{client_id}/status`  | Client status                                                                        |
| `GET`   | `/admin/api/manifest-clients/{client_id}/history` | Manifest deployment history                                                          |
| `GET`   | `/admin/api/reports/{client_id}`                  | Reports for a client                                                                 |
| `GET`   | `/admin/api/reports/{client_id}/{manifest_id}`    | Reports for specific manifest                                                        |
| `PATCH` | `/admin/api/manifest-clients/{client_id}/expire`  | Expire current manifest                                                              |

**Approver Review API** (Access enforced by `ox_webservice` via JWT):

| Method | Path                                       | Description                              |
| ------ | ------------------------------------------ | ---------------------------------------- |
| `GET`  | `/admin/api/pending`                       | List templates pending approval          |
| `GET`  | `/admin/api/pending/{template_id}`         | Decoded payload + client list for review |
| `POST` | `/admin/api/pending/{template_id}/approve` | Approve                                  |
| `POST` | `/admin/api/pending/{template_id}/reject`  | Reject with reason                       |

### HTML UI Routes (served by `ox_webservice_template_jinja2`)

| Path                             | Template               | Description                                                |
| -------------------------------- | ---------------------- | ---------------------------------------------------------- |
| `/admin/`                        | `index.html`           | Dashboard: client list with last-seen and current manifest |
| `/admin/templates/new`           | `template_new.html`    | Submission form with client multi-select                   |
| `/admin/templates/{template_id}` | `template_detail.html` | Template status per client                                 |
| `/admin/pending`                 | `pending.html`         | Approver queue                                             |
| `/admin/clients/{client_id}`     | `client_status.html`   | Client status and report history                           |

**Client selection feature**: `template_new.html` calls `GET /admin/api/clients` on
page load to populate a multi-select list of enrolled clients. The operator selects
which clients will receive the manifest before submitting the form. The form POST sends
the full `client_ids` array to `POST /admin/api/templates`.

### Configuration (`conf/admin_plugin.example.yaml`)

```yaml
broker_url: "https://broker.internal"
manifest_instance_url: "https://manifest.example.com"
db_path: "/var/lib/ox_cc/admin.db"
tls:
  client_cert: "/etc/ox_cc/admin.crt"
  client_key: "/etc/ox_cc/admin.key"
  ca_cert: "/etc/ox_cc/broker_ca.crt"
```

### Admin State Database (`admin.db`, encrypted via **SQLCipher**, `chmod 600`, WAL mode with `PRAGMA busy_timeout=5000`)

```sql
-- Templates owned by this admin node; template_id assigned locally before broker submission
CREATE TABLE templates (
  template_id       TEXT PRIMARY KEY,
  created_at        TEXT NOT NULL,
  created_by        TEXT NOT NULL,
  consumer          TEXT NOT NULL,
  name              TEXT NOT NULL,
  description       TEXT NOT NULL,
  client_ids_json   TEXT NOT NULL,   -- JSON array of target client_ids
  status            TEXT NOT NULL,   -- draft | submitted | pending | approved |
                                     -- partially_approved | rejected | deployed
  broker_status     TEXT,
  rejected_reason   TEXT,
  failed_client_ids TEXT             -- JSON array, from broker on partial failure
);

-- Maps each per-client manifest_id back to its parent template
-- Populated when the admin acknowledges and stores the approved batch
CREATE TABLE manifest_deployments (
  manifest_id     TEXT PRIMARY KEY,
  template_id     TEXT NOT NULL REFERENCES templates(template_id),
  client_id       TEXT NOT NULL,
  deployed_at     TEXT,
  envelope_json   TEXT             -- local copy of the signed+encrypted envelope
);
```

---

## Component: `ox_cc_client`

### Configuration (`conf/client.example.yaml`)

```yaml
client_id: "hostname-or-uuid"
manifest_url: "https://manifest.example.com/cc/manifest/{client_id}/latest"
report_url: "https://manifest.example.com/cc/report/{client_id}"
poll_interval_secs: 300       # actual interval = this ± 10% random jitter
max_manifest_window_secs: 7776000  # 90 days — reject envelopes with longer window
state_db: "/var/lib/ox_cc/client.db"   # SQLite, mode 600, owned by ox_cc

consumers:
  arcnition:
    payload_dir: "/var/repos/arcnition/conf/pipeline"

tls:
  ca_cert: "/etc/ox_cc/server_ca.crt"
  client_cert: "/etc/ox_cc/client.crt"
  client_key: "/etc/ox_cc/client.key"   # mode 440, group ox_cc
  min_version: "1.3"

crypto:
  client_enc_privkey: "/etc/ox_cc/client_enc.key"
  broker_signing_pubkeys_dir: "/etc/ox_cc/trusted_broker_keys/"
```

### Poll Loop (`fetcher.rs`)

```
On startup and every poll_interval_secs ± 10% jitter:

  0. Check SQLite for any rows where applied_notified_at IS NULL
     → retry "applied" POST for each with exponential backoff (notify_retry_count)

  1. GET manifest_url over mTLS (TLS 1.3 minimum, OCSP required)
     - On 304 Not Modified (ETag/If-None-Match) → skip
     - On 401/403 → log + alert, continue
     - On 404 → log, continue
     - On network error → log + exponential backoff, continue

  2. Parse JSON → EncryptedManifestEnvelope

  3. Check envelope.client_id == configured client_id → discard if mismatch

  4. Check envelope.expires_at > now → discard if expired (no decryption)

  5. Iterate through `.pub` files in `broker_signing_pubkeys_dir`; Ed25519_verify(pubkey, canonical_bytes, signature)
     → discard if no key validates the signature (no decryption)

  6. ECDH + symmetric decrypt (cipher per envelope.cipher field)

  7. Zero symmetric key (ZeroizeOnDrop)

  8. Deserialize inner manifest; confirm manifest_id, expires_at, consumer match envelope

  9. Check manifest.issued_at ≤ now (within 60s clock-skew tolerance)
     Check manifest.expires_at − manifest.issued_at ≤ max_manifest_window_secs
     → discard if window exceeds configured maximum

 10. Check SQLite manifests table: skip if manifest_id already applied

 11. Look up manifest.consumer in consumers config → get payload_dir
     → discard if unknown consumer

 12. Call applier::apply(payload_dir, manifest)
     → writes manifest.json (payload + report metadata) atomically

 13. Insert row into manifests table (SQLite, WAL mode)

 14. POST "applied" notification; set applied_notified_at on success
     (consuming agent handles all subsequent progress reports directly)
```

### Manifest Applier (`applier.rs`)

1. Build a single JSON object combining the decrypted payload and report metadata:

```json
{
  "manifest_id": "550e8400-e29b-41d4-a716-446655440000",
  "report_url": "https://manifest.example.com/cc/report/hostname-or-uuid",
  "client_cert": "/etc/ox_cc/client.crt",
  "client_key": "/etc/ox_cc/client.key",
  "ca_cert": "/etc/ox_cc/server_ca.crt",
  "payload": { "pypeline": [ ... ] }
}
```

2. Write to a temp file in `payload_dir` (same filesystem for atomic rename).
3. Atomically rename to `manifest.json` — **one rename, no mismatched-pair risk**.

The consuming agent reads `manifest.json` to get both the payload and the reporting
metadata. It uses `manifest_id` to tag its progress reports and the `report_url` +
cert paths to POST directly to the Manifest instance over mTLS.

`ox_cc_client` is not an intermediary for reports. Multiple manifests from different
consumers may be active simultaneously; each has its own `manifest_id` and payload_dir.

### State Database (`client.db`, encrypted via **SQLCipher**, `chmod 600`, owned by `ox_cc`, WAL mode with `PRAGMA busy_timeout=5000`)

WAL mode enabled. SQLite page checksums provide integrity. File ACL restricts access
to the `ox_cc` service account.

```sql
CREATE TABLE manifests (
  manifest_id           TEXT PRIMARY KEY,
  consumer              TEXT NOT NULL,
  name                  TEXT NOT NULL,
  description           TEXT NOT NULL,
  applied_at            TEXT NOT NULL,
  expires_at            TEXT NOT NULL,
  applied_notified_at   TEXT,          -- NULL until "applied" POST succeeds
  notify_retry_count    INTEGER NOT NULL DEFAULT 0
);
```

If the database is missing or corrupt on startup, the client logs a security alert,
rebuilds an empty database, and treats no manifest as having been applied.

---

## Data Flow Summary

```
Admin (via browser)   Broker             Approver      Manifest Instance   Client Host
   [Admin Instance]      │                   │                │                │
        │                │                   │                │                │
        │─POST /templates►│                   │                │                │
        │                │ validate policy    │                │                │
        │◄─{template_id}─│                   │                │                │
        │                │                   │                │                │
        │                │◄──GET /pending─────│                │                │
        │                │──decoded view─────►│                │                │
        │                │◄──POST /approve────│                │                │
        │                │ encrypt+sign (×N)  │                │                │
        │                │                   │                │                │
        │─GET /approved──►│                   │                │                │
        │◄─N envelopes───│                   │                │                │
        │─POST /acknowledge►│                 │                │                │
        │                │                   │                │                │
        │─POST /cc/manifest/{id} (×N) ───────────────────────►│                │
        │                │                   │                │                │
        │                │                   │       mTLS+OCSP│                │
        │                │                   │◄───GET /latest──────────────────│
        │                │                   │────envelope────────────────────►│
        │                │                   │          verify│                │
        │                │                   │          decrypt                │
        │                │                   │          write manifest.json    │
        │                │                   │◄───POST /report (×N progress)───│
        │                │                   │          [consuming agent]      │
        │─GET /reports───────────────────────────────────────►│                │
```

---

## Security Properties

| Threat                                                | Mitigation                                                                                         |
| ----------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Compromised Manifest instance forges pipeline         | Ed25519 sig rejects it — signing key never leaves Broker                                           |
| Compromised Manifest instance reads manifest contents | Impossible — encrypted per-client before storage                                                   |
| Single operator submits and self-approves a manifest  | The broker ensures `operator_id` (from JWT) of the Approver is strictly unequal to the Submitter   |
| Broker pushes malicious data to admin node            | Impossible — admin always polls; broker never pushes                                               |
| Unenrolled machine connects to Manifest instance      | mTLS + OCSP rejects connection                                                                     |
| Revoked machine cert used after decommission          | OCSP hard-required; revoked cert rejected immediately                                              |
| Valid cert fetches another client's manifest          | `client_id` in envelope verified; ECDH decryption fails without correct privkey                    |
| Replay of valid envelope to wrong host                | `client_id` embedded and verified; ECDH keyed to client privkey                                    |
| Replay of expired envelope                            | `expires_at` in signed envelope checked before decryption                                          |
| Envelope `expires_at` extended by attacker            | Field covered by Ed25519 signature                                                                 |
| Client configured with oversized expiry window        | Client enforces `max_manifest_window_secs` independently post-decryption                           |
| Clock manipulation to accept expired manifests        | NTP dependency documented; `issued_at` sanity checked post-decryption                              |
| MITM on manifest fetch                                | mTLS (TLS 1.3 min) + Ed25519 sig — two independent layers                                          |
| Signature fingerprinting plaintext payload            | Signature is over ciphertext — plaintext never seen by signer output                               |
| Parser attack on envelope before sig check            | Both envelope and inner manifest are JSON — minimal attack surface                                 |
| Broker signing key exfiltration                       | Broker bound to internal interface; key file mode 400; ZeroizeOnDrop                               |
| Flood of signing requests                             | Configurable rate limiting per role                                                                |
| Report endpoint flooded by valid cert holder          | Configurable per-client rate limit on report plugin                                                |
| State DB tampered to force re-apply or block update   | SQLite page checksums; mode 600 ACL; corrupt DB resets to safe state                               |
| Setuid binary invoked with path traversal             | Binary validates path is within configured staging root                                            |
| Malicious stage injected via broker                   | Consumer-scoped allowlist; metacharacter check; name/description validated                         |
| `broker_enc_pubkey` substituted in envelope           | Field covered by Ed25519 signature                                                                 |
| Per-manifest symmetric key reuse                      | HKDF salt = manifest UUID; info includes client_id + consumer                                      |
| Sensitive data in logs                                | Explicit implementation constraint: no key material or plaintext in tracing output                 |
| Consuming agent runs as wrong user, can't read key    | Key file mode 440, group ox_cc; consuming agent must be in ox_cc group                             |
| Partial signing failure silently drops clients        | Logged per client_id; `failed_client_ids` in broker response; template marked `partially_approved` |
| name/description injection (XSS, control chars)       | Validated at broker: max length, no HTML tags, no control characters                               |
| Payload exposed from broker DB exfiltration           | Payload stored as file, not inline; payload directory must be encrypted at rest                    |

---

## Dependency Choices

| Crate                       | Purpose                                                           |
| --------------------------- | ----------------------------------------------------------------- |
| `ed25519-dalek`             | Ed25519 signing and verification (ZeroizeOnDrop)                  |
| `x25519-dalek`              | X25519 ECDH shared secret derivation (ZeroizeOnDrop)              |
| `aes-gcm`                   | AES-256-GCM authenticated encryption                              |
| `chacha20poly1305`          | ChaCha20-Poly1305 authenticated encryption (ARM / no AES-NI)      |
| `hkdf` + `sha2`             | HKDF-SHA256 symmetric key derivation                              |
| `serde` + `serde_json`      | Manifest and envelope serialization (JSON throughout)             |
| `serde_yaml`                | Plugin configuration file loading (YAML)                          |
| `ox_workflow_abi`           | ox_webservice C-FFI plugin contract (path dep to oxIDIZER)        |
| `reqwest` (rustls, TLS 1.3) | mTLS HTTPS client in `ox_cc_client` and `ox_cc_admin_plugin`      |
| `uuid`                      | UUIDv4 manifest, template, and request IDs                        |
| `chrono`                    | Timestamp handling and expiry checks                              |
| `tokio`                     | Async runtime                                                     |
| `clap`                      | CLI argument parsing                                              |
| `tracing`                   | Structured logging (no key material or plaintext)                 |
| `base64`                    | base64url encoding for binary fields                              |
| `rand`                      | Cryptographically secure nonce generation                         |
| `rusqlite`                  | SQLite databases — WAL mode, `bundled-sqlcipher` feature          |
| `zeroize`                   | Explicit key material zeroing where ZeroizeOnDrop is insufficient |

`serde_yaml` is still used for loading plugin configuration files (YAML). Both the outer
envelope and the inner manifest are JSON, but configuration files remain YAML.
No standalone HTTP server dependency — plugin crates delegate HTTP to their
ox_webservice host via the `ox_workflow_abi` C-FFI interface (path dependency to oxIDIZER).

---

## Open Questions / Future Work

- **ox_webservice mTLS / cert-based auth (hard dependency)**: mTLS with client
  certificate validation is currently disabled in ox_webservice (`with_no_client_auth()`
  in `ox_webservice/src/main.rs:303`). Per-path CN enforcement (cert CN must match the
  `client_id` URL segment) also does not yet exist. Both must be implemented in
  ox_webservice before this system can be deployed. Until then, ECDH encryption still
  protects confidentiality, but connection-level access control is incomplete.

- **Key rotation**: The client watches a directory of trusted broker signing keys to allow
  overlapping trust windows. However, rotating the broker enc key continuously may require
  additional V2 design; `broker_enc_pubkey` in the envelope supports this without client changes.

- **Log Pruning**: SQLite databases for reports and audit logs are append-only. A future
  feature is required to implement data retention policies and automatically prune or archive
  rows older than a configured threshold to prevent disk exhaustion.

- **Enrollment automation**: v1 enrollment is manual. Future: enrollment API protected
  by a one-time token.

- **Report confidentiality**: v1 status reports are sent in plaintext over mTLS.
  Future: encrypt reports using the same ECDH scheme in reverse.

- **Broker HA**: v1 is single-instance. Future: HSM or Vault for key storage and
  signing, enabling multi-instance broker without key replication.

- **Push delivery**: v1 polls. Future: SSE or WebSocket push from Manifest instance.

- **Admin UI**: In this repo as `ox_cc_admin_plugin` + jinja2 templates. The Admin
  ox_webservice instance is separate from the oxIDIZER deployment.
