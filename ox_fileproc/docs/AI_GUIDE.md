# oxFileproc AI Guide
Token-efficient reference for AI agents.

## Core API
- **`process_file(path, depth) -> Value`**: Recursive loader. Resolves `include`, `merge`, `merge_recursive`, `${{VAR}}`.
- **`RawFile::open(path) -> RawFile`**: Loads text for surgical editing.
- **`RawFile::find(query) -> Iterator<Cursor>`**: XPath-ish query: `key/sub[id=val]/target`.
- **`RawFile::update(span, val)`**: `replace_range` on raw string.
- **`RawFile::append(cursor, item)`**: Inserts at `cursor.span.end`.
- **`Cursor { span, format, content_ref }`**: Byte range in `RawFile`.

## Query Syntax
- `arr[key=val]`: Filters list for item with matching key-value pair.
- `path/to/key`: Nested navigation.

## Key Files
- `src/processor.rs`: Recursive logic, merge rules.
- `src/cursor.rs`: `RawFile`, `Cursor`, query parsing.
- `src/scanners/`: Format heuristics (YAML/JSON).

## Editing Workflow
1. `raw = RawFile::open(p)`
2. `span = raw.find(q).next().span`
3. `raw.update(span, newVal)`
4. `raw.save()`

## Constraints
- Max recursion depth: Default 5.
- Formats: JSON, YAML, TOML, XML, JSON5, KDL.
- Surgical edits only touch `span`, preserving comments/whitespace.
