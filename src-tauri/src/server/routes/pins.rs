//! Pinned sessions (`server::pins`): the Sessions page's *Pinned* group, the
//! same on the desktop and every paired device.
//!
//! | Route | Who | Does |
//! |---|---|---|
//! | `GET /api/pins` | Authed | `{pins, sessions}` |
//! | `PUT /api/pins/{id}?machine=` | Authed | pins it (pinned already: unchanged) |
//! | `DELETE /api/pins/{id}?machine=` | Authed | unpins it |
//!
//! `pins` are `{id, machine?, at}` in the order they were pinned; `sessions`
//! are this computer's pinned sessions as the session list knows them, also
//! ones no longer active enough for the list's own frames. A change answers
//! with the new view and goes out as `{t:"pins"}`. A pin is a view setting:
//! it changes nothing in a session.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::claude::model::SessionSummary;
use crate::server::auth::Authed;
use crate::server::error::{ApiError, ApiResult};
use crate::server::pins::{self, Pin};
use crate::server::ws::ServerEvent;
use crate::state::AppState;

/// `GET /api/pins`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinsView {
    pub pins: Vec<Pin>,
    pub sessions: Vec<SessionSummary>,
}

/// Which computer's session (`?machine=`; absent: this computer's).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Where {
    pub machine: Option<String>,
}

async fn view(state: &AppState) -> PinsView {
    let pins = state.pins.list();
    let snap = state.snapshot.read().await;
    let sessions = pins
        .iter()
        .filter(|p| p.machine.is_none())
        .filter_map(|p| snap.find(&p.id).cloned())
        .collect();
    PinsView { pins, sessions }
}

fn checked<'a>(id: &str, w: &'a Where) -> ApiResult<Option<&'a str>> {
    let machine = w.machine.as_deref().filter(|m| !m.is_empty());
    if !pins::valid(id, machine) {
        return Err(ApiError::unprocessable(
            "invalid_session_id",
            "Only a Claude Code session can be pinned.",
        ));
    }
    Ok(machine)
}

/// `GET /api/pins`.
pub async fn list(Authed(_ctx): Authed, State(state): State<AppState>) -> Json<PinsView> {
    Json(view(&state).await)
}

/// `PUT /api/pins/{id}?machine=`.
pub async fn pin(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(w): Query<Where>,
) -> ApiResult<Json<PinsView>> {
    let machine = checked(&id, &w)?;
    let now = chrono::Utc::now().timestamp_millis();
    if state.pins.pin(&id, machine, now) {
        state.broadcast(ServerEvent::Pins);
    }
    Ok(Json(view(&state).await))
}

/// `DELETE /api/pins/{id}?machine=`.
pub async fn unpin(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(w): Query<Where>,
) -> ApiResult<Json<PinsView>> {
    let machine = checked(&id, &w)?;
    if state.pins.unpin(&id, machine) {
        state.broadcast(ServerEvent::Pins);
    }
    Ok(Json(view(&state).await))
}
