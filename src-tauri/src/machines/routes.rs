//! Machines' routes (docs/PLAN-MACHINES.md §7).
//!
//! * `/api/machines/*` — this machine's dashboard: reading is `Authed` (a
//!   phone sees Machines read-only); everything that changes anything is
//!   `DesktopOnly` (pairing, grants, revoking, settings, fixes, the test).
//! * `/api/roots` — the roots (§5.12): `GET` and `PUT {upsert, remove}`,
//!   the desktop or a paired device (`ManageData`), as folder pairs are.
//! * `/peer/*` — the other machines, on the `Peer` ingress only
//!   (`peer_app`): pairing (`/peer/pair/*`, the one unpaired key pairing
//!   right now), then a paired machine's hello, setup checks, asks and
//!   settings offer, the readiness test's transfer and the roots (`GET`
//!   with `view`, `POST` with `move`: roots steer where a move lands), and
//!   the setup's rest (`setup::routes`: accounts, origins, the code test);
//!   the session stream (`/ws`), the allow-listed session routes (`/api/*`,
//!   [`peer_route_grant`]), a move's target half (`move_routes`), code
//!   sync's (`sync::routes`) and runs for another computer (`work::routes`,
//!   the `run` grant). Nothing else is mounted there.
//!
//! While Machines is off here, another computer gets only what pairing and
//! the setup need: `/ws`, `/api/*` and `/peer/moves/*` answer `403
//! machines_off` (`super::serving`); this computer's proxy and moves answer
//! `409 machines_off` (`super::using`).

use axum::body::Body as HttpBody;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, patch, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::grants::{Grant, Grants};
use super::protocol::Hello;
use super::store::RelaySetting;
use super::transport::{self, PathKind};
use super::{intro, pairing, revocation, roots, setup};
use crate::server::auth::{Authed, DesktopOnly, ManageData};
use crate::server::context::{ClientContext, Ingress, PeerCaller, Principal};
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// `/api/machines/*` (merged into the protected API router).
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/machines", get(get_view))
        .route("/machines/self", patch(rename_self))
        .route("/machines/pair", post(pair).delete(cancel_pair))
        .route("/machines/join", post(join))
        .route("/machines/confirm", post(confirm))
        .route("/machines/nearby", get(nearby))
        .route("/machines/setup", get(get_setup))
        .route("/machines/setup/check", post(recheck))
        .route("/machines/setup/fix", post(fix))
        .route("/machines/setup/ask", post(ask))
        .route("/machines/setup/progress", put(progress))
        .route("/machines/test", post(test))
        .route("/machines/settings", get(get_settings).put(put_settings))
        .route("/machines/settings/offer", post(answer_offer))
        .route("/machines/enable", post(enable))
        // Three computers and more (Phase 5): introductions, revoke
        // everywhere.
        .route("/machines/introduce", post(introduce))
        .route("/machines/introductions/{id}", post(answer_introduction))
        .route("/machines/{id}/revoke-everywhere", post(revoke_everywhere))
        .route("/machines/{id}", patch(update_peer).delete(revoke_peer))
        .route(
            "/machines/{id}/proxy/{*rest}",
            any(proxy).layer(axum::extract::DefaultBodyLimit::max(PROXY_BODY_MAX)),
        )
        .route("/roots", get(get_roots).put(put_roots))
        // Moving sessions (Phase 3a).
        .merge(super::move_routes::api_router())
        // The setup's rest (Phase 3a): accounts, folders, asks, clones.
        .merge(super::setup::routes::api_router())
        // Code sync (§5.15).
        .merge(super::sync::routes::api_router())
        // Runs on other computers (Phase 4).
        .merge(super::work::routes::api_router())
        // Copies kept current (Phase 3b).
        .merge(super::copies::routes::api_router())
}

/// The largest request forwarded to another machine (a message with images).
const PROXY_BODY_MAX: usize = 32 * 1024 * 1024;

/// `/peer/*`.
pub fn peer_router() -> Router<AppState> {
    Router::new()
        .route("/peer/hello", post(peer_hello))
        .route("/peer/pair/join", post(peer_pair_join))
        .route("/peer/pair/reveal", post(peer_pair_reveal))
        .route("/peer/pair/confirm", post(peer_pair_confirm))
        .route("/peer/pair/status", get(peer_pair_status))
        .route("/peer/pair/cancel", post(peer_pair_cancel))
        .route(intro::DECLINE_PATH, post(peer_intro_decline))
        .route(intro::OFFER_PATH, post(peer_intro))
        .route(intro::WITHDRAW_PATH, post(peer_intro_withdraw))
        .route(revocation::PEER_PATH, post(peer_revoke))
        .route("/peer/setup", get(peer_setup))
        .route("/peer/setup/ask", post(peer_setup_ask))
        .route("/peer/settings", post(peer_settings))
        .route("/peer/bulk", get(peer_bulk))
        .route(roots::PEER_PATH, get(peer_get_roots).post(peer_post_roots))
        // A move's target half, and the leases (Phase 3a).
        .merge(super::move_routes::peer_router())
        // The setup's rest (Phase 3a): accounts, origins, the code test.
        .merge(super::setup::routes::peer_router())
        // Code sync's other half (§5.15).
        .merge(super::sync::routes::peer_router())
        // Running commands for another computer (Phase 4).
        .merge(super::work::routes::peer_router())
        // Copies of its sessions this computer keeps for it (Phase 3b).
        .merge(super::copies::routes::peer_router())
        // The paired machine's view of this one's sessions (plan §5.6).
        .route("/ws", get(peer_ws))
        // `/api/*` is served from the fallback, so no path parameter of this
        // router reaches the dashboard's handlers it forwards to.
        .fallback(peer_api)
}

