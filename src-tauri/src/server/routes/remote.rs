//! Remote-access routes (design §2.8).
//!
//! Owner: **B1 (security/remote)** — STUBS. State lives in
//! `server/remote.rs` (`state.remote`).

use axum::extract::State;
use axum::Json;
use serde_json::Value;

use super::Body;
use crate::server::auth::DesktopOnly;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// `GET /api/remote` (desktop) → `{lan, internet, bridge}`. STUB.
pub async fn get_remote(
    DesktopOnly(_ctx): DesktopOnly,
    State(_state): State<AppState>,
) -> ApiResult<Json<Value>> {
    Err(ApiError::not_implemented("Remote access status (B1)"))
}

/// `PUT /api/remote` (desktop) `{internetEnabled?, manualUrl?,
/// requireApproval?}`. STUB.
pub async fn put_remote(
    DesktopOnly(_ctx): DesktopOnly,
    State(_state): State<AppState>,
    Body(_body): Body<Value>,
) -> ApiResult<Json<Value>> {
    Err(ApiError::not_implemented("Remote access settings (B1)"))
}
