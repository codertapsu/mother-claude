//! The `/api` route table (design §4.5) — every endpoint, with its guard.
//!
//! Owner: foundation. Handlers live in per-area files owned by the agents:
//! `data.rs` (B2), `control.rs` (B3, dispatchers by the foundation),
//! `foreign.rs` (B4), `pairing.rs` + `remote.rs` (B1), `push.rs` (B5).
//! Agents fill in handler BODIES; if a new route is truly needed, add it here
//! in one line (merge hazard: keep the table sorted by area as it is).
//!
//! Guards: public routes are listed in `public`; everything in `protected`
//! additionally passes `auth::require_api_principal` (desktop or device; the
//! sidecar only on its one route). Each handler still names its own guard
//! extractor (`Authed`, `DesktopOnly`, `DangerousOk`, `SidecarOnly`).

pub mod accounts;
pub mod backup;
pub mod control;
pub mod data;
pub mod disk;
pub mod foreign;
pub mod pairing;
pub mod push;
pub mod remote;
pub mod transfers;
pub mod uploads;
pub mod usage;

use axum::extract::{DefaultBodyLimit, FromRequest, Request, State};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::de::DeserializeOwned;

use super::auth::{self, Authed};
use super::error::ApiError;
use crate::state::AppState;

/// JSON body extractor whose rejections use the API error envelope. An empty
/// body deserializes as `{}` so optional-body POSTs work without a payload.
pub struct Body<T>(pub T);

impl<S, T> FromRequest<S> for Body<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|e| {
                if e.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::new(
                        e.status(),
                        "payload_too_large",
                        "The request body is too large for this endpoint.",
                    )
                } else {
                    ApiError::bad_request(format!("Could not read the body: {e}"))
                }
            })?;
        let slice: &[u8] = if bytes.iter().all(u8::is_ascii_whitespace) {
            b"{}"
        } else {
            &bytes
        };
        serde_json::from_slice::<T>(slice)
            .map(Body)
            .map_err(|e| ApiError::unprocessable("invalid_body", format!("Invalid JSON body: {e}")))
    }
}

