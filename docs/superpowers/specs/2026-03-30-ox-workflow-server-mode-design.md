# ox_workflow Server Mode & Admin Interface Integration Design

**Date:** 2026-03-30
**Status:** Approved

---

## Overview

This design covers two related deliverables:

1. **ox_workflow server mode** — running the workflow engine (scheduler, task API, management API) as plugins inside ox_webservice, with MQTT as the shared control and task plane across a cluster of nodes.
2. **ox_cc admin interface integration** — restoring the `ox_cc_admin_plugin` into the active module configuration, with a new `--set-module-config` CLI flag on ox_webservice for runtime config path overrides.

---

## Architecture

### Core Principle

All workflow server functionality is exposed as ox_webservice cdylib plugins. There is no separate binary. A "workflow server" is an ox_webservice instance configured with one or more workflow plugins. Multiple instances can share the same MQTT broker and SQLite database, enabling horizontal scaling and cluster-wide management.

### Plugin Crates

Three new cdylib crates are introduced under `crates/workflow/`:

| Crate | Purpose |
|-------|---------|
| `ox_workflow_scheduler_plugin` | Loads flows/stages, connects to MQTT task queues and control topic, runs the scheduler loop as a background tokio task |
| `ox_workflow_api_plugin` | Exposes task CRUD HTTP routes (`/workflow/api/tasks/...`), publishes to MQTT task queues |
| `ox_workflow_management_plugin` | Exposes cluster management HTTP routes (`/workflow/mgmt/...`), publishes control messages to MQTT control topic, reads SQLite for status |

### Existing Crates (unchanged)

- `ox_workflow_core` — Task, FlowDef, StageDef, TaskStatus, TaskFlags, HistoryRecord
- `ox_workflow_executor` — FlowManager, FlowRunner, StageRunner, CoreHostApi
- `ox_workflow_storage` — WorkflowStorage (SQLite via sqlx)
- `ox_workflow_scheduler` — WorkflowScheduler (event loop, semaphore, child workflow dispatch)
- `ox_workflow_config` — EngineConfig, QueueConfig, config loading helpers
- `ox_workflow_abi` — C ABI definitions, FlowControl codes
- `ox_event_bus` — EventBus trait
- `ox_event_bus_mqtt` — MQTT implementation of EventBus

---

## MQTT Topics

| Topic | Publisher | Subscriber | Purpose |
|-------|-----------|------------|---------|
| `workflow/tasks/pending` | API plugin | Scheduler plugin | New and resumed task notifications |
| `workflow/control` | Management plugin, any MQTT client | Scheduler plugin(s) | Cluster-wide control commands |
| `workflow/status` | Scheduler plugin | Management plugin, any MQTT client | Heartbeat and status broadcasts |

### Control Message Schema

All messages on `workflow/control` are JSON:

```json
{"command": "pause",  "queue": "workflow/tasks/pending", "node": "*"}
{"command": "resume", "queue": "workflow/tasks/pending", "node": "*"}
{"command": "pause",  "queue": "*",                      "node": "*"}
{"command": "resume", "queue": "*",                      "node": "*"}
{"command": "reload",                                     "node": "*"}
{"command": "drain",                                      "node": "*"}
{"command": "status_request",                             "node": "*"}
```

- `node`: target node ID (matches `client_id` in scheduler config) or `"*"` for all nodes
- `queue`: MQTT topic name or `"*"` for all queues (for pause/resume)

### Status Broadcast Schema

Scheduler nodes publish to `workflow/status` on startup, on state change, and in response to `status_request`:

```json
{
  "node": "worker-1",
  "status": "running",
  "queues": [
    {"name": "workflow/tasks/pending", "paused": false}
  ],
  "active_tasks": 12,
  "semaphore_permits_remaining": 88
}
```

---

## Plugin Designs

### `ox_workflow_scheduler_plugin`

**Init (`ox_plugin_init`):**
1. Parse config path from plugin config JSON (`config_file` field)
2. Load `scheduler.yaml` via ox_fileproc
3. Connect to MQTT broker → `Arc<dyn EventBus>`
4. Open SQLite → `WorkflowStorage`
5. Load flows and stages from configured directories → `FlowManager`
6. Build `CoreHostApi`
7. Build `WorkflowScheduler`
8. Subscribe to `workflow/control` topic
9. Spawn tokio tasks: scheduler event loop + control listener
10. Return plugin context pointer

