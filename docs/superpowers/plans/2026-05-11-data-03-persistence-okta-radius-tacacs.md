# Okta + RADIUS + TACACS+ Persistence Drivers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `ox_persistence_okta`, `ox_persistence_radius`, and `ox_persistence_tacacs` — three `PersistenceDriver` cdylib crates that translate the canonical IAM schema to and from their respective backends, with automatic overflow routing to a local SQL store for entities those backends cannot natively hold.

**Architecture:** Each driver declares which canonical entity locations it supports natively via `list_datasets`. Locations not in that list (`PermissionGrant`, `SessionRecord` for RADIUS/TACACS+) are not persisted by the driver — callers must route these to an overflow store. The Okta driver injects an `OktaHttpClient` trait so tests never touch the real Okta API. RADIUS and TACACS+ are read-extraction-only (membership from Access-Accept/accounting responses) and explicitly return `OxDataError::DriverError("not supported")` from `persist` on locations they cannot write.

**Tech Stack:** Rust, `reqwest = "0.12"` (rustls-tls, json, blocking), `serde_json = "1"`, `tokio = "1"` (rt-multi-thread), `ox_persistence`, `ox_data_object`, `ox_data_error`, `ox_type_converter`, `libc`

---

## File Structure

```
crates/data/ox_persistence/drivers/cloud/
  ox_persistence_okta/
    Cargo.toml
    src/
      lib.rs          — PersistenceDriver impl + FFI exports
      http_client.rs  — OktaHttpClient trait + RealOktaHttpClient + MockOktaHttpClient
      mapping.rs      — Okta REST path and JSON field mapping for each canonical location
      error.rs        — OktaDriverError -> OxDataError
    tests/
      okta_tests.rs   — unit tests using MockOktaHttpClient

crates/data/ox_persistence/drivers/network/
  ox_persistence_radius/
    Cargo.toml
    src/
      lib.rs          — PersistenceDriver impl (read-only) + FFI exports
      response_parser.rs  — parse RADIUS Access-Accept attributes into canonical map
    tests/
      radius_tests.rs

  ox_persistence_tacacs/
    Cargo.toml
    src/
      lib.rs              — PersistenceDriver impl (read-only) + FFI exports
      response_parser.rs  — parse TACACS+ AV pairs into canonical map
    tests/
      tacacs_tests.rs
```

Root workspace file to modify:
```
Cargo.toml  — add three new member entries
```

---

## Task 1: Workspace registration and scaffolding

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/Cargo.toml`
- Create: `crates/data/ox_persistence/drivers/network/ox_persistence_radius/Cargo.toml`
- Create: `crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/Cargo.toml`

- [ ] **Step 1: Add all three crates to workspace**

In `/var/repos/oxIDIZER/Cargo.toml`, inside the `members = [...]` array, after the line `"crates/data/ox_persistence/drivers/file/ox_persistence_driver_file_delimited",` add:

```toml
    "crates/data/ox_persistence/drivers/cloud/ox_persistence_okta",
    "crates/data/ox_persistence/drivers/network/ox_persistence_radius",
    "crates/data/ox_persistence/drivers/network/ox_persistence_tacacs",
```

- [ ] **Step 2: Create `ox_persistence_okta/Cargo.toml`**

Create `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/Cargo.toml`:

```toml
[package]
name = "ox_persistence_okta"
version = "0.1.0"
license = "GPL-3.0-only"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_persistence   = { path = "../../../../ox_persistence" }
ox_data_object   = { path = "../../../../ox_data_object" }
ox_data_error    = { path = "../../../../ox_data_error" }
ox_type_converter = { path = "../../../../ox_type_converter" }
reqwest          = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "blocking"] }
serde_json       = "1"
libc             = "0.2"

[dev-dependencies]
tokio = { version = "1", features = ["rt", "macros"] }
```

- [ ] **Step 3: Create `ox_persistence_radius/Cargo.toml`**

Create `crates/data/ox_persistence/drivers/network/ox_persistence_radius/Cargo.toml`:

```toml
[package]
name = "ox_persistence_radius"
version = "0.1.0"
license = "GPL-3.0-only"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_persistence   = { path = "../../../../ox_persistence" }
ox_data_error    = { path = "../../../../ox_data_error" }
ox_type_converter = { path = "../../../../ox_type_converter" }
serde_json       = "1"
libc             = "0.2"
```

- [ ] **Step 4: Create `ox_persistence_tacacs/Cargo.toml`**

Create `crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/Cargo.toml`:

```toml
[package]
name = "ox_persistence_tacacs"
version = "0.1.0"
license = "GPL-3.0-only"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
ox_persistence   = { path = "../../../../ox_persistence" }
ox_data_error    = { path = "../../../../ox_data_error" }
ox_type_converter = { path = "../../../../ox_type_converter" }
serde_json       = "1"
libc             = "0.2"
```

- [ ] **Step 5: Verify workspace parses**

```bash
cd /var/repos/oxIDIZER && cargo metadata --no-deps --format-version 1 \
  | python3 -c "import sys,json; pkgs=[p['name'] for p in json.load(sys.stdin)['packages']]; \
    print(all(n in pkgs for n in ['ox_persistence_okta','ox_persistence_radius','ox_persistence_tacacs']))"
```

Expected: `True`

- [ ] **Step 6: Commit scaffolding**

```bash
git add Cargo.toml \
  crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/Cargo.toml \
  crates/data/ox_persistence/drivers/network/ox_persistence_radius/Cargo.toml \
  crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/Cargo.toml
git commit -m "chore(data): scaffold ox_persistence_okta, ox_persistence_radius, ox_persistence_tacacs workspace members"
```

---

## Task 2: Okta HTTP client abstraction (`http_client.rs` + `error.rs`)

**Files:**
- Create: `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/error.rs`
- Create: `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/http_client.rs`
- Create: `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/tests/okta_tests.rs`

- [ ] **Step 1: Write the failing HTTP client tests**

Create `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/tests/okta_tests.rs`:

```rust
mod http_client_tests {
    use ox_persistence_okta::http_client::{MockOktaHttpClient, OktaHttpClient, OktaRequest, OktaResponse};
    use std::collections::HashMap;

    #[test]
    fn mock_get_returns_canned_response() {
        let mock = MockOktaHttpClient::new();
        mock.expect_get(
            "/api/v1/users/alice",
            serde_json::json!({
                "id": "00uABCD1234",
                "profile": { "login": "alice", "displayName": "Alice" }
            }),
        );

        let resp = mock.get("/api/v1/users/alice").expect("get failed");
        assert_eq!(resp["id"], "00uABCD1234");
        assert_eq!(resp["profile"]["login"], "alice");
    }

    #[test]
    fn mock_post_records_request_body() {
        let mock = MockOktaHttpClient::new();
        mock.expect_post(
            "/api/v1/users",
            serde_json::json!({ "id": "00uNEW0001", "profile": { "login": "newuser" } }),
        );

        let body = serde_json::json!({ "profile": { "login": "newuser", "displayName": "New User" } });
        let resp = mock.post("/api/v1/users", &body).expect("post failed");
        assert_eq!(resp["id"], "00uNEW0001");

        let recorded = mock.recorded_posts();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["profile"]["login"], "newuser");
    }

    #[test]
    fn mock_put_records_call() {
        let mock = MockOktaHttpClient::new();
        mock.expect_put("/api/v1/groups/grp001/users/usr001", serde_json::json!({}));
        mock.put("/api/v1/groups/grp001/users/usr001", &serde_json::json!({}))
            .expect("put failed");
        assert_eq!(mock.recorded_puts().len(), 1);
    }
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_okta -- http_client_tests 2>&1 | head -20
```

Expected: compile error — modules not found.

- [ ] **Step 3: Implement `error.rs`**

Create `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/error.rs`:

```rust
//! OktaDriverError to OxDataError conversion.

use ox_data_error::OxDataError;

#[derive(Debug)]
pub enum OktaDriverError {
    HttpError(String),
    NotFound(String),
    InvalidConfig(String),
    DeserializationError(String),
}

