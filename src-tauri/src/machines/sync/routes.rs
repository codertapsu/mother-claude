//! Code sync's routes (docs/PLAN-MACHINES.md §5.15).
//!
//! This computer's dashboard (merged into `/api` by `machines::routes`):
//!
//! | Route | Who | Does |
//! |---|---|---|
//! | `GET /api/machines/sync` | Authed (a phone sees it read-only) | `{pairs: [view…], watching}` |
//! | `PUT /api/machines/sync {project, machine, on}` | DesktopOnly | on: asks the other computer (its desktop confirms there); off: at once here, the other told; answers the list |
//! | `POST /api/machines/sync/{id}/now` | DesktopOnly | a step now; answers the pair |
//! | `POST /api/machines/sync/{id}/resolve {keep: "here"\|"there"}` | DesktopOnly | the other side's state kept under a ref and put back, then the winner's lands; answers the pair |
//!
//! The other computers (`/peer/sync/*`, merged into the peer router), each
//! needing this computer's `move` grant for the caller on the path the
//! request came over, and Machines on here (`403 machines_off`) — but
//! `POST …/off`, which only stops it and is served to any paired computer
//! at any time: `GET /peer/sync/{id}`, `POST /peer/sync/{id}/steps`,
//! `GET|PUT /peer/sync/{id}/files/{name}?offset=`, `POST /peer/sync/{id}/
//! land`, `…/agree`, `…/pause`, `…/push`, `…/lose`, `…/confirm`, `…/off`.

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use super::{is_sync_id, on, peer, step};
use crate::machines::grants::Grant;
use crate::machines::{self as m};
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::context::{ClientContext, Principal};
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// `404 sync_unknown`.
pub fn unknown() -> ApiError {
    ApiError::not_found("sync_unknown", "No such synced project here.")
}

/// The dashboard's routes.
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/machines/sync", get(list).put(set))
        .route("/machines/sync/{id}/now", post(now))
        .route("/machines/sync/{id}/resolve", post(resolve))
}

/// The other computers' routes.
pub fn peer_router() -> Router<AppState> {
    Router::new()
        .route("/peer/sync/{id}", get(peer_state))
        .route("/peer/sync/{id}/steps", post(peer_begin))
        .route(
            "/peer/sync/{id}/files/{name}",
            get(peer_file).put(peer_chunk).layer(DefaultBodyLimit::max(
                crate::machines::chunks::CHUNK + 64 * 1024,
            )),
        )
        .route("/peer/sync/{id}/land", post(peer_land))
        .route("/peer/sync/{id}/agree", post(peer_agree))
        .route("/peer/sync/{id}/pause", post(peer_pause))
        .route("/peer/sync/{id}/push", post(peer_push))
        .route("/peer/sync/{id}/lose", post(peer_lose))
        .route("/peer/sync/{id}/confirm", post(peer_confirm))
        .route("/peer/sync/{id}/off", post(peer_off))
}

fn sync_id(id: &str) -> ApiResult<()> {
    if is_sync_id(id) {
        Ok(())
    } else {
        Err(unknown())
    }
}

// ─── this computer's dashboard ───────────────────────────────────────────────────

async fn list(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(super::view(&state))
}

async fn set(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<on::SetBody>,
) -> ApiResult<Json<Value>> {
    on::set(&state, &ctx, body).await.map(Json)
}

