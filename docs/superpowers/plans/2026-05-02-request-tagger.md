# Request Tagger + CoreHostApi Tag ABI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a generic `tags` namespace to the task context (via two new ABI functions) and implement `ox_webservice_request_tagger`, a plugin that matches request paths against URL patterns and stamps key-value tags onto the context for downstream modules to read.

**Architecture:** `CoreHostApi` gains `get_tag`/`set_tag` function pointers backed by a new `tags: HashMap<String, String>` field on `Task`. The tagger plugin reads `content/conf/request_tags.yaml`, compiles each route's regex at init, and on every request evaluates all patterns in order — all matches apply their tags, last-set-wins per key.

**Tech Stack:** Rust, `ox_workflow_abi`, `ox_workflow_core`, `ox_workflow_executor`, `ox_webservice_test_utils`, `regex`, `serde`/`serde_yaml`, `ox_fileproc`

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Modify | `crates/workflow/ox_workflow_core/src/lib.rs` | Add `tags` field to `Task` |
| Modify | `crates/workflow/ox_workflow_abi/src/lib.rs` | Add `get_tag`/`set_tag` to `CoreHostApi` |
| Modify | `crates/workflow/ox_workflow_executor/src/lib.rs` | Implement `get_tag_impl`/`set_tag_impl`, wire into `create_host_api()` |
| Modify | `crates/webservice/ox_webservice_test_utils/src/lib.rs` | Add `tags` to `MockTaskState`, mock functions, `get_mock_tag` helper |
| Create | `crates/webservice/ox_webservice_request_tagger/Cargo.toml` | New crate manifest |
| Create | `crates/webservice/ox_webservice_request_tagger/src/lib.rs` | Plugin: init, process, destroy |
| Create | `crates/webservice/ox_webservice_request_tagger/src/tests.rs` | Plugin unit tests |
| Create | `crates/webservice/ox_webservice_request_tagger/conf/ox_webservice_request_tagger.yaml` | Default module config |
| Modify | `Cargo.toml` | Add new crate to workspace `members` |
| Create | `content/conf/request_tags.yaml` | Default tag routing rules |
| Create | `personas/all-services/modules/active/ox_webservice_request_tagger.yaml` | Activation YAML |

---

## Task 1: Add `tags` field to `Task`

**Files:**
- Modify: `crates/workflow/ox_workflow_core/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/workflow/ox_workflow_core/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_has_empty_tags_on_creation() {
        let task = Task::new(1);
        assert!(task.tags.is_empty());
    }
}
```

- [ ] **Step 2: Run test — verify it fails**

```bash
cd /var/repos/oxIDIZER
cargo test -p ox_workflow_core test_task_has_empty_tags_on_creation 2>&1
```

Expected: compile error — `tags` field does not exist on `Task`.

- [ ] **Step 3: Add `tags` field to `Task`**

In `crates/workflow/ox_workflow_core/src/lib.rs`, add the field to the `Task` struct after `api_limits`:

```rust
    /// Per-function call limits (function name → max, 0 = unlimited)
    pub api_limits: HashMap<String, u32>,
    /// Generic key-value tags set by pipeline plugins (e.g. request_tagger)
    pub tags: HashMap<String, String>,
```

In `Task::new()`, add initialisation after `api_limits`:

```rust
            api_limits: HashMap::new(),
            tags: HashMap::new(),
```

- [ ] **Step 4: Run test — verify it passes**

```bash
cargo test -p ox_workflow_core test_task_has_empty_tags_on_creation 2>&1
```

Expected: `test tests::test_task_has_empty_tags_on_creation ... ok`

- [ ] **Step 5: Verify nothing else broke**

```bash
cargo test -p ox_workflow_core 2>&1
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/workflow/ox_workflow_core/src/lib.rs
git commit -m "feat(core): add tags HashMap to Task for pipeline annotation"
```

---

## Task 2: Extend `CoreHostApi` with `get_tag`/`set_tag`

**Files:**
- Modify: `crates/workflow/ox_workflow_abi/src/lib.rs`
- Modify: `crates/workflow/ox_workflow_executor/src/lib.rs`
- Modify: `crates/webservice/ox_webservice_test_utils/src/lib.rs`

