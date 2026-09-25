//! Web Push routes (design §4.5 "Push", §6).
//!
//! Owner: **B5 (push)** — STUBS. State lives in `server/push.rs`
//! (`state.push`). Subscriptions are per device: use
//! `ctx.device_id()` (the desktop has no device id and gets in-app alerts).

use axum::extract::State;
use axum::Json;
use serde_json::Value;

use super::Body;
use crate::server::auth::Authed;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// `GET /api/push/vapid` → `{publicKey}`. STUB.
pub async fn vapid(Authed(_ctx): Authed, State(_state): State<AppState>) -> ApiResult<Json<Value>> {
    Err(ApiError::not_implemented("Web Push keys (B5)"))
}

/// `POST /api/push/subscribe {endpoint, keys:{p256dh, auth}}`. STUB.
pub async fn subscribe(
    Authed(_ctx): Authed,
    State(_state): State<AppState>,
    Body(_body): Body<Value>,
) -> ApiResult<Json<Value>> {
    Err(ApiError::not_implemented("Web Push subscriptions (B5)"))
}

/// `DELETE /api/push/subscribe {endpoint}`. STUB.
pub async fn unsubscribe(
    Authed(_ctx): Authed,
    State(_state): State<AppState>,
    Body(_body): Body<Value>,
) -> ApiResult<Json<Value>> {
    Err(ApiError::not_implemented("Web Push subscriptions (B5)"))
}

/// `POST /api/push/test`. STUB.
pub async fn test(Authed(_ctx): Authed, State(_state): State<AppState>) -> ApiResult<Json<Value>> {
    Err(ApiError::not_implemented("Test notifications (B5)"))
}

/// `GET /api/push/prefs` → `{permission, question, plan, finished, failed}`.
/// STUB.
pub async fn get_prefs(
    Authed(_ctx): Authed,
    State(_state): State<AppState>,
) -> ApiResult<Json<Value>> {
    Err(ApiError::not_implemented("Notification preferences (B5)"))
}

/// `PUT /api/push/prefs`. STUB.
pub async fn put_prefs(
    Authed(_ctx): Authed,
    State(_state): State<AppState>,
    Body(_body): Body<Value>,
) -> ApiResult<Json<Value>> {
    Err(ApiError::not_implemented("Notification preferences (B5)"))
}
