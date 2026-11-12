//! The routes of copies (docs/PLAN-MACHINES.md §7, `machines::copies`).
//!
//! This computer's dashboard (`/api`, merged into `machines::routes`):
//!
//! | Route | Who | Does |
//! |---|---|---|
//! | `GET /api/sessions/{id}/copies` | Authed | every paired computer, whether the session keeps a copy there (its own choice or its project's) and how it stands |
//! | `PUT /api/sessions/{id}/copies` | Authed | `{machines: [id…]}` its own choice, `{machines: null}` its project's; a computer left out drops its copy |
//! | `GET /api/machines/copies` | ManageData | `{held, kept, capBytes, used}`: the copies held here, and this computer's kept elsewhere |
//! | `DELETE /api/machines/copies/{id}` | ManageData | `held.<computer>.<session>` removes a copy held here; `kept.<computer>.<session>` stops keeping one there (and drops it) |
//!
//! Another computer (`/peer`, each needing this computer's `move` grant for
//! it on the path the request came over, and Machines on here — but for the
//! `DELETE`, which only takes a copy away): `POST /peer/copies/{session}/
//! offer`, `PUT /peer/copies/{session}/rounds/{round}?offset=&total=`,
//! `POST /peer/copies/{session}/rounds/{round}/apply`, `GET|DELETE
//! /peer/copies/{session}`.

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::source;
use super::store::Kept;
use super::wire;
use crate::machines::grants::Grant;
use crate::machines::{self as m, lease};
use crate::server::auth::{Authed, ManageData};
use crate::server::context::{ClientContext, Principal};
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// The largest offer or round description (a session with many files).
const DESCRIBE_MAX: usize = 32 * 1024 * 1024;

/// The dashboard's copies routes.
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/sessions/{id}/copies", get(get_session).put(put_session))
        .route("/machines/copies", get(get_all))
        .route("/machines/copies/{id}", delete(remove))
}

/// The other computers' copies routes.
pub fn peer_router() -> Router<AppState> {
    Router::new()
        .route("/peer/copies/{session}", get(peer_state).delete(peer_drop))
        .route(
            "/peer/copies/{session}/offer",
            post(peer_offer).layer(DefaultBodyLimit::max(DESCRIBE_MAX)),
        )
        .route(
            "/peer/copies/{session}/rounds/{round}",
            put(peer_chunk).layer(DefaultBodyLimit::max(
                crate::machines::chunks::CHUNK + 64 * 1024,
            )),
        )
        .route(
            "/peer/copies/{session}/rounds/{round}/apply",
            post(peer_apply).layer(DefaultBodyLimit::max(DESCRIBE_MAX)),
        )
}

// ─── this computer's dashboard ─────────────────────────────────────────────────

async fn get_session(
    Authed(_): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    crate::claude::ids::require_session_id(&id)?;
    Ok(Json(source::session_view(&state, &id).await))
}

#[derive(Debug, Default, Deserialize)]
struct SessionCopies {
    /// The computers it keeps a copy on; `null`: as its project says.
    #[serde(default)]
    machines: Option<Vec<String>>,
}

/// Its copies there go (best effort: an offline computer keeps it until its
/// cap or its desktop takes it away).
async fn drop_there(state: &AppState, session: &str, machine: &str) {
    let _ = m::moves::call(
        state,
        machine,
        "DELETE",
        &format!("/peer/copies/{session}"),
        None,
        "",
        std::time::Duration::from_secs(20),
    )
    .await;
}

