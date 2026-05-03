# ox_data Administration Guide

This guide is for operators installing, configuring, and monitoring the ox_data layer.

---

## Installing Drivers

Drivers are `cdylib` shared libraries loaded at runtime. There are two ways to install
them:

### Manual Installation

1. Build the driver crate as a shared library:
   ```bash
   cargo build --release -p ox_persistence_driver_db_postgres
   ```
2. Copy the library to the driver root (default: `conf/drivers/`):
   ```bash
   cp target/release/libox_persistence_driver_db_postgres.so conf/drivers/
   ```
3. Register it in `conf/drivers.yaml` (see below).
4. Load it via the driver manager REST API or restart the server.

### Via Driver Installer

If `ox_persistence_driver_installer` is in your pipeline:
```bash
curl -X POST https://server/drivers/install \
  -d '{"package": "ox_persistence_driver_db_postgres", "version": "1.0.0"}'
```
This downloads, builds, registers, and loads the driver automatically.

---

## conf/drivers.yaml

Lists which drivers should be loaded at startup or on reload:

```yaml
drivers:
  - id: "pg-main"
    name: "ox_persistence_driver_db_postgres"
    library_path: "/opt/ox/drivers"   # optional; defaults to conf/drivers/
    state: "enabled"                  # "enabled" | "disabled"
  - id: "sqlite-dev"
    name: "ox_persistence_driver_db_sqlite"
    state: "enabled"
  - id: "csv-files"
    name: "ox_persistence_driver_file_delimited"
    state: "disabled"
```

`id` is a stable identifier referenced by datasource definitions. `name` is the library
name (without `lib` prefix and `.so`/`.dylib`/`.dll` suffix).

Reload drivers without restarting the server:
```bash
curl -X POST https://server/drivers/reload
```
Response: `{"loaded": 2, "errors": []}` or `{"loaded": 1, "errors": ["..."]}` (207).

---

## Configuring Datasources

Datasources map a logical name to a driver plus connection parameters. They are stored
as YAML files in `conf/datastores/` (one file per datasource).

### Create via REST API

```bash
curl -X POST https://server/data_sources \
  -H "Content-Type: application/json" \
  -d '{
    "id": "main-db",
    "name": "Main PostgreSQL Database",
    "driver_id": "pg-main",
    "config": {
      "host": "db.example.com",
      "port": 5432,
      "database": "myapp",
      "username": "app",
      "password": "secret"
    }
  }'
```

### Create via Form UI

```
GET /data_sources/new/form?driver=pg-main
```

This renders an HTML form generated from the driver's config schema. Submit it to
`POST /data_sources` to save.

### Driver-Specific Connection Parameters

Each driver exports its own config schema via `ox_driver_get_config_schema()`. Use the
form UI or call the driver introspection APIs to discover available parameters.

Common parameters for DB drivers:

| Parameter | Description |
|---|---|
| `host` | Database hostname |
| `port` | Port number |
| `database` | Database/schema name |
| `username` | Login username |
| `password` | Login password |
| `pool_size` | Connection pool size (default varies by driver) |

Common parameters for file drivers:

| Parameter | Description |
|---|---|
| `base_dir` | Root directory for data files |
| `delimiter` | Field delimiter (delimited driver, default `,`) |

---

## Driver YAML Format

The YAML stored per datasource in `conf/datastores/{id}.yaml`:

```yaml
id: main-db
name: Main PostgreSQL Database
driver_id: pg-main
config:
  host: db.example.com
  port: 5432
  database: myapp
  username: app
  password: secret
```

The `config` block is passed verbatim to `ox_driver_init(config_json)`.

---

## Dataset Discovery and Auto-Import

Once a datasource is configured, you can discover its schema and auto-populate the data
dictionary:

```bash
# List available tables/datasets
curl https://server/data_sources/main-db/datasets

# Get schema for a specific dataset
curl https://server/data_sources/main-db/datasets/users/schema

# Auto-create DataStoreContainer + DataObjectDefinition from schema
curl -X POST https://server/data_sources/main-db/datasets/users/import
# Returns: { "container_id": "...", "object_id": "..." }
```

Auto-import creates a starting-point definition. Attributes can be modified afterward
via the dictionary manager REST API.

---

## The Persistence Driver Manager UI

The driver manager plugin (`ox_persistence_driver_manager`) provides:

| Endpoint | Description |
|---|---|
| `GET /drivers` | List all loaded drivers with metadata |
| `POST /drivers/reload` | Re-read `conf/drivers.yaml` and reload enabled drivers |
| `POST /drivers/{name}/unload` | Unregister a specific driver |

View currently loaded drivers:
```bash
curl https://server/drivers | jq '.[].name'
```

---

## Monitoring

### Health Indicators

There is no dedicated ox_data health endpoint. Monitor the system by:

1. **Driver load status:** `GET /drivers` — check that all expected drivers are listed
   with correct metadata.
2. **Datasource connectivity:** Use `POST /data/{driver_name}/fetch` with an empty filter
   and `limit=1` as a lightweight ping.
3. **Broker availability:** `GET /drivers` returning 200 confirms the broker plugin is
   operational.

### WebSocket Change Feed

Clients can subscribe to real-time change notifications:

```javascript
const ws = new WebSocket('wss://server/data/listen');
ws.onopen = () => ws.send(JSON.stringify({ subscribe: ['object-uuid-1', 'object-uuid-2'] }));
ws.onmessage = (e) => console.log(JSON.parse(e.data));
```

Each change event includes `object_id`, `attribute`, `value`, and `event` fields. Use
`after_commit` events (transaction-based changes) as a trigger to re-fetch the full
object.

### Logging

All data layer operations use the `ox_workflow` host logger. Key log patterns:

| Level | When |
|---|---|
| `ERROR` | Driver load failure, persist/restore failure, storage error |
| `WARN` | Partial reload failure (some drivers loaded, some failed) |
| `INFO` | Driver loaded/unloaded, datasource created/deleted |
| `DEBUG` | Per-operation routing, driver dispatch details |
