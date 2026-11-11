//! Machines' routes (docs/PLAN-MACHINES.md §7).
//!
//! * `/api/machines/*` — this machine's dashboard: reading is `Authed` (a
//!   phone sees Machines read-only); everything that changes anything is
//!   `DesktopOnly` (pairing, grants, revoking, settings, fixes, the test).
//! * `/peer/*` — the other machines, on the `Peer` ingress only
//!   (`peer_app`): pairing (`/peer/pair/*`, the one unpaired key pairing
//!   right now), then a paired machine's hello, setup checks, settings
//!   offer and the readiness test's transfer. Nothing else is mounted there.

use axum::body::Body as HttpBody;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::grants::{Grant, Grants};
use super::protocol::Hello;
use super::store::RelaySetting;
use super::transport::{self, PathKind};
use super::{pairing, setup};
use crate::server::auth::{Authed, DesktopOnly};
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
        .route("/machines/{id}", patch(update_peer).delete(revoke_peer))
}

/// `/peer/*`.
pub fn peer_router() -> Router<AppState> {
    Router::new()
        .route("/peer/hello", post(peer_hello))
        .route("/peer/pair/join", post(peer_pair_join))
        .route("/peer/pair/reveal", post(peer_pair_reveal))
        .route("/peer/pair/confirm", post(peer_pair_confirm))
        .route("/peer/pair/status", get(peer_pair_status))
        .route("/peer/pair/cancel", post(peer_pair_cancel))
        .route("/peer/setup", get(peer_setup))
        .route("/peer/setup/ask", post(peer_setup_ask))
        .route("/peer/settings", post(peer_settings))
        .route("/peer/bulk", get(peer_bulk))
        .fallback(|| async {
            ApiError::not_found("not_found", "No such route for another computer.")
        })
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
    check: String,
}

/// Asks another machine to show one of its fixes to whoever is at it.
async fn ask(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<AskBody>,
) -> ApiResult<Json<Value>> {
    if !super::ids::is_machine_id(&req.machine) || req.check.len() > 80 {
        return Err(ApiError::bad_request("Malformed request."));
    }
    let (status, body) = transport::request(
        &state,
        &req.machine,
        "POST",
        "/peer/setup/ask",
        Some(json!({ "check": req.check })),
        transport::REQUEST_TIMEOUT,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "machines_unreachable", e))?;
    if status >= 300 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "machines_refused",
            body.pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("The other computer refused."),
        ));
    }
    Ok(Json(json!({ "asked": true })))
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

