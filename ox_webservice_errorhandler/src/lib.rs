use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
};
use minijinja::Environment;
use serde_json;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Arc};
use anyhow::Result;
use log::{error, info};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub enum ErrorHandlerMode {
    Render,
    Forward,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ErrorHandlerConfig {
    pub mode: ErrorHandlerMode,
    pub content_root: Option<PathBuf>,
    pub redirect_url: Option<String>,
}

impl ErrorHandlerConfig {
    pub fn from_yaml_file(path: &PathBuf) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: ErrorHandlerConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }
}

pub async fn handle_error(
    status_code: StatusCode,
    message: String,
    context: String,
    config_path: PathBuf,
) -> impl IntoResponse {
    info!("Handling error: status_code={}, message={}, context={}", status_code, message, context);

    let config = match ErrorHandlerConfig::from_yaml_file(&config_path) {
        Ok(cfg) => Arc::new(cfg),
        Err(e) => {
            error!("Failed to load error handler configuration from {:?}: {}", config_path, e);
            return internal_error_response(status_code, message, context);
        }
    };

    match config.mode {
        ErrorHandlerMode::Render => {
            if let Some(content_root) = &config.content_root {
                let template_name = format!("{}.jinja2", status_code.as_u16());
                let template_path = content_root.join(&template_name);

                if template_path.exists() {
                    match render_template(&content_root, &template_name, status_code, &message, &context) {
                        Ok(html) => (status_code, Html(html)).into_response(),
                        Err(e) => {
                            error!("Failed to render template {}: {}", template_name, e);
                            internal_error_response(status_code, message, context)
                        }
                    }
                } else {
                    info!("Template {} not found. Rendering default HTML.", template_name);
                    default_html_response(status_code, message, context)
                }
            } else {
                error!("Content root not specified for render mode.");
                default_html_response(status_code, message, context)
            }
        }
        ErrorHandlerMode::Forward => {
            if let Some(redirect_url_base) = &config.redirect_url {
                let redirect_url = format!("{}{}.jinja2", redirect_url_base, status_code.as_u16());
                info!("Redirecting to: {}", redirect_url);
                Redirect::to(&redirect_url).into_response()
            } else {
                error!("Redirect URL not specified for forward mode.");
                default_html_response(status_code, message, context)
            }
        }
    }
}


fn render_template(
    content_root: &PathBuf,
    template_name: &str,
    status_code: StatusCode,
    message: &str,
    context: &str,
) -> Result<String> {
    let mut env = Environment::new();
    let template_content = fs::read_to_string(content_root.join(template_name))?;
    env.add_template(template_name, &template_content)?;

    let json_context = serde_json::json!({
        "status_code": status_code.as_u16(),
        "message": message,
        "context": context,
    });
    let minijinja_value_context = minijinja::Value::from_serialize(&json_context);

    Ok(env.get_template(template_name)?.render(minijinja_value_context)?)
}

fn default_html_response(status_code: StatusCode, message: String, context: String) -> axum::response::Response {
    let html_content = format!(
        "<h1>{}</h1><h2>{}</h2><h3>{}</h3>",
        status_code.as_u16(),
        message,
        context
    );
    (status_code, Html(html_content)).into_response()
}

fn internal_error_response(status_code: StatusCode, message: String, context: String) -> axum::response::Response {
    let html_content = format!(
        "<h1>{}</h1><h2>{}</h2><h3>{}</h3><p>Additionally, an internal error occurred while trying to handle this error.</p>",
        status_code.as_u16(),
        message,
        context
    );
    (StatusCode::INTERNAL_SERVER_ERROR, Html(html_content)).into_response()
}