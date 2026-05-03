# Drivers & Management Plugins

Covers the driver FFI ABI, the GDO relational driver, and the three management plugins.

---

## Driver FFI ABI

Persistence drivers are dynamically loaded shared libraries. Each driver exports the
following C-compatible symbols. The host (`ox_data_broker`, `ox_persistence_datasource_manager`)
loads them via `libloading`.

| Symbol | Signature | Description |
|--------|-----------|-------------|
| `ox_driver_init` | `fn(*const c_char) -> *mut c_void` | Initialize driver; arg is JSON config string; returns opaque context pointer |
| `ox_driver_destroy` | `fn(*mut c_void)` | Free context allocated by init |
| `ox_driver_persist` | `fn(*mut c_void, *const c_char, *const c_char) -> c_int` | Persist serialized GDO (JSON map arg) to location; 0 = success |
| `ox_driver_restore` | `fn(*mut c_void, *const c_char, *const c_char) -> OxBuffer` | Restore GDO by ID from location; returns JSON map as OxBuffer |
| `ox_driver_fetch` | `fn(*mut c_void, *const c_char, *const c_char) -> OxBuffer` | Fetch matching IDs; filter arg is JSON map; returns JSON array of ID strings |
| `ox_driver_free_buffer` | `fn(OxBuffer)` | Free an OxBuffer returned by restore or fetch |
| `ox_driver_get_driver_metadata` | `fn() -> *mut c_char` | Returns JSON-serialized `DriverMetadata`; caller frees with `libc::free` |
| `ox_driver_get_config_schema` | `fn() -> *mut c_char` | Returns YAML schema for driver configuration (used by datasource manager form renderer) |
| `ox_driver_delete` | `fn(*mut c_void, *const c_char, *const c_char) -> c_int` | Delete record by ID from location; args: (ctx, location, id); 0 = success |
| `ox_driver_call_action` | `fn(*mut c_void, *const c_char, *const c_char) -> OxBuffer` | Optional. Execute named action with JSON params; returns JSON result |
| `ox_driver_list_datasets` | `fn(*mut c_void, *const c_char) -> OxBuffer` | Optional. List available datasets/tables; returns JSON array of strings |
| `ox_driver_describe_dataset` | `fn(*mut c_void, *const c_char) -> OxBuffer` | Optional. Return field schema for a named dataset; arg is dataset name; returns JSON `DriverDatasetSchema` |

**JSON map format** (for persist/restore/fetch): matches `to_serializable_map()` output.
Each key maps to a JSON array: `[value_string, type_string, {param_key: param_val, …}]`.

```json
{
  "id":    ["550e8400-…", "uuid",    {}],
  "name":  ["Alice",      "string",  {}],
  "score": ["99.5",       "float",   {"precision": "2"}],
  "__extensions__": ["{\"ox.persistence\":{…}}", "string", {}]
}
```

The driver stores and returns the `"__extensions__"` key opaquely — it does not interpret
its contents.

---

## ox_persistence_gdo_relational

**Crate:** `ox_persistence_gdo_relational`
**Type:** `cdylib` (persistence driver)

A meta-driver that stores GDO relationships (not the GDOs themselves). It delegates actual
storage to a configured inner driver. Its `describe_dataset` reports the `"relationships"`
schema — a table recording cross-GDO links between datasources.

### Relationships Schema

| Field | Type | Description |
|-------|------|-------------|
| `id` | uuid | Relationship record ID |
| `source_gdo_id` | uuid | ID of the source GDO |
| `source_driver_name` | string | Driver storing the source GDO |
| `source_location` | string | Location in that driver |
| `target_gdo_id` | uuid | ID of the target GDO |
| `target_driver_name` | string | Driver storing the target GDO |
| `target_location` | string | Location in that driver |
| `relationship_type` | string | Semantic type, e.g. `"one-to-many"` |
| `relationship_name` | string | Named role, e.g. `"orders"` |

### Configuration

