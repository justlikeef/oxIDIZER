# Directory Restructure Design

**Date:** 2026-03-22
**Status:** Approved
**Scope:** Reorganize the oxIDIZER Rust workspace from a flat root layout into a `crates/<domain>/` hierarchy, clean up root artifacts, and resolve structural inconsistencies.

---

## Problem Statement

The current workspace root contains 57+ crate directories mixed with configuration, logs, build artifacts, and documentation. Related crate families are grouped inconsistently — some are nested (e.g., `ox_forms/`), some are flat at root (e.g., all `ox_webservice_*` plugins), and `ox_cc/` uses its own subdirectory convention. Root-level log files, PID files, and AI working files pollute the repository. One empty directory (`ox_pipeline/`) exists along with a stale test artifact directory at `ox_messaging_client/ox_messaging_mqtt/` (contains only `systems_tests/`, not a crate). Additionally, 42 artifact files (logs, PIDs) are tracked in git, and four `ox_cc` plugin crates contain machine-specific absolute paths.

---

## Goals

- Organize all crates under `crates/<domain>/<crate_name>/`
- Establish seven domains: `webservice`, `messaging`, `workflow`, `forms`, `cc`, `data`, `util`
- Clean root of all artifacts, logs, and temp files; remove all 42 tracked artifact files from git
- Resolve `ox_cc/` by dissolving it into `crates/cc/` (it is not a sub-workspace — its crates are referenced directly by the root `Cargo.toml`)
- Delete the stale `ox_messaging_client/ox_messaging_mqtt/` test artifact directory (not a crate)
- Delete the empty `ox_pipeline/` directory
- Promote 9 currently non-workspace-member crates to full workspace members (see below)
- Update `.gitignore` to prevent artifact recurrence

---

## Target Directory Structure

```
/
├── Cargo.toml
├── Cargo.lock
├── LICENSE
├── README.md
├── crates/
│   ├── cc/
│   │   ├── ox_cc_admin_plugin/
│   │   ├── ox_cc_broker_plugin/
│   │   ├── ox_cc_client/
│   │   ├── ox_cc_common/
│   │   ├── ox_cc_executor/
│   │   ├── ox_cc_keygen/
│   │   ├── ox_cc_manifest_plugin/
│   │   └── ox_cc_report_plugin/
│   ├── data/
│   │   ├── ox_data_broker/
│   │   ├── ox_data_object/           # retains ox_data_object_dictionary_manager/ and ox_data_object_manager/ sub-crates (not workspace members, not promoted)
│   │   ├── ox_locking/
│   │   ├── ox_persistence/           # retains ox_persistence_api/, ox_persistence_dictionary_manager/, drivers/ sub-crates (not workspace members, not promoted)
│   │   ├── ox_persistence_datasource_manager/
│   │   ├── ox_persistence_driver_installer/
│   │   ├── ox_persistence_driver_manager/
│   │   ├── ox_persistence_gdo_relational/
│   │   └── ox_type_converter/
│   ├── forms/
│   │   ├── ox_forms/
│   │   ├── ox_forms_api/
│   │   ├── ox_forms_client/
│   │   ├── ox_forms_server/
│   │   └── ox_forms_std_renderers/
│   ├── messaging/
│   │   ├── ox_event_bus/             # retains ox_event_bus_mqtt/ sub-crate (not workspace member, not promoted)
│   │   ├── ox_messaging_client/
│   │   └── ox_messaging_mqtt/
│   ├── util/
│   │   ├── ox_callback_manager/
│   │   ├── ox_fileproc/
│   │   └── ox_package_manager/
│   ├── webservice/
│   │   ├── ox_auth_ip/
│   │   ├── ox_webservice/
│   │   ├── ox_webservice_api/
│   │   ├── ox_webservice_errorhandler_jinja2/
│   │   ├── ox_webservice_errorhandler_json/
│   │   ├── ox_webservice_forwarded_for/
│   │   ├── ox_webservice_ping/
│   │   ├── ox_webservice_redirect/
│   │   ├── ox_webservice_restore_ip/
│   │   ├── ox_webservice_rewrite/
│   │   ├── ox_webservice_router/
│   │   ├── ox_webservice_status/
│   │   ├── ox_webservice_stream/
│   │   ├── ox_webservice_template_jinja2/
│   │   ├── ox_webservice_test_utils/
│   │   ├── ox_webservice_vary_header/
│   │   └── ox_webservice_wsgi/
│   └── workflow/
│       ├── ox_workflow_abi/
│       ├── ox_workflow_api/
│       ├── ox_workflow_config/
│       ├── ox_workflow_core/
│       ├── ox_workflow_executor/
│       ├── ox_workflow_scheduler/
│       └── ox_workflow_storage/
├── conf/                        # runtime configuration (unchanged)
├── content/                     # web content assets (unchanged)
├── docs/                        # documentation
├── sample_projects/             # unchanged
├── scripts/                     # build and dev scripts
└── tests/                       # renamed from systems_tests/
```

