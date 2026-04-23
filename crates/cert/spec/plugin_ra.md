# ox_cert_ra

**Purpose:** Registration Authority — approval workflow for certificate requests requiring
manual review. Provides approval/denial API and re-submits approved requests into the
workflow pipeline via `ox_messaging`.

---

## Phase
`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/ra/pending` | List pending approval requests |
| `GET` | `/api/v1/ra/pending/{id}` | Get request details (CSR, requester, policy notes) |
| `POST` | `/api/v1/ra/pending/{id}/approve` | Approve → triggers re-issuance |
| `POST` | `/api/v1/ra/pending/{id}/deny` | Deny with reason |
| `GET` | `/api/v1/ra/history` | Approved + denied requests with pagination |
| `GET` | `/api/v1/ra/requests/{id}/certificate` | Poll for issued certificate after approval |

Route registration: `"GET,POST /api/v1/ra/*"`.

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `ox_cert_core` | `CertStore`, `ApprovalRequest`, `AuditEvent`, `CertError`, `enqueue_task` helper |
| `serde` / `serde_json` | Request/response serialization |
| `uuid` (v4) | Generate approval request IDs and new task IDs |
| `time` | Timestamps |

---

## Config

```rust
#[derive(Debug, Deserialize)]
pub struct RaConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    /// Queue name to publish approved task IDs into (default: "tasks.pending").
    pub resubmit_queue: String,
    /// Priority for re-submitted issuance tasks (0–255; default: 100).
    pub resubmit_priority: u8,
    /// Auto-approval rules — requests matching these are approved without human action.
    pub auto_approve_rules: Vec<AutoApproveRule>,
    /// Webhook URL to notify RA officers of new pending requests.
    pub notification_webhook: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AutoApproveRule {
    /// Regex applied to `requester_identity`.
    pub identity_pattern: String,
    /// Profiles this rule applies to.
    pub profiles: Vec<String>,
}
```

---

## Re-Submission Mechanism

When an RA officer approves a request, `ox_cert_ra` does NOT make an HTTP call to
`/api/v1/certificates`. Instead, it uses the `ox_messaging` task queue:

1. Load the `ApprovalRequest` from `CertStore` (includes `csr_pem`, `profile`, `sans`).
2. Build the re-submission request body JSON string:
   ```json
   { "csr": "<csr_pem>", "profile": "<profile>", "sans": ["..."] }
   ```
3. Create a new workflow `Task` record in the workflow storage system with the following
   `metadata` map (all values are `String`):

   | Key | Value |
   |---|---|
   | `flow_name` | Name of the issuance pipeline flow (same flow that handled the original request) |
   | `cert.ra.approved` | `"true"` |
   | `cert.ra.request_id` | The `ApprovalRequest.id` |
   | `request.body` | The re-submission request body JSON string (step 2) |
   | `request.method` | `"POST"` |
   | `request.path` | `"/api/v1/certificates"` |
   | `tenant_id` | `config.tenant_id` |

4. Publish the new task's UUID to the resubmit queue:
   ```rust
   ox_cert_core::enqueue_task(&api, &task_id.to_string(), config.resubmit_priority)?;
   ```
   This calls `CoreHostApi::publish_to_queue` (see spec/core.md for the extension spec).
4. The workflow scheduler picks up the task and runs the standard issuance pipeline.
   `ox_cert_issue` reads `cert.ra.approved == "true"` from TaskState metadata and
   proceeds past the RA check.
5. The newly issued cert's serial is written back to `ApprovalRequest.certificate_serial`
   (new optional field) by `ox_cert_issue` after successful storage.

---

## Processing (by endpoint)

### `GET /api/v1/ra/pending`

```
store.list_ra_pending(tenant_id, &pagination) → PagedResult<ApprovalRequest>
```
Return JSON list with pagination meta.

### `GET /api/v1/ra/pending/{id}`

```
store.get_ra_request(tenant_id, id) → ApprovalRequest | 404
```
Return full request including `csr_pem` (for RA officer to inspect).

### `POST /api/v1/ra/pending/{id}/approve`

Input body: `{ "reviewer_notes": "Approved — matched CMDB inventory" }` (optional)

1. `store.get_ra_request(tenant_id, id)` → 404 if not found.
2. If `status != Pending` → 409 with message "Request already processed".
3. Check auto-approve rules: if request matches a rule, this endpoint can still be called
   by an officer, but the auto-approve path (see below) may have already handled it.
4. `store.update_ra_request(tenant_id, id, Approved, reviewer_identity, notes)`.
5. Execute re-submission (steps 1–5 in Re-Submission Mechanism above).
6. `store.store_audit_event(tenant_id, AuditEvent { action: RaApprove, ... })`.
7. Return 200 `{ data: { id, status: "approved", task_id }, meta: { tenant_id } }`.

### `POST /api/v1/ra/pending/{id}/deny`

Input body: `{ "reason": "Domain not in CMDB" }` (required)

1. `store.get_ra_request(tenant_id, id)` → 404 if not found.
2. If `status != Pending` → 409.
3. `store.update_ra_request(tenant_id, id, Denied, reviewer_identity, reason)`.
4. `store.store_audit_event(...)`.
5. Return 200 `{ data: { id, status: "denied" }, meta: { tenant_id } }`.

### `GET /api/v1/ra/history`

```
store.list_ra_pending filtered by status IN (Approved, Denied) with pagination
```
(Uses `CertStore` custom filter or raw_sql call for status filtering.)

### `GET /api/v1/ra/requests/{id}/certificate`

1. `store.get_ra_request(tenant_id, id)` → 404 if not found.
2. If `status != Approved` → 202 with `{ status: "pending" }`.
3. If `certificate_serial` is None → 202 with `{ status: "processing" }` (task queued
   but not yet issued).
4. If `certificate_serial` is Some: `store.get_cert_by_serial(tenant_id, serial)`.
5. Return 200 with the certificate JSON (same shape as `ox_cert_issue` response).

---

## Auto-Approval

During `ox_cert_issue`, before creating an `ApprovalRequest`, the plugin evaluates
the configured `auto_approve_rules`. If any rule matches (`identity_pattern` regex matches
`requester_identity` AND profile is in `rule.profiles`), the request is auto-approved:
the `cert.ra.approved = "true"` flag is set directly in TaskState (without storing an
`ApprovalRequest`) and issuance proceeds normally.

`ox_cert_ra` also exposes a background check: on startup and every 5 minutes, it scans
`list_ra_pending` and re-applies auto-approve rules, handling requests that arrived when
the rule was not yet configured.

---

## Notification

When a new `ApprovalRequest` is stored by `ox_cert_issue`, a notification is posted to
`notification_webhook` (if configured):

```json
{
  "event": "ra_request_pending",
  "request_id": "uuid",
  "tenant_id": "acme-corp",
  "requester_identity": "10.0.0.1",
  "profile": "long_lived",
  "sans": ["internal.example.com"],
  "created_at": "2026-04-22T10:00:00Z"
}
```

The webhook call is made by `ox_cert_issue` (not `ox_cert_ra`) immediately after storing
the `ApprovalRequest`. It is fire-and-forget (failure is logged but does not affect the
202 response).

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| Request not found | 404 | `NOT_FOUND` |
| Request already processed | 409 | `INVALID_REQUEST` |
| `reason` missing on deny | 400 | `INVALID_REQUEST` |
| Queue publish failure on approve | 500 | `INTERNAL_ERROR` |
| Storage failure | 500 | `INTERNAL_ERROR` |
