# ox_persistence_datasource_manager

REST plugin for managing datasource definitions. A datasource maps a logical name to a
driver plus its connection configuration. Also provides dataset discovery and
auto-import into the data dictionary.

---

## Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/data_sources` | List all datasource definitions |
| `POST` | `/data_sources` | Create a new datasource |
| `DELETE` | `/data_sources/{id}` | Remove a datasource |
| `GET` | `/data_sources/new/form?driver=ID` | Render HTML config form for a driver |
| `GET` | `/data_sources/new/form?id=ID` | Render pre-filled form for existing datasource |
| `POST` | `/action/{driver_id}/{action_name}` | Execute a driver action |
| `POST` | `/list_datasets/{driver_id}` | List datasets available via a driver |
| `GET` | `/data_sources/{id}/datasets` | List datasets in a configured datasource |
| `GET` | `/data_sources/{id}/datasets/{name}/schema` | Get `DriverDatasetSchema` for a dataset |
| `POST` | `/data_sources/{id}/datasets/{name}/import` | Auto-create dictionary entries from driver schema |

---

## DataSource Structure

```rust
pub struct DataSource {
    pub id: String,
    pub name: String,
    pub driver_id: String,          // matches id in conf/drivers.yaml
    pub config: serde_json::Value,  // driver-specific connection parameters
}
```

Stored as individual YAML files: `{data_sources_dir}/{id}.yaml`.

---

## Config Reference (plugin init)

| Key | Default | Description |
|---|---|---|
| `data_sources_dir` | `conf/datastores` | Directory for datasource YAML files |
| `drivers_file` | `conf/drivers.yaml` | Path to driver list |
| `driver_root` | `conf/drivers` | Root directory for driver libraries |
| `on_content_conflict` | `Skip` | `Overwrite` / `Append` / `Skip` / `Error` |

---

## Dataset Discovery and Auto-Import

Discovery flow:

1. `GET /data_sources/{id}/datasets` — calls `ox_driver_list_datasets(ctx, config_json)` → list of dataset names
2. `GET /data_sources/{id}/datasets/{name}/schema` — calls `ox_driver_describe_dataset(ctx, name)` → `DriverDatasetSchema`
3. `POST /data_sources/{id}/datasets/{name}/import`:
   - Calls `ox_driver_describe_dataset` to get schema
   - Builds `DataStoreContainer` from schema fields
   - Builds `DataObjectDefinition` with `Direct` mappings; designates `id` field (or first field) as primary key
   - POSTs both to `ox_data_object_dictionary_manager`
   - Returns `{ "container_id": "...", "object_id": "..." }`

Auto-import produces a starting-point definition. Attributes are fully editable via the
dictionary manager API afterward.

---

## Form Rendering

`GET /data_sources/new/form?driver=ID` calls `ox_driver_get_config_schema()` on the
driver library to get a YAML schema, then calls `ox_forms_api::render_form(schema,
values)` to produce an HTML form. Pre-populates when `?id=existing_id` is supplied.

---

## DriverDatasetSchema

```json
{
  "dataset_name": "users",
  "fields": [
    { "name": "id",    "data_type": "uuid",   "parameters": {},                "description": "Primary key" },
    { "name": "email", "data_type": "string",  "parameters": {},                "description": null },
    { "name": "score", "data_type": "float",   "parameters": {"precision": "2"}, "description": null }
  ]
}
```