**Process (`ox_plugin_process`):** No-op — this plugin has no HTTP routes. Returns `FLOW_CONTROL_CONTINUE` immediately.

**Control listener:** Reads messages from `workflow/control`, filters by `node` field, dispatches:
- `pause`/`resume` — toggles a per-queue `AtomicBool` that the scheduler event loop checks before consuming
- `reload` — calls `FlowManager::load_from_directory()` under a write lock
- `drain` — sets a global drain flag; scheduler stops accepting new tasks, waits for in-flight to complete
- `status_request` — publishes current state to `workflow/status`

**Destroy (`ox_plugin_destroy`):** Signals background tasks to stop, waits for drain.

**Config file (`scheduler.yaml`):**
```yaml
node_id: "worker-1"
mqtt:
  broker_url: "mqtt://localhost:1883"
  client_id: "ox_workflow_scheduler_1"
  keep_alive_secs: 30
db_path: "data/workflow.db"
flows_dir: "conf/workflow/flows"
stages_dir: "conf/workflow/stages"
plugin_paths:
  my_plugin: "plugins/libmy_plugin.so"
queues:
  - "workflow/tasks/pending"
max_concurrent_tasks: 100
tick_interval_ms: 1000
```

---

### `ox_workflow_api_plugin`

**Init (`ox_plugin_init`):**
1. Parse config path from plugin config JSON
2. Load `api.yaml` via ox_fileproc
3. Connect to MQTT broker → `Arc<dyn EventBus>` (for enqueue/resume)
4. Open SQLite connection pool → `WorkflowStorage`
5. Return plugin context pointer

**Process (`ox_plugin_process`):** Routes requests matching `/workflow/api/*`:

| Method | Path | Action |
|--------|------|--------|
| POST | `/workflow/api/flows` | Enqueue a new task |
| GET | `/workflow/api/tasks` | List tasks (by status or flow) |
| GET | `/workflow/api/tasks/:id` | Get task |
| DELETE | `/workflow/api/tasks/:id` | Delete task |
| GET | `/workflow/api/tasks/:id/history` | Get execution history |
| POST | `/workflow/api/tasks/:id/cancel` | Cancel task |
| POST | `/workflow/api/tasks/:id/resume` | Resume paused task |

Non-matching paths return `FLOW_CONTROL_CONTINUE` to pass to the next plugin.

**Config file (`api.yaml`):**
```yaml
mqtt:
  broker_url: "mqtt://localhost:1883"
  client_id: "ox_workflow_api_1"
db_path: "data/workflow.db"
```

---

### `ox_workflow_management_plugin`

**Init (`ox_plugin_init`):**
1. Parse config path from plugin config JSON
2. Load `management.yaml` via ox_fileproc
3. Connect to MQTT broker → `Arc<dyn EventBus>`
4. Open SQLite → `WorkflowStorage` (read-only for status queries)
5. Subscribe to `workflow/status` to cache latest node states
6. Return plugin context pointer

**Process (`ox_plugin_process`):** Routes requests matching `/workflow/mgmt/*`:

| Method | Path | Action |
|--------|------|--------|
| GET | `/workflow/mgmt/status` | Cluster status (cached from `workflow/status` broadcasts) |
| GET | `/workflow/mgmt/nodes` | List known scheduler nodes and their state |
| POST | `/workflow/mgmt/queues/pause` | Publish pause-all to `workflow/control` |
| POST | `/workflow/mgmt/queues/resume` | Publish resume-all to `workflow/control` |
| POST | `/workflow/mgmt/queues/:name/pause` | Publish pause for named queue |
| POST | `/workflow/mgmt/queues/:name/resume` | Publish resume for named queue |
| POST | `/workflow/mgmt/reload` | Publish reload to `workflow/control` |
| POST | `/workflow/mgmt/drain` | Publish drain to `workflow/control` |
| GET | `/workflow/mgmt/workers` | Active worker counts from cached status |

**Config file (`management.yaml`):**
```yaml
mqtt:
  broker_url: "mqtt://localhost:1883"
  client_id: "ox_workflow_mgmt_1"
db_path: "data/workflow.db"
status_cache_ttl_secs: 30
```

---

## ox_webservice Changes

