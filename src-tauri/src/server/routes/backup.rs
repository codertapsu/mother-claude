//! Backups, restores and pulls (docs/ACCOUNTS.md §4.4, P4.3, P5.1).
//!
//! * `GET /api/backup` (Authed): the settings, whether a passphrase is set,
//!   each account's set, folder suggestions (iCloud Drive, Google Drive…), the
//!   recent jobs.
//! * `PUT /api/backup/settings {folder?, everyHours?, pullOnLaunch?}` (the
//!   desktop or a paired device, docs/ACCOUNTS.md §13.9).
//! * `POST /api/backup/passphrase {generate: true}` → `{passphrase, rekeyed,
//!   left}` shown this once, or `{passphrase}` of the user's own
//!   (DesktopOnly). Every set in the folder is re-keyed, all or nothing as
//!   far as it can be (`ops::set_passphrase`); the passphrase is kept in the
//!   Keychain. A failure after some set has it already carries
//!   `error.applied: true` and, for a generated one, `error.passphrase`.
//! * `POST /api/backup/push {account?}` (desktop or device) → `202 {jobs}`; without
//!   `account`, one job per set (a token account's is the Default's); a set
//!   being backed up already gives that job's id.
//! * `GET /api/backup/index?account=&into=` (Authed): the set's sessions
//!   compared with an account's directory, each with what a pull or restore
//!   would do and the pruning warning; `?storage=<id>&set=<name>&into=` the
//!   same for a set of any storage (docs/ACCOUNTS.md §13.4), its items saying
//!   which are in storage only (`archived`).
//! * `POST /api/backup/storage {path, label?}` → `201` the storage;
//!   `PATCH /api/backup/storage/{id} {label}` → the storage;
//!   `DELETE /api/backup/storage/{id}` → `204` (forgotten, its files stay)
//!   (desktop or device).
//! * `POST /api/backup/restore {account, into?, ids, trust?}` → `202 {job}`, or
//!   `{account, config: true}` → `{folder}` (the configuration bundle unpacked
//!   for review, never over the live one) (desktop or device; `trust: true`
//!   needs the dangerous right).
//! * `POST /api/backup/pull {account?, ids?, trust?}` (desktop or device;
//!   `trust: true` needs the dangerous right) →
//!   `202 {jobs, skipped: [{account, code, message}]}`: without `account`,
//!   every set, a set that cannot be read named in `skipped` and the rest
//!   pulled.
//!
//! The passphrase stays the desktop's (`POST /api/backup/passphrase`), and
//! never reaches a log, a frame or the audit line.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::Body;
use crate::claude::backup::{ops, BackupError};
use crate::claude::secrets::Secret;
use crate::server::auth::{self, Authed, DesktopOnly, ManageData};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

