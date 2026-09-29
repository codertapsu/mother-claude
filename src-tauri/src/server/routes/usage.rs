//! Plan usage (design2 §3): the claude.ai plan's 5-hour and weekly limits,
//! served from the cache `claude::usage` keeps (never a read per request).
//!
//! * `GET /api/usage` (Authed) → `PlanUsage`.
//! * `POST /api/usage/refresh` (Authed) → `202 PlanUsage` with
//!   `refreshing: true` (the result arrives as a `planUsage` frame), or `429`
//!   with `Retry-After` within a minute of the last read.
//! * `PUT /api/usage/settings {intervalSecs}` (DesktopOnly) → `PlanUsage`;
//!   `422` for anything but 0 / 180 / 300 / 600.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use super::Body;
use crate::claude::model::PlanUsage;
use crate::claude::usage;
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// `GET /api/usage`.
pub async fn get_usage(Authed(_ctx): Authed, State(state): State<AppState>) -> Json<PlanUsage> {
    Json(usage::current(&state))
}

/// `POST /api/usage/refresh`.
pub async fn refresh(Authed(_ctx): Authed, State(state): State<AppState>) -> ApiResult<Response> {
    match usage::request_refresh(&state) {
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