### New CLI Flag: `--set-module-config`

```
ox_webservice [-c ox_webservice.yaml] [--set-module-config <id>=<path>]... <run|configcheck|daemon-run>
```

- Repeatable — multiple overrides allowed
- Applied after ox_fileproc loads the config: finds module by `id`, overwrites its `config_file` field
- Allows production deployments to point at `/etc/ox_workflow/scheduler.yaml` without modifying the module YAML

**Example:**
```sh
ox_webservice -c conf/ox_webservice.yaml \
  --set-module-config ox_workflow_scheduler=/etc/ox_workflow/scheduler.yaml \
  --set-module-config ox_workflow_api=/etc/ox_workflow/api.yaml \
  run
```

### No other changes to ox_webservice

The plugin lifecycle, flow, routing, and module loading are untouched.

---

## ox_cc Admin Interface Integration

### Restore Module Config

Restore `conf/modules/available/ox_cc_admin_plugin.yaml`:

```yaml
modules:
  - id: "ox_cc_admin"
    name: "ox_cc_admin_plugin"
    config_file: "crates/cc/ox_cc_admin_plugin/conf/ox_cc_admin_plugin.yaml"

routes:
  - url: "^/admin/api(/.*)?$"
    module_id: "ox_cc_admin"
```

To activate for development, symlink or copy to `conf/modules/active/`. For production:

```sh
ox_webservice --set-module-config ox_cc_admin=/etc/ox_cc/admin_plugin.yaml run
```

---

## Deployment Patterns

### Single-node development

One ox_webservice instance, all three plugins active:

```yaml
# conf/modules/active/
ox_workflow_scheduler.yaml  # start_scheduler: true
ox_workflow_api.yaml
ox_workflow_management.yaml
```

### Worker cluster

**Worker nodes** (N instances, no HTTP exposure for workflow):
```yaml
modules: [ox_workflow_scheduler]
# No routes for workflow/* — scheduler has no process handler
```

**Gateway node** (1 instance):
```yaml
modules: [ox_workflow_api, ox_workflow_management]
routes:
  - url: "^/workflow/api(/.*)?$"  module_id: ox_workflow_api
  - url: "^/workflow/mgmt(/.*)?$" module_id: ox_workflow_management
```

### Admin-only node

```yaml
modules: [ox_workflow_management]
routes:
  - url: "^/workflow/mgmt(/.*)?$" module_id: ox_workflow_management
```

Any MQTT client (shell scripts, CI/CD, other services) can publish directly to `workflow/control` to manage the cluster without going through the HTTP management API.

---

## Config Directory Layout

```
conf/workflow/
  scheduler.yaml
  api.yaml
  management.yaml
  flows/
    example_flow.yaml
  stages/
    example_stage.yaml

conf/modules/available/
  ox_workflow_scheduler.yaml
  ox_workflow_api.yaml
  ox_workflow_management.yaml
  ox_cc_admin_plugin.yaml

conf/modules/active/
  (symlinks or copies of the above as needed)
```

---

## Data Layer

- **SQLite in WAL mode** — multiple readers, one writer, safe for multiple plugin instances in the same process and across processes on the same host
- **Cross-host clusters** — require a shared SQLite file (NFS/network share) or migrating to a networked DB (Postgres, etc.) in the future. This design does not block that migration — `WorkflowStorage` is an abstraction.
- **Task state** — stored in SQLite by both the scheduler (after execution) and the API (on enqueue). MQTT carries only the task UUID.

---

## Error Handling

- **MQTT disconnect:** Scheduler plugin logs error, retries connection with exponential backoff. Tasks already in-flight complete normally.
- **SQLite error on enqueue:** API plugin returns HTTP 503.
- **Flow not found:** Scheduler logs error, marks task as `Errored` in SQLite.
- **Plugin panic:** Caught by `catch_unwind` in ox_workflow_executor (existing behavior).
- **Control message malformed:** Scheduler logs and ignores.

---

## Testing

- **Unit tests** in each plugin crate: handler logic tested with mock storage and mock EventBus
- **Integration test** (`systems_tests/`): single-node scenario — start ox_webservice with all three plugins, enqueue a task via API, verify it completes, verify management status reflects it
- Existing `ox_workflow_executor` and `ox_workflow_storage` unit tests are unchanged
