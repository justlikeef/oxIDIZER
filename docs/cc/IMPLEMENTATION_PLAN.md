# ox_c_c Implementation Plan

See `DESIGN.md` for architecture, security properties, and data formats.
This file is the step-by-step development roadmap.

---

## Current State

The workspace scaffold is complete and compilable structure exists for all crates.
What is NOT yet implemented (returns stub errors or is `todo!()`):

- `ox_cc_common::verify` — imports compile but `StaticSecret` field interaction needs
  testing against real key generation
- `ox_cc_broker_plugin` — config loading uses `serde_yaml` (added to dep); actual
  key loading and signing I/O in `signing.rs` is wired but untested
- `ox_cc_admin_plugin` — all broker/manifest outbound calls return `Err("not yet implemented")`
  (reqwest mTLS client not configured)
- `ox_cc_client::fetcher` — reqwest blocking client is wired; needs integration testing
  against the manifest plugin
- Rate limiting in `ox_cc_report_plugin` — config field exists; enforcement logic not
  implemented

---

## Phase 1 — Build and Test ox_cc_common

**Goal**: cryptographic core compiles and passes unit tests.

### Steps

1. **Verify workspace compiles**
   ```
   cd /var/repos/ox_c_c_client
   cargo check --workspace
   ```
   Fix any type errors, import issues, or missing `pub` visibility.

2. **Fix known issues to address first**
   - `ox_cc_common/src/verify.rs`: `StaticSecret` must use `diffie_hellman` which
     takes `&PublicKey` — confirm borrow is correct (no owned conversion needed).
   - `ox_cc_broker_plugin/src/config.rs`: `serde_yaml` in `[dependencies]` — confirm
     correct workspace feature flag.
   - `ox_cc_broker_plugin/src/handlers.rs`: `body_str` closure captures `ctx` by ref;
     check borrow lifetime across `db` lock scope.
   - `ox_cc_client/src/fetcher.rs`: `reqwest::blocking::Client` does not implement
     `Clone` in all versions — may need `Arc<Client>` instead.

3. **Write unit tests for ox_cc_common**

   File: `crates/ox_cc_common/src/tests.rs` (add `#[cfg(test)] mod tests;` to lib.rs)

   Tests to write:
   - `test_encrypt_decrypt_roundtrip` — generate keypairs, encrypt, verify+decrypt,
     assert plaintext matches
   - `test_signature_verification_fails_on_tamper` — flip a byte in ciphertext,
     assert `CryptoError::SignatureInvalid`
   - `test_client_id_mismatch_rejected` — encrypt for client A, attempt to decrypt
     as client B, assert `CryptoError::ClientIdMismatch`
   - `test_expired_envelope_rejected` — set `expires_at` in the past, assert
     `CryptoError::ManifestExpired`
   - `test_validity_window_too_large` — set window > `max_manifest_window_secs`,
     assert `CryptoError::ValidityWindowTooLarge`
   - `test_signing_bytes_stable` — serialize with and without signature field,
     confirm canonical bytes are identical
   - `test_aes_gcm_and_chacha` — run roundtrip for both ciphers

4. **Key generation helpers** (add to `ox_cc_common` or a dev binary)

   These are needed for enrollment and testing:
   - Generate Ed25519 keypair → write 32-byte seed + verifying key
   - Generate X25519 keypair → write 32-byte scalar + public key
   - Both in a `ox_cc_keygen` binary or a `cargo run --example keygen` script

---

## Phase 2 — Broker Plugin

**Goal**: broker plugin compiles as cdylib and passes integration tests.

### Steps

1. **Integration test: broker round-trip**
   - Spin up an in-process SQLite broker DB.
   - Call `POST /broker/clients` to register a test client key into the database.
   - Call `submit_template` → `list_pending` → `approve_template`.
   - Ensure `approve_template` yields `202 Accepted` and an async worker processes the signing queue.
   - Confirm `signing_requests` rows have `status = 'approved'` and
     `envelope_wire` is a valid `base64url.base64url` string once the background worker completes.
   - Split the wire string, verify Ed25519 signature, base64-decode, decrypt, assert manifest fields.

