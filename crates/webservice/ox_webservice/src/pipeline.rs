use std::collections::HashMap;
use std::sync::Arc;
use std::net::SocketAddr;
use axum::http::Request;
use axum::body::{Body, to_bytes};
use axum::response::Response;
use ox_workflow_core::Task;
use ox_workflow_core::{FlowDef, StageDef, PluginDef};
use ox_workflow_executor::{FlowManager, create_host_api, FlowRunner};
use ox_workflow_abi::CoreHostApi;
use crate::ServerConfig;
use tokio::sync::RwLock;

pub struct Pipeline {
    pub main_config_json: String,
    pub flow_manager: Arc<RwLock<FlowManager>>,
    pub main_flow: Arc<FlowRunner>,
    pub api: CoreHostApi,
}

impl Pipeline {
    pub fn new(config: &ServerConfig, config_json: String) -> Result<Self, String> {
        let mut manager = FlowManager::new();
        let api = create_host_api();
        
        let mut plugin_paths = HashMap::new();
        
        let router_path = std::env::var("OX_ROUTER_PATH").unwrap_or_else(|_| "../target/debug/libox_webservice_router.so".to_string());
        plugin_paths.insert("ox_webservice_router".to_string(), router_path);
        
        let mut stages = Vec::new();
        let mut router_routes = Vec::new();

        for mod_cfg in &config.modules {
            let path = mod_cfg.path.clone().unwrap_or_else(|| format!("../target/debug/lib{}.so", mod_cfg.name));
            let id = mod_cfg.id.clone().unwrap_or_else(|| mod_cfg.name.clone());
            plugin_paths.insert(id.clone(), path);

            // Each module gets its own stage, named after its ID. This allows the router to jump directly to it.
            let mod_config_val = serde_json::to_value(mod_cfg).unwrap_or(serde_json::Value::Null);
            manager.stage_defs.insert(id.clone(), StageDef {
                name: id.clone(),
                runner: "sequential".to_string(),
                plugins: vec![PluginDef {
                    name: id.clone(),
                    config: Some(mod_config_val),
                }],
                on_error: Some("errored".to_string()),
            });
            stages.push(id.clone());
        }

        for r in &config.routes {
             let mut map = serde_json::Map::new();
             if let Some(p) = &r.protocol { map.insert("protocol".to_string(), serde_json::Value::String(p.clone())); }
             if let Some(h) = &r.hostname { map.insert("hostname".to_string(), serde_json::Value::String(h.clone())); }
             if let Some(u) = &r.url { map.insert("path".to_string(), serde_json::Value::String(u.clone())); }
             if let Some(sc) = &r.status_code { map.insert("status_code".to_string(), serde_json::Value::String(sc.clone())); }
             
             let mut headers_map = serde_json::Map::new();
             if let Some(h) = &r.headers {
                 for (k, v) in h { headers_map.insert(k.clone(), serde_json::Value::String(v.clone())); }
             }
             if !headers_map.is_empty() { map.insert("headers".to_string(), serde_json::Value::Object(headers_map)); }

             let mut query_map = serde_json::Map::new();
             if let Some(q) = &r.query {
                 for (k, v) in q { query_map.insert(k.clone(), serde_json::Value::String(v.clone())); }
             }
             if !query_map.is_empty() { map.insert("query".to_string(), serde_json::Value::Object(query_map)); }

             let entry = serde_json::json!({
                 "matcher": map,
                 "module_id": r.module_id.clone().unwrap_or_default(),
                 "priority": r.priority
             });
             router_routes.push(entry);
        }

        let router_cfg = serde_json::json!({ "routes": router_routes });
        manager.stage_defs.insert("router".to_string(), StageDef {
            name: "router".to_string(),
            runner: "sequential".to_string(),
            plugins: vec![PluginDef {
                name: "ox_webservice_router".to_string(),
                config: Some(router_cfg),
            }],
            on_error: Some("continue".to_string()),
        });

        // The flow starts at the router. The router will emit a JUMP FlowControl to the specific module stage.
        let mut final_stages = vec!["router".to_string()];
        final_stages.extend(stages); // Add all possible jump targets so they are compiled into the FlowRunner

        let flow_def = FlowDef {
            name: "http_pipeline".to_string(),
            persistent: false,
            stages: final_stages,
        };

        let main_flow = unsafe {
            manager.build_flow(&flow_def, &api, &plugin_paths)
                .map_err(|e| format!("Failed to build flow: {:?}", e))?
        };

        Ok(Self {
            main_config_json: config_json,
            flow_manager: Arc::new(RwLock::new(manager)),
            main_flow,
            api,
        })
    }

    pub async fn execute_request(&self, addr: SocketAddr, req: Request<Body>, protocol: String) -> Response {
        let mut task = Task::new(1);
        
        let (parts, body) = req.into_parts();
        
        {
            let mut w = task.state.write();
            w.fields.insert("request.protocol".to_string(), ox_workflow_core::state::FieldValue::String(protocol));
            w.fields.insert("request.method".to_string(), ox_workflow_core::state::FieldValue::String(parts.method.to_string()));
            w.fields.insert("request.path".to_string(), ox_workflow_core::state::FieldValue::String(parts.uri.path().to_string()));
            w.fields.insert("request.query".to_string(), ox_workflow_core::state::FieldValue::String(parts.uri.query().unwrap_or("").to_string()));
            w.fields.insert("request.source_ip".to_string(), ox_workflow_core::state::FieldValue::String(addr.to_string()));
            
            for (k, v) in parts.headers.iter() {
                w.fields.insert(format!("request.header.{}", k.as_str()), ox_workflow_core::state::FieldValue::String(v.to_str().unwrap_or("").to_string()));
            }

            // Defaults for response in case flow fails early
            w.fields.insert("response.status".to_string(), ox_workflow_core::state::FieldValue::String("404".to_string()));
            w.fields.insert("response.body".to_string(), ox_workflow_core::state::FieldValue::String("".to_string()));
        }
        
        // Try reading body for TaskState (only up to memory limit). Usually plugins read it inside. But we inject it if possible.
        if let Ok(bytes) = to_bytes(body, 1024 * 1024 * 10).await {
            task.state.write().fields.insert("request.body".to_string(), ox_workflow_core::state::FieldValue::String(String::from_utf8_lossy(&bytes).to_string()));
        }

        let _last_fc = self.main_flow.run(&mut task, &self.api);

        let r = task.state.read();
        let status_code = r.fields.get("response.status")
            .and_then(|f| match f { ox_workflow_core::state::FieldValue::String(s) => s.parse::<u16>().ok() })
            .unwrap_or(404);
            
        let body_content = r.fields.get("response.body")
            .map(|f| match f { ox_workflow_core::state::FieldValue::String(s) => s.clone() })
            .unwrap_or("Not Found".to_string());
            
        let mut res = Response::builder().status(status_code);
        
        // Reconstruct response headers
        let mut headers = HashMap::new();
        for (k, v) in r.fields.iter() {
            if k.starts_with("response.header.") {
                headers.insert(k["response.header.".len()..].to_string(), match v { ox_workflow_core::state::FieldValue::String(s) => s.clone() });
            }
        }
        
        for (k, v) in headers {
            res = res.header(k, v);
        }

        res.body(Body::from(body_content)).unwrap_or_else(|_| {
            Response::builder().status(500).body(Body::from("Internal Error")).unwrap()
        })
    }

    pub async fn handle_socket(&self, _socket: axum::extract::ws::WebSocket, _addr: SocketAddr, _path: String, _ws_protocol: String) {
        // Unimplemented for phase 1 of rewrite
    }
}
