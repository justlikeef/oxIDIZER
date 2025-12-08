# ox_fileproc API Documentation

`ox_fileproc` is a Rust library designed for processing configuration files with support for multiple formats, variable substitution, and file inclusion.

## Core API

### `process_file`

```rust
pub fn process_file(path: &Path, max_depth: usize) -> Result<serde_json::Value>
```

Processes the file at the given path, handling format parsing, variable substitution, and recursive inclusions.

**Arguments:**
*   `path`: Path to the configuration file.
*   `max_depth`: Maximum allowed recursion depth for includes. Set to `0` for no limit. Recommended default is `5`.

**Returns:**
*   `Result<Value>`: On success, returns the parsed and processed JSON value. On failure, returns an error describing the issue (e.g., file not found, parse error, infinite recursion).

## Supported Formats

The library automatically detects the file format based on the extension:

*   **JSON** (`.json`)
*   **YAML** (`.yaml`, `.yml`)
*   **TOML** (`.toml`)
*   **XML** (`.xml`)
*   **JSON5** (`.json5`)
*   **KDL** (`.kdl`)

## Features

### 1. Variable Substitution

You can define variables and substitute them within string values in your configuration files.

**Syntax:** `${VAR_NAME}`

**Defining Variables:**
Variables are defined in a `substitutions` section (key) within the file. This can be either an inline map or a path to another file.

**Inline Definition:**
<details>
<summary>JSON</summary>

```json
{
  "substitutions": {
    "BASE_URL": "https://api.example.com",
    "TIMEOUT": "5000"
  },
  "url": "${BASE_URL}/v1",
  "timeout": "${TIMEOUT}"
}
```

</details>

<details>
<summary>YAML</summary>

```yaml
substitutions:
  BASE_URL: "https://api.example.com"
  TIMEOUT: "5000"
url: "${BASE_URL}/v1"
timeout: "${TIMEOUT}"
```

</details>

<details>
<summary>TOML</summary>

```toml
url = "${BASE_URL}/v1"
timeout = "${TIMEOUT}"

[substitutions]
BASE_URL = "https://api.example.com"
TIMEOUT = "5000"
```

</details>

<details>
<summary>XML</summary>

```xml
<root>
  <substitutions>
    <BASE_URL>https://api.example.com</BASE_URL>
    <TIMEOUT>5000</TIMEOUT>
  </substitutions>
  <url>${BASE_URL}/v1</url>
  <timeout>${TIMEOUT}</timeout>
</root>
```

</details>

<details>
<summary>JSON5</summary>

```json5
{
  substitutions: {
    BASE_URL: "https://api.example.com",
    TIMEOUT: "5000",
  },
  url: "${BASE_URL}/v1",
  timeout: "${TIMEOUT}",
}
```

</details>

<details>
<summary>KDL</summary>

```kdl
substitutions {
    BASE_URL "https://api.example.com"
    TIMEOUT "5000"
}
url "${BASE_URL}/v1"
timeout "${TIMEOUT}"
```

</details>

**File-Based Definition:**
<details>
<summary>JSON</summary>

```json
{
  "substitutions": "variables.json",
  "url": "${BASE_URL}/v1"
}
```

</details>

<details>
<summary>YAML</summary>

```yaml
substitutions: "variables.yaml"
url: "${BASE_URL}/v1"
```

</details>

<details>
<summary>TOML</summary>

```toml
substitutions = "variables.toml"
url = "${BASE_URL}/v1"
```

</details>

<details>
<summary>XML</summary>

```xml
<root>
  <substitutions>variables.xml</substitutions>
  <url>${BASE_URL}/v1</url>
</root>
```

</details>

<details>
<summary>JSON5</summary>

```json5
{
  substitutions: "variables.json5",
  url: "${BASE_URL}/v1",
}
```

</details>

<details>
<summary>KDL</summary>

```kdl
substitutions "variables.kdl"
url "${BASE_URL}/v1"
```

</details>

**Behavior:**
*   Variables follow lexical scoping: usage in the current file uses definitions from the current file or inherited from the parent.
*   "Last definition wins": If a variable is redefined in a file, it overrides the value passed from the parent for that file and its children.

### 2. File Inclusion

You can include the content of other files using the special `include` key.

**Syntax:**
```json
{
  "include": "relative/path/to/file.json",
  "other_key": "other_value"
}
```

**Behavior:**
*   The value of the `include` key must be a relative path to the file to be included.
*   The included file is processed recursively (handling its own substitutions and includes).
*   If the included file returns an Object (map), its keys are merged into the container object.
*   **Merge Priority**: Keys in the generic container object override keys from the included file if there is a conflict.

## Example Usage

<details>
<summary>JSON</summary>

**main.json**
```json
{
  "substitutions": {
    "ENV": "production"
  },
  "server": {
    "include": "server_config.json",
    "port": 8080
  }
}
```

**server_config.json**
```json
{
  "host": "0.0.0.0",
  "port": 9090,
  "environment": "${ENV}"
}
```

</details>

<details>
<summary>YAML</summary>

**main.yaml**
```yaml
substitutions:
  ENV: "production"
server:
  include: "server_config.yaml"
  port: 8080
```

**server_config.yaml**
```yaml
host: "0.0.0.0"
port: 9090
environment: "${ENV}"
```

</details>

<details>
<summary>TOML</summary>

**main.toml**
```toml
[substitutions]
ENV = "production"

[server]
include = "server_config.toml"
port = 8080
```

**server_config.toml**
```toml
host = "0.0.0.0"
port = 9090
environment = "${ENV}"
```

</details>

<details>
<summary>XML</summary>

**main.xml**
```xml
<root>
  <substitutions>
    <ENV>production</ENV>
  </substitutions>
  <server>
    <include>server_config.xml</include>
    <port>8080</port>
  </server>
</root>
```

**server_config.xml**
```xml
<root>
  <host>0.0.0.0</host>
  <port>9090</port>
  <environment>${ENV}</environment>
</root>
```

</details>

<details>
<summary>JSON5</summary>

**main.json5**
```json5
{
  substitutions: {
    ENV: "production",
  },
  server: {
    include: "server_config.json5",
    port: 8080,
  },
}
```

**server_config.json5**
```json5
{
  host: "0.0.0.0",
  port: 9090,
  environment: "${ENV}",
}
```

</details>

<details>
<summary>KDL</summary>

**main.kdl**
```kdl
substitutions {
    ENV "production"
}
server {
    include "server_config.kdl"
    port 8080
}
```

**server_config.kdl**
```kdl
host "0.0.0.0"
port 9090
environment "${ENV}"
```

</details>

**Result (Common for all formats):**
```json
{
  "server": {
    "host": "0.0.0.0",
    "port": 8080,
    "environment": "production"
  }
}
```
*Note: `port` is 8080 because `main` overrides the included value.*
