//! The routes of a move (docs/PLAN-MACHINES.md §7, `machines::moves`).
//!
//! This machine's dashboard (`/api`, merged into `routes::api_router`; the
//! other machine's screens reach them through the proxy with the `move`
//! grant, `routes::peer_route_grant`):
//!
//! | Route | Who | Does |
//! |---|---|---|
//! | `POST /api/sessions/{id}/move/preflight` | Authed | the move sheet's checks, nothing written: `{ok, blockers, warnings, session, to, code, target, recipe, asCopy}` |
//! | `POST /api/sessions/{id}/move` | desktop, device, another computer | starts a move: `202` with its record; `takeOver: true` → `200 {takenOver, session, home, from, epoch, warning}`; a blocker → `409 move_blocked` with `preflight` |
//! | `GET /api/moves` | Authed | `{moves: [record…]}`, newest first |
//! | `GET /api/moves/{id}` | Authed | one record |
//! | `POST /api/moves/{id}/cancel` | desktop, device, another computer | until the target starts placing it (`409 move_too_late` after) |
//! | `POST /api/moves/{id}/undo` | desktop, device, another computer | undone on the target (asked from the source) |
//! | `POST /api/moves/{id}/retry` | desktop, device, another computer | a move that landed but did not start there starts again |
//!
//! The body of a move: `{to, code: "bundle"|"remote"|"none", apply:
//! "checkout"|"worktree", folder?, keepDriving?, asCopy?, recipe?, trust?,
//! takeOver?, when?: "idle"|"now", remote?}` (`trust` needs the dangerous
//! right, as a pull's).
//!
//! The other machines (`/peer`, merged into `routes::peer_router`), each
//! needing this machine's `move` grant for the caller on the path the
//! request came over: `POST /peer/moves/preflight`, `POST|GET|DELETE
//! /peer/moves/{id}`, `GET|PUT /peer/moves/{id}/files/{name}?offset=`,
//! `POST /peer/moves/{id}/land`, `POST /peer/moves/{id}/undo`, `GET|POST
//! /peer/leases`.
//!
//! While Machines is off here, `POST /api/sessions/{id}/move` and its
//! preflight answer `409 machines_off`, and `/peer/moves/*` answer `403
//! machines_off` — but for `DELETE /peer/moves/{id}`, which only takes back
//! what a move that stopped left here. The leases stay open (a hello carries
//! them anyway).

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::grants::Grant;
use super::moves::{self, wire};
use crate::server::auth::Authed;
use crate::server::context::{ClientContext, Principal};
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// The dashboard's move routes.
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/sessions/{id}/move", post(start))
        .route("/sessions/{id}/move/preflight", post(preflight))
        .route("/moves", get(list))
        .route("/moves/{id}", get(get_one))
        .route("/moves/{id}/cancel", post(cancel))
        .route("/moves/{id}/undo", post(undo))
        .route("/moves/{id}/retry", post(retry))
}