/// The peer router with the hardening layers and the context middleware,
/// as every listener's router has (`server::router_for`).
pub fn peer_app(state: AppState) -> Router {
    let app =
        crate::server::security::apply(peer_router().with_state(state.clone()), Ingress::Peer);
    crate::server::context::layer(app, state, Ingress::Peer)
}

// ─── this machine's dashboard ────────────────────────────────────────────────────

async fn get_view(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(super::view(&state).await)
}

async fn view_json(state: &AppState) -> ApiResult<Json<Value>> {
    Ok(Json(super::view(state).await))
}

#[derive(Deserialize)]
struct Rename {
    name: String,
}

fn clean_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return Err(ApiError::unprocessable(
            "machines_bad_name",
            "A name has 1 to 40 characters.",
        ));
    }
    Ok(name.to_string())
}

async fn rename_self(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<Rename>,
) -> ApiResult<Json<Value>> {
    let name = clean_name(&req.name)?;
    state.machines.store_write(|f| f.name = Some(name));
    super::publish(&state, "self");
    view_json(&state).await
}

async fn pair(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    pairing::host_start(&state).await?;
    view_json(&state).await
}

async fn cancel_pair(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    pairing::cancel(&state).await;
    view_json(&state).await
}

#[derive(Deserialize)]
struct JoinBody {
    code: String,
    #[serde(default)]
    name: Option<String>,
}

async fn join(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<JoinBody>,
) -> ApiResult<Json<Value>> {
    if let Some(name) = req.name.as_deref().filter(|n| !n.trim().is_empty()) {
        let name = clean_name(name)?;
        state.machines.store_write(|f| f.name = Some(name));
    }
    pairing::join_start(&state, &req.code).await?;
    view_json(&state).await
}

#[derive(Deserialize)]
struct ConfirmBody {
    grants: Grants,
    #[serde(default)]
    digits: Option<String>,
}

async fn confirm(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<ConfirmBody>,
) -> ApiResult<Json<Value>> {
    pairing::confirm(&state, req.grants, req.digits).await?;
    view_json(&state).await
}

async fn nearby(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    // Listening needs the transport (and its mDNS).
    transport::ensure_started(&state)
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "machines_transport", e))?;
    let list: Vec<Value> = state
        .machines
        .nearby()
        .into_iter()
        .map(|n| json!({ "id": n.id, "name": n.name, "os": n.os }))
        .collect();
    Ok(Json(json!({ "machines": list })))
}

#[derive(Deserialize)]
struct SetupQuery {
    #[serde(default)]
    fresh: bool,
}

async fn get_setup(
    Authed(_): Authed,
    State(state): State<AppState>,
    Query(q): Query<SetupQuery>,
) -> Json<Value> {
    Json(setup::view(&state, q.fresh).await)
}

/// Runs every check again, here and on the paired machines; starts the
/// transport (the setup is when Machines comes into use).
async fn recheck(DesktopOnly(_): DesktopOnly, State(state): State<AppState>) -> Json<Value> {
    if let Err(e) = transport::ensure_started(&state).await {
        tracing::warn!(error = %e, "Machines transport did not start for the checks");
    }
    Json(setup::view(&state, true).await)
}

#[derive(Deserialize)]
struct FixBody {
    check: String,
}

async fn fix(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<FixBody>,
) -> ApiResult<Json<Value>> {
    setup::fix_check(&state, &req.check).await?;
    Ok(Json(setup::view(&state, false).await))
}

#[derive(Deserialize)]
struct AskBody {
    machine: String,
    /// A check's id, or an asked action (`signIn`, `addAccount`, `clone`)
    /// with its `account`, or `origin`, `root` and `rel`.
    #[serde(flatten)]
    ask: setup::asks::AskRequest,
}

