use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::Body,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use rust_embed::RustEmbed;
use serde_json::json;
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::{
    hub::SessionHub,
    protocol::{ClientMessage, ServerMessage},
};

#[derive(Clone)]
pub struct AppState {
    pub hub: Arc<SessionHub>,
}

#[derive(RustEmbed)]
#[folder = "web/"]
struct WebAssets;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/ws", get(websocket))
        .fallback(get(asset))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "service": "pi-mobile-ui", "protocol": 1 }))
}

async fn websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if !same_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.max_message_size(2 * 1024 * 1024)
        .on_upgrade(move |socket| run_socket(socket, state))
        .into_response()
}

fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .and_then(|value| value.split('/').next())
        == Some(host)
}

async fn run_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut bus = state.hub.bus.subscribe();
    if send(&mut sender, &ServerMessage::Hello { protocol: 1 })
        .await
        .is_err()
    {
        return;
    }
    let sessions = state.hub.list().await;
    if send(&mut sender, &ServerMessage::Sessions { sessions })
        .await
        .is_err()
    {
        return;
    }

    let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(error) = handle_client(text.as_str(), &state, &mut sender).await {
                            let _ = send(&mut sender, &ServerMessage::Error { message: error.to_string() }).await;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(error)) => {
                        debug!(%error, "browser WebSocket closed");
                        break;
                    }
                    _ => {}
                }
            }
            event = bus.recv() => {
                match event {
                    Ok(event) => if send(&mut sender, &event).await.is_err() { break; },
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "browser lagged behind the event stream");
                    },
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = heartbeat.tick() => {
                if sender.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
            }
        }
    }
}

async fn handle_client(
    text: &str,
    state: &AppState,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    let message: ClientMessage = serde_json::from_str(text)?;
    match message {
        ClientMessage::ListSessions => {
            send(
                sender,
                &ServerMessage::Sessions {
                    sessions: state.hub.list().await,
                },
            )
            .await?;
        }
        ClientMessage::CreateSession => {
            let runtime = state.hub.create().await?;
            send(sender, &state.hub.open_payload(&runtime).await?).await?;
            send(
                sender,
                &ServerMessage::Sessions {
                    sessions: state.hub.list().await,
                },
            )
            .await?;
        }
        ClientMessage::OpenSession { catalog_id } => {
            let runtime = state.hub.open_catalog(&catalog_id).await?;
            send(sender, &state.hub.open_payload(&runtime).await?).await?;
            send(
                sender,
                &ServerMessage::Sessions {
                    sessions: state.hub.list().await,
                },
            )
            .await?;
        }
        ClientMessage::ActivateSession { runtime_id } => {
            let runtime = state.hub.get(&runtime_id).await?;
            send(sender, &state.hub.open_payload(&runtime).await?).await?;
        }
        ClientMessage::Prompt {
            runtime_id,
            message,
            behavior,
        } => {
            state.hub.prompt(&runtime_id, message, behavior).await?;
        }
        ClientMessage::Abort { runtime_id } => {
            state.hub.get(&runtime_id).await?.abort().await?;
        }
        ClientMessage::Compact { runtime_id } => {
            state.hub.command(&runtime_id, "compact", json!({})).await?;
        }
        ClientMessage::SetModel {
            runtime_id,
            provider,
            model_id,
        } => {
            state
                .hub
                .command(
                    &runtime_id,
                    "set_model",
                    json!({ "provider": provider, "modelId": model_id }),
                )
                .await?;
        }
        ClientMessage::SetThinkingLevel { runtime_id, level } => {
            state
                .hub
                .command(&runtime_id, "set_thinking_level", json!({ "level": level }))
                .await?;
        }
    }
    Ok(())
}

async fn send(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: &ServerMessage,
) -> anyhow::Result<()> {
    let text = serde_json::to_string(message)?;
    sender.send(Message::Text(text.into())).await?;
    Ok(())
}

async fn asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(asset) = WebAssets::get(path).or_else(|| WebAssets::get("index.html")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(
            header::CACHE_CONTROL,
            if path == "index.html" {
                "no-cache"
            } else {
                "public, max-age=3600"
            },
        )
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header("referrer-policy", "no-referrer")
        .header("x-frame-options", "DENY")
        .header(
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self' ws: wss:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        )
        .body(Body::from(asset.data.into_owned()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
