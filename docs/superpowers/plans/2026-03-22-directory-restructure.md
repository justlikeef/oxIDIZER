# Directory Restructure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorganize the oxIDIZER Rust workspace from a flat root layout into a `crates/<domain>/` hierarchy, clean up root artifacts, and resolve structural inconsistencies.

**Architecture:** Each domain migrates in a dependency-respecting order (util → workflow → messaging → webservice → forms → data → cc). Each step moves crates via `git mv`, updates path deps in moved crates (cross-domain deps point to wherever they currently live), updates root `Cargo.toml` members, and verifies with `cargo build` before committing. Within-domain crates become siblings (`"../sibling_name"`); cross-domain references use `"../../domain/crate_name"`.

**Tech Stack:** Rust/Cargo workspace, git, bash

---

## Path Dependency Reference

All relative paths are from `crates/<domain>/<crate_name>/Cargo.toml`. Two levels up (`../../`) reaches `crates/`. Three levels up (`../../../`) reaches workspace root (used temporarily during migration for not-yet-migrated deps).

---

## Task 1: Create `crates/` scaffold and migrate `util` domain

**Files:**
- Move: `ox_callback_manager/` → `crates/util/ox_callback_manager/`
- Move: `ox_fileproc/` → `crates/util/ox_fileproc/`
- Move: `ox_package_manager/` → `crates/util/ox_package_manager/`
- Modify: `crates/util/ox_package_manager/Cargo.toml`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create domain directories**

```bash
mkdir -p crates/util crates/workflow crates/messaging crates/webservice crates/forms crates/data crates/cc
```

- [ ] **Step 2: Move util crates**

```bash
git mv ox_callback_manager crates/util/ox_callback_manager
git mv ox_fileproc crates/util/ox_fileproc
git mv ox_package_manager crates/util/ox_package_manager
```

- [ ] **Step 3: Update `crates/util/ox_package_manager/Cargo.toml` path deps**

`ox_fileproc` is now a sibling — path stays the same. `ox_webservice_api` and `ox_workflow_abi` are still at root — temporarily 3 levels up:

```toml
ox_fileproc    = { path = "../ox_fileproc" }
ox_webservice_api = { path = "../../../ox_webservice_api" }
ox_workflow_abi   = { path = "../../../ox_workflow_abi" }
```

- [ ] **Step 4: Update root `Cargo.toml` workspace members**

Replace the current util entries (none existed for `ox_callback_manager`) with:

```toml
# util
"crates/util/ox_callback_manager",
"crates/util/ox_fileproc",
"crates/util/ox_package_manager",
```

Remove the old entries: `"ox_fileproc"`, `"ox_package_manager"`.

> **Do NOT remove `"ox_type_converter"`** — it stays as a workspace member at its current root path until Task 6 moves it. Task 6 Step 11 will replace it with `"crates/data/ox_type_converter"`.

> Note: `ox_callback_manager` was not previously a workspace member. This step adds it for the first time.

- [ ] **Step 5: Verify build**

```bash
cargo build -p ox_fileproc -p ox_callback_manager -p ox_package_manager
```