These three files must be changed together — adding fields to `CoreHostApi` is a compile-time breaking change for every site that constructs it.

- [ ] **Step 1: Add `get_tag`/`set_tag` to `CoreHostApi`**

In `crates/workflow/ox_workflow_abi/src/lib.rs`, add two fields at the end of the `CoreHostApi` struct, after `has_field`:

```rust
    pub has_field: extern "C" fn(task_ctx: *mut c_void, key: *const c_char) -> bool,

    // Tag namespace — separate from HTTP request/response fields
    /// Set a tag on the task context (overwrites if key already exists)
    pub set_tag: extern "C" fn(task_ctx: *mut c_void, key: *const c_char, value: *const c_char),
    /// Get a tag value from the task context; returns null if key not present
    pub get_tag: extern "C" fn(task_ctx: *mut c_void, key: *const c_char) -> *const c_char,
```

- [ ] **Step 2: Implement `get_tag_impl`/`set_tag_impl` in executor**

In `crates/workflow/ox_workflow_executor/src/lib.rs`, add the two implementations inside `create_host_api()`, after `has_field_impl`:

```rust
    extern "C" fn set_tag_impl(task_ctx: *mut c_void, key: *const c_char, value: *const c_char) {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let key_str = unsafe { CStr::from_ptr(key) }.to_string_lossy().to_string();
        let val_str = unsafe { CStr::from_ptr(value) }.to_string_lossy().to_string();
        task.tags.insert(key_str, val_str);
    }

    extern "C" fn get_tag_impl(task_ctx: *mut c_void, key: *const c_char) -> *const c_char {
        let task = unsafe { &mut *(task_ctx as *mut Task) };
        let key_str = unsafe { CStr::from_ptr(key) }.to_string_lossy().to_string();
        if let Some(val) = task.tags.get(&key_str) {
            let cstr = CString::new(val.clone()).unwrap_or_default();
            let ptr = cstr.into_raw();
            task.ffi_arena.push(ptr);
            return ptr;
        }
        std::ptr::null()
    }
```

Then add them to the `CoreHostApi { ... }` construction at the bottom of `create_host_api()`:

```rust
        unset_field: unset_field_impl,
        has_field: has_field_impl,
        set_tag: set_tag_impl,
        get_tag: get_tag_impl,
    }
```

- [ ] **Step 3: Update `ox_webservice_test_utils` mock**

In `crates/webservice/ox_webservice_test_utils/src/lib.rs`:

Add `tags` field to `MockTaskState`:

```rust
#[derive(Default)]
pub struct MockTaskState {
    pub fields: HashMap<String, String>,
    pub tags: HashMap<String, String>,
}
```

Add mock functions (after the existing `mock_has_field` function):

```rust
pub extern "C" fn mock_set_tag(task_ctx: *mut c_void, key: *const c_char, value: *const c_char) {
    if task_ctx.is_null() || key.is_null() { return; }
    let key_str = unsafe { CStr::from_ptr(key).to_string_lossy().into_owned() };
    let val_str = if value.is_null() { String::new() } else { unsafe { CStr::from_ptr(value).to_string_lossy().into_owned() } };
    let state = unsafe { &*(task_ctx as *const RwLock<MockTaskState>) };
    let mut lock = state.write().unwrap();
    lock.tags.insert(key_str, val_str);
}

pub extern "C" fn mock_get_tag(task_ctx: *mut c_void, key: *const c_char) -> *const c_char {
    if task_ctx.is_null() || key.is_null() { return ptr::null(); }
    let key_str = unsafe { CStr::from_ptr(key).to_string_lossy().into_owned() };
    let state = unsafe { &*(task_ctx as *const RwLock<MockTaskState>) };
    let lock = state.read().unwrap();
    if let Some(v) = lock.tags.get(&key_str) {
        let c = CString::new(v.as_str()).unwrap();
        return c.into_raw() as *const c_char;
    }
    ptr::null()
}
```

Add `get_mock_tag` helper (after the existing `get_mock_field` function):

