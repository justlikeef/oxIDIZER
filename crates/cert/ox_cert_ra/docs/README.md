# ox_cert_ra

Registration Authority. Provides an approval workflow for certificate requests that
require manual review. After approval, re-submits the request into the workflow pipeline
via the task queue — not via a direct HTTP call.

---

## Phase

`Content`

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/ra/pending` | List pending requests (paginated) |
| `GET` | `/api/v1/ra/pending/{id}` | Get request details including CSR |
| `POST` | `/api/v1/ra/pending/{id}/approve` | Approve and trigger re-issuance |
| `POST` | `/api/v1/ra/pending/{id}/deny` | Deny with reason |
| `GET` | `/api/v1/ra/history` | Approved and denied requests |
| `GET` | `/api/v1/ra/requests/{id}/certificate` | Poll for issued cert after approval |

Route registration: `"GET,POST /api/v1/ra/*"`.

---

## Config Reference

```rust
pub struct RaConfig {
    pub tenant_id: String,
    pub store: CertStoreConfig,
    pub resubmit_queue: String,          // default: "tasks.pending"
    pub resubmit_priority: u8,           // default: 100
    pub auto_approve_rules: Vec<AutoApproveRule>,
    pub notification_webhook: Option<String>,
}

pub struct AutoApproveRule {
    pub identity_pattern: String,        // regex applied to requester_identity
    pub profiles: Vec<String>,           // profiles this rule applies to
}
```

| Field | Default | Description |
|---|---|---|
| `resubmit_queue` | `"tasks.pending"` | Queue name for re-submitting approved tasks |
| `resubmit_priority` | `100` | Priority for re-submitted tasks (0–255) |
| `auto_approve_rules` | `[]` | Rules for automatic approval without human action |
| `notification_webhook` | absent | URL to notify RA officers of new pending requests |

---

## Re-Submission Mechanism

When an RA officer approves a request, the plugin does NOT make an HTTP call to
`/api/v1/certificates`. Instead:

1. Loads the stored `ApprovalRequest` (has `csr_pem`, `profile`, `sans`).
2. Creates a new workflow `Task` record with metadata:
   - `cert.ra.approved = "true"`
   - `cert.ra.request_id = <approval request UUID>`
   - `request.body = <JSON: {"csr": ..., "profile": ..., "sans": [...]}>`
   - `request.method = "POST"`, `request.path = "/api/v1/certificates"`
3. Publishes the task UUID to `tasks.pending` via `CoreHostApi::publish_to_queue`.
4. The workflow scheduler picks up the task and runs the standard issuance pipeline.
5. `ox_cert_issue` reads `cert.ra.approved == "true"` and skips the RA check.

After issuance, `ox_cert_issue` writes the new `CertificateRecord.serial` back to
`ApprovalRequest.certificate_serial`. Callers can poll
`GET /api/v1/ra/requests/{id}/certificate` to check issuance status.

---

## Auto-Approval

If an `auto_approve_rules` entry matches (identity pattern AND profile match), the request
is approved automatically during `ox_cert_issue` evaluation — the RA approval check is
bypassed by setting `cert.ra.approved = "true"` directly in `TaskState`. No
`ApprovalRequest` record is stored.

`ox_cert_ra` also runs a background scan every 5 minutes: any pending requests that now
match an auto-approve rule are approved and re-queued automatically.

---

## Error Cases

| Condition | HTTP | Code |
|---|---|---|
| Request not found | 404 | `NOT_FOUND` |
| Request already processed (approve/deny) | 409 | `INVALID_REQUEST` |
| `reason` missing on deny | 400 | `INVALID_REQUEST` |
| Queue publish failure | 500 | `INTERNAL_ERROR` |
| Storage failure | 500 | `INTERNAL_ERROR` |

---

## Implementation Notes

- The `notification_webhook` fires when `ox_cert_issue` stores the `ApprovalRequest`. It
  is fire-and-forget from `ox_cert_issue`, not from `ox_cert_ra`.
- `GET /api/v1/ra/requests/{id}/certificate` returns 202 while the task is queued but
  not yet processed; 200 with the certificate once issued.
- This plugin requires `CoreHostApi::publish_to_queue` — the extension function pointer
  described in `spec/core.md`. It is not part of the base `ox_workflow_abi`.