---

## Domain Assignments

| Domain | Crates |
|--------|--------|
| **webservice** | ox_webservice, ox_webservice_api, ox_webservice_router, ox_webservice_test_utils, ox_auth_ip, ox_webservice_forwarded_for, ox_webservice_restore_ip, ox_webservice_errorhandler_jinja2, ox_webservice_errorhandler_json, ox_webservice_redirect, ox_webservice_rewrite, ox_webservice_stream, ox_webservice_template_jinja2, ox_webservice_ping, ox_webservice_status, ox_webservice_vary_header, ox_webservice_wsgi |
| **messaging** | ox_event_bus, ox_messaging_client, ox_messaging_mqtt |
| **workflow** | ox_workflow_abi, ox_workflow_api, ox_workflow_config, ox_workflow_core, ox_workflow_executor, ox_workflow_scheduler, ox_workflow_storage |
| **forms** | ox_forms, ox_forms_api, ox_forms_client, ox_forms_server, ox_forms_std_renderers |
| **cc** | ox_cc_admin_plugin, ox_cc_broker_plugin, ox_cc_client, ox_cc_common, ox_cc_executor, ox_cc_keygen, ox_cc_manifest_plugin, ox_cc_report_plugin |
| **data** | ox_data_broker, ox_data_object, ox_locking, ox_persistence, ox_persistence_datasource_manager, ox_persistence_driver_installer, ox_persistence_driver_manager, ox_persistence_gdo_relational, ox_type_converter |
| **util** | ox_callback_manager, ox_fileproc, ox_package_manager |

---

## Non-Member Crates Being Promoted to Workspace Members

The following 9 crates currently exist on disk and are used via `path =` dependencies but are **not** listed in the root `Cargo.toml` workspace members. This restructure promotes them to full workspace members, meaning `cargo build` and `cargo test` will compile and test them as part of the workspace.

| Crate | Current Location | Domain |
|-------|-----------------|--------|
| `ox_data_object` | `ox_data_object/` (root) | data |
| `ox_locking` | `ox_locking/` (root) | data |
| `ox_persistence_gdo_relational` | `ox_persistence_gdo_relational/` (root) | data |
| `ox_callback_manager` | `ox_callback_manager/` (root) | util |
| `ox_forms` | `ox_forms/` (root, dual-role crate+container) | forms |
| `ox_forms_api` | `ox_forms/ox_forms_api/` (nested) | forms |
| `ox_forms_client` | `ox_forms/ox_forms_client/` (nested) | forms |
| `ox_forms_server` | `ox_forms/ox_forms_server/` (nested) | forms |
| `ox_forms_std_renderers` | `ox_forms/ox_forms_std_renderers/` (nested) | forms |

Note: `ox_forms_api`, `ox_forms_client`, `ox_forms_server`, and `ox_forms_std_renderers` are currently nested inside `ox_forms/` — they have no root-level counterpart. The migration step for the `forms` domain must handle them starting from their nested locations, not from the root.

---

## Workspace Cargo.toml Structure

