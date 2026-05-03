# ox_persistence_driver_file_xml

XML file persistence driver. Stores each `GenericDataObject` as an individual XML file
named `{id}.xml` within the configured base directory.

---

## Driver Name

`ox_persistence_driver_file_xml`

---

## Connection Config Parameters

| Parameter | Required | Description |
|---|---|---|
| `base_dir` | yes | Base directory for XML files |
| `root_element` | no (default: `"object"`) | Name of the XML root element |
| `pretty_print` | no (default: false) | Indent XML output |

---

## Storage Layout

```
{base_dir}/{location}/{id}.xml
```

Example file structure:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<object>
  <field name="id" type="uuid"><value>550e8400-...</value></field>
  <field name="name" type="string"><value>Alice</value></field>
  <field name="score" type="float" precision="2"><value>99.5</value></field>
</object>
```

---

## Storage Behavior

Same semantics as the JSON and YAML drivers: one file per GDO, directory scan for
`fetch`, full-directory for `list_datasets`, first-file type inference for
`describe_dataset`.

---

## Implementation Notes

- Parsed with `quick-xml` or `roxmltree`. Fields and type parameters are stored as XML
  element attributes for machine readability.
- Suitable for integration with legacy XML-based systems or audit exports.
- `notify_lock_status_change` is a no-op.
- Fetch is a full directory scan — not suitable for large datasets.
