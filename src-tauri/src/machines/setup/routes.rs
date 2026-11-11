//! The routes of the setup's rest (docs/PLAN-MACHINES.md §7):
//!
//! * `GET /api/machines/accounts` (Authed) — every computer's accounts
//!   matched by name, each one's sign-in, what was asked;
//! * `GET /api/machines/folders?fresh` (DesktopOnly) — repositories matched
//!   by origin: proposed roots, matched, mapped elsewhere, on one side only;
//! * `POST /api/machines/asks/{id} {answer: "confirm"|"decline"}`
//!   (DesktopOnly) — answers what another computer asked this one;
//! * `POST /api/machines/clone {origin, root, rel}` (DesktopOnly) — *Clone
//!   it here*: `git clone` into a folder of a root on this computer;
//! * `POST /api/machines/setup/remote {project}` (DesktopOnly) — whether a
//!   project's remote answers without a prompt, here and on each computer a
//!   root maps it to (on demand only: never in the background).
//!
//! For the other computers (the `view` grant): `GET /peer/accounts`, `GET
//! /peer/origins`, `POST /peer/setup/changed` (read
//! my checks again) and `POST /peer/setup/remote {root, rel}`.

use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{accounts, asks, folders};
use crate::claude::paths::{self, PathForm};
use crate::machines::code::GitRunner;
use crate::machines::grants::Grant;
use crate::machines::roots;
use crate::machines::routes::granted;
use crate::machines::transport;
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// Merged into `/api` (the protected router).
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/machines/accounts", get(accounts_view))
        .route("/machines/folders", get(folders_view))
        .route("/machines/asks/{id}", post(answer_ask))
        .route("/machines/clone", post(clone_here))
        .route("/machines/setup/remote", post(check_remote))
}

/// Merged into the peer router.
pub fn peer_router() -> Router<AppState> {
    Router::new()
        .route(accounts::PEER_PATH, get(peer_accounts))
        .route(folders::PEER_PATH, get(peer_origins))
        .route("/peer/setup/changed", post(peer_changed))
        .route("/peer/setup/remote", post(peer_remote))
}

#[derive(Debug, Default, Deserialize)]
struct FreshQuery {
    #[serde(default)]
    fresh: bool,
}

// ─── this computer's dashboard ─────────────────────────────────────────────────

async fn accounts_view(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(accounts::view(&state).await)
}

