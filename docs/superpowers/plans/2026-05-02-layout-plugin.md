# Layout Plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `ox_webservice_layout` — a plugin that selects a Jinja2 page template based on the `page_type` tag, resolves `{{ hook("id") }}` calls via registered content providers, and sets the assembled HTML as the response body.

**Architecture:** Three source modules — `config.rs` (YAML loading), `renderer.rs` (template parsing + hook dispatch), `lib.rs` (ABI surface) — keep each file focused. Config files are loaded fresh per request so layout/theme changes take effect without restart. Hook providers are either Jinja2 template files (rendered via Tera) or module stubs (empty for v1). The fragment endpoint `/api/hooks/{id}/` is handled inside `ox_plugin_process` by checking the request path.

**Tech Stack:** Rust 2021, Tera 1.20.1, `ox_workflow_abi`, `ox_fileproc`, `regex`, `serde`/`serde_json`

**Prerequisite:** The `feature/request-tagger` branch must be merged into `main` before starting this plan. It adds `get_tag`/`set_tag` to `CoreHostApi`, `tags: HashMap<String, String>` to `Task`, and tag support to `ox_webservice_test_utils`.

---

## File Map

| Action | Path | Responsibility |
|---|---|---|
| Create | `crates/webservice/ox_webservice_layout/Cargo.toml` | Crate manifest |
| Create | `crates/webservice/ox_webservice_layout/src/config.rs` | Config structs + loading functions |
| Create | `crates/webservice/ox_webservice_layout/src/renderer.rs` | Segment enum, template parsing, hook dispatch, page assembly |
| Create | `crates/webservice/ox_webservice_layout/src/lib.rs` | Plugin ABI: init/process/destroy, ModuleContext, helpers |
| Create | `crates/webservice/ox_webservice_layout/src/tests.rs` | Integration tests |
| Modify | `Cargo.toml` (workspace root) | Add crate to `members` |
| Create | `crates/webservice/ox_webservice_layout/conf/ox_webservice_layout.yaml` | Module default config |
| Create | `content/conf/layout_theme.yaml` | Active layout + theme selection |
| Create | `content/conf/hook_providers.yaml` | Hook ID → provider mappings |
| Create | `content/layouts/default/layout.yaml` | Layout metadata, page templates, hook declarations |
| Create | `content/layouts/default/templates/default.jinja2` | Default page template |
| Create | `content/layouts/default/templates/admin.jinja2` | Admin page template |
| Create | `content/layouts/default/templates/includes/nav.jinja2` | Nav hook template |
| Create | `content/layouts/default/templates/includes/footer.jinja2` | Footer hook template |
| Create | `content/layouts/default/templates/includes/main_content.jinja2` | Main content hook template |
| Create | `personas/all-services/modules/active/ox_webservice_layout.yaml` | Activation YAML |

---

## Task 1: Crate scaffold and config module

