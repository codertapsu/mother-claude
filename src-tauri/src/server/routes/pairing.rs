//! Pairing and device management routes (design §2.6, §10 B1).
//!
//! State lives in `server/pairing.rs` (`state.pairing`) and
//! `server/devices.rs` (`state.devices`). After any device/pending change the
//! desktop gets a `devices` frame (`ws::publish_devices`); a pairing that needs
//! approval also sends `pairRequest` (without the fingerprint or poll secret).

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::Body;
use crate::server::auth::{self, Authed, DesktopOnly};
use crate::server::context::ClientContext;
use crate::server::devices::{self, audit::AuditEntry};
use crate::server::error::{ApiError, ApiResult};
use crate::server::pairing::{self as pair, CodeRequest, ExchangeRequest};
use crate::state::AppState;

/// Largest body accepted on `/api/pair/*`.
pub const PAIR_BODY_MAX: usize = 4 * 1024;

/// `POST /api/pair/codes` `{target:"lan"|"internet"|"self", baseUrl?}` →
/// `{code, display, url, svg, expiresAt, ttlSec, target, baseUrl}`. `lan` and
/// `internet` are desktop-only; `self` is for a paired device (a second token
/// for its own Home Screen app).
pub async fn create_code(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Body(body): Body<CodeRequest>,
) -> ApiResult<Json<Value>> {
    if !pair::may_create(&ctx.principal, body.target) {
        return Err(ApiError::forbidden(
            "desktop_only",
            "Pairing codes are created on the Mother Claude desktop app (a paired device may \
             only create a code for itself).",
        ));
    }
    pair::create_code(&state, &ctx, body).map(Json)
}

/// `POST /api/pair/exchange` (public, rate-limited) `{code, deviceName,
/// platform}` → `201 {deviceId, token, name}` | `202 {pendingId, pollSecret,
/// fingerprint, expiresAt}`.
pub async fn exchange(
    State(state): State<AppState>,
    ctx: ClientContext,
    headers: HeaderMap,
    Body(body): Body<ExchangeRequest>,
) -> ApiResult<Response> {
    let (status, value) = pair::exchange(&state, &ctx, &headers, body)?;
    Ok((status, no_store(), Json(value)).into_response())
}

fn no_store() -> [(header::HeaderName, &'static str); 1] {
    [(header::CACHE_CONTROL, "no-store")]
}

/// `GET /api/pair/pending/{pendingId}` (public, `X-Pair-Secret`) →
/// `{status: pending|approved|denied|expired|contested, token?, deviceId?}`;
/// the token is returned exactly once.
pub async fn pending_status(
    State(state): State<AppState>,
    ctx: ClientContext,
    headers: HeaderMap,
    Path(pending_id): Path<String>,
) -> ApiResult<Response> {
    let secret = headers
        .get("x-pair-secret")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let v = pair::poll(&state, &ctx, &pending_id, secret)?;
    Ok((StatusCode::OK, no_store(), Json(v)).into_response())
}

/// `GET /api/devices` (desktop) → `{devices, pending}` (never a token, a hash,
/// a fingerprint or a poll secret).
pub async fn list_devices(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
) -> Json<Value> {
    let devices: Vec<_> = state
        .devices
        .list()
        .into_iter()
        .map(|mut d| {
            d.sockets = state.hub.device_sockets(&d.id);
            d
        })
        .collect();
    Json(json!({
        "devices": devices,
        "pending": state.pairing.pending_views(),
        "idleDays": state.devices.idle_days(),
    }))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApproveBody {
    #[serde(default)]
    pub fingerprint: String,
}

/// `POST /api/devices/pending/{id}/approve {fingerprint}` (desktop): the 4
/// digits the phone shows must be typed here.
pub async fn approve(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<ApproveBody>,
) -> ApiResult<Json<Value>> {
    pair::approve(&state, &ctx, &id, &body.fingerprint).map(Json)
}

/// `POST /api/devices/pending/{id}/deny` (desktop).
pub async fn deny(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    pair::deny(&state, &ctx, &id).map(Json)
}

/// `DELETE /api/devices/{id}` (desktop): revoke (closes its sockets, drops its
/// push subscriptions). Deleting an already revoked device forgets it.
pub async fn revoke(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let Some(device) = state.devices.get(&id) else {
        return Err(ApiError::not_found("not_found", "No such device."));
    };
    if device.revoked {
        state.devices.forget(&id);
        crate::server::ws::publish_devices(&state);
        return Ok(Json(json!({ "ok": true, "forgotten": true })));
    }
    devices::revoke(&state, &id, "revoked");
    Ok(Json(json!({ "ok": true, "revoked": true })))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateBody {
    pub name: Option<String>,
    pub allow_dangerous: Option<bool>,
    pub allow_dangerous_internet: Option<bool>,
}

/// `PATCH /api/devices/{id}` (desktop) `{name?, allowDangerous?,
/// allowDangerousInternet?}`.
pub async fn update(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<UpdateBody>,
) -> ApiResult<Json<Value>> {
    if state.devices.get(&id).is_none_or(|d| d.revoked) {
        return Err(ApiError::not_found("not_found", "No such device."));
    }
    if let Some(name) = &body.name {
        state.devices.rename(&id, name);
    }
    let granted = body.allow_dangerous.is_some() || body.allow_dangerous_internet.is_some();
    let device = if granted {
        state
            .devices
            .set_grants(&id, body.allow_dangerous, body.allow_dangerous_internet)
    } else {
        state.devices.get(&id)
    }
    .ok_or_else(|| ApiError::not_found("not_found", "No such device."))?;
    if granted {
        state.devices.audit.record(AuditEntry {
            device_id: Some(id.clone()),
            device_name: Some(device.name.clone()),
            detail: Some(json!({
                "allowDangerous": device.allow_dangerous,
                "allowDangerousInternet": device.allow_dangerous_internet,
            })),
            ..AuditEntry::from_ctx(&ctx, "device.grants")
        });
    }
    crate::server::ws::publish_devices(&state);
    Ok(Json(json!({ "ok": true, "device": device })))
}

/// `DELETE /api/me` (a paired device): unpair itself.
pub async fn unpair_self(
    Authed(ctx): Authed,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    let Some(id) = ctx.device_id().map(str::to_string) else {
        return Err(ApiError::bad_request(
            "Only a paired device can unpair itself; manage devices in Remote access.",
        ));
    };
    devices::revoke(&state, &id, "unpaired");
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/pair/ca.crt` (public): the local CA certificate (DER) a phone can
/// install to trust the LAN listeners. Only the public certificate.
pub async fn ca_cert(State(state): State<AppState>) -> Response {
    match state.remote.lan().ca_der {
        Some(der) => (
            [
                (header::CONTENT_TYPE, "application/x-x509-ca-cert"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"mother-claude-ca.crt\"",
                ),
            ],
            der,
        )
            .into_response(),
        None => ApiError::not_found(
            "no_local_ca",
            "Mother Claude is not serving the local network, so there is no certificate.",
        )
        .into_response(),
    }
}

/// `GET /api/pair/ca.mobileconfig` (public): an iOS profile installing the CA.
pub async fn ca_mobileconfig(State(state): State<AppState>) -> Response {
    match state.remote.lan().ca_der {
        Some(der) => (
            [
                (header::CONTENT_TYPE, "application/x-apple-aspen-config"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"mother-claude.mobileconfig\"",
                ),
            ],
            crate::server::tls::mobileconfig(&der),
        )
            .into_response(),
        None => ApiError::not_found(
            "no_local_ca",
            "Mother Claude is not serving the local network, so there is no certificate.",
        )
        .into_response(),
    }
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
