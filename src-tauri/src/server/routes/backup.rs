//! Backups, restores and pulls (docs/ACCOUNTS.md §4.4, P4.3, P5.1).
//!
//! * `GET /api/backup` (Authed): the settings, whether a passphrase is set,
//!   each account's set, folder suggestions (iCloud Drive, Google Drive…), the
//!   recent jobs.
//! * `PUT /api/backup/settings {folder?, everyHours?, pullOnLaunch?}`
//!   (DesktopOnly).
//! * `POST /api/backup/passphrase {generate: true}` → `{passphrase}` shown
//!   this once, or `{passphrase}` of the user's own (DesktopOnly). Existing
//!   sets are re-keyed; the passphrase is kept in the Keychain.
//! * `POST /api/backup/push {account?}` (DesktopOnly) → `202 {jobs}`.
//! * `GET /api/backup/index?account=&into=` (Authed): the set's sessions
//!   compared with an account's directory, each with what a pull or restore
//!   would do and the pruning warning.
//! * `POST /api/backup/restore {account, into?, ids, trust?}` → `202 {job}`, or
//!   `{account, config: true}` → `{folder}` (the configuration bundle unpacked
//!   for review, never over the live one) (DesktopOnly).
//! * `POST /api/backup/pull {account?, ids?, trust?}` (DesktopOnly) →
//!   `202 {jobs}`.
//!
//! The passphrase never reaches a log, a frame or the audit line.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::Body;
use crate::claude::backup::{ops, BackupError};
use crate::claude::secrets::Secret;
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

fn backup_error(e: BackupError) -> ApiError {
    let status = match &e {
        BackupError::NoFolder
        | BackupError::NoPassphrase
        | BackupError::Tampered(_)
        | BackupError::KeyMissing(_) => StatusCode::CONFLICT,
        BackupError::WrongPassphrase => StatusCode::FORBIDDEN,
        BackupError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
        BackupError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        BackupError::Transfer(t) => {
            return crate::server::routes::transfers::transfer_error(t.clone())
        }
    };
    ApiError::new(status, e.code(), e.to_string())
}

fn audit(state: &AppState, ctx: &ClientContext, action: &str, detail: serde_json::Value) {
    state.devices.audit.record(AuditEntry {
        detail: Some(detail),
        ..AuditEntry::from_ctx(ctx, action)
    });
}