/// Asks another machine to show one of its fixes, or an asked action, to
/// whoever is at it: nothing changes there until someone confirms it there.
async fn ask(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<AskBody>,
) -> ApiResult<Json<Value>> {
    if !super::ids::is_machine_id(&req.machine) || req.ask.check.len() > 80 {
        return Err(ApiError::bad_request("Malformed request."));
    }
    let (status, body) = transport::request(
        &state,
        &req.machine,
        "POST",
        "/peer/setup/ask",
        Some(serde_json::to_value(&req.ask).unwrap_or_default()),
        transport::REQUEST_TIMEOUT,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "machines_unreachable", e))?;
    if status >= 300 {
        let code = body
            .pointer("/error/code")
            .and_then(Value::as_str)
            .unwrap_or("machines_refused");
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "machines_refused",
            body.pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("The other computer refused."),
        )
        .with("reason", code));
    }
    // What it keeps of it, until its checks say how it went.
    let kept = body
        .get("ask")
        .and_then(|a| serde_json::from_value::<setup::asks::Ask>(a.clone()).ok())
        .filter(|a| setup::asks::is_ask_id(&a.id));
    if let Some(a) = &kept {
        setup::note_asked(&state, &req.machine, a.clone());
    }
    Ok(Json(json!({ "asked": true, "ask": kept })))
}

#[derive(Deserialize)]
struct ProgressBody {
    step: Option<u8>,
}

async fn progress(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<ProgressBody>,
) -> ApiResult<Json<Value>> {
    let step = req.step.filter(|s| (1..=7).contains(s));
    state.machines.store_write(|f| f.setup_step = step);
    super::publish(&state, "setup");
    view_json(&state).await
}

#[derive(Deserialize)]
struct TestBody {
    #[serde(default)]
    machine: Option<String>,
}

async fn test(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<TestBody>,
) -> ApiResult<Json<Value>> {
    if let Some(m) = &req.machine {
        if !state.machines.store_read(|f| f.peer(m).is_some()) {
            return Err(ApiError::not_found(
                "machines_unknown",
                "Not a paired computer.",
            ));
        }
    }
    transport::ensure_started(&state)
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "machines_transport", e))?;
    Ok(Json(setup::readiness(&state, req.machine.as_deref()).await))
}

/// The settings with every project's own (`GET`/`PUT /api/machines/settings`).
fn settings_answer(state: &AppState) -> Value {
    let mut v = super::settings_view(&state.machines.settings());
    v["projects"] = json!(setup::options::view(state));
    v
}

