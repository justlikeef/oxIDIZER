# oxFileproc Development Guide

Welcome to the professional development guide for `oxFileproc`. This document provides a deep dive into the library's architecture, design decisions, and internal logic for developers and AI agents wishing to extend or maintain the project.

## Quick Links
- **API Documentation**: [RustDoc (Local)](../target/doc/ox_fileproc/index.html)
- **Practical Examples**:
    - [Basic Loading](../examples/load_config.rs)
    - [Surgical Editing](../examples/edit_config.rs)
    - [Complex Replacements](../examples/replacements.rs)
    - [Inclusion Logic](../examples/inclusions.rs)

---

## Architecture & Design Philosophy

`oxFileproc` is designed to be a high-performance, structure-aware tool for configuration management. It addresses two primary use cases:

### 1. The Recursive Processor (`processor.rs`)
The processor manages complex configuration trees.
- **Cycle Detection**: Canonicalizes paths and tracks them in `visited` to prevent infinite loops.
- **Smart Merging**: Supports identity-based merging for lists (using `id` fields).

### 2. The Cursor Engine (`cursor.rs`)
The "Surgical" engine allows editing without full re-serialization.
- **`RawFile`**: The main interface for file mutation.
- **`Cursor`**: A non-owning window into a specific text segment.
- **`Scanner`**: Pluggable heuristics for finding spans in different formats.

## Internal Implementation Details

### Surgical Update Logic
Unlike standard `serde` workflows, `oxFileproc` keeps the file as a raw string. When `update(span, new_val)` is called, it performs a standard string `replace_range`. The `Scanner` is responsible for ensuring the `span` covers exactly what the user intends to replace.

### Scanner Heuristics
- **YAML**: Relies on indentation levels for block identification.
- **JSON**: Uses regex for keys and brace-counting for structure-aware span identification.

## Extension & Contributions

### Adding a New Format
1. Implement the `Scanner` trait in `src/scanners/`.
2. Register the format in `src/cursor.rs`.
3. Add a representative example in `examples/`.

## Testing
Always run the full suite before submitting:
```bash
cargo test
cargo check --examples
```