async fn put_session(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<SessionCopies>,
) -> ApiResult<Json<Value>> {
    crate::claude::ids::require_session_id(&id)?;
    m::using(&state)?;
    lease::require_here(&state, &id)?;
    if state.find_session(&id).await.is_none() && state.index.transcript_path(&id).is_none() {
        return Err(ApiError::no_such_session(&id));
    }
    let chosen = match body.machines {
        Some(list) => {
            if list.len() > 50 {
                return Err(ApiError::unprocessable(
                    "invalid_copies",
                    "At most 50 computers.",
                ));
            }
            let mut out: Vec<String> = Vec::new();
            for mid in list {
                if !m::ids::is_machine_id(&mid)
                    || !state.machines.store_read(|f| f.peer(&mid).is_some())
                {
                    return Err(ApiError::not_found(
                        "machines_unknown",
                        "Not a paired computer.",
                    ));
                }
                if !out.contains(&mid) {
                    out.push(mid);
                }
            }
            Some(out)
        }
        None => None,
    };
    let (before, _, _) = source::choice_of(&state, &id).await;
    let now = m::now();
    state.machines.copies.out_write(|f| match &chosen {
        Some(ms) => {
            f.sessions.insert(
                id.clone(),
                Kept {
                    machines: ms.clone(),
                    updated_at: now,
                },
            );
        }
        None => {
            f.sessions.remove(&id);
        }
    });
    let (after, explicit, _) = source::choice_of(&state, &id).await;
    // The computers it no longer keeps a copy on drop theirs.
    let gone: Vec<String> = before.into_iter().filter(|b| !after.contains(b)).collect();
    state.machines.copies.out_write(|f| {
        if let Some(per) = f.status.get_mut(&id) {
            per.retain(|mid, _| after.contains(mid));
        }
        f.status.retain(|_, per| !per.is_empty());
    });
    for mid in &gone {
        let (st, sid, mid) = (state.clone(), id.clone(), mid.clone());
        tokio::spawn(async move { drop_there(&st, &sid, &mid).await });
    }
    state
        .devices
        .audit
        .record(crate::server::devices::audit::AuditEntry {
            detail: Some(json!({
                "session": id,
                "machines": after,
                "explicit": explicit,
                "dropped": gone,
            })),
            ..crate::server::devices::audit::AuditEntry::from_ctx(&ctx, "machine.copies.set")
        });
    crate::server::auth::note_device_action(
        &state,
        &ctx,
        &if after.is_empty() {
            "stopped keeping copies of a session on other computers".to_string()
        } else {
            format!(
                "keeps a copy of a session on {}",
                after
                    .iter()
                    .map(|m| m::peer_name(&state, m))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
    );
    super::publish(&state);
    Ok(Json(source::session_view(&state, &id).await))
}

/// `GET /api/machines/copies`.
pub async fn view(state: &AppState) -> Value {
    let held = super::target::held_view(state);
    let kept = source::kept_view(state).await;
    let used: serde_json::Map<String, Value> = state.machines.copies.held_read(|i| {
        let mut per: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        for h in i.copies.values() {
            *per.entry(h.machine.clone()).or_default() += h.bytes;
        }
        per.into_iter().map(|(k, v)| (k, json!(v))).collect()
    });
    json!({
        "enabled": state.machines.enabled(),
        "held": held,
        "kept": kept,
        "capBytes": super::target::cap(state),
        "used": used,
    })
}

async fn get_all(ManageData(_): ManageData, State(state): State<AppState>) -> Json<Value> {
    Json(view(&state).await)
}

/// `held.<machine>.<session>` or `kept.<machine>.<session>`.
fn parse_id(id: &str) -> Option<(&str, &str, &str)> {
    let mut parts = id.splitn(3, '.');
    let kind = parts.next()?;
    let machine = parts.next()?;
    let session = parts.next()?;
    (matches!(kind, "held" | "kept")
        && m::ids::is_machine_id(machine)
        && crate::claude::ids::is_session_id(session))
    .then_some((kind, machine, session))
}

async fn remove(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let (kind, machine, session) =
        parse_id(&id).ok_or_else(|| ApiError::not_found("copy_not_found", "No such copy."))?;
    if kind == "held" {
        super::target::remove_here(&state, &ctx, machine, session).await?;
        crate::server::auth::note_device_action(
            &state,
            &ctx,
            &format!(
                "removed a copy of {}'s session from this computer",
                m::peer_name(&state, machine)
            ),
        );
    } else {
        let (now_on, explicit, _) = source::choice_of(&state, session).await;
        if !now_on.iter().any(|x| x == machine) && !explicit {
            return Err(ApiError::not_found("copy_not_found", "No such copy."));
        }
        let rest: Vec<String> = now_on.into_iter().filter(|x| x != machine).collect();
        let at = m::now();
        state.machines.copies.out_write(|f| {
            f.sessions.insert(
                session.to_string(),
                Kept {
                    machines: rest.clone(),
                    updated_at: at,
                },
            );
            if let Some(per) = f.status.get_mut(session) {
                per.remove(machine);
            }
            f.status.retain(|_, per| !per.is_empty());
        });
        drop_there(&state, session, machine).await;
        state
            .devices
            .audit
            .record(crate::server::devices::audit::AuditEntry {
                device_id: Some(machine.to_string()),
                device_name: Some(m::peer_name(&state, machine)),
                detail: Some(json!({ "session": session })),
                ..crate::server::devices::audit::AuditEntry::from_ctx(&ctx, "machine.copy.stopped")
            });
        crate::server::auth::note_device_action(
            &state,
            &ctx,
            &format!(
                "stopped keeping a copy of a session on {}",
                m::peer_name(&state, machine)
            ),
        );
    }
    super::publish(&state);
    Ok(Json(view(&state).await))
}

// ─── the other computers ───────────────────────────────────────────────────────

/// A paired computer holding this computer's `move` grant for the path the
/// request came over: `(machine, lan)`.
fn copier(state: &AppState, ctx: &ClientContext) -> ApiResult<(String, bool)> {
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

/// [`copier`], while Machines is on here (`403 machines_off`).
fn copying_here(state: &AppState, ctx: &ClientContext) -> ApiResult<(String, bool)> {
    if matches!(ctx.principal, Principal::Peer { .. }) {
        m::serving(state)?;
    }
    copier(state, ctx)
}

async fn peer_state(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(session): Path<String>,
) -> ApiResult<Json<wire::CopyThere>> {
    let (from, _) = copying_here(&state, &ctx)?;
    crate::claude::ids::require_session_id(&session)?;
    Ok(Json(super::target::state_of(&state, &from, &session)))
}

/// The source keeps no copy here any more: served while Machines is off
/// too (it only takes a copy away).
async fn peer_drop(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(session): Path<String>,
) -> ApiResult<Json<Value>> {
    let (from, _) = copier(&state, &ctx)?;
    crate::claude::ids::require_session_id(&session)?;
    Ok(Json(super::target::dropped(&state, &from, &session).await))
}

async fn peer_offer(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path(session): Path<String>,
    Body(offer): Body<wire::Offer>,
) -> ApiResult<Json<wire::Holdings>> {
    let (from, lan) = copying_here(&state, &ctx)?;
    Ok(Json(
        super::target::offer(&state, &from, lan, &session, offer).await?,
    ))
}

#[derive(Deserialize)]
struct ChunkQuery {
    offset: u64,
    total: u64,
}

async fn peer_chunk(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path((session, round)): Path<(String, String)>,
    Query(q): Query<ChunkQuery>,
    body: axum::body::Bytes,
) -> Response {
    let answer = async {
        let (from, _) = copying_here(&state, &ctx)?;
        super::target::put_chunk(&state, &from, &session, &round, q.offset, q.total, body).await
    }
    .await;
    match answer {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn peer_apply(
    ctx: ClientContext,
    State(state): State<AppState>,
    Path((session, round)): Path<(String, String)>,
    Body(body): Body<wire::Apply>,
) -> ApiResult<Json<wire::Applied>> {
    let (from, _) = copying_here(&state, &ctx)?;
    Ok(Json(
        super::target::apply(&state, &from, &session, &round, body).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copys_id_names_its_kind_computer_and_session() {
        let m = "0123456789abcdef0123456789abcdef";
        let s = "0f1e2d3c-4b5a-4968-8776-655443322110";
        assert_eq!(parse_id(&format!("held.{m}.{s}")), Some(("held", m, s)));
        assert_eq!(parse_id(&format!("kept.{m}.{s}")), Some(("kept", m, s)));
        for bad in [
            format!("other.{m}.{s}"),
            format!("held.{s}.{m}"),
            format!("held.{m}"),
            format!("held.../{m}.{s}"),
        ] {
            assert!(parse_id(&bad).is_none(), "{bad}");
        }
    }
}
