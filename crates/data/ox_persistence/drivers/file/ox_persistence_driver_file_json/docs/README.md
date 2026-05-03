# ox_persistence_driver_file_json

JSON file persistence driver. Stores each `GenericDataObject` as an individual JSON file
named `{id}.json` within the configured base directory.

---

## Driver Name

`ox_persistence_driver_file_json`

---

## Connection Config Parameters

| Parameter | Required | Description |
|---|---|---|
| `base_dir` | yes | Base directory for JSON files |
| `pretty_print` | no (default: false) | Indent JSON output for human readability |

---

## Storage Layout

```
{base_dir}/{location}/{id}.json
```

`location` (the table/container concept) becomes a subdirectory under `base_dir`.
Each GDO is a separate file; the file contains the serializable map as JSON.

Example file content:
```json
{
  "id":    ["550e8400-...", "uuid",   {}],
  "name":  ["Alice",       "string", {}],
  "score": ["99.5",        "float",  {"precision": "2"}]
}
```

---

## Storage Behavior

- `persist`: writes `{id}.json` (creates or overwrites).
- `restore`: reads `{id}.json`; returns 404-equivalent `OxDataError::NotFound` if absent.
- `fetch`: scans the directory for `*.json` files, reads each, applies equality filters.
  Returns a list of matching IDs. **Not suitable for large datasets** — performs a full
  directory scan.
- `delete`: removes `{id}.json`.

---

## `list_datasets`

Lists subdirectory names under `base_dir`. Each subdirectory is a dataset.

---

## `describe_dataset`

Reads the first JSON file found in `{base_dir}/{dataset_name}/` and infers field names
and types from its contents. Returns `DriverDatasetSchema`.

---

## Implementation Notes

- File operations are synchronous. Large datasets will incur significant I/O on fetch.
- No locking mechanism beyond filesystem rename-based atomic writes for `persist`.
- Suitable for configuration storage, test fixtures, or small reference datasets.
- `notify_lock_status_change` is a no-op.
