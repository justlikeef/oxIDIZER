# Design: Commandset Executor and Session Authorization

**Date**: 2026-03-20
**Status**: Approved

---

## Overview

Two independent features extending the ox_c_c_client system:

1. **Commandset Executor** — structured, ordered, multi-step command execution driven by the manifest payload, with per-command failure handling and cumulative state passing between steps.
2. **Session Authorization** — a pre-approved session mechanism allowing programmatic manifest submission (e.g., from arcnition) without per-manifest human approval, while preserving two-person integrity at session open time.

---

## Feature 1: Commandset Executor

### Problem

The manifest `payload` field is currently opaque JSON forwarded verbatim to the consuming agent. There is no standard way to declare a sequence of operations, pass state between steps, or report per-step results. arcnition and other consumers each need to implement their own dispatch logic.

### Solution

Define `commandset` as a first-class structured field in the manifest payload. `ox_cc_client` interprets it via a new `ox_cc_executor` crate, dispatching named commands to built-in implementations or external plugin binaries.

---

### Manifest Payload Structure

`commandset` is a JSON **array** — order is significant and preserved exactly as declared. Each element is a command object:

```json
{
  "commandset": [
    {
      "command": "download",
      "on_failure": "fail",
      "params": {
        "url": "https://cdn.example.com/foo.deb",
        "dest": "/tmp/foo.deb"
      }
    },
    {
      "command": "verify",
      "on_failure": "fail",
      "params": {
        "path": "$dest",
        "sha256": "abc123..."
      }
    },
    {
      "command": "log_info",
      "on_failure": "continue",
      "params": {
        "message": "verified $dest successfully"
      }
    },
    {
      "command": "install",
      "on_failure": "fail",
      "params": {
        "source": "$dest"
      }
    },
    {
      "command": "arcnition",
      "on_failure": "fail",
      "params": {
        "config": "/etc/arcnition/config.yaml"
      }
    }
  ]
}
```

**`on_failure`** controls what happens when the command exits with a non-zero code or returns an error:
- `"fail"` — stop execution immediately, mark commandset as failed. **Default when omitted.**
- `"continue"` — log the failure, record it in the results, continue to the next command.

**`$variable` substitution** — string-typed param values beginning with `$` are resolved from the cumulative state map immediately before each command runs. If a referenced key is not yet present in the state map at the moment a command is dispatched, that is a runtime error for that command (governed by `on_failure`). Pre-execution validation only checks that `$variable` references are syntactically well-formed (non-empty key name after `$`), not that the value is already present — values from earlier command outputs cannot be known before execution.

---

### New Crate: `ox_cc_executor`

```
crates/ox_cc_executor/
└── src/
    ├── lib.rs            # public API: Executor, CommandPlugin, StateMap, CommandEntry
    ├── executor.rs       # sequential dispatch loop + state map management
    ├── substitute.rs     # $variable resolution in params
    └── commands/
        ├── mod.rs        # registry: name → built-in or plugin dir lookup
        ├── download.rs   # HTTPS file download (reuses reqwest)
        ├── install.rs    # file install, chmod, ownership
        ├── log.rs        # log_info: write message to tracing output
        ├── os_info.rs    # gather OS metadata into state map
        └── process.rs    # generic subprocess runner (arcnition and others)
```

`ox_cc_executor` is a library crate. `ox_cc_client` depends on it and calls `executor::run(commandset, &cfg)` after `applier::apply()` writes `manifest.json`.

---

### Core Types

```rust
pub struct CommandEntry {
    pub command: String,
    pub on_failure: OnFailure,
    pub params: serde_json::Map<String, Value>,
}

pub enum OnFailure {
    Fail,       // default
    Continue,
}

/// Cumulative key-value store built up across command outputs.
pub type StateMap = HashMap<String, Value>;

#[async_trait]
pub trait CommandPlugin: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(
        &self,
        params: &serde_json::Map<String, Value>,
        state: &StateMap,
    ) -> anyhow::Result<serde_json::Map<String, Value>>;
}
```