async fn get_settings(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(super::settings_view(&state.machines.settings()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsBody {
    #[serde(default)]
    relay: Option<RelaySetting>,
    #[serde(default)]
    keep_awake: Option<bool>,
}

async fn put_settings(
    DesktopOnly(_): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<SettingsBody>,
) -> ApiResult<Json<Value>> {
    if let Some(r) = &req.relay {
        r.validate()
            .map_err(|m| ApiError::unprocessable("machines_bad_relay", m).with("field", "relay"))?;
    }
    let before = state.machines.settings();
    state.machines.store_write(|f| {
        if let Some(r) = req.relay.clone() {
            f.settings.relay = r;
        }
        if let Some(k) = req.keep_awake {
            f.settings.keep_awake = k;
        }
    });
    let after = state.machines.settings();
    super::awake::update(&state);
    if !after.relay.same_as(&before.relay) {
        // A new relay means a new endpoint; then the others are offered it.
        let was_running = transport::current(&state).await.is_some();
        transport::stop(&state).await;
        if was_running {
            if let Err(e) = transport::ensure_started(&state).await {
                tracing::warn!(error = %e, "Machines transport did not restart");
            }
        }
        offer_settings(&state).await;
    }
    super::publish(&state, "settings");
    Ok(Json(super::settings_view(&after)))
}

/// Offers this machine's shared settings to every online paired machine.
pub async fn offer_settings(state: &AppState) {
    let relay = state.machines.settings().relay;
    let peers: Vec<String> = state
        .machines
        .store_read(|f| f.peers.iter().map(|p| p.id.clone()).collect());
    let body = json!({ "relay": relay });
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
    if req.accept && offer.relay.validate().is_ok() {
        state
            .machines
            .store_write(|f| f.settings.relay = offer.relay.clone());
        let was_running = transport::current(&state).await.is_some();
        transport::stop(&state).await;
        if was_running {
            let _ = transport::ensure_started(&state).await;
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
    state.machines.store_write(|f| {
        f.enabled = req.enabled;
        if req.enabled {
            f.setup_step = None;
        }
    });
    super::publish(&state, "enabled");
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

// ─── the other machines ──────────────────────────────────────────────────────────

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
        Principal::Peer { machine, lan } => Ok((machine.clone(), *lan)),
        _ => Err(ApiError::forbidden(
            "peer_only",
            "Only a paired computer can call this.",
        )),
    }
}

/// A paired machine holding `grant` for this request's path.
fn granted(state: &AppState, ctx: &ClientContext, grant: Grant) -> ApiResult<String> {
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
    let (machine, _) = paired(&ctx)?;
    let caller = caller_of(caller)?;
    if hello.machine.id != machine || hello.machine.key != caller.key {
        return Err(ApiError::forbidden(
            "peer_mismatch",
            "That is not the computer this key was paired as.",
        ));
    }
    let conn = state.machines.live_connection(&caller.key);
    super::learn(&state, &hello, conn.as_ref());
    let peer = state
        .machines
        .store_read(|f| f.peer(&machine).cloned())
        .ok_or_else(|| ApiError::forbidden("peer_only", "Not paired."))?;
    let reply = Hello {
        machine: super::machine_info(&state),
        relay: state.machines.settings().relay,
        grants_you_have: peer.grants,
        capabilities: super::protocol::CAPABILITIES
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };
    Ok(Json(serde_json::to_value(reply).unwrap_or(Value::Null)))
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

/// This machine's checks, for a paired machine's side-by-side view.
async fn peer_setup(ctx: ClientContext, State(state): State<AppState>) -> ApiResult<Json<Value>> {
    granted(&state, &ctx, Grant::View)?;
    let v = setup::view(&state, false).await;
    Ok(Json(json!({ "checks": v["self"]["checks"] })))
}

#[derive(Deserialize)]
struct PeerAsk {
    check: String,
}

/// Another machine asks this one to show a fix to whoever is at it: a
/// desktop notice — nothing runs until someone confirms it here.
async fn peer_setup_ask(
    ctx: ClientContext,
    State(state): State<AppState>,
    Json(req): Json<PeerAsk>,
) -> ApiResult<Json<Value>> {
    let machine = granted(&state, &ctx, Grant::View)?;
    let name = state
        .machines
        .store_read(|f| f.peer(&machine).map(|p| p.display_name().to_string()))
        .unwrap_or_default();
    let check = setup::view(&state, false).await["self"]["checks"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["id"] == req.check.as_str()).cloned());
    let Some(check) = check else {
        return Err(ApiError::not_found(
            "machines_unknown_check",
            "No such check here.",
        ));
    };
    let title = check["title"].as_str().unwrap_or("A check");
    let fix = check["fix"]["label"]
        .as_str()
        .unwrap_or("Open Machines › Set up");
    crate::server::ws::publish_desktop_notice(
        &state,
        crate::claude::model::NoticeLevel::Warning,
        format!("{name} asks you to fix \"{title}\" on this computer: {fix} (Machines › Set up)."),
    );
    super::audit(&state, "machine.ask", &machine, &name);
    Ok(Json(json!({ "shown": true })))
}

#[derive(Deserialize)]
struct PeerSettings {
    relay: RelaySetting,
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
    if req.relay.same_as(&state.machines.settings().relay) {
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
        at: super::now(),
    }));
    super::publish(&state, "settings");
    Ok(Json(json!({ "offered": true })))
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