/// The other machines' move routes.
pub fn peer_router() -> Router<AppState> {
    Router::new()
        .route("/peer/moves/preflight", post(peer_preflight))
        .route(
            "/peer/moves/{id}",
            get(peer_state)
                .post(peer_create)
                .delete(peer_drop)
                // A manifest may carry the messages queued behind a turn.
                .layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route(
            "/peer/moves/{id}/files/{name}",
            get(peer_file)
                .put(peer_chunk)
                .layer(DefaultBodyLimit::max(moves::CHUNK + 64 * 1024)),
        )
        .route("/peer/moves/{id}/land", post(peer_land))
        .route("/peer/moves/{id}/undo", post(peer_undo))
        .route("/peer/leases", get(peer_leases_get).post(peer_leases_post))
}

// ─── this machine's dashboard ────────────────────────────────────────────────────

/// Who may start, cancel or undo a move: the desktop, a paired device, or
/// another of your computers (through the proxy, with its `move` grant
/// here) — never the local-script token.
fn may_move(ctx: &ClientContext) -> ApiResult<()> {
    match ctx.principal {
        Principal::Desktop | Principal::Device { .. } | Principal::Peer { .. } => Ok(()),
        _ => Err(ApiError::forbidden(
            "desktop_only",
            "Moving a session is only available on the Mother Claude desktop app, a paired \
             device or another of your computers.",
        )),
    }
}

fn valid_session(id: &str) -> ApiResult<()> {
    crate::claude::ids::require_session_id(id)
}

async fn preflight(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<moves::MoveBody>,
) -> ApiResult<Json<moves::Preflight>> {
    valid_session(&id)?;
    super::using(&state)?;
    Ok(Json(moves::preflight(&state, &ctx, &id, &body).await?))
}

async fn start(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<moves::MoveBody>,
) -> ApiResult<Response> {
    may_move(&ctx)?;
    valid_session(&id)?;
    super::using(&state)?;
    let (status, v) = moves::start(&state, &ctx, &id, &body).await?;
    Ok((status, Json(v)).into_response())
}

async fn list(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(moves::list_view(&state))
}

async fn get_one(
    Authed(_): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let rec = moves::load(&state, &id).ok_or_else(moves::not_found)?;
    Ok(Json(rec.view()))
}

async fn cancel(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    may_move(&ctx)?;
    let rec = moves::cancel(&state, &id).await?;
    Ok(Json(rec.view()))
}

async fn undo(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    may_move(&ctx)?;
    let rec = moves::undo(&state, &ctx, &id).await?;
    Ok(Json(rec.view()))
}

async fn retry(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    may_move(&ctx)?;
    let rec = moves::retry(&state, &ctx, &id).await?;
    Ok(Json(rec.view()))
}

// ─── the other machines ──────────────────────────────────────────────────────────

/// A paired machine holding this machine's `move` grant for the path the
/// request came over (the LAN column on a direct local path, the internet
/// one otherwise): `(machine, lan)`.
fn mover(state: &AppState, ctx: &ClientContext) -> ApiResult<(String, bool)> {
    let Principal::Peer { machine, lan, .. } = &ctx.principal else {
        return Err(ApiError::forbidden(
            "peer_only",
            "Only a paired computer can call this.",
        ));
    };
    if !state.machines.allows(machine, Grant::Move, *lan) {
        return Err(ApiError::forbidden(
            "grant_missing",
            "This computer has not given yours the \"move\" permission.",
        )
        .with("grant", "move"));
    }
    Ok((machine.clone(), *lan))
}

/// [`mover`], for a move's target half: refused while Machines is off here
/// (`403 machines_off`).
fn moving_here(state: &AppState, ctx: &ClientContext) -> ApiResult<(String, bool)> {
    if !matches!(ctx.principal, Principal::Peer { .. }) {
        return mover(state, ctx);
    }
    super::serving(state)?;
    mover(state, ctx)
}

fn move_id(id: &str) -> ApiResult<()> {
    if moves::is_move_id(id) {
        Ok(())
    } else {
        Err(moves::not_found())
    }
}

async fn peer_preflight(
    ctx: ClientContext,
    State(state): State<AppState>,
    Body(ask): Body<wire::Ask>,
) -> ApiResult<Json<wire::Answer>> {
    let (from, _) = moving_here(&state, &ctx)?;
    Ok(Json(moves::peer_preflight(&state, &ctx, &from, ask).await?))
}

async fn peer_create(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(mf): Body<wire::Manifest>,
) -> ApiResult<Json<Value>> {
    let (from, _) = moving_here(&state, &ctx)?;
    move_id(&id)?;
    Ok(Json(
        moves::peer_create(&state, &ctx, &from, &id, mf).await?,
    ))
}

async fn peer_state(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let (from, _) = moving_here(&state, &ctx)?;
    move_id(&id)?;
    Ok(Json(moves::peer_state(&state, &from, &id).await?))
}

/// The source gave a move up: what it left here goes. Served while Machines
/// is off too — it only takes back what that move brought (a move stopped
/// by Machines turning off here is dropped at once, not at the next start).
async fn peer_drop(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let (from, _) = mover(&state, &ctx)?;
    move_id(&id)?;
    Ok(Json(moves::peer_drop(&state, &from, &id).await?))
}

async fn peer_file(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let (from, _) = moving_here(&state, &ctx)?;
    move_id(&id)?;
    Ok(Json(
        moves::peer_file_size(&state, &from, &id, &name).await?,
    ))
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
        let (from, _) = moving_here(&state, &ctx)?;
        move_id(&id)?;
        moves::peer_put_chunk(&state, &from, &id, &name, q.offset, body).await
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
) -> ApiResult<Json<wire::Landed>> {
    let (from, _) = moving_here(&state, &ctx)?;
    move_id(&id)?;
    Ok(Json(moves::peer_land(&state, &ctx, &from, &id).await?))
}

async fn peer_undo(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let (from, _) = moving_here(&state, &ctx)?;
    move_id(&id)?;
    Ok(Json(moves::peer_undo(&state, &from, &id).await?))
}

async fn peer_leases_get(
    ctx: ClientContext,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    mover(&state, &ctx)?;
    Ok(Json(json!({ "leases": state.machines.leases.wire() })))
}

async fn peer_leases_post(
    ctx: ClientContext,
    State(state): State<AppState>,
    Body(body): Body<wire::LeasesBody>,
) -> ApiResult<Response> {
    let (from, lan) = mover(&state, &ctx)?;
    let v = moves::peer_leases(&state, &from, lan, body).await?;
    Ok((StatusCode::OK, Json(v)).into_response())
}