async fn get_settings(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(settings_answer(&state))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsBody {
    #[serde(default)]
    relay: Option<RelaySetting>,
    #[serde(default)]
    keep_awake: Option<bool>,
    /// The code options every move starts from (offered to the others).
    #[serde(default)]
    code: Option<setup::options::CodeInput>,
    /// Projects' own code options and recipes (this machine's only).
    #[serde(default)]
    projects: Option<setup::options::ProjectsChange>,
    /// What this computer keeps at most of each other computer's copies.
    #[serde(default)]
    copies_cap_bytes: Option<u64>,
}

async fn put_settings(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<SettingsBody>,
) -> ApiResult<Json<Value>> {
    if let Some(r) = &req.relay {
        r.validate()
            .map_err(|m| ApiError::unprocessable("machines_bad_relay", m).with("field", "relay"))?;
    }
    if let Some(cap) = req.copies_cap_bytes {
        if !(super::copies::MIN_CAP..=super::copies::MAX_CAP).contains(&cap) {
            return Err(ApiError::unprocessable(
                "machines_bad_cap",
                "Keep between 64 MB and 4 TB of each computer's copies.",
            )
            .with("field", "copiesCapBytes"));
        }
    }
    // A project's files to carry need the dangerous right: checked (and
    // applied, all or nothing) before anything else changes.
    if let Some(p) = &req.projects {
        setup::options::change(&state, &ctx, p)?;
    }
    let before = state.machines.settings();
    state.machines.store_write(|f| {
        if let Some(r) = req.relay.clone() {
            f.settings.relay = r;
        }
        if let Some(k) = req.keep_awake {
            f.settings.keep_awake = k;
        }
        if let Some(c) = &req.code {
            f.settings.code = c.over(&f.settings.code);
        }
        if let Some(cap) = req.copies_cap_bytes {
            f.settings.copies_cap_bytes = cap;
        }
    });
    let after = state.machines.settings();
    super::awake::update(&state);
    let relay_changed = !after.relay.same_as(&before.relay);
    if relay_changed {
        // A new relay means a new endpoint; then the others are offered it.
        let was_running = transport::current(&state).await.is_some();
        transport::stop(&state).await;
        if was_running {
            if let Err(e) = transport::ensure_started(&state).await {
                tracing::warn!(error = %e, "Machines transport did not restart");
            }
        }
    }
    if relay_changed || !after.code.same_as(&before.code) {
        offer_settings(&state).await;
    }
    super::publish(&state, "settings");
    Ok(Json(settings_answer(&state)))
}

/// Offers this machine's shared settings (the relay, the code options) to
/// every online paired machine.
pub async fn offer_settings(state: &AppState) {
    let settings = state.machines.settings();
    let peers: Vec<String> = state
        .machines
        .store_read(|f| f.peers.iter().map(|p| p.id.clone()).collect());
    let body = json!({ "relay": settings.relay, "code": settings.code });
    let sends = peers.iter().map(|id| {
        transport::request(
            state,
            id,
            "POST",
            "/peer/settings",
            Some(body.clone()),
            transport::REQUEST_TIMEOUT,
        )
    });
    futures_util::future::join_all(sends).await;
}

#[derive(Deserialize)]
struct OfferAnswer {
    accept: bool,
}

async fn answer_offer(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<OfferAnswer>,
) -> ApiResult<Json<Value>> {
    let Some(offer) = state.machines.offer() else {
        return Err(ApiError::conflict(
            "machines_no_offer",
            "Nothing was suggested.",
        ));
    };
    state.machines.set_offer(None);
    if req.accept {
        let before = state.machines.settings();
        let relay_changes = offer.relay.validate().is_ok() && !offer.relay.same_as(&before.relay);
        state.machines.store_write(|f| {
            if relay_changes {
                f.settings.relay = offer.relay.clone();
            }
            if let Some(c) = &offer.code {
                // The choices only: what a newer build keeps here stays.
                f.settings.code.transfer = c.transfer;
                f.settings.code.apply = c.apply;
                f.settings.code.keep_remote_refs = c.keep_remote_refs;
            }
        });
        if relay_changes {
            let was_running = transport::current(&state).await.is_some();
            transport::stop(&state).await;
            if was_running {
                let _ = transport::ensure_started(&state).await;
            }
        }
    }
    super::publish(&state, "settings");
    view_json(&state).await
}

#[derive(Deserialize)]
struct EnableBody {
    enabled: bool,
}

/// Turns Machines on (refused while a blocker is left) or off.
async fn enable(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<EnableBody>,
) -> ApiResult<Json<Value>> {
    if req.enabled {
        setup::view(&state, true).await;
        let blockers = setup::blockers(&state);
        if !blockers.is_empty() {
            let list: Vec<Value> = blockers
                .iter()
                .map(|c| json!({ "id": c.id, "title": c.title, "detail": c.detail }))
                .collect();
            return Err(ApiError::conflict(
                "machines_blocked",
                format!(
                    "{} {} left: fix {} first.",
                    blockers.len(),
                    if blockers.len() == 1 {
                        "required check is"
                    } else {
                        "required checks are"
                    },
                    if blockers.len() == 1 { "it" } else { "them" }
                ),
            )
            .with("blockers", list));
        }
    }
    super::set_enabled(&state, req.enabled);
    view_json(&state).await
}

#[derive(Deserialize)]
struct PeerUpdate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    grants: Option<Grants>,
}

async fn update_peer(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(req): Body<PeerUpdate>,
) -> ApiResult<Json<Value>> {
    if !super::ids::is_machine_id(&id) || !state.machines.store_read(|f| f.peer(&id).is_some()) {
        return Err(ApiError::not_found(
            "machines_unknown",
            "Not a paired computer.",
        ));
    }
    let alias = match req.name.as_deref() {
        Some(n) if n.trim().is_empty() => Some(None),
        Some(n) => Some(Some(clean_name(n)?)),
        None => None,
    };
    let grants = req.grants.map(Grants::normalized);
    let name = state.machines.store_write(|f| {
        let p = f.peer_mut(&id)?;
        if let Some(a) = alias {
            p.alias = a;
        }
        if let Some(g) = grants {
            p.grants = g;
        }
        Some(p.display_name().to_string())
    });
    if grants.is_some() {
        super::audit(&state, "machine.grants", &id, name.as_deref().unwrap_or(""));
        // No `run` grant left: what it runs here stops.
        super::work::grants_changed(&state, &id);
        // Tell it at once what it may now do here.
        let key = state
            .machines
            .store_read(|f| f.peer(&id).map(|p| p.key.clone()));
        if let Some(key) = key {
            tokio::spawn(super::greet(state.clone(), id.clone(), key));
        }
    }
    super::publish(&state, "peers");
    view_json(&state).await
}

async fn revoke_peer(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !super::ids::is_machine_id(&id) || !super::revoke(&state, &id) {
        return Err(ApiError::not_found(
            "machines_unknown",
            "Not a paired computer.",
        ));
    }
    view_json(&state).await
}

/// *Revoke everywhere*: revokes the computer here and asks every other
/// paired computer to (`revocation`); answers the view, whose
/// `revocations` say which took it and which will when they are back.
async fn revoke_everywhere(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !super::ids::is_machine_id(&id) {
        return Err(ApiError::not_found(
            "machines_unknown",
            "Not a paired computer.",
        ));
    }
    revocation::revoke_everywhere(&state, &id).await?;
    view_json(&state).await
}

