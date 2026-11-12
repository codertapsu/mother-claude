//! Team roles' routes (docs/PLAN-MACHINES.md Phase 4 decision 9).
//!
//! This computer's dashboard (merged into `/api` by `machines::routes`):
//!
//! | Route | Who | Does |
//! |---|---|---|
//! | `GET /api/machines/roles` | Authed | `{roles: [...]}`: the team roles this computer runs for other computers' leads |
//! | `POST /api/machines/roles/{binding}/stop` | Authed | stops one's session here (its lead starts it again when it has work) |
//! | `DELETE /api/machines/roles/{binding}` | ManageData | removes one here: its session, its worktree, its record (its lead mints a new binding at its next start) |
//!
//! The other computers (merged into the peer router):
//!
//! * the lead's, on the computer that runs its role — each with this
//!   computer's `run` grant for it on the request's path and Machines on
//!   here (`work::target::running_peer`): `POST /peer/teams/roles/prepare`,
//!   `POST /peer/teams/roles` (its code coming), `PUT …/{binding}/files/
//!   {name}?offset=`, `POST …/{binding}/start`, `GET …/{binding}?landed=`,
//!   `POST …/{binding}/message`, `POST …/{binding}/stop`, `DELETE
//!   …/{binding}`, `GET …/{binding}/result`, `GET …/{binding}/result/files/
//!   {name}?seq=&offset=`;
//! * the computer that runs a role, on its lead's: `POST /peer/teams/board`
//!   (a team tool its role session called) and `POST /peer/teams/roles/wake`
//!   (a message for its stopped session there, which the lead's launcher
//!   resumes, `lead::wake`) — with Machines on here and the binding this
//!   computer minted (`lead::board_call`).

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::Value;

use super::{host, lead};
use crate::machines::work::target::running_peer;
use crate::server::auth::{Authed, ManageData};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// The dashboard's routes.
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/machines/roles", get(list))
        .route("/machines/roles/{binding}", axum::routing::delete(remove))
        .route("/machines/roles/{binding}/stop", post(stop))
}

/// The other computers' routes.
pub fn peer_router() -> Router<AppState> {
    Router::new()
        .route("/peer/teams/board", post(peer_board))
        .route("/peer/teams/roles/wake", post(peer_wake))
        .route("/peer/teams/roles/prepare", post(peer_prepare))
        .route("/peer/teams/roles", post(peer_begin))
        .route(
            "/peer/teams/roles/{binding}",
            get(peer_status).delete(peer_remove),
        )
        .route(
            "/peer/teams/roles/{binding}/files/{name}",
            put(peer_chunk).layer(DefaultBodyLimit::max(
                crate::machines::chunks::CHUNK + 64 * 1024,
            )),
        )
        .route("/peer/teams/roles/{binding}/start", post(peer_start))
        .route("/peer/teams/roles/{binding}/message", post(peer_message))
        .route("/peer/teams/roles/{binding}/stop", post(peer_stop))
        .route("/peer/teams/roles/{binding}/result", get(peer_result))
        .route(
            "/peer/teams/roles/{binding}/result/files/{name}",
            get(peer_result_bytes),
        )
}

fn unknown() -> ApiError {
    ApiError::not_found("binding_unknown", "No such team role here.")
}

fn hosted(state: &AppState, binding: &str) -> ApiResult<std::sync::Arc<super::Hosted>> {
    if !super::is_binding_id(binding) {
        return Err(unknown());
    }
    state.machines.work.roles.get(binding).ok_or_else(unknown)
}

// ─── this computer's dashboard ───────────────────────────────────────────────────

async fn list(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(super::view(&state))
}

async fn stop(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(binding): Path<String>,
) -> ApiResult<Json<Value>> {
    let h = hosted(&state, &binding)?;
    host::stop_here(&state, &h, "stopped from this computer's Machines page").await;
    crate::server::auth::note_device_action(
        &state,
        &ctx,
        "stopped a team role another computer runs here",
    );
    Ok(Json(host::status_view(&state, &h)))
}

