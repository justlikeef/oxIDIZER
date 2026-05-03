# ox_persistence_dictionary_manager

REST plugin for managing persistence-layer dictionary definitions. Bridges
`ox_persistence` driver metadata with the data dictionary maintained by
`ox_data_object_dictionary_manager`.

---

## Purpose

Exposes persistence-specific schema information (datasource schemas, column definitions)
as a REST API and keeps the `DataDictionary` synchronized with the physical schemas
reported by loaded drivers.

---

## Key Responsibilities

- Serve driver schema information (`DriverDatasetSchema`) over HTTP
- Accept driver schema imports and translate them into `DataStoreContainer` entries in
  the dictionary
- Coordinate with `ox_persistence_datasource_manager` for discovering datasets in
  configured datasources

---

## Integration Points

- **`ox_persistence_datasource_manager`** calls this plugin's endpoints during dataset
  auto-import to create `DataStoreContainer` definitions.
- **`ox_data_object_dictionary_manager`** stores the resulting definitions; this plugin
  acts as the translation layer between the physical driver schema and the logical
  dictionary format.

---

## Implementation Notes

- Driver schemas are fetched via `ox_driver_describe_dataset()` from the loaded driver
  library. The returned `DriverDatasetSchema` includes field names, types, and
  optional descriptions.
- Schema fields are mapped to `DataStoreField` entries with `ValueType` inferred from
  the driver's data type strings.
