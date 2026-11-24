//! Plan usage (design2 §3): the claude.ai plan's 5-hour and weekly limits,
//! served from the cache `claude::usage` keeps (never a read per request).
//!
//! * `GET /api/usage` (Authed) → `PlanUsage`, plus `accounts: {id: PlanUsage}`
//!   for every account but the Default (docs/ACCOUNTS.md §5.6).
//! * `POST /api/usage/refresh {account?}` (Authed) → `202 PlanUsage` with
//!   `refreshing: true` (the result arrives as a `planUsage` frame), or `429`
//!   with `Retry-After` within a minute of the last read.
//! * `PUT /api/usage/settings {intervalSecs}` (DesktopOnly) → `PlanUsage`;
//!   `422` for anything but 0 / 180 / 300 / 600.
//!
//! The usage guard (`claude::usage_guard`, Settings › Usage limits):
//!
//! | Route | Who | Does |
//! |---|---|---|
//! | `GET /api/usage-guard` | Authed | `{settings, pauses}` |
//! | `PUT /api/usage-guard/settings` `{enabled?, thresholdPct?, autoResume?, bufferSecs?, includeElsewhere?}` | ManageData | the new view (`422 invalid_settings`) |
//! | `POST /api/usage-guard/resume` `{pause?}` | ManageData | *Resume now*: that pause, or every active one |
//! | `POST /api/usage-guard/pauses/{id}/hold` `{hold}` | ManageData | *Don't resume* (or let it resume again) |
//! | `DELETE /api/usage-guard/pauses/{id}` | ManageData | forget it without resuming |
//!
//! Each change is audited, told to the desktop when a device made it, and to
//! every client as `{t:"usageGuard"}`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::Body;
use crate::claude::model::PlanUsage;
use crate::claude::usage;
use crate::claude::usage_guard::{self, GuardView, SettingsPatch};
use crate::server::auth::{note_device_action, Authed, DesktopOnly, ManageData};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// `GET /api/usage`: the Default account's usage as always, plus every other
/// account's under `accounts` (absent while only the Default exists).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageResponse {
    #[serde(flatten)]
    pub usage: PlanUsage,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, PlanUsage>,
}

/// `GET /api/usage`.
pub async fn get_usage(Authed(_ctx): Authed, State(state): State<AppState>) -> Json<UsageResponse> {
    Json(UsageResponse {
        usage: usage::current(&state),
        accounts: usage::others(&state),
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RefreshBody {
    /// Whose usage (default: the Default account's).
    pub account: Option<String>,
}

/// `POST /api/usage/refresh {account?}`.
pub async fn refresh(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Body(body): Body<RefreshBody>,
) -> ApiResult<Response> {
    let account = body
        .account
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or(crate::claude::accounts::DEFAULT_ID);
    if state.accounts.get(account).is_none() {
        return Err(ApiError::not_found(
            "account_not_found",
            format!("No account {account:?}."),
        ));
    }
    match usage::request_refresh_for(&state, account) {
        Ok(u) => Ok((StatusCode::ACCEPTED, Json(u)).into_response()),
        Err(wait) => Err(ApiError::too_many_requests(
            wait,
            "Plan usage was read less than a minute ago.",
        )),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsBody {
    pub interval_secs: u32,
}

/// `PUT /api/usage/settings {intervalSecs}`.
pub async fn put_settings(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<SettingsBody>,
) -> ApiResult<Json<PlanUsage>> {
    usage::set_interval(&state, body.interval_secs)
        .map(Json)
        .map_err(|m| ApiError::unprocessable("invalid_interval", m))
}

// ───────────────────────────── the usage guard ─────────────────────────────

fn guard_changed(state: &AppState, ctx: &ClientContext, action: &str, detail: Value, what: &str) {
    state.devices.audit.record(AuditEntry {
        detail: Some(detail),
        ..AuditEntry::from_ctx(ctx, action)
    });
    note_device_action(state, ctx, what);
}

/// A pause's id as the guard makes them (`up_` and 12 hex digits).
fn pause_id(id: &str) -> ApiResult<&str> {
    let ok = id
        .strip_prefix("up_")
        .is_some_and(|h| h.len() == 12 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    if ok {
        Ok(id)
    } else {
        Err(no_pause())
    }
}

fn no_pause() -> ApiError {
    ApiError::not_found("pause_not_found", "No such pause.")
}

/// `GET /api/usage-guard`.
pub async fn get_guard(Authed(_ctx): Authed, State(state): State<AppState>) -> Json<GuardView> {
    Json(usage_guard::view(&state))
}

/// `PUT /api/usage-guard/settings`.
pub async fn put_guard_settings(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(patch): Body<SettingsPatch>,
) -> ApiResult<Json<GuardView>> {
    let view = usage_guard::set_settings(&state, &patch)
        .map_err(|m| ApiError::unprocessable("invalid_settings", m))?;
    let s = &view.settings;
    guard_changed(
        &state,
        &ctx,
        "usage.guard.settings",
        json!({
            "enabled": s.enabled,
            "thresholdPct": s.threshold_pct,
            "autoResume": s.auto_resume,
            "bufferSecs": s.buffer_secs,
            "includeElsewhere": s.include_elsewhere,
        }),
        "changed the usage limit settings",
    );
    Ok(Json(view))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ResumeBody {
    /// That pause (default: every active one).
    pub pause: Option<String>,
}

/// `POST /api/usage-guard/resume {pause?}`.
pub async fn post_guard_resume(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<ResumeBody>,
) -> ApiResult<Json<GuardView>> {
    let pause = body.pause.as_deref().map(pause_id).transpose()?;
    let view = usage_guard::resume_now(&state, pause)
        .await
        .map_err(|_| no_pause())?;
    guard_changed(
        &state,
        &ctx,
        "usage.guard.resume",
        json!({ "pause": pause }),
        "resumed sessions paused near a usage limit",
    );
    Ok(Json(view))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HoldBody {
    pub hold: bool,
}

/// `POST /api/usage-guard/pauses/{id}/hold {hold}`.
pub async fn post_guard_hold(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<HoldBody>,
) -> ApiResult<Json<GuardView>> {
    let id = pause_id(&id)?;
    let view = usage_guard::set_hold(&state, id, body.hold).map_err(|_| no_pause())?;
    guard_changed(
        &state,
        &ctx,
        "usage.guard.hold",
        json!({ "pause": id, "hold": body.hold }),
        if body.hold {
            "kept a usage pause from resuming by itself"
        } else {
            "let a usage pause resume by itself"
        },
    );
    Ok(Json(view))
}

/// `DELETE /api/usage-guard/pauses/{id}`.
pub async fn delete_guard_pause(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<GuardView>> {
    let id = pause_id(&id)?;
    let view = usage_guard::dismiss(&state, id).map_err(|_| no_pause())?;
    guard_changed(
        &state,
        &ctx,
        "usage.guard.dismiss",
        json!({ "pause": id }),
        "dismissed a usage pause without resuming it",
    );
    Ok(Json(view))
}
