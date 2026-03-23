# Replace ox_pipeline with ox_workflow & Manual TcpListener

This plan details the full global migration from the `ox_pipeline` execution model to the new `ox_workflow` engine across the entire `oxIDIZER` repository. This change applies to all modules, crates, plugins, and internal code currently utilizing the legacy `ox_pipeline` mechanism. Additionally, it integrates the architectural goal of utilizing a manual `tokio::net::TcpListener` accept loop.

## Architectural Decisions

This migration incorporates the following finalized architectural decisions which resolve previously open gaps:

1. **Routing Architecture:** `ox_pipeline_router` will be reimagined as a `StageRunner` rather than a `FlowRunner` or host-side wrapper. The router will evaluate `request.path` (and other pre-injected fields) from `TaskState`, compute the appropriate route, and set a `route.target` field. Subsequent stages in the flow will be conditionally executed or skipped via `FlowControl` evaluation based on this `route.target` value, ensuring only the correct plugin chain runs.
2. **Virtual Field Resolution:** System-wide variables and HTTP request fields (`request.method`, `request.path`, etc.) will be pre-injected into the `TaskState` at flow creation time by the host.
3. **Manual TcpListener:** A manual `tokio::net::TcpListener` accept loop will be implemented. This initial pass will establish the foundational loop and filter connections by IPs defined in the configuration. Valid connections will be properly fed into the `axum` service framework (e.g., via `hyper` connection builders or a wrapper that `axum::serve` can consume) to preserve graceful shutdown semantics, rather than using detached `tokio::spawn` calls.
4. **Crate Duplication (`ox_event_bus` & `ox_messaging_mqtt`):** The `oxIDIZER` workspace versions of these crates will be entirely removed in favor of the standardized versions shipped within the `oxWorkflow` repository to eliminate duplication.
5. **FlowControl Naming Collision:** Both `ox_pipeline_plugin` and `ox_workflow_abi` export a `FlowControl` type. During the transition, all migrated code must fully qualify or alias the `ox_workflow_abi::FlowControl` type at import sites to prevent compilation errors until the legacy type is fully purged. Note: Remember to remove these fully qualified paths and aliases during the final cleanup phase once the conflict is eliminated.

## Migration Order

To prevent breaking the workspace mid-migration, the rollout will follow a strict ordering:
1. **Infra Layer:** Update `ox_webservice_api` and create `ox_webservice_router`. Delete duplicated infrastructure (`ox_event_bus`, `ox_messaging_mqtt`).
2. **Host Layer:** Refactor `ox_webservice` to rip out `ox_pipeline` and stand up the `ox_workflow` `PluginRegistry`, `FlowRunner`/`StageRunner` logic, and the custom `TcpListener`.
3. **Plugin Layer:** Migrate the remaining ~25 legacy crates individually over to the new ABI.
4. **Cleanup:** Finally, delete the `ox_pipeline` crate.

## Proposed Changes

### Memory Architecture Shift
A significant architectural change in this migration is moving away from `bumpalo` and arena memory handling. 
*   **Current State:** `ox_pipeline` relies heavily on `bumpalo::Bump` arenas passed across the C-ABI (e.g. `alloc_fn(arena, ...)`), requiring plugins to manage raw pointers and arena lifecycles.
*   **New State:** `oxWorkflow` removes arena memory entirely across the ABI. All memory is managed safely. Host access via `CoreHostApi` avoids raw pointers, and serialization to `TaskState` handles memory boundary passing, eliminating the need for `bumpalo` in all plugins.

### ox_pipeline
Remove the legacy pipeline crate, as memory architecture is shifting away from `bumpalo` and arenas entirely.
#### [MODIFY] `/var/repos/oxIDIZER/Cargo.toml`
- Remove `ox_pipeline` from the `[workspace.members]` list.
#### [DELETE] `/var/repos/oxIDIZER/ox_pipeline`

### ox_webservice_router
Create the standalone router encapsulation as a `StageRunner` compatible plugin.
#### [NEW] `/var/repos/oxIDIZER/ox_webservice_router/Cargo.toml`
- Add to the root workspace members.
#### [NEW] `/var/repos/oxIDIZER/ox_webservice_router/src/lib.rs`

### ox_webservice_api
Remove legacy `PipelineState`, memory struct arguments, and `ModuleInterface`. Re-export `ox_workflow_abi` core types (`CoreHostApi`, `FlowControl`).
#### [MODIFY] `/var/repos/oxIDIZER/ox_webservice_api/Cargo.toml`
#### [MODIFY] `/var/repos/oxIDIZER/ox_webservice_api/src/lib.rs`