```toml
[workspace]
resolver = "2"
members = [
    # util (migrated first — many domains depend on it)
    "crates/util/ox_callback_manager",
    "crates/util/ox_fileproc",
    "crates/util/ox_package_manager",

    # workflow
    "crates/workflow/ox_workflow_abi",
    "crates/workflow/ox_workflow_core",
    "crates/workflow/ox_workflow_config",
    "crates/workflow/ox_workflow_executor",
    "crates/workflow/ox_workflow_storage",
    "crates/workflow/ox_workflow_api",
    "crates/workflow/ox_workflow_scheduler",

    # messaging
    "crates/messaging/ox_event_bus",
    "crates/messaging/ox_messaging_client",
    "crates/messaging/ox_messaging_mqtt",

    # webservice
    "crates/webservice/ox_webservice",
    "crates/webservice/ox_webservice_api",
    "crates/webservice/ox_webservice_router",
    "crates/webservice/ox_webservice_test_utils",
    "crates/webservice/ox_auth_ip",
    "crates/webservice/ox_webservice_forwarded_for",
    "crates/webservice/ox_webservice_restore_ip",
    "crates/webservice/ox_webservice_errorhandler_jinja2",
    "crates/webservice/ox_webservice_errorhandler_json",
    "crates/webservice/ox_webservice_redirect",
    "crates/webservice/ox_webservice_rewrite",
    "crates/webservice/ox_webservice_stream",
    "crates/webservice/ox_webservice_template_jinja2",
    "crates/webservice/ox_webservice_ping",
    "crates/webservice/ox_webservice_status",
    "crates/webservice/ox_webservice_vary_header",
    "crates/webservice/ox_webservice_wsgi",

    # forms
    "crates/forms/ox_forms",
    "crates/forms/ox_forms_api",
    "crates/forms/ox_forms_client",
    "crates/forms/ox_forms_server",
    "crates/forms/ox_forms_std_renderers",

    # data
    "crates/data/ox_data_object",
    "crates/data/ox_data_broker",
    "crates/data/ox_locking",
    "crates/data/ox_type_converter",
    "crates/data/ox_persistence",
    "crates/data/ox_persistence_datasource_manager",
    "crates/data/ox_persistence_driver_installer",
    "crates/data/ox_persistence_driver_manager",
    "crates/data/ox_persistence_gdo_relational",

    # cc
    "crates/cc/ox_cc_common",
    "crates/cc/ox_cc_broker_plugin",
    "crates/cc/ox_cc_manifest_plugin",
    "crates/cc/ox_cc_report_plugin",
    "crates/cc/ox_cc_admin_plugin",
    "crates/cc/ox_cc_client",
    "crates/cc/ox_cc_keygen",
    "crates/cc/ox_cc_executor",
]
```

---

## Special Cases

### ox_cc Directory

`ox_cc/` is **not** a sub-workspace. Its crates are registered directly in the root workspace `Cargo.toml` via paths like `"ox_cc/crates/ox_cc_common"`. There is no `[workspace]` Cargo.toml to delete. The migration only updates the root `Cargo.toml` member paths.

Non-crate contents of `ox_cc/` and their disposition:

| Item | Disposition |
|------|-------------|
| `DESIGN.md`, `IMPLEMENTATION_PLAN.md`, `NEW_FEATURE.md`, `PROJECT_INFO.md`, `WORK_IN_PROGRESS.md` | Move to `docs/cc/` |
| `docs/superpowers/` | Merge into root `docs/superpowers/` |
| `conf/` (example yamls, module yamls, service file) | Move to `conf/cc/` |
| `edit_docs.py`, `edit_roles.py` | Move to `scripts/` |
| `.claude/settings.json` | Review and merge relevant settings into root `.claude/settings.json`, then delete |
| `target/` | Delete (build artifact, not tracked) |

### Hardcoded Paths Requiring Fixes

**`../../oxIDIZER/` relative paths (three workflow crates):**

Three workflow crates contain a broken ancestor-traversal path that currently works by accident from the root but must be fixed as part of their migration in step 2:

- `ox_workflow_config/Cargo.toml`: `ox_fileproc = { path = "../../oxIDIZER/ox_fileproc" }`
- `ox_workflow_core/Cargo.toml`: `ox_fileproc = { path = "../../oxIDIZER/ox_fileproc" }`
- `ox_workflow_executor/Cargo.toml`: `ox_fileproc = { path = "../../oxIDIZER/ox_fileproc" }`

After migration these must become `path = "../../util/ox_fileproc"` (relative from `crates/workflow/<crate>/`).

**Machine-absolute paths in four `ox_cc` plugin crates:**

Four `ox_cc` plugin crates contain machine-specific absolute paths that will break after `ox_workflow_abi` is moved in step 2. These must be fixed as part of step 7 (cc migration):

- `ox_cc/crates/ox_cc_broker_plugin/Cargo.toml`
- `ox_cc/crates/ox_cc_admin_plugin/Cargo.toml`
- `ox_cc/crates/ox_cc_manifest_plugin/Cargo.toml`
- `ox_cc/crates/ox_cc_report_plugin/Cargo.toml`

