//! Moving and copying data between accounts, and the data jobs that do it
//! (docs/ACCOUNTS.md §4.4, P3.2).
//!
//! * `POST /api/data/move` (DesktopOnly) `{what: "session", id} |
//!   {what: "project", cwd}` plus `{from, to, mode: "move"|"copy", trust?,
//!   dryRun?}` → `200 {plan}` for a dry run, else `202 {job}`: the transfer
//!   runs in the background as a data job.
//! * `GET /api/data/jobs` and `GET /api/data/jobs/{id}` (Authed): progress,
//!   result, until when undo works.
//! * `POST /api/data/jobs/{id}/undo` (DesktopOnly).
//!
//! Every refusal has its own code (`same_directory`, `session_live`,
//! `already_exists`, `directory_in_use`, `config_locked`, …) and every
//! transfer and undo is audited.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::Body;
use crate::claude::transfer::{self, ops, Mode, Request, TransferError, What};
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Map a refusal onto the API envelope.
pub fn transfer_error(e: TransferError) -> ApiError {
    let status = match &e {
        TransferError::NotFound(_) => StatusCode::NOT_FOUND,
        TransferError::SessionLive(_)
        | TransferError::AlreadyExists(_)
        | TransferError::DirectoryInUse(_)
        | TransferError::ConfigLocked(_)
        | TransferError::UndoConflict(_) => StatusCode::CONFLICT,
        TransferError::Io(_) | TransferError::VerifyFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
        TransferError::SameDirectory | TransferError::Invalid(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
    };
    ApiError::new(status, e.code(), e.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveBody {
    #[serde(flatten)]
    pub what: What,
    pub from: String,
    pub to: String,
    pub mode: Mode,
    #[serde(default)]
    pub trust: bool,
    #[serde(default)]
    pub dry_run: bool,
}

/// `POST /api/data/move`.
pub async fn post_move(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<MoveBody>,
) -> ApiResult<Response> {
    if ops::undo_root(&state).is_none() {
        return Err(ApiError::conflict(
            "no_data_dir",
            "This build has no data directory to keep an undo in.",
        ));
    }
    let req = Request {
        what: body.what.clone(),
        mode: body.mode,
        trust: body.trust,
        snapshot: false,
    };
    let plan = ops::plan(&state, &body.from, &body.to, req)
        .await
        .map_err(transfer_error)?;
    if body.dry_run {
        return Ok(Json(json!({ "plan": plan })).into_response());
    }
    let job_id = transfer::new_job_id();
    state.devices.audit.record(AuditEntry {
        detail: Some(json!({
            "job": job_id, "what": body.what, "from": body.from, "to": body.to,
            "mode": body.mode, "trust": body.trust,
            "sessions": plan.sessions.len(), "files": plan.files, "bytes": plan.bytes,
        })),
        ..AuditEntry::from_ctx(
            &ctx,
            match body.mode {
                Mode::Move => "data.move",
                Mode::Copy => "data.copy",
            },
        )
    });
    // Registered before it runs, so the first poll finds it.
    let kind = match body.mode {
        Mode::Move => "move",
        Mode::Copy => "copy",
    };
    drop(
        state
            .jobs
            .start(&job_id, kind, plan.files as u64, plan.bytes),
    );
    let st = state.clone();
    let id = job_id.clone();
    tokio::spawn(async move {
        if let Err(e) = ops::execute(&st, &id, plan).await {
            tracing::warn!(job = %id, error = %e, "data transfer failed");
        }
    });
    let view = ops::job(&state, &job_id);
    Ok((StatusCode::ACCEPTED, Json(json!({ "job": view }))).into_response())
}

/// `GET /api/data/jobs`.
pub async fn list_jobs(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "jobs": state.jobs.list() }))
}

/// `GET /api/data/jobs/{id}`.
pub async fn get_job(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<transfer::JobView>> {
    ops::job(&state, &id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("job_not_found", "No such data job."))
}

/// `POST /api/data/jobs/{id}/undo`.
pub async fn undo_job(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<transfer::JobView>> {
    ops::undo(&state, &id).await.map_err(transfer_error)?;
    state.devices.audit.record(AuditEntry {
        detail: Some(json!({ "job": id })),
        ..AuditEntry::from_ctx(&ctx, "data.undo")
    });
    ops::job(&state, &id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("job_not_found", "No such data job."))
}
