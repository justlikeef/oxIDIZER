# ox_persistence Enhancements — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend ox_persistence drivers with DDL execution, raw SQL queries, and comparison filter operators — the capabilities the ox_cert CA system requires but that aren't present today.

**Architecture:** All changes are additive: new `call_action` action names in the SQLite and PostgreSQL drivers, plus new filter operator metadata understood by the shared `SqlBuilder`. No existing behaviour is changed.

**Tech Stack:** Rust 2021, rusqlite 0.31 (SQLite driver), postgres 0.19 (Postgres driver), existing SqlBuilder in `ox_persistence_driver_db_sql`.

---

## Audit of Current Gaps

| Gap | Impact on ox_cert |
|-----|------------------|
| `prepare_datastore()` is a no-op — no schema creation | Cannot bootstrap the 15 cert tables |
| `call_action()` returns Err by default | Cannot run migrations or complex queries |
| `SqlBuilder::build_fetch` only generates `=` predicates | Cannot express `not_after < NOW()`, `revoked_at >= ?`, etc. |
| `fetch()` → IDs only, then N individual `restore()` calls | Acceptable for now; bulk optimisation is future work |
| No atomic-increment action | Cannot generate SSH certificate serials safely |

---

## File Map

### Modified files

```
crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sql/src/sql_builder.rs
    — add op metadata support to build_fetch; new build_execute helper

crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sqlite/src/lib.rs
    — implement call_action: "run_ddl", "raw_sql", "atomic_increment"

crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_postgres/src/lib.rs
    — implement call_action: "run_ddl", "raw_sql", "atomic_increment"
```

### New files

```
crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sqlite/src/actions.rs
    — call_action implementations for SQLite

crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_postgres/src/actions.rs
    — call_action implementations for PostgreSQL
```

---

## Filter Operator Specification

The existing filter map type is:
```
HashMap<field, (value_str, ValueType, metadata)>
```

The `metadata` HashMap supports a new key `"op"` with these values:

| `op` value | SQL predicate | Notes |
|------------|---------------|-------|
| `"eq"` (default) | `field = ?` | Same as current behaviour |
| `"ne"` | `field != ?` | |
| `"lt"` | `field < ?` | |
| `"lte"` | `field <= ?` | |
| `"gt"` | `field > ?` | |
| `"gte"` | `field >= ?` | |
| `"like"` | `field LIKE ?` | |
| `"is_null"` | `field IS NULL` | value_str ignored |
| `"is_not_null"` | `field IS NOT NULL` | value_str ignored |

---

## call_action Specification

All drivers implement these actions. Params and return are `serde_json::Value`.

### `"run_ddl"`

Executes one DDL statement (CREATE TABLE IF NOT EXISTS, ALTER TABLE, etc.). No rows returned.

```json
// params
{ "sql": "CREATE TABLE IF NOT EXISTS certificates (serial TEXT PRIMARY KEY, ...)" }

// return on success
{ "ok": true }
```

### `"raw_sql"`

Executes one SELECT query with positional parameters. Returns rows as a JSON array of objects.

```json
// params
{
  "sql": "SELECT serial FROM certificates WHERE tenant_id = $1 AND not_after < $2",
  "params": ["acme-corp", "2026-05-23T00:00:00Z"]
}

// return
{ "rows": [ {"serial": "abc-123"}, ... ] }
```

SQLite uses `?` placeholders; the driver substitutes `$N` → `?` before execution.
PostgreSQL uses `$N` natively.

### `"atomic_increment"`

Atomically increments a BIGINT column and returns the new value. Used for SSH serial counter.

```json
// params
{ "table": "ssh_serial_counter", "column": "next_serial", "id_column": "tenant_id", "id_value": "acme-corp" }

// return
{ "value": 42 }
```

SQLite implementation: `UPDATE … SET next_serial = next_serial + 1 WHERE tenant_id = ? RETURNING next_serial`. If no row exists, inserts `(tenant_id, 1)` first.

PostgreSQL implementation: same SQL, uses `$1` parameter.

---

## Task 1: Extend SqlBuilder with operator support

**Files:**
- Modify: `crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sql/src/sql_builder.rs`

- [ ] **Step 1: Write the failing test**

