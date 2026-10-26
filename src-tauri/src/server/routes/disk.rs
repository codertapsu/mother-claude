//! Finding conversations on disk (docs/ACCOUNTS.md §14.1–14.2).
//!
//! * `GET /api/data/projects` (`Authed`) → `{projects: [{account, cwd, name,
//!   folder, sessions, live, bytes, lastActivityAt}]}`: every project folder
//!   that holds conversations, in every account's directory, newest first.
//! * `POST /api/sessions/{id}/reveal` (`DesktopOnly`) → `204`: the transcript
//!   selected in Finder or File Explorer (its folder opened elsewhere);
//!   `404 session_not_found` / `transcript_not_found`.
//! * `POST /api/data/projects/reveal {account, cwd}` (`DesktopOnly`) → `204`:
//!   that account's `projects/<enc>` folder of `cwd` opened; `404
//!   project_not_found`.
//!
//! A session's own paths are in its detail (`files`). Every path comes from
//! the index, never from a request.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::Body;
use crate::claude::reveal::Target;
use crate::claude::{disk, ids};
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::error::{ApiError, ApiResult};
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

/// `POST /api/sessions/{id}/reveal`.
pub async fn reveal_session(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    ids::require_session_id(&id)?;
    let known = state.find_session(&id).await.is_some() || state.index.session_files(&id).is_some();
    if !known {
        return Err(ApiError::no_such_session(&id));
    }
    let missing = || {
        ApiError::not_found(
            "transcript_not_found",
            "This conversation has no transcript on disk.",
        )
    };
    let file = state.index.transcript_path(&id).ok_or_else(missing)?;
    reveal(&state, Target::File(file), missing).await
}

/// The body of `POST /api/data/projects/reveal`: a row of `GET
/// /api/data/projects`.
#[derive(Debug, Deserialize)]
pub struct ProjectRef {
    pub account: String,
    pub cwd: String,
}

/// `POST /api/data/projects/reveal`.
pub async fn reveal_project(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<ProjectRef>,
) -> ApiResult<StatusCode> {
    let missing = || {
        ApiError::not_found(
            "project_not_found",
            "That account has no conversations of that folder.",
        )
    };
    let summaries = state.sessions().await;
    let st = state.clone();
    let folder = tokio::task::spawn_blocking(move || {
        disk::project_folder(&st, &summaries, &body.account, &body.cwd)
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(missing)?;
    reveal(&state, Target::Folder(folder), missing).await
}

/// Show `target` once it is checked to be there (a regular file, a
/// directory — never through a symlink).
async fn reveal(
    state: &AppState,
    target: Target,
    missing: impl FnOnce() -> ApiError,
) -> ApiResult<StatusCode> {
    let st = state.clone();
    let shown = tokio::task::spawn_blocking(move || {
        let there = match &target {
            Target::File(p) => std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()),
            Target::Folder(p) => std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()),
        };
        there.then(|| st.reveal.reveal(&target))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    match shown {
        None => Err(missing()),
        Some(Ok(())) => Ok(StatusCode::NO_CONTENT),
        Some(Err(e)) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "reveal_failed",
            format!("Could not open the file manager: {e}"),
        )),
    }
}
