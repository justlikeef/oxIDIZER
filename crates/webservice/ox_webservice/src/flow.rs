use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::net::SocketAddr;
use axum::http::Request;
use axum::body::{Body, to_bytes};
use axum::response::Response;
use ox_workflow_core::Task;
use ox_workflow_core::{FlowDef, PluginDef};
use ox_workflow_executor::{FlowManager, create_host_api, FlowRunner};
use ox_workflow_executor::plugin_registry::PluginInstance;
use ox_workflow_abi::{CoreHostApi, FLOW_CONTROL_STREAM_FILE};
use crate::ServerConfig;
use tokio::sync::RwLock;

pub struct Flow {
    pub main_config_json: String,
    pub flow_manager: Arc<RwLock<FlowManager>>,
    pub main_flow: Arc<FlowRunner>,
    pub api: CoreHostApi,
    /// Limits the number of plugin pipelines executing concurrently.
    /// Excess requests wait as cheap async futures rather than spawning threads.
    pub plugin_semaphore: Arc<tokio::sync::Semaphore>,
    /// Holds initialized contexts for phase=Init modules (startup-only plugins).
    /// Kept alive until the Flow is dropped so ox_plugin_destroy is called on shutdown.
    #[allow(dead_code)]
    init_plugins: Vec<PluginInstance>,
}

/// Normalized route entry used during flow construction.
struct EffectiveRoute {
    stage: String,
    module_id: String,
    priority: u16,
    path: String,
    method: Option<String>,
    headers: Option<HashMap<String, String>>,
    query: Option<HashMap<String, String>>,
    protocol: Option<String>,
    hostname: Option<String>,
    status_code: Option<String>,
}

impl Flow {
    pub fn new(config: &ServerConfig, config_json: String) -> Result<Self, String> {
        let mut manager = FlowManager::new();
        let api = create_host_api();

        let mut plugin_paths = HashMap::new();

        // ------------------------------------------------------------------
        // Collect all routes from both sources:
        //   1. Top-level config.routes  (UrlRoute, has module_id field)
        //   2. Module-embedded module.routes (UriMatcher, module_id = module id/name)
        // Resolve stage: route.stage → module.stage → "Content"
        // ------------------------------------------------------------------
        let mut all_routes: Vec<EffectiveRoute> = Vec::new();

        // Build a quick lookup: module_id → module stage
        // Check m.stage first, then fall back to extra_params["phase"] (used by YAML modules that
        // declare their stage as `phase: Content` instead of `stage: Content`).
        let module_stage_map: HashMap<String, Option<String>> = config.modules.iter().map(|m| {
            let id = m.id.clone().unwrap_or_else(|| m.name.clone());
            let stage = m.stage.clone().or_else(|| {
                m.extra_params.get("phase")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.eq_ignore_ascii_case("init"))
                    .map(|s| s.to_string())
            });
            (id, stage)
        }).collect();

        // 1. Top-level routes
        for r in &config.routes {
            let module_id = r.module_id.clone().unwrap_or_default();
            if module_id.is_empty() { continue; }

            let route_stage = r.stage.as_deref();
            let module_stage = module_stage_map.get(&module_id).and_then(|p| p.as_deref());
            let stage = route_stage.or(module_stage).unwrap_or("Content").to_string();

            all_routes.push(EffectiveRoute {
                stage,
                module_id,
                priority: r.priority,
                path: r.url.clone().unwrap_or_default(),
                method: r.method.clone(),
                headers: r.headers.clone(),
                query: r.query.clone(),
                protocol: r.protocol.clone(),
                hostname: r.hostname.clone(),
                status_code: r.status_code.clone(),
            });
        }

