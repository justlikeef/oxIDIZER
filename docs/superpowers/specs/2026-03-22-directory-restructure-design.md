# Directory Restructure Design

**Date:** 2026-03-22
**Status:** Approved
**Scope:** Reorganize the oxIDIZER Rust workspace from a flat root layout into a `crates/<domain>/` hierarchy, clean up root artifacts, and resolve structural inconsistencies.

---

## Problem Statement

The current workspace root contains 57+ crate directories mixed with configuration, logs, build artifacts, and documentation. Related crate families are grouped inconsistently — some are nested (e.g., `ox_forms/`), some are flat at root (e.g., all `ox_webservice_*` plugins), and `ox_cc/` uses its own sub-workspace convention. Root-level log files, PID files, and AI working files pollute the repository. One empty directory (`ox_pipeline/`) and a duplicate crate (`ox_messaging_mqtt` exists both at root and nested inside `ox_messaging_client/`) add further confusion.

---

## Goals

- Organize all crates under `crates/<domain>/<crate_name>/`
- Establish seven domains: `webservice`, `messaging`, `workflow`, `forms`, `cc`, `data`, `util`
- Clean root of all artifacts, logs, and temp files
- Resolve `ox_cc/` sub-workspace by dissolving it into `crates/cc/`
- Remove the duplicate `ox_messaging_mqtt` nested inside `ox_messaging_client/`
- Delete the empty `ox_pipeline/` directory
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
│   │   ├── ox_data_object/           # retains ox_data_object_dictionary_manager/ and ox_data_object_manager/ sub-crates
│   │   ├── ox_locking/
│   │   ├── ox_persistence/           # retains ox_persistence_api/, ox_persistence_dictionary_manager/, drivers/ sub-crates
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
│   │   ├── ox_event_bus/
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
└── tests/                       # renamed from functional_tests/
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

## Workspace Cargo.toml Structure