#[derive(Deserialize)]
struct IntroduceBody {
    /// The computer introduced (it dials)…
    machine: String,
    /// …to this one (it waits).
    with: String,
}

/// Introduces two paired computers to each other: each shows it, and pairs
/// only when both desktops accept and confirm the six digits (`intro`).
async fn introduce(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<IntroduceBody>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        intro::introduce(&state, &req.machine, &req.with).await?,
    ))
}

#[derive(Deserialize)]
struct IntroAnswer {
    accept: bool,
}

/// This desktop accepts an introduction (its side of the pairing starts)
/// or declines it (the other computer is told); answers the view.
async fn answer_introduction(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(req): Body<IntroAnswer>,
) -> ApiResult<Json<Value>> {
    if !intro::is_intro_id(&id) {
        return Err(ApiError::not_found(
            "intro_unknown",
            "No such introduction here.",
        ));
    }
    intro::answer(&state, &id, req.accept).await?;
    view_json(&state).await
}

// ─── driving another machine through this one ───────────────────────────────────

/// `/api/machines/{id}/proxy/<path>`: forwards a dashboard call to the paired
/// machine's `/api/<path>` with who acts here (`X-MC-Acting`); the other
/// machine decides with its grants and that person's right. Refused while
/// Machines is off here (`409 machines_off`).
async fn proxy(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path((id, rest)): Path<(String, String)>,
    req: axum::extract::Request,
) -> Response {
    if !super::ids::is_machine_id(&id) || !state.machines.store_read(|f| f.peer(&id).is_some()) {
        return ApiError::not_found("machines_unknown", "Not a paired computer.").into_response();
    }
    if let Err(e) = super::using(&state) {
        return e.into_response();
    }
    let method = req.method().clone();
    // The local-script token never starts, cancels, undoes or retries a
    // move here; nor on another computer through this one (it may read).
    if ctx.principal == Principal::LocalScript && super::move_routes::changes_a_move(&method, &rest)
    {
        if let Err(e) = super::move_routes::may_move(&ctx, None) {
            return e.into_response();
        }
    }
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let content_type = req
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = match axum::body::to_bytes(req.into_body(), PROXY_BODY_MAX).await {
        Ok(b) => b,
        Err(_) => {
            return ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "That is too large to send.",
            )
            .into_response()
        }
    };
    let mut headers = vec![(
        super::acting::HEADER,
        super::acting::header_for(&state, &ctx),
    )];
    if let Some(ct) = content_type {
        headers.push(("content-type", ct));
    }
    let path = format!("/api/{rest}{query}");
    match transport::request_raw(
        &state,
        &id,
        method.as_str(),
        &path,
        &headers,
        body,
        PROXY_TIMEOUT,
    )
    .await
    {
        // A 401 means "your credential here is gone" to every client (a phone
        // unpairs itself); the other computer refusing is not that.
        Ok((401, _, _)) => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "machine_refused",
            format!("{} refused this computer.", super::peer_name(&state, &id)),
        )
        .with("machine", id)
        .into_response(),
        Ok((status, content_type, bytes)) => {
            let mut resp = Response::new(HttpBody::from(bytes));
            *resp.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            if let Some(ct) = content_type.and_then(|c| axum::http::HeaderValue::from_str(&c).ok())
            {
                resp.headers_mut()
                    .insert(axum::http::header::CONTENT_TYPE, ct);
            }
            resp
        }
        Err(e) => {
            let name = super::peer_name(&state, &id);
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "machine_unreachable",
                format!("{name} can't be reached right now: {e}"),
            )
            .with("machine", id)
            .into_response()
        }
    }
}

const PROXY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

// ─── roots (§5.12) ───────────────────────────────────────────────────────────────

/// `GET /api/roots`: this machine's id, the machines a root may name and the
/// roots (`roots::view`).
async fn get_roots(
    ManageData(_): ManageData,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    tokio::task::spawn_blocking(move || roots::view(&state))
        .await
        .map(Json)
        .map_err(|e| ApiError::internal(e.to_string()))
}

/// `PUT /api/roots {upsert: [{id?, name, paths}], remove: [id]}`: all or
/// nothing; answers the view (`roots::change`).
async fn put_roots(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(req): Body<roots::Change>,
) -> ApiResult<Json<Value>> {
    tokio::task::spawn_blocking(move || roots::change(&state, &ctx, &req))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map(Json)
}

