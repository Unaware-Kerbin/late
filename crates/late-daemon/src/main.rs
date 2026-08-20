mod rpc;

use anyhow::Context;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use late_core::origin::{is_allowed_origin, is_loopback_host_header};
use late_core::App;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "late-daemon", about = "Local AI Terminal Emulator daemon")]
struct Args {
    /// Bind address (JSON-RPC WebSocket + REST)
    #[arg(long, default_value = "127.0.0.1:7420")]
    bind: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let app = App::boot().context("boot late-core")?;
    let bind: SocketAddr = args.bind.parse().context("parse --bind")?;
    if !bind.ip().is_loopback() && std::env::var("LATE_INSECURE_BIND").ok().as_deref() != Some("1")
    {
        anyhow::bail!(
            "refusing to bind {bind} (not loopback). Set LATE_INSECURE_BIND=1 only for isolated labs."
        );
    }

    let router = Router::new()
        .route("/health", get(health))
        .route("/healthz", get(health))
        .route("/rpc", post(rpc_http))
        .route("/ws", get(ws_upgrade))
        .route("/internal/provider-keys", get(internal_provider_keys))
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(app);

    tracing::info!("late-daemon listening on {bind}");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _| {
            origin.to_str().map(is_allowed_origin).unwrap_or(false)
        }))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::HeaderName::from_static("x-late-token"),
            header::HeaderName::from_static("x-late-sidecar-token"),
        ])
}

#[derive(Debug, Deserialize, Default)]
struct TokenQuery {
    token: Option<String>,
}

fn presented_token(headers: &HeaderMap, query: Option<&str>) -> Option<String> {
    for name in ["x-late-token", "x-late-sidecar-token"] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    if let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(t) = v
            .strip_prefix("Bearer ")
            .or_else(|| v.strip_prefix("bearer "))
        {
            let t = t.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    query
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Token is authentication. Origin is CORS only (browsers set it; curl can forge it).
fn authorize(
    app: &App,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> std::result::Result<(), (StatusCode, Json<Value>)> {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !is_loopback_host_header(host) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "host not allowed"})),
        ));
    }
    let Some(tok) = presented_token(headers, query_token) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        ));
    };
    if app.providers.token_matches(&tok) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        ))
    }
}

async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "service": "late-daemon",
        "features": ["pcap.edgeshark", "pcap.wireshark", "pcap.start", "pcap.remote.start"]
    }))
}

/// Sidecar-only. Rejects browser Origin. Requires 0600 token. Never on the UI RPC surface.
async fn internal_provider_keys(
    State(app): State<App>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if headers.get(header::ORIGIN).is_some() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "browser origin rejected"})),
        );
    }
    let Some(presented) = headers
        .get("x-late-sidecar-token")
        .and_then(|v| v.to_str().ok())
    else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing sidecar token"})),
        );
    };
    if !app.providers.token_matches(presented) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "sidecar token mismatch"})),
        );
    }
    match app.providers.materialize() {
        Ok(keys) => (StatusCode::OK, Json(json!(keys))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "unavailable"})),
        ),
    }
}

async fn rpc_http(State(app): State<App>, headers: HeaderMap, body: String) -> impl IntoResponse {
    if let Err(resp) = authorize(&app, &headers, None) {
        return resp.into_response();
    }
    let resp = rpc::handle(&app, &body).await;
    ([(header::CONTENT_TYPE, "application/json")], resp).into_response()
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(resp) = authorize(&app, &headers, q.token.as_deref()) {
        return resp.into_response();
    }
    ws.on_upgrade(move |socket| ws_loop(socket, app))
        .into_response()
}

async fn ws_loop(mut socket: WebSocket, app: App) {
    let mut events = app.events.subscribe();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(t))) => {
                        let resp = rpc::handle(&app, t.as_str()).await;
                        if socket.send(Message::Text(resp.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = socket.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            ev = events.recv() => {
                match ev {
                    Ok(e) => {
                        if let Ok(s) = serde_json::to_string(&e) {
                            if socket.send(Message::Text(s.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => {
                        events = app.events.subscribe();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn presented_token_reads_headers_and_query() {
        let mut h = HeaderMap::new();
        h.insert("x-late-token", HeaderValue::from_static("abc"));
        assert_eq!(presented_token(&h, None).as_deref(), Some("abc"));

        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer xyz"),
        );
        assert_eq!(presented_token(&h, Some("ignored")).as_deref(), Some("xyz"));

        let h = HeaderMap::new();
        assert_eq!(presented_token(&h, Some("qtok")).as_deref(), Some("qtok"));
        assert!(presented_token(&h, Some("")).is_none());
    }
}