pub(crate) fn backup_error(e: BackupError) -> ApiError {
    let status = match &e {
        BackupError::NoFolder
        | BackupError::NoPassphrase
        | BackupError::Tampered(_)
        | BackupError::KeyMissing(_)
        | BackupError::StorageUnavailable(_) => StatusCode::CONFLICT,
        BackupError::WrongPassphrase => StatusCode::FORBIDDEN,
        BackupError::StorageNotFound(_) => StatusCode::NOT_FOUND,
        BackupError::Invalid(_) | BackupError::InvalidStorage(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
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
    ManageData(ctx): ManageData,
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
        // Never inside (or around) another storage (§13.3 rule 8).
        if let Some(f) = &s.folder {
            if let Some(other) = s
                .storages
                .iter()
                .find(|o| f.starts_with(&o.path) || o.path.starts_with(f))
            {
                return Err(ApiError::unprocessable(
                    "invalid_folder",
                    format!(
                        "{} is inside (or around) the storage {:?}",
                        f.display(),
                        other.label
                    ),
                ));
            }
        }
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
    auth::note_device_action(&state, &ctx, "changed the backup settings");
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
    let result = tokio::task::spawn_blocking(move || ops::set_passphrase(&st, &new))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let change = match result {
        Ok(change) => change,
        Err(failure) => {
            audit(
                &state,
                &ctx,
                "backup.passphrase.failed",
                json!({ "generated": shown.is_some(), "applied": failure.applied }),
            );
            let err = backup_error(failure.error).with("applied", failure.applied);
            // Some set has it already: a generated one is never lost.
            return Err(match (failure.applied, shown) {
                (true, Some(p)) => err.with("passphrase", p),
                _ => err,
            });
        }
    };
    audit(
        &state,
        &ctx,
        "backup.passphrase",
        json!({ "generated": shown.is_some(), "rekeyed": change.rekeyed.len() }),
    );
    // Shown this once; it is in the Keychain from now on.
    Ok(Json(json!({
        "passphrase": shown,
        "rekeyed": change.rekeyed,
        "left": change.left,
    }))
    .into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PushBody {
    #[serde(default)]
    pub account: Option<String>,
}

/// `POST /api/backup/push`.
pub async fn post_push(
    ManageData(ctx): ManageData,
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
    auth::note_device_action(&state, &ctx, "started a backup");
    Ok((StatusCode::ACCEPTED, Json(json!({ "jobs": jobs }))).into_response())
}

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub into: Option<String>,
    /// A set of any storage (with `set`; `backup` when only `set` is given).
    #[serde(default)]
    pub storage: Option<String>,
    #[serde(default)]
    pub set: Option<String>,
}

/// `GET /api/backup/index`.
pub async fn get_index(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Query(q): Query<IndexQuery>,
) -> ApiResult<Json<ops::IndexView>> {
    let result = match (&q.storage, &q.set, &q.account) {
        (storage, Some(set), _) => {
            let storage = storage
                .as_deref()
                .unwrap_or(crate::claude::accounts::BACKUP_STORAGE_ID);
            ops::index_of_set(&state, storage, set, q.into.as_deref()).await
        }
        (Some(_), None, _) => {
            return Err(ApiError::unprocessable(
                "invalid_query",
                "Name the set of the storage (`set`).",
            ))
        }
        (None, None, Some(account)) => ops::index(&state, account, q.into.as_deref()).await,
        (None, None, None) => {
            return Err(ApiError::unprocessable(
                "invalid_query",
                "Name an account (`account`), or a storage and a set (`storage`, `set`).",
            ))
        }
    };
    result.map(Json).map_err(backup_error)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageBody {
    pub path: std::path::PathBuf,
    #[serde(default)]
    pub label: Option<String>,
}

/// A storage's view, as `GET /api/backup` lists it.
fn storage_view(
    state: &AppState,
    s: &crate::claude::backup::storage::Storage,
) -> serde_json::Value {
    let owners = ops::set_owner_names(state);
    serde_json::to_value(crate::claude::backup::storage::view_of(s, &owners))
        .unwrap_or(serde_json::Value::Null)
}

/// `POST /api/backup/storage`.
pub async fn post_storage(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<StorageBody>,
) -> ApiResult<Response> {
    let st = state.clone();
    let added = tokio::task::spawn_blocking(move || {
        crate::claude::backup::storage::add(&st, &body.path, body.label.as_deref())
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(backup_error)?;
    audit(
        &state,
        &ctx,
        "backup.storage.add",
        json!({ "storage": added.id, "label": added.label, "path": added.path }),
    );
    auth::note_device_action(
        &state,
        &ctx,
        &format!("added the storage {:?}", added.label),
    );
    let view = storage_view(&state, &added);
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageLabelBody {
    pub label: String,
}

/// `PATCH /api/backup/storage/{id}`.
pub async fn patch_storage(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Body(body): Body<StorageLabelBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let s =
        crate::claude::backup::storage::rename(&state, &id, &body.label).map_err(backup_error)?;
    audit(
        &state,
        &ctx,
        "backup.storage.rename",
        json!({ "storage": s.id, "label": s.label }),
    );
    auth::note_device_action(&state, &ctx, &format!("renamed the storage {:?}", s.label));
    let st = state.clone();
    let view = tokio::task::spawn_blocking(move || storage_view(&st, &s))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(view))
}

/// `DELETE /api/backup/storage/{id}`: forgotten; its files stay.
pub async fn delete_storage(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<StatusCode> {
    let gone = crate::claude::backup::storage::remove(&state, &id).map_err(backup_error)?;
    audit(
        &state,
        &ctx,
        "backup.storage.remove",
        json!({ "storage": gone.id, "label": gone.label, "path": gone.path }),
    );
    auth::note_device_action(
        &state,
        &ctx,
        &format!("removed the storage {:?}", gone.label),
    );
    Ok(StatusCode::NO_CONTENT)
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
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<RestoreBody>,
) -> ApiResult<Response> {
    if body.trust && !body.config {
        auth::require_dangerous(&ctx, "Restoring a folder's trust")?;
    }
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
        auth::note_device_action(&state, &ctx, "unpacked a configuration bundle for review");
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
    auth::note_device_action(
        &state,
        &ctx,
        &format!("started to restore {} conversation(s)", body.ids.len()),
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
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<PullBody>,
) -> ApiResult<Response> {
    if body.trust {
        auth::require_dangerous(&ctx, "Pulling folders' trust")?;
    }
    // Everything: each set once (a token account's is the Default's).
    let accounts: Vec<String> = match &body.account {
        Some(a) => vec![a.clone()],
        None => ops::set_owner_ids(&state),
    };
    let mut jobs = Vec::new();
    let mut skipped = Vec::new();
    for a in &accounts {
        match ops::pull(&state, a, body.ids.clone(), body.trust).await {
            Ok(j) => jobs.push(j),
            // No folder or passphrase: nothing can be pulled at all.
            Err(e @ (BackupError::NoFolder | BackupError::NoPassphrase)) => {
                return Err(backup_error(e))
            }
            // Pulling everything: a set that cannot be read (none, its key
            // missing…) is named and the others go ahead. A wrong passphrase
            // or an unreadable index shows on that set's job.
            Err(e) if body.account.is_none() => skipped.push(json!({
                "account": a,
                "code": e.code(),
                "message": e.to_string(),
            })),
            Err(e) => return Err(backup_error(e)),
        }
    }
    audit(
        &state,
        &ctx,
        "backup.pull",
        json!({ "account": body.account, "jobs": jobs, "trust": body.trust }),
    );
    auth::note_device_action(&state, &ctx, "started a pull");
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "jobs": jobs, "skipped": skipped })),
    )
        .into_response())
}