| Parameter | Required | Description |
|-----------|----------|-------------|
| `internal_driver_name` | yes | Name of the inner driver in `PERSISTENCE_DRIVER_REGISTRY` |
| `internal_location` | yes | Location string passed to inner driver |

### C Exports

`get_driver_metadata_json()` → JSON `DriverMetadata`.
`create_driver()` → returns null (requires configuration; use `ox_driver_init` with JSON config).
`destroy_driver(ptr)` → drops the driver instance.

---

## ox_persistence_driver_manager (plugin)

**Crate:** `ox_persistence_driver_manager`
**Type:** `cdylib` plugin (`ox_plugin_init` / `ox_plugin_process` / `ox_plugin_destroy`)

Manages the lifecycle of persistence driver libraries. Exposes a REST API for listing,
loading, and unloading drivers at runtime.

### Routes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/drivers` | List all registered drivers with metadata |
| `POST` | `/drivers/reload` | Re-read `conf/drivers.yaml` and load/reload enabled drivers |
| `POST` | `/drivers/{name}/unload` | Unregister a driver |

### conf/drivers.yaml

```yaml
drivers:
  - id: "pg-main"
    name: "ox_persistence_pg"
    library_path: "/opt/ox/drivers"   # optional; defaults to target/debug
    state: "enabled"                  # "enabled" | "disabled"
  - id: "sqlite-local"
    name: "ox_persistence_sqlite"
    state: "enabled"
```

### Reload Processing

1. Read `conf/drivers.yaml` via `ox_fileproc::process_file` (supports `!include` directives).
2. For each entry where `state == "enabled"`:
   a. Construct platform-specific library filename (`libNAME.so` / `.dylib` / `.dll`).
   b. Load the library via `libloading::Library`.
   c. Call `ox_driver_get_driver_metadata()` to get name.
   d. Register in `PERSISTENCE_DRIVER_REGISTRY`.
3. Return `{"loaded": N, "errors": [...]}`.

---

## ox_persistence_datasource_manager (plugin)

**Crate:** `ox_persistence_datasource_manager`
**Type:** `cdylib` plugin

CRUD management for datasource definitions. Datasources are YAML/JSON files in a
configured directory. Each datasource record maps a logical name to a driver plus its
connection configuration.

### DataSource

```rust
pub struct DataSource {
    pub id: String,
    pub name: String,
    pub driver_id: String,       // matches ConfiguredDriver.id in drivers.yaml
    pub config: serde_json::Value,  // driver-specific connection params
}
```

### DriverDatasetSchema

Returned by `ox_driver_describe_dataset`. Describes the physical fields of one dataset
so the system can propose a `DataStoreContainer` + `DataObjectDefinition` without manual
entry.

```json
{
  "dataset_name": "users",
  "fields": [
    { "name": "id",    "data_type": "uuid",   "parameters": {}, "description": "Primary key" },
    { "name": "email", "data_type": "string",  "parameters": {}, "description": null },
    { "name": "score", "data_type": "float",   "parameters": {"precision": "2"}, "description": null }
  ]
}
```

### Routes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/data_sources` | List all datasource definitions |
| `POST` | `/data_sources` | Create a new datasource (body: JSON `DataSource`) |
| `DELETE` | `/data_sources/{id}` | Remove a datasource definition |
| `GET` | `/data_sources/new/form?driver=ID` | Render HTML configuration form for a driver |
| `GET` | `/data_sources/new/form?id=ID` | Render pre-filled form for an existing datasource |
| `POST` | `/action/{driver_id}/{action_name}` | Execute a driver action (e.g., `discover_local`) |
| `POST` | `/list_datasets/{driver_id}` | List datasets available via a driver |
| `GET` | `/data_sources/{id}/datasets` | List datasets available in a configured datasource |
| `GET` | `/data_sources/{id}/datasets/{name}/schema` | Return `DriverDatasetSchema` for a dataset |
| `POST` | `/data_sources/{id}/datasets/{name}/import` | Auto-create `DataStoreContainer` + `DataObjectDefinition` in the dictionary from driver schema |

### Dataset Discovery Flow