```rust
pub fn get_mock_tag(task_ctx: *mut c_void, key: &str) -> Option<String> {
    let state = unsafe { &*(task_ctx as *const RwLock<MockTaskState>) };
    let lock = state.read().unwrap();
    lock.tags.get(key).cloned()
}
```

Add `set_tag` and `get_tag` to `create_mock_api()`:

```rust
pub fn create_mock_api() -> CoreHostApi {
    CoreHostApi {
        get_field: mock_get_field,
        set_field: mock_set_field,
        get_field_bytes: mock_get_field_bytes,
        set_field_bytes: mock_set_field_bytes,
        get_metadata: mock_get_metadata,
        insert_into_flow: mock_insert_into_flow,
        pause_task: mock_pause_task,
        log: mock_log,
        set_flag: mock_set_flag,
        set_flags: mock_set_flags,
        has_flag: mock_has_flag,
        clear_flag: mock_clear_flag,
        get_keys: mock_get_keys,
        unset_field: mock_unset_field,
        has_field: mock_has_field,
        set_tag: mock_set_tag,
        get_tag: mock_get_tag,
    }
}
```

- [ ] **Step 4: Verify the workspace compiles**

```bash
cargo build --workspace 2>&1
```

Expected: clean build, no errors. All existing crates that construct `CoreHostApi` (only `ox_workflow_executor` and `ox_webservice_test_utils`) have been updated.

- [ ] **Step 5: Write executor tag test**

Add to the existing test block in `crates/workflow/ox_workflow_executor/src/lib.rs` (or add `#[cfg(test)] mod tests { ... }` if none exists):

```rust
#[cfg(test)]
mod tag_tests {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::ffi::c_void;

    #[test]
    fn test_set_and_get_tag_via_api() {
        let api = create_host_api();
        let mut task = Task::new(1);
        let task_ptr = &mut task as *mut Task as *mut c_void;

        let key = CString::new("page_type").unwrap();
        let val = CString::new("admin").unwrap();

        (api.set_tag)(task_ptr, key.as_ptr(), val.as_ptr());

        let result = (api.get_tag)(task_ptr, key.as_ptr());
        assert!(!result.is_null());
        let result_str = unsafe { CStr::from_ptr(result).to_string_lossy() };
        assert_eq!(result_str, "admin");
    }

    #[test]
    fn test_get_tag_missing_key_returns_null() {
        let api = create_host_api();
        let mut task = Task::new(1);
        let task_ptr = &mut task as *mut Task as *mut c_void;

        let key = CString::new("nonexistent").unwrap();
        let result = (api.get_tag)(task_ptr, key.as_ptr());
        assert!(result.is_null());
    }

    #[test]
    fn test_set_tag_overwrites_existing() {
        let api = create_host_api();
        let mut task = Task::new(1);
        let task_ptr = &mut task as *mut Task as *mut c_void;

        let key = CString::new("page_type").unwrap();
        let v1 = CString::new("default").unwrap();
        let v2 = CString::new("admin").unwrap();

        (api.set_tag)(task_ptr, key.as_ptr(), v1.as_ptr());
        (api.set_tag)(task_ptr, key.as_ptr(), v2.as_ptr());

        let result = (api.get_tag)(task_ptr, key.as_ptr());
        let result_str = unsafe { CStr::from_ptr(result).to_string_lossy() };
        assert_eq!(result_str, "admin");
    }
}
```

- [ ] **Step 6: Run executor tag tests**

```bash
cargo test -p ox_workflow_executor tag_tests 2>&1
```

Expected: all three tests pass.

- [ ] **Step 7: Run full test suite**

```bash
cargo test --workspace 2>&1
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add \
  crates/workflow/ox_workflow_abi/src/lib.rs \
  crates/workflow/ox_workflow_executor/src/lib.rs \
  crates/webservice/ox_webservice_test_utils/src/lib.rs
git commit -m "feat(abi): add get_tag/set_tag to CoreHostApi and Task"
```

---

## Task 3: Create `ox_webservice_request_tagger` crate

**Files:**
- Create: `crates/webservice/ox_webservice_request_tagger/Cargo.toml`
- Create: `crates/webservice/ox_webservice_request_tagger/src/lib.rs`
- Create: `crates/webservice/ox_webservice_request_tagger/src/tests.rs`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Create the Cargo.toml**