/// `GET /api/me`.
pub async fn me(Authed(ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    Json(auth::me(&state, &ctx))
}

/// Largest body for routes that carry images (up to 10 × 5 MB, base64) and
/// for hook payloads (a PostToolUse carries the whole tool output).
pub const IMAGE_BODY_MAX: usize = 64 * 1024 * 1024;

/// Default body limit of every other `/api` route (JSON commands are small;
/// routes that carry images, files or tool inputs raise it explicitly).
pub const API_BODY_MAX: usize = 64 * 1024;

/// `/bridge/start` defaults may carry a long appended system prompt.
pub const BRIDGE_START_BODY_MAX: usize = 1024 * 1024;

/// Move data and deleting may name thousands of conversations (a selection,
/// §13.6, §14.4).
pub const DATA_MOVE_BODY_MAX: usize = 1024 * 1024;

/// Build the `/api` router (nested at `/api` by `server::router_for`).
pub fn api_router() -> Router<AppState> {
    // No credential needed.
    let public = Router::new()
        .route("/health", get(data::health))
        .route(
            "/pair/exchange",
            post(pairing::exchange).layer(DefaultBodyLimit::max(pairing::PAIR_BODY_MAX)),
        )
        .route("/pair/pending/{pending_id}", get(pairing::pending_status))
        .route("/pair/ca.crt", get(pairing::ca_cert))
        .route("/pair/ca.mobileconfig", get(pairing::ca_mobileconfig));

    let protected = Router::new()
        .route("/me", get(me).delete(pairing::unpair_self))
        // ── data (B2) ──
        .route(
            "/sessions",
            get(data::list_sessions)
                .post(control::spawn)
                .layer(DefaultBodyLimit::max(IMAGE_BODY_MAX)),
        )
        .route("/sessions/{id}", get(data::get_session))
        .route("/sessions/{id}/reveal", post(disk::reveal_session))
        .route("/sessions/{id}/transcript", get(data::transcript))
        .route("/sessions/{id}/media/{offset}/{uuid}/{block}", get(data::media))
        .route("/sessions/{id}/agents", get(data::session_agents))
        .route("/agents", get(data::fleet))
        .route("/sessions/{id}/tasks/{task_id}/output", get(data::task_output))
        .route("/sessions/{id}/diff", get(data::diff))
        .route("/sessions/{id}/file-patch", get(data::file_patch))
        .route("/sessions/{id}/files", get(data::files))
        .route("/sessions/{id}/commands", get(data::commands))
        .route("/projects", get(data::projects))
        .route("/defaults", get(data::defaults))
        // ── accounts (docs/ACCOUNTS.md) ──
        .route("/accounts", get(accounts::list).post(accounts::add))
        .route("/accounts/defaults", put(accounts::put_defaults))
        .route("/accounts/copy-consent", put(accounts::put_copy_consent))
        .route(
            "/accounts/{id}",
            axum::routing::patch(accounts::update).delete(accounts::remove),
        )
        .route("/accounts/{id}/refresh", post(accounts::refresh))
        .route("/accounts/{id}/login", post(accounts::login))
        .route("/accounts/{id}/hooks/install", post(accounts::hooks_install))
        .route("/accounts/{id}/hooks/uninstall", post(accounts::hooks_uninstall))
        .route(
            "/accounts/{id}/token",
            get(accounts::token_status)
                .post(accounts::token)
                .delete(accounts::cancel_token),
        )
        // ── backups, restores, pulls (docs/ACCOUNTS.md) ──
        .route("/backup", get(backup::get_backup))
        .route("/backup/settings", put(backup::put_settings))
        .route("/backup/passphrase", post(backup::post_passphrase))
        .route("/backup/push", post(backup::post_push))
        .route("/backup/index", get(backup::get_index))
        .route("/backup/restore", post(backup::post_restore))
        .route("/backup/pull", post(backup::post_pull))
        .route("/backup/storage", post(backup::post_storage))
        .route(
            "/backup/storage/{id}",
            axum::routing::patch(backup::patch_storage).delete(backup::delete_storage),
        )
        // ── moving data between accounts, data jobs (docs/ACCOUNTS.md) ──
        .route(
            "/data/move",
            post(transfers::post_move).layer(DefaultBodyLimit::max(DATA_MOVE_BODY_MAX)),
        )
        // ── finding conversations on disk (docs/ACCOUNTS.md §14) ──
        .route("/data/projects", get(disk::projects))
        .route("/data/projects/reveal", post(disk::reveal_project))
        .route(
            "/data/delete",
            post(transfers::post_delete).layer(DefaultBodyLimit::max(DATA_MOVE_BODY_MAX)),
        )
        .route("/data/jobs", get(transfers::list_jobs))
        .route("/data/jobs/{id}", get(transfers::get_job))
        .route("/data/jobs/{id}/undo", post(transfers::undo_job))
        .route("/usage", get(usage::get_usage))
        .route("/usage/refresh", post(usage::refresh))
        .route("/usage/settings", put(usage::put_settings))
        .route("/inbox", get(data::inbox))
        .route("/services", get(data::services))
        .route("/daemon", get(data::daemon))
        // ── control (B3; dispatchers route to B3/B4) ──
        .route(
            "/sessions/{id}/message",
            post(control::message).layer(DefaultBodyLimit::max(IMAGE_BODY_MAX)),
        )
        .route(
            "/sessions/{id}/uploads",
            post(uploads::upload).layer(DefaultBodyLimit::max(uploads::UPLOAD_BODY_MAX)),
        )
        .route("/sessions/{id}/queue/{message_id}", delete(control::cancel_queued))
        .route("/sessions/{id}/interrupt", post(control::interrupt))
        .route("/sessions/{id}/model", post(control::set_model))
        .route("/sessions/{id}/mode", post(control::set_mode))
        .route("/sessions/{id}/effort", post(control::set_effort))
        .route("/sessions/{id}/ultracode", post(control::set_ultracode))
        .route("/sessions/{id}/thinking", post(control::set_thinking))
        .route("/sessions/{id}/rename", post(control::rename))
        .route("/sessions/{id}/tasks/{task_id}/stop", post(control::stop_task))
        .route("/sessions/{id}/respond", post(control::respond))
        .route(
            "/sessions/{id}/continue",
            post(control::continue_session).layer(DefaultBodyLimit::max(IMAGE_BODY_MAX)),
        )
        .route(
            "/sessions/{id}/account",
            post(control::switch_account).delete(control::cancel_switch),
        )
        .route("/sessions/{id}/stop", post(control::stop))
        .route("/sessions/{id}/respawn", post(control::respawn))
        .route("/sessions/{id}/rm", post(control::rm))
        .route(
            "/sessions/{id}/permission-request",
            // The sidecar forwards whole tool inputs (a Write's content).
            post(control::permission_request).layer(DefaultBodyLimit::max(IMAGE_BODY_MAX)),
        )
        .route("/sessions/{id}/permission", post(control::legacy_permission))
        .route("/sessions/{id}/answer", post(control::legacy_answer))
        .route("/sessions/{id}/context", get(control::context_usage))
        .route("/sessions/{id}/mcp", get(control::mcp_status))
        // ── foreign control (B4) ──
        .route("/sessions/{id}/keys", post(foreign::keys))
        .route("/sessions/{id}/terminal", get(foreign::terminal))
        .route("/sessions/{id}/pty-attach", post(foreign::pty_attach))
        .route("/sessions/{id}/pty-inject", post(foreign::pty_inject))
        .route("/control/foreign", get(foreign::status))
        .route("/control/foreign/install", post(foreign::install))
        .route("/control/foreign/uninstall", post(foreign::uninstall))
        .route("/hooks/install", post(foreign::legacy_install_hooks))
        // ── pairing / devices (B1) ──
        .route("/pair/codes", post(pairing::create_code).layer(DefaultBodyLimit::max(pairing::PAIR_BODY_MAX)))
        .route("/pairing", get(pairing::legacy_pairing))
        .route("/devices", get(pairing::list_devices))
        .route("/devices/pending/{pending_id}/approve", post(pairing::approve))
        .route("/devices/pending/{pending_id}/deny", post(pairing::deny))
        .route(
            "/devices/{device_id}",
            delete(pairing::revoke).patch(pairing::update),
        )
        // ── remote access (B1) ──
        .route("/remote", get(remote::get_remote).put(remote::put_remote))
        // ── push (B5) ──
        .route("/push/vapid", get(push::vapid))
        .route("/push/subscribe", post(push::subscribe).delete(push::unsubscribe))
        .route("/push/test", post(push::test))
        .route("/push/prefs", get(push::get_prefs).put(push::put_prefs))
        // ── HTTP bridge control (existing) ──
        .route("/bridge", get(crate::bridge::control::get_status))
        .route(
            "/bridge/start",
            post(crate::bridge::control::post_start)
                .layer(DefaultBodyLimit::max(BRIDGE_START_BODY_MAX)),
        )
        .route("/bridge/stop", post(crate::bridge::control::post_stop))
        .route(
            "/bridge/active-thread",
            post(crate::bridge::control::post_active_thread),
        )
        // ── API servers besides the main one (docs/ACCOUNTS.md §13.5) ──
        .route(
            "/bridge/servers",
            post(crate::bridge::control::post_server)
                .layer(DefaultBodyLimit::max(BRIDGE_START_BODY_MAX)),
        )
        .route(
            "/bridge/servers/{id}",
            axum::routing::patch(crate::bridge::control::patch_server)
                .delete(crate::bridge::control::delete_server)
                .layer(DefaultBodyLimit::max(BRIDGE_START_BODY_MAX)),
        )
        .route(
            "/bridge/servers/{id}/start",
            post(crate::bridge::control::post_server_start)
                .layer(DefaultBodyLimit::max(BRIDGE_START_BODY_MAX)),
        )
        .route(
            "/bridge/servers/{id}/stop",
            post(crate::bridge::control::post_server_stop),
        )
        .route(
            "/bridge/servers/{id}/active-thread",
            post(crate::bridge::control::post_server_active_thread),
        )
        .route_layer(axum::middleware::from_fn(auth::require_api_principal))
        // Outermost, so a route's own limit (set closer to its handler)
        // overrides it.
        .layer(DefaultBodyLimit::max(API_BODY_MAX));

    public
        .merge(protected)
        .fallback(|| async { ApiError::not_found("not_found", "No such API endpoint.") })
}
