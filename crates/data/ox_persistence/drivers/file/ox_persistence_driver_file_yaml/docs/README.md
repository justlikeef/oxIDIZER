# ox_persistence_driver_file_yaml

YAML file persistence driver. Stores each `GenericDataObject` as an individual YAML file
named `{id}.yaml` within the configured base directory.

---

## Driver Name

`ox_persistence_driver_file_yaml`

---

## Connection Config Parameters

| Parameter | Required | Description |
|---|---|---|
| `base_dir` | yes | Base directory for YAML files |

---

## Storage Layout

```
{base_dir}/{location}/{id}.yaml
```

`location` becomes a subdirectory. Each GDO is a separate `.yaml` file.

Example file content:
```yaml
id:
  - "550e8400-..."
  - uuid
  - {}
name:
  - Alice
  - string
  - {}
score:
  - "99.5"
  - float
  - precision: "2"
```

---

## Storage Behavior

Same semantics as the JSON driver: individual files per GDO, directory scan for fetch,
full-directory for `list_datasets`, first-file type inference for `describe_dataset`.

---

## Implementation Notes

- Suitable for human-editable configuration objects and small reference data.
- YAML is parsed with `serde_yaml`. Files are written atomically via rename.
- `notify_lock_status_change` is a no-op.
- Fetch performs a full directory scan — not suitable for large datasets.
