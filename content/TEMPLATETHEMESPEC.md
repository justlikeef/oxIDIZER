# Layout / Theme System Specification

## Overview

A WordPress/Joomla-style layout and theme system for oxIDIZER. Layouts define structural HTML skeletons with named hook insertion points. Themes provide visual CSS and asset overlays. One layout and one theme are active at a time, selected via an admin module.

The primary performance goal is **fast time to first paint**. This is achieved through HTTP chunked streaming: the server flushes response chunks in priority order so the browser begins rendering immediately as bytes arrive, without waiting for the full response to be assembled.

---

## Directory Structure

All directory paths are specified in each module's own config file and linked into the global config via an activation YAML in `modules/active/`, following the same pattern as `ox_webservice_stream` and `ox_webservice_template_jinja2`. No paths are hardcoded in the modules themselves.

Logical layout of the content tree (actual paths come from module config):

```
content/
  conf/                           # runtime config files — path set in ox_webservice_layout config
    layout_theme.yaml             # active layout and theme selection
    hook_providers.yaml           # hook ID → content provider mappings
  layouts/                        # path set in ox_webservice_layout config
    <name>/
      layout.yaml                 # metadata, page_templates, hook declarations
      templates/
        base.jinja2               # outer HTML shell
        default.jinja2            # default page template
        admin.jinja2              # admin page template
        includes/                 # partial templates for hook providers
      www/                        # layout-specific static assets
  themes/                         # path set in ox_webservice_layout config
    <name>/
      theme.yaml                  # metadata
      www/
        css/                      # theme CSS (variables, overrides)
        images/                   # theme-specific images and icons
```

### Module Config Files

Each new module carries a default config in its `conf/` directory:

**`crates/webservice/ox_webservice_layout/conf/ox_webservice_layout.yaml`**
```yaml
conf_dir:    "content/conf"
layouts_dir: "content/layouts"
themes_dir:  "content/themes"
```

**`crates/webservice/ox_webservice_request_tagger/conf/ox_webservice_request_tagger.yaml`**
```yaml
tags_file: "content/conf/request_tags.yaml"
```

`content/conf/request_tags.yaml` — URL pattern → tag mappings (path set in request tagger config above):
```yaml
routes:
  - pattern: ".*"
    tags:
      - page_type: "default"
```

### Activation YAMLs

Activation YAMLs in `modules/active/` link the module config files into the global config, exactly as existing modules do:

**`modules/active/ox_webservice_layout.yaml`**
```yaml
modules:
  - id: "layout"
    name: "ox_webservice_layout"
    params:
      config_file: "<crate_path>/conf/ox_webservice_layout.yaml"

routes:
  - url: ".*"
    module_id: "layout"
    stage: Content
    priority: 1000
```

**`modules/active/ox_webservice_request_tagger.yaml`**
```yaml
modules:
  - id: "request_tagger"
    name: "ox_webservice_request_tagger"
    params:
      config_file: "<crate_path>/conf/ox_webservice_request_tagger.yaml"

routes:
  - url: ".*"
    module_id: "request_tagger"
    stage: PreContent
    priority: 1
```

---

## Concepts

### Layout

The structural skeleton of a page. Defines the HTML chrome (header, navigation, footer, sidebar) and declares named hook positions where content modules inject their output.

Location: `content/layouts/<name>/`

### Theme

Visual-only layer. Provides CSS variables, colour schemes, and theme-specific assets. Contains no structural HTML.

Location: `content/themes/<name>/`

### Hook

A named insertion point declared in a layout template. The server resolves each hook's content synchronously during the streaming render pass. The rendered output is a plain HTML region — no wrapper div or data attributes are emitted unless the hook provider explicitly includes them.

Declaration in a template:
```jinja2
{{ hook("main_content") }}
```

Each hook has a registered **content provider** — either a Jinja2 template file or a Rust plugin — identified by the hook ID. The same provider codepath serves both the inline SSR render and any subsequent `/api/hooks/{id}/` fragment requests.

### layout.yaml

Declares the layout's metadata, page type → template mappings, and hook positions.

