# ox_data_object_dictionary_manager — Dictionary Manager Plugin

**Crate:** `ox_data_object_dictionary_manager`
**Type:** `cdylib` plugin (`ox_plugin_init` / `ox_plugin_process` / `ox_plugin_destroy`)

Exposes the `DataDictionary` via a REST API, allowing runtime management of object
definitions, containers, and relationships without recompiling. The dictionary is persisted
as a JSON file; all changes are written through immediately.

---

## Routes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/dictionary/objects` | List all `DataObjectDefinition`s |
| `GET` | `/dictionary/objects/{id}` | Get a single object definition |
| `POST` | `/dictionary/objects` | Create a new object definition |
| `PUT` | `/dictionary/objects/{id}` | Replace an object definition |
| `DELETE` | `/dictionary/objects/{id}` | Remove an object definition |
| `GET` | `/dictionary/containers` | List all `DataStoreContainer`s |
| `GET` | `/dictionary/containers/{id}` | Get a single container |
| `POST` | `/dictionary/containers` | Create a new container |
| `PUT` | `/dictionary/containers/{id}` | Replace a container |
| `DELETE` | `/dictionary/containers/{id}` | Remove a container |
| `GET` | `/dictionary/objects/{id}/relationships` | List relationships for an object |
| `POST` | `/dictionary/objects/{id}/relationships` | Add a relationship to an object |
| `DELETE` | `/dictionary/objects/{id}/relationships/{rel_id}` | Remove a relationship |
| `POST` | `/dictionary/reload` | Reload dictionary from file (discards in-memory changes) |
| `GET` | `/dictionary/export` | Export full dictionary as JSON |
| `POST` | `/dictionary/import` | Replace full dictionary from JSON body |

---

## Configuration (plugin init JSON)

| Key | Default | Description |
|-----|---------|-------------|
| `dictionary_path` | `conf/dictionary.json` | Path to the JSON dictionary file |
| `auto_save` | `true` | Write dictionary to file after every mutating operation |

---

## Plugin Lifecycle

### `ox_plugin_init`

1. Read `dictionary_path` from config JSON.
2. Load `DataDictionary` from the file if it exists; start with an empty dictionary
   otherwise.
3. Store dictionary and config in a `ModuleContext` heap allocation.
4. Return context pointer.

### `ox_plugin_process`

Routes by `request.path` and `request.method`. All responses are JSON.
Mutating operations (`POST`, `PUT`, `DELETE`) call `dictionary.save_to_file` if
`auto_save` is enabled.

### `ox_plugin_destroy`

If `auto_save` is enabled, flushes the dictionary to file before freeing the context.

---

## Route Details

### `GET /dictionary/objects`

Returns all object definitions as a JSON array.

```json
[
  {
    "id": "user",
    "name": "User",
    "description": "Application user",
    "attributes": [...],
    "relationships": [...]
  }
]
```

### `GET /dictionary/objects/{id}`

Returns the single definition or `404`.

### `POST /dictionary/objects`

Body: `DataObjectDefinition` JSON. Inserts via `dictionary.add_object`. Returns `201`
with the created definition. Returns `409` if an object with the same `id` already exists.

### `PUT /dictionary/objects/{id}`

Body: `DataObjectDefinition` JSON. Replaces the existing definition. Returns `404` if
not found.

### `DELETE /dictionary/objects/{id}`

Removes the definition. Returns `404` if not found. Does not remove associated containers
or data from the backing store.

### `POST /dictionary/objects/{id}/relationships`

Body: `RelationshipDefinition` JSON. Appends to the object definition's `relationships`
list. Returns `201` with the created relationship. Returns `404` if the object is not
found.

### `DELETE /dictionary/objects/{id}/relationships/{rel_id}`

Removes the relationship by `rel_id` from the object definition. Returns `404` if either
the object or the relationship is not found.

### Container Routes

Mirror the object routes for `DataStoreContainer`. `POST /dictionary/containers` uses
`dictionary.add_container`; `PUT` uses `dictionary.merge_container`.

### `POST /dictionary/reload`

Discards the in-memory dictionary and reloads from `dictionary_path`. Returns `200` with
the reloaded dictionary's object and container counts. Returns `500` if the file cannot
be read.

### `GET /dictionary/export`

Returns the full `DataDictionary` as a pretty-printed JSON response.

### `POST /dictionary/import`

Body: full `DataDictionary` JSON. Replaces the in-memory dictionary entirely and saves to
`dictionary_path`. Returns `200`. Returns `400` if the body cannot be parsed.

---

## Error Cases

| Condition | HTTP | Body |
|-----------|------|------|
| Object/container not found | 404 | `{"error": "Not found"}` |
| Duplicate id on create | 409 | `{"error": "Already exists"}` |
| Invalid JSON body | 400 | `{"error": "Invalid JSON: ..."}` |
| File I/O failure | 500 | `{"error": "..."}` |

---

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ox_data_object_manager` | `DataDictionary`, `DataObjectDefinition`, `DataStoreContainer`, `RelationshipDefinition` |
| `ox_workflow_abi` | Plugin ABI |
| `serde` / `serde_json` | Serialization |