2. **Build cdylib**
   ```
   cargo build --release -p ox_cc_broker_plugin
   ```
   Confirm `.so` / `.dll` is produced in `target/release/`.

---

## Phase 3 — Manifest and Report Plugins

**Goal**: manifest/report plugins compile; end-to-end envelope deploy + poll works.

### Steps

1. **Implement rate limiting in ox_cc_report_plugin**
   - Add a `Mutex<LruCache<String, (Instant, u32)>>` (via `lru` crate) to the report module for
     per-client rate tracking to prevent unbounded memory growth from millions of unique IPs.
   - Return 429 Too Many Requests when the limit is exceeded.
   - Reset the window when `Instant::elapsed() > rate_limits.window_secs`.

2. **Integration test: deploy + poll**
   - Open shared manifest_instance.db.
   - Call `handlers::deploy_envelope` with a test envelope.
   - Call `handlers::get_latest` — assert envelope returned.
   - Assert `last_polled_at` is updated.
   - Call `handlers::get_history` — assert one row.
   - Call `handlers::expire_manifest` — assert `is_latest = 0`.
   - Call `handlers::get_latest` again — assert 404.

3. **Integration test: reports**
   - POST two reports for the same `manifest_id` with `sequence` 0 and 1.
   - GET reports — assert both returned in sequence order.
   - POST duplicate `report_id` — assert 200 with `duplicate: true`.
   - POST beyond rate limit — assert 429.

4. **Build cdylibs**
   ```
   cargo build --release -p ox_cc_manifest_plugin -p ox_cc_report_plugin
   ```

---

## Phase 4 — Admin Plugin (outbound HTTP)

**Goal**: admin plugin can make real mTLS calls to broker and manifest.

### Steps

1. **Wire reqwest mTLS client into ox_cc_admin_plugin**
   - Add an `Arc<reqwest::Client>` (async) to the admin module.
   - Build it using `config.tls.*` (identity + CA cert) with `rustls` backend.
   - Pass it into every handler that needs it.
   - Replace all `Err("not yet implemented")` stubs in `handlers.rs`.

2. **Integration test: admin → broker → manifest round-trip**
   - Submit template via admin → verify broker pending.
   - Approve via admin → verify broker approved.
   - Deploy via admin → verify manifest plugin has envelope.
   - Poll GET from client → verify envelope delivered.

---

## Phase 5 — Client Daemon

**Goal**: ox_cc_client compiles and runs the poll loop end-to-end.

### Steps

1. **Fix fetcher blocking/async boundary**
   - `reqwest::blocking::Client` spins up its own thread pools and can cause fragmentation in an async context.
     Migrate completely to `reqwest::Client` (async) to avoid blocking the executor.
   - Confirm `fetcher::Fetcher` correctly builds client with identity from PEM files.

2. **Test poll loop**
   - Write a test that creates a temp manifest_instance.db, inserts an envelope,
     and runs `poll_cycle` once — assert `manifest.json` is written and DB row exists.

3. **Test applied notification retry**
   - Insert a DB row with `applied_notified_at IS NULL`.
   - Run `retry_pending_notifications` against a mock report endpoint.
   - Assert `applied_notified_at` is set on success.
   - Assert `notify_retry_count` increments on failure.

4. **Systemd unit file** (Linux)
   ```
   conf/ox_cc_client.service
   ```
   - `User=ox_cc`, `Group=ox_cc`
   - `ExecStart=/usr/bin/ox_cc_client --config /etc/ox_cc/client.yaml`
   - `Restart=on-failure`, `RestartSec=30s`

5. **Build binary**
   ```
   cargo build --release -p ox_cc_client
   ```

---

## Phase 6 — Admin HTML UI (Jinja2 Templates)

**Goal**: Admin instance serves functional HTML UI via ox_webservice_template_jinja2.

### Steps

1. Write Jinja2 templates in `crates/ox_cc_admin_plugin/templates/`:
   - `index.html` — dashboard: client list with `last_polled_at` and latest manifest
   - `template_new.html` — submission form; calls `GET /admin/api/clients` to populate
     multi-select client list, submits to `POST /admin/api/templates`
   - `template_detail.html` — template detail + per-client envelope status. Must include a "Resubmit for Failed Clients" button to streamline partial batch remediation.
   - `pending.html` — approver queue with approve/reject buttons
   - `client_status.html` — client status, manifest history, recent reports