Expected: compiles successfully.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: migrate util domain to crates/util/"
```

---

## Task 2: Migrate `workflow` domain

**Files:**
- Move: `ox_workflow_abi/`, `ox_workflow_core/`, `ox_workflow_config/`, `ox_workflow_executor/`, `ox_workflow_storage/`, `ox_workflow_api/`, `ox_workflow_scheduler/` → `crates/workflow/<name>/`
- Modify: `crates/workflow/ox_workflow_config/Cargo.toml`
- Modify: `crates/workflow/ox_workflow_core/Cargo.toml`
- Modify: `crates/workflow/ox_workflow_executor/Cargo.toml`
- Modify: `crates/workflow/ox_workflow_api/Cargo.toml`
- Modify: `crates/workflow/ox_workflow_scheduler/Cargo.toml`
- Modify: `crates/util/ox_package_manager/Cargo.toml` (update now-migrated dep)
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Move workflow crates**

```bash
git mv ox_workflow_abi       crates/workflow/ox_workflow_abi
git mv ox_workflow_core      crates/workflow/ox_workflow_core
git mv ox_workflow_config    crates/workflow/ox_workflow_config
git mv ox_workflow_executor  crates/workflow/ox_workflow_executor
git mv ox_workflow_storage   crates/workflow/ox_workflow_storage
git mv ox_workflow_api       crates/workflow/ox_workflow_api
git mv ox_workflow_scheduler crates/workflow/ox_workflow_scheduler
```

- [ ] **Step 2: Fix hardcoded `../../oxIDIZER/` paths and cross-domain deps in workflow crates**

`ox_workflow_config/Cargo.toml` — fix broken ancestor path:
```toml
ox_fileproc = { path = "../../util/ox_fileproc" }
```

`ox_workflow_core/Cargo.toml`:
```toml
ox_workflow_abi    = { path = "../ox_workflow_abi" }
ox_workflow_config = { path = "../ox_workflow_config" }
ox_fileproc        = { path = "../../util/ox_fileproc" }
```

`ox_workflow_executor/Cargo.toml`:
```toml
ox_workflow_core   = { path = "../ox_workflow_core" }
ox_workflow_abi    = { path = "../ox_workflow_abi" }
ox_workflow_config = { path = "../ox_workflow_config" }
ox_fileproc        = { path = "../../util/ox_fileproc" }
```

`ox_workflow_storage/Cargo.toml` — intra-domain only, no change needed:
```toml
ox_workflow_core   = { path = "../ox_workflow_core" }
ox_workflow_config = { path = "../ox_workflow_config" }
```

`ox_workflow_api/Cargo.toml` — `ox_event_bus` still at root (migrates in step 3):
```toml
ox_workflow_core    = { path = "../ox_workflow_core" }
ox_workflow_storage = { path = "../ox_workflow_storage" }
ox_event_bus        = { path = "../../../ox_event_bus" }
```

`ox_workflow_scheduler/Cargo.toml` — `ox_event_bus` still at root:
```toml
ox_workflow_core     = { path = "../ox_workflow_core" }
ox_workflow_config   = { path = "../ox_workflow_config" }
ox_workflow_storage  = { path = "../ox_workflow_storage" }
ox_workflow_executor = { path = "../ox_workflow_executor" }
ox_event_bus         = { path = "../../../ox_event_bus" }
ox_workflow_abi      = { path = "../ox_workflow_abi" }
```

- [ ] **Step 3: Update previously-migrated `ox_package_manager` to use new workflow path**

`crates/util/ox_package_manager/Cargo.toml`:
```toml
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
```
(was `"../../../ox_workflow_abi"`)

- [ ] **Step 4: Update root `Cargo.toml` workspace members**

```toml
# workflow
"crates/workflow/ox_workflow_abi",
"crates/workflow/ox_workflow_core",
"crates/workflow/ox_workflow_config",
"crates/workflow/ox_workflow_executor",
"crates/workflow/ox_workflow_storage",
"crates/workflow/ox_workflow_api",
"crates/workflow/ox_workflow_scheduler",
```

Remove old entries: `"ox_workflow_abi"`, `"ox_workflow_core"`, `"ox_workflow_config"`, `"ox_workflow_executor"`, `"ox_workflow_storage"`, `"ox_workflow_api"`, `"ox_workflow_scheduler"`.

- [ ] **Step 5: Verify build**

```bash
cargo build -p ox_workflow_abi -p ox_workflow_core -p ox_workflow_scheduler
```

Expected: compiles successfully.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: migrate workflow domain to crates/workflow/"
```

---

## Task 3: Migrate `messaging` domain

**Files:**
- Move: `ox_event_bus/`, `ox_messaging_client/`, `ox_messaging_mqtt/` → `crates/messaging/<name>/`
- Delete: `crates/messaging/ox_messaging_client/ox_messaging_mqtt/` (stale test artifact — moves with parent, then removed)
- Modify: `crates/messaging/ox_messaging_client/Cargo.toml`
- Modify: `crates/messaging/ox_messaging_mqtt/Cargo.toml`
- Modify: `crates/workflow/ox_workflow_api/Cargo.toml` (update now-migrated dep)
- Modify: `crates/workflow/ox_workflow_scheduler/Cargo.toml` (update now-migrated dep)
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Move messaging crates**

```bash
git mv ox_event_bus       crates/messaging/ox_event_bus
git mv ox_messaging_client crates/messaging/ox_messaging_client
git mv ox_messaging_mqtt  crates/messaging/ox_messaging_mqtt
```

- [ ] **Step 2: Remove stale test artifact directory (moved with ox_messaging_client)**

```bash
git rm -r crates/messaging/ox_messaging_client/ox_messaging_mqtt
```

- [ ] **Step 3: Update path deps in moved messaging crates**

`crates/messaging/ox_messaging_client/Cargo.toml`:
```toml
ox_event_bus      = { path = "../ox_event_bus" }
ox_event_bus_mqtt = { path = "../ox_event_bus/ox_event_bus_mqtt" }
ox_workflow_config = { path = "../../workflow/ox_workflow_config" }
```

`crates/messaging/ox_messaging_mqtt/Cargo.toml`:
```toml
ox_fileproc    = { path = "../../util/ox_fileproc" }
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
ox_event_bus   = { path = "../ox_event_bus" }
```

- [ ] **Step 4: Update workflow crates that now point to migrated `ox_event_bus`**

`crates/workflow/ox_workflow_api/Cargo.toml`:
```toml
ox_event_bus = { path = "../../messaging/ox_event_bus" }
```
(was `"../../../ox_event_bus"`)

