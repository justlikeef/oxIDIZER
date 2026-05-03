## Goal
Refactor `crates/data` to use a unified `OxDataError` and implement expanded specifications
for `ox_transaction`, `ox_introspection`, and `ox_data_broker`.

## Constraints & Preferences
- Use `OxDataError` instead of `String` for error returns across all data crates.
- **Error Propagation**: Never swallow errors; pass them upstream using `Result` until handled.
- Track logic, todos, and progress in real-time within files in the `crates/data` directory.

## Progress

### Done
- `ox_data_error` standalone crate created; `OxDataError` variants:
  `ConversionError`, `TypeMismatch`, `RegistryError`, `InternalError`, `DriverError`,
  `ValidationError`, `TransactionError`, `CallbackError`.
- `ox_type_converter`, `ox_data_object`, `ox_persistence`, SQL/GDO drivers migrated to `OxDataError`.
- `ox_callback_manager` uses `CallbackError`; `OxDataError::CallbackError` wraps it.
- `GenericDataObject` methods return `Result<..., OxDataError>`.
- `ValueType` expanded: `Text`, `BigInt`, `Json`, `Timestamp` added for SQL-oriented schemas.
- `DataObjectSchema` / `FieldDescriptor` builder API in `ox_data_object_manager::dictionary`.
- `ox_data_object_dictionary_manager` handles new error types.
- `ox_persistence_driver_manager` — JSON error responses now use `e.to_string()` (resolved
  `OxDataError: serde::Serialize` compilation error).
- Unused imports in `ox_data_object` and `ox_data_object_manager` cleaned up.
- `set_attribute_value` result properly propagated with `?` in `load_data_object`.
- Mock driver `fetch` in tests fixed to return serialized row data.
- `ox_locking` renamed to `ox_transaction` and fully expanded:
  - `LockStatus::Locked { holder, expires_at }` with TTL expiry.
  - `Transactable` trait implemented directly on `GenericDataObject` (state in `"ox.transaction"` extension slot).
  - Full lock/unlock/force_unlock/request_lock/request_unlock API.
  - Transaction lifecycle: `begin_transaction`, `commit_transaction`, `rollback_transaction`.
  - `after_commit` / `after_rollback` callback events.
  - Legacy `TransactableGenericDataObject` wrapper kept for backward compat.
  - 7 unit tests passing.
- `ox_data_broker` cleanup:
  - Already uses `OxDataError`.
  - Added `DeleteFn` type and `ox_driver_delete` ABI support (optional symbol).
  - Added `DELETE /data/{driver}/record/{id}` route.
  - Removed dead-code byte helper functions.

### In Progress
- (none)

### Blocked
- `ox_cert_core/src/store.rs` uses deprecated API (`save_object`, `get_object`,
  `serde::Deserialize` on `AttributeValue`, wrong `GenericDataObject::new` signature).
  **Not in scope for current session** — fix in dedicated cert session.

## Key Decisions & Observations
- **Standalone Error Crate**: `OxDataError` lives in `ox_data_error` (not `ox_data_object`)
  to prevent circular deps.
- **`Transactable` on GDO directly**: Extension slot `"ox.transaction"` carries lock/tx state.
  No wrapper struct needed in new code.
- **`ox_driver_delete` is optional**: Brokers load it with `lib.get(...).ok()` so older
  drivers without the symbol still load correctly.
- **Query engine fetch format**: `PersistenceDriver::fetch` returns `Vec<String>` where each
  element is a JSON-serialized `HashMap<String, (String, ValueType, HashMap<String, String>)>`.

## Next Steps
- Wire `notify_lock_change` to persistence driver in `ox_transaction`.
- `ox_data_broker`: location resolution from datasource config (replace hardcoded `"data.csv"`).
- `ox_data_broker`: WebSocket / LISTENER_REGISTRY change notification system.
- Implement `ox_introspection` crate.
- Fix `ox_cert_core` store API usage (separate session).

## Critical Context
- **`OxDataError` variants**: `ConversionError`, `TypeMismatch`, `RegistryError`,
  `InternalError`, `DriverError`, `ValidationError`, `TransactionError`, `CallbackError`.
- **`Transactable` admin token**: `force_unlock` validates against `OX_FORCE_UNLOCK_TOKEN`
  env var (empty = allow any token in dev/test).
- **Test count**: 59 tests pass across core data crates as of this session.

## Relevant Files
- `crates/data/ox_data_error/src/lib.rs` — central error type.
- `crates/data/ox_transaction/src/lib.rs` — full Transactable implementation.
- `crates/data/ox_data_broker/src/lib.rs` — REST broker plugin.
- `crates/data/ox_data_object/ox_data_object_manager/src/lib.rs` — DataObjectManager.
- `crates/data/ox_data_object/ox_data_object_manager/src/query.rs` — QueryEngine.
- `crates/data/ox_data_object/ox_data_object_manager/src/tests.rs` — integration tests.
- `crates/data/ox_persistence_driver_manager/src/lib.rs` — persistence driver plugin.