async fn now(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    sync_id(&id)?;
    let p = state.machines.sync.pair(&id).ok_or_else(unknown)?;
    if !p.here_on {
        return Err(unknown());
    }
    step::run(&state, &id, super::Trigger::Now).await;
    let p = state.machines.sync.pair(&id).ok_or_else(unknown)?;
    Ok(Json(super::pair_view(&state, &p)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveBody {
    /// `here` | `there`.
    keep: String,
}

async fn resolve(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<ResolveBody>,
) -> ApiResult<Json<Value>> {
    sync_id(&id)?;
    let keep_here = match body.keep.as_str() {
        "here" => true,
        "there" => false,
        _ => {
            return Err(ApiError::unprocessable(
                "sync_bad_keep",
                "Keep \"here\" or \"there\".",
            ))
        }
    };
    step::resolve(&state, &id, keep_here).await.map(Json)
}

// ─── the other computers ─────────────────────────────────────────────────────────

/// A paired computer holding this computer's `move` grant for the path the
/// request came over, while Machines is on here.
fn syncing(state: &AppState, ctx: &ClientContext) -> ApiResult<String> {
    let Principal::Peer { machine, lan, .. } = &ctx.principal else {
        return Err(ApiError::forbidden(
            "peer_only",
            "Only a paired computer can call this.",
        ));
    };
    m::serving(state)?;
    if !state.machines.allows(machine, Grant::Move, *lan) {
        return Err(ApiError::forbidden(
            "grant_missing",
            "This computer has not given yours the \"move\" permission.",
        )
        .with("grant", "move"));
    }
    Ok(machine.clone())
}

async fn peer_state(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<peer::StateQuery>,
) -> ApiResult<Json<Value>> {
    let from = syncing(&state, &ctx)?;
    sync_id(&id)?;
    peer::state_for(&state, &from, &id, q).await.map(Json)
}

async fn peer_begin(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(b): Body<peer::Begin>,
) -> ApiResult<Json<Value>> {
    let from = syncing(&state, &ctx)?;
    sync_id(&id)?;
    peer::begin(&state, &from, &id, b).await.map(Json)
}

async fn peer_file(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let from = syncing(&state, &ctx)?;
    sync_id(&id)?;
    peer::file_size(&state, &from, &id, &name).await.map(Json)
}

#[derive(Deserialize)]
struct ChunkQuery {
    offset: u64,
}

async fn peer_chunk(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
    Query(q): Query<ChunkQuery>,
    body: axum::body::Bytes,
) -> Response {
    let answer = async {
        let from = syncing(&state, &ctx)?;
        sync_id(&id)?;
        peer::put_chunk(&state, &from, &id, &name, q.offset, body).await
    }
    .await;
    match answer {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn peer_land(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(b): Body<peer::LandBody>,
) -> ApiResult<Json<Value>> {
    let from = syncing(&state, &ctx)?;
    sync_id(&id)?;
    peer::land(&state, &ctx, &from, &id, b).await.map(Json)
}

async fn peer_agree(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(b): Body<peer::AgreeBody>,
) -> ApiResult<Json<Value>> {
    let from = syncing(&state, &ctx)?;
    sync_id(&id)?;
    peer::agree(&state, &from, &id, b).await.map(Json)
}

async fn peer_pause(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(b): Body<peer::PauseBody>,
) -> ApiResult<Json<Value>> {
    let from = syncing(&state, &ctx)?;
    sync_id(&id)?;
    peer::pause(&state, &from, &id, b).await.map(Json)
}

async fn peer_push(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let from = syncing(&state, &ctx)?;
    sync_id(&id)?;
    peer::push(&state, &from, &id).await.map(Json)
}

async fn peer_lose(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let from = syncing(&state, &ctx)?;
    sync_id(&id)?;
    peer::lose(&state, &from, &id).await.map(Json)
}

async fn peer_confirm(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(b): Body<peer::ConfirmBody>,
) -> ApiResult<Json<Value>> {
    let from = syncing(&state, &ctx)?;
    sync_id(&id)?;
    peer::confirm(&state, &from, &id, b).await.map(Json)
}

/// Turning it off there: any paired computer, at any time (it only stops
/// writes here).
async fn peer_off(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(b): Body<peer::OffBody>,
) -> ApiResult<Json<Value>> {
    let Principal::Peer { machine, .. } = &ctx.principal else {
        return Err(ApiError::forbidden(
            "peer_only",
            "Only a paired computer can call this.",
        ));
    };
    sync_id(&id)?;
    peer::off(&state, machine, &id, b).await.map(Json)
}
