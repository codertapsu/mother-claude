//! The foreign-control hook dispatcher (design §5.2, server side).
//!
//! Owner: **B4 (foreign control)** — minimal STUB. The plugin's `mc-hook` /
//! `mc-wake` scripts call:
//!
//! * `POST /hooks/event` ([`post_event`], [`HookOnly`] guard — the `mch_…`
//!   token from a local client): parse `hook_event_name`, `session_id`,
//!   `agent_id`, `tool_use_id`, `tool_name`, `tool_input`, `prompt_id`;
//!   maintain per-session HookState; hold `PermissionRequest` as a
//!   `crate::pending::raise` item (source `Hook`) until decided / withdrawn /
//!   3580 s; answer PreToolUse/PostToolUse/Stop with interrupt or relay output.
//!   Owned session ids ⇒ `{}` immediately.
//! * `GET /hooks/wait?session=&pid=` ([`wait`]): single long-poll waiter per
//!   session (newest wins, older get 410); 200 + RELAY text when a queued
//!   message can be delivered to an idle session, 204 after 50 s.
//!
//! The foundation's stub records the time of the last event per session so
//! `ControlCaps.hooksActive` works, and answers `{}` (no decision) — the same
//! observable behaviour as before for a hook that does not hold.
//!
//! Functions called by other agents (keep the signatures):
//! [`is_active`] (data layer, caps), [`enqueue_relay`], [`request_interrupt`],
//! [`cancel_queued`], [`queued`] (message dispatcher in
//! `server/routes/control.rs`). AppState field: `state.hooks`.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::auth::HookOnly;
use super::error::{ApiError, ApiResult};
use crate::claude::model::QueuedMessage;
use crate::state::AppState;

/// How recent the last hook event must be for `hooksActive`.
pub const ACTIVE_WINDOW_MS: i64 = 30 * 60 * 1000;

/// Per-session hook bookkeeping. STUB: last event time only.
#[derive(Debug, Default)]
pub struct HookHub {
    last_event: Mutex<HashMap<String, i64>>,
}

impl HookHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `session_id` sent a hook event at `now_ms`.
    pub fn touch(&self, session_id: &str, now_ms: i64) {
        if let Ok(mut m) = self.last_event.lock() {
            m.insert(session_id.to_string(), now_ms);
        }
    }

    /// Epoch ms of the session's last hook event.
    pub fn last_event_at(&self, session_id: &str) -> Option<i64> {
        self.last_event
            .lock()
            .ok()
            .and_then(|m| m.get(session_id).copied())
    }

    /// Epoch ms of the most recent hook event from any session.
    pub fn last_any_event_at(&self) -> Option<i64> {
        self.last_event
            .lock()
            .ok()
            .and_then(|m| m.values().copied().max())
    }
}

/// Whether hooks from `session_id` were seen recently (drives
/// `ControlCaps.hooksActive` and the message dispatcher).
pub fn is_active(state: &AppState, session_id: &str) -> bool {
    let now = crate::claude::registry::now_ms();
    state
        .hooks
        .last_event_at(session_id)
        .is_some_and(|t| now - t < ACTIVE_WINDOW_MS)
}

/// Queue `text` for delivery to a hooked foreign session (next PostToolUse /
/// Stop when busy; wake the `/hooks/wait` waiter when idle). STUB.
pub async fn enqueue_relay(
    _state: &AppState,
    _session_id: &str,
    _text: &str,
) -> ApiResult<QueuedMessage> {
    Err(ApiError::not_implemented(
        "Relaying messages through hooks (B4)",
    ))
}

/// Flag the session so its next PreToolUse/PostToolUse/Stop answers
/// `{"continue":false}`. STUB.
pub async fn request_interrupt(_state: &AppState, _session_id: &str) -> ApiResult<()> {
    Err(ApiError::not_implemented("Interrupting through hooks (B4)"))
}

/// Cancel a queued relay message. STUB.
pub async fn cancel_queued(
    _state: &AppState,
    _session_id: &str,
    _message_id: &str,
) -> ApiResult<()> {
    Err(ApiError::not_implemented(
        "Cancelling relayed messages (B4)",
    ))
}

/// Queued relay messages of a session (SessionDetail.queuedMessages). STUB.
pub fn queued(_state: &AppState, _session_id: &str) -> Vec<QueuedMessage> {
    Vec::new()
}

/// Start background work (e.g. withdraw held requests whose pidfile left
/// `waiting`). Called once from `server::serve`. STUB.
pub fn start(_state: &AppState) {}

/// `POST /hooks/event`. STUB: record activity, never decide.
pub async fn post_event(
    HookOnly(_ctx): HookOnly,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    if let Some(session) = payload.get("session_id").and_then(Value::as_str) {
        state
            .hooks
            .touch(session, crate::claude::registry::now_ms());
    }
    let name = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("?");
    tracing::debug!(hook = name, "hook event");
    // `{}` = no decision; the CLI carries on as if no hook answered.
    Json(json!({})).into_response()
}

/// `GET /hooks/wait?session=&pid=`. STUB.
pub async fn wait(
    HookOnly(_ctx): HookOnly,
    State(_state): State<AppState>,
    Query(_params): Query<HashMap<String, String>>,
) -> Response {
    ApiError::not_implemented("The hook wake channel (B4)").into_response()
}