Create `crates/webservice/ox_webservice_request_tagger/Cargo.toml`:

```toml
[package]
name = "ox_webservice_request_tagger"
version = "0.0.1"
license = "GPL-3.0-only"
edition = "2024"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc = { path = "../../util/ox_fileproc" }
regex = "1.10"
libc = "0.2"

[dev-dependencies]
tempfile = "3.23.0"
ox_webservice_test_utils = { path = "../ox_webservice_test_utils" }
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
```

- [ ] **Step 2: Add to workspace**

In the root `Cargo.toml`, add to the `# Plugins - content serving` section:

```toml
    # Plugins - content serving
    "crates/webservice/ox_webservice_stream",
    "crates/webservice/ox_webservice_template_jinja2",
    "crates/webservice/ox_webservice_request_tagger",
```

- [ ] **Step 3: Write the failing tests first**

Create `crates/webservice/ox_webservice_request_tagger/src/tests.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::{ox_plugin_init, ox_plugin_process};
    use ox_webservice_test_utils::{
        create_mock_api, create_task_state, drop_task_state,
        get_mock_tag, set_mock_field, PluginHandle,
    };
    use ox_workflow_abi::FLOW_CONTROL_CONTINUE;
    use std::io::Write;
    use tempfile::Builder;

    fn make_tags_file(content: &str) -> tempfile::NamedTempFile {
        let mut f = Builder::new().suffix(".yaml").tempfile().unwrap();
        writeln!(f, "{}", content).unwrap();
        f
    }

    fn make_config(tags_file_path: &str) -> tempfile::NamedTempFile {
        let mut f = Builder::new().suffix(".yaml").tempfile().unwrap();
        writeln!(f, "tags_file: \"{}\"", tags_file_path).unwrap();
        f
    }

    #[test]
    fn test_single_pattern_sets_tags() {
        let tags_file = make_tags_file(
            "routes:\n  - pattern: \".*\"\n    tags:\n      - page_type: \"default\""
        );
        let config = make_config(tags_file.path().to_str().unwrap());
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config.path().to_str().unwrap());
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("init failed");

        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/some/path");
        let flow = handle.process(ox_plugin_process, task_ctx);

        assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
        assert_eq!(get_mock_tag(task_ctx, "page_type"), Some("default".to_string()));

        unsafe { drop_task_state(task_ctx); }
    }

    #[test]
    fn test_all_patterns_evaluated_last_set_wins() {
        let tags_file = make_tags_file(
"routes:
  - pattern: \".*\"
    tags:
      - page_type: \"default\"
      - auth_required: \"false\"
  - pattern: \"^/admin/\"
    tags:
      - page_type: \"admin\"
      - auth_required: \"true\"
  - pattern: \"^/admin/public/\"
    tags:
      - auth_required: \"false\""
        );
        let config = make_config(tags_file.path().to_str().unwrap());
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config.path().to_str().unwrap());
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("init failed");

        // /admin/public/ matches all three routes
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/admin/public/dashboard");
        handle.process(ox_plugin_process, task_ctx);

        // last match for page_type was /admin/ rule → "admin"
        assert_eq!(get_mock_tag(task_ctx, "page_type"), Some("admin".to_string()));
        // last match for auth_required was /admin/public/ rule → "false"
        assert_eq!(get_mock_tag(task_ctx, "auth_required"), Some("false".to_string()));

        unsafe { drop_task_state(task_ctx); }
    }

    #[test]
    fn test_non_matching_pattern_sets_no_tags() {
        let tags_file = make_tags_file(
            "routes:\n  - pattern: \"^/admin/\"\n    tags:\n      - page_type: \"admin\""
        );
        let config = make_config(tags_file.path().to_str().unwrap());
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config.path().to_str().unwrap());
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("init failed");

        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/public/page");
        handle.process(ox_plugin_process, task_ctx);

        assert_eq!(get_mock_tag(task_ctx, "page_type"), None);

        unsafe { drop_task_state(task_ctx); }
    }

    #[test]
    fn test_empty_path_is_handled_gracefully() {
        let tags_file = make_tags_file(
            "routes:\n  - pattern: \".*\"\n    tags:\n      - page_type: \"default\""
        );
        let config = make_config(tags_file.path().to_str().unwrap());
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config.path().to_str().unwrap());
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("init failed");

        let task_ctx = create_task_state();
        // no request.path set
        let flow = handle.process(ox_plugin_process, task_ctx);

        assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);

        unsafe { drop_task_state(task_ctx); }
    }

    #[test]
    fn test_init_fails_on_missing_tags_file() {
        let mut config = Builder::new().suffix(".yaml").tempfile().unwrap();
        writeln!(config, "tags_file: \"/nonexistent/path/tags.yaml\"").unwrap();
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config.path().to_str().unwrap());
        let result = PluginHandle::init(ox_plugin_init, &params, &api);
        assert!(result.is_err(), "should fail when tags_file does not exist");
    }

    #[test]
    fn test_init_fails_on_missing_config_file() {
        let api = create_mock_api();
        let result = PluginHandle::init(ox_plugin_init, r#"{"config_file": "/no/such/file.yaml"}"#, &api);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 4: Run tests — verify they fail**

```bash
cargo test -p ox_webservice_request_tagger 2>&1
```

Expected: compile error — `src/lib.rs` does not exist yet.

- [ ] **Step 5: Create the plugin implementation**

Create `crates/webservice/ox_webservice_request_tagger/src/lib.rs`:

```rust
use libc::{c_char, c_void};
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO, OX_LOG_WARN};
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::path::PathBuf;

