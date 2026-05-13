# Security Persona Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a composite security persona YAML that loads all security plugins together (`ox_security_pipeline` + `ox_security_idp`) and serves a minimal static admin UI via `ox_webservice_stream`.

**Architecture:** A `personas/security/` directory houses the composite persona definition. `ox_security_pipeline` handles auth pipeline + authz/accounting admin routes. `ox_security_idp` handles OAuth2/OIDC/SAML protocol + IdP admin routes. `ox_webservice_stream` serves the static admin HTML from `crates/security/ox_security_pipeline/content/www/`. All security plugins are given module IDs so personas can selectively include only the modules they need.

**Tech Stack:** YAML persona files, `ox_webservice_stream`, HTML/CSS (no JS framework), existing `ox_security_pipeline` and `ox_security_idp` cdylib plugins.

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `personas/security/security.yaml` | Composite persona activating all security modules |
| Create | `personas/security/modules/available/ox_security_stream.yaml` | Stream module for security admin static content |
| Create | `crates/security/ox_security_pipeline/content/www/index.html` | Admin UI landing page |
| Create | `crates/security/ox_security_pipeline/content/www/admin/index.html` | Admin dashboard |
| Create | `crates/security/ox_security_pipeline/conf/stream.yaml` | ox_webservice_stream config |

---

### Task 1: Composite persona YAML

**Files:**
- Create: `personas/security/security.yaml`

- [ ] **Step 1: Check existing persona structure for reference**

```bash
ls personas/ca/
cat personas/ca/ca.yaml 2>/dev/null || ls personas/ca/*.yaml | head -5
```

Use this structure as a reference for what fields belong in a persona YAML.

- [ ] **Step 2: Create personas/security/security.yaml**

After checking the structure of an existing persona, create `personas/security/security.yaml`:

```yaml
# Security persona — loads all security plugins
# Include this persona in a webservice to enable the security AAA pipeline and IdP.
persona: "security"
version: "0.1.0"

includes:
  - "modules/available/ox_security_pipeline.yaml"
  - "modules/available/ox_security_idp.yaml"
  - "modules/available/ox_security_stream.yaml"
```

> **Note:** If the project's persona format uses a different top-level key (check the ca persona), adapt accordingly. The `includes` key may be `modules` or `routes` at the top level — follow the existing pattern exactly.

- [ ] **Step 3: Verify persona YAML is valid**

```bash
python3 -c "import yaml; yaml.safe_load(open('personas/security/security.yaml'))" 2>&1
```
Expected: no output (valid YAML).

- [ ] **Step 4: Commit**

```bash
git add personas/security/security.yaml
git commit -m "feat(security-persona): add composite security persona YAML"
```

---

### Task 2: Stream config and static admin content

**Files:**
- Create: `crates/security/ox_security_pipeline/conf/stream.yaml`
- Create: `personas/security/modules/available/ox_security_stream.yaml`
- Create: `crates/security/ox_security_pipeline/content/www/index.html`
- Create: `crates/security/ox_security_pipeline/content/www/admin/index.html`

- [ ] **Step 1: Create ox_webservice_stream config**

Create `crates/security/ox_security_pipeline/conf/stream.yaml`:

```yaml
content_root: "${{OX_BASE}}/crates/security/ox_security_pipeline/content/www"
mimetypes_file: "${{OX_BASE}}/personas/common/mimetypes.yaml"
default_documents:
  - document: "index.html"
on_content_conflict: "skip"
```

- [ ] **Step 2: Create the stream module YAML**

Create `personas/security/modules/available/ox_security_stream.yaml`:

```yaml
modules:
  - id: "security_stream"
    name: "ox_webservice_stream"
    params:
      config_file: "${{OX_BASE}}/crates/security/ox_security_pipeline/conf/stream.yaml"

routes:
  - url: "^/security/(.*)$"
    module_id: "security_stream"
    priority: 50
    phase: Content
    path_capture: true
```

- [ ] **Step 3: Verify mimetypes.yaml exists**

```bash
ls personas/common/mimetypes.yaml
```
Expected: file exists. If not, copy from the cert admin stream config path.

- [ ] **Step 4: Create content directory structure**

```bash
mkdir -p crates/security/ox_security_pipeline/content/www/admin
```

- [ ] **Step 5: Create landing page**

Create `crates/security/ox_security_pipeline/content/www/index.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>ox_security</title>
  <style>
    body { font-family: sans-serif; max-width: 600px; margin: 4rem auto; }
    a { color: #2563eb; }
  </style>
</head>
<body>
  <h1>ox_security</h1>
  <p>Security pipeline and identity provider.</p>
  <ul>
    <li><a href="/security/admin/">Admin console</a></li>
    <li><a href="/api/v1/security/health">Health check</a></li>
    <li><a href="/oidc/.well-known/openid-configuration">OIDC discovery</a></li>
  </ul>
</body>
</html>
```

