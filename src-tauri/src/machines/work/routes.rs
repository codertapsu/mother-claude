//! Runs' routes (docs/PLAN-MACHINES.md Phase 4).
//!
//! This computer's dashboard (merged into `/api` by `machines::routes`):
//!
//! | Route | Who | Does |
//! |---|---|---|
//! | `GET /api/machines/runs` | Authed | `{runs: [summary…]}`: both ways, newest first (each with its output's last 2 KiB) |
//! | `GET /api/machines/runs/{id}` | Authed | one run whole: its output's last 64 KiB, the files it made |
//! | `POST /api/machines/runs/{id}/cancel` | Authed | stops it — there, or here — and answers it |
//! | `GET /api/machines/run-grants` | Authed | `{grants: […]}`: the standing grants for runs this computer asks for |
//! | `DELETE /api/machines/run-grants/{id}` | DesktopOnly | revokes one; answers the list |
//!
//! The other computers (`/peer/work/*`, merged into the peer router): each
//! needs this computer's `run` grant for the caller on the request's path
//! and Machines on here (`target::running_peer`) — but the echo, which the
//! setup's readiness test runs before Machines is on, and `echo-back`
//! (`view`): `POST /peer/work/prepare`, `POST /peer/work/runs`,
//! `GET|PUT /peer/work/runs/{id}/files/{name}?offset=`, `POST …/{id}/start`,
//! `GET …/{id}?wait=&size=&state=`, `GET …/{id}/output?from=&max=`,
//! `POST …/{id}/cancel`, `GET …/{id}/artifact?path=`,
//! `GET …/{id}/artifact/bytes?path=&offset=`, `POST /peer/work/echo`,
//! `POST /peer/work/echo-back`.

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

use super::{grants, source, target};
use crate::machines::grants::Grant;
use crate::machines::{self as m};
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::context::{ClientContext, Principal};
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// The dashboard's routes.
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/machines/runs", get(list))
        .route("/machines/runs/{id}", get(one))
        .route("/machines/runs/{id}/cancel", post(cancel))
        .route("/machines/run-grants", get(list_grants))
        .route("/machines/run-grants/{id}", delete(revoke_grant))
}

/// The other computers' routes.
pub fn peer_router() -> Router<AppState> {
    Router::new()
        .route("/peer/work/prepare", post(peer_prepare))
        .route("/peer/work/runs", post(peer_begin))
        .route("/peer/work/runs/{id}", get(peer_status))
        .route(
            "/peer/work/runs/{id}/files/{name}",
            put(peer_chunk).layer(DefaultBodyLimit::max(
                crate::machines::chunks::CHUNK + 64 * 1024,
            )),
        )
        .route("/peer/work/runs/{id}/start", post(peer_start))
        .route("/peer/work/runs/{id}/output", get(peer_output))
        .route("/peer/work/runs/{id}/cancel", post(peer_cancel))
        .route("/peer/work/runs/{id}/artifact", get(peer_artifact))
        .route(
            "/peer/work/runs/{id}/artifact/bytes",
            get(peer_artifact_bytes),
        )
        .route("/peer/work/echo", post(peer_echo))
        .route("/peer/work/echo-back", post(peer_echo_back))
}

fn unknown() -> ApiError {
    ApiError::not_found("run_unknown", "No such run.")
}

// ─── this computer's dashboard ───────────────────────────────────────────────────

async fn list(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(super::view(&state))
}

async fn one(
    Authed(_): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !super::is_run_id(&id) {
        return Err(unknown());
    }
    let run = state.machines.work.get(&id).ok_or_else(unknown)?;
    Ok(Json(run.record().full()))
}

async fn cancel(
    Authed(_): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !super::is_run_id(&id) {
        return Err(unknown());
    }
    let run = state.machines.work.get(&id).ok_or_else(unknown)?;
    let r = source::cancel_run(&state, &run).await?;
    Ok(Json(r.full()))
}

fn grants_answer(state: &AppState) -> Value {
    let list: Vec<Value> = state
        .machines
        .work
        .grants
        .list()
        .iter()
        .map(|g| grants::grant_view(state, g))
        .collect();
    json!({ "grants": list })
}

async fn list_grants(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(grants_answer(&state))
}