Add to the existing `sql_builder.rs` test module (or create `tests/sql_builder_tests.rs`):

```rust
#[cfg(test)]
mod op_tests {
    use super::*;
    use std::collections::HashMap;
    use ox_type_converter::ValueType;

    fn meta(op: &str) -> HashMap<String, String> {
        HashMap::from([("op".to_string(), op.to_string())])
    }

    #[test]
    fn test_build_fetch_lte() {
        let builder = SqlBuilder::new(SqlDialect::Sqlite);
        let keys = vec!["tenant_id".to_string(), "not_after".to_string()];
        let metas = vec![HashMap::new(), meta("lte")];
        let sql = builder.build_fetch_with_ops("certificates", &keys, &metas);
        assert!(sql.contains("not_after <= ?"), "got: {sql}");
        assert!(sql.contains("tenant_id = ?"), "got: {sql}");
    }

    #[test]
    fn test_build_fetch_is_null() {
        let builder = SqlBuilder::new(SqlDialect::Sqlite);
        let keys = vec!["revoked_at".to_string()];
        let metas = vec![meta("is_null")];
        let sql = builder.build_fetch_with_ops("certificates", &keys, &metas);
        assert!(sql.contains("revoked_at IS NULL"), "got: {sql}");
    }

    #[test]
    fn test_build_fetch_default_op_is_eq() {
        let builder = SqlBuilder::new(SqlDialect::Sqlite);
        let keys = vec!["status".to_string()];
        let metas = vec![HashMap::new()];
        let sql = builder.build_fetch_with_ops("certificates", &keys, &metas);
        assert!(sql.contains("status = ?"), "got: {sql}");
    }
}
```

- [ ] **Step 2: Run test — verify it fails**

```bash
cd /var/repos/oxIDIZER
cargo test -p ox_persistence_driver_db_sql test_build_fetch_lte 2>&1 | tail -5
```

Expected: `error[E0425]: cannot find function 'build_fetch_with_ops'`

- [ ] **Step 3: Add `build_fetch_with_ops` to SqlBuilder**

In `sql_builder.rs`, add after the existing `build_fetch`:

```rust
/// Like build_fetch but honours the "op" key in each field's metadata.
pub fn build_fetch_with_ops(
    &self,
    table: &str,
    keys: &[String],
    metas: &[HashMap<String, String>],
) -> String {
    let quoted_table = self.quote_identifier(table);
    let quoted_id = self.quote_identifier(&self.id_field);
    let mut query = format!("SELECT {quoted_id} FROM {quoted_table} WHERE 1=1");
    let mut param_idx = 0usize;

    for (i, key) in keys.iter().enumerate() {
        let op = metas.get(i)
            .and_then(|m| m.get("op"))
            .map(String::as_str)
            .unwrap_or("eq");
        let quoted_key = self.quote_identifier(key);

        match op {
            "is_null" => {
                query.push_str(&format!(" AND {quoted_key} IS NULL"));
            }
            "is_not_null" => {
                query.push_str(&format!(" AND {quoted_key} IS NOT NULL"));
            }
            _ => {
                param_idx += 1;
                let placeholder = self.placeholder(param_idx - 1);
                let sql_op = match op {
                    "ne"  => "!=",
                    "lt"  => "<",
                    "lte" => "<=",
                    "gt"  => ">",
                    "gte" => ">=",
                    "like" => "LIKE",
                    _     => "=",  // "eq" and anything unknown
                };
                query.push_str(&format!(" AND {quoted_key} {sql_op} {placeholder}"));
            }
        }
    }
    query
}
```

- [ ] **Step 4: Run tests — verify they pass**

```bash
cargo test -p ox_persistence_driver_db_sql 2>&1 | tail -5
```

Expected: `test result: ok.`

- [ ] **Step 5: Commit**

```bash
git add crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sql/
git commit -m "feat(ox_persistence): add operator-aware build_fetch_with_ops to SqlBuilder"
```

---

## Task 2: Implement call_action for the SQLite driver

**Files:**
- Create: `crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sqlite/src/actions.rs`
- Modify: `crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sqlite/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sqlite/tests/actions_test.rs`:

