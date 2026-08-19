mod rpc;

use anyhow::Context;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
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
            let Ok(s) = origin.to_str() else {
                return false;
            };
            let s = s.to_ascii_lowercase();
            s.starts_with("http://localhost")
                || s.starts_with("http://127.0.0.1")
                || s.starts_with("https://localhost")
                || s.starts_with("https://127.0.0.1")
                || s.starts_with("tauri://")
                || s.contains("tauri.localhost")
        }))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(tower_http::cors::Any)
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

async fn rpc_http(State(app): State<App>, body: String) -> impl IntoResponse {
    let resp = rpc::handle(&app, &body).await;
    ([(header::CONTENT_TYPE, "application/json")], resp)
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(app): State<App>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_loop(socket, app))
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
