# oxFileproc User Guide

`oxFileproc` is a powerful Rust library designed for advanced file processing, configuration management, and surgical code editing.

## Features

- **Recursive Configuration Loading**: Automatically resolves `include` directives in JSON, YAML, TOML, XML, JSON5, and KDL files.
- **Variable Substitution**: Supports strict `${{VAR}}` syntax. Environment variables are only resolved if explicitly enabled.

## Installation

Add `ox_fileproc` to your `Cargo.toml`:

```toml
[dependencies]
ox_fileproc = { git = "https://github.com/justlikeef/oxFileproc" }
```

```

## Default Configuration

The `Processor` uses secure defaults out of the box:
- **Strict Directory Includes**: `true`. Included directories must be fully readable. Any IO error (permission denied, missing file) causes the entire `process` call to fail.
- **Environment Variables**: `false`. The processor will NOT read from the system environment by default. You must explicitly opt-in via `.use_env_vars(true)`.
- **Root Directory**: `None`. By default, there is no jail. Use `.with_root_dir()` to restrict file access.
- **Max Depth**: `10`. To prevent stack overflows or infinite loops.

## Configuration Loading

Use `Processor` to load and fully resolve a configuration file.

```rust
use ox_fileproc::processor::Processor;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    // Default processor configuration:
    // - strict_dir_includes: true (Fail on IO errors)
    // - use_env_vars: false (No environment variable fallback)
    // - root_dir: None (No restriction, unless explicitly set)
    // - max_depth: 10
    
    // Note: `process_file` uses the default secure configuration:
    // - No environment variables
    // - Strict error handling
    // To enable environment variables or change defaults, use the Processor builder.
    
    // Example:
    // let config = Processor::new().use_env_vars(true).process("config.yaml")?;
    
    let config = Processor::new().process("config.yaml")?;
    println!("Loaded config: {}", config);
    Ok(())
}
```

### Variable Substitution

You can use variables in your configuration files with the syntax `${{VAR_NAME}}`. Variables are resolved from:
1.  **Inline Substitutions**: Defined in a `substitutions` block within the file.
2.  **Substitution Files**: Loaded via path strings in the `substitutions` block.
3.  **Environment Variables**: Only if enabled via `.use_env_vars(true)`.

### Escaping Rules
To treat a placeholder as a literal string, precede it with a backslash.
- **`\${{VAR}}`** -> Output: `${{VAR}}` (Literal, preserved)
- **`\\${{VAR}}`** -> Output: `\VALUE` (Backslash is escaped, variable IS substituted)
- **`\\\${{VAR}}`** -> Output: `\${{VAR}}` (Literal backslash + Literal token)

**Rule of Thumb**: An **odd** number of backslashes escapes the token (making it literal). An **even** number escapes the backslash itself (allowing substitution).

### Directives

- **`include`**: Merges another file or directory into the current object.
- **`substitutions`**: Defines variables local to the file/scope. Syntax: `"val": "my ${{VAR}}"`

## Security & Environment Variables

### Environment Variables
By default, environment variables are **not** substituted. To enable them (e.g., `${{HOME}}`):

```rust
let processor = Processor::new().use_env_vars(true);
```

### Root Directory Enforcement (Recommended)
To prevent path traversal attacks (e.g., config files accessing `/etc/passwd`), use the `Processor` builder to enforce a root directory.

```rust
use ox_fileproc::processor::Processor;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    // Only allow files within the "config" directory to be loaded
    let processor = Processor::new()
        .with_root_dir("config")
        .with_max_depth(5); // Prevent infinite recursion

    let config = processor.process("config/main.yaml")?;
    println!("Safely loaded: {}", config);
    Ok(())
}
```

### Strict Directory Includes
By default, `Processor` fails hard if an included directory contains unreadable files or errors. You can relax this (NOT recommended for security):

```rust
let lenient = Processor::new()
    .strict_dir_includes(false) // Skip errors instead of failing
    .process("config/main.yaml")?;
```


## Surgical Editing

Use `RawFile` to edit files without destroying formatting.

```rust
use ox_fileproc::{RawFile, Format};

fn main() -> anyhow::Result<()> {
    let mut raw = RawFile::open("config.yaml")?;
    
    // Find a node using a path query
    // Query format: "key/child[id=value]"
    if let Some(cursor) = raw.find("server/port").next() {
        // Update the value strictly at that position
        raw.update(cursor.span, "8080");
    }
    
    raw.save()?;
    Ok(())
}
```

### Query Syntax

- `key`: Selects child with key "key".
- `section/key`: Selects nested child.
- `items[id=my_item]`: Selects an item in a list where the `id` field equals `my_item`. 
