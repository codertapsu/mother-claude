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

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Body;
use crate::claude::model::PlanUsage;
use crate::claude::usage;
use crate::server::auth::{Authed, DesktopOnly};
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
