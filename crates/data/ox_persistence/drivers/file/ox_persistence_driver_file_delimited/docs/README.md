# ox_persistence_driver_file_delimited

Delimited text file persistence driver. Stores `GenericDataObject` records in CSV, TSV,
or other delimiter-separated value files. Multiple objects share one file per location
(unlike the JSON/YAML/XML drivers which use one file per object).

---

## Driver Name

`ox_persistence_driver_file_delimited`

---

## Connection Config Parameters

| Parameter | Required | Description |
|---|---|---|
| `base_dir` | yes | Base directory for delimited files |
| `delimiter` | no (default: `,`) | Field separator character |
| `has_header` | no (default: true) | Whether the first row is a header row |
| `quote_char` | no (default: `"`) | Character used to quote fields containing the delimiter |
| `file_extension` | no (default: `csv`) | File extension for created files |

---

## Storage Layout

```
{base_dir}/{location}.{file_extension}
```

`location` maps directly to a filename (without extension). All records for a given
location share one file. The first row (if `has_header = true`) is the field name header.

---

## Storage Behavior

- `persist`: appends or updates a row. Identifies the row by the `id` field value.
  If a row with the same ID exists, it is replaced; otherwise, a new row is appended.
  Row replacement rewrites the entire file.
- `restore`: reads the file and returns the row matching the given ID.
- `fetch`: reads the file and returns all matching row IDs (equality filter on columns).
- `delete`: rewrites the file without the row matching the given ID.

**Note:** All operations except append require reading and rewriting the entire file.
This driver is suitable only for small datasets (hundreds to low-thousands of records).

---

## `list_datasets`

Lists files with the configured extension in `base_dir`. Each file (without extension)
is a dataset name.

---

## `describe_dataset`

Reads the header row (if `has_header = true`) to determine field names. All fields are
reported as `ValueType::String` since delimiter files have no type metadata.

---

## Implementation Notes

- This driver is primarily useful for importing from or exporting to CSV/TSV files, for
  integration with spreadsheet workflows, and for simple configuration tables.
- `notify_lock_status_change` is a no-op.
- File rewrites are not atomic on all platforms. For write-heavy workloads, use a DB driver.