impl From<OktaDriverError> for OxDataError {
    fn from(e: OktaDriverError) -> Self {
        match e {
            OktaDriverError::HttpError(m)           => OxDataError::DriverError(format!("Okta HTTP error: {}", m)),
            OktaDriverError::NotFound(id)           => OxDataError::InternalError(format!("Okta entity not found: {}", id)),
            OktaDriverError::InvalidConfig(m)       => OxDataError::DriverError(format!("Okta config error: {}", m)),
            OktaDriverError::DeserializationError(m) => OxDataError::InternalError(format!("Okta deserialize error: {}", m)),
        }
    }
}
```

- [ ] **Step 4: Implement `http_client.rs`**

Create `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/http_client.rs`:

```rust
//! Okta HTTP client abstraction.  Production uses RealOktaHttpClient (reqwest blocking).
//! Tests use MockOktaHttpClient which returns pre-programmed responses.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ox_data_error::OxDataError;
use serde_json::Value;

// Unused type aliases kept for documentation clarity.
pub type OktaRequest = Value;
pub type OktaResponse = Value;

/// Minimal HTTP operations needed by the Okta driver.
pub trait OktaHttpClient: Send + Sync {
    fn get(&self, path: &str) -> Result<Value, OxDataError>;
    fn post(&self, path: &str, body: &Value) -> Result<Value, OxDataError>;
    fn put(&self, path: &str, body: &Value) -> Result<Value, OxDataError>;
    fn delete(&self, path: &str) -> Result<(), OxDataError>;
}

// ---------------------------------------------------------------------------
// Real client backed by reqwest blocking
// ---------------------------------------------------------------------------

pub struct RealOktaHttpClient {
    base_url: String,   // e.g. "https://yourorg.okta.com"
    api_token: String,  // SSWS token
}

impl RealOktaHttpClient {
    pub fn new(domain: &str, api_token: &str) -> Self {
        let base_url = if domain.starts_with("https://") {
            domain.to_string()
        } else {
            format!("https://{}", domain)
        };
        Self { base_url, api_token: api_token.to_string() }
    }

    fn full_url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    fn client(&self) -> reqwest::blocking::Client {
        reqwest::blocking::Client::new()
    }

    fn auth_header(&self) -> String {
        format!("SSWS {}", self.api_token)
    }
}

impl OktaHttpClient for RealOktaHttpClient {
    fn get(&self, path: &str) -> Result<Value, OxDataError> {
        let resp = self.client()
            .get(&self.full_url(path))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .send()
            .map_err(|e| OxDataError::DriverError(format!("Okta GET {}: {}", path, e)))?;

        if resp.status().is_success() {
            resp.json::<Value>()
                .map_err(|e| OxDataError::InternalError(format!("Okta GET parse: {}", e)))
        } else {
            Err(OxDataError::DriverError(format!("Okta GET {} returned {}", path, resp.status())))
        }
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, OxDataError> {
        let resp = self.client()
            .post(&self.full_url(path))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .json(body)
            .send()
            .map_err(|e| OxDataError::DriverError(format!("Okta POST {}: {}", path, e)))?;

        if resp.status().is_success() {
            resp.json::<Value>()
                .map_err(|e| OxDataError::InternalError(format!("Okta POST parse: {}", e)))
        } else {
            Err(OxDataError::DriverError(format!("Okta POST {} returned {}", path, resp.status())))
        }
    }

    fn put(&self, path: &str, body: &Value) -> Result<Value, OxDataError> {
        let resp = self.client()
            .put(&self.full_url(path))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .json(body)
            .send()
            .map_err(|e| OxDataError::DriverError(format!("Okta PUT {}: {}", path, e)))?;

        if resp.status().is_success() {
            resp.json::<Value>()
                .map_err(|e| OxDataError::InternalError(format!("Okta PUT parse: {}", e)))
        } else {
            Err(OxDataError::DriverError(format!("Okta PUT {} returned {}", path, resp.status())))
        }
    }

    fn delete(&self, path: &str) -> Result<(), OxDataError> {
        let resp = self.client()
            .delete(&self.full_url(path))
            .header("Authorization", self.auth_header())
            .send()
            .map_err(|e| OxDataError::DriverError(format!("Okta DELETE {}: {}", path, e)))?;

        if resp.status().is_success() || resp.status().as_u16() == 204 {
            Ok(())
        } else {
            Err(OxDataError::DriverError(format!("Okta DELETE {} returned {}", path, resp.status())))
        }
    }
}

// ---------------------------------------------------------------------------
// Mock client (tests only)
// ---------------------------------------------------------------------------

/// MockOktaHttpClient stores canned GET/POST responses keyed by path and records
/// what was actually posted/put so tests can assert on outbound request bodies.
#[derive(Default)]
pub struct MockOktaHttpClient {
    get_responses:   Arc<Mutex<HashMap<String, Value>>>,
    post_responses:  Arc<Mutex<HashMap<String, Value>>>,
    put_responses:   Arc<Mutex<HashMap<String, Value>>>,
    recorded_posts:  Arc<Mutex<Vec<Value>>>,
    recorded_puts:   Arc<Mutex<Vec<Value>>>,
}

impl MockOktaHttpClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn expect_get(&self, path: &str, response: Value) {
        self.get_responses.lock().unwrap().insert(path.to_string(), response);
    }

    pub fn expect_post(&self, path: &str, response: Value) {
        self.post_responses.lock().unwrap().insert(path.to_string(), response);
    }

    pub fn expect_put(&self, path: &str, response: Value) {
        self.put_responses.lock().unwrap().insert(path.to_string(), response);
    }

    pub fn recorded_posts(&self) -> Vec<Value> {
        self.recorded_posts.lock().unwrap().clone()
    }

    pub fn recorded_puts(&self) -> Vec<Value> {
        self.recorded_puts.lock().unwrap().clone()
    }
}

impl OktaHttpClient for MockOktaHttpClient {
    fn get(&self, path: &str) -> Result<Value, OxDataError> {
        self.get_responses
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| OxDataError::DriverError(format!("MockOktaHttpClient: no GET canned for {}", path)))
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, OxDataError> {
        self.recorded_posts.lock().unwrap().push(body.clone());
        self.post_responses
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| OxDataError::DriverError(format!("MockOktaHttpClient: no POST canned for {}", path)))
    }

    fn put(&self, path: &str, body: &Value) -> Result<Value, OxDataError> {
        self.recorded_puts.lock().unwrap().push(body.clone());
        self.put_responses
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| OxDataError::DriverError(format!("MockOktaHttpClient: no PUT canned for {}", path)))
    }

    fn delete(&self, _path: &str) -> Result<(), OxDataError> {
        Ok(())
    }
}
```

- [ ] **Step 5: Run HTTP client tests**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_okta -- http_client_tests 2>&1 | tail -15
```

Expected: 3 tests pass.

- [ ] **Step 6: Commit HTTP client abstraction**

```bash
git add crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/error.rs \
        crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/http_client.rs \
        crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/tests/okta_tests.rs
git commit -m "feat(okta-driver): add OktaHttpClient trait, RealOktaHttpClient, and MockOktaHttpClient"
```

---

## Task 3: Okta schema mapping (`mapping.rs`)

**Files:**
- Create: `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/mapping.rs`

The Okta REST API uses specific JSON paths for each canonical entity:

| Canonical location | Okta API path | Notes |
|---|---|---|
| `principals` | `GET/POST /api/v1/users` | `id` = Okta user id; `profile.login` = `principal_id` |
| `groups` | `GET/POST /api/v1/groups` | `id` = Okta group id; `profile.name` = `group_id` |
| `members` | `PUT /api/v1/groups/{groupId}/users/{userId}` | Write only; read via `GET /api/v1/users/{userId}/groups` |
| `grants` | Not natively supported — overflow store only | |
| `sessions` | Not natively supported — overflow store only | |

- [ ] **Step 1: No new tests needed** — mapping is covered by the driver tests in Task 4.

- [ ] **Step 2: Implement `mapping.rs`**

Create `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/mapping.rs`:

```rust
//! Translates canonical IAM serializable_maps to/from Okta REST JSON bodies.

use std::collections::HashMap;
use ox_type_converter::ValueType;
use serde_json::{json, Value};

pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// Returns the canonical field value from a map, or empty string.
pub fn canon_get(map: &CanonicalMap, key: &str) -> String {
    map.get(key).map(|(v, _, _)| v.clone()).unwrap_or_default()
}

/// Builds an Okta user creation/update body from a canonical `principals` map.
///
/// Okta user body format:
/// ```json
/// { "profile": { "login": "<principal_id>", "displayName": "<display_name>" } }
/// ```
pub fn principal_to_okta_body(map: &CanonicalMap) -> Value {
    json!({
        "profile": {
            "login": canon_get(map, "principal_id"),
            "displayName": canon_get(map, "display_name"),
            "oxSource": canon_get(map, "source"),
            "oxTenantId": canon_get(map, "tenant_id"),
        }
    })
}

/// Extracts a canonical `principals` map from an Okta user JSON object.
pub fn okta_user_to_canonical(user: &Value) -> CanonicalMap {
    let mut map = CanonicalMap::new();
    let profile = &user["profile"];
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();

    map.insert("principal_id".to_string(), (s(&profile["login"]),      ValueType::String, HashMap::new()));
    map.insert("display_name".to_string(), (s(&profile["displayName"]),ValueType::String, HashMap::new()));
    map.insert("source".to_string(),       (s(&profile["oxSource"]),    ValueType::String, HashMap::new()));
    map.insert("tenant_id".to_string(),    (s(&profile["oxTenantId"]), ValueType::String, HashMap::new()));
    // Store the Okta internal ID as an annotation — callers can use it for group membership calls.
    map.insert("_okta_id".to_string(),     (s(&user["id"]),             ValueType::String, HashMap::new()));
    map
}

/// Builds an Okta group creation body from a canonical `groups` map.
///
/// Okta group body format:
/// ```json
/// { "profile": { "name": "<group_id>", "description": "<name>" } }
/// ```
pub fn group_to_okta_body(map: &CanonicalMap) -> Value {
    json!({
        "profile": {
            "name": canon_get(map, "group_id"),
            "description": canon_get(map, "name"),
            "oxSource": canon_get(map, "source"),
            "oxTenantId": canon_get(map, "tenant_id"),
        }
    })
}

/// Extracts a canonical `groups` map from an Okta group JSON object.
pub fn okta_group_to_canonical(group: &Value) -> CanonicalMap {
    let mut map = CanonicalMap::new();
    let profile = &group["profile"];
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();

    map.insert("group_id".to_string(),  (s(&profile["name"]),        ValueType::String, HashMap::new()));
    map.insert("name".to_string(),      (s(&profile["description"]), ValueType::String, HashMap::new()));
    map.insert("source".to_string(),    (s(&profile["oxSource"]),    ValueType::String, HashMap::new()));
    map.insert("tenant_id".to_string(), (s(&profile["oxTenantId"]), ValueType::String, HashMap::new()));
    map.insert("_okta_id".to_string(),  (s(&group["id"]),            ValueType::String, HashMap::new()));
    map
}

/// Returns the Okta REST path for adding a user to a group.
/// `group_okta_id` is the Okta internal group id (not the canonical group_id).
/// `user_okta_id` is the Okta internal user id.
pub fn group_membership_put_path(group_okta_id: &str, user_okta_id: &str) -> String {
    format!("/api/v1/groups/{}/users/{}", group_okta_id, user_okta_id)
}

/// Returns the Okta REST path for listing a user's groups.
pub fn user_groups_path(user_okta_id: &str) -> String {
    format!("/api/v1/users/{}/groups", user_okta_id)
}

/// Returns the list of locations this driver supports natively (no overflow needed).
pub fn supported_locations() -> Vec<String> {
    vec![
        "principals".to_string(),
        "groups".to_string(),
        "members".to_string(),
    ]
}
```

---

## Task 4: `OktaPersistenceDriver` — `PersistenceDriver` impl and FFI (`lib.rs`)

**Files:**
- Create: `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/lib.rs`
- Extend: `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/tests/okta_tests.rs`

- [ ] **Step 1: Write the failing driver tests**

Add to `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/tests/okta_tests.rs`:

```rust
mod driver_tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use ox_persistence::PersistenceDriver;
    use ox_persistence_okta::OktaPersistenceDriver;
    use ox_persistence_okta::http_client::MockOktaHttpClient;
    use ox_type_converter::ValueType;

    fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
        (v.to_string(), ValueType::String, HashMap::new())
    }

    fn make_driver() -> (OktaPersistenceDriver, Arc<MockOktaHttpClient>) {
        let mock = Arc::new(MockOktaHttpClient::new());
        let driver = OktaPersistenceDriver::new_with_client(mock.clone());
        (driver, mock)
    }

    #[test]
    fn persist_principal_posts_to_okta_users() {
        let (driver, mock) = make_driver();
        mock.expect_post("/api/v1/users", serde_json::json!({
            "id": "00u001",
            "profile": { "login": "alice" }
        }));

        let mut data = HashMap::new();
        data.insert("principal_id".to_string(), str_entry("alice"));
        data.insert("display_name".to_string(), str_entry("Alice"));
        data.insert("source".to_string(),       str_entry("Okta"));
        data.insert("tenant_id".to_string(),    str_entry("t1"));

        driver.persist(&data, "principals").expect("persist failed");
        let posts = mock.recorded_posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0]["profile"]["login"], "alice");
    }

    #[test]
    fn restore_principal_by_id_calls_get_user() {
        let (driver, mock) = make_driver();
        mock.expect_get("/api/v1/users/alice", serde_json::json!({
            "id": "00u001",
            "profile": {
                "login": "alice",
                "displayName": "Alice Liddell",
                "oxSource": "Okta",
                "oxTenantId": "t1"
            }
        }));

        let restored = driver.restore("principals", "alice").expect("restore failed");
        assert_eq!(restored.get("principal_id").unwrap().0, "alice");
        assert_eq!(restored.get("display_name").unwrap().0, "Alice Liddell");
    }

    #[test]
    fn fetch_principals_calls_list_users() {
        let (driver, mock) = make_driver();
        mock.expect_get("/api/v1/users", serde_json::json!([
            { "id": "00u001", "profile": { "login": "alice", "displayName": "Alice", "oxSource": "Okta", "oxTenantId": "t1" } },
            { "id": "00u002", "profile": { "login": "bob",   "displayName": "Bob",   "oxSource": "Okta", "oxTenantId": "t1" } }
        ]));

        let filter = HashMap::new();
        let ids = driver.fetch(&filter, "principals").expect("fetch failed");
        assert!(ids.contains(&"alice".to_string()));
        assert!(ids.contains(&"bob".to_string()));
    }

    #[test]
    fn persist_group_posts_to_okta_groups() {
        let (driver, mock) = make_driver();
        mock.expect_post("/api/v1/groups", serde_json::json!({
            "id": "grp001",
            "profile": { "name": "ops", "description": "Operations" }
        }));

        let mut data = HashMap::new();
        data.insert("group_id".to_string(),  str_entry("ops"));
        data.insert("name".to_string(),      str_entry("Operations"));
        data.insert("source".to_string(),    str_entry("Okta"));
        data.insert("tenant_id".to_string(), str_entry("t1"));

        driver.persist(&data, "groups").expect("persist failed");
        assert_eq!(mock.recorded_posts().len(), 1);
    }

    #[test]
    fn persist_grant_returns_not_supported() {
        let (driver, _) = make_driver();
        let mut data = HashMap::new();
        data.insert("node_path".to_string(),     str_entry("com.justlikeef.data"));
        data.insert("group_id".to_string(),       str_entry("ops"));
        data.insert("operation_name".to_string(), str_entry("read"));
        data.insert("allow_deny".to_string(),     str_entry("Allow"));
        data.insert("tenant_id".to_string(),      str_entry("t1"));

        let result = driver.persist(&data, "grants");
        assert!(result.is_err(), "grants should not be supported natively");
    }

    #[test]
    fn list_datasets_returns_supported_locations() {
        let (driver, _) = make_driver();
        let datasets = driver.list_datasets(&HashMap::new()).expect("list failed");
        assert!(datasets.contains(&"principals".to_string()));
        assert!(datasets.contains(&"groups".to_string()));
        assert!(datasets.contains(&"members".to_string()));
        // grants and sessions must NOT be in the list (overflow responsibility of caller)
        assert!(!datasets.contains(&"grants".to_string()));
        assert!(!datasets.contains(&"sessions".to_string()));
    }
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_okta -- driver_tests 2>&1 | head -25
```

Expected: compile error — `OktaPersistenceDriver` not defined.

- [ ] **Step 3: Implement `lib.rs`**

Create `crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/lib.rs`:

```rust
//! ox_persistence_okta — Okta REST API persistence driver for canonical IAM entities.
//!
//! Natively supports: principals (Okta users), groups (Okta groups), members (group membership).
//! Does NOT support: grants, sessions — callers must route those to an overflow store.