`GET /data_sources/{id}/datasets` loads the datasource by `id`, resolves the driver
library, calls `ox_driver_list_datasets(ctx, config_json)`, and returns the JSON array
of dataset names.

`GET /data_sources/{id}/datasets/{name}/schema` calls `ox_driver_describe_dataset(ctx,
name)` and returns the `DriverDatasetSchema` JSON.

`POST /data_sources/{id}/datasets/{name}/import` performs the full auto-population:

1. Call `ox_driver_describe_dataset` to get `DriverDatasetSchema`.
2. Build a `DataStoreContainer` from the schema:
   - `datasource_id` = datasource `id`
   - `name` = dataset name
   - `fields` = schema fields mapped to `DataStoreField`
3. Build a `DataObjectDefinition`:
   - `id` and `name` default to the dataset name; caller may override via request body
   - Each field becomes a `DataObjectAttribute` with `AttributeMapping::Direct` to the container
   - The field named `id` (or the first field if none is named `id`) is designated the primary key attribute
4. `POST` both to `ox_data_object_dictionary_manager` (`/dictionary/containers` and `/dictionary/objects`).
5. Return `{ "container_id": "...", "object_id": "..." }` so the caller can open the definitions for editing.

The resulting definitions are fully editable via the dictionary manager REST API. The
auto-populated definitions are a starting point, not a final schema.

### Storage

Datasource definitions are stored as individual YAML files: `{data_sources_dir}/{id}.yaml`.
On conflict (`on_content_conflict`): `Overwrite` replaces the file; `Append` merges;
`Skip` leaves existing; `Error` returns HTTP 500.

### Form Rendering

`GET /data_sources/new/form?driver=ID` calls `ox_driver_get_config_schema()` on the
driver library to get a YAML schema, then calls `ox_forms_api::render_form(schema, values)`
to produce an HTML form. Pre-populates fields when `?id=existing_id` is supplied.

### Configuration (plugin init JSON)

| Key | Default | Description |
|-----|---------|-------------|
| `data_sources_dir` | `/var/repos/oxIDIZER/ox_persistence/conf/datastores` | Directory for datasource YAML files |
| `drivers_file` | `conf/drivers.yaml` | Path to driver list |
| `driver_root` | `conf/drivers` | Root directory for driver libraries |
| `on_content_conflict` | `Skip` | `Overwrite` \| `Append` \| `Skip` \| `Error` |

---

## ox_persistence_driver_installer (plugin)

**Crate:** `ox_persistence_driver_installer`
**Type:** `cdylib` plugin

Integrates with `ox_package_manager` to install persistence driver crates. Handles
downloading, building, and registering driver libraries.

### Routes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/drivers/available` | List installable driver packages |
| `POST` | `/drivers/install` | Install a driver package (body: `{"package": "ox_persistence_pg", "version": "1.0.0"}`) |
| `DELETE` | `/drivers/{name}/uninstall` | Uninstall and unregister a driver |

### Install Flow

1. Call `ox_package_manager` to resolve and download the crate.
2. Build the `cdylib` target.
3. Copy the library to the configured driver root.
4. Append an entry to `conf/drivers.yaml`.
5. Trigger a `POST /drivers/reload` to activate.

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ox_workflow_abi` | Plugin ABI (`CoreHostApi`, `FlowControl`) |
| `ox_persistence` | `OxBuffer`, `DriverMetadata`, `PERSISTENCE_DRIVER_REGISTRY` |
| `ox_data_object_manager` | `DataStoreContainer`, `DataObjectDefinition`, `DataObjectAttribute`, `AttributeMapping` — datasource manager import feature |
| `ox_fileproc` | Config file loading with `!include` support |
| `ox_forms_api` | Form rendering for datasource configuration UI (datasource manager only) |
| `ox_package_manager` | Driver installation (installer plugin only) |
| `libloading` | Dynamic library loading |
| `prost` | Protobuf encoding for datasource list in task state |
| `serde` / `serde_json` / `serde_yaml` | Serialization |
