//! Finding conversations on disk (docs/ACCOUNTS.md §14.2).
//!
//! * `GET /api/data/projects` (`Authed`) → `{projects: [{account, cwd, name,
//!   folder, sessions, live, bytes, lastActivityAt}]}`: every project folder
//!   that holds conversations, in every account's directory, newest first.
//!
//! Every path comes from the index, never from a request.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::claude::disk;
use crate::server::auth::Authed;
use crate::state::AppState;

/// `GET /api/data/projects`.
pub async fn projects(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    let summaries = state.sessions().await;
    let st = state.clone();
    let rows = tokio::task::spawn_blocking(move || disk::projects(&st, &summaries))
        .await
        .unwrap_or_default();
    Json(json!({ "projects": rows }))
}