/// What another machine may reach of this one's API, and the grant each
/// needs. Everything else is not mounted for it (plan §5.6).
pub fn peer_route_grant(method: &axum::http::Method, path: &str) -> Option<Grant> {
    use axum::http::Method;
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    let read = *method == Method::GET;
    let write = *method == Method::POST || *method == Method::DELETE;
    let grant = match (segs.as_slice(), read, write) {
        (["sessions"], true, _) => Grant::View,
        (["sessions"], _, true) if *method == Method::POST => Grant::Control,
        (["sessions", _], true, _) => Grant::View,
        (
            ["sessions", _, "transcript" | "agents" | "diff" | "file-patch" | "files" | "commands" | "context"
            | "mcp" | "terminal"],
            true,
            _,
        ) => Grant::View,
        (["sessions", _, "media", _, _, _], true, _) => Grant::View,
        (["sessions", _, "tasks", _, "output"], true, _) => Grant::View,
        (["agents" | "projects" | "defaults" | "inbox" | "usage"], true, _) => Grant::View,
        (
            ["sessions", _, "message" | "uploads" | "interrupt" | "model" | "mode" | "effort" | "ultracode"
            | "thinking" | "rename" | "respond" | "continue" | "stop" | "respawn" | "keys"],
            _,
            true,
        ) if *method == Method::POST => Grant::Control,
        (["sessions", _, "tasks", _, "stop"], _, true) if *method == Method::POST => Grant::Control,
        (["sessions", _, "queue", _], _, true) if *method == Method::DELETE => Grant::Control,
        (["sessions", _, "queue", _, "send-now"], _, true) if *method == Method::POST => {
            Grant::Control
        }
        // Moving a session away from here, or watching one move (Phase 3a).
        (["sessions", _, "move"] | ["sessions", _, "move", "preflight"], _, true)
            if *method == Method::POST =>
        {
            Grant::Move
        }
        (["moves"] | ["moves", _], true, _) => Grant::View,
        (["moves", _, "cancel" | "undo" | "retry"], _, true) if *method == Method::POST => {
            Grant::Move
        }
        // Keeping a copy of one of its sessions (Phase 3b): the first half
        // of a move.
        (["sessions", _, "copies"], true, _) => Grant::View,
        (["sessions", _, "copies"], _, _) if *method == Method::PUT => Grant::Move,
        _ => return None,
    };
    Some(grant)
}

// ─── the other machines ──────────────────────────────────────────────────────────

/// Another machine's call on this one's API: allow-listed, while Machines is
/// on here, with its grant, then served by the very router the dashboard
/// uses.
async fn peer_api(
    ctx: ClientContext,
    State(state): State<AppState>,
    mut req: axum::extract::Request,
) -> Response {
    let (machine, lan) = match paired(&ctx) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let full = req.uri().path().to_string();
    let Some(path) = full.strip_prefix("/api/").map(|p| format!("/{p}")) else {
        return ApiError::not_found("not_found", "No such route for another computer.")
            .into_response();
    };
    let Some(grant) = peer_route_grant(req.method(), &path) else {
        return ApiError::not_found("not_found", "No such route for another computer.")
            .into_response();
    };
    if let Err(e) = super::serving(&state) {
        return e.into_response();
    }
    if !state.machines.allows(&machine, grant, lan) {
        return ApiError::forbidden(
            "grant_missing",
            format!(
                "This computer has not given yours the \"{}\" permission.",
                grant.as_str()
            ),
        )
        .with("grant", grant.as_str())
        .into_response();
    }
    let rewritten = match req.uri().query() {
        Some(q) => format!("{path}?{q}"),
        None => path,
    };
    match rewritten.parse::<axum::http::Uri>() {
        Ok(uri) => *req.uri_mut() = uri,
        Err(_) => return ApiError::bad_request("Malformed path.").into_response(),
    }
    use tower::ServiceExt;
    match state.machines.api_app(&state).oneshot(req).await {
        Ok(resp) => resp,
        Err(never) => match never {},
    }
}

/// The other machine subscribes to this one's sessions (while Machines is
/// on here).
async fn peer_ws(
    ctx: ClientContext,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    ws: axum::extract::WebSocketUpgrade,
) -> Response {
    let allowed = paired(&ctx)
        .and_then(|_| super::serving(&state))
        .and_then(|_| granted(&state, &ctx, Grant::View));
    if let Err(e) = allowed {
        return e.into_response();
    }
    crate::server::ws::ws_handler(ws, State(state), ctx, headers).await
}

/// The caller as the transport authenticated it (never from the request).
fn caller_of(caller: Option<Extension<PeerCaller>>) -> ApiResult<PeerCaller> {
    caller.map(|Extension(c)| c).ok_or_else(|| {
        ApiError::forbidden("peer_only", "Only another of your computers can call this.")
    })
}

/// A paired machine (any grant), and whether this request came over the
/// local network.
fn paired(ctx: &ClientContext) -> ApiResult<(String, bool)> {
    match &ctx.principal {
        Principal::Peer { machine, lan, .. } => Ok((machine.clone(), *lan)),
        _ => Err(ApiError::forbidden(
            "peer_only",
            "Only a paired computer can call this.",
        )),
    }
}