- [ ] **Step 6: Create admin dashboard**

Create `crates/security/ox_security_pipeline/content/www/admin/index.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>Security Admin</title>
  <style>
    body { font-family: sans-serif; max-width: 800px; margin: 2rem auto; }
    section { border: 1px solid #ddd; border-radius: 4px; padding: 1rem; margin-bottom: 1rem; }
    h2 { margin-top: 0; }
    button { background: #2563eb; color: white; border: none; padding: .5rem 1rem; border-radius: 4px; cursor: pointer; }
    pre { background: #f5f5f5; padding: .5rem; border-radius: 4px; overflow-x: auto; }
  </style>
</head>
<body>
  <h1>Security Admin</h1>

  <section>
    <h2>Health</h2>
    <button onclick="fetch('/api/v1/security/health').then(r=>r.json()).then(d=>document.getElementById('health').textContent=JSON.stringify(d,null,2))">Check</button>
    <pre id="health"></pre>
  </section>

  <section>
    <h2>Accounting Events</h2>
    <button onclick="fetch('/api/v1/admin/accounting/events').then(r=>r.json()).then(d=>document.getElementById('events').textContent=JSON.stringify(d,null,2))">Load</button>
    <pre id="events"></pre>
  </section>

  <section>
    <h2>Authz Grants</h2>
    <button onclick="fetch('/api/v1/admin/authz/grants').then(r=>r.json()).then(d=>document.getElementById('grants').textContent=JSON.stringify(d,null,2))">Load</button>
    <pre id="grants"></pre>
  </section>

  <section>
    <h2>IdP Tokens</h2>
    <button onclick="fetch('/api/v1/admin/idp/tokens').then(r=>r.json()).then(d=>document.getElementById('tokens').textContent=JSON.stringify(d,null,2))">Load</button>
    <pre id="tokens"></pre>
  </section>
</body>
</html>
```

- [ ] **Step 7: Verify HTML files are valid**

```bash
ls -la crates/security/ox_security_pipeline/content/www/
ls -la crates/security/ox_security_pipeline/content/www/admin/
```
Expected: both index.html files present.

- [ ] **Step 8: Commit**

```bash
git add crates/security/ox_security_pipeline/conf/stream.yaml \
        crates/security/ox_security_pipeline/content/ \
        personas/security/modules/available/ox_security_stream.yaml
git commit -m "feat(security-persona): add ox_webservice_stream config and static admin UI"
```

---

### Task 3: Delete superseded plan files

**Files:**
- Delete: `docs/superpowers/plans/2026-05-11-security-14-idp-oauth2-oidc.md`
- Delete: `docs/superpowers/plans/2026-05-11-security-15-idp-saml.md`
- Delete: `docs/superpowers/plans/2026-05-11-security-16-admin-api.md`

- [ ] **Step 1: Remove the axum-era plan files**

These plans described an axum-based implementation that was discarded in favour of the FFI plugin architecture. The replacements are Plans 14–16 dated 2026-05-12.

```bash
rm docs/superpowers/plans/2026-05-11-security-14-idp-oauth2-oidc.md
rm docs/superpowers/plans/2026-05-11-security-15-idp-saml.md
rm docs/superpowers/plans/2026-05-11-security-16-admin-api.md
```

- [ ] **Step 2: Commit**

```bash
git add -A docs/superpowers/plans/
git commit -m "docs: replace axum-era Plans 14–16 with FFI plugin plans"
```

---

### Task 4: Final validation

- [ ] **Step 1: Build all security crates**

```bash
cargo build -p ox_security_pipeline -p ox_security_idp 2>&1 | tail -10
```
Expected: both build cleanly with no warnings about unused imports.

- [ ] **Step 2: Run all security plugin tests**

```bash
cargo test -p ox_security_pipeline -p ox_security_idp 2>&1 | tail -20
```
Expected: all tests pass.

- [ ] **Step 3: Verify cdylib artifacts exist**

```bash
find target/debug -name "libox_security_pipeline*.so" -o -name "libox_security_idp*.so" 2>/dev/null | head -5
```
Expected: `.so` files present (on Linux) confirming cdylib targets built.

- [ ] **Step 4: Commit (if any fixes were made)**

```bash
git status && git diff --stat
```
Only commit if there are outstanding changes from fixing build issues.
