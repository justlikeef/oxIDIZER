# Vary Header Module (`ox_webservice_vary_header`)

## Overview

The `ox_webservice_vary_header` module addresses specific browser caching behaviors associated with Single Page Applications (SPAs) and dynamic content negotiation. It ensures that browsers respect the `Accept` header when caching responses, preventing situations where a JSON API response is cached and subsequently displayed when the user navigates back to a page expecting HTML.

## Purpose

When a URL (e.g., `/drivers/`) serves both HTML (for the web page) and JSON (for the API call) based on the `Accept` header, browsers may aggressively cache the JSON response. If a user navigates away and then uses the "Back" button, the browser might serve the cached JSON instead of re-requesting the HTML page.

This module inspects outgoing responses during the `LateRequest` phase. If the response is JSON (`application/json`), it appends the `Vary: Accept` header. This instructs the browser (and any intermediate proxies) that the response content varies based on the request's `Accept` header, enforcing separate cache entries for HTML and JSON versions of the same resource.

## How It Works

1.  **Phase**: Runs in the `LateRequest` phase, after content generation but before the response is finalized.
2.  **Detection**: Inspects the `Content-Type` header of the response.
3.  **Action**: If the content type is `application/json` (or `text/json`) and the `Vary: Accept` header is missing, it adds `Vary: Accept` to the response headers.

## Configuration

This module requires no specific parameters but must be enabled in the pipeline configuration.

**File:** `conf/modules/active/ox_webservice_vary_header.yaml`

```yaml
modules:
  - id: "vary_header"
    name: "ox_webservice_vary_header"
    phase: LateRequest
    params: {}

routes:
  - url: ".*"
    match_type: "regex"
    module_id: "vary_header"
```