async fn revoke_grant(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !grants::is_grant_id(&id) {
        return Err(ApiError::not_found(
            "grant_unknown",
            "No such standing grant.",
        ));
    }
    let g = state
        .machines
        .work
        .grants
        .remove(&id)
        .ok_or_else(|| ApiError::not_found("grant_unknown", "No such standing grant."))?;
    state.devices.audit.record(AuditEntry {
        device_id: Some(g.machine.clone()),
        device_name: Some(m::peer_name(&state, &g.machine)),
        detail: Some(json!({
            "grant": g.id,
            "program": super::program_of(&g.prefix),
            "prefixSha256": super::command_sha256(&g.prefix),
            "project": g.project_name,
        })),
        ..AuditEntry::from_ctx(&ctx, "machine.run.grant.remove")
    });
    m::publish(&state, "runGrants");
    Ok(Json(grants_answer(&state)))
}

// ─── the other computers ─────────────────────────────────────────────────────────

async fn peer_prepare(
    ctx: ClientContext,
    State(state): State<AppState>,
    Body(b): Body<target::PrepareBody>,
) -> ApiResult<Json<Value>> {
    let from = target::running_peer(&state, &ctx)?;
    target::prepare(&state, &from, b).await.map(Json)
}

async fn peer_begin(
    ctx: ClientContext,
    State(state): State<AppState>,
    Body(b): Body<target::BeginBody>,
) -> ApiResult<Json<Value>> {
    let from = target::running_peer(&state, &ctx)?;
    target::begin(&state, &ctx, &from, b).await.map(Json)
}

async fn peer_status(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<target::StatusQuery>,
) -> ApiResult<Json<Value>> {
    let from = target::running_peer(&state, &ctx)?;
    target::status(&state, &from, &id, q).await.map(Json)
}

#[derive(serde::Deserialize)]
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
        let from = target::running_peer(&state, &ctx)?;
        target::put_chunk(&state, &from, &id, &name, q.offset, body).await
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
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let from = target::running_peer(&state, &ctx)?;
    target::start(&state, &ctx, &from, &id).await.map(Json)
}

async fn peer_output(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<target::OutputQuery>,
) -> ApiResult<Json<Value>> {
    let from = target::running_peer(&state, &ctx)?;
    target::output_page(&state, &from, &id, q).await.map(Json)
}

async fn peer_cancel(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let from = target::running_peer(&state, &ctx)?;
    target::cancel(&state, &from, &id).await.map(Json)
}

async fn peer_artifact(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<target::ArtifactQuery>,
) -> ApiResult<Json<Value>> {
    let from = target::running_peer(&state, &ctx)?;
    target::artifact_info(&state, &from, &id, q).await.map(Json)
}

async fn peer_artifact_bytes(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<target::ArtifactQuery>,
) -> Response {
    let answer = async {
        let from = target::running_peer(&state, &ctx)?;
        target::artifact_bytes(&state, &from, &id, q).await
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

async fn peer_echo(ctx: ClientContext, State(state): State<AppState>) -> ApiResult<Json<Value>> {
    target::echo(&state, &ctx).await.map(Json)
}

/// The readiness test's other direction: the asking computer has this one
/// run the echo on it (its own `run` grant for this one decides there).
async fn peer_echo_back(
    ctx: ClientContext,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    let Principal::Peer { machine, .. } = &ctx.principal else {
        return Err(ApiError::forbidden(
            "peer_only",
            "Only a paired computer can call this.",
        ));
    };
    crate::machines::routes::granted(&state, &ctx, Grant::View)?;
    Ok(Json(echo_on(&state, machine).await))
}

/// Has `machine` run the readiness test's echo: its answer, or why not.
pub async fn echo_on(state: &AppState, machine: &str) -> Value {
    let acting = m::acting::machine_header(&m::this_name(state), false);
    match crate::machines::moves::call(
        state,
        machine,
        "POST",
        "/peer/work/echo",
        None,
        &acting,
        std::time::Duration::from_secs(90),
    )
    .await
    {
        Ok((200, v)) => v,
        Ok((status, v)) => json!({
            "ok": false,
            "status": status,
            "error": v.pointer("/error/message").and_then(Value::as_str).unwrap_or("it refused"),
            "code": v.pointer("/error/code").and_then(Value::as_str),
        }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}
