# Refactoring `crates/data` Error Handling and Transaction Expansion

The goal is to complete the refactoring of `crates/data` based on the progress recorded in `development_status.md` and the design from `OXDATASPEC.md`.

> [!WARNING]
> **Circular Dependency Issue Identified**
> `development_status.md` indicates that `OxDataError` was moved to `ox_data_object` and that `ox_type_converter` should be updated to use it. However, `ox_data_object` depends on `ox_type_converter`. If `ox_type_converter` depends on `ox_data_object` to access `OxDataError`, this creates an illegal circular dependency in Cargo.

## Open Questions

> [!IMPORTANT]
> **How should we resolve the circular dependency with `OxDataError`?**
> - **Option A (Recommended):** Create a new, lightweight crate `ox_data_error` (or `ox_core_types`) that contains **only** `OxDataError`. Both `ox_type_converter` and `ox_data_object` (and all other data crates) would depend on it. This solves the circular dependency and provides a truly central error type for the `data` ecosystem.
> - **Option B:** Move `OxDataError` back to `ox_type_converter` (since it sits at the bottom of the dependency tree) and add the `CallbackError` variant there.
> - **Option C:** Let `ox_type_converter` return its own `ConversionError`, and have `ox_data_object` wrap it into `OxDataError::ConversionError(e.to_string())`.
> 
> *Please let me know which option you prefer! (Option A is standard practice for large workspaces).*

## Proposed Changes

### `ox_data_error` (Assuming Option A is chosen)
#### [NEW] `crates/data/ox_data_error`
- Initialize a new crate to hold `OxDataError`.
- Move the `OxDataError` definition from `ox_data_object/src/error.rs` here.

---

### `ox_type_converter`
#### [MODIFY] `crates/data/ox_type_converter/Cargo.toml`
- Add dependency on `ox_data_error`.

#### [MODIFY] `crates/data/ox_type_converter/src/lib.rs` and `src/error.rs`
- Remove the duplicated `OxDataError` definition and use the one from `ox_data_error` (or handle based on the option chosen above).

---

### `ox_locking` -> `ox_transaction`
#### [MODIFY] `crates/data/ox_locking/Cargo.toml` -> `crates/data/ox_transaction/Cargo.toml`
- Rename the package `name = "ox_transaction"`.
- Update all internal paths to match.

#### [MODIFY] `crates/data/ox_locking/src/*` -> `crates/data/ox_transaction/src/*`
- Rename the crate folder on disk from `ox_locking` to `ox_transaction`.
- Refactor the crate to propagate `OxDataError` instead of using `Option` or `String`.
- Expand capabilities to handle generic transaction boundaries, rollback, and commit, interacting with the Transaction Manager as specified in `OXDATASPEC.md`.

---

### Data Crates Refactoring (`ox_persistence`, `ox_data_object_manager`, `ox_data_broker`)
#### [MODIFY] Various `Cargo.toml`
- Update the dependencies to point to the new `ox_transaction` and/or `ox_data_error`.

#### [MODIFY] Various `src/lib.rs` and modules
- Refactor all error handling in these crates. Replace methods returning `Result<T, String>` or swallowing errors with `Result<T, OxDataError>`.

## Verification Plan

### Automated Tests
- Run `cargo check --workspace` to ensure the circular dependency is resolved and that the renaming of `ox_locking` didn't break external consumers.
- Run `cargo test -p ox_type_converter -p ox_data_object -p ox_transaction` to verify core data logic.

### Manual Verification
- Review the `development_status.md` and update it to reflect the resolution of the `OxDataError` relocation and `ox_transaction` rename.