All four contain: `ox_workflow_abi = { path = "/var/repos/oxIDIZER/ox_workflow_abi" }`

After migration this must become `path = "../../workflow/ox_workflow_abi"` (relative from `crates/cc/<crate>/`).

### ox_messaging_client/ox_messaging_mqtt/ Directory

`ox_messaging_client/ox_messaging_mqtt/` is **not** a crate. It contains only a `systems_tests/` subdirectory with no `Cargo.toml`. It is a stale test artifact directory. It should be deleted during the messaging migration step (step 3). The root-level `ox_messaging_mqtt/` is the sole canonical crate and moves to `crates/messaging/ox_messaging_mqtt/`.

### ox_forms Dual Role

`ox_forms/` currently acts as both a crate (has `src/lib.rs`) and a family container (sub-crates are nested inside it). During the forms migration:
- `ox_forms/` itself moves to `crates/forms/ox_forms/`
- The four nested sub-crates (`ox_forms_api/`, `ox_forms_client/`, `ox_forms_server/`, `ox_forms_std_renderers/`) are extracted from inside `ox_forms/` and placed as siblings at `crates/forms/`

### Non-Promoted Nested Sub-Crates

The following nested sub-crates are **not** promoted to workspace members. They remain nested within their parent crate and are used via `path =` dependencies only:

- `ox_data_object/ox_data_object_dictionary_manager/`
- `ox_data_object/ox_data_object_manager/`
- `ox_persistence/ox_persistence_api/`
- `ox_persistence/ox_persistence_dictionary_manager/`
- `ox_persistence/drivers/` (all DB and file drivers)
- `ox_event_bus/ox_event_bus_mqtt/` — referenced by `ox_messaging_client` via `path = "../ox_event_bus/ox_event_bus_mqtt"`; after migration this path becomes `../ox_event_bus/ox_event_bus_mqtt` (unchanged since it stays nested inside `ox_event_bus`)

---

## Migration Order

The order respects the actual cross-domain dependency graph. Steps run sequentially; each step moves crates, updates the root `Cargo.toml` members, updates all `path =` references within the moved crates (pointing to root for not-yet-migrated deps, `crates/<domain>/` for already-migrated ones), and verifies the build.

| Step | Domain | Key cross-domain path deps to update | Build Verification |
|------|--------|--------------------------------------|--------------------|
| 1 | `util` | `ox_package_manager` → `ox_webservice_api` (still at root), `ox_workflow_abi` (still at root) | `cargo build -p ox_fileproc -p ox_callback_manager -p ox_package_manager` |
| 2 | `workflow` | Fix hardcoded `../../oxIDIZER/ox_fileproc` → `../../util/ox_fileproc` in 3 crates; `ox_workflow_scheduler` and `ox_workflow_api` → `ox_event_bus` (still at root) | `cargo build -p ox_workflow_abi -p ox_workflow_core -p ox_workflow_scheduler` |
| 3 | `messaging` | Delete `ox_messaging_client/ox_messaging_mqtt/` stale directory; update `ox_workflow_scheduler` and `ox_workflow_api` paths to `../../messaging/ox_event_bus` | `cargo build -p ox_event_bus -p ox_messaging_client -p ox_messaging_mqtt` |
| 4 | `webservice` | All 17 webservice crates that reference `ox_workflow_abi` or `ox_fileproc` must update those paths to `../../workflow/ox_workflow_abi` and `../../util/ox_fileproc`. Update `ox_package_manager` path to `../../webservice/ox_webservice_api` and `../../workflow/ox_workflow_abi` | `cargo build -p ox_webservice -p ox_webservice_api -p ox_auth_ip` |
| 5 | `forms` | Extract `ox_forms_api/`, `ox_forms_client/`, `ox_forms_server/`, `ox_forms_std_renderers/` from inside `ox_forms/` to `crates/forms/`; promote all 5 to workspace members | `cargo build -p ox_forms -p ox_forms_server -p ox_forms_client` |
| 6 | `data` | `ox_data_broker` → `ox_webservice_api` now at `../../webservice/`; `ox_persistence_datasource_manager` → `ox_forms_api` now at `../../forms/` | `cargo build -p ox_data_object -p ox_persistence -p ox_persistence_datasource_manager` |
| 7 | `cc` | Move 8 crates from `ox_cc/crates/` to `crates/cc/`; move non-crate `ox_cc/` contents per Special Cases table; fix absolute `path = "/var/repos/oxIDIZER/ox_workflow_abi"` in 4 plugin crates → `../../workflow/ox_workflow_abi` | `cargo build -p ox_cc_common -p ox_cc_broker_plugin -p ox_cc_client` |
| 8 | Rename `systems_tests/` → `tests/`; merge `ox_cc/docs/superpowers/` into `docs/superpowers/` | No path deps affected; `cargo build` expected to remain passing from Step 7 unchanged. Flag any test harness scripts that reference `systems_tests/` by path | — |
| 9 | Delete `ox_pipeline/`; delete and untrack all 42 artifact files; update `.gitignore` | — | `cargo build` (full workspace) |