mod tests;

const MODULE_NAME: &str = "ox_webservice_request_tagger";

#[derive(Debug, Deserialize)]
struct TaggerConfig {
    tags_file: String,
}

#[derive(Debug, Deserialize)]
struct TagsConfig {
    routes: Vec<TagRoute>,
}

#[derive(Debug, Deserialize)]
struct TagRoute {
    pattern: String,
    tags: Vec<HashMap<String, String>>,
    #[serde(skip)]
    compiled_regex: Option<Regex>,
}

pub struct ModuleContext {
    routes: Vec<TagRoute>,
    api: CoreHostApi,
}

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let res_ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if res_ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(res_ptr).to_string_lossy().into_owned() }
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };

    let params_str = if !plugin_config_ctx.is_null() {
        unsafe { CStr::from_ptr(plugin_config_ctx).to_string_lossy().to_string() }
    } else { "{}".to_string() };

    let params: Value = serde_json::from_str(&params_str).unwrap_or(Value::Null);

    let config_file = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("{}: missing config_file param", MODULE_NAME));
            return std::ptr::null_mut();
        }
    };

    let tagger_config: TaggerConfig = match ox_fileproc::process_file(&PathBuf::from(&config_file), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("{}: bad config: {}", MODULE_NAME, e));
                return std::ptr::null_mut();
            }
        },
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("{}: cannot read config: {}", MODULE_NAME, e));
            return std::ptr::null_mut();
        }
    };

    let mut tags_config: TagsConfig = match ox_fileproc::process_file(&PathBuf::from(&tagger_config.tags_file), 5) {
        Ok(v) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("{}: bad tags_file: {}", MODULE_NAME, e));
                return std::ptr::null_mut();
            }
        },
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR, &format!("{}: cannot read tags_file: {}", MODULE_NAME, e));
            return std::ptr::null_mut();
        }
    };

    for route in &mut tags_config.routes {
        match Regex::new(&route.pattern) {
            Ok(re) => route.compiled_regex = Some(re),
            Err(e) => {
                log(&api, std::ptr::null_mut(), OX_LOG_WARN, &format!("{}: invalid regex '{}': {}", MODULE_NAME, route.pattern, e));
            }
        }
    }

    log(&api, std::ptr::null_mut(), OX_LOG_INFO, &format!("{}: initialised with {} routes", MODULE_NAME, tags_config.routes.len()));

    let ctx = Box::new(ModuleContext { routes: tags_config.routes, api });
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    if plugin_config_ctx.is_null() {
        return FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    }
    let context = unsafe { &*(plugin_config_ctx as *mut ModuleContext) };
    let api = &context.api;

    let path = {
        let p = get_field(api, task_ctx, "request.path");
        if p.is_empty() { "/".to_string() } else { p }
    };

    for route in &context.routes {
        if let Some(re) = &route.compiled_regex {
            if re.is_match(&path) {
                for tag_map in &route.tags {
                    for (key, value) in tag_map {
                        if let (Ok(k), Ok(v)) = (CString::new(key.as_str()), CString::new(value.as_str())) {
                            (api.set_tag)(task_ctx, k.as_ptr(), v.as_ptr());
                        }
                    }
                }
            }
        }
    }

    FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = unsafe { Box::from_raw(plugin_config_ctx as *mut ModuleContext) };
    }
}
```

- [ ] **Step 6: Run tests — verify they pass**

```bash
cargo test -p ox_webservice_request_tagger 2>&1
```

Expected: all 6 tests pass.

- [ ] **Step 7: Run full workspace tests**

```bash
cargo test --workspace 2>&1
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add \
  crates/webservice/ox_webservice_request_tagger/ \
  Cargo.toml