```toml
[workspace]
resolver = "2"
members = [
    # workflow
    "crates/workflow/ox_workflow_abi",
    "crates/workflow/ox_workflow_core",
    "crates/workflow/ox_workflow_config",
    "crates/workflow/ox_workflow_executor",
    "crates/workflow/ox_workflow_storage",
    "crates/workflow/ox_workflow_api",
    "crates/workflow/ox_workflow_scheduler",

    # util
    "crates/util/ox_callback_manager",
    "crates/util/ox_fileproc",
    "crates/util/ox_package_manager",

    # messaging
    "crates/messaging/ox_event_bus",
    "crates/messaging/ox_messaging_client",
    "crates/messaging/ox_messaging_mqtt",

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

    # forms
    "crates/forms/ox_forms",
    "crates/forms/ox_forms_api",
    "crates/forms/ox_forms_client",
    "crates/forms/ox_forms_server",
    "crates/forms/ox_forms_std_renderers",

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

### ox_cc Sub-Workspace Dissolution
The current `ox_cc/` directory is a nested sub-workspace with its own `Cargo.toml`, `DESIGN.md`, `IMPLEMENTATION_PLAN.md`, and `.claude/` settings. During migration:
- The 8 crates under `ox_cc/crates/` move to `crates/cc/`
- `ox_cc/DESIGN.md` and `ox_cc/IMPLEMENTATION_PLAN.md` move to `docs/`
- `ox_cc/.claude/` settings are reviewed and merged into the root `.claude/` settings if applicable
- The `ox_cc/` directory is removed

### Duplicate ox_messaging_mqtt
`ox_messaging_mqtt` exists both at the workspace root and nested inside `ox_messaging_client/ox_messaging_mqtt/`. The root-level copy is the canonical workspace member. The nested copy inside `ox_messaging_client/` is removed.

### ox_forms Dual Role
`ox_forms/` currently acts as both a crate (has `src/lib.rs`) and a family container (has sub-crate subdirectories). It moves cleanly to `crates/forms/ox_forms/` with its sub-crates (`ox_forms_api/`, `ox_forms_client/`, `ox_forms_server/`, `ox_forms_std_renderers/`) promoted to siblings at `crates/forms/`.

### ox_persistence Sub-Crates
`ox_persistence/` contains nested sub-crates (`ox_persistence_api/`, `ox_persistence_dictionary_manager/`, `drivers/`). These are not workspace members and are not promoted — they remain nested inside `crates/data/ox_persistence/`.

---

## Migration Order

Each step moves crates, updates `Cargo.toml` workspace members, updates all `path = ` dependencies within affected crate `Cargo.toml` files, and verifies with a targeted `cargo build`.

| Step | Domain | Build Verification |
|------|--------|--------------------|
| 1 | `workflow` | `cargo build -p ox_workflow_core` |
| 2 | `util` | `cargo build -p ox_fileproc` |
| 3 | `messaging` (+ remove duplicate ox_messaging_mqtt) | `cargo build -p ox_event_bus` |
| 4 | `data` | `cargo build -p ox_persistence` |
| 5 | `forms` | `cargo build -p ox_forms` |
| 6 | `webservice` | `cargo build -p ox_webservice` |
| 7 | `cc` (dissolve ox_cc sub-workspace) | `cargo build -p ox_cc_common` |
| 8 | Rename `functional_tests/` → `tests/`, move `ox_cc/` docs to `docs/` | — |
| 9 | Delete `ox_pipeline/`, delete root artifacts, update `.gitignore` | `cargo build` (full workspace) |

---

## Artifact Cleanup

**Delete from root:**
- All `*.log` files (`build_error.log`, `full_suite_run.log`, `ox_webservice.log`, `ox_webservice_api_tests.log`, `ox_messaging_client_tests.log`, `ox_package_manager_tests.log`, `ox_persistence_datasource_manager_tests.log`, `ox_persistence_driver_manager_tests.log`, `ox_webservice_errorhandler_jinja2_tests.log`, `ox_webservice_errorhandler_json_tests.log`, `ox_webservice_forwarded_for_tests.log`, `ox_webservice_ping_tests.log`, `ox_webservice_status_tests.log`, `ox_webservice_stream_tests.log`, `ox_webservice_template_jinja2_tests.log`, `ox_webservice_tests.log`, `ox_fileproc_tests.log`, `ox_data_source_test.log`, `ox_driver_manager_test.log`, `ox_persistence_test.log`, `repro_crash.log`, `server.log`, `server_output.log`, `test_log.log`, `log4rs.yaml`)
- PID files: `ox_webservice.pid`
- Temp/intermediate files: `form_output.html`, `all_modules.txt`, `failing_modules.txt`, `modules-functional_tests.txt`, `single_module.txt`, `test_modules.txt`, `test_single_module.txt`, `repro_crash.sh`
- AI working files: `ai_info.md`, `ai_information.txt`, `ai_tests.txt`
- Empty directory: `ox_pipeline/`

**Add to `.gitignore`:**
```gitignore
# Build artifacts
target/
*.log
*.pid

# Test output
*_tests.log

# Runtime temp files
form_output.html
```

---

## Success Criteria

1. `cargo build` passes for the full workspace after all steps complete
2. All crates are located under `crates/<domain>/<crate_name>/`
3. Root contains only: `Cargo.toml`, `Cargo.lock`, `LICENSE`, `README.md`, `crates/`, `conf/`, `content/`, `docs/`, `scripts/`, `tests/`, `sample_projects/`, `.github/`, `.gitignore`, `images/`, `logo/`, `pkg/`, `test_assets/`, `test_plugins/`
4. No `*.log`, `*.pid`, or build artifact files tracked in git
5. `ox_pipeline/` directory is deleted
6. Duplicate `ox_messaging_mqtt` is removed
7. `ox_cc/` sub-workspace is dissolved into `crates/cc/`
