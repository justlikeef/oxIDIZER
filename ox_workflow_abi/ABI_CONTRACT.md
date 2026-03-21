# oxWorkflow Plugin ABI Contract

## Versioning
All plugins and host functions conform to the `OX_WORKFLOW_ABI_VERSION` defined in this repository. Plugins must verify the host ABI version during initialization.

## Memory Protocol
- **Flow Runner Ownership**: The host fully owns task memory via `Arc<parking_lot::RwLock<TaskState>>`.
- **Stateless Plugins**: The plugin context (`plugin_config_ctx`) returned from `ox_plugin_init` must be strictly immutable. It is shared across concurrent task executions. Mutable task state must be retrieved and updated exclusively via the task context (`task_ctx`).
- **Data Access**: Plugins interact with task state strictly using `CoreHostApi::get_field` and `CoreHostApi::set_field`. No raw memory pointers to internal state structures are ever provided across the FFI boundary.

### FFI Overhead Edge Case
For typical operations, `get_field` and `set_field` are optimal. However, when processing extremely large, deeply nested datasets (e.g., parsing a 10,000-line network configuration switch), making hundreds of successive `set_field` calls incurs a cumulative FFI boundary crossing penalty. In these edge cases, plugins should serialize the structured dataset into a single string (e.g., JSON), manipulate it internally, and write it back as a single field value.

## Fork Field-Disjointness (Last Writer Wins)
When a task is `FORK`ed into multiple concurrent flows, the child runners share the same synchronized `TaskState`. The engine employs a "Last Writer Wins" model for data integrity. 
If parallel branches write to identical field keys simultaneously, the final write overwrites previous entries. Plugin developers and workflow designers must ensure disjoint field usage (e.g., prefixing fields with branch names like `branch_A_result`) when aggregating parallel processing results.

## Flow Control Codes
Plugins return a `FlowControl` struct containing a code and an optional payload payload.
- `CONTINUE`
- `END`
- `ERROR`
- `JUMP`
- `SKIP`
- `SUSPEND`
- `YIELD`

### Embedded Mode `YIELD`
When `oxWorkflow` is running embedded without the Tokio scheduler loop (e.g., inline within the `oxWebservice` synchronous pipeline), `YIELD` signals are silently interpreted as `CONTINUE`.

## Out-of-Process Isolation (Phase 3)
*Note on Crashes:* C plugins that execute a segmentation fault (`SIGSEGV`) or call `abort()` cannot be caught by Rust's `catch_unwind` and will bring down the entire host process. Untrusted or unstable C plugins are executed out-of-process via IPC starting in Phase 3.