```yaml
name: "default"
description: "Default single-column layout"
page_templates:
  default:  "templates/default.jinja2"
  admin:    "templates/admin.jinja2"
  creator:  "templates/creator.jinja2"
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
  - id: "sidebar"
    description: "Right-hand sidebar"
    above_fold: false
  - id: "footer"
    description: "Page footer"
    above_fold: false
```

`above_fold: true` hooks are rendered and flushed before below-fold hooks begin, ensuring visible content reaches the browser first.

### Template Selection

The layout plugin selects a page template using the following logic at request time:

1. Call `get_tag("page_type")` on the task context
2. If the tag is absent or empty, use `"default"`
3. Look up the value in the active layout's `page_templates` map
4. If the key is not in the map, use `"default"`
5. Resolve the template path relative to the layout's `templates/` directory and check it exists on disk
6. If the file does not exist, use `"default"`
7. Render with the selected template

The `default` entry in `page_templates` is required. The layout plugin refuses to initialise if it is absent or its file does not exist.

### Active Configuration

A YAML file managed by the admin module that records the active layout and theme. Nothing else — template mappings belong to the layout definition, not the selection config.

```yaml
# content/conf/layout_theme.yaml
active_layout: "default"
active_theme: "blue"
```

---

## Request Tagging

### Overview

A dedicated plugin (`ox_webservice_request_tagger`) runs early in the pipeline. It matches the request path against an ordered list of URL patterns and copies the matched tags onto the task context. The layout plugin (and any other module) reads tags by key to influence its behaviour. The tagger knows nothing about what the tags mean.

### Matching Semantics

All patterns are evaluated in order. Every matching pattern applies its tags. For any given tag key, later matches overwrite earlier ones — **last set wins**. The final value of each key after all patterns are evaluated is what gets written to the context.

This allows broad defaults near the top of the list and specific overrides lower down:

```yaml
# content/conf/request_tags.yaml
routes:
  - pattern: ".*"
    tags:
      - page_type: "default"
      - auth_required: "false"
  - pattern: "^/admin/"
    tags:
      - page_type: "admin"
      - auth_required: "true"
  - pattern: "^/admin/public/"
    tags:
      - auth_required: "false"
```

In this example a request to `/admin/public/dashboard` matches all three patterns. The final tag state is `page_type: "admin"`, `auth_required: "false"`.

### Task Context — Tags Namespace

Tags are stored in a dedicated `tags` collection on the task context, separate from the HTTP request/response fields. This keeps tag keys from colliding with field names like `request.path` or `response.status`.

The `CoreHostApi` ABI is extended with two new functions:

```c
// Set a tag on the task context (overwrites if key already exists)
void (*set_tag)(void *task_ctx, const char *key, const char *value);

// Get a tag value from the task context (returns NULL if key not present)
const char *(*get_tag)(void *task_ctx, const char *key);
```

The tagger plugin calls `set_tag` for each key in each matched route's tags block. Consuming modules call `get_tag("page_type")`, `get_tag("auth_required")`, etc.

### Page Type → Template Mapping

The layout plugin maps the `page_type` tag to a template file using the `page_templates` map in the active layout's `layout.yaml`. See **Template Selection** for the full fallback logic — absent tag, missing map entry, and missing file all fall back to `default`.

### Tags in Jinja2 Template Context

All matched tags are available inside hook templates under the `tags` namespace:

```jinja2
{% if tags.auth_required == "true" %}
  ...
{% endif %}
```

### Relationship to Router

The request tagger and `ox_webservice_router` are independent. The router matches routes to modules and manages request dispatch. The tagger matches routes to tag sets and annotates the context. Neither is aware of the other. Both use URL pattern matching but serve different purposes and maintain separate config files.

---

## Rendering Pipeline

The layout plugin (`ox_webservice_layout`) handles all page assembly. It replaces the current `ox_webservice_template_jinja2` for full-page requests.

### Streaming SSR Render Sequence

