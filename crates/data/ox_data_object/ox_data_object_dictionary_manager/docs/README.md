# ox_data_object_dictionary_manager

REST plugin for managing the data dictionary (`DataDictionary`). Provides CRUD operations
for `DataStoreContainer` definitions and `DataObjectDefinition` objects via HTTP.

---

## Purpose

Exposes the in-memory `DataDictionary` (held by `DataObjectManager`) as a REST API.
Operators and tooling use this to define logical object types and their physical mappings
without editing JSON files directly.

---

## Routes (expected)

| Method | Path | Description |
|---|---|---|
| `GET` | `/dictionary/containers` | List all containers |
| `POST` | `/dictionary/containers` | Create or update a container |
| `DELETE` | `/dictionary/containers/{id}` | Remove a container |
| `GET` | `/dictionary/objects` | List all object definitions |
| `POST` | `/dictionary/objects` | Create or update an object definition |
| `DELETE` | `/dictionary/objects/{id}` | Remove an object definition |

These endpoints are also consumed by `ox_persistence_datasource_manager` during
dataset auto-import.

---

## Integration with Datasource Manager

`POST /data_sources/{id}/datasets/{name}/import` in `ox_persistence_datasource_manager`
calls this plugin to create:

1. A `DataStoreContainer` built from the driver's `DriverDatasetSchema`
2. A `DataObjectDefinition` with `Direct` attribute mappings derived from container fields

The resulting definitions are editable via this plugin's REST API.

---

## Implementation Notes

- The dictionary is serialized to/from JSON (`DataDictionary::save_to_file` /
  `load_from_file`). The backing JSON file path is configured at plugin init.
- `merge_container` performs an upsert: updates fields of an existing container or inserts
  a new one, rather than replacing the entire entry. This allows incremental schema
  updates from the datasource manager without losing manually added metadata.