git commit -m "feat: add ox_webservice_request_tagger plugin"
```

---

## Task 4: Create config files and activation YAML

**Files:**
- Create: `crates/webservice/ox_webservice_request_tagger/conf/ox_webservice_request_tagger.yaml`
- Create: `content/conf/request_tags.yaml`
- Create: `personas/all-services/modules/active/ox_webservice_request_tagger.yaml`

- [ ] **Step 1: Create the module default config**

Create `crates/webservice/ox_webservice_request_tagger/conf/ox_webservice_request_tagger.yaml`:

```yaml
tags_file: "/var/repos/oxIDIZER/content/conf/request_tags.yaml"
```

- [ ] **Step 2: Create the default tag routing rules**

Create `content/conf/request_tags.yaml`:

```yaml
# URL pattern → tag mappings for ox_webservice_request_tagger.
# All matching patterns are evaluated in order. For any given tag key,
# the last matching pattern's value wins.
routes:
  - pattern: ".*"
    tags:
      - page_type: "default"
```

- [ ] **Step 3: Create the activation YAML**

Create `personas/all-services/modules/active/ox_webservice_request_tagger.yaml`:

```yaml
modules:
  - id: "request_tagger"
    name: "ox_webservice_request_tagger"
    params:
      config_file: "/var/repos/oxIDIZER/crates/webservice/ox_webservice_request_tagger/conf/ox_webservice_request_tagger.yaml"

routes:
  - url: ".*"
    module_id: "request_tagger"
    stage: PreContent
    priority: 1
```

- [ ] **Step 4: Build to confirm config files are well-formed**

```bash
cargo build -p ox_webservice_request_tagger 2>&1
```

Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add \
  crates/webservice/ox_webservice_request_tagger/conf/ \
  content/conf/request_tags.yaml \
  personas/all-services/modules/active/ox_webservice_request_tagger.yaml
git commit -m "feat: add request_tagger config files and activation YAML"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Covered by |
|---|---|
| `get_tag`/`set_tag` ABI extension on `CoreHostApi` | Task 2 |
| `tags` stored in dedicated namespace separate from request/response fields | Task 1 (Task struct), Task 2 (separate function pointers) |
| All patterns evaluated in order, last-set-wins | Task 3 `ox_plugin_process` logic + test |
| Tags config in `content/conf/request_tags.yaml` | Task 4 |
| Module config in crate `conf/` dir linked via activation YAML | Task 4 |
| `stage: PreContent` so tagger runs before content modules | Task 4 activation YAML |
| Init fails gracefully on missing tags_file | Task 3 test + init error handling |

**No placeholders found.**

**Type consistency:** `set_tag`/`get_tag` named consistently in `CoreHostApi` struct (Task 2), `set_tag_impl`/`get_tag_impl` in executor (Task 2), `mock_set_tag`/`mock_get_tag` in test_utils (Task 2), `get_mock_tag` helper (Task 2). `ox_plugin_process` calls `api.set_tag` (Task 3). All names consistent.

---

> **Note:** Plan 2 (`ox_webservice_layout` — streaming SSR, hook dispatch, template selection) depends on this plan being complete and should be written and executed separately.
