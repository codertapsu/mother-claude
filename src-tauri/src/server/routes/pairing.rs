//! Pairing and device management routes (design §2.6).
//!
//! Owner: **B1 (security/remote)** — STUBS (except the legacy pairing view).
//! State lives in `server/pairing.rs` (`state.pairing`) and
//! `server/devices.rs` (`state.devices`). After any device/pending change B1
//! calls `crate::server::ws::publish_devices`; a new internet pairing that
//! needs approval also calls `publish_pair_request`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use super::Body;
use crate::server::auth::{self, DesktopOnly};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// `POST /api/pair/codes` (desktop) `{target:"lan"|"internet", baseUrl?}` →
/// `{code, url, svg, expiresAt, target, baseUrl}`. STUB.
pub async fn create_code(
    DesktopOnly(_ctx): DesktopOnly,
    State(_state): State<AppState>,
    Body(_body): Body<Value>,
) -> ApiResult<Json<Value>> {
    Err(ApiError::not_implemented("Pairing codes (B1)"))
}

/// `POST /api/pair/exchange` (public, rate-limited) `{code, deviceName,
/// platform}` → `201 {deviceId, token, name}` | `202 {pendingId, pollSecret,
/// fingerprint}`. STUB.
pub async fn exchange(State(_state): State<AppState>, Body(_body): Body<Value>) -> ApiError {
    ApiError::not_implemented("Pairing exchange (B1)")
}

/// `GET /api/pair/pending/{pendingId}` (public, `X-Pair-Secret`) →
/// `{status, token?, deviceId?}`. STUB.
pub async fn pending_status(
    State(_state): State<AppState>,
    Path(_pending_id): Path<String>,
) -> ApiError {
    ApiError::not_implemented("Pairing approval polling (B1)")
}

/// `GET /api/devices` (desktop) → `{devices, pending}`. Foundation: the store's
/// list (empty until B1), no pending.
pub async fn list_devices(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
) -> Json<Value> {
    Json(json!({ "devices": state.devices.list(), "pending": [] }))
}

/// `POST /api/devices/pending/{id}/approve` (desktop). STUB.
pub async fn approve(DesktopOnly(_ctx): DesktopOnly, Path(_id): Path<String>) -> ApiError {
    ApiError::not_implemented("Approving pairings (B1)")
}

/// `POST /api/devices/pending/{id}/deny` (desktop). STUB.
pub async fn deny(DesktopOnly(_ctx): DesktopOnly, Path(_id): Path<String>) -> ApiError {
    ApiError::not_implemented("Denying pairings (B1)")
}

/// `DELETE /api/devices/{id}` (desktop). STUB. Must close the device's sockets
/// (`ws::close_device`) and drop its push subscriptions (`push::drop_device`).
pub async fn revoke(DesktopOnly(_ctx): DesktopOnly, Path(_id): Path<String>) -> ApiError {
    ApiError::not_implemented("Revoking devices (B1)")
}

/// `PATCH /api/devices/{id}` (desktop) `{name?, allowDangerous?}`. STUB.
pub async fn update(
    DesktopOnly(_ctx): DesktopOnly,
    Path(_id): Path<String>,
    Body(_body): Body<Value>,
) -> ApiError {
    ApiError::not_implemented("Editing devices (B1)")
}

/// Legacy `GET /api/pairing` (desktop): QR of the LAN base URL, TLS
/// fingerprint, addresses. **Never returns a token.**
pub async fn legacy_pairing(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let fingerprint = state
        .fingerprint
        .read()
        .await
        .clone()
        .unwrap_or_else(|| "n/a (http)".to_string());
    Json(auth::build_pairing(&state, &fingerprint))
}