/// `GET /api/backup`.
pub async fn get_backup(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
) -> Json<ops::BackupView> {
    let st = state.clone();
    Json(
        tokio::task::spawn_blocking(move || ops::view(&st))
            .await
            .unwrap_or_else(|_| ops::view(&state)),
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsBody {
    #[serde(default)]
    pub folder: Option<Option<String>>,
    #[serde(default)]
    pub every_hours: Option<u32>,
    #[serde(default)]
    pub pull_on_launch: Option<bool>,
}

/// `PUT /api/backup/settings`.
pub async fn put_settings(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<SettingsBody>,
) -> ApiResult<Json<ops::BackupView>> {
    let mut s = state.accounts.backup_settings();
    if let Some(folder) = body.folder {
        s.folder = folder
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .map(|f| crate::claude::accounts::normalize_dir(std::path::Path::new(&f)))
            .transpose()
            .map_err(|e| ApiError::unprocessable("invalid_folder", e.to_string()))?;
    }
    if let Some(h) = body.every_hours {
        if h > 24 * 7 {
            return Err(ApiError::unprocessable(
                "invalid_schedule",
                "Back up at least once a week (everyHours ≤ 168), or 0 for by hand only.",
            ));
        }
        s.every_hours = h;
    }
    if let Some(p) = body.pull_on_launch {
        s.pull_on_launch = p;
    }
    state
        .accounts
        .set_backup_settings(s.clone())
        .map_err(|e| ApiError::unprocessable(e.code(), e.to_string()))?;
    audit(
        &state,
        &ctx,
        "backup.settings",
        json!({ "folder": s.folder, "everyHours": s.every_hours, "pullOnLaunch": s.pull_on_launch }),
    );
    Ok(Json(ops::view(&state)))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PassphraseBody {
    #[serde(default)]
    pub generate: bool,
    #[serde(default)]
    pub passphrase: Option<String>,
}

/// `POST /api/backup/passphrase`.
pub async fn post_passphrase(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<PassphraseBody>,
) -> ApiResult<Response> {
    let (new, shown) = match (body.generate, body.passphrase) {
        (true, None) => {
            let p = ops::generate_passphrase();
            (Secret::new(p.clone()), Some(p))
        }
        (false, Some(p)) => (Secret::new(p), None),
        _ => {
            return Err(ApiError::unprocessable(
                "invalid_body",
                "Send {\"generate\": true} or {\"passphrase\": \"…\"}.",
            ))
        }
    };
    let st = state.clone();
    tokio::task::spawn_blocking(move || ops::set_passphrase(&st, &new))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(backup_error)?;
    audit(
        &state,
        &ctx,
        "backup.passphrase",
        json!({ "generated": shown.is_some() }),
    );
    // Shown this once; it is in the Keychain from now on.
    Ok(Json(json!({ "passphrase": shown })).into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PushBody {
    #[serde(default)]
    pub account: Option<String>,
}

/// `POST /api/backup/push`.
pub async fn post_push(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<PushBody>,
) -> ApiResult<Response> {
    let jobs = match &body.account {
        Some(a) => vec![ops::push(&state, a).await.map_err(backup_error)?],
        None => ops::push_all(&state).await.map_err(backup_error)?,
    };
    audit(
        &state,
        &ctx,
        "backup.push",
        json!({ "account": body.account, "jobs": jobs }),
    );
    Ok((StatusCode::ACCEPTED, Json(json!({ "jobs": jobs }))).into_response())
}

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    pub account: String,
    #[serde(default)]
    pub into: Option<String>,
}

/// `GET /api/backup/index`.
pub async fn get_index(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Query(q): Query<IndexQuery>,
) -> ApiResult<Json<ops::IndexView>> {
    ops::index(&state, &q.account, q.into.as_deref())
        .await
        .map(Json)
        .map_err(backup_error)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreBody {
    pub account: String,
    #[serde(default)]
    pub into: Option<String>,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub trust: bool,
    #[serde(default)]
    pub config: bool,
}

/// `POST /api/backup/restore`.
pub async fn post_restore(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<RestoreBody>,
) -> ApiResult<Response> {
    if body.config {
        let folder = ops::extract_config(&state, &body.account)
            .await
            .map_err(backup_error)?;
        audit(
            &state,
            &ctx,
            "backup.restore.config",
            json!({ "account": body.account }),
        );
        return Ok(Json(json!({ "folder": folder })).into_response());
    }
    if body.ids.is_empty() || body.ids.len() > 10_000 {
        return Err(ApiError::unprocessable(
            "invalid_body",
            "Name the sessions to restore (`ids`).",
        ));
    }
    let into = body.into.clone().unwrap_or_else(|| body.account.clone());
    let job = ops::restore(&state, &body.account, &into, body.ids.clone(), body.trust)
        .await
        .map_err(backup_error)?;
    audit(
        &state,
        &ctx,
        "backup.restore",
        json!({ "account": body.account, "into": into, "sessions": body.ids.len(),
                "trust": body.trust, "job": job }),
    );
    Ok((StatusCode::ACCEPTED, Json(json!({ "job": job }))).into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PullBody {
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    #[serde(default)]
    pub trust: bool,
}

/// `POST /api/backup/pull`.
pub async fn post_pull(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<PullBody>,
) -> ApiResult<Response> {
    let accounts: Vec<String> = match &body.account {
        Some(a) => vec![a.clone()],
        None => state
            .accounts
            .accounts()
            .into_iter()
            .map(|a| a.id)
            .collect(),
    };
    let mut jobs = Vec::new();
    let mut skipped = Vec::new();
    for a in &accounts {
        match ops::pull(&state, a, body.ids.clone(), body.trust).await {
            Ok(j) => jobs.push(j),
            // Pulling everything: an account without a set has nothing to pull.
            Err(BackupError::Invalid(m)) if body.account.is_none() => skipped.push(m),
            Err(e) => return Err(backup_error(e)),
        }
    }
    audit(
        &state,
        &ctx,
        "backup.pull",
        json!({ "account": body.account, "jobs": jobs, "trust": body.trust }),
    );
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "jobs": jobs, "skipped": skipped })),
    )
        .into_response())
}