`crates/workflow/ox_workflow_scheduler/Cargo.toml`:
```toml
ox_event_bus = { path = "../../messaging/ox_event_bus" }
```
(was `"../../../ox_event_bus"`)

- [ ] **Step 5: Update root `Cargo.toml` workspace members**

```toml
# messaging
"crates/messaging/ox_event_bus",
"crates/messaging/ox_messaging_client",
"crates/messaging/ox_messaging_mqtt",
```

Remove old entries: `"ox_event_bus"`, `"ox_messaging_client"`, `"ox_messaging_mqtt"`.

- [ ] **Step 6: Verify build**

```bash
cargo build -p ox_event_bus -p ox_messaging_client -p ox_messaging_mqtt
```

Expected: compiles successfully.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: migrate messaging domain to crates/messaging/"
```

---

## Task 4: Migrate `webservice` domain

**Files:**
- Move: all 17 webservice crates → `crates/webservice/<name>/`
- Modify: `Cargo.toml` for each webservice crate (cross-domain path updates)
- Modify: `crates/util/ox_package_manager/Cargo.toml` (update now-migrated dep)
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Move webservice crates**

```bash
git mv ox_webservice                      crates/webservice/ox_webservice
git mv ox_webservice_api                  crates/webservice/ox_webservice_api
git mv ox_webservice_router               crates/webservice/ox_webservice_router
git mv ox_webservice_test_utils           crates/webservice/ox_webservice_test_utils
git mv ox_auth_ip                         crates/webservice/ox_auth_ip
git mv ox_webservice_forwarded_for        crates/webservice/ox_webservice_forwarded_for
git mv ox_webservice_restore_ip           crates/webservice/ox_webservice_restore_ip
git mv ox_webservice_errorhandler_jinja2  crates/webservice/ox_webservice_errorhandler_jinja2
git mv ox_webservice_errorhandler_json    crates/webservice/ox_webservice_errorhandler_json
git mv ox_webservice_redirect             crates/webservice/ox_webservice_redirect
git mv ox_webservice_rewrite              crates/webservice/ox_webservice_rewrite
git mv ox_webservice_stream               crates/webservice/ox_webservice_stream
git mv ox_webservice_template_jinja2      crates/webservice/ox_webservice_template_jinja2
git mv ox_webservice_ping                 crates/webservice/ox_webservice_ping
git mv ox_webservice_status               crates/webservice/ox_webservice_status
git mv ox_webservice_vary_header          crates/webservice/ox_webservice_vary_header
git mv ox_webservice_wsgi                 crates/webservice/ox_webservice_wsgi
```

- [ ] **Step 2: Update path deps in `ox_webservice`**

```toml
ox_webservice_api    = { path = "../ox_webservice_api" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_workflow_core     = { path = "../../workflow/ox_workflow_core" }
ox_workflow_executor = { path = "../../workflow/ox_workflow_executor" }
ox_fileproc          = { path = "../../util/ox_fileproc" }
```

- [ ] **Step 3: Update path deps in `ox_webservice_api`**

```toml
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
```

- [ ] **Step 4: Update path deps in router, test_utils, auth_ip**

`ox_webservice_router/Cargo.toml`:
```toml
ox_webservice_api = { path = "../ox_webservice_api" }
```

`ox_webservice_test_utils/Cargo.toml`:
```toml
ox_webservice_api = { path = "../ox_webservice_api" }
```

`ox_auth_ip/Cargo.toml`:
```toml
ox_workflow_abi   = { path = "../../workflow/ox_workflow_abi" }
ox_webservice_api = { path = "../ox_webservice_api" }
ox_fileproc       = { path = "../../util/ox_fileproc" }
```

- [ ] **Step 5: Update path deps in forwarded_for and restore_ip**

`ox_webservice_forwarded_for/Cargo.toml`:
```toml
ox_webservice_api    = { path = "../ox_webservice_api" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_webservice_test_utils = { path = "../ox_webservice_test_utils" }
```

`ox_webservice_restore_ip/Cargo.toml`:
```toml
ox_webservice_api = { path = "../ox_webservice_api" }
ox_workflow_abi   = { path = "../../workflow/ox_workflow_abi" }
```

- [ ] **Step 6: Update path deps in error handlers**

`ox_webservice_errorhandler_jinja2/Cargo.toml`:
```toml
ox_webservice_api    = { path = "../ox_webservice_api" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc          = { path = "../../util/ox_fileproc" }
ox_webservice_test_utils = { path = "../ox_webservice_test_utils" }
```

`ox_webservice_errorhandler_json/Cargo.toml`:
```toml
ox_webservice_api = { path = "../ox_webservice_api" }
ox_workflow_abi   = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc       = { path = "../../util/ox_fileproc" }
```

- [ ] **Step 7: Update path deps in routing plugins**

`ox_webservice_redirect/Cargo.toml`:
```toml
ox_webservice_api = { path = "../ox_webservice_api" }
ox_workflow_abi   = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc       = { path = "../../util/ox_fileproc" }
```

`ox_webservice_rewrite/Cargo.toml`:
```toml
ox_webservice_api = { path = "../ox_webservice_api" }
ox_workflow_abi   = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc       = { path = "../../util/ox_fileproc" }
```

- [ ] **Step 8: Update path deps in content-serving and response plugins**

`ox_webservice_stream/Cargo.toml`:
```toml
ox_webservice_api    = { path = "../ox_webservice_api" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc          = { path = "../../util/ox_fileproc" }
ox_webservice_test_utils = { path = "../ox_webservice_test_utils" }
```

`ox_webservice_template_jinja2/Cargo.toml`:
```toml
ox_webservice_api    = { path = "../ox_webservice_api" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc          = { path = "../../util/ox_fileproc" }
ox_webservice_test_utils = { path = "../ox_webservice_test_utils" }
```

`ox_webservice_ping/Cargo.toml`:
```toml
ox_webservice_api    = { path = "../ox_webservice_api" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_webservice_test_utils = { path = "../ox_webservice_test_utils" }
```

`ox_webservice_status/Cargo.toml`:
```toml
ox_webservice_api    = { path = "../ox_webservice_api" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc          = { path = "../../util/ox_fileproc" }
ox_webservice_test_utils = { path = "../ox_webservice_test_utils" }
```

`ox_webservice_vary_header/Cargo.toml`:
```toml
ox_webservice_api = { path = "../ox_webservice_api" }
```

`ox_webservice_wsgi/Cargo.toml`:
```toml
ox_webservice_api = { path = "../ox_webservice_api" }
ox_workflow_abi   = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc       = { path = "../../util/ox_fileproc" }
```

- [ ] **Step 9: Update previously-migrated `ox_package_manager`**

`crates/util/ox_package_manager/Cargo.toml`:
```toml
ox_webservice_api = { path = "../../webservice/ox_webservice_api" }
```
(was `"../../../ox_webservice_api"`)

- [ ] **Step 10: Update root `Cargo.toml` workspace members**

```toml
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
```

Remove old entries.

- [ ] **Step 11: Verify build**

```bash
cargo build -p ox_webservice -p ox_webservice_api -p ox_auth_ip
```

Expected: compiles successfully.

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "refactor: migrate webservice domain to crates/webservice/"
```

---

## Task 5: Migrate `forms` domain

`ox_forms/` currently acts as both a crate and a container. The four nested sub-crates are extracted first, then `ox_forms/` itself is moved.

**Files:**
- Move (from nested): `ox_forms/ox_forms_api/` → `crates/forms/ox_forms_api/`
- Move (from nested): `ox_forms/ox_forms_client/` → `crates/forms/ox_forms_client/`
- Move (from nested): `ox_forms/ox_forms_server/` → `crates/forms/ox_forms_server/`
- Move (from nested): `ox_forms/ox_forms_std_renderers/` → `crates/forms/ox_forms_std_renderers/`
- Move: `ox_forms/` → `crates/forms/ox_forms/`
- Modify: Cargo.toml for all 5 forms crates
- Modify: `Cargo.toml` (workspace members — adds 5 new members)

- [ ] **Step 1: Extract nested sub-crates from `ox_forms/` into `crates/forms/`**

```bash
git mv ox_forms/ox_forms_api          crates/forms/ox_forms_api
git mv ox_forms/ox_forms_client       crates/forms/ox_forms_client
git mv ox_forms/ox_forms_server       crates/forms/ox_forms_server
git mv ox_forms/ox_forms_std_renderers crates/forms/ox_forms_std_renderers
```

- [ ] **Step 2: Move `ox_forms` crate itself**

```bash
git mv ox_forms crates/forms/ox_forms
```

- [ ] **Step 3: Update `crates/forms/ox_forms/Cargo.toml`**

`ox_data_object`, `ox_type_converter`, `ox_data_object_manager` are still at root (data migrates in Task 6 — point to root temporarily):

```toml
ox_data_object        = { path = "../../../ox_data_object" }
ox_type_converter     = { path = "../../../ox_type_converter" }
ox_data_object_manager = { path = "../../../ox_data_object/ox_data_object_manager" }
ox_webservice_api     = { version = "0.0.1", path = "../../webservice/ox_webservice_api" }
```

- [ ] **Step 4: Update `crates/forms/ox_forms_api/Cargo.toml`**

```toml
ox_webservice_api = { path = "../../webservice/ox_webservice_api" }
```
(was `"../../ox_webservice_api"` from its old nested position)

- [ ] **Step 5: Update `crates/forms/ox_forms_client/Cargo.toml`**

```toml
ox_forms = { path = "../ox_forms" }
```
(was `".."` when nested inside ox_forms; now a sibling)

- [ ] **Step 6: Update `crates/forms/ox_forms_server/Cargo.toml`**

```toml
ox_forms             = { path = "../ox_forms" }
ox_webservice_api    = { path = "../../webservice/ox_webservice_api" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc          = { path = "../../util/ox_fileproc" }
ox_forms_std_renderers = { path = "../ox_forms_std_renderers" }
```

- [ ] **Step 7: Update `crates/forms/ox_forms_std_renderers/Cargo.toml`**

```toml
ox_forms = { path = "../ox_forms" }
```
(was `".."` when nested)

- [ ] **Step 8: Update root `Cargo.toml` workspace members**

```toml
# forms  (all 5 are new workspace members)
"crates/forms/ox_forms",
"crates/forms/ox_forms_api",
"crates/forms/ox_forms_client",
"crates/forms/ox_forms_server",
"crates/forms/ox_forms_std_renderers",
```

Remove old entry (if any existed for ox_forms — it was previously not a member).

- [ ] **Step 9: Verify build**

```bash
cargo build -p ox_forms -p ox_forms_server -p ox_forms_client
```

Expected: compiles successfully.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "refactor: migrate forms domain to crates/forms/"
```

---

## Task 6: Migrate `data` domain

**Files:**
- Move: `ox_data_object/`, `ox_data_broker/`, `ox_locking/`, `ox_type_converter/`, `ox_persistence/`, `ox_persistence_datasource_manager/`, `ox_persistence_driver_installer/`, `ox_persistence_driver_manager/`, `ox_persistence_gdo_relational/` → `crates/data/<name>/`
- Modify: Cargo.toml for each data crate
- Modify: `crates/forms/ox_forms/Cargo.toml` (update now-migrated data deps)
- Modify: `Cargo.toml` (workspace members — adds `ox_data_object`, `ox_locking`, `ox_persistence_gdo_relational` as new members)

- [ ] **Step 1: Move data crates**

```bash
git mv ox_data_object                   crates/data/ox_data_object
git mv ox_data_broker                   crates/data/ox_data_broker
git mv ox_locking                       crates/data/ox_locking
git mv ox_type_converter                crates/data/ox_type_converter
git mv ox_persistence                   crates/data/ox_persistence
git mv ox_persistence_datasource_manager crates/data/ox_persistence_datasource_manager
git mv ox_persistence_driver_installer  crates/data/ox_persistence_driver_installer
git mv ox_persistence_driver_manager    crates/data/ox_persistence_driver_manager
git mv ox_persistence_gdo_relational    crates/data/ox_persistence_gdo_relational
```

- [ ] **Step 2: Update `ox_data_object/Cargo.toml`**

```toml
ox_type_converter = { path = "../ox_type_converter" }
```

- [ ] **Step 3: Update `ox_locking/Cargo.toml`**

```toml
ox_data_object     = { path = "../ox_data_object" }
ox_callback_manager = { path = "../../util/ox_callback_manager" }
ox_type_converter  = { path = "../ox_type_converter" }
```

- [ ] **Step 4: Update `ox_persistence/Cargo.toml`**

```toml
ox_data_object    = { path = "../ox_data_object" }
ox_type_converter = { path = "../ox_type_converter" }
```

- [ ] **Step 5: Update `ox_persistence_gdo_relational/Cargo.toml`**

```toml
ox_persistence    = { path = "../ox_persistence" }
ox_data_object    = { path = "../ox_data_object" }
ox_type_converter = { path = "../ox_type_converter" }
ox_locking        = { path = "../ox_locking" }
```

- [ ] **Step 6: Update `ox_data_broker/Cargo.toml`**

```toml
ox_persistence            = { path = "../ox_persistence" }
ox_persistence_api        = { path = "../ox_persistence/ox_persistence_api" }
ox_persistence_gdo_relational = { path = "../ox_persistence_gdo_relational" }
ox_locking                = { path = "../ox_locking" }
ox_webservice_api         = { path = "../../webservice/ox_webservice_api" }
ox_fileproc               = { path = "../../util/ox_fileproc" }
```

- [ ] **Step 7: Update `ox_persistence_datasource_manager/Cargo.toml`**

```toml
ox_webservice_api = { path = "../../webservice/ox_webservice_api" }
ox_workflow_abi   = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc       = { path = "../../util/ox_fileproc" }
ox_persistence    = { path = "../ox_persistence" }
ox_forms_api      = { version = "0.1.0", path = "../../forms/ox_forms_api" }
```

- [ ] **Step 8: Update `ox_persistence_driver_installer/Cargo.toml`**

```toml
ox_persistence  = { path = "../ox_persistence" }
ox_fileproc     = { path = "../../util/ox_fileproc" }
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
```

- [ ] **Step 9: Update `ox_persistence_driver_manager/Cargo.toml`**

```toml
ox_workflow_abi       = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc           = { path = "../../util/ox_fileproc" }
ox_persistence        = { path = "../ox_persistence" }
ox_webservice_test_utils = { path = "../../webservice/ox_webservice_test_utils" }
ox_webservice_api     = { path = "../../webservice/ox_webservice_api" }
```

- [ ] **Step 10: Update `crates/forms/ox_forms/Cargo.toml` (data now migrated)**

```toml
ox_data_object         = { path = "../../data/ox_data_object" }
ox_type_converter      = { path = "../../data/ox_type_converter" }
ox_data_object_manager = { path = "../../data/ox_data_object/ox_data_object_manager" }
ox_webservice_api      = { version = "0.0.1", path = "../../webservice/ox_webservice_api" }
```
(removes the temporary `"../../../"` paths set in Task 5)

- [ ] **Step 11: Update root `Cargo.toml` workspace members**

```toml
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
```

Remove old entries: `"ox_data_broker"`, `"ox_persistence"`, `"ox_persistence_datasource_manager"`, `"ox_persistence_driver_installer"`, `"ox_persistence_driver_manager"`, `"ox_package_manager"` (was removed in Task 1).

> Note: `ox_data_object`, `ox_locking`, `ox_persistence_gdo_relational` are new workspace members being added for the first time.

- [ ] **Step 12: Verify build**

```bash
cargo build -p ox_data_object -p ox_persistence -p ox_persistence_datasource_manager
```

Expected: compiles successfully.

- [ ] **Step 13: Commit**

```bash
git add -A
git commit -m "refactor: migrate data domain to crates/data/"
```

---

## Task 7: Migrate `cc` domain and dissolve `ox_cc/`

**Files:**
- Move: `ox_cc/crates/ox_cc_*` (8 crates) → `crates/cc/<name>/`
- Move: `ox_cc/DESIGN.md`, `IMPLEMENTATION_PLAN.md`, `NEW_FEATURE.md`, `PROJECT_INFO.md`, `WORK_IN_PROGRESS.md` → `docs/cc/`
- Move: `ox_cc/docs/superpowers/plans/*` → `docs/superpowers/plans/`
- Move: `ox_cc/docs/superpowers/specs/*` → `docs/superpowers/specs/`
- Move: `ox_cc/conf/` → `conf/cc/`
- Move: `ox_cc/edit_docs.py`, `ox_cc/edit_roles.py` → `scripts/`
- Review: `ox_cc/.claude/settings.json` — merge relevant settings into root `.claude/settings.json`
- Delete: `ox_cc/` (empty after above moves)
- Modify: `crates/cc/ox_cc_broker_plugin/Cargo.toml`, `ox_cc_admin_plugin`, `ox_cc_manifest_plugin`, `ox_cc_report_plugin` (fix absolute paths)
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create `docs/cc/` and move cc markdown docs**

```bash
mkdir -p docs/cc
git mv ox_cc/DESIGN.md           docs/cc/DESIGN.md
git mv ox_cc/IMPLEMENTATION_PLAN.md docs/cc/IMPLEMENTATION_PLAN.md
git mv ox_cc/NEW_FEATURE.md      docs/cc/NEW_FEATURE.md
git mv ox_cc/PROJECT_INFO.md     docs/cc/PROJECT_INFO.md
git mv ox_cc/WORK_IN_PROGRESS.md docs/cc/WORK_IN_PROGRESS.md
```

- [ ] **Step 2: Merge `ox_cc/docs/superpowers/` into root `docs/superpowers/`**

```bash
git mv ox_cc/docs/superpowers/plans/2026-03-20-commandset-executor.md \
       docs/superpowers/plans/2026-03-20-commandset-executor.md
git mv ox_cc/docs/superpowers/plans/2026-03-20-session-authorization.md \
       docs/superpowers/plans/2026-03-20-session-authorization.md
git mv ox_cc/docs/superpowers/specs/2026-03-20-commandset-executor-and-session-authorization-design.md \
       docs/superpowers/specs/2026-03-20-commandset-executor-and-session-authorization-design.md
```

- [ ] **Step 3: Move `ox_cc/conf/` to `conf/cc/`**

```bash
git mv ox_cc/conf conf/cc
```

- [ ] **Step 4: Move scripts**

```bash
git mv ox_cc/edit_docs.py scripts/edit_docs.py
git mv ox_cc/edit_roles.py scripts/edit_roles.py
```

- [ ] **Step 5: Review `ox_cc/.claude/settings.json`**

Read the file:
```bash
cat ox_cc/.claude/settings.json
```

Compare with root `.claude/settings.json`. Manually merge any relevant settings (e.g., permissions, hooks specific to the cc workspace). Then delete:
```bash
git rm ox_cc/.claude/settings.json
```

- [ ] **Step 6: Move cc crates**

```bash
git mv ox_cc/crates/ox_cc_common          crates/cc/ox_cc_common
git mv ox_cc/crates/ox_cc_broker_plugin   crates/cc/ox_cc_broker_plugin
git mv ox_cc/crates/ox_cc_manifest_plugin crates/cc/ox_cc_manifest_plugin
git mv ox_cc/crates/ox_cc_report_plugin   crates/cc/ox_cc_report_plugin
git mv ox_cc/crates/ox_cc_admin_plugin    crates/cc/ox_cc_admin_plugin
git mv ox_cc/crates/ox_cc_client          crates/cc/ox_cc_client
git mv ox_cc/crates/ox_cc_keygen          crates/cc/ox_cc_keygen
git mv ox_cc/crates/ox_cc_executor        crates/cc/ox_cc_executor
```

- [ ] **Step 7: Fix absolute `ox_workflow_abi` paths in four cc plugin crates**

`crates/cc/ox_cc_broker_plugin/Cargo.toml`:
```toml
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
ox_cc_common    = { path = "../ox_cc_common" }
```

`crates/cc/ox_cc_admin_plugin/Cargo.toml`:
```toml
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
ox_cc_common    = { path = "../ox_cc_common" }
```

`crates/cc/ox_cc_manifest_plugin/Cargo.toml`:
```toml
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
ox_cc_common    = { path = "../ox_cc_common" }
```

`crates/cc/ox_cc_report_plugin/Cargo.toml`:
```toml
ox_cc_common         = { path = "../ox_cc_common" }
ox_workflow_abi      = { path = "../../workflow/ox_workflow_abi" }
ox_cc_manifest_plugin = { path = "../ox_cc_manifest_plugin" }
```

- [ ] **Step 8: Verify `ox_cc/` is now empty and remove it**

```bash
find ox_cc -not -path 'ox_cc/target*' | sort
```

Expected: only `ox_cc/` itself and possibly `ox_cc/target/` remain. If so:
```bash
rm -rf ox_cc/target
rmdir ox_cc
```

- [ ] **Step 9: Update root `Cargo.toml` workspace members**

```toml
# cc
"crates/cc/ox_cc_common",
"crates/cc/ox_cc_broker_plugin",
"crates/cc/ox_cc_manifest_plugin",
"crates/cc/ox_cc_report_plugin",
"crates/cc/ox_cc_admin_plugin",
"crates/cc/ox_cc_client",
"crates/cc/ox_cc_keygen",
"crates/cc/ox_cc_executor",
```

Remove old entries: `"ox_cc/crates/ox_cc_common"`, etc.

- [ ] **Step 10: Verify build**

```bash
cargo build -p ox_cc_common -p ox_cc_broker_plugin -p ox_cc_client
```

Expected: compiles successfully.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "refactor: migrate cc domain to crates/cc/ and dissolve ox_cc/"
```

---

## Task 8: Rename `systems_tests/` and reorganize docs

**Files:**
- Move: `systems_tests/` → `tests/`
- Move: `IMPLEMENT_OX_WORKFLOW.md` → `docs/IMPLEMENT_OX_WORKFLOW.md`
- Move: `ProcessingPhases.md` → `docs/ProcessingPhases.md`
- Move: `bulk_cargo.py` → `scripts/bulk_cargo.py`

- [ ] **Step 1: Rename `systems_tests/` to `tests/`**

```bash
git mv systems_tests tests
```

- [ ] **Step 2: Check for test scripts referencing `systems_tests/` by path**

```bash
grep -r "systems_tests" tests/ scripts/ conf/ --include="*.sh" --include="*.yaml" --include="*.py" -l
```

Update any references found. Common locations: test runner scripts in `tests/`, conf files that reference test directories.

- [ ] **Step 3: Move root markdown docs to `docs/`**

```bash
git mv IMPLEMENT_OX_WORKFLOW.md docs/IMPLEMENT_OX_WORKFLOW.md
git mv ProcessingPhases.md      docs/ProcessingPhases.md
```

- [ ] **Step 4: Move `bulk_cargo.py` to `scripts/`**

```bash
git mv bulk_cargo.py scripts/bulk_cargo.py
```

- [ ] **Step 5: Verify workspace build still passes**

```bash
cargo build
```

Expected: compiles successfully (no path deps affected).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: rename systems_tests/ to tests/, move docs and scripts"
```

---

## Task 9: Artifact cleanup and `.gitignore` update

**Files:**
- Delete: `ox_pipeline/` (empty directory)
- Delete + untrack: all 42 tracked artifact files (logs, PIDs)
- Delete: misc root temp files
- Modify: `.gitignore`

- [ ] **Step 1: Delete `ox_pipeline/` directory**

```bash
rm -rf ox_pipeline
git add -A
```

(It's an untracked empty directory — no `git rm` needed, just delete.)

- [ ] **Step 2: Untrack and delete all root-level artifact files**

```bash
git rm --cached \
  build_error.log full_suite_run.log ox_data_source_test.log \
  ox_driver_manager_test.log ox_fileproc_tests.log \
  ox_messaging_client_tests.log ox_package_manager_tests.log \
  ox_persistence_datasource_manager_tests.log \
  ox_persistence_driver_manager_tests.log ox_persistence_test.log \
  ox_webservice.log ox_webservice.pid ox_webservice_api_tests.log \
  ox_webservice_errorhandler_jinja2_tests.log \
  ox_webservice_forwarded_for_tests.log ox_webservice_ping_tests.log \
  ox_webservice_status_tests.log ox_webservice_stream_tests.log \
  ox_webservice_template_jinja2_tests.log ox_webservice_tests.log \
  repro_crash.log server.log server_output.log test_log.log 2>/dev/null || true

rm -f build_error.log full_suite_run.log ox_data_source_test.log \
  ox_driver_manager_test.log ox_fileproc_tests.log \
  ox_messaging_client_tests.log ox_package_manager_tests.log \
  ox_persistence_datasource_manager_tests.log \
  ox_persistence_driver_manager_tests.log ox_persistence_test.log \
  ox_webservice.log ox_webservice.pid ox_webservice_api_tests.log \
  ox_webservice_errorhandler_jinja2_tests.log \
  ox_webservice_forwarded_for_tests.log ox_webservice_ping_tests.log \
  ox_webservice_status_tests.log ox_webservice_stream_tests.log \
  ox_webservice_template_jinja2_tests.log ox_webservice_tests.log \
  repro_crash.log server.log server_output.log test_log.log
```

- [ ] **Step 3: Untrack and delete `logs/` directory**

```bash
git rm --cached logs/ox_webservice.log logs/startup.log
rm -rf logs/
```

- [ ] **Step 4: Untrack and delete nested artifact files in `crates/data/ox_persistence_datasource_manager/`**

```bash
git rm --cached \
  crates/data/ox_persistence_datasource_manager/logs/ox_webservice.log \
  crates/data/ox_persistence_datasource_manager/ox_webservice.pid 2>/dev/null || true

rm -f crates/data/ox_persistence_datasource_manager/logs/ox_webservice.log
rm -f crates/data/ox_persistence_datasource_manager/ox_webservice.pid
```

- [ ] **Step 5: Untrack and delete nested artifact files in `crates/util/ox_package_manager/systems_tests/`**

```bash
git rm --cached $(git ls-files crates/util/ox_package_manager/systems_tests/ | grep -E "\.(log|pid)$|start_script")
rm -f $(git -C . ls-files --error-unmatch crates/util/ox_package_manager/systems_tests/ 2>/dev/null | grep -E "\.(log|pid)$|start_script" || true)
```

Or more directly:
```bash
find crates/util/ox_package_manager/systems_tests/ -name "*.log" -o -name "*.pid" -o -name "start_script.log" | xargs git rm --cached 2>/dev/null || true
find crates/util/ox_package_manager/systems_tests/ \( -name "*.log" -o -name "*.pid" -o -name "start_script.log" \) -delete
```

- [ ] **Step 6: Delete misc root temp files**

```bash
git rm --cached \
  ai_info.md ai_information.txt ai_tests.txt \
  all_modules.txt failing_modules.txt modules-systems_tests.txt \
  single_module.txt test_modules.txt test_single_module.txt \
  form_output.html repro_crash.sh log4rs.yaml 2>/dev/null || true

rm -f ai_info.md ai_information.txt ai_tests.txt \
  all_modules.txt failing_modules.txt modules-systems_tests.txt \
  single_module.txt test_modules.txt test_single_module.txt \
  form_output.html repro_crash.sh log4rs.yaml
```

- [ ] **Step 7: Update `.gitignore`**

Add to `.gitignore`:
```gitignore
# Build artifacts
target/
*.log
*.pid

# Test output
*_tests.log
```

- [ ] **Step 8: Final full workspace build**

```bash
cargo build
```

Expected: compiles successfully with no errors.

- [ ] **Step 9: Verify success criteria**

```bash
# Criterion 2: all crates under crates/<domain>/
ls crates/

# Criterion 3: clean root
ls /var/repos/oxIDIZER/

# Criterion 4: no tracked artifacts
git ls-files | grep -E "\.(log|pid)$"
# Expected: empty output

# Criterion 9: no hardcoded absolute or ../../oxIDIZER/ paths
grep -r "\/var\/repos\|oxIDIZER" crates/ --include="Cargo.toml"
grep -r "../../oxIDIZER" crates/ --include="Cargo.toml"
# Expected: empty output
```

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "chore: clean up artifacts, empty dirs, and update .gitignore"
```