### ox_webservice
Replace legacy execution logic with `ox_workflow_executor::PluginRegistry` and `FlowRunner` / `StageRunner` integration. Add a manual `tokio::net::TcpListener` accept loop.
#### [MODIFY] `/var/repos/oxIDIZER/ox_webservice/Cargo.toml`
- Remove `ox_pipeline`. Add `ox_workflow_executor`, `ox_workflow_abi`, `ox_workflow_core` by path pointing to `/var/repos/oxWorkflow/*`.
#### [MODIFY] `/var/repos/oxIDIZER/ox_webservice/src/pipeline.rs`
- Replace `ox_pipeline::Pipeline` with `ox_workflow` executor logic. Create the `CoreHostApi` integrations without `bumpalo`.
- **Virtual Field Resolution:** Pre-inject all HTTP request fields (`request.method`, `request.path` etc.) into `TaskState` at flow creation time in the host.
#### [MODIFY] `/var/repos/oxIDIZER/ox_webservice/src/main.rs`
- Implement a manual `tokio::net::TcpListener` accept loop for connection-level filtering before `axum` processing. Link accepted and filtered connections explicitly to the axum service router to retain proper shutdown capabilities.

### All Legacy Plugins and Crates
Migrate all the following crates to the new `ox_workflow` ABI:
- `ox_webservice_errorhandler_jinja2`
- `ox_webservice_errorhandler_json`
- `ox_webservice_template_jinja2`
- `ox_webservice_rewrite`
- `ox_webservice_redirect`
- `ox_webservice_ping`
- `ox_webservice_stream`
- `ox_webservice_status`
- `ox_webservice_restore_ip`
- `ox_webservice_wsgi`
- `ox_webservice_forwarded_for`
- `ox_webservice_vary_header`
- `ox_webservice_test_utils`
- `ox_package_manager`
- `ox_forms/ox_forms_server`, `ox_forms_api`, `ox_forms_std_renderers`, `ox_forms_client`
- `ox_auth_ip`
- `ox_data_broker`
- `ox_data_object/ox_data_object_dictionary_manager`
- `ox_persistence_datasource_manager`, `ox_persistence_driver_installer`, `ox_persistence_driver_manager`
- `ox_persistence/ox_persistence_dictionary_manager`
- `ox_content`
- `ox_server_info`

*(Note 1: `ox_locking` is explicitly out of scope per user specification)*
*(Note 2: `ox_event_bus` and `ox_messaging_client/ox_messaging_mqtt` instances within oxIDIZER will be DELETED and replaced by the versions inside the `oxWorkflow` repository).*

**Migration Steps per Plugin:**
- Remove all `bumpalo` dependencies.
- Refactor C-ABI exports to `ox_plugin_init` and `ox_plugin_process`.
- Replace `ctx.get("...")` with `(api.get_field)(task_ctx, "...")`.
- Use the standard `log` crate natively.
- Provide `ox_plugin_error` and `ox_plugin_destroy`.
- Carefully manage the `FlowControl` naming collision between libraries.

#### [MODIFY] `Cargo.toml` for each crate
Update dependencies to point to the new `ox_webservice_api`.
#### [MODIFY] `src/lib.rs` for each crate
Implement the `ox_workflow_core` execution and C-ABI requirements.

## Verification Plan

### Automated Tests
1. **Compilation:** Run `cargo build --workspace` to ensure successful compilation without warnings.
   *   **Warning Strictness Policy**: All code must compile and run absolutely without warnings. Warnings should **not** simply be silenced by adding the `_` prefix or using silencing attributes like `#[allow(dead_code)]`. Leave warnings visible until properly resolved.
2. **Functional Tests:** Run the existing functional bash scripts across the repo to ensure plugins integrate correctly. Do not use the `-b` or `-r` flags for `run_functional_tests.sh` unless code changes necessitate a build.

### Manual Verification
1. Launch the `ox_webservice` binary manually.
2. Validate that the custom `tokio::net::TcpListener` correctly accepts incoming connections and gracefully routes them to axum without degrading shutdown behavior.
3. Issue an HTTP request (e.g. `curl -v http://localhost:.../missing_page`) to verify the workflow engine correctly handles the request and plugins return expected responses.