---

## Artifact Cleanup

### Git-Tracked Artifact Files to Remove (42 total)

Run `git rm --cached` for all of these, then delete them from disk:

**Root-level:**
`build_error.log`, `full_suite_run.log`, `ox_data_source_test.log`, `ox_driver_manager_test.log`, `ox_fileproc_tests.log`, `ox_messaging_client_tests.log`, `ox_package_manager_tests.log`, `ox_persistence_datasource_manager_tests.log`, `ox_persistence_driver_manager_tests.log`, `ox_persistence_test.log`, `ox_webservice.log`, `ox_webservice.pid`, `ox_webservice_api_tests.log`, `ox_webservice_errorhandler_jinja2_tests.log`, `ox_webservice_forwarded_for_tests.log`, `ox_webservice_ping_tests.log`, `ox_webservice_status_tests.log`, `ox_webservice_stream_tests.log`, `ox_webservice_template_jinja2_tests.log`, `ox_webservice_tests.log`, `repro_crash.log`, `server.log`, `server_output.log`, `test_log.log`

**`logs/` directory (delete entire directory):**
`logs/ox_webservice.log`, `logs/startup.log`

**Nested in `ox_package_manager/systems_tests/`:**
All `logs/ox_webservice.log`, `logs/test.log`, `start_script.log`, and `ox_webservice.pid` files under functional test subdirectories.

**Nested in `ox_persistence_datasource_manager/`:**
`logs/ox_webservice.log`, `ox_webservice.pid`

### Additional Root Files to Resolve

| File | Disposition |
|------|-------------|
| `ai_info.md`, `ai_information.txt`, `ai_tests.txt` | Delete |
| `all_modules.txt`, `failing_modules.txt`, `modules-systems_tests.txt`, `single_module.txt`, `test_modules.txt`, `test_single_module.txt` | Delete |
| `form_output.html`, `repro_crash.sh` | Delete |
| `log4rs.yaml` | Delete (duplicate of `conf/log4rs.yaml`) |
| `IMPLEMENT_OX_WORKFLOW.md` | Move to `docs/` |
| `ProcessingPhases.md` | Move to `docs/` |
| `TODO.md` | Keep at root (standard project file) |
| `bulk_cargo.py` | Move to `scripts/` |

### Updated `.gitignore` Additions

```gitignore
# Build artifacts
target/
*.log
*.pid

# Test output
*_tests.log
```

---

## Success Criteria

1. `cargo build` passes for the full workspace after all steps complete
2. All crates are located under `crates/<domain>/<crate_name>/`
3. Root contains only: `Cargo.toml`, `Cargo.lock`, `LICENSE`, `README.md`, `TODO.md`, `crates/`, `conf/`, `content/`, `docs/`, `scripts/`, `tests/`, `sample_projects/`, `.github/`, `.gitignore`, `images/`, `logo/`, `pkg/`, `test_assets/`, `test_plugins/`
4. No `*.log`, `*.pid`, or build artifact files tracked in git
5. `ox_pipeline/` directory is deleted
6. Stale `ox_messaging_client/ox_messaging_mqtt/` directory is deleted
7. All 9 previously non-member crates are listed as workspace members in `Cargo.toml`
8. `ox_cc/` directory is fully dissolved (crates in `crates/cc/`, docs/conf/scripts redistributed)
9. No hardcoded absolute paths or `../../oxIDIZER/` paths remain in any `Cargo.toml`