```rust
use ox_persistence_driver_db_sqlite::SqlitePersistenceDriver;
use ox_persistence::PersistenceDriver;
use std::sync::Mutex;
use rusqlite::Connection;
use serde_json::json;

fn make_driver() -> SqlitePersistenceDriver {
    let conn = Connection::open_in_memory().unwrap();
    SqlitePersistenceDriver { conn: Mutex::new(Some(conn)), connection_string: Mutex::new(":memory:".to_string()) }
}

#[test]
fn test_run_ddl_creates_table() {
    let d = make_driver();
    let result = d.call_action("run_ddl", &json!({
        "sql": "CREATE TABLE IF NOT EXISTS test_tbl (id TEXT PRIMARY KEY, val TEXT)"
    }));
    assert!(result.is_ok(), "{result:?}");
    // Verify the table exists by inserting a row
    let rows = d.call_action("raw_sql", &json!({
        "sql": "SELECT name FROM sqlite_master WHERE type='table' AND name='test_tbl'",
        "params": []
    })).unwrap();
    assert_eq!(rows["rows"].as_array().unwrap().len(), 1);
}

#[test]
fn test_raw_sql_select() {
    let d = make_driver();
    d.call_action("run_ddl", &json!({"sql": "CREATE TABLE t (id TEXT PRIMARY KEY, n INTEGER)"})).unwrap();
    d.call_action("run_ddl", &json!({"sql": "INSERT INTO t VALUES ('a', 1)"})).unwrap();
    d.call_action("run_ddl", &json!({"sql": "INSERT INTO t VALUES ('b', 2)"})).unwrap();

    let res = d.call_action("raw_sql", &json!({
        "sql": "SELECT id, n FROM t WHERE n > $1",
        "params": ["1"]
    })).unwrap();
    let rows = res["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "b");
}

#[test]
fn test_atomic_increment_creates_row_if_missing() {
    let d = make_driver();
    d.call_action("run_ddl", &json!({
        "sql": "CREATE TABLE ssh_serial_counter (tenant_id TEXT PRIMARY KEY, next_serial INTEGER NOT NULL DEFAULT 0)"
    })).unwrap();
    let res = d.call_action("atomic_increment", &json!({
        "table": "ssh_serial_counter",
        "column": "next_serial",
        "id_column": "tenant_id",
        "id_value": "acme"
    })).unwrap();
    assert_eq!(res["value"], 1);
    // Second call returns 2
    let res2 = d.call_action("atomic_increment", &json!({
        "table": "ssh_serial_counter", "column": "next_serial",
        "id_column": "tenant_id", "id_value": "acme"
    })).unwrap();
    assert_eq!(res2["value"], 2);
}
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
cargo test -p ox_persistence_driver_db_sqlite 2>&1 | tail -5
```