1. **Receive request** — layout plugin intercepts the request before content modules run
2. **Load active config** — read `layout_theme.yaml` from the configured `conf_dir` to determine active layout and theme
3. **Split template at hook boundaries** — the base template is pre-parsed at init time into ordered segments: static HTML segments interleaved with hook IDs
4. **Begin streaming response** — set `Transfer-Encoding: chunked`, `Content-Type: text/html`
5. **Flush segments in order**:
   - Static segment (e.g. `<head>` with theme CSS link) → flush immediately
   - For each hook in document order:
     - Dispatch to the registered content provider for that hook ID
     - Render the provider's output (Jinja2 or plugin)
     - Flush the rendered chunk
   - Remaining static segments → flush
6. **Close response**

Above-fold hooks are encountered first in document order by template design, so the browser receives and paints visible content before below-fold hooks are resolved.

### Asset Layering

The `<head>` segment includes both layout and theme assets in order:

```html
<link rel="stylesheet" href="/layout/css/base.css">   <!-- structural -->
<link rel="stylesheet" href="/theme/css/theme.css">   <!-- visual overrides -->
```

Static assets are served by the existing `ox_webservice_stream` instances, one rooted at the layout's `www/` and one at the theme's `www/`. URL prefixes `/layout/` and `/theme/` route to the respective roots.

### Fragment Endpoint

Each hook is also accessible as a standalone fragment via `/api/hooks/{id}/`. The response is the rendered hook content only, with no surrounding layout HTML. This endpoint uses the same provider dispatch as the inline SSR render.

Used for: live-refresh of dynamic hooks (e.g. status widgets) after initial page load. Not used during initial page assembly.

---

## JavaScript Role

JS is **not required** for initial page render. All content is present in the streamed SSR response.

JS may be used after page load for hooks that contain live data (periodic refresh, user-triggered updates). A cookie (`js=1`) set by an inline script in `<head>` signals JS capability to the server on subsequent requests, enabling future optimisations if needed. This cookie is advisory only and does not change the initial render strategy.

---

## Admin Module

A content admin module (`ox_cc_layout_admin` or similar) provides:

- List available layouts and themes
- Select and persist the active layout + theme pair to `layout_theme.yaml` in the configured `conf_dir`
- Preview a layout/theme combination
- Register or deregister hook content providers

The admin module reads the `layout.yaml` from each layout to enumerate available hooks and presents them for provider assignment.

---

## Hook Content Provider Registration

Providers are registered in a YAML config file:

```yaml
# content/conf/hook_providers.yaml
providers:
  - hook_id: "nav"
    type: "template"
    path: "content/layouts/default/templates/includes/nav.jinja2"
  - hook_id: "main_content"
    type: "module"
    module_id: "ox_webservice_wsgi"
  - hook_id: "footer"
    type: "template"
    path: "content/layouts/default/templates/includes/footer.jinja2"
```

Provider types:
- `template` — renders a Jinja2 file via Tera; receives the request context as template variables
- `module` — dispatches to a loaded plugin via the existing `CoreHostApi` module dispatch mechanism

---

## Jinja2 Template Context

All hook templates receive a standard context object:

```
request.path
request.method
request.query
layout.name
theme.name
server.status
```

Additional context keys can be injected by the content provider plugin before the template is rendered.

---

## Relationship to Existing Modules

| Module | Role in New System |
|---|---|
| `ox_webservice_stream` | Continues to serve layout and theme static assets |
| `ox_webservice_template_jinja2` | Used by hook providers for fragment rendering |
| `ox_webservice_errorhandler_jinja2` | Unchanged; error pages remain separate from layout system |
| `ox_webservice_layout` (new) | Full-page assembly, streaming, hook dispatch, page type → template selection |
| `ox_webservice_request_tagger` (new) | URL pattern matching, copies matched tags onto task context via `set_tag` |

---

## Open Questions

- Caching strategy for rendered hook fragments (ETag, Cache-Control per hook)
- Hot-reload of `layout_theme.yaml` without server restart
- Hook provider fallback if a registered provider is unavailable
- Tera template pre-compilation and segment splitting implementation detail