async fn remove(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(binding): Path<String>,
) -> ApiResult<Json<Value>> {
    let h = hosted(&state, &binding)?;
    host::remove_here(&state, &h).await;
    crate::server::auth::note_device_action(
        &state,
        &ctx,
        "removed a team role another computer ran here",
    );
    Ok(Json(serde_json::json!({ "removed": true })))
}

// ─── the other computers ─────────────────────────────────────────────────────────

async fn peer_board(
    ctx: ClientContext,
    State(state): State<AppState>,
    Body(b): Body<lead::BoardCall>,
) -> ApiResult<Json<Value>> {
    lead::board_call(&state, &ctx, b).await.map(Json)
}

async fn peer_wake(
    ctx: ClientContext,
    State(state): State<AppState>,
    Body(b): Body<lead::WakeCall>,
) -> ApiResult<Json<Value>> {
    lead::wake(&state, &ctx, b).await.map(Json)
}

async fn peer_prepare(
    ctx: ClientContext,
    State(state): State<AppState>,
    Body(b): Body<host::PrepareBody>,
) -> ApiResult<Json<Value>> {
    let from = running_peer(&state, &ctx)?;
    host::prepare(&state, &from, b).await.map(Json)
}

async fn peer_begin(
    ctx: ClientContext,
    State(state): State<AppState>,
    Body(b): Body<host::BeginBody>,
) -> ApiResult<Json<Value>> {
    let from = running_peer(&state, &ctx)?;
    host::begin(&state, &ctx, &from, b).await.map(Json)
}

#[derive(serde::Deserialize)]
struct ChunkQuery {
    offset: u64,
}

async fn peer_chunk(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path((binding, name)): Path<(String, String)>,
    Query(q): Query<ChunkQuery>,
    body: axum::body::Bytes,
) -> Response {
    let answer = async {
        let from = running_peer(&state, &ctx)?;
        host::put_chunk(&state, &from, &binding, &name, q.offset, body).await
    }
    .await;
    match answer {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn peer_start(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(binding): Path<String>,
    Body(b): Body<host::StartBody>,
) -> ApiResult<Json<Value>> {
    let from = running_peer(&state, &ctx)?;
    host::start(&state, &ctx, &from, &binding, b)
        .await
        .map(Json)
}

async fn peer_status(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(binding): Path<String>,
    Query(q): Query<host::StatusQuery>,
) -> ApiResult<Json<Value>> {
    let from = running_peer(&state, &ctx)?;
    host::status(&state, &from, &binding, q).await.map(Json)
}

async fn peer_message(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(binding): Path<String>,
    Body(b): Body<host::MessageBody>,
) -> ApiResult<Json<Value>> {
    let from = running_peer(&state, &ctx)?;
    host::message(&state, &ctx, &from, &binding, b)
        .await
        .map(Json)
}

async fn peer_stop(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(binding): Path<String>,
    Body(b): Body<host::StopBody>,
) -> ApiResult<Json<Value>> {
    let from = running_peer(&state, &ctx)?;
    host::stop(&state, &from, &binding, b).await.map(Json)
}

async fn peer_remove(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(binding): Path<String>,
) -> ApiResult<Json<Value>> {
    let from = running_peer(&state, &ctx)?;
    host::remove(&state, &from, &binding).await.map(Json)
}

async fn peer_result(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(binding): Path<String>,
) -> ApiResult<Json<Value>> {
    let from = running_peer(&state, &ctx)?;
    host::result_info(&state, &from, &binding).await.map(Json)
}

async fn peer_result_bytes(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path((binding, name)): Path<(String, String)>,
    Query(q): Query<host::BytesQuery>,
) -> Response {
    let answer = async {
        let from = running_peer(&state, &ctx)?;
        host::result_bytes(&state, &from, &binding, &name, q).await
    }
    .await;
    match answer {
        Ok(bytes) => (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}
