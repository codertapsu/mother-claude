//! Remote-access routes (design §2.8). State lives in `server/remote.rs`
//! (`state.remote`).

use axum::extract::State;
use axum::Json;
use serde_json::Value;

use super::Body;
use crate::server::auth::DesktopOnly;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::ApiResult;
use crate::state::AppState;

/// `GET /api/remote` (desktop) → `{lan, internet, bridge, devices}`.
pub async fn get_remote(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    Ok(Json(crate::server::remote::status(&state).await))
}

/// `PUT /api/remote` (desktop) `{internetEnabled?, manualUrl?,
/// requireApproval?, bindTailscale?, deviceIdleDays?}` → the new status.
pub async fn put_remote(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<Value>,
) -> ApiResult<Json<Value>> {
    let out = crate::server::remote::update(&state, &body).await?;
    state.devices.audit.record(AuditEntry {
        detail: Some(body),
        ..AuditEntry::from_ctx(&ctx, "remote.update")
    });
    Ok(Json(out))
}