/// A paired machine holding `grant` for this request's path.
pub(crate) fn granted(state: &AppState, ctx: &ClientContext, grant: Grant) -> ApiResult<String> {
    let (machine, lan) = paired(ctx)?;
    if !state.machines.allows(&machine, grant, lan) {
        return Err(ApiError::forbidden(
            "grant_missing",
            format!(
                "This computer has not given yours the \"{}\" permission.",
                grant.as_str()
            ),
        )
        .with("grant", grant.as_str()));
    }
    Ok(machine)
}

async fn peer_hello(
    ctx: ClientContext,
    State(state): State<AppState>,
    caller: Option<Extension<PeerCaller>>,
    Json(hello): Json<Hello>,
) -> ApiResult<Json<Value>> {
    let (machine, lan) = paired(&ctx)?;
    let caller = caller_of(caller)?;
    if hello.machine.id != machine || hello.machine.key != caller.key {
        return Err(ApiError::forbidden(
            "peer_mismatch",
            "That is not the computer this key was paired as.",
        ));
    }
    let conn = state.machines.live_connection(&caller.key);
    super::learn(&state, &hello, conn.as_ref());
    // Its leases (with this machine's `move` grant for it): which machine
    // each session lives on.
    let changed = super::lease::accept(&state, &machine, lan, &hello.leases).await;
    super::moves::on_leases_changed(&state, &changed).await;
    let peer = state
        .machines
        .store_read(|f| f.peer(&machine).cloned())
        .ok_or_else(|| ApiError::forbidden("peer_only", "Not paired."))?;
    // The same hello it sends (its roots' digest, its leases).
    let reply: Hello = super::hello_for(&state, &peer);
    roots::after_hello(&state, &machine, &hello);
    // A *Revoke everywhere* it has not taken yet: it is back.
    if revocation::pending_for(&state, &machine) {
        let st = state.clone();
        tokio::spawn(async move { revocation::deliver_to(&st, &machine).await });
    }
    Ok(Json(serde_json::to_value(reply).unwrap_or(Value::Null)))
}

/// A paired computer introduces another one (its `view` grant): kept for
/// this desktop to answer, nothing paired (`intro`).
async fn peer_intro(
    ctx: ClientContext,
    State(state): State<AppState>,
    Json(offer): Json<intro::Offer>,
) -> ApiResult<Json<Value>> {
    let machine = granted(&state, &ctx, Grant::View)?;
    Ok(Json(intro::receive(&state, &machine, offer)?))
}

#[derive(Deserialize)]
struct IntroId {
    id: String,
}

/// The introducer takes an introduction back (the other side was not
/// reached).
async fn peer_intro_withdraw(
    ctx: ClientContext,
    State(state): State<AppState>,
    Json(req): Json<IntroId>,
) -> ApiResult<Json<Value>> {
    let (machine, _) = paired(&ctx)?;
    Ok(Json(intro::withdraw(&state, &machine, &req.id)?))
}

/// The other introduced computer declined (its key, while the introduction
/// is open here: the two are not paired).
async fn peer_intro_decline(
    State(state): State<AppState>,
    caller: Option<Extension<PeerCaller>>,
    Json(req): Json<IntroId>,
) -> ApiResult<Json<Value>> {
    let caller = caller_of(caller)?;
    Ok(Json(intro::declined_by(&state, &caller.key, &req.id)?))
}

/// A paired computer asks this one to revoke a third (*Revoke
/// everywhere*): it only removes trust, so it is applied at once, audited
/// and shown here (`revocation`).
async fn peer_revoke(
    ctx: ClientContext,
    State(state): State<AppState>,
    Json(req): Json<revocation::RevokeRequest>,
) -> ApiResult<Json<Value>> {
    let (machine, _) = paired(&ctx)?;
    Ok(Json(revocation::receive(&state, &machine, req)?))
}

async fn peer_pair_join(
    State(state): State<AppState>,
    caller: Option<Extension<PeerCaller>>,
    path: Option<Extension<PathKind>>,
    Json(req): Json<pairing::JoinRequest>,
) -> ApiResult<Json<Value>> {
    let caller = caller_of(caller)?;
    let path = path.map(|Extension(p)| p).unwrap_or(PathKind::Relay);
    Ok(Json(pairing::host_join(&state, &caller.key, path, req)?))
}

async fn peer_pair_reveal(
    State(state): State<AppState>,
    caller: Option<Extension<PeerCaller>>,
    Json(req): Json<pairing::RevealRequest>,
) -> ApiResult<Json<Value>> {
    let caller = caller_of(caller)?;
    Ok(Json(pairing::host_reveal(&state, &caller.key, req)?))
}

async fn peer_pair_confirm(
    State(state): State<AppState>,
    caller: Option<Extension<PeerCaller>>,
    Json(req): Json<pairing::PeerConfirm>,
) -> ApiResult<Json<Value>> {
    let caller = caller_of(caller)?;
    Ok(Json(pairing::host_peer_confirm(&state, &caller.key, req)?))
}