2. Configure `ox_webservice_template_jinja2` on the admin instance alongside
   `ox_cc_admin_plugin`. No changes to the jinja2 plugin itself.

---

## Phase 7 — ox_webservice mTLS (Hard Dependency)

**Goal**: mTLS client certificate enforcement is live.

### Scope (in the oxIDIZER repo, not this repo)

- Replace `with_no_client_auth()` in `ox_webservice/src/main.rs:303` with
  `with_client_cert_required(ca_cert)`.
- Expose the verified client cert CN in the pipeline state
  (e.g. `request.client_cert_cn`).
- Add per-path CN enforcement in the manifest plugin:
  - `GET /cc/manifest/{client_id}/latest`: require `request.client_cert_cn == client_id`
- Delegate Admin and Approver endpoints to `ox_webservice` JWT enforcement.

Until this phase is complete, the system provides encryption (confidentiality)
but NOT access control at the connection level.

---

## Phase 8 — Deployment Hardening

1. **Key generation**: finalize `keygen` utility (Ed25519 + X25519 keypairs).
2. **Enrollment flow**: document automated enrollment steps (cert issuance +
   X25519 pubkey registration via `POST /broker/clients` into `broker.db`).
3. **File permissions**: document required ACLs for all key files and databases.
4. **OCSP**: document CA configuration requirement (hard-fail OCSP responder).
5. **Payload directory encryption**: document OS-level requirement (dm-crypt / BitLocker).
6. **Smoke test**: end-to-end test against real ox_webservice instances.

---

## Compilation Order

```
ox_cc_common          (lib — no cdylib, no I/O)
  ↓
ox_cc_broker_plugin   (cdylib)
ox_cc_manifest_plugin (cdylib)
ox_cc_report_plugin   (cdylib)
ox_cc_admin_plugin    (cdylib)
ox_cc_client          (bin)
```

All crates are members of the workspace:
```
cargo check --workspace   # type-check all
cargo test --workspace    # run all tests
cargo build --release     # build everything
```

---

## Known Compilation Issues to Fix First

Before writing any new functionality, fix these items that are known to need
attention after the scaffold was written:

1. **`serde_yaml` in ox_cc_client** — add `serde_yaml = { workspace = true }` to
   `crates/ox_cc_client/Cargo.toml` (config.rs uses `serde_yaml::from_str`).

2. **`ox_cc_client` fetcher is blocking** — `reqwest::blocking::Client` must be
   replaced with `reqwest::Client` (async). Blocking reqwest spawns its own thread
   pool and cannot be called from within a tokio async runtime safely.

3. **`ox_cc_common` exports** — `lib.rs` re-exports `Manifest` and
   `EncryptedManifestEnvelope`. Ensure `verify` module is also `pub` in `lib.rs`
   so `ox_cc_client` can call `ox_cc_common::verify::verify_and_decrypt`.

4. **`uuid` in ox_cc_client** — `fetcher.rs` uses `uuid::Uuid::new_v4()` but
   `uuid` is listed as a workspace dep; confirm it is also in
   `crates/ox_cc_client/Cargo.toml` `[dependencies]`.

5. **`rusqlite` must use `sqlcipher` feature** — workspace Cargo.toml currently has
   `features = ["bundled"]` which compiles plain SQLite without encryption. Change to
   `features = ["sqlcipher"]` and ensure system libsqlcipher is installed. All four
   `db.rs` open methods must issue `PRAGMA key = '...'` immediately after open.

6. **`query_map` result pattern** — several handlers call
   `.unwrap_or_default()` on `Result<MappedRows<'_>>` but `MappedRows` does not
   implement `Default`. Correct pattern:
   ```rust
   .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
   .unwrap_or_default()
   ```

7. **`parse::<Value>()` in admin handlers** — `serde_json::Value` does not implement
   `FromStr`. Replace with `serde_json::from_str(&s)?`.
