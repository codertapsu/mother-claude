//! WebSocket fan-out: one socket per client, fed from the broadcast bus.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use tokio::sync::broadcast::error::RecvError;

use crate::state::{AppState, ServerEvent};

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle(socket, state))
}

async fn handle(mut socket: WebSocket, state: AppState) {
    // Send an immediate snapshot so a freshly connected client renders at once.
    let snapshot = ServerEvent::Sessions(state.sessions.read().await.clone());
    if let Ok(text) = serde_json::to_string(&snapshot) {
        if socket.send(Message::Text(text)).await.is_err() {
            return;
        }
    }

    let mut rx = state.bus.subscribe();
    loop {
        tokio::select! {
            received = rx.recv() => match received {
                Ok(event) => {
                    if let Ok(text) = serde_json::to_string(&event) {
                        if socket.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "ws client lagged; dropping events");
                }
                Err(RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => { /* client->server messages unused for now */ }
                Some(Err(_)) => break,
            },
        }
    }
}