async fn peer_pair_status(
    State(state): State<AppState>,
    caller: Option<Extension<PeerCaller>>,
) -> ApiResult<Json<Value>> {
    let caller = caller_of(caller)?;
    Ok(Json(pairing::host_status(&state, &caller.key)?))
}

async fn peer_pair_cancel(
    State(state): State<AppState>,
    caller: Option<Extension<PeerCaller>>,
) -> ApiResult<Json<Value>> {
    let caller = caller_of(caller)?;
    Ok(Json(pairing::host_peer_cancel(&state, &caller.key)?))
}

/// This machine's checks, for a paired machine's side-by-side view, with its
/// tools and what that machine asked of it.
async fn peer_setup(ctx: ClientContext, State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let machine = granted(&state, &ctx, Grant::View)?;
    Ok(Json(setup::peer_answer(&state, &machine).await))
}

/// Another machine asks this one to show a fix, or an asked action, to
/// whoever is at it: kept as an ask with a desktop notice — nothing runs
/// until someone confirms it here (`setup::asks`).
async fn peer_setup_ask(
    ctx: ClientContext,
    State(state): State<AppState>,
    Json(req): Json<setup::asks::AskRequest>,
) -> ApiResult<Json<Value>> {
    let machine = granted(&state, &ctx, Grant::View)?;
    let ask = setup::asks::receive(&state, &machine, req).await?;
    Ok(Json(json!({ "shown": true, "ask": ask })))
}

#[derive(Deserialize)]
struct PeerSettings {
    relay: RelaySetting,
    /// Its code options (absent from an older build).
    #[serde(default)]
    code: Option<super::store::CodeSettings>,
}

/// Another machine suggests the shared settings: kept as an offer until
/// this machine's desktop accepts or declines it.
async fn peer_settings(
    ctx: ClientContext,
    State(state): State<AppState>,
    Json(req): Json<PeerSettings>,
) -> ApiResult<Json<Value>> {
    let machine = granted(&state, &ctx, Grant::View)?;
    if req.relay.validate().is_err() {
        return Err(ApiError::unprocessable(
            "machines_bad_relay",
            "Not a valid relay setting.",
        ));
    }
    let mine = state.machines.settings();
    let code_differs = req.code.as_ref().is_some_and(|c| !c.same_as(&mine.code));
    if req.relay.same_as(&mine.relay) && !code_differs {
        state.machines.set_offer(None);
        return Ok(Json(json!({ "offered": false })));
    }
    let name = state
        .machines
        .store_read(|f| f.peer(&machine).map(|p| p.display_name().to_string()))
        .unwrap_or_default();
    state.machines.set_offer(Some(super::SettingsOffer {
        from: machine,
        from_name: name,
        relay: req.relay,
        code: req.code,
        at: super::now(),
    }));
    super::publish(&state, "settings");
    Ok(Json(json!({ "offered": true })))
}

/// Another machine reads this one's roots, tombstones included (`view`).
async fn peer_get_roots(
    ctx: ClientContext,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    granted(&state, &ctx, Grant::View)?;
    Ok(Json(roots::message(&state)))
}

#[derive(Deserialize)]
struct PeerRoots {
    #[serde(default)]
    roots: Value,
}

/// Another machine sends its roots: merged here only when this machine gave
/// it the `move` grant for this request's path — roots steer where a move
/// lands. The answer is this machine's roots, which the sender takes when
/// it gave this machine the same grant.
async fn peer_post_roots(
    ctx: ClientContext,
    State(state): State<AppState>,
    Json(req): Json<PeerRoots>,
) -> ApiResult<Json<Value>> {
    let machine = granted(&state, &ctx, Grant::Move)?;
    let changed = roots::receive(&state, &machine, &req.roots)
        .map_err(|e| ApiError::unprocessable("roots_refused", e))?;
    let mut answer = roots::message(&state);
    answer["changed"] = json!(changed);
    Ok(Json(answer))
}

#[derive(Deserialize)]
struct BulkQuery {
    #[serde(default)]
    mib: Option<u64>,
}

/// Zeros for the readiness test's transfer (at most 64 MiB).
async fn peer_bulk(
    ctx: ClientContext,
    State(state): State<AppState>,
    Query(q): Query<BulkQuery>,
) -> Response {
    if let Err(e) = granted(&state, &ctx, Grant::View) {
        return e.into_response();
    }
    let total = q.mib.unwrap_or(setup::TEST_MIB).min(64) << 20;
    let chunk = axum::body::Bytes::from(vec![0u8; 64 * 1024]);
    let n = (total / chunk.len() as u64) as usize;
    let stream =
        futures_util::stream::iter((0..n).map(move |_| Ok::<_, std::io::Error>(chunk.clone())));
    (
        [("content-length", total.to_string())],
        HttpBody::from_stream(stream),
    )
        .into_response()
}
