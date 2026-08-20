mod rpc;

use anyhow::Context;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use late_core::origin::{is_allowed_origin, is_loopback_host_header};
use late_core::App;
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

fn host_header_enforced() -> bool {
    std::env::var("LATE_INSECURE_BIND").ok().as_deref() != Some("1")
}

/// Browser WebSocket cannot set `X-Late-Token`. The token is a `late.<hex>` subprotocol.
fn ws_late_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::SEC_WEBSOCKET_PROTOCOL)?.to_str().ok()?;
    for part in raw.split(',') {
        let p = part.trim();
        let Some(tok) = p.strip_prefix("late.") else {
            continue;
        };
        if tok.is_empty() || tok.len() > 128 {
            continue;
        }
        if tok
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return Some(tok.to_string());
        }
    }
    None
}

fn presented_token(headers: &HeaderMap, extra: Option<&str>) -> Option<String> {
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
    extra
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Token is authentication. Origin is CORS only (browsers set it; curl can forge it).
/// Host is a DNS-rebinding guard on loopback bind — not a network boundary when
/// `LATE_INSECURE_BIND=1`.
fn authorize(
    app: &App,
    headers: &HeaderMap,
    extra_token: Option<&str>,
) -> std::result::Result<(), (StatusCode, Json<Value>)> {
    if host_header_enforced() {
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
    }
    let Some(tok) = presented_token(headers, extra_token) else {
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
    Json(json!({ "ok": true }))
}

/// Sidecar-only. Rejects browser Origin. Same Host + token gate as `/rpc`.
async fn internal_provider_keys(
    State(app): State<App>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if headers.get(header::ORIGIN).is_some() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "browser origin rejected"})),
        )
            .into_response();
    }
    if let Err(resp) = authorize(&app, &headers, None) {
        return resp.into_response();
    }
    match app.providers.materialize() {
        Ok(keys) => (StatusCode::OK, Json(json!(keys))).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "unavailable"})),
        )
            .into_response(),
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
) -> impl IntoResponse {
    let proto_tok = ws_late_token(&headers);
    let header_tok = presented_token(&headers, None);
    if let (Some(p), Some(h)) = (proto_tok.as_deref(), header_tok.as_deref()) {
        if p != h {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized"})),
            )
                .into_response();
        }
    }
    if let Err(resp) = authorize(&app, &headers, proto_tok.as_deref()) {
        return resp.into_response();
    }
    let upgrade = if let Some(ref tok) = proto_tok {
        ws.protocols([format!("late.{tok}")])
    } else {
        ws
    };
    upgrade
        .on_upgrade(move |socket| ws_loop(socket, app))
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
    fn presented_token_reads_headers_then_extra() {
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

    #[test]
    fn ws_subprotocol_parses_late_token() {
        let mut h = HeaderMap::new();
        h.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("late.deadbeef"),
        );
        assert_eq!(ws_late_token(&h).as_deref(), Some("deadbeef"));

        let mut h = HeaderMap::new();
        h.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("chat, late.ab12"),
        );
        assert_eq!(ws_late_token(&h).as_deref(), Some("ab12"));

        let mut h = HeaderMap::new();
        h.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("late../etc/passwd"),
        );
        assert!(ws_late_token(&h).is_none());
    }
}