Expected: tests not found (actions module doesn't exist yet).

- [ ] **Step 3: Create `actions.rs`**

`crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sqlite/src/actions.rs`:

```rust
use rusqlite::{Connection, types::ToSql};
use serde_json::{json, Value};
use std::sync::MutexGuard;

/// Execute a DDL statement (CREATE TABLE, ALTER TABLE, INSERT, etc.) with no result set.
pub fn run_ddl(conn: &mut Connection, params: &Value) -> Result<Value, String> {
    let sql = params["sql"].as_str().ok_or("run_ddl: missing 'sql'")?;
    conn.execute_batch(sql).map_err(|e| format!("run_ddl: {e}"))?;
    Ok(json!({"ok": true}))
}

/// Execute a SELECT query with positional $N parameters. Returns rows as JSON array.
pub fn raw_sql(conn: &mut Connection, params: &Value) -> Result<Value, String> {
    let sql = params["sql"].as_str().ok_or("raw_sql: missing 'sql'")?;
    // Rewrite $1..$N → ? for SQLite
    let sqlite_sql = rewrite_placeholders(sql);

    let param_values: Vec<String> = params["params"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect();

    let mut stmt = conn.prepare(&sqlite_sql).map_err(|e| format!("raw_sql prepare: {e}"))?;
    let col_count = stmt.column_count();
    let col_names: Vec<String> = (0..col_count)
        .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
        .collect();

    let sql_params: Vec<&dyn ToSql> = param_values.iter()
        .map(|s| s as &dyn ToSql)
        .collect();

    let rows_iter = stmt.query(sql_params.as_slice())
        .map_err(|e| format!("raw_sql query: {e}"))?;
    let mut rows = Vec::new();
    let mut rows_iter = rows_iter;
    while let Some(row) = rows_iter.next().map_err(|e| format!("raw_sql next: {e}"))? {
        let mut obj = serde_json::Map::new();
        for (i, name) in col_names.iter().enumerate() {
            let val: String = row.get::<_, Option<String>>(i)
                .unwrap_or(None)
                .unwrap_or_default();
            obj.insert(name.clone(), Value::String(val));
        }
        rows.push(Value::Object(obj));
    }
    Ok(json!({"rows": rows}))
}

/// Atomically increment a BIGINT column. Inserts row with value 1 if absent.
pub fn atomic_increment(conn: &mut Connection, params: &Value) -> Result<Value, String> {
    let table     = params["table"].as_str().ok_or("atomic_increment: missing 'table'")?;
    let column    = params["column"].as_str().ok_or("atomic_increment: missing 'column'")?;
    let id_col    = params["id_column"].as_str().ok_or("atomic_increment: missing 'id_column'")?;
    let id_val    = params["id_value"].as_str().ok_or("atomic_increment: missing 'id_value'")?;

    // Try UPDATE first
    let updated = conn.execute(
        &format!("UPDATE {table} SET {column} = {column} + 1 WHERE {id_col} = ?"),
        rusqlite::params![id_val],
    ).map_err(|e| format!("atomic_increment update: {e}"))?;

    if updated == 0 {
        // Row didn't exist — insert with value 1
        conn.execute(
            &format!("INSERT OR IGNORE INTO {table} ({id_col}, {column}) VALUES (?, 1)"),
            rusqlite::params![id_val],
        ).map_err(|e| format!("atomic_increment insert: {e}"))?;
    }

    let value: i64 = conn.query_row(
        &format!("SELECT {column} FROM {table} WHERE {id_col} = ?"),
        rusqlite::params![id_val],
        |row| row.get(0),
    ).map_err(|e| format!("atomic_increment select: {e}"))?;

    Ok(json!({"value": value}))
}

/// Rewrite $1, $2, … to ?, ?, … for SQLite compatibility.
fn rewrite_placeholders(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            result.push('?');
            while chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                chars.next();
            }
        } else {
            result.push(ch);
        }
    }
    result
}
```

- [ ] **Step 4: Wire `call_action` into the SQLite driver**

In `lib.rs`, add `mod actions;` and implement:

```rust
fn call_action(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut guard = self.conn.lock().map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or("SQLite connection not initialized")?;
    match action {
        "run_ddl"          => actions::run_ddl(conn, params),
        "raw_sql"          => actions::raw_sql(conn, params),
        "atomic_increment" => actions::atomic_increment(conn, params),
        other              => Err(format!("SQLite driver: action '{other}' not supported")),
    }
}
```

- [ ] **Step 5: Run tests — verify they pass**

```bash
cargo test -p ox_persistence_driver_db_sqlite 2>&1 | tail -10
```

Expected: `test result: ok. 3 passed`

- [ ] **Step 6: Commit**

```bash
git add crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sqlite/
git commit -m "feat(sqlite-driver): implement call_action — run_ddl, raw_sql, atomic_increment"
```

---

## Task 3: Implement call_action for the PostgreSQL driver

**Files:**
- Create: `crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_postgres/src/actions.rs`
- Modify: `crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_postgres/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `tests/actions_test.rs` (skipped in CI if `PG_TEST_URL` env var is absent):

```rust
use ox_persistence_driver_db_postgres::PostgresPersistenceDriver;
use ox_persistence::PersistenceDriver;
use serde_json::json;

fn pg_driver() -> Option<PostgresPersistenceDriver> {
    let url = std::env::var("PG_TEST_URL").ok()?;
    Some(PostgresPersistenceDriver::connect(&url).ok()?)
}

#[test]
fn test_pg_run_ddl_and_raw_sql() {
    let Some(d) = pg_driver() else { return; };
    d.call_action("run_ddl", &json!({
        "sql": "CREATE TABLE IF NOT EXISTS pg_test_tbl (id TEXT PRIMARY KEY, val INTEGER)"
    })).unwrap();
    d.call_action("run_ddl", &json!({"sql": "INSERT INTO pg_test_tbl VALUES ('x', 10) ON CONFLICT DO NOTHING"})).unwrap();

    let res = d.call_action("raw_sql", &json!({
        "sql": "SELECT val FROM pg_test_tbl WHERE id = $1",
        "params": ["x"]
    })).unwrap();
    let rows = res["rows"].as_array().unwrap();
    assert_eq!(rows[0]["val"], "10");

    d.call_action("run_ddl", &json!({"sql": "DROP TABLE pg_test_tbl"})).unwrap();
}
```

- [ ] **Step 2: Create `actions.rs` for PostgreSQL**

```rust
use postgres::{Client, types::ToSql, Row};
use serde_json::{json, Value};
use std::sync::MutexGuard;

pub fn run_ddl(client: &mut Client, params: &Value) -> Result<Value, String> {
    let sql = params["sql"].as_str().ok_or("run_ddl: missing 'sql'")?;
    client.batch_execute(sql).map_err(|e| format!("run_ddl: {e}"))?;
    Ok(json!({"ok": true}))
}

pub fn raw_sql(client: &mut Client, params: &Value) -> Result<Value, String> {
    let sql = params["sql"].as_str().ok_or("raw_sql: missing 'sql'")?;
    let param_strings: Vec<String> = params["params"]
        .as_array().unwrap_or(&vec![])
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect();
    let sql_params: Vec<&(dyn ToSql + Sync)> = param_strings.iter()
        .map(|s| s as &(dyn ToSql + Sync))
        .collect();

    let pg_rows: Vec<Row> = client.query(sql, &sql_params)
        .map_err(|e| format!("raw_sql: {e}"))?;
    let mut rows = Vec::new();
    for row in &pg_rows {
        let mut obj = serde_json::Map::new();
        for col in row.columns() {
            let val: String = row.try_get::<_, Option<String>>(col.name())
                .unwrap_or(None).unwrap_or_default();
            obj.insert(col.name().to_string(), Value::String(val));
        }
        rows.push(Value::Object(obj));
    }
    Ok(json!({"rows": rows}))
}

pub fn atomic_increment(client: &mut Client, params: &Value) -> Result<Value, String> {
    let table  = params["table"].as_str().ok_or("missing 'table'")?;
    let column = params["column"].as_str().ok_or("missing 'column'")?;
    let id_col = params["id_column"].as_str().ok_or("missing 'id_column'")?;
    let id_val = params["id_value"].as_str().ok_or("missing 'id_value'")?;

    let sql = format!(
        "INSERT INTO {table} ({id_col}, {column}) VALUES ($1, 1) \
         ON CONFLICT ({id_col}) DO UPDATE SET {column} = {table}.{column} + 1 \
         RETURNING {column}"
    );
    let rows = client.query(&sql, &[&id_val]).map_err(|e| format!("atomic_increment: {e}"))?;
    let value: i64 = rows[0].get(0);
    Ok(json!({"value": value}))
}
```

- [ ] **Step 3: Wire `call_action` into the Postgres driver**

In `lib.rs`, add `mod actions;` and implement `call_action`:

```rust
fn call_action(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut guard = self.client.lock().map_err(|e| e.to_string())?;
    let client = guard.as_mut().ok_or("PostgreSQL client not initialized")?;
    match action {
        "run_ddl"          => actions::run_ddl(client, params),
        "raw_sql"          => actions::raw_sql(client, params),
        "atomic_increment" => actions::atomic_increment(client, params),
        other              => Err(format!("Postgres driver: action '{other}' not supported")),
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p ox_persistence_driver_db_postgres 2>&1 | tail -5
# With Postgres available:
PG_TEST_URL="postgresql://localhost/test" cargo test -p ox_persistence_driver_db_postgres 2>&1 | tail -5
```

Expected (without Postgres): `test result: ok. 0 passed; 0 filtered out` (test skips).

- [ ] **Step 5: Commit**

```bash
git add crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_postgres/
git commit -m "feat(postgres-driver): implement call_action — run_ddl, raw_sql, atomic_increment"
```

---

## Task 4: Wire operator support into the SQLite fetch path

The SqlBuilder now has `build_fetch_with_ops`. The SQLite driver's `fetch()` currently calls `build_fetch` (equality only). Update it to use the metadata map operators if present.

**Files:**
- Modify: `crates/data/ox_persistence/drivers/db/ox_persistence_driver_db_sqlite/src/lib.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_fetch_with_lte_filter() {
    // Create driver with in-memory DB
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE certs (id TEXT PRIMARY KEY, not_after TEXT, tenant_id TEXT)").unwrap();
    conn.execute_batch("INSERT INTO certs VALUES ('a', '2026-01-01', 'acme')").unwrap();
    conn.execute_batch("INSERT INTO certs VALUES ('b', '2026-12-31', 'acme')").unwrap();
    let driver = /* construct driver with this conn */;

    // Filter: tenant_id = 'acme' AND not_after <= '2026-06-01'
    use std::collections::HashMap;
    use ox_type_converter::ValueType;
    let filter = HashMap::from([
        ("tenant_id".to_string(), ("acme".to_string(), ValueType::Text, HashMap::new())),
        ("not_after".to_string(), ("2026-06-01".to_string(), ValueType::Text,
            HashMap::from([("op".to_string(), "lte".to_string())]))),
    ]);
    let ids = driver.fetch(&filter, "certs").unwrap();
    assert_eq!(ids, vec!["a"]);
}
```

- [ ] **Step 2: Update `fetch()` in the SQLite driver**

Replace the `build_fetch` call with `build_fetch_with_ops`, extracting the metadata map from each filter entry:

```rust
fn fetch(&self, filter: &HashMap<String, (String, ValueType, HashMap<String, String>)>, location: &str) -> Result<Vec<String>, String> {
    let mut guard = self.conn.lock().map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or("not initialized")?;

    let builder = SqlBuilder::new(SqlDialect::Sqlite);
    let keys: Vec<String> = filter.keys().cloned().collect();
    let metas: Vec<HashMap<String, String>> = keys.iter()
        .map(|k| filter[k].2.clone())
        .collect();
    let query = builder.build_fetch_with_ops(location, &keys, &metas);

    // Collect non-null values (skip is_null / is_not_null entries)
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    for key in &keys {
        let (val, vtype, meta) = &filter[key];
        let op = meta.get("op").map(String::as_str).unwrap_or("eq");
        if op == "is_null" || op == "is_not_null" { continue; }
        match vtype {
            ValueType::Integer => params.push(Box::new(val.parse::<i64>().unwrap_or(0))),
            _ => params.push(Box::new(val.clone())),
        }
    }
    // … execute and return IDs as before
}
```

- [ ] **Step 3: Run all SQLite driver tests**

```bash
cargo test -p ox_persistence_driver_db_sqlite 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 4: Apply same change to the PostgreSQL driver**

Mirror the `fetch()` update in the Postgres driver, using `build_fetch_with_ops` and `SqlDialect::Postgres`.

- [ ] **Step 5: Run all persistence driver tests**

```bash
cargo test -p ox_persistence_driver_db_sqlite -p ox_persistence_driver_db_postgres 2>&1 | tail -5
```

- [ ] **Step 6: Commit**

```bash
git add crates/data/ox_persistence/drivers/
git commit -m "feat(persistence-drivers): use operator-aware fetch for comparison filters"
```

---

## Self-Review Checklist

- [x] **DDL execution**: `run_ddl` covers `CREATE TABLE IF NOT EXISTS` for schema bootstrap and migrations.
- [x] **Complex queries**: `raw_sql` covers all ox_cert queries that can't be expressed as equality filters (list_expiring, list_revoked_since, acquire_crl_lock).
- [x] **SSH serial**: `atomic_increment` produces a safe, unique u64 per tenant with no race condition.
- [x] **Filter operators**: `build_fetch_with_ops` supports all comparisons ox_cert needs (lte, gte, is_null).
- [x] **Backwards compatibility**: Existing `build_fetch` and all existing `call_action` defaults unchanged.
- [x] **PostgreSQL placeholder syntax**: `$N` used natively by Postgres; `rewrite_placeholders` converts to `?` for SQLite.
- [ ] **PKCS#11 / other drivers**: No changes needed — those drivers are not used by ox_cert in Phase 1.