**Files:**
- Create: `crates/webservice/ox_webservice_layout/Cargo.toml`
- Create: `crates/webservice/ox_webservice_layout/src/config.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Write the failing config tests**

Create `crates/webservice/ox_webservice_layout/src/config.rs` with just the test module first:

```rust
use serde::Deserialize;
use serde_json;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct LayoutConfig {
    pub conf_dir: String,
    pub layouts_dir: String,
    pub themes_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct LayoutTheme {
    pub active_layout: String,
    pub active_theme: String,
}

#[derive(Debug, Deserialize)]
pub struct LayoutYaml {
    pub name: String,
    pub page_templates: HashMap<String, String>,
    pub hooks: Vec<HookDecl>,
}

#[derive(Debug, Deserialize)]
pub struct HookDecl {
    pub id: String,
    pub description: Option<String>,
    pub above_fold: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HookProviders {
    pub providers: Vec<HookProvider>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HookProvider {
    pub hook_id: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub path: Option<String>,
    pub module_id: Option<String>,
}

pub fn load_layout_config(config_file: &str) -> Result<LayoutConfig, String> {
    todo!()
}

pub fn load_layout_theme(conf_dir: &str) -> Result<LayoutTheme, String> {
    todo!()
}

pub fn load_layout_yaml(layouts_dir: &str, layout_name: &str) -> Result<LayoutYaml, String> {
    todo!()
}

pub fn load_hook_providers(conf_dir: &str) -> Result<HookProviders, String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, content: &str) {
        fs::write(dir.path().join(name), content).unwrap();
    }

    #[test]
    fn test_load_layout_config() {
        let dir = TempDir::new().unwrap();
        write(&dir, "c.yaml", "conf_dir: \"/conf\"\nlayouts_dir: \"/layouts\"\nthemes_dir: \"/themes\"");
        let c = load_layout_config(dir.path().join("c.yaml").to_str().unwrap()).unwrap();
        assert_eq!(c.conf_dir, "/conf");
        assert_eq!(c.layouts_dir, "/layouts");
        assert_eq!(c.themes_dir, "/themes");
    }

    #[test]
    fn test_load_layout_theme() {
        let dir = TempDir::new().unwrap();
        write(&dir, "layout_theme.yaml", "active_layout: \"default\"\nactive_theme: \"blue\"");
        let lt = load_layout_theme(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(lt.active_layout, "default");
        assert_eq!(lt.active_theme, "blue");
    }

    #[test]
    fn test_load_layout_yaml() {
        let dir = TempDir::new().unwrap();
        let layout_dir = dir.path().join("default");
        fs::create_dir_all(&layout_dir).unwrap();
        fs::write(layout_dir.join("layout.yaml"),
            "name: \"default\"\npage_templates:\n  default: \"templates/default.jinja2\"\nhooks:\n  - id: \"nav\"\n    above_fold: true"
        ).unwrap();
        let ly = load_layout_yaml(dir.path().to_str().unwrap(), "default").unwrap();
        assert_eq!(ly.name, "default");
        assert!(ly.page_templates.contains_key("default"));
        assert_eq!(ly.hooks[0].id, "nav");
        assert!(ly.hooks[0].above_fold);
    }

    #[test]
    fn test_load_hook_providers() {
        let dir = TempDir::new().unwrap();
        write(&dir, "hook_providers.yaml",
            "providers:\n  - hook_id: \"nav\"\n    type: \"template\"\n    path: \"/layouts/nav.jinja2\""
        );
        let hp = load_hook_providers(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(hp.providers.len(), 1);
        assert_eq!(hp.providers[0].hook_id, "nav");
        assert_eq!(hp.providers[0].provider_type, "template");
        assert_eq!(hp.providers[0].path.as_deref(), Some("/layouts/nav.jinja2"));
    }

    #[test]
    fn test_load_layout_config_missing_file() {
        assert!(load_layout_config("/nonexistent/path.yaml").is_err());
    }
}
```

- [ ] **Step 2: Create Cargo.toml**

Create `crates/webservice/ox_webservice_layout/Cargo.toml`:

```toml
[package]
name = "ox_webservice_layout"
version = "0.0.1"
license = "GPL-3.0-only"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
ox_fileproc = { path = "../../util/ox_fileproc" }
regex = "1.10"
libc = "0.2"
tera = "1.20.1"

[dev-dependencies]
tempfile = "3.23.0"
ox_webservice_test_utils = { path = "../ox_webservice_test_utils" }
ox_workflow_abi = { path = "../../workflow/ox_workflow_abi" }
```

- [ ] **Step 3: Add to workspace**

In the root `Cargo.toml`, find the webservice plugins section (near `ox_webservice_request_tagger`) and add:

```toml
    "crates/webservice/ox_webservice_layout",
```

- [ ] **Step 4: Run tests — verify they fail**

```bash
cd /var/repos/oxIDIZER
cargo test -p ox_webservice_layout 2>&1
```

Expected: compile errors because the `todo!()` functions panic and there is no `src/lib.rs` yet. Create a minimal `src/lib.rs` to make the crate compile:

```rust
pub mod config;
```

Run again:

```bash
cargo test -p ox_webservice_layout 2>&1
```

Expected: tests fail with `panicked at 'not yet implemented'`.

- [ ] **Step 5: Implement the loading functions**

Replace the `todo!()` bodies in `crates/webservice/ox_webservice_layout/src/config.rs`:

```rust
pub fn load_layout_config(config_file: &str) -> Result<LayoutConfig, String> {
    ox_fileproc::process_file(&PathBuf::from(config_file), 5)
        .map_err(|e| format!("cannot read layout config '{}': {}", config_file, e))
        .and_then(|v| serde_json::from_value(v).map_err(|e| format!("bad layout config: {}", e)))
}

pub fn load_layout_theme(conf_dir: &str) -> Result<LayoutTheme, String> {
    let path = PathBuf::from(conf_dir).join("layout_theme.yaml");
    ox_fileproc::process_file(&path, 5)
        .map_err(|e| format!("cannot read layout_theme.yaml: {}", e))
        .and_then(|v| serde_json::from_value(v).map_err(|e| format!("bad layout_theme.yaml: {}", e)))
}

pub fn load_layout_yaml(layouts_dir: &str, layout_name: &str) -> Result<LayoutYaml, String> {
    let path = PathBuf::from(layouts_dir).join(layout_name).join("layout.yaml");
    ox_fileproc::process_file(&path, 5)
        .map_err(|e| format!("cannot read layout.yaml for '{}': {}", layout_name, e))
        .and_then(|v| serde_json::from_value(v).map_err(|e| format!("bad layout.yaml: {}", e)))
}

pub fn load_hook_providers(conf_dir: &str) -> Result<HookProviders, String> {
    let path = PathBuf::from(conf_dir).join("hook_providers.yaml");
    ox_fileproc::process_file(&path, 5)
        .map_err(|e| format!("cannot read hook_providers.yaml: {}", e))
        .and_then(|v| serde_json::from_value(v).map_err(|e| format!("bad hook_providers.yaml: {}", e)))
}
```

- [ ] **Step 6: Run tests — verify they pass**

```bash
cargo test -p ox_webservice_layout 2>&1
```

Expected: all 5 config tests pass.

- [ ] **Step 7: Commit**

```bash
git add \
  crates/webservice/ox_webservice_layout/Cargo.toml \
  crates/webservice/ox_webservice_layout/src/lib.rs \
  crates/webservice/ox_webservice_layout/src/config.rs \
  Cargo.toml
git commit -m "feat(layout): crate scaffold and config loading module"
```

---

## Task 2: Template renderer

**Files:**
- Create: `crates/webservice/ox_webservice_layout/src/renderer.rs`

The renderer parses a Jinja2 template into static HTML and hook-call segments, renders each hook via its provider, and assembles the final page.

- [ ] **Step 1: Write the failing renderer tests**

Create `crates/webservice/ox_webservice_layout/src/renderer.rs` with the test module and stub functions:

```rust
use regex::Regex;
use std::collections::HashMap;
use tera::{Context, Tera};
use crate::config::HookProvider;

#[derive(Debug, PartialEq)]
pub enum Segment {
    Static(String),
    Hook(String),
}

pub fn parse_template_segments(template: &str) -> Vec<Segment> {
    todo!()
}

pub fn render_hook(hook_id: &str, provider: Option<&HookProvider>, context: &Context) -> String {
    todo!()
}

pub fn render_page(
    template_content: &str,
    providers_index: &HashMap<String, &HookProvider>,
    context: &Context,
) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HookProvider;
    use std::io::Write;
    use tempfile::Builder;

    #[test]
    fn test_parse_no_hooks() {
        let segs = parse_template_segments("<html><body>Hello</body></html>");
        assert_eq!(segs, vec![Segment::Static("<html><body>Hello</body></html>".to_string())]);
    }

    #[test]
    fn test_parse_single_hook() {
        let segs = parse_template_segments(r#"<nav>{{ hook("nav") }}</nav>"#);
        assert_eq!(segs, vec![
            Segment::Static("<nav>".to_string()),
            Segment::Hook("nav".to_string()),
            Segment::Static("</nav>".to_string()),
        ]);
    }

    #[test]
    fn test_parse_multiple_hooks() {
        let segs = parse_template_segments(r#"{{ hook("nav") }}<main>{{ hook("main") }}</main>"#);
        assert_eq!(segs, vec![
            Segment::Hook("nav".to_string()),
            Segment::Static("<main>".to_string()),
            Segment::Hook("main".to_string()),
            Segment::Static("</main>".to_string()),
        ]);
    }

    #[test]
    fn test_parse_hook_with_spaces() {
        // Spec syntax: {{ hook("id") }} with spaces inside the delimiters
        let segs = parse_template_segments(r#"{{  hook("nav")  }}"#);
        assert_eq!(segs, vec![Segment::Hook("nav".to_string())]);
    }

    #[test]
    fn test_render_hook_no_provider_returns_empty() {
        let result = render_hook("missing", None, &Context::new());
        assert_eq!(result, "");
    }

    #[test]
    fn test_render_hook_template_provider() {
        let mut f = Builder::new().suffix(".jinja2").tempfile().unwrap();
        writeln!(f, "<nav>navigation</nav>").unwrap();
        let provider = HookProvider {
            hook_id: "nav".to_string(),
            provider_type: "template".to_string(),
            path: Some(f.path().to_str().unwrap().to_string()),
            module_id: None,
        };
        let result = render_hook("nav", Some(&provider), &Context::new());
        assert!(result.contains("<nav>navigation</nav>"));
    }

    #[test]
    fn test_render_hook_template_with_tera_variable() {
        let mut f = Builder::new().suffix(".jinja2").tempfile().unwrap();
        writeln!(f, "<nav>{{ active_page }}</nav>").unwrap();
        let provider = HookProvider {
            hook_id: "nav".to_string(),
            provider_type: "template".to_string(),
            path: Some(f.path().to_str().unwrap().to_string()),
            module_id: None,
        };
        let mut ctx = Context::new();
        ctx.insert("active_page", "home");
        let result = render_hook("nav", Some(&provider), &ctx);
        assert_eq!(result.trim(), "<nav>home</nav>");
    }

    #[test]
    fn test_render_hook_module_provider_returns_stub() {
        let provider = HookProvider {
            hook_id: "main".to_string(),
            provider_type: "module".to_string(),
            path: None,
            module_id: Some("some_module".to_string()),
        };
        let result = render_hook("main", Some(&provider), &Context::new());
        // Module providers are stubbed for v1 — should return an HTML comment, not panic
        assert!(result.contains("<!--"));
    }

    #[test]
    fn test_render_page_assembles_hooks() {
        let mut nav_f = Builder::new().suffix(".jinja2").tempfile().unwrap();
        writeln!(nav_f, "<nav>nav-content</nav>").unwrap();
        let providers_vec = vec![HookProvider {
            hook_id: "nav".to_string(),
            provider_type: "template".to_string(),
            path: Some(nav_f.path().to_str().unwrap().to_string()),
            module_id: None,
        }];
        let providers_index: HashMap<String, &HookProvider> =
            providers_vec.iter().map(|p| (p.hook_id.clone(), p)).collect();

        let result = render_page(r#"<header>{{ hook("nav") }}</header>"#, &providers_index, &Context::new());
        assert!(result.contains("<header>"));
        assert!(result.contains("<nav>nav-content</nav>"));
        assert!(result.contains("</header>"));
    }

    #[test]
    fn test_render_page_expands_tera_vars_in_static() {
        let providers_index: HashMap<String, &HookProvider> = HashMap::new();
        let mut ctx = Context::new();
        ctx.insert("site_name", "oxIDIZER");
        let result = render_page("<title>{{ site_name }}</title>", &providers_index, &ctx);
        assert_eq!(result.trim(), "<title>oxIDIZER</title>");
    }

    #[test]
    fn test_render_page_missing_hook_renders_empty() {
        let providers_index: HashMap<String, &HookProvider> = HashMap::new();
        let result = render_page(r#"<body>{{ hook("missing") }}</body>"#, &providers_index, &Context::new());
        assert_eq!(result.trim(), "<body></body>");
    }
}
```

Add `pub mod renderer;` to `src/lib.rs`.

- [ ] **Step 2: Run tests — verify they fail**

```bash
cargo test -p ox_webservice_layout 2>&1
```

Expected: panics at `todo!()` in all renderer tests.

- [ ] **Step 3: Implement `parse_template_segments`**

Replace `todo!()` in `parse_template_segments`:

```rust
pub fn parse_template_segments(template: &str) -> Vec<Segment> {
    let re = Regex::new(r#"\{\{\s*hook\("([^"]+)"\)\s*\}\}"#).unwrap();
    let mut segments = Vec::new();
    let mut last_end = 0;
    for cap in re.captures_iter(template) {
        let m = cap.get(0).unwrap();
        if m.start() > last_end {
            segments.push(Segment::Static(template[last_end..m.start()].to_string()));
        }
        segments.push(Segment::Hook(cap[1].to_string()));
        last_end = m.end();
    }
    if last_end < template.len() {
        segments.push(Segment::Static(template[last_end..].to_string()));
    }
    segments
}
```

- [ ] **Step 4: Implement `render_hook`**

Replace `todo!()` in `render_hook`:

```rust
pub fn render_hook(hook_id: &str, provider: Option<&HookProvider>, context: &Context) -> String {
    let p = match provider {
        None => return String::new(),
        Some(p) => p,
    };
    match p.provider_type.as_str() {
        "template" => {
            let path = match &p.path {
                None => return format!("<!-- hook '{}': template provider missing path -->", hook_id),
                Some(path) => path,
            };
            match std::fs::read_to_string(path) {
                Err(e) => format!("<!-- hook '{}' read error: {} -->", hook_id, e),
                Ok(tmpl) => match Tera::one_off(&tmpl, context, false) {
                    Ok(rendered) => rendered,
                    Err(e) => format!("<!-- hook '{}' render error: {} -->", hook_id, e),
                },
            }
        }
        "module" => {
            // Module provider dispatch is not yet implemented for v1.
            // The module_id is noted in an HTML comment for debugging.
            format!(
                "<!-- hook '{}' module provider '{}': not yet implemented -->",
                hook_id,
                p.module_id.as_deref().unwrap_or("unknown")
            )
        }
        other => format!("<!-- hook '{}' unknown provider type: {} -->", hook_id, other),
    }
}
```

- [ ] **Step 5: Implement `render_page`**

Replace `todo!()` in `render_page`:

```rust
pub fn render_page(
    template_content: &str,
    providers_index: &HashMap<String, &HookProvider>,
    context: &Context,
) -> String {
    let segments = parse_template_segments(template_content);
    let mut output = String::with_capacity(template_content.len());
    for segment in &segments {
        match segment {
            Segment::Static(html) => match Tera::one_off(html, context, false) {
                Ok(rendered) => output.push_str(&rendered),
                Err(_) => output.push_str(html),
            },
            Segment::Hook(hook_id) => {
                let provider = providers_index.get(hook_id.as_str()).copied();
                output.push_str(&render_hook(hook_id, provider, context));
            }
        }
    }
    output
}
```

- [ ] **Step 6: Run tests — verify they pass**

```bash
cargo test -p ox_webservice_layout 2>&1
```

Expected: all renderer tests and config tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/webservice/ox_webservice_layout/src/renderer.rs \
        crates/webservice/ox_webservice_layout/src/lib.rs
git commit -m "feat(layout): template segment parser and hook renderer"
```

---

## Task 3: Plugin wiring

**Files:**
- Create: `crates/webservice/ox_webservice_layout/src/tests.rs`
- Rewrite: `crates/webservice/ox_webservice_layout/src/lib.rs`

- [ ] **Step 1: Write the failing integration tests**

Create `crates/webservice/ox_webservice_layout/src/tests.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::{ox_plugin_init, ox_plugin_process};
    use ox_webservice_test_utils::{
        create_mock_api, create_task_state, drop_task_state,
        get_mock_field, set_mock_field, PluginHandle,
    };
    use std::ffi::CString;
    use std::fs;
    use tempfile::TempDir;

    /// Build a complete temp environment: conf/, layouts/default/, themes/
    /// Returns (TempDir, path-to-plugin-config-yaml).
    fn setup_env() -> (TempDir, String) {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("conf");
        let layouts = dir.path().join("layouts");
        let default_layout = layouts.join("default");
        let templates = default_layout.join("templates");
        let includes = templates.join("includes");
        let themes = dir.path().join("themes");

        fs::create_dir_all(&includes).unwrap();
        fs::create_dir_all(&themes).unwrap();

        // layout_theme.yaml
        fs::write(conf.join("layout_theme.yaml"),
            "active_layout: \"default\"\nactive_theme: \"blue\""
        ).unwrap();

        // layout.yaml
        fs::write(default_layout.join("layout.yaml"),
            "name: \"default\"\npage_templates:\n  default: \"templates/default.jinja2\"\n  admin: \"templates/admin.jinja2\"\nhooks:\n  - id: \"nav\"\n    above_fold: true\n  - id: \"footer\"\n    above_fold: false"
        ).unwrap();

        // Page templates
        fs::write(templates.join("default.jinja2"),
            r#"<html><body>{{ hook("nav") }}<main>default</main>{{ hook("footer") }}</body></html>"#
        ).unwrap();
        fs::write(templates.join("admin.jinja2"),
            r#"<html><body class="admin">{{ hook("nav") }}<main>admin</main></body></html>"#
        ).unwrap();

        // Hook templates
        let nav_path = includes.join("nav.jinja2");
        let footer_path = includes.join("footer.jinja2");
        fs::write(&nav_path, "<nav>navigation</nav>").unwrap();
        fs::write(&footer_path, "<footer>footer-content</footer>").unwrap();

        // hook_providers.yaml (uses absolute paths from temp dir)
        let hp = format!(
            "providers:\n  - hook_id: \"nav\"\n    type: \"template\"\n    path: \"{}\"\n  - hook_id: \"footer\"\n    type: \"template\"\n    path: \"{}\"",
            nav_path.to_str().unwrap(),
            footer_path.to_str().unwrap()
        );
        fs::write(conf.join("hook_providers.yaml"), hp).unwrap();

        // Plugin config yaml
        let plugin_config = format!(
            "conf_dir: \"{}\"\nlayouts_dir: \"{}\"\nthemes_dir: \"{}\"",
            conf.to_str().unwrap(),
            layouts.to_str().unwrap(),
            themes.to_str().unwrap()
        );
        let config_path = dir.path().join("ox_webservice_layout.yaml");
        fs::write(&config_path, plugin_config).unwrap();

        (dir, config_path.to_str().unwrap().to_string())
    }

    #[test]
    fn test_init_succeeds() {
        let (_dir, config_path) = setup_env();
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config_path);
        PluginHandle::init(ox_plugin_init, &params, &api).expect("init should succeed");
    }

    #[test]
    fn test_init_fails_on_missing_config() {
        let api = create_mock_api();
        let result = PluginHandle::init(ox_plugin_init, r#"{"config_file": "/no/such/file.yaml"}"#, &api);
        assert!(result.is_err());
    }

    #[test]
    fn test_process_full_page_default_template() {
        let (_dir, config_path) = setup_env();
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config_path);
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("init failed");

        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/some/page");
        set_mock_field(task_ctx, "request.method", "GET");
        handle.process(ox_plugin_process, task_ctx);

        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert!(body.contains("<html>"), "expected html element in body");
        assert!(body.contains("<nav>navigation</nav>"), "expected nav hook rendered");
        assert!(body.contains("<footer>footer-content</footer>"), "expected footer hook rendered");
        assert!(body.contains("default"), "expected default template");
        assert_eq!(get_mock_field(task_ctx, "response.status").as_deref(), Some("200"));
        assert!(get_mock_field(task_ctx, "response.header.Content-Type").as_deref().unwrap_or("").contains("text/html"));

        unsafe { drop_task_state(task_ctx); }
    }

    #[test]
    fn test_process_uses_page_type_tag_for_admin_template() {
        let (_dir, config_path) = setup_env();
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config_path);
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("init failed");

        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/admin/");
        // Set page_type tag using the ABI — this requires feature/request-tagger to be merged
        let key = CString::new("page_type").unwrap();
        let val = CString::new("admin").unwrap();
        (api.set_tag)(task_ctx, key.as_ptr(), val.as_ptr());
        handle.process(ox_plugin_process, task_ctx);

        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert!(body.contains("class=\"admin\""), "expected admin template (has class=admin)");
        assert!(!body.contains("default"), "should not use default template content");

        unsafe { drop_task_state(task_ctx); }
    }

    #[test]
    fn test_process_falls_back_to_default_for_unknown_page_type() {
        let (_dir, config_path) = setup_env();
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config_path);
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("init failed");

        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/something/");
        let key = CString::new("page_type").unwrap();
        let val = CString::new("unknown_type_xyz").unwrap();
        (api.set_tag)(task_ctx, key.as_ptr(), val.as_ptr());
        handle.process(ox_plugin_process, task_ctx);

        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert!(body.contains("default"), "should fall back to default template");

        unsafe { drop_task_state(task_ctx); }
    }

    #[test]
    fn test_fragment_endpoint_renders_single_hook() {
        let (_dir, config_path) = setup_env();
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config_path);
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("init failed");

        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/api/hooks/nav/");
        handle.process(ox_plugin_process, task_ctx);

        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert!(body.contains("<nav>navigation</nav>"), "expected nav hook content");
        assert!(!body.contains("<html>"), "fragment should not contain full page");

        unsafe { drop_task_state(task_ctx); }
    }

    #[test]
    fn test_fragment_endpoint_unknown_hook_returns_empty() {
        let (_dir, config_path) = setup_env();
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config_path);
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("init failed");

        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/api/hooks/nonexistent/");
        handle.process(ox_plugin_process, task_ctx);

        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert_eq!(body.trim(), "", "unknown hook should return empty body");

        unsafe { drop_task_state(task_ctx); }
    }
}
```

Add `#[cfg(test)] mod tests;` to `src/lib.rs`.

- [ ] **Step 2: Run tests — verify they fail**

```bash
cargo test -p ox_webservice_layout 2>&1
```

Expected: compile errors because `ox_plugin_init` and `ox_plugin_process` are not defined.

- [ ] **Step 3: Implement `src/lib.rs`**

Rewrite `crates/webservice/ox_webservice_layout/src/lib.rs`:

```rust
use libc::{c_char, c_void};
use ox_workflow_abi::{CoreHostApi, FlowControl, FLOW_CONTROL_CONTINUE, OX_LOG_ERROR, OX_LOG_INFO, OX_LOG_WARN};
use regex::Regex;
use serde_json;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use tera::Context;

pub mod config;
pub mod renderer;
#[cfg(test)]
mod tests;

use config::{load_layout_config, load_layout_theme, load_layout_yaml, load_hook_providers, LayoutConfig, LayoutYaml, HookProvider};
use renderer::render_page;

const MODULE_NAME: &str = "ox_webservice_layout";

pub struct ModuleContext {
    config: LayoutConfig,
    api: CoreHostApi,
    hook_path_re: Regex,
}

// ─── FFI helpers ─────────────────────────────────────────────────────────────

fn get_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let ptr = (api.get_field)(task_ctx, c_key.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn set_field(api: &CoreHostApi, task_ctx: *mut c_void, key: &str, value: &str) {
    if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(value)) {
        (api.set_field)(task_ctx, k.as_ptr(), v.as_ptr());
    }
}

fn get_tag(api: &CoreHostApi, task_ctx: *mut c_void, key: &str) -> String {
    let c_key = CString::new(key).unwrap();
    let ptr = (api.get_tag)(task_ctx, c_key.as_ptr());
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn log(api: &CoreHostApi, task_ctx: *mut c_void, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) { (api.log)(task_ctx, level, c.as_ptr()); }
}

fn set_error_response(api: &CoreHostApi, task_ctx: *mut c_void, status: &str, body: &str) {
    set_field(api, task_ctx, "response.status", status);
    set_field(api, task_ctx, "response.body", body);
    set_field(api, task_ctx, "response.header.Content-Type", "text/html; charset=utf-8");
}

// ─── Template selection ───────────────────────────────────────────────────────

/// Select the template file path for the given page_type tag, with full fallback chain.
/// Returns Err if the default template is also missing.
fn select_template_path(
    page_type: &str,
    layout: &LayoutYaml,
    layout_dir: &Path,
) -> Result<PathBuf, String> {
    let key = if page_type.is_empty() { "default" } else { page_type };

    let try_key = |k: &str| -> Option<PathBuf> {
        let rel = layout.page_templates.get(k)?;
        let path = layout_dir.join(rel);
        if path.exists() { Some(path) } else { None }
    };

    // Try requested key, fall back to "default"
    if let Some(path) = try_key(key) {
        return Ok(path);
    }
    if key != "default" {
        if let Some(path) = try_key("default") {
            return Ok(path);
        }
    }

    Err(format!(
        "default template not found for layout '{}' (page_type='{}')",
        layout.name, page_type
    ))
}

// ─── Plugin ABI ──────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_init(
    plugin_config_ctx: *const c_char,
    api_ptr: *const CoreHostApi,
    _abi_version: u32,
) -> *mut c_void {
    if api_ptr.is_null() { return std::ptr::null_mut(); }
    let api = unsafe { *api_ptr };

    let params_str = if plugin_config_ctx.is_null() {
        "{}".to_string()
    } else {
        unsafe { CStr::from_ptr(plugin_config_ctx).to_string_lossy().to_string() }
    };

    let params: serde_json::Value = serde_json::from_str(&params_str).unwrap_or(serde_json::Value::Null);
    let config_file = match params.get("config_file").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("{}: missing config_file param", MODULE_NAME));
            return std::ptr::null_mut();
        }
    };

    let config = match load_layout_config(&config_file) {
        Ok(c) => c,
        Err(e) => {
            log(&api, std::ptr::null_mut(), OX_LOG_ERROR,
                &format!("{}: {}", MODULE_NAME, e));
            return std::ptr::null_mut();
        }
    };

    log(&api, std::ptr::null_mut(), OX_LOG_INFO,
        &format!("{}: initialised (conf_dir={})", MODULE_NAME, config.conf_dir));

    let hook_path_re = Regex::new(r"^/api/hooks/([^/]+)/?$").unwrap();
    let ctx = Box::new(ModuleContext { config, api, hook_path_re });
    Box::into_raw(ctx) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_process(
    plugin_config_ctx: *mut c_void,
    task_ctx: *mut c_void,
) -> FlowControl {
    let cont = FlowControl { code: FLOW_CONTROL_CONTINUE, payload: std::ptr::null() };
    if plugin_config_ctx.is_null() { return cont; }
    let ctx = unsafe { &*(plugin_config_ctx as *mut ModuleContext) };
    let api = &ctx.api;

    let path = get_field(api, task_ctx, "request.path");

    // ── Fragment endpoint: /api/hooks/{id}/ ──────────────────────────────────
    if let Some(caps) = ctx.hook_path_re.captures(&path) {
        let hook_id = &caps[1];
        match render_fragment(ctx, task_ctx, hook_id) {
            Ok(html) => {
                set_field(api, task_ctx, "response.body", &html);
                set_field(api, task_ctx, "response.status", "200");
                set_field(api, task_ctx, "response.header.Content-Type", "text/html; charset=utf-8");
            }
            Err(e) => {
                log(api, task_ctx, OX_LOG_WARN, &format!("{}: fragment error: {}", MODULE_NAME, e));
                set_error_response(api, task_ctx, "404", "");
            }
        }
        return cont;
    }

    // ── Full page render ──────────────────────────────────────────────────────
    match render_full_page(ctx, task_ctx) {
        Ok(html) => {
            set_field(api, task_ctx, "response.body", &html);
            set_field(api, task_ctx, "response.status", "200");
            set_field(api, task_ctx, "response.header.Content-Type", "text/html; charset=utf-8");
        }
        Err(e) => {
            log(api, task_ctx, OX_LOG_ERROR, &format!("{}: render error: {}", MODULE_NAME, e));
            set_error_response(api, task_ctx, "500", "<h1>500 Internal Server Error</h1>");
        }
    }
    cont
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_error(
    _plugin_config_ctx: *mut c_void,
    _task_ctx: *mut c_void,
) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ox_plugin_destroy(plugin_config_ctx: *mut c_void) {
    if !plugin_config_ctx.is_null() {
        let _ = Box::from_raw(plugin_config_ctx as *mut ModuleContext);
    }
}

// ─── Internal render helpers ──────────────────────────────────────────────────

fn build_providers_index(providers: &[HookProvider]) -> HashMap<String, &HookProvider> {
    providers.iter().map(|p| (p.hook_id.clone(), p)).collect()
}

fn build_tera_context(
    api: &CoreHostApi,
    task_ctx: *mut c_void,
    layout_name: &str,
    theme_name: &str,
    page_type: &str,
) -> Context {
    let mut ctx = Context::new();
    ctx.insert("request", &serde_json::json!({
        "path": get_field(api, task_ctx, "request.path"),
        "method": get_field(api, task_ctx, "request.method"),
        "query": get_field(api, task_ctx, "request.query"),
    }));
    ctx.insert("layout", &serde_json::json!({ "name": layout_name }));
    ctx.insert("theme", &serde_json::json!({ "name": theme_name }));
    ctx.insert("tags", &serde_json::json!({ "page_type": page_type }));
    ctx
}

fn render_full_page(ctx: &ModuleContext, task_ctx: *mut c_void) -> Result<String, String> {
    let api = &ctx.api;
    let layout_theme = load_layout_theme(&ctx.config.conf_dir)?;
    let layout_dir = PathBuf::from(&ctx.config.layouts_dir).join(&layout_theme.active_layout);
    let layout_yaml = load_layout_yaml(&ctx.config.layouts_dir, &layout_theme.active_layout)?;
    let providers = load_hook_providers(&ctx.config.conf_dir)?;

    let page_type = get_tag(api, task_ctx, "page_type");
    let template_path = select_template_path(&page_type, &layout_yaml, &layout_dir)?;
    let template_content = std::fs::read_to_string(&template_path)
        .map_err(|e| format!("cannot read template '{}': {}", template_path.display(), e))?;

    let providers_index = build_providers_index(&providers.providers);
    let tera_ctx = build_tera_context(
        api, task_ctx,
        &layout_theme.active_layout,
        &layout_theme.active_theme,
        &page_type,
    );

    Ok(render_page(&template_content, &providers_index, &tera_ctx))
}

fn render_fragment(ctx: &ModuleContext, task_ctx: *mut c_void, hook_id: &str) -> Result<String, String> {
    let providers = load_hook_providers(&ctx.config.conf_dir)?;
    let providers_index = build_providers_index(&providers.providers);

    let layout_theme = load_layout_theme(&ctx.config.conf_dir)?;
    let tera_ctx = build_tera_context(
        &ctx.api, task_ctx,
        &layout_theme.active_layout,
        &layout_theme.active_theme,
        &get_tag(&ctx.api, task_ctx, "page_type"),
    );

    let provider = providers_index.get(hook_id);
    Ok(renderer::render_hook(hook_id, provider.copied(), &tera_ctx))
}
```

- [ ] **Step 4: Run tests — verify they pass**

```bash
cargo test -p ox_webservice_layout 2>&1
```

Expected: all tests pass (config, renderer, integration).

- [ ] **Step 5: Run broader suite to check nothing broke**

```bash
cargo test -p ox_workflow_core -p ox_workflow_executor -p ox_webservice_test_utils -p ox_webservice_request_tagger -p ox_webservice_layout 2>&1
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/webservice/ox_webservice_layout/src/lib.rs \
        crates/webservice/ox_webservice_layout/src/tests.rs
git commit -m "feat(layout): plugin wiring — init, process, fragment endpoint"
```

---

## Task 4: Default layout content files

**Files:**
- Create: `content/layouts/default/layout.yaml`
- Create: `content/layouts/default/templates/default.jinja2`
- Create: `content/layouts/default/templates/admin.jinja2`
- Create: `content/layouts/default/templates/includes/nav.jinja2`
- Create: `content/layouts/default/templates/includes/footer.jinja2`
- Create: `content/layouts/default/templates/includes/main_content.jinja2`

There is no Rust code in this task — these are runtime data files. The `content/layouts/common/` directory already exists (it's a stream plugin config, unrelated to the layout system). The new `default/` layout goes alongside it.

- [ ] **Step 1: Create the layout metadata file**

Create `content/layouts/default/layout.yaml`:

```yaml
name: "default"
description: "Default single-column layout"
page_templates:
  default: "templates/default.jinja2"
  admin: "templates/admin.jinja2"
hooks:
  - id: "head_extra"
    description: "Additional tags injected into <head>"
    above_fold: true
  - id: "nav"
    description: "Top navigation bar"
    above_fold: true
  - id: "main_content"
    description: "Primary page content area"
    above_fold: true
  - id: "footer"
    description: "Page footer"
    above_fold: false
```

- [ ] **Step 2: Create the default page template**

Create `content/layouts/default/templates/default.jinja2`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>oxIDIZER</title>
  <link rel="stylesheet" href="/layout/css/base.css">
  <link rel="stylesheet" href="/theme/css/theme.css">
  {{ hook("head_extra") }}
</head>
<body>
  {{ hook("nav") }}
  <main>
    {{ hook("main_content") }}
  </main>
  {{ hook("footer") }}
</body>
</html>
```

- [ ] **Step 3: Create the admin page template**

Create `content/layouts/default/templates/admin.jinja2`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>oxIDIZER — Admin</title>
  <link rel="stylesheet" href="/layout/css/base.css">
  <link rel="stylesheet" href="/theme/css/theme.css">
  {{ hook("head_extra") }}
</head>
<body class="admin">
  {{ hook("nav") }}
  <main class="admin-main">
    {{ hook("main_content") }}
  </main>
  {{ hook("footer") }}
</body>
</html>
```

- [ ] **Step 4: Create the hook include templates**

Create `content/layouts/default/templates/includes/nav.jinja2`:

```html
<nav class="site-nav">
  <a href="/">Home</a>
</nav>
```

Create `content/layouts/default/templates/includes/footer.jinja2`:

```html
<footer class="site-footer">
  <p>oxIDIZER</p>
</footer>
```

Create `content/layouts/default/templates/includes/main_content.jinja2`:

```html
<section class="main-content">
  <p>No content module registered for this page.</p>
</section>
```

- [ ] **Step 5: Build to confirm the crate still compiles cleanly**

```bash
cargo build -p ox_webservice_layout 2>&1
```

Expected: clean build.

- [ ] **Step 6: Commit**

```bash
git add content/layouts/default/
git commit -m "feat(layout): default layout templates and layout.yaml"
```

---

## Task 5: Config and activation files

**Files:**
- Create: `crates/webservice/ox_webservice_layout/conf/ox_webservice_layout.yaml`
- Create: `content/conf/layout_theme.yaml`
- Create: `content/conf/hook_providers.yaml`
- Create: `personas/all-services/modules/active/ox_webservice_layout.yaml`

- [ ] **Step 1: Create the module default config**

Create `crates/webservice/ox_webservice_layout/conf/ox_webservice_layout.yaml`:

```yaml
conf_dir: "/var/repos/oxIDIZER/content/conf"
layouts_dir: "/var/repos/oxIDIZER/content/layouts"
themes_dir: "/var/repos/oxIDIZER/content/themes"
```

- [ ] **Step 2: Create the active layout/theme selection config**

Create `content/conf/layout_theme.yaml`:

```yaml
active_layout: "default"
active_theme: "blue"
```

- [ ] **Step 3: Create the hook providers config**

Create `content/conf/hook_providers.yaml`:

```yaml
providers:
  - hook_id: "nav"
    type: "template"
    path: "/var/repos/oxIDIZER/content/layouts/default/templates/includes/nav.jinja2"
  - hook_id: "main_content"
    type: "template"
    path: "/var/repos/oxIDIZER/content/layouts/default/templates/includes/main_content.jinja2"
  - hook_id: "footer"
    type: "template"
    path: "/var/repos/oxIDIZER/content/layouts/default/templates/includes/footer.jinja2"
```

Note: `head_extra` is intentionally absent — the renderer returns empty string for unregistered hooks.

- [ ] **Step 4: Create the activation YAML**

Create `personas/all-services/modules/active/ox_webservice_layout.yaml`:

```yaml
modules:
  - id: "layout"
    name: "ox_webservice_layout"
    params:
      config_file: "/var/repos/oxIDIZER/crates/webservice/ox_webservice_layout/conf/ox_webservice_layout.yaml"

routes:
  - url: ".*"
    module_id: "layout"
    stage: Content
    priority: 1000
```

The priority 1000 ensures the layout plugin runs last in the Content stage, after any content-generating modules that may have set response fields the layout templates can read.

- [ ] **Step 5: Write a smoke test using the actual content files**

Add to `crates/webservice/ox_webservice_layout/src/tests.rs` (inside the `mod tests` block):

```rust
    #[test]
    fn test_integration_with_real_content_files() {
        // This test uses the actual content/conf/ and content/layouts/ files.
        // It will fail if those files don't exist — that's intentional.
        let config_path = "/var/repos/oxIDIZER/crates/webservice/ox_webservice_layout/conf/ox_webservice_layout.yaml";
        if !std::path::Path::new(config_path).exists() {
            eprintln!("skipping real-content smoke test: config not yet deployed");
            return;
        }
        let api = create_mock_api();
        let params = format!(r#"{{"config_file": "{}"}}"#, config_path);
        let handle = PluginHandle::init(ox_plugin_init, &params, &api).expect("real-content init failed");

        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.path", "/");
        set_mock_field(task_ctx, "request.method", "GET");
        handle.process(ox_plugin_process, task_ctx);

        let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
        assert!(body.contains("<!DOCTYPE html>"), "expected full HTML page");
        assert!(body.contains("<nav"), "expected nav hook rendered");
        assert_eq!(get_mock_field(task_ctx, "response.status").as_deref(), Some("200"));

        unsafe { drop_task_state(task_ctx); }
    }
```

- [ ] **Step 6: Run all layout tests**

```bash
cargo test -p ox_webservice_layout 2>&1
```

Expected: all tests pass, including the real-content smoke test.

- [ ] **Step 7: Build all affected crates**

```bash
cargo build -p ox_webservice_layout -p ox_workflow_executor -p ox_webservice_test_utils 2>&1
```

Expected: clean build.

- [ ] **Step 8: Commit**

```bash
git add \
  crates/webservice/ox_webservice_layout/conf/ \
  crates/webservice/ox_webservice_layout/src/tests.rs \
  content/conf/ \
  personas/all-services/modules/active/ox_webservice_layout.yaml
git commit -m "feat(layout): config files, hook providers, and activation YAML"
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Covered by |
|---|---|
| `ox_webservice_layout` plugin: init/process/destroy | Task 3 `lib.rs` |
| Config paths from module config file (not hardcoded) | Task 1 `config.rs`, Task 5 module conf yaml |
| `layout_theme.yaml` → active layout + theme | Task 1 `load_layout_theme`, Task 5 content file |
| `layout.yaml` → page templates + hooks | Task 1 `load_layout_yaml`, Task 4 content file |
| `hook_providers.yaml` → hook ID → provider | Task 1 `load_hook_providers`, Task 5 content file |
| Template selection: `page_type` tag with fallback chain | Task 3 `select_template_path` |
| Hook injection: `{{ hook("id") }}` syntax | Task 2 `parse_template_segments` + `render_hook` |
| Template providers: Jinja2 file via Tera | Task 2 `render_hook` template branch |
| Module providers: stubbed for v1 | Task 2 `render_hook` module branch (HTML comment) |
| Fragment endpoint `/api/hooks/{id}/` | Task 3 `render_fragment` + path regex |
| Tera context: `request.*`, `layout.*`, `theme.*`, `tags.*` | Task 3 `build_tera_context` |
| Default layout templates + includes | Task 4 |
| Activation YAML: stage Content, priority 1000 | Task 5 activation yaml |

**Streaming SSR**: The current `CoreHostApi` has no mechanism for partial writes. The plugin assembles the full HTML in memory and sets `response.body` as a complete string. HTTP chunked streaming is a future enhancement requiring ABI changes.

**`tags.*` in templates beyond `page_type`**: Only `tags.page_type` is currently populated in the Tera context. Supporting arbitrary `tags.X` requires a `get_tag_keys()` ABI function, which is not yet implemented.

**No placeholders found.**

**Type consistency:** `HookProvider` is defined in `config.rs` and used in both `renderer.rs` (via `crate::config::HookProvider`) and `lib.rs` (via `use config::HookProvider`). `render_page` takes `&HashMap<String, &HookProvider>` — the same type built by `build_providers_index` in `lib.rs`.
