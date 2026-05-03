# ox_transaction — Transactions & Record Locking

**Crate:** `ox_transaction`
**Type:** library

Provides two related capabilities as a single addon:

1. **Record-level locking** — pessimistic concurrency control preventing concurrent
   modifications to the same object.
2. **Transaction semantics** — when locking is active on a multi-container save,
   operations complete atomically (all succeed or all are rolled back). Without locking,
   failures are logged but execution continues.

Both capabilities store their state in the GDO's extension slot under `"ox.transaction"`.

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

Serialized in the extension slot as:

```json
{ "status": "Unlocked" }
{ "status": "Locked", "holder": "550e8400-...", "expires_at": "2026-04-23T12:00:00Z" }
{ "status": "PendingLock", "holder": "..." }
{ "status": "PendingUnlock", "holder": "..." }
```

`expires_at` is optional. When set, the lock is automatically released by any lock
attempt after the expiry time has passed, regardless of who holds it.

---

## Transaction Callback Events

All operations fire through the GDO's two-level callback dispatch.

| Event | Fired by |
|-------|----------|
| `before_lock` / `after_lock` / `on_error_lock` | `lock` |
| `before_unlock` / `after_unlock` / `on_error_unlock` | `unlock` |
| `before_request_lock` / `after_request_lock` / `on_error_request_lock` | `request_lock` |
| `before_request_unlock` / `after_request_unlock` / `on_error_request_unlock` | `request_unlock` |
| `before_force_unlock` / `after_force_unlock` / `on_error_force_unlock` | `force_unlock` |

`after_*` fires only on success. `on_error_*` fires in its place on failure with
`params.error` set to the error description.

---

## Transactable Trait

Implemented directly on `GenericDataObject`. State is read from and written to the
`"ox.transaction"` extension slot.

```rust
pub trait Transactable {
    fn get_lock_status(&self) -> LockStatus;
    fn set_lock_status(&mut self, status: LockStatus);

    fn lock(&mut self, holder: Uuid, ttl: Option<Duration>) -> Result<(), OxDataError>;
    fn unlock(&mut self, holder: Uuid) -> Result<(), OxDataError>;
    fn force_unlock(&mut self, admin_token: &str) -> Result<(), OxDataError>;
    fn request_lock(&mut self, holder: Uuid, ttl: Option<Duration>);
    fn request_unlock(&mut self, holder: Uuid);

    fn is_locked(&self) -> bool;
    fn is_locked_by(&self, holder: Uuid) -> bool;
    fn is_expired(&self) -> bool;

    fn transaction_active(&self) -> bool;
    fn begin_transaction(&mut self, holder: Uuid, ttl: Option<Duration>) -> Result<(), OxDataError>;
    fn commit_transaction(&mut self) -> Result<(), OxDataError>;
    fn rollback_transaction(&mut self) -> Result<(), OxDataError>;
}
```

### lock(holder, ttl)

1. Fire `before_lock`.
2. If current lock is `Locked { expires_at: Some(t) }` and `t < now()`: treat as `Unlocked` (expired).
3. If `Unlocked` or `PendingLock(holder)`:
   - Set status to `Locked { holder, expires_at: ttl.map(|d| now() + d) }`.
   - Call `notify_lock_change("locked", holder)` on the persistence driver.
   - Fire `after_lock`. Return `Ok(())`.
4. If `Locked` by a different holder: fire `on_error_lock`, return `Err`.

### unlock(holder)

1. Fire `before_unlock`.
2. If `Locked { holder: h, .. }` and `h == holder`:
   - Set status to `Unlocked`.
   - Call `notify_lock_change("unlocked", holder)`.
   - Fire `after_unlock`. Return `Ok(())`.
3. If locked by a different holder: fire `on_error_unlock`, return `Err`.
4. If already `Unlocked`: fire `after_unlock`, return `Ok(())`.

### force_unlock(admin_token)

Breaks any lock regardless of holder. `admin_token` is validated against a configured
secret — callers without the token receive `Err`.

1. Fire `before_force_unlock`.
2. Validate `admin_token`.
3. Set status to `Unlocked`. Notify driver.
4. Fire `after_force_unlock`. On invalid token: fire `on_error_force_unlock`.

### begin_transaction / commit_transaction / rollback_transaction

Convenience wrappers for multi-container save operations:

- `begin_transaction(holder, ttl)` — calls `lock(holder, ttl)` and marks transaction as
  active in the extension slot.
- `commit_transaction()` — calls `unlock(holder)`, clears transaction state.
- `rollback_transaction()` — calls `discard()` on the GDO (re-loads from store), then
  `unlock(holder)`. Rolls back any in-memory changes.

---

## Transaction Semantics for Multi-Container Saves

`DataObjectManager::save_data_object` checks whether the GDO has an active transaction.
The system follows **standard SQL isolation levels** (e.g., Read Committed) for data consistency.

**With active transaction (locking enabled):**
1. Write to container A.
2. If container B write fails: call `driver_a.delete(location, id)` to undo the A write,
   call `rollback_transaction()` on the GDO, return `Err`.
3. If all writes succeed: call `commit_transaction()`.

**Without active transaction:**
1. Write to each container in sequence.
2. If any write fails: log the error, continue to remaining containers.
3. Return a summary of any failures (does not return `Err` — caller may inspect).

---

## Extension Slot: `"ox.transaction"`

```json
{
  "status": "Locked",
  "holder": "550e8400-e29b-41d4-a716-446655440000",
  "expires_at": "2026-04-23T12:00:00Z",
  "transaction_active": true
}
```

Lock state is advisory when persisted. The authoritative lock is in the backing datastore.
On `hydrate()`, the lock status is refreshed from the persisted value.

---

## notify_lock_change

Calls `PersistenceDriver::notify_lock_status_change(status, gdo_id)` on the active driver
using the GDO's `PersistenceInfo`. No-op if persistence info is not set.

---

## LockableGenericDataObject (legacy)

Kept for backward compatibility only. New code uses `impl Transactable for GenericDataObject`.

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ox_data_object` | `GenericDataObject`, `CallbackManager` |
| `ox_persistence` | `PERSISTENCE_DRIVER_REGISTRY`, `PersistenceInfo`, `discard()` |
| `ox_callback_manager` | Event dispatch |
| `uuid` | Lock holder identity |
| `chrono` | Lock expiry timestamps |
