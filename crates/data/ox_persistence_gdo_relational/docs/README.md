# ox_persistence_gdo_relational

A meta persistence driver that stores relationships between GDOs rather than GDO attributes themselves. Delegates actual record storage to a configured inner driver.

## Type

`cdylib` — dynamically loaded persistence driver.

## Purpose

Tracks cross-GDO links: which objects are related to which, across which datasources, with an optional relationship type label. The GDOs themselves are stored by their own drivers; this driver records only the relationship edges.

## Relationships Schema

| Field | Type | Description |
|---|---|---|
| `id` | uuid | Relationship record ID |
| `source_gdo_id` | uuid | ID of the source GDO |
| `source_driver_name` | string | Driver storing the source GDO |
| `source_location` | string | Dataset/table for the source GDO |
| `target_gdo_id` | uuid | ID of the target GDO |
| `target_driver_name` | string | Driver storing the target GDO |
| `target_location` | string | Dataset/table for the target GDO |
| `relationship_type` | string | Optional label (e.g. `"parent"`, `"member"`) |

## Driver FFI Exports

Implements the standard persistence driver ABI:

| Symbol | Behaviour |
|---|---|
| `ox_driver_init` | Accepts JSON config; initialises inner driver |
| `ox_driver_persist` | Stores a relationship record via the inner driver |
| `ox_driver_restore` | Loads a relationship record by ID |
| `ox_driver_fetch` | Returns relationship IDs matching an equality filter |
| `ox_driver_delete` | Removes a relationship record |
| `ox_driver_describe_dataset` | Returns the `relationships` schema above |
| `ox_driver_get_driver_metadata` | Returns `DriverMetadata` JSON |

## Config

```yaml
inner_driver: postgres          # driver name to use for actual storage
inner_location: gdo_relationships  # table/collection name
```

## Usage

Register under a driver name (e.g. `ox_persistence_gdo_relational`) in `conf/drivers.yaml`. Then use the data broker or `ox_persistence_datasource_manager` to create, query, and delete relationship records.

```json
POST /data/ox_persistence_gdo_relational/persist
{
  "source_gdo_id": ["550e8400-...", "uuid", {}],
  "target_gdo_id": ["6ba7b810-...", "uuid", {}],
  "relationship_type": ["parent", "string", {}]
}
```

## See Also

- [Data system architecture](../../../docs/architecture.md)
- [Driver FFI ABI](../../../docs/administration.md#driver-ffi-abi)
- [spec/drivers.md](../../../spec/drivers.md)
