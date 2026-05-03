# Data Module Refactor Tasks

## Unified Error Handling
- `[x]` Create `ox_data_error` crate.
- `[x]` Update `ox_type_converter` to use `ox_data_error`.
- `[x]` Update `ox_data_object` to use `ox_data_error`.
- `[x]` Update `ox_persistence` API to use `OxDataError`.
- `[x]` Refactor SQL and GDO Relational drivers to use `OxDataError`.
- `[x]` Fix `ox_persistence_driver_manager` (use `e.to_string()` in JSON error responses).

## Persistence API Stabilization
- `[x]` Add `Text`, `BigInt`, `Json`, `Timestamp` variants to `ValueType`.
- `[x]` Implement `DataObjectSchema` and `FieldDescriptor` builder APIs in `ox_data_object_manager`.
- `[x]` Fix non-exhaustive matches in `TypeConverter`.
- `[x]` Fix mock driver `fetch` to return serialized rows (not just keys).

## Infrastructure & Subsystems
- `[x]` Update `ox_data_object_dictionary_manager` to handle new error types.
- `[x]` Rename `ox_locking` to `ox_transaction` and expand functionality (full Transactable trait, TTL locks, transaction lifecycle on GenericDataObject).
- `[x]` `ox_data_broker` already uses `OxDataError`; added DELETE route, removed dead-code warnings.

## Code Quality
- `[x]` Remove unused imports (`CallbackError`, `EventType`, `std::any::Any`) from data crates.
- `[x]` Propagate `set_attribute_value` error with `?` in `load_data_object`.

## Verification
- `[x]` All data crates compile with zero errors and zero warnings.
- `[x]` `cargo test` passes: 59 tests across ox_data_error, ox_type_converter, ox_data_object, ox_data_object_manager, ox_persistence, ox_transaction, ox_validation.

## Remaining / Future Work
- `[ ]` ox_transaction: wire `notify_lock_change` to persistence driver when PersistenceInfo is set.
- `[ ]` ox_data_broker: implement location resolution from datasource config (replace hardcoded "data.csv").
- `[ ]` ox_data_broker: implement WebSocket / LISTENER_REGISTRY change notification.
- `[ ]` ox_data_broker: implement `ox_transaction` awareness in `after_set` callback.
- `[ ]` Implement `ox_introspection` crate (ObjectSchema, FieldDescriptor, forms integration).
- `[ ]` Implement `ox_validation` full rule set.
- `[ ]` Fix `ox_cert_core/src/store.rs` to use correct GDO API (separate session).
- `[ ]` Run `cargo test --workspace` once cert crate is fixed.