async fn folders_view(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Query(q): Query<FreshQuery>,
) -> Json<Value> {
    Json(folders::view(&state, q.fresh).await)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerBody {
    /// `confirm` | `decline`.
    answer: String,
}

async fn answer_ask(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(req): Body<AnswerBody>,
) -> ApiResult<Json<Value>> {
    let confirm = match req.answer.as_str() {
        "confirm" => true,
        "decline" => false,
        _ => {
            return Err(ApiError::unprocessable(
                "machines_bad_answer",
                "Answer confirm or decline.",
            ))
        }
    };
    asks::answer(&state, &ctx, &id, confirm).await.map(Json)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloneBody {
    origin: String,
    root: String,
    rel: String,
}

/// *Clone it here*: the desktop at this computer asks for it, so it runs.
async fn clone_here(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<CloneBody>,
) -> ApiResult<Json<Value>> {
    if folders::sanitize_origin(&req.origin).as_deref() != Some(req.origin.as_str()) {
        return Err(ApiError::unprocessable(
            "machines_bad_origin",
            "That is not a git remote Mother Claude can clone (https, ssh or git, without credentials).",
        ));
    }
    let folder = asks::folder_in_root(&state, &req.root, &req.rel)
        .map_err(|e| ApiError::unprocessable("machines_bad_folder", e))?;
    if !asks::free_folder(&folder) {
        return Err(ApiError::conflict(
            "machines_folder_taken",
            format!("{} is there already.", folder.display()),
        ));
    }
    super::note(
        &state,
        &format!("clone {} into {}", req.origin, folder.display()),
    );
    let origin = req.origin.clone();
    let target = folder.clone();
    tokio::task::spawn_blocking(move || folders::clone_into(&GitRunner::new(), &origin, &target))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::unprocessable("machines_clone_failed", e))?;
    state.devices.audit.record(AuditEntry {
        detail: Some(json!({ "folder": folder.to_string_lossy() })),
        ..AuditEntry::from_ctx(&ctx, "machines.clone")
    });
    crate::machines::publish(&state, "folders");
    Ok(Json(
        json!({ "cloned": true, "folder": folder.to_string_lossy() }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteBody {
    project: String,
}

/// A project's folder on this computer that holds a git repository.
fn repo_here(raw: &str) -> ApiResult<String> {
    let folder = paths::clean_folder(raw, PathForm::here()).map_err(|why| {
        ApiError::unprocessable("machines_bad_project", format!("That project: {why}."))
    })?;
    if !FsPath::new(&folder).join(".git").exists() {
        return Err(ApiError::unprocessable(
            "machines_not_a_repository",
            "That project is not a git repository.",
        ));
    }
    Ok(folder)
}

/// Whether a project's remote answers without a prompt, here and on every
/// paired computer a root maps the project to.
async fn check_remote(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<RemoteBody>,
) -> ApiResult<Json<Value>> {
    let project = repo_here(&req.project)?;
    let me = crate::machines::machine_info(&state);
    let here = folders::remote_answers_async(PathBuf::from(&project)).await;
    let roots_now = roots::roots(&state);
    let peers: Vec<(String, String)> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .map(|p| (p.id.clone(), p.display_name().to_string()))
            .collect()
    });
    let asks = peers.iter().map(|(id, name)| {
        let st = state.clone();
        let mapped = folders::root_for(&roots_now, &me.id, id, &project);
        async move {
            let Some((root, rel, _)) = mapped else {
                return json!({
                    "machine": id, "name": name, "here": false, "state": "skipped",
                    "detail": format!("No folder maps this project to {name}."),
                });
            };
            let r = transport::request(
                &st,
                id,
                "POST",
                "/peer/setup/remote",
                Some(json!({ "root": root, "rel": rel })),
                folders::REMOTE_TIMEOUT + Duration::from_secs(10),
            )
            .await;
            let (state, detail) = match r {
                Ok((200, v)) => (
                    v["state"].as_str().unwrap_or("failed").to_string(),
                    v["detail"].as_str().unwrap_or("").to_string(),
                ),
                Ok((404, _)) => (
                    "skipped".to_string(),
                    format!("Update Mother Claude on {name} to check it there."),
                ),
                Ok((_, v)) => (
                    "failed".to_string(),
                    v.pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("It refused.")
                        .to_string(),
                ),
                Err(e) => ("failed".to_string(), format!("Can't reach {name}: {e}")),
            };
            json!({ "machine": id, "name": name, "here": false, "state": state, "detail": detail })
        }
    });
    let mut results = vec![json!({
        "machine": me.id, "name": me.name, "here": true,
        "state": here.state, "detail": here.detail,
    })];
    results.extend(futures_util::future::join_all(asks).await);
    Ok(Json(json!({
        "project": project,
        "origin": here.origin,
        "results": results,
    })))
}

// ─── the other computers ───────────────────────────────────────────────────────

async fn peer_accounts(
    ctx: ClientContext,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    granted(&state, &ctx, Grant::View)?;
    Ok(Json(accounts::peer_answer(&state)))
}

async fn peer_origins(
    ctx: ClientContext,
    State(state): State<AppState>,
    Query(q): Query<FreshQuery>,
) -> ApiResult<Json<Value>> {
    granted(&state, &ctx, Grant::View)?;
    Ok(Json(folders::peer_answer(&state, q.fresh).await))
}

/// The other computer says its checks (or an ask) changed: read them again.
async fn peer_changed(ctx: ClientContext, State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let machine = granted(&state, &ctx, Grant::View)?;
    let st = state.clone();
    tokio::spawn(async move { super::fetch_peer_checks_of(&st, &machine).await });
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerRemoteBody {
    root: String,
    rel: String,
}

/// Whether the remote of this computer's folder `rel` in root `root`
/// answers without a prompt (read-only: `git ls-remote`).
async fn peer_remote(
    ctx: ClientContext,
    State(state): State<AppState>,
    Json(req): Json<PeerRemoteBody>,
) -> ApiResult<Json<Value>> {
    granted(&state, &ctx, Grant::View)?;
    let folder = asks::folder_in_root(&state, &req.root, &req.rel)
        .map_err(|e| ApiError::unprocessable("machines_bad_folder", e))?;
    if !folder.join(".git").exists() {
        return Ok(Json(json!({
            "state": "failed",
            "detail": "The project is not on this computer: clone it there first.",
        })));
    }
    let _slot = folders::remote_slot(&state).ok_or_else(|| {
        ApiError::conflict(
            "machines_busy",
            "This computer is checking remotes already: try again in a moment.",
        )
    })?;
    let r = folders::remote_answers_async(folder).await;
    Ok(Json(json!({ "state": r.state, "detail": r.detail })))
}