        // 2. Module-embedded routes (UriMatcher format)
        for mod_cfg in &config.modules {
            if let Some(mod_routes) = &mod_cfg.routes {
                let module_id = mod_cfg.id.as_deref().unwrap_or(&mod_cfg.name).to_string();
                let module_stage = mod_cfg.stage.as_deref().or_else(|| {
                    mod_cfg.extra_params.get("phase")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.eq_ignore_ascii_case("init"))
                });

                for r in mod_routes {
                    let route_stage = r.stage.as_deref();
                    let stage = route_stage.or(module_stage).unwrap_or("Content").to_string();

                    all_routes.push(EffectiveRoute {
                        stage,
                        module_id: module_id.clone(),
                        priority: r.priority,
                        path: r.path.clone(),
                        method: r.method.clone(),
                        headers: r.headers.clone(),
                        query: r.query.clone(),
                        protocol: r.protocol.clone(),
                        hostname: r.hostname.clone(),
                        status_code: r.status_code.clone(),
                    });
                }
            }
        }

        // ------------------------------------------------------------------
        // Group routes by stage and sort each group by priority (ascending).
        // Lower priority number = higher precedence = checked first by router.
        // ------------------------------------------------------------------
        let mut routes_by_stage: HashMap<String, Vec<&EffectiveRoute>> = HashMap::new();
        for r in &all_routes {
            routes_by_stage.entry(r.stage.clone()).or_default().push(r);
        }
        for routes in routes_by_stage.values_mut() {
            routes.sort_by_key(|r| r.priority);
        }

        // Resolve the plugin directory: OX_PLUGIN_DIR overrides the default dev path.
        // Set OX_PLUGIN_DIR=/usr/lib/ox_webservice for installed deployments.
        let plugin_dir = std::env::var("OX_PLUGIN_DIR")
            .unwrap_or_else(|_| "target/debug".to_string());

        // Resolve the router .so path once (OX_ROUTER_PATH still overrides if set)
        let router_path = std::env::var("OX_ROUTER_PATH")
            .unwrap_or_else(|_| format!("{}/libox_webservice_router.so", plugin_dir));

        // ------------------------------------------------------------------
        // Build pipeline stages (from workflow config).
        // For each stage, inject a per-stage router config and embed the
        // module plugins that belong to this stage, in priority order.
        // ------------------------------------------------------------------
        let workflow_stages = config.workflow.as_ref()
            .map(|w| w.stages.clone())
            .unwrap_or_default();

        let mut final_stages: Vec<String> = Vec::new();
        // Collect router configs per stage so we can build the post-injection config JSON later.
        let mut stage_router_configs: HashMap<String, serde_json::Value> = HashMap::new();
        // Track which (stage, plugin_id) pairs belong to the status module so we can
        // inject the built config JSON after all router configs are known.
        let mut status_plugin_locations: Vec<(String, String)> = Vec::new();

        for mut stage in workflow_stages {
            let stage_name = stage.name.clone();
            let stage_routes = routes_by_stage.get(&stage_name).cloned().unwrap_or_default();

            // Build router config for this stage only
            let router_routes: Vec<serde_json::Value> = stage_routes.iter().map(|r| {
                let mut matcher = serde_json::Map::new();
                if !r.path.is_empty() {
                    matcher.insert("path".to_string(), serde_json::Value::String(r.path.clone()));
                }
                if let Some(p) = &r.protocol {
                    matcher.insert("protocol".to_string(), serde_json::Value::String(p.clone()));
                }
                if let Some(h) = &r.hostname {
                    matcher.insert("hostname".to_string(), serde_json::Value::String(h.clone()));
                }
                if let Some(sc) = &r.status_code {
                    matcher.insert("status_code".to_string(), serde_json::Value::String(sc.clone()));
                }
                if let Some(m) = &r.method {
                    matcher.insert("method".to_string(), serde_json::Value::String(m.clone()));
                }
                if let Some(hdrs) = &r.headers {
                    let hm: serde_json::Map<_, _> = hdrs.iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect();
                    if !hm.is_empty() {
                        matcher.insert("headers".to_string(), serde_json::Value::Object(hm));
                    }
                }
                if let Some(qry) = &r.query {
                    let qm: serde_json::Map<_, _> = qry.iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect();
                    if !qm.is_empty() {
                        matcher.insert("query".to_string(), serde_json::Value::Object(qm));
                    }
                }
                serde_json::json!({
                    "matcher": matcher,
                    "module_id": r.module_id,
                    "priority": r.priority
                })
            }).collect();

            let router_cfg = serde_json::json!({ "routes": router_routes });
            stage_router_configs.insert(stage_name.clone(), router_cfg.clone());

            // Inject router config into any ox_webservice_router plugin in this stage
            for plugin in &mut stage.plugins {
                if plugin.name == "ox_webservice_router" {
                    plugin_paths.entry("ox_webservice_router".to_string())
                        .or_insert_with(|| router_path.clone());
                    plugin.config = Some(router_cfg.clone());
                }
            }

            // Add module plugins for this stage (deduplicated, in priority order)
            let mut seen_modules: HashSet<String> = HashSet::new();
            for route in &stage_routes {
                let module_id = &route.module_id;
                if !seen_modules.insert(module_id.clone()) {
                    continue; // already added
                }

                // Find the module config
                let mod_cfg = config.modules.iter().find(|m| {
                    m.id.as_deref() == Some(module_id) || &m.name == module_id
                });

                let Some(mod_cfg) = mod_cfg else { continue };

                let path = mod_cfg.path.clone()
                    .unwrap_or_else(|| format!("{}/lib{}.so", plugin_dir, mod_cfg.name));
                plugin_paths.insert(module_id.clone(), path);

                // Build plugin config by merging extra_params and params sub-object
                let mut plugin_map = serde_json::Map::new();
                for (k, v) in &mod_cfg.extra_params {
                    // Skip non-config fields that shouldn't be passed to the plugin
                    if k == "stage" || k == "routes" { continue; }
                    plugin_map.insert(k.clone(), v.clone());
                }
                if let Some(params) = &mod_cfg.params {
                    if let Some(params_obj) = params.as_object() {
                        for (k, v) in params_obj {
                            plugin_map.insert(k.clone(), v.clone());
                        }
                    }
                }
                // For the status module, record its location so we can inject
                // the post-injection config JSON after all stages are built.
                if mod_cfg.name == "ox_webservice_status" {
                    status_plugin_locations.push((stage_name.clone(), module_id.clone()));
                }

                let mod_config_val = serde_json::Value::Object(plugin_map);

                stage.plugins.push(PluginDef {
                    name: module_id.clone(),
                    config: Some(mod_config_val),
                });
            }

            final_stages.push(stage.name.clone());
            manager.stage_defs.insert(stage.name.clone(), stage);
        }

        // Build a post-injection config JSON where router plugins have their actual configs.
        // The original config_json has config: null for all router plugins; we patch it here.
        let built_config_json: String = {
            let mut result = config_json.clone();
            if let Ok(mut cfg) = serde_json::from_str::<serde_json::Value>(&config_json) {
                if let Some(stages) = cfg.get_mut("workflow")
                    .and_then(|w| w.get_mut("stages"))
                    .and_then(|s| s.as_array_mut())
                {
                    for stage in stages.iter_mut() {
                        let stage_name = stage.get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(router_cfg) = stage_router_configs.get(&stage_name) {
                            if let Some(plugins) = stage.get_mut("plugins").and_then(|p| p.as_array_mut()) {
                                for plugin in plugins.iter_mut() {
                                    if plugin.get("name").and_then(|n| n.as_str()) == Some("ox_webservice_router") {
                                        if let Some(obj) = plugin.as_object_mut() {
                                            obj.insert("config".to_string(), router_cfg.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                result = serde_json::to_string(&cfg).unwrap_or(config_json.clone());
            }
            result
        };

        // Inject built_config_json into the status module plugin config in stage defs.
        for (stage_name, plugin_id) in &status_plugin_locations {
            if let Some(stage_def) = manager.stage_defs.get_mut(stage_name) {
                for plugin in &mut stage_def.plugins {
                    if plugin.name == *plugin_id {
                        if let Some(serde_json::Value::Object(ref mut obj)) = plugin.config {
                            obj.insert(
                                "_server_config_json".to_string(),
                                serde_json::Value::String(built_config_json.clone()),
                            );
                        }
                    }
                }
            }
        }

        let flow_def = FlowDef {
            name: "http_flow".to_string(),
            persistent: false,
            stages: final_stages,
        };

        let main_flow = unsafe {
            manager.build_flow(&flow_def, &api, &plugin_paths)
                .map_err(|e| format!("Failed to build flow: {:?}", e))?
        };

        // Run phase=Init modules: load and initialize them once at startup.
        // Their ox_plugin_process is a no-op; all work is done in ox_plugin_init.
        // Contexts are kept alive until Flow is dropped so ox_plugin_destroy fires.
        let mut init_plugins: Vec<PluginInstance> = Vec::new();
        for mod_cfg in &config.modules {
            let is_init = mod_cfg.extra_params.get("phase")
                .and_then(|v| v.as_str())
                .map(|s| s.eq_ignore_ascii_case("init"))
                .unwrap_or(false);
            if !is_init { continue; }

            let module_id = mod_cfg.id.as_deref().unwrap_or(&mod_cfg.name).to_string();
            let path = mod_cfg.path.clone()
                .unwrap_or_else(|| format!("{}/lib{}.so", plugin_dir, mod_cfg.name));

            let mut plugin_map = serde_json::Map::new();
            for (k, v) in &mod_cfg.extra_params {
                if k == "phase" || k == "routes" { continue; }
                plugin_map.insert(k.clone(), v.clone());
            }
            if let Some(params) = &mod_cfg.params {
                if let Some(params_obj) = params.as_object() {
                    for (k, v) in params_obj {
                        plugin_map.insert(k.clone(), v.clone());
                    }
                }
            }
            let init_config_json = serde_json::to_string(&serde_json::Value::Object(plugin_map))
                .unwrap_or_else(|_| "{}".to_string());

            unsafe {
                match manager.ensure_plugin_loaded(&module_id, &path) {
                    Ok(plugin) => match plugin.init(&init_config_json, &api) {
                        Ok(ctx) => {
                            init_plugins.push(PluginInstance { name: module_id.clone(), plugin, ctx });
                        }
                        Err(e) => {
                            return Err(format!("Init plugin '{}' failed: {:?}", module_id, e));
                        }
                    },
                    Err(e) => {
                        return Err(format!("Failed to load init plugin '{}': {:?}", module_id, e));
                    }
                }
            }
        }

        // Allow up to 4× CPU threads to run plugin pipelines concurrently.
        // Requests beyond this limit wait as async futures — no thread explosion.
        let concurrency = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8) * 4;

        Ok(Self {
            main_config_json: config_json,
            flow_manager: Arc::new(RwLock::new(manager)),
            main_flow,
            api,
            plugin_semaphore: Arc::new(tokio::sync::Semaphore::new(concurrency)),
            init_plugins,
        })
    }

    pub async fn execute_request(self: Arc<Self>, addr: SocketAddr, req: Request<Body>, protocol: String) -> Response {
        let mut task = Task::new(1);

        let (parts, body) = req.into_parts();

        {
            let mut w = task.state.write();
            w.fields.insert("request.protocol".to_string(), ox_workflow_core::state::FieldValue::String(protocol));
            w.fields.insert("request.method".to_string(), ox_workflow_core::state::FieldValue::String(parts.method.to_string()));
            w.fields.insert("request.path".to_string(), ox_workflow_core::state::FieldValue::String(parts.uri.path().to_string()));
            w.fields.insert("request.query".to_string(), ox_workflow_core::state::FieldValue::String(parts.uri.query().unwrap_or("").to_string()));
            w.fields.insert("request.source_ip".to_string(), ox_workflow_core::state::FieldValue::String(addr.ip().to_string()));

            for (k, v) in parts.headers.iter() {
                w.fields.insert(format!("request.header.{}", k.as_str()), ox_workflow_core::state::FieldValue::String(v.to_str().unwrap_or("").to_string()));
            }

            // HTTP/2 sends :authority pseudo-header instead of Host; fall back to URI authority.
            if !w.fields.contains_key("request.header.host") {
                if let Some(authority) = parts.uri.authority() {
                    w.fields.insert("request.header.host".to_string(), ox_workflow_core::state::FieldValue::String(authority.to_string()));
                }
            }

            // Defaults for response in case flow fails early
            w.fields.insert("response.status".to_string(), ox_workflow_core::state::FieldValue::String("500".to_string()));
            w.fields.insert("response.body".to_string(), ox_workflow_core::state::FieldValue::String("".to_string()));
        }

        // Read body bytes. Write to a temp file for binary-safe access (file uploads etc),
        // and also store as a lossy UTF-8 string for text-based modules.
        let body_tmp_path: Option<std::path::PathBuf> = if let Ok(bytes) = to_bytes(body, 1024 * 1024 * 10).await {
            task.state.write().fields.insert(
                "request.body".to_string(),
                ox_workflow_core::state::FieldValue::String(String::from_utf8_lossy(&bytes).to_string()),
            );
            if !bytes.is_empty() {
                let tmp_path = std::env::temp_dir().join(format!(
                    "ox_body_{}_{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0)
                ));
                if std::fs::write(&tmp_path, &bytes).is_ok() {
                    task.state.write().fields.insert(
                        "request.body_path".to_string(),
                        ox_workflow_core::state::FieldValue::String(tmp_path.to_string_lossy().to_string()),
                    );
                    Some(tmp_path)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Acquire a concurrency slot before dispatching. Excess requests wait here as
        // cheap async futures rather than spawning unbounded blocking threads.
        let permit = Arc::clone(&self.plugin_semaphore).acquire_owned().await
            .unwrap_or_else(|_| panic!("plugin semaphore closed"));

        // Run the blocking plugin pipeline on a dedicated blocking thread so tokio async
        // worker threads are never stalled by slow plugins (e.g. sysinfo refresh_all).
        enum PluginResult {
            StreamFile { file_path: String, status_code: u16, response_headers: Vec<(String, String)> },
            Response { status_code: u16, response_headers: HashMap<String, String>, body_bytes: axum::body::Bytes },
        }

        let flow = Arc::clone(&self);
        let plugin_result = tokio::task::spawn_blocking(move || {
            let _permit = permit; // released when this closure returns
            let last_fc = flow.main_flow.run(&mut task, &flow.api);

            if last_fc.code == FLOW_CONTROL_STREAM_FILE && !last_fc.payload.is_null() {
                // Extract and free the heap-allocated file path before leaving this thread.
                let file_path = unsafe { std::ffi::CStr::from_ptr(last_fc.payload) }.to_string_lossy().to_string();
                let _ = unsafe { std::ffi::CString::from_raw(last_fc.payload as *mut std::ffi::c_char) };

                let (status_code, response_headers) = {
                    let r = task.state.read();
                    let sc = r.fields.get("response.status")
                        .and_then(|f| match f {
                            ox_workflow_core::state::FieldValue::String(s) => s.parse::<u16>().ok(),
                            ox_workflow_core::state::FieldValue::Bytes(_) => None,
                        })
                        .unwrap_or(200);
                    let hdrs = r.fields.iter()
                        .filter(|(k, _)| k.starts_with("response.header."))
                        .filter_map(|(k, v)| {
                            if let ox_workflow_core::state::FieldValue::String(s) = v {
                                Some((k["response.header.".len()..].to_string(), s.clone()))
                            } else {
                                None
                            }
                        })
                        .collect();
                    (sc, hdrs)
                };

                if let Some(path) = body_tmp_path {
                    let _ = std::fs::remove_file(path);
                }

                return PluginResult::StreamFile { file_path, status_code, response_headers };
            }

            let r = task.state.read();
            let status_code = r.fields.get("response.status")
                .and_then(|f| match f {
                    ox_workflow_core::state::FieldValue::String(s) => s.parse::<u16>().ok(),
                    ox_workflow_core::state::FieldValue::Bytes(_) => None,
                })
                .unwrap_or(404);

            // response.body may be a String (text/JSON/HTML) or Bytes (protobuf-encoded).
            let body_bytes: axum::body::Bytes = match r.fields.get("response.body") {
                Some(ox_workflow_core::state::FieldValue::String(s)) => axum::body::Bytes::from(s.clone()),
                Some(ox_workflow_core::state::FieldValue::Bytes(b)) => axum::body::Bytes::from(b.clone()),
                None => axum::body::Bytes::from("Not Found"),
            };

            let mut response_headers = HashMap::new();
            for (k, v) in r.fields.iter() {
                if k.starts_with("response.header.") {
                    if let ox_workflow_core::state::FieldValue::String(s) = v {
                        response_headers.insert(k["response.header.".len()..].to_string(), s.clone());
                    }
                }
            }
            drop(r);

            if let Some(path) = body_tmp_path {
                let _ = std::fs::remove_file(path);
            }

            PluginResult::Response { status_code, response_headers, body_bytes }
        }).await.unwrap_or_else(|_| PluginResult::Response {
            status_code: 500,
            response_headers: HashMap::new(),
            body_bytes: axum::body::Bytes::from("Internal Error"),
        });

        match plugin_result {
            PluginResult::StreamFile { file_path, status_code, response_headers } => {
                let mut res = Response::builder().status(status_code);
                for (k, v) in response_headers {
                    res = res.header(k, v);
                }
                match tokio::fs::File::open(&file_path).await {
                    Ok(file) => {
                        use tokio_util::io::ReaderStream;
                        let stream = ReaderStream::new(file);
                        res.body(Body::from_stream(stream)).unwrap_or_else(|_| {
                            Response::builder().status(500).body(Body::from("Internal Error")).unwrap()
                        })
                    }
                    Err(_) => {
                        Response::builder().status(404).body(Body::from("Not Found")).unwrap()
                    }
                }
            }
            PluginResult::Response { status_code, response_headers, body_bytes } => {
                let mut res = Response::builder().status(status_code);
                for (k, v) in response_headers {
                    res = res.header(k, v);
                }
                res.body(Body::from(body_bytes)).unwrap_or_else(|_| {
                    Response::builder().status(500).body(Body::from("Internal Error")).unwrap()
                })
            }
        }
    }

    pub async fn handle_socket(self: Arc<Self>, mut socket: axum::extract::ws::WebSocket, addr: SocketAddr, path: String, ws_protocol: String) {
        use axum::extract::ws::Message;

        while let Some(Ok(msg)) = socket.recv().await {
            let text = match msg {
                Message::Text(t) => t,
                Message::Close(_) => break,
                _ => continue,
            };

            let mut task = Task::new(1);
            {
                let mut w = task.state.write();
                w.fields.insert("request.protocol".to_string(), ox_workflow_core::state::FieldValue::String(ws_protocol.clone()));
                w.fields.insert("request.method".to_string(), ox_workflow_core::state::FieldValue::String("GET".to_string()));
                w.fields.insert("request.path".to_string(), ox_workflow_core::state::FieldValue::String(format!("/{}", path)));
                w.fields.insert("request.query".to_string(), ox_workflow_core::state::FieldValue::String(String::new()));
                w.fields.insert("request.source_ip".to_string(), ox_workflow_core::state::FieldValue::String(addr.ip().to_string()));
                w.fields.insert("request.header.accept".to_string(), ox_workflow_core::state::FieldValue::String("application/json".to_string()));
                w.fields.insert("request.body".to_string(), ox_workflow_core::state::FieldValue::String(text));
                w.fields.insert("response.status".to_string(), ox_workflow_core::state::FieldValue::String("500".to_string()));
                w.fields.insert("response.body".to_string(), ox_workflow_core::state::FieldValue::String(String::new()));
            }

            let permit = Arc::clone(&self.plugin_semaphore).acquire_owned().await
                .unwrap_or_else(|_| panic!("plugin semaphore closed"));

            let flow = Arc::clone(&self);
            let body = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let _last_fc = flow.main_flow.run(&mut task, &flow.api);
                let r = task.state.read();
                r.fields.get("response.body")
                    .map(|f| match f {
                        ox_workflow_core::state::FieldValue::String(s) => s.clone(),
                        ox_workflow_core::state::FieldValue::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
                    })
                    .unwrap_or_default()
            }).await.unwrap_or_default();

            let _ = socket.send(Message::Text(body)).await;
        }
    }
}