---

### Execution Model

1. Validate all `$variable` references are syntactically well-formed (non-empty key name after `$`). Fail before executing any command if the syntax is invalid.
2. Execute commands sequentially in array order.
3. Immediately before dispatching each command, resolve all `$variable` references in its params from the current state map. If a key is not yet present, treat it as a command failure governed by `on_failure`.
4. After each command completes successfully, merge its output map into the cumulative state map.
5. If a command fails:
   - `on_failure: Fail` → stop, mark remaining commands as `skipped`.
   - `on_failure: Continue` → record failure, continue to next command.
6. Top-level commandset status is `failed` if any `Fail` command failed; `complete` otherwise.

**External process interface**: params are passed as JSON on stdin; the process writes its output JSON object to stdout; the client captures stdout and merges it into the state map. Non-zero exit code is a command failure.

**Plugin discovery**: commands not matching a built-in are looked up as `{plugin_dir}/{command_name}` binary. `plugin_dir` is a new field in `ClientConfig`.

---

### Reporting

A single report is sent to `report_url` at the end of commandset execution. The executor returns a `CommandsetResult` which `main.rs` passes to an extended `post_applied` call.

**Wiring into the existing report infrastructure:**

`post_applied` in `fetcher.rs` (via the `Notifier` trait) gains an optional `detail: Option<String>` parameter. The executor serializes its `CommandsetResult` to a JSON string and passes it as `detail`. The `ReportRequest` struct already has `detail: Option<String>`, so no schema change is needed — the executor's output is serialized to a JSON string before placement in this field.

```json
{
  "status": "failed",
  "commands": [
    { "command": "download", "status": "ok",      "output": { "dest": "/tmp/foo.deb" } },
    { "command": "verify",   "status": "failed",  "error": "checksum mismatch" },
    { "command": "install",  "status": "skipped" }
  ]
}
```

Top-level `status` is `"complete"` or `"failed"`. Commands after a fatal failure are `"skipped"`. `continue` commands that fail appear as `"failed"` but do not propagate to top-level status.

For manifests with no `commandset` in the payload, `detail` remains `None` — backward compatible with the existing behavior.

---

### Broker Policy Integration

`commandset` is added to the per-consumer `allowed_payload_keys`. Optionally, a consumer policy can declare `allowed_commands` to restrict which command names are permitted in submissions to the broker.

---

### Testing

- Unit tests per built-in command using `tempfile` temp directories.
- `CommandPlugin` mock implementations in `#[cfg(test)]` blocks — same pattern as `Notifier` and `HttpClient` already in the codebase.
- Executor integration tests: multi-step commandset with mock plugins asserting correct state map accumulation, `on_failure` behavior, and `$variable` substitution.

---

## Feature 2: Session Authorization

### Problem

