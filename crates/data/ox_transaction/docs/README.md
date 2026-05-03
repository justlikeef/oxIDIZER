# ox_transaction

Record-level locking and transaction semantics for `GenericDataObject`. Both capabilities
store their state in the GDO extension slot under `"ox.transaction"`.

---

## LockStatus

```rust
pub enum LockStatus {
    Unlocked,
    Locked { holder: Uuid, expires_at: Option<DateTime<Utc>> },
    PendingLock(Uuid),
    PendingUnlock(Uuid),
}
```

When `expires_at` is set, the lock is automatically released on the next lock attempt
after the expiry time — regardless of who holds it.

---

## Transactable Trait

Implemented on `GenericDataObject`. State is read from and written to the
`"ox.transaction"` extension slot.

| Method | Description |
|---|---|
| `lock(holder, ttl)` | Acquire lock. Fails if held by a different holder. |
| `unlock(holder)` | Release lock (must be same holder). |
| `force_unlock(admin_token)` | Break any lock; requires admin token. |
| `request_lock(holder, ttl)` | Queue a lock request (non-blocking intent). |
| `request_unlock(holder)` | Queue an unlock request. |
| `is_locked()` | Check lock status. |
| `is_locked_by(holder)` | Check if specific holder holds the lock. |
| `is_expired()` | Check if lock TTL has elapsed. |
| `begin_transaction(holder, ttl)` | Lock + mark transaction active in extension slot. |
| `commit_transaction()` | Unlock + clear transaction state. |
| `rollback_transaction()` | Re-load from store (`discard()`) + unlock. |
| `transaction_active()` | True if a transaction is in progress. |

---

## Transaction Semantics

`DataObjectManager::save_data_object` checks `gdo.transaction_active()`:

**With active transaction (locking enabled):**
1. Write to each container in sequence.
2. If any write fails: reverse completed writes, call `rollback_transaction()`, return `Err`.
3. If all succeed: call `commit_transaction()`.

**Without active transaction:**
1. Write to each container.
2. If any write fails: log error, continue.
3. Return success (with warnings logged).

---

## Callback Events

| Event pair | Fired by |
|---|---|
| `before_lock` / `after_lock` / `on_error_lock` | `lock` |
| `before_unlock` / `after_unlock` / `on_error_unlock` | `unlock` |
| `before_request_lock` / `after_request_lock` | `request_lock` |
| `before_force_unlock` / `after_force_unlock` | `force_unlock` |

---

## Extension Slot (`"ox.transaction"`)

```json
{
  "status": "Locked",
  "holder": "550e8400-...",
  "expires_at": "2026-04-23T12:00:00Z",
  "transaction_active": true
}
```

The extension slot value survives `to_serializable_map()` / `from_serializable_map()`
so lock state is preserved during GDO serialization. On `hydrate()`, the lock status
is refreshed from the backing store.

---

## notify_lock_change

Calls `PersistenceDriver::notify_lock_status_change("locked"/"unlocked", gdo_id)` on
the active driver using the GDO's `PersistenceInfo`. This enables the driver to enforce
the lock at the storage level (e.g., PostgreSQL advisory lock). No-op if
`PersistenceInfo` is not set.

---

## Example

```rust
use ox_transaction::Transactable;

let session_id = Uuid::new_v4();

// Atomic multi-container save
gdo.begin_transaction(session_id, Some(Duration::from_secs(30)))?;
match manager.save_data_object("user", &mut gdo) {
    Ok(_) => {}  // commit_transaction() was called by save_data_object
    Err(e) => {
        // rollback_transaction() was called by save_data_object
        eprintln!("Save failed and rolled back: {}", e);
    }
}
```
