# oxFileproc Project Information

## Overview
**oxFileproc** is a robust Rust library designed for advanced configuration management and surgical file processing. It bridges the gap between static configuration loading and dynamic, structure-aware file editing.

## Key Capabilities

### 1. Recursive Configuration Loading
- **Multi-format Support**: JSON, YAML, TOML, XML, JSON5, KDL.
- **Deep Merging**: Recursively merges objects and lists across files.
- **Includes**: Directives (`include`, `merge`, `merge_recursive`) to compose configs from multiple files or directories.
- **Variable Substitution**: Dynamic value injection using `${{VAR}}`.

### 2. Surgical File Editing ("Cursor Engine")
- **Preserves Formatting**: Edits values in-place without re-serializing, preserving comments and whitespace.
- **Query Language**: XPath-like syntax for locating nodes (e.g., `section/list[id=val]/key`).
- **Format Agnostic Core**: Extensible `Scanner` trait allows adding support for new formats easily.

## Technical Stack
- **Language**: Rust (Edition 2024)
- **Core Dependencies**: `serde`, `serde_json`, `serde_yaml_ng` (aliased as `serde_yaml`), `toml`, `quick-xml`, `kdl`, `regex`.
- **Testing**: Comprehensive unit tests and integration tests.

## Directory Structure
```
oxFileproc/
├── src/
│   ├── lib.rs          # Crate root
│   ├── processor.rs    # Recursive loader & processor logic
│   ├── cursor.rs       # RawFile & Cursor definitions
│   ├── scanners/       # Format-specific scanners (YAML, JSON)
│   └── substitutor.rs  # Variable substitution logic
├── docs/
│   ├── USER_GUIDE.md        # Guide for end-users
│   └── DEVELOPMENT_GUIDE.md # Technical guide for developers
├── examples/           # Runnable examples
│   ├── load_config.rs
│   ├── edit_config.rs
│   ├── inclusions.rs
│   ├── replacements.rs
│   └── manual_cursor.rs
└── tests/              # Integration tests
```

## Getting Started
- **User Guide**: [docs/USER_GUIDE.md](./docs/USER_GUIDE.md)
- **Development Guide**: [docs/DEVELOPMENT_GUIDE.md](./docs/DEVELOPMENT_GUIDE.md)
- **AI Agent Guide**: [docs/AI_GUIDE.md](./docs/AI_GUIDE.md)
- **API Documentation**: [target/doc/ox_fileproc/index.html](./target/doc/ox_fileproc/index.html) (Local)

## License
[Determine License if known, otherwise placeholder]