pub mod error;
pub mod http_client;
pub mod mapping;

use std::collections::HashMap;
use std::sync::Arc;
use ox_data_error::OxDataError;
use ox_persistence::{
    PersistenceDriver, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer,
};
use ox_type_converter::ValueType;
use serde_json::Value;

use http_client::{OktaHttpClient, RealOktaHttpClient};
use mapping::{
    principal_to_okta_body, okta_user_to_canonical,
    group_to_okta_body, okta_group_to_canonical,
    supported_locations, canon_get, CanonicalMap,
};

/// The Okta persistence driver.
pub struct OktaPersistenceDriver {
    client: Arc<dyn OktaHttpClient>,
}

impl OktaPersistenceDriver {
    pub fn new(domain: &str, api_token: &str) -> Self {
        Self {
            client: Arc::new(RealOktaHttpClient::new(domain, api_token)),
        }
    }

    /// Constructor for tests — accepts a pre-built mock client.
    pub fn new_with_client(client: Arc<dyn OktaHttpClient>) -> Self {
        Self { client }
    }
}

impl PersistenceDriver for OktaPersistenceDriver {
    fn persist(
        &self,
        serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<(), OxDataError> {
        match location {
            "principals" => {
                let body = principal_to_okta_body(serializable_map);
                // Use ?activate=false to not immediately activate (default active).
                self.client.post("/api/v1/users?activate=true", &body)?;
                Ok(())
            }
            "groups" => {
                let body = group_to_okta_body(serializable_map);
                self.client.post("/api/v1/groups", &body)?;
                Ok(())
            }
            "members" => {
                // Requires both _okta_group_id and _okta_user_id to be provided as annotations.
                // These must be resolved by the caller (or via a fetch before persist).
                let group_id = canon_get(serializable_map, "_okta_group_id");
                let user_id  = canon_get(serializable_map, "_okta_user_id");
                if group_id.is_empty() || user_id.is_empty() {
                    return Err(OxDataError::DriverError(
                        "Okta members persist requires '_okta_group_id' and '_okta_user_id' in the map".to_string(),
                    ));
                }
                let path = mapping::group_membership_put_path(&group_id, &user_id);
                self.client.put(&path, &serde_json::json!({}))?;
                Ok(())
            }
            other => Err(OxDataError::DriverError(format!(
                "Okta driver does not support persist for location '{}'. Route to overflow store.",
                other
            ))),
        }
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, OxDataError> {
        match location {
            "principals" => {
                let path = format!("/api/v1/users/{}", id);
                let user = self.client.get(&path)?;
                Ok(okta_user_to_canonical(&user))
            }
            "groups" => {
                // Okta group lookup by name requires a search.
                let path = format!("/api/v1/groups?q={}", id);
                let groups = self.client.get(&path)?;
                let arr = groups.as_array()
                    .and_then(|a| a.first())
                    .ok_or_else(|| OxDataError::InternalError(format!("Okta group not found: {}", id)))?;
                Ok(okta_group_to_canonical(arr))
            }
            other => Err(OxDataError::DriverError(format!(
                "Okta driver does not support restore for location '{}'", other
            ))),
        }
    }

    fn fetch(
        &self,
        _filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<Vec<String>, OxDataError> {
        match location {
            "principals" => {
                // List all users (no filter applied — caller should post-filter or extend
                // with Okta search query parameters via call_action).
                let users = self.client.get("/api/v1/users")?;
                let arr = users.as_array().ok_or_else(|| {
                    OxDataError::InternalError("Okta users response was not an array".to_string())
                })?;
                Ok(arr.iter()
                    .map(|u| okta_user_to_canonical(u))
                    .filter_map(|m| m.get("principal_id").map(|(v, _, _)| v.clone()))
                    .collect())
            }
            "groups" => {
                let groups = self.client.get("/api/v1/groups")?;
                let arr = groups.as_array().ok_or_else(|| {
                    OxDataError::InternalError("Okta groups response was not an array".to_string())
                })?;
                Ok(arr.iter()
                    .map(|g| okta_group_to_canonical(g))
                    .filter_map(|m| m.get("group_id").map(|(v, _, _)| v.clone()))
                    .collect())
            }
            other => Err(OxDataError::DriverError(format!(
                "Okta driver does not support fetch for location '{}'", other
            ))),
        }
    }

    fn notify_lock_status_change(&self, _lock_status: &str, _gdo_id: &str) {
        // No-op: Okta has no native lock notification.
    }

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), OxDataError> {
        // Verify connectivity by fetching the current user.
        self.client.get("/api/v1/users/me")?;
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError> {
        Ok(supported_locations())
    }

    fn describe_dataset(
        &self,
        _connection_info: &HashMap<String, String>,
        dataset_name: &str,
    ) -> Result<DataSet, OxDataError> {
        let pk = match dataset_name {
            "principals" => "principal_id",
            "groups"     => "group_id",
            "members"    => "principal_id",
            other => return Err(OxDataError::DriverError(format!("Unknown Okta location: {}", other))),
        };
        Ok(DataSet {
            name: dataset_name.to_string(),
            columns: vec![ColumnDefinition {
                name: pk.to_string(),
                data_type: "string".to_string(),
                metadata: ColumnMetadata::default(),
            }],
        })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "domain".to_string(),
                description: "Okta organization domain (e.g. yourorg.okta.com)".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
            ConnectionParameter {
                name: "api_token".to_string(),
                description: "Okta API token (SSWS token from Okta admin console)".to_string(),
                data_type: "string".to_string(),
                is_required: true,
                default_value: None,
            },
        ]
    }
}

// ---------------------------------------------------------------------------
// FFI exports
// ---------------------------------------------------------------------------

use std::ffi::{c_void, CString, CStr};
use libc::c_char;

#[no_mangle]
pub extern "C" fn ox_driver_init(config_json: *const c_char) -> *mut c_void {
    let config_str = unsafe { CStr::from_ptr(config_json).to_string_lossy() };
    let info: HashMap<String, String> = serde_json::from_str(&config_str).unwrap_or_default();
    let domain    = info.get("domain").cloned().unwrap_or_default();
    let api_token = info.get("api_token").cloned().unwrap_or_default();
    if domain.is_empty() {
        eprintln!("ox_persistence_okta: missing 'domain' in config");
        return std::ptr::null_mut();
    }
    let driver = Box::new(OktaPersistenceDriver::new(&domain, &api_token));
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut OktaPersistenceDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    ctx: *mut c_void,
    data_json: *const c_char,
    location: *const c_char,
) -> i32 {
    let driver = &*(ctx as *mut OktaPersistenceDriver);
    let data_str = CStr::from_ptr(data_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&data_str) {
        Ok(map) => match driver.persist(&map, &location_str) {
            Ok(_) => 0,
            Err(e) => { eprintln!("Okta persist error: {}", e); -1 }
        },
        Err(e) => { eprintln!("Okta persist JSON error: {}", e); -2 }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_restore(
    ctx: *mut c_void,
    location: *const c_char,
    id: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut OktaPersistenceDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();
    match driver.restore(&location_str, &id_str) {
        Ok(map) => OxBuffer::from_str(serde_json::to_string(&map).unwrap_or_default()),
        Err(e) => { eprintln!("Okta restore error: {}", e); OxBuffer::empty() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_fetch(
    ctx: *mut c_void,
    filter_json: *const c_char,
    location: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut OktaPersistenceDriver);
    let filter_str = CStr::from_ptr(filter_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&filter_str) {
        Ok(filter) => match driver.fetch(&filter, &location_str) {
            Ok(ids) => OxBuffer::from_str(serde_json::to_string(&ids).unwrap_or_default()),
            Err(e) => { eprintln!("Okta fetch error: {}", e); OxBuffer::empty() }
        },
        Err(e) => { eprintln!("Okta fetch JSON error: {}", e); OxBuffer::empty() }
    }
}

#[no_mangle]
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compat = HashMap::new();
    compat.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "Okta Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );
    let metadata = DriverMetadata {
        name: "ox_persistence_okta".to_string(),
        friendly_name: Some("Okta".to_string()),
        description: "Persists canonical IAM principals, groups, and group membership to Okta via REST API. Grants and sessions require an overflow store.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules: compat,
    };
    CString::new(serde_json::to_string(&metadata).expect("serialize")).expect("CString").into_raw()
}

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = r#"
parameters:
  - name: domain
    type: string
    required: true
    description: "Okta organization domain"
  - name: api_token
    type: string
    required: true
    description: "Okta SSWS API token"
"#;
    CString::new(schema).expect("CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}
```

- [ ] **Step 4: Run Okta driver tests**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_okta 2>&1 | tail -20
```

Expected: 9 tests pass (3 http_client + 6 driver tests).

- [ ] **Step 5: Commit Okta driver**

```bash
git add crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/lib.rs \
        crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/src/mapping.rs \
        crates/data/ox_persistence/drivers/cloud/ox_persistence_okta/tests/okta_tests.rs
git commit -m "feat(okta-driver): implement OktaPersistenceDriver with full trait, FFI, and 9 tests"
```

---

## Task 5: RADIUS response parser and `PersistenceDriver` impl

**Files:**
- Create: `crates/data/ox_persistence/drivers/network/ox_persistence_radius/src/response_parser.rs`
- Create: `crates/data/ox_persistence/drivers/network/ox_persistence_radius/src/lib.rs`
- Create: `crates/data/ox_persistence/drivers/network/ox_persistence_radius/tests/radius_tests.rs`

The RADIUS driver does NOT connect to a RADIUS server directly — it parses attribute-value pairs from an Access-Accept packet that another layer hands to it as a structured map. `persist` is not supported; `restore` and `fetch` read from a cached in-memory map of Access-Accept attributes.

- [ ] **Step 1: Write the failing tests**

Create `crates/data/ox_persistence/drivers/network/ox_persistence_radius/tests/radius_tests.rs`:

```rust
use ox_persistence_radius::response_parser::{
    parse_access_accept, RadiusAttribute,
};
use ox_persistence_radius::RadiusPersistenceDriver;
use ox_persistence::PersistenceDriver;
use std::collections::HashMap;
use ox_type_converter::ValueType;

fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

// ---------------------------------------------------------------------------
// Parser tests
// ---------------------------------------------------------------------------

#[test]
fn parse_access_accept_extracts_user_name() {
    // Simulate an Access-Accept with User-Name and Class attributes.
    let attrs = vec![
        RadiusAttribute { attr_type: 1,   value: b"alice".to_vec() }, // User-Name = 1
        RadiusAttribute { attr_type: 25,  value: b"grp-ops".to_vec() }, // Class = 25
    ];
    let canonical = parse_access_accept(&attrs, "t1");
    assert_eq!(canonical.get("principal_id").unwrap().0, "alice");
    assert_eq!(canonical.get("tenant_id").unwrap().0, "t1");
}

#[test]
fn parse_access_accept_extracts_class_as_group() {
    let attrs = vec![
        RadiusAttribute { attr_type: 1,  value: b"bob".to_vec() },
        RadiusAttribute { attr_type: 25, value: b"network-admins".to_vec() },
    ];
    let canonical = parse_access_accept(&attrs, "tenant1");
    assert_eq!(canonical.get("group_id").unwrap().0, "network-admins");
    assert_eq!(canonical.get("principal_id").unwrap().0, "bob");
}

// ---------------------------------------------------------------------------
// Driver tests
// ---------------------------------------------------------------------------

#[test]
fn radius_driver_restore_returns_cached_principal() {
    let mut cache = HashMap::new();
    let mut entry = HashMap::new();
    entry.insert("principal_id".to_string(), str_entry("alice"));
    entry.insert("display_name".to_string(), str_entry("Alice"));
    entry.insert("source".to_string(),       str_entry("Radius"));
    entry.insert("tenant_id".to_string(),    str_entry("t1"));
    cache.insert("alice".to_string(), entry);

    let driver = RadiusPersistenceDriver::new_with_cache(cache);
    let restored = driver.restore("principals", "alice").expect("restore failed");
    assert_eq!(restored.get("principal_id").unwrap().0, "alice");
}

#[test]
fn radius_driver_persist_returns_not_supported() {
    let driver = RadiusPersistenceDriver::new_with_cache(HashMap::new());
    let mut data = HashMap::new();
    data.insert("principal_id".to_string(), str_entry("alice"));
    let result = driver.persist(&data, "principals");
    assert!(result.is_err(), "RADIUS persist should return not-supported error");
}

#[test]
fn radius_driver_list_datasets_returns_principals_and_members() {
    let driver = RadiusPersistenceDriver::new_with_cache(HashMap::new());
    let datasets = driver.list_datasets(&HashMap::new()).expect("list failed");
    assert!(datasets.contains(&"principals".to_string()));
    assert!(datasets.contains(&"members".to_string()));
    assert!(!datasets.contains(&"grants".to_string()));
    assert!(!datasets.contains(&"sessions".to_string()));
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_radius 2>&1 | head -20
```

Expected: compile error — modules not found.

- [ ] **Step 3: Implement `response_parser.rs`**

Create `crates/data/ox_persistence/drivers/network/ox_persistence_radius/src/response_parser.rs`:

```rust
//! Parses RADIUS Access-Accept attribute-value pairs into canonical IAM maps.
//! RADIUS attribute type numbers follow RFC 2865.

use std::collections::HashMap;
use ox_type_converter::ValueType;

pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// A single RADIUS attribute from an Access-Accept packet.
#[derive(Debug, Clone)]
pub struct RadiusAttribute {
    /// RADIUS attribute type number (e.g. 1 = User-Name, 25 = Class).
    pub attr_type: u8,
    /// Raw value bytes.
    pub value: Vec<u8>,
}

impl RadiusAttribute {
    /// Decodes the value as a UTF-8 string, replacing invalid bytes with the replacement char.
    pub fn value_str(&self) -> String {
        String::from_utf8_lossy(&self.value).into_owned()
    }
}

fn s(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

/// Converts a list of RADIUS Access-Accept attributes into a canonical IAM map.
///
/// Mappings:
///   - Attribute type 1 (User-Name) → `principal_id`
///   - Attribute type 6 (Service-Type) → `source` annotation
///   - Attribute type 25 (Class) → `group_id` (first Class attribute wins for group_id)
///   - `tenant_id` is injected from the caller-supplied argument
///   - `source` is hard-coded to "Radius"
///
/// The Class attribute (type 25) contains group or role names depending on the
/// RADIUS server configuration.  If multiple Class attributes are present, the
/// first is used for `group_id` and the rest are stored as `group_id_2`, `group_id_3`, …
pub fn parse_access_accept(attrs: &[RadiusAttribute], tenant_id: &str) -> CanonicalMap {
    let mut map = CanonicalMap::new();
    map.insert("source".to_string(),    s("Radius"));
    map.insert("tenant_id".to_string(), s(tenant_id));

    let mut group_counter: u32 = 0;

    for attr in attrs {
        match attr.attr_type {
            1 => {
                // User-Name
                map.insert("principal_id".to_string(), s(&attr.value_str()));
                map.insert("display_name".to_string(), s(&attr.value_str()));
            }
            25 => {
                // Class — first occurrence becomes group_id, subsequent become group_id_N
                if group_counter == 0 {
                    map.insert("group_id".to_string(), s(&attr.value_str()));
                } else {
                    map.insert(format!("group_id_{}", group_counter + 1), s(&attr.value_str()));
                }
                group_counter += 1;
            }
            _ => {
                // Unknown attributes are ignored — the driver only extracts what it understands.
            }
        }
    }

    map
}
```

- [ ] **Step 4: Implement `lib.rs`**

Create `crates/data/ox_persistence/drivers/network/ox_persistence_radius/src/lib.rs`:

```rust
//! ox_persistence_radius — read-only RADIUS persistence driver.
//!
//! RADIUS servers do not expose a directory API.  This driver can only:
//! 1. Return principal and group-membership data extracted from a cached
//!    Access-Accept attribute set (populated by the auth driver after a
//!    successful authentication round-trip).
//! 2. Persist is not supported — returns OxDataError::DriverError for all locations.
//!
//! Supported locations (read-only): "principals", "members"
//! Unsupported locations: "groups", "grants", "sessions"

pub mod response_parser;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ox_data_error::OxDataError;
use ox_persistence::{
    PersistenceDriver, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer,
};
use ox_type_converter::ValueType;

pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// In-memory cache of principal data populated from RADIUS Access-Accept responses.
/// Key: principal_id; Value: canonical map for that principal.
type PrincipalCache = HashMap<String, CanonicalMap>;

pub struct RadiusPersistenceDriver {
    cache: Arc<Mutex<PrincipalCache>>,
}

impl RadiusPersistenceDriver {
    /// Constructs a driver backed by an empty cache.
    /// The cache is populated by the auth driver via `insert_cached_principal`.
    pub fn new() -> Self {
        Self { cache: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Constructs a driver with a pre-populated cache (used by tests).
    pub fn new_with_cache(cache: PrincipalCache) -> Self {
        Self { cache: Arc::new(Mutex::new(cache)) }
    }

    /// Called by the RADIUS auth driver after a successful Access-Accept to populate the cache.
    pub fn insert_cached_principal(&self, principal_id: &str, data: CanonicalMap) {
        self.cache.lock().unwrap().insert(principal_id.to_string(), data);
    }
}

impl Default for RadiusPersistenceDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistenceDriver for RadiusPersistenceDriver {
    fn persist(
        &self,
        _serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<(), OxDataError> {
        Err(OxDataError::DriverError(format!(
            "RADIUS driver is read-only: persist not supported for location '{}'",
            location
        )))
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, OxDataError> {
        match location {
            "principals" => {
                let cache = self.cache.lock().unwrap();
                cache.get(id)
                    .cloned()
                    .ok_or_else(|| OxDataError::InternalError(format!("RADIUS: principal '{}' not in cache", id)))
            }
            other => Err(OxDataError::DriverError(format!(
                "RADIUS driver does not support restore for location '{}'", other
            ))),
        }
    }

    fn fetch(
        &self,
        filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<Vec<String>, OxDataError> {
        match location {
            "principals" => {
                let cache = self.cache.lock().unwrap();
                // Return IDs of all cached principals that match every filter field.
                let ids: Vec<String> = cache
                    .iter()
                    .filter(|(_, entry)| {
                        filter.iter().all(|(fk, (fv, _, _))| {
                            entry.get(fk).map_or(false, |(ev, _, _)| ev == fv)
                        })
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                Ok(ids)
            }
            "members" => {
                // Return principal IDs whose group_id matches the filter.
                let group_id_filter = filter.get("group_id").map(|(v, _, _)| v.clone());
                if group_id_filter.is_none() {
                    return Err(OxDataError::DriverError(
                        "RADIUS members fetch requires 'group_id' in filter".to_string(),
                    ));
                }
                let gid = group_id_filter.unwrap();
                let cache = self.cache.lock().unwrap();
                let ids: Vec<String> = cache
                    .iter()
                    .filter(|(_, entry)| {
                        entry.get("group_id").map_or(false, |(v, _, _)| v == &gid)
                    })
                    .filter_map(|(_, entry)| entry.get("principal_id").map(|(v, _, _)| v.clone()))
                    .collect();
                Ok(ids)
            }
            other => Err(OxDataError::DriverError(format!(
                "RADIUS driver does not support fetch for location '{}'", other
            ))),
        }
    }

    fn notify_lock_status_change(&self, _lock_status: &str, _gdo_id: &str) {}

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), OxDataError> {
        // No preparation needed for the in-memory cache driver.
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError> {
        Ok(vec!["principals".to_string(), "members".to_string()])
    }

    fn describe_dataset(
        &self,
        _connection_info: &HashMap<String, String>,
        dataset_name: &str,
    ) -> Result<DataSet, OxDataError> {
        let pk = match dataset_name {
            "principals" => "principal_id",
            "members"    => "principal_id",
            other => return Err(OxDataError::DriverError(format!("Unknown RADIUS location: {}", other))),
        };
        Ok(DataSet {
            name: dataset_name.to_string(),
            columns: vec![ColumnDefinition {
                name: pk.to_string(),
                data_type: "string".to_string(),
                metadata: ColumnMetadata::default(),
            }],
        })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "server".to_string(),
                description: "RADIUS server address (not used for direct persistence — this driver operates on cached Access-Accept data)".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
        ]
    }
}

// ---------------------------------------------------------------------------
// FFI exports
// ---------------------------------------------------------------------------

use std::ffi::{c_void, CString, CStr};
use libc::c_char;

#[no_mangle]
pub extern "C" fn ox_driver_init(_config_json: *const c_char) -> *mut c_void {
    let driver = Box::new(RadiusPersistenceDriver::new());
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut RadiusPersistenceDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    _ctx: *mut c_void,
    _data_json: *const c_char,
    location: *const c_char,
) -> i32 {
    let location_str = CStr::from_ptr(location).to_string_lossy();
    eprintln!("RADIUS driver: persist not supported for location '{}'", location_str);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_restore(
    ctx: *mut c_void,
    location: *const c_char,
    id: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut RadiusPersistenceDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();
    match driver.restore(&location_str, &id_str) {
        Ok(map) => OxBuffer::from_str(serde_json::to_string(&map).unwrap_or_default()),
        Err(e) => { eprintln!("RADIUS restore error: {}", e); OxBuffer::empty() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_fetch(
    ctx: *mut c_void,
    filter_json: *const c_char,
    location: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut RadiusPersistenceDriver);
    let filter_str = CStr::from_ptr(filter_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&filter_str) {
        Ok(filter) => match driver.fetch(&filter, &location_str) {
            Ok(ids) => OxBuffer::from_str(serde_json::to_string(&ids).unwrap_or_default()),
            Err(e) => { eprintln!("RADIUS fetch error: {}", e); OxBuffer::empty() }
        },
        Err(e) => { eprintln!("RADIUS fetch JSON error: {}", e); OxBuffer::empty() }
    }
}

#[no_mangle]
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compat = HashMap::new();
    compat.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "RADIUS Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );
    let metadata = DriverMetadata {
        name: "ox_persistence_radius".to_string(),
        friendly_name: Some("RADIUS".to_string()),
        description: "Read-only RADIUS persistence driver. Extracts principal and group membership from cached Access-Accept attributes. Persist not supported — route grants and sessions to overflow store.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules: compat,
    };
    CString::new(serde_json::to_string(&metadata).expect("serialize")).expect("CString").into_raw()
}

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = r#"
parameters:
  - name: server
    type: string
    required: false
    description: "RADIUS server address (informational only)"
"#;
    CString::new(schema).expect("CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}
```

- [ ] **Step 5: Run RADIUS tests**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_radius 2>&1 | tail -15
```

Expected: 5 tests pass.

- [ ] **Step 6: Commit RADIUS driver**

```bash
git add crates/data/ox_persistence/drivers/network/ox_persistence_radius/src/lib.rs \
        crates/data/ox_persistence/drivers/network/ox_persistence_radius/src/response_parser.rs \
        crates/data/ox_persistence/drivers/network/ox_persistence_radius/tests/radius_tests.rs
git commit -m "feat(radius-driver): implement read-only RadiusPersistenceDriver with Access-Accept parser (5 tests)"
```

---

## Task 6: TACACS+ response parser and `PersistenceDriver` impl

**Files:**
- Create: `crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/src/response_parser.rs`
- Create: `crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/src/lib.rs`
- Create: `crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/tests/tacacs_tests.rs`

TACACS+ is structurally similar to RADIUS for our purposes: the auth driver completes a TACACS+ authentication exchange, extracts AV pairs from the Authorization-Response, and populates the persistence cache. `persist` is not supported.

- [ ] **Step 1: Write failing tests**

Create `crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/tests/tacacs_tests.rs`:

```rust
use ox_persistence_tacacs::response_parser::{parse_av_pairs, TacacsAvPair};
use ox_persistence_tacacs::TacacsPersistenceDriver;
use ox_persistence::PersistenceDriver;
use std::collections::HashMap;
use ox_type_converter::ValueType;

fn str_entry(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

// ---------------------------------------------------------------------------
// Parser tests
// ---------------------------------------------------------------------------

#[test]
fn parse_av_pairs_extracts_user_from_service_pair() {
    let pairs = vec![
        TacacsAvPair { attribute: "service".to_string(), value: "shell".to_string() },
        TacacsAvPair { attribute: "priv-lvl".to_string(), value: "15".to_string() },
        TacacsAvPair { attribute: "user".to_string(), value: "alice".to_string() },
    ];
    let canonical = parse_av_pairs(&pairs, "t1");
    assert_eq!(canonical.get("principal_id").unwrap().0, "alice");
    assert_eq!(canonical.get("tenant_id").unwrap().0, "t1");
}

#[test]
fn parse_av_pairs_maps_priv_level_to_group() {
    let pairs = vec![
        TacacsAvPair { attribute: "user".to_string(),     value: "bob".to_string() },
        TacacsAvPair { attribute: "priv-lvl".to_string(), value: "15".to_string() },
    ];
    let canonical = parse_av_pairs(&pairs, "t1");
    // priv-lvl 15 maps to the "network-admin" role by convention (configurable).
    // The parser maps priv-lvl value to group_id as "priv-lvl-<N>".
    assert_eq!(canonical.get("group_id").unwrap().0, "priv-lvl-15");
}

#[test]
fn parse_av_pairs_custom_group_av() {
    let pairs = vec![
        TacacsAvPair { attribute: "user".to_string(),   value: "carol".to_string() },
        TacacsAvPair { attribute: "oxgroup".to_string(), value: "netops".to_string() },
    ];
    let canonical = parse_av_pairs(&pairs, "t1");
    assert_eq!(canonical.get("group_id").unwrap().0, "netops");
}

// ---------------------------------------------------------------------------
// Driver tests
// ---------------------------------------------------------------------------

#[test]
fn tacacs_driver_restore_returns_cached_principal() {
    let mut cache = HashMap::new();
    let mut entry = HashMap::new();
    entry.insert("principal_id".to_string(), str_entry("alice"));
    entry.insert("source".to_string(),       str_entry("Tacacs"));
    entry.insert("tenant_id".to_string(),    str_entry("t1"));
    cache.insert("alice".to_string(), entry);

    let driver = TacacsPersistenceDriver::new_with_cache(cache);
    let restored = driver.restore("principals", "alice").expect("restore failed");
    assert_eq!(restored.get("principal_id").unwrap().0, "alice");
}

#[test]
fn tacacs_driver_persist_returns_not_supported() {
    let driver = TacacsPersistenceDriver::new_with_cache(HashMap::new());
    let mut data = HashMap::new();
    data.insert("principal_id".to_string(), str_entry("alice"));
    let result = driver.persist(&data, "principals");
    assert!(result.is_err());
}

#[test]
fn tacacs_driver_list_datasets_does_not_include_grants() {
    let driver = TacacsPersistenceDriver::new_with_cache(HashMap::new());
    let datasets = driver.list_datasets(&HashMap::new()).expect("list failed");
    assert!(datasets.contains(&"principals".to_string()));
    assert!(!datasets.contains(&"grants".to_string()));
    assert!(!datasets.contains(&"sessions".to_string()));
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_tacacs 2>&1 | head -20
```

Expected: compile error.

- [ ] **Step 3: Implement `response_parser.rs`**

Create `crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/src/response_parser.rs`:

```rust
//! Parses TACACS+ authorization response AV (Attribute-Value) pairs into canonical IAM maps.
//! TACACS+ AV pairs are defined in RFC 8907 and RFC 1492.

use std::collections::HashMap;
use ox_type_converter::ValueType;

pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;

/// A single TACACS+ attribute-value pair from an Authorization-Response.
#[derive(Debug, Clone)]
pub struct TacacsAvPair {
    pub attribute: String,
    pub value: String,
}

fn s(v: &str) -> (String, ValueType, HashMap<String, String>) {
    (v.to_string(), ValueType::String, HashMap::new())
}

/// Converts a list of TACACS+ AV pairs from an Authorization-Response into a canonical IAM map.
///
/// Mappings:
///   - `user` AV pair → `principal_id`, `display_name`
///   - `priv-lvl` AV pair → `group_id` formatted as `"priv-lvl-<N>"`
///   - `oxgroup` AV pair (custom) → `group_id` (overrides priv-lvl if present)
///   - `source` is hard-coded to "Tacacs"
///   - `tenant_id` is injected from the caller-supplied argument
///
/// The `priv-lvl` convention maps privilege levels (0-15) to group identifiers.
/// Custom AV pairs like `oxgroup=netops` can be used on TACACS+ servers that
/// support custom attributes to carry richer group information.
pub fn parse_av_pairs(pairs: &[TacacsAvPair], tenant_id: &str) -> CanonicalMap {
    let mut map = CanonicalMap::new();
    map.insert("source".to_string(),    s("Tacacs"));
    map.insert("tenant_id".to_string(), s(tenant_id));

    let mut has_explicit_group = false;

    for pair in pairs {
        match pair.attribute.as_str() {
            "user" => {
                map.insert("principal_id".to_string(), s(&pair.value));
                map.insert("display_name".to_string(), s(&pair.value));
            }
            "priv-lvl" => {
                if !has_explicit_group {
                    map.insert("group_id".to_string(), s(&format!("priv-lvl-{}", pair.value)));
                }
            }
            "oxgroup" => {
                // Custom AV pair carrying explicit group name — takes precedence over priv-lvl.
                map.insert("group_id".to_string(), s(&pair.value));
                has_explicit_group = true;
            }
            _ => {
                // Unknown AV pairs are ignored.
            }
        }
    }

    map
}
```

- [ ] **Step 4: Implement `lib.rs`**

Create `crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/src/lib.rs`:

```rust
//! ox_persistence_tacacs — read-only TACACS+ persistence driver.
//!
//! TACACS+ servers expose authentication and authorization but not a directory
//! query API.  This driver persists principal and group-membership data extracted
//! from TACACS+ Authorization-Response AV pairs into an in-memory cache.
//!
//! Supported locations (read-only): "principals", "members"
//! Unsupported locations: "groups", "grants", "sessions"

pub mod response_parser;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ox_data_error::OxDataError;
use ox_persistence::{
    PersistenceDriver, DataSet, ColumnDefinition, ColumnMetadata, ConnectionParameter, DriverMetadata, ModuleCompatibility, OxBuffer,
};
use ox_type_converter::ValueType;

pub type CanonicalMap = HashMap<String, (String, ValueType, HashMap<String, String>)>;
type PrincipalCache = HashMap<String, CanonicalMap>;

pub struct TacacsPersistenceDriver {
    cache: Arc<Mutex<PrincipalCache>>,
}

impl TacacsPersistenceDriver {
    pub fn new() -> Self {
        Self { cache: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn new_with_cache(cache: PrincipalCache) -> Self {
        Self { cache: Arc::new(Mutex::new(cache)) }
    }

    /// Called by the TACACS+ auth driver after a successful Authorization-Response.
    pub fn insert_cached_principal(&self, principal_id: &str, data: CanonicalMap) {
        self.cache.lock().unwrap().insert(principal_id.to_string(), data);
    }
}

impl Default for TacacsPersistenceDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistenceDriver for TacacsPersistenceDriver {
    fn persist(
        &self,
        _serializable_map: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<(), OxDataError> {
        Err(OxDataError::DriverError(format!(
            "TACACS+ driver is read-only: persist not supported for location '{}'",
            location
        )))
    }

    fn restore(
        &self,
        location: &str,
        id: &str,
    ) -> Result<HashMap<String, (String, ValueType, HashMap<String, String>)>, OxDataError> {
        match location {
            "principals" => {
                let cache = self.cache.lock().unwrap();
                cache.get(id)
                    .cloned()
                    .ok_or_else(|| OxDataError::InternalError(format!("TACACS+: principal '{}' not in cache", id)))
            }
            other => Err(OxDataError::DriverError(format!(
                "TACACS+ driver does not support restore for location '{}'", other
            ))),
        }
    }

    fn fetch(
        &self,
        filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>,
        location: &str,
    ) -> Result<Vec<String>, OxDataError> {
        match location {
            "principals" => {
                let cache = self.cache.lock().unwrap();
                let ids: Vec<String> = cache
                    .iter()
                    .filter(|(_, entry)| {
                        filter.iter().all(|(fk, (fv, _, _))| {
                            entry.get(fk).map_or(false, |(ev, _, _)| ev == fv)
                        })
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                Ok(ids)
            }
            "members" => {
                let group_id_filter = filter.get("group_id").map(|(v, _, _)| v.clone());
                if group_id_filter.is_none() {
                    return Err(OxDataError::DriverError(
                        "TACACS+ members fetch requires 'group_id' in filter".to_string(),
                    ));
                }
                let gid = group_id_filter.unwrap();
                let cache = self.cache.lock().unwrap();
                let ids: Vec<String> = cache
                    .iter()
                    .filter(|(_, entry)| {
                        entry.get("group_id").map_or(false, |(v, _, _)| v == &gid)
                    })
                    .filter_map(|(_, entry)| entry.get("principal_id").map(|(v, _, _)| v.clone()))
                    .collect();
                Ok(ids)
            }
            other => Err(OxDataError::DriverError(format!(
                "TACACS+ driver does not support fetch for location '{}'", other
            ))),
        }
    }

    fn notify_lock_status_change(&self, _lock_status: &str, _gdo_id: &str) {}

    fn prepare_datastore(&self, _connection_info: &HashMap<String, String>) -> Result<(), OxDataError> {
        Ok(())
    }

    fn list_datasets(&self, _connection_info: &HashMap<String, String>) -> Result<Vec<String>, OxDataError> {
        Ok(vec!["principals".to_string(), "members".to_string()])
    }

    fn describe_dataset(
        &self,
        _connection_info: &HashMap<String, String>,
        dataset_name: &str,
    ) -> Result<DataSet, OxDataError> {
        let pk = match dataset_name {
            "principals" => "principal_id",
            "members"    => "principal_id",
            other => return Err(OxDataError::DriverError(format!("Unknown TACACS+ location: {}", other))),
        };
        Ok(DataSet {
            name: dataset_name.to_string(),
            columns: vec![ColumnDefinition {
                name: pk.to_string(),
                data_type: "string".to_string(),
                metadata: ColumnMetadata::default(),
            }],
        })
    }

    fn get_connection_parameters(&self) -> Vec<ConnectionParameter> {
        vec![
            ConnectionParameter {
                name: "server".to_string(),
                description: "TACACS+ server address (informational only — this driver operates on cached authorization response data)".to_string(),
                data_type: "string".to_string(),
                is_required: false,
                default_value: None,
            },
        ]
    }
}

// ---------------------------------------------------------------------------
// FFI exports
// ---------------------------------------------------------------------------

use std::ffi::{c_void, CString, CStr};
use libc::c_char;

#[no_mangle]
pub extern "C" fn ox_driver_init(_config_json: *const c_char) -> *mut c_void {
    let driver = Box::new(TacacsPersistenceDriver::new());
    Box::into_raw(driver) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        let _ = Box::from_raw(ctx as *mut TacacsPersistenceDriver);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_persist(
    _ctx: *mut c_void,
    _data_json: *const c_char,
    location: *const c_char,
) -> i32 {
    let location_str = CStr::from_ptr(location).to_string_lossy();
    eprintln!("TACACS+ driver: persist not supported for location '{}'", location_str);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_restore(
    ctx: *mut c_void,
    location: *const c_char,
    id: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut TacacsPersistenceDriver);
    let location_str = CStr::from_ptr(location).to_string_lossy();
    let id_str = CStr::from_ptr(id).to_string_lossy();
    match driver.restore(&location_str, &id_str) {
        Ok(map) => OxBuffer::from_str(serde_json::to_string(&map).unwrap_or_default()),
        Err(e) => { eprintln!("TACACS+ restore error: {}", e); OxBuffer::empty() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_fetch(
    ctx: *mut c_void,
    filter_json: *const c_char,
    location: *const c_char,
) -> OxBuffer {
    let driver = &*(ctx as *mut TacacsPersistenceDriver);
    let filter_str = CStr::from_ptr(filter_json).to_string_lossy();
    let location_str = CStr::from_ptr(location).to_string_lossy();
    match serde_json::from_str::<HashMap<String, (String, ValueType, HashMap<String, String>)>>(&filter_str) {
        Ok(filter) => match driver.fetch(&filter, &location_str) {
            Ok(ids) => OxBuffer::from_str(serde_json::to_string(&ids).unwrap_or_default()),
            Err(e) => { eprintln!("TACACS+ fetch error: {}", e); OxBuffer::empty() }
        },
        Err(e) => { eprintln!("TACACS+ fetch JSON error: {}", e); OxBuffer::empty() }
    }
}

#[no_mangle]
pub extern "C" fn ox_driver_get_driver_metadata() -> *mut c_char {
    let mut compat = HashMap::new();
    compat.insert(
        "ox_data_broker_server".to_string(),
        ModuleCompatibility {
            human_name: "TACACS+ Persistence Driver".to_string(),
            crate_type: "Data Source Driver".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );
    let metadata = DriverMetadata {
        name: "ox_persistence_tacacs".to_string(),
        friendly_name: Some("TACACS+".to_string()),
        description: "Read-only TACACS+ persistence driver. Extracts principal and group membership from cached Authorization-Response AV pairs. Persist not supported — route grants and sessions to overflow store.".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        compatible_modules: compat,
    };
    CString::new(serde_json::to_string(&metadata).expect("serialize")).expect("CString").into_raw()
}

#[no_mangle]
pub extern "C" fn ox_driver_get_config_schema() -> *mut c_char {
    let schema = r#"
parameters:
  - name: server
    type: string
    required: false
    description: "TACACS+ server address (informational only)"
"#;
    CString::new(schema).expect("CString").into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn ox_driver_free_buffer(buf: OxBuffer) {
    ox_persistence::free_ox_buffer(buf);
}
```

- [ ] **Step 5: Run TACACS+ tests**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_tacacs 2>&1 | tail -15
```

Expected: 6 tests pass (3 parser + 3 driver).

- [ ] **Step 6: Commit TACACS+ driver**

```bash
git add crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/src/lib.rs \
        crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/src/response_parser.rs \
        crates/data/ox_persistence/drivers/network/ox_persistence_tacacs/tests/tacacs_tests.rs
git commit -m "feat(tacacs-driver): implement read-only TacacsPersistenceDriver with AV-pair parser (6 tests)"
```

---

## Task 7: Final integration check

- [ ] **Step 1: Build all workspace crates**

```bash
cd /var/repos/oxIDIZER && cargo build --workspace 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 2: Run all tests in all three new crates**

```bash
cd /var/repos/oxIDIZER && cargo test -p ox_persistence_okta -p ox_persistence_radius -p ox_persistence_tacacs -- --test-threads=4 2>&1 | tail -30
```

Expected: 20 tests pass (9 Okta + 5 RADIUS + 6 TACACS+), 0 failures.

- [ ] **Step 3: Commit if any adjustments were made during integration**

```bash
git add -p
git commit -m "fix(okta-radius-tacacs-drivers): post-integration adjustments"
```
