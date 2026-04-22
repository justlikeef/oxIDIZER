use axum::{extract::{ws::WebSocketUpgrade, Request}, body::Body, response::Response, routing::get, Router};

async fn handler(ws: Option<WebSocketUpgrade>, req: Request<Body>) -> Response {
    if let Some(ws) = ws {
        ws.on_upgrade(|_socket| async { })
    } else {
        Response::builder().body(Body::from("http")).unwrap()
    }
}
pub fn check() { let _app = Router::new().route("/*path", axum::routing::any(handler)); }
