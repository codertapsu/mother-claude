//! Moving and copying data between accounts, and the data jobs that do it
//! (docs/ACCOUNTS.md §4.4, P3.2).
//!
//! * `POST /api/data/move` (the desktop or a paired device, §13.9)
//!   `{what: "session", id} | {what: "project", cwd}` plus `{from, to, mode:
//!   "move"|"copy", trust?, dryRun?}` → `200 {plan}` for a dry run, else
//!   `202 {job}`: the transfer runs in the background as a data job.
//!   `trust: true` needs the dangerous right.
//! * `GET /api/data/jobs` and `GET /api/data/jobs/{id}` (Authed): progress,
//!   result, until when undo works, the accounts involved (`from`/`to`,
//!   `account`). The list has this run's jobs and earlier ones that can still
//!   be undone.
//! * `POST /api/data/jobs/{id}/undo` (the desktop or a paired device).
//!
//! Every refusal has its own code (`same_directory`, `session_live`,
//! `already_exists`, `directory_in_use`, `config_locked`, `session_changed`,
//! `data_job_running`, …) and every transfer and undo is audited. A job that
//! failed and could not put everything back (`transfer_incomplete`) keeps its
//! undo directory and offers Undo (`undoUntil`).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::Body;
use crate::claude::transfer::{self, ops, Mode, Request, TransferError, What};
use crate::server::auth::{self, Authed, ManageData};
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
        | TransferError::UndoConflict(_)
        | TransferError::SessionChanged(_)
        | TransferError::Busy => StatusCode::CONFLICT,
        TransferError::Io(_) | TransferError::VerifyFailed(_) | TransferError::Incomplete(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        TransferError::SameDirectory | TransferError::Invalid(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        // A backup set's refusal keeps the status a backup route gives it.
        TransferError::Backup { code, .. } => match *code {
            "storage_not_found" => StatusCode::NOT_FOUND,
            "wrong_passphrase" => StatusCode::FORBIDDEN,
            "invalid_backup" | "invalid_storage" => StatusCode::UNPROCESSABLE_ENTITY,
            "backup_io" => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::CONFLICT,
        },
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
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<MoveBody>,
) -> ApiResult<Response> {
    if body.trust {
        auth::require_dangerous_unless_dry(&ctx, "Carrying a folder's trust", body.dry_run)?;
    }
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
        refresh: false,
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
    let what = format!("started to {kind} {} conversation(s)", plan.sessions.len());
    let st = state.clone();
    let id = job_id.clone();
    tokio::spawn(async move {
        if let Err(e) = ops::execute(&st, &id, plan).await {
            tracing::warn!(job = %id, error = %e, "data transfer failed");
        }
    });
    auth::note_device_action(&state, &ctx, &what);
    let view = ops::job(&state, &job_id);
    Ok((StatusCode::ACCEPTED, Json(json!({ "job": view }))).into_response())
}

/// `GET /api/data/jobs`.
pub async fn list_jobs(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    let st = state.clone();
    let jobs = tokio::task::spawn_blocking(move || ops::list(&st))
        .await
        .unwrap_or_else(|_| state.jobs.list());
    Json(json!({ "jobs": jobs }))
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
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<transfer::JobView>> {
    ops::undo(&state, &id).await.map_err(transfer_error)?;
    state.devices.audit.record(AuditEntry {
        detail: Some(json!({ "job": id })),
        ..AuditEntry::from_ctx(&ctx, "data.undo")
    });
    auth::note_device_action(&state, &ctx, "undid a data job");
    ops::job(&state, &id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("job_not_found", "No such data job."))
}
