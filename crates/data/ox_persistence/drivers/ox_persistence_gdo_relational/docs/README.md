# ox_persistence_gdo_relational

Meta-driver that stores cross-GDO relationships (not GDO data itself). Delegates actual
storage to a configured inner driver.

---

## Purpose

Records and queries directed links between GDOs stored in different datasources. A
relationship record captures source GDO, target GDO, and the semantic type of the
connection (e.g., `"one-to-many"`, `"orders"`).

---

## Relationships Schema

| Field | Type | Description |
|---|---|---|
| `id` | uuid | Relationship record ID |
| `source_gdo_id` | uuid | ID of the source GDO |
| `source_driver_name` | string | Driver holding the source GDO |
| `source_location` | string | Location in that driver |
| `target_gdo_id` | uuid | ID of the target GDO |
| `target_driver_name` | string | Driver holding the target GDO |
| `target_location` | string | Location in that driver |
| `relationship_type` | string | Semantic type, e.g. `"one-to-many"` |
| `relationship_name` | string | Named role, e.g. `"orders"` |

---

## Configuration (driver init JSON)

| Parameter | Required | Description |
|---|---|---|
| `internal_driver_name` | yes | Name of the inner driver in `PERSISTENCE_DRIVER_REGISTRY` |
| `internal_location` | yes | Location string passed to the inner driver |

---

## C Exports

| Symbol | Description |
|---|---|
| `get_driver_metadata_json()` | Returns JSON `DriverMetadata` |
| `create_driver()` | Returns null — requires config; use `ox_driver_init` |
| `destroy_driver(ptr)` | Drops the driver instance |

---

## Implementation Notes

- This driver does not implement `describe_dataset` for GDO attributes — it reports the
  `"relationships"` schema only.
- It is designed to work alongside other drivers: GDO data is held by a primary driver
  (e.g., postgres); relationships are stored by this driver (possibly using the same
  inner postgres driver, just a different table).
- `RelationshipDefinition` entries in `DataObjectDefinition` instruct `DataObjectManager`
  to join data across containers. This driver is not required for same-datasource
  relationships — those are handled by `QueryEngine` join nodes directly.