Programmatic clients (e.g., arcnition's command server) need to submit a series of manifests as part of a multi-round workflow: send manifest → client executes → client reports results → process results → send next manifest. Requiring human approval for each round is impractical. A pre-approved session mechanism is needed without weakening the two-person integrity guarantee.

### Solution

Sessions are a first-class broker entity. A session is submitted and approved through the same two-person workflow as a template. Once approved, manifests submitted under the session token skip the pending queue and are signed immediately — the session approval IS the human review for all subsequent manifests within the session's declared scope.

---

### Session Scope

A session declares its authority upfront:
- **`client_ids`**: exact set of client_ids the session may target. Manifests submitted under the session must target only these clients.
- **`allowed_commands`**: exact set of command names the session may use. Commands in submitted commandsets must be within this list.
- **`expires_at`**: optional hard expiry timestamp.

The broker enforces both constraints at signing time — a manifest that exceeds the session's declared scope is rejected (422) even if the session token is valid.

---

### Broker Data Model

New table in `broker.db`:

```sql
CREATE TABLE sessions (
    session_id       TEXT PRIMARY KEY,
    submitted_at     TEXT NOT NULL,
    submitted_by     TEXT NOT NULL,       -- cert CN of submitter
    client_ids       TEXT NOT NULL,       -- JSON array
    allowed_commands TEXT NOT NULL,       -- JSON array of permitted command names
    expires_at       TEXT,                -- optional ISO 8601, NULL = no hard expiry
    status           TEXT NOT NULL,       -- pending|approved|rejected|closed|expired
    actioned_at      TEXT,
    actioned_by      TEXT,                -- cert CN of approver/rejecter; must != submitted_by
    rejected_reason  TEXT,
    token            TEXT UNIQUE          -- 32-byte random base64url; set on approval only
);
CREATE INDEX idx_sessions_status ON sessions(status);
CREATE INDEX idx_sessions_token  ON sessions(token);
```

`token` is generated by the broker on approval and returned once. It is never stored in plaintext logs. The admin plugin stores it in `admin.db` for use in subsequent manifest submissions.

---

### Session Token Enforcement

The existing `SubmitTemplateRequest` struct gains one optional field:

```rust
pub struct SubmitTemplateRequest {
    // ... existing fields (template_id, consumer, name, description, payload, client_ids, expires_in_secs, submitted_by) ...
    pub session_token: Option<String>,   // present only for session-driven submissions
}
```

When `session_token` is `None`, the request follows the existing pending-queue path unchanged.

When `POST /broker/request` includes `"session_token": "<token>"`:

1. Look up session by token — 401 if not found.
2. Check `session.status = approved` — 403 if pending/closed/rejected.
3. Check `session.expires_at` has not passed — 403 if expired (also sets status to `expired`).
4. Check all submitted `client_ids ⊆ session.client_ids` — 422 if outside scope.
5. Check all `commandset[*].command ⊆ session.allowed_commands` — 422 if outside scope.
6. Apply existing policy engine (name, description, payload structure) — same as non-session path.
7. Sign immediately — skip pending queue entirely.

Two-person rule: `session.actioned_by != session.submitted_by` enforced at session approval. The session itself satisfies two-person integrity for all manifests within its scope.

Note: the existing template approval flow has this check as a TODO pending mTLS cert CN availability (currently `submitted_by`/`actioned_by` are request body fields). Sessions implement the check now using the same fields. When mTLS is available, both flows will enforce it via cert CN.

---

### Broker API Additions

| Method | Path | Description |
|--------|------|-------------|
| POST | `/broker/sessions` | Submit session open request |
| GET | `/broker/sessions/pending` | List sessions awaiting approval |
| GET | `/broker/sessions/pending/{session_id}` | Session detail for approver |
| POST | `/broker/sessions/pending/{session_id}/approve` | Approve; generate and return token: `{"session_id": "...", "token": "<base64url>"}` |
| POST | `/broker/sessions/pending/{session_id}/reject` | Reject with reason |
| POST | `/broker/sessions/{session_id}/close` | Close active session (admin or approver) |

---

### Admin API Additions

The admin plugin owns the session lifecycle from the operator's perspective. It stores sessions in `admin.db` and proxies to the broker via `HttpClient`.

New table in `admin.db`:

```sql
CREATE TABLE sessions (
    session_id       TEXT PRIMARY KEY,
    created_at       TEXT NOT NULL,       -- local timestamp (= submitted_at sent to broker)
    created_by       TEXT NOT NULL,       -- local operator (= submitted_by sent to broker)
    client_ids       TEXT NOT NULL,       -- JSON array
    allowed_commands TEXT NOT NULL,       -- JSON array
    expires_at       TEXT,
    status           TEXT NOT NULL,       -- mirrors broker status
    token            TEXT                 -- stored on approval; used for manifest submission
);
```

`created_at`/`created_by` are the admin plugin's local field names. When proxying to `POST /broker/sessions`, the admin plugin maps these to `submitted_at`/`submitted_by` to match the broker's schema.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/admin/api/sessions` | Open session request (proxies to broker) |
| GET | `/admin/api/sessions` | List sessions with status |
| GET | `/admin/api/sessions/pending` | Approver queue for sessions |
| POST | `/admin/api/sessions/{session_id}/approve` | Approve (proxies to broker; stores token) |
| POST | `/admin/api/sessions/{session_id}/reject` | Reject (proxies to broker) |
| POST | `/admin/api/sessions/{session_id}/close` | Close session (proxies to broker) |
| POST | `/admin/api/sessions/{session_id}/manifests` | Submit manifest under session (arcnition entry point) |

`POST /admin/api/sessions/{session_id}/manifests` is the programmatic API arcnition uses. The admin plugin retrieves the session token from `admin.db` and includes it in `POST /broker/request`.

---

### Session Lifecycle

```
arcnition: POST /admin/api/sessions  { client_ids, allowed_commands, expires_at }
    ↓
Admin: INSERT admin.db sessions (status=pending), POST /broker/sessions
    ↓
Approver: GET /broker/sessions/pending/{session_id}  (reviews scope)
    ↓
Approver: POST /broker/sessions/pending/{session_id}/approve
    → Broker generates token, stores in sessions.token, returns token
    ↓
Admin: UPDATE admin.db sessions (status=approved, token=...)
    ↓
arcnition loop:
    POST /admin/api/sessions/{session_id}/manifests  { commandset, client_id, ... }
        → Admin includes session_token in POST /broker/request
        → Broker validates scope, signs immediately
        → Admin deploys to manifest instance
        → Client executes commandset
        → Client POSTs results to report_url
        → arcnition reads results, prepares next manifest
    (repeat)
    ↓
arcnition: POST /admin/api/sessions/{session_id}/close
    → Broker sets status=closed; token immediately invalid
```

---

### Testing

**Broker session tests** (inline `#[cfg(test)]` in `broker/db.rs` and `broker/handlers.rs`):
- Token validation: valid token → auto-approve; invalid token → 401.
- Scope enforcement: client_id outside session → 422; command outside allowlist → 422.
- Closed/expired session → 403.
- Two-person rule: same submitter/approver → 403.

**Admin session API tests** (`admin/tests.rs` with `RecordingClient`):
- Session open proxies correctly to broker.
- Token stored on approval and used in manifest submission.
- Close proxies to broker.

---

## Repository Changes Summary

### New files
- `crates/ox_cc_executor/Cargo.toml`
- `crates/ox_cc_executor/src/lib.rs`
- `crates/ox_cc_executor/src/executor.rs`
- `crates/ox_cc_executor/src/substitute.rs`
- `crates/ox_cc_executor/src/commands/mod.rs`
- `crates/ox_cc_executor/src/commands/download.rs`
- `crates/ox_cc_executor/src/commands/install.rs`
- `crates/ox_cc_executor/src/commands/log.rs`
- `crates/ox_cc_executor/src/commands/os_info.rs`
- `crates/ox_cc_executor/src/commands/process.rs`

### Modified files
- `Cargo.toml` — add `ox_cc_executor` to workspace members
- `crates/ox_cc_common/src/manifest.rs` — add `CommandEntry`, `OnFailure` types
- `crates/ox_cc_client/src/config.rs` — add `plugin_dir` field
- `crates/ox_cc_client/src/main.rs` — call executor after applier; pass `CommandsetResult` to extended `post_applied`
- `crates/ox_cc_client/src/fetcher.rs` — extend `Notifier::post_applied` signature to accept `Option<String>` detail
- `crates/ox_cc_broker_plugin/src/db.rs` — add `sessions` table
- `crates/ox_cc_broker_plugin/src/handlers.rs` — add session endpoints + token validation in `POST /broker/request`
- `crates/ox_cc_admin_plugin/src/db.rs` — add `sessions` table
- `crates/ox_cc_admin_plugin/src/handlers.rs` — add session endpoints
- `DESIGN.md` — update repository structure and architecture sections
