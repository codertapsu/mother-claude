//! Driving sessions (design §4.5 "Control", §5.3).
//!
//! Owner: **B3 (owned control)**. The foundation wrote the dispatchers: every
//! control route resolves the session's channel ([`route_for`]) and calls the
//! owning subsystem's facade:
//!
//! | channel | callee (owner) |
//! |---|---|
//! | HTTP bridge conversation | `crate::bridge` |
//! | owned (live control handle) | `crate::claude::owned` (B3) |
//! | tmux | `crate::claude::tmux` (B4) |
//! | hooks active | `crate::server::hooks` (B4) |
//! | experimental PTY (background) | `super::foreign::pty_inject_text` (B4) |
//! | none | `409 no_channel` with the session's capability notes |
//!
//! The channel for foreign sessions comes from `SessionSummary.control.channel`
//! (computed by `claude::caps`), so B4 changes routing by changing the caps.
//! B3 fills in the owned facade and may extend these handlers; keep the
//! dispatcher order (bridge → owned → caps channel).
//!
//! **No unapproved execution for callers without dangerous rights:** a
//! message, `/continue` or spawn into a session whose effective permission
//! mode is bypassPermissions needs L (`may_dangerous`), whatever the channel.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::foreign;
use super::Body;
use crate::claude::model::{
    ControlChannel, Delivery, ImageInput, MessageReceipt, NoticeLevel, PendingItem, PendingKind,
    PendingSource, Question, QuestionOption, QueueStatus, SessionSummary,
};
use crate::claude::owned::{self, Caller, ContinueRequest, SpawnRequest};
use crate::claude::tmux;
use crate::pending::{self, Decision, RespondBody};
use crate::server::auth::{require_dangerous, Authed, DangerousOk, SidecarOnly};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::server::{hooks, ws};
use crate::state::AppState;

/// How a session can be reached right now.
#[derive(Debug, Clone)]
pub enum Route {
    /// A conversation of the HTTP bridge's Agent SDK host.
    Bridge,
    /// A child Mother Claude holds a live control handle for.
    Owned,
    /// A foreign session in a tmux pane.
    Tmux(SessionSummary),
    /// A foreign session with the hooks plugin active.
    Hooks(SessionSummary),
    /// A background session driven through experimental PTY attach.
    Pty(SessionSummary),
    /// Nothing reaches it.
    None(SessionSummary),
}

/// Resolve the channel for session `id` (404 when unknown).
pub async fn route_for(state: &AppState, id: &str) -> ApiResult<Route> {
    if state.bridge.thread(id).await.is_some() {
        return Ok(Route::Bridge);
    }
    if owned::has_handle(state, id) {
        return Ok(Route::Owned);
    }
    let s = state
        .find_session(id)
        .await
        .ok_or_else(|| ApiError::no_such_session(id))?;
    Ok(match s.control.channel {
        ControlChannel::Tmux => Route::Tmux(s),
        ControlChannel::Hooks => Route::Hooks(s),
        ControlChannel::Pty => Route::Pty(s),
        // `Owned` in the caps without a live handle = the child is gone.
        ControlChannel::Owned | ControlChannel::None => Route::None(s),
    })
}

/// `409 no_channel`, carrying the capability notes for the UI.
pub fn no_channel(s: &SessionSummary) -> ApiError {
    ApiError::conflict(
        "no_channel",
        "Mother Claude cannot reach this session. Install foreign-session control on the \
         desktop, or continue it here as an owned session.",
    )
    .with("notes", json!(s.control.notes))
    .with("limits", json!(s.control.limits))
}

fn unsupported(what: &str, route: &Route) -> ApiError {
    match route {
        Route::None(s) => no_channel(s),
        _ => ApiError::conflict(
            "not_supported",
            format!("{what} is not supported for this session's control channel."),
        ),
    }
}

fn ok() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// How long a dashboard → bridge-host call may take.
const BRIDGE_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// 403 unless the caller may cause unapproved execution: the target runs in
/// bypassPermissions, so anything it is told runs with nobody approving it.
async fn gate_unapproved(state: &AppState, ctx: &ClientContext, id: &str) -> ApiResult<()> {
    if owned::runs_unapproved(state, id).await {
        require_dangerous(
            ctx,
            "This session runs without permission prompts (bypassPermissions); driving it",
        )?;
    }
    Ok(())
}

/// `POST /api/sessions` → `201 {id}` (bypass-class modes = L).
pub async fn spawn(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Body(req): Body<SpawnRequest>,
) -> ApiResult<Response> {
    if req.cwd.trim().is_empty() {
        return Err(ApiError::unprocessable("invalid_cwd", "`cwd` is required."));
    }
    if owned::is_dangerous_mode(req.permission_mode.as_deref()) {
        require_dangerous(&ctx, "Starting a session without permission prompts")?;
    }
    let id = owned::spawn_as(&state, Caller::of(&ctx), req).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))).into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageBody {
    pub text: String,
    pub images: Vec<ImageInput>,
}

/// `POST /api/sessions/{id}/message` → `{delivery, messageId}`.
pub async fn message(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<MessageBody>,
) -> ApiResult<Json<MessageReceipt>> {
    if body.text.trim().is_empty() && body.images.is_empty() {
        return Err(ApiError::unprocessable("empty_message", "Send some text."));
    }
    let route = route_for(&state, &id).await?;
    if !body.images.is_empty() && !matches!(route, Route::Owned) {
        return Err(ApiError::unprocessable(
            "images_not_supported",
            "Images can only be sent to sessions Mother Claude owns.",
        ));
    }
    gate_unapproved(&state, &ctx, &id).await?;
    let message_id = uuid::Uuid::new_v4().to_string();
    let delivery = match route {
        Route::Bridge => {
            crate::bridge::send_message(&state, &id, &body.text).await?;
            Delivery::Sent
        }
        Route::Owned => {
            return Ok(Json(
                owned::send_as(&state, ctx.may_dangerous(), &id, &body.text, &body.images).await?,
            ))
        }
        Route::Tmux(s) => match tmux::send_text_as(&state, &ctx, &s, &body.text).await {
            Ok(()) => Delivery::Relayed,
            // The pane cannot be reached from here (custom socket, tmux gone)
            // but the hooks can: relay through them (the caps follow).
            Err(e) if tmux::is_unreachable_error(&e) && hooks::is_active(&state, &id) => {
                crate::claude::caps::refresh(&state, &id).await;
                return relay(&state, &ctx, &id, &body.text).await;
            }
            Err(e) => return Err(e),
        },
        Route::Hooks(_) => return relay(&state, &ctx, &id, &body.text).await,
        Route::Pty(s) => {
            foreign::pty_inject_text(&state, &ctx, &s, &body.text).await?;
            Delivery::Relayed
        }
        Route::None(s) => return Err(no_channel(&s)),
    };
    Ok(Json(MessageReceipt {
        delivery,
        message_id,
    }))
}

/// Queue `text` for a hooked foreign session.
async fn relay(
    state: &AppState,
    ctx: &ClientContext,
    id: &str,
    text: &str,
) -> ApiResult<Json<MessageReceipt>> {
    let queued = hooks::enqueue_relay_as(state, ctx, id, text).await?;
    Ok(Json(MessageReceipt {
        delivery: if queued.status == QueueStatus::Delivered {
            Delivery::Relayed
        } else {
            Delivery::Queued
        },
        message_id: queued.id,
    }))
}

/// `DELETE /api/sessions/{id}/queue/{messageId}`.
pub async fn cancel_queued(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path((id, message_id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    match route_for(&state, &id).await? {
        Route::Owned => {
            owned::cancel_queued(&state, &id, &message_id).await?;
            Ok(ok())
        }
        _ => {
            hooks::cancel_queued(&state, &id, &message_id).await?;
            Ok(ok())
        }
    }
}

/// `POST /api/sessions/{id}/interrupt`.
pub async fn interrupt(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    match route_for(&state, &id).await? {
        Route::Owned => owned::interrupt(&state, &id).await?,
        Route::Tmux(s) => match tmux::send_key(&state, &s, "Escape").await {
            Err(e) if tmux::is_unreachable_error(&e) && hooks::is_active(&state, &id) => {
                crate::claude::caps::refresh(&state, &id).await;
                hooks::request_interrupt(&state, &id).await?
            }
            other => other?,
        },
        Route::Hooks(_) => hooks::request_interrupt(&state, &id).await?,
        Route::Bridge => {
            // Interrupt the conversation's running turn, if any (naming the
            // turn lets the host refuse if a different one is live).
            if let Some(turn) = state.bridge.thread(&id).await.and_then(|t| t.active_turn) {
                state
                    .bridge
                    .host
                    .call(
                        &state,
                        "session.interrupt",
                        json!({ "sessionId": id, "turnId": turn }),
                        BRIDGE_CALL_TIMEOUT,
                    )
                    .await?;
                state.bridge.set_active_turn(&id, None).await;
            }
        }
        other => return Err(unsupported("Interrupting", &other)),
    }
    Ok(ok())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ModelBody {
    pub model: String,
}

/// `POST /api/sessions/{id}/model {model}`.
pub async fn set_model(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<ModelBody>,
) -> ApiResult<Json<Value>> {
    let model = owned::validate_model(&body.model)?;
    let model = model.as_str();
    match route_for(&state, &id).await? {
        Route::Owned => owned::set_model(&state, &id, model).await?,
        Route::Bridge => {
            state
                .bridge
                .host
                .call(
                    &state,
                    "session.setModel",
                    json!({ "sessionId": id, "model": model }),
                    BRIDGE_CALL_TIMEOUT,
                )
                .await?;
            let m = model.to_string();
            state
                .bridge
                .update_thread(&id, move |r| r.model = Some(m))
                .await;
        }
        Route::Tmux(s) => tmux::send_text_as(&state, &ctx, &s, &format!("/model {model}")).await?,
        other => return Err(unsupported("Changing the model", &other)),
    }
    Ok(ok())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ModeBody {
    pub mode: String,
}

/// `POST /api/sessions/{id}/mode {mode}` (bypass-class = L).
pub async fn set_mode(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<ModeBody>,
) -> ApiResult<Json<Value>> {
    owned::validate_launch(None, None, Some(&body.mode))?;
    if owned::is_dangerous_mode(Some(&body.mode)) {
        require_dangerous(&ctx, "Switching to a mode without permission prompts")?;
    }
    match route_for(&state, &id).await? {
        Route::Owned => owned::set_mode(&state, &id, &body.mode).await?,
        Route::Bridge => {
            state
                .bridge
                .host
                .call(
                    &state,
                    "session.setPermissionMode",
                    json!({ "sessionId": id, "mode": body.mode }),
                    BRIDGE_CALL_TIMEOUT,
                )
                .await?;
            let mode = body.mode.clone();
            state
                .bridge
                .update_thread(&id, move |r| r.permission_mode = mode)
                .await;
        }
        other => return Err(unsupported("Changing the permission mode", &other)),
    }
    Ok(ok())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct EffortBody {
    pub effort: String,
}

/// `POST /api/sessions/{id}/effort {effort}`.
pub async fn set_effort(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<EffortBody>,
) -> ApiResult<Json<Value>> {
    owned::validate_launch(Some(&body.effort), None, None)?;
    match route_for(&state, &id).await? {
        Route::Owned => owned::set_effort(&state, &id, &body.effort).await?,
        Route::Tmux(s) => {
            tmux::send_text_as(&state, &ctx, &s, &format!("/effort {}", body.effort)).await?
        }
        other => return Err(unsupported("Changing the effort", &other)),
    }
    Ok(ok())
}

#[derive(Debug, Deserialize)]
pub struct SwitchBody {
    pub enabled: bool,
}

/// `409 not_supported` for a session-setting switch outside an owned session.
fn owned_only(what: &str) -> ApiError {
    ApiError::conflict(
        "not_supported",
        format!(
            "{what} can only be switched in a session Mother Claude owns: continue this \
             session here first."
        ),
    )
}

/// `POST /api/sessions/{id}/ultracode {enabled}` → 204 (owned Path B
/// sessions; session-scoped, never persisted).
pub async fn set_ultracode(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<SwitchBody>,
) -> ApiResult<StatusCode> {
    match route_for(&state, &id).await? {
        Route::Owned => owned::set_ultracode(&state, &id, body.enabled).await?,
        _ => return Err(owned_only("Ultracode")),
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/sessions/{id}/thinking {enabled}` → 204 (owned Path B
/// sessions).
pub async fn set_thinking(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<SwitchBody>,
) -> ApiResult<StatusCode> {
    match route_for(&state, &id).await? {
        Route::Owned => owned::set_thinking(&state, &id, body.enabled).await?,
        _ => return Err(owned_only("Thinking")),
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Characters a name typed into a terminal (`/rename <name>`) may not carry.
const SHELL_METACHARACTERS: [char; 8] = [';', '|', '&', '$', '`', '<', '>', '\\'];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RenameBody {
    pub name: String,
}

/// `POST /api/sessions/{id}/rename {name}`.
pub async fn rename(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<RenameBody>,
) -> ApiResult<Json<Value>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::unprocessable(
            "invalid_name",
            "Name must not be empty.",
        ));
    }
    match route_for(&state, &id).await? {
        Route::Owned => owned::rename(&state, &id, name).await?,
        Route::Tmux(s) => {
            let one_line: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
            // Defence in depth: typed into a terminal, a name must never be
            // able to act as a shell command line.
            if one_line.contains(SHELL_METACHARACTERS) {
                return Err(ApiError::unprocessable(
                    "invalid_name",
                    "A session name typed into its terminal may not contain ; | & $ ` < > \\.",
                ));
            }
            tmux::send_text_as(&state, &ctx, &s, &format!("/rename {one_line}")).await?
        }
        other => return Err(unsupported("Renaming", &other)),
    }
    Ok(ok())
}

/// `POST /api/sessions/{id}/tasks/{taskId}/stop`.
pub async fn stop_task(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path((id, task_id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    match route_for(&state, &id).await? {
        Route::Owned => owned::stop_task(&state, &id, &task_id).await?,
        other => return Err(unsupported("Stopping a background task", &other)),
    }
    Ok(ok())
}

/// `POST /api/sessions/{id}/respond` (design §4.2).
pub async fn respond(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<RespondBody>,
) -> ApiResult<Json<Value>> {
    let RespondBody {
        request_id,
        decision,
    } = body;
    match pending::respond(&state, &id, &request_id, decision, &ctx).await {
        Ok(_) => {
            let resolution = state.pending.resolution(&request_id);
            Ok(Json(
                json!({ "ok": true, "requestId": request_id, "resolution": resolution }),
            ))
        }
        Err(e) => Err(already_decided_with_resolution(&state, e, &request_id)),
    }
}

/// A refused answer; a `409 already_decided` carries how it was resolved
/// ("Answered on Mac", "Answered elsewhere (…)") instead of a bare error.
fn already_decided_with_resolution(
    state: &AppState,
    e: pending::RespondError,
    request_id: &str,
) -> ApiError {
    let decided = e == pending::RespondError::AlreadyDecided;
    let mut err = e.into_api(request_id);
    if decided {
        if let Some(r) = state.pending.resolution(request_id) {
            if let Some(text) = &r.text {
                err.message = format!("{text}.");
            }
            err = err.with("resolution", json!(r));
        }
    }
    err
}

/// `POST /api/sessions/{id}/continue` → `201 {id}`.
pub async fn continue_session(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(req): Body<ContinueRequest>,
) -> ApiResult<Response> {
    let s = state
        .find_session(&id)
        .await
        .ok_or_else(|| ApiError::no_such_session(&id))?;
    if owned::is_dangerous_mode(req.permission_mode.as_deref()) {
        require_dangerous(&ctx, "Continuing without permission prompts")?;
    }
    gate_unapproved(&state, &ctx, &id).await?;
    let new_id = owned::continue_as(&state, Caller::of(&ctx), &id, &s.cwd, req).await?;
    let forked = new_id != id;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": new_id, "forked": forked })),
    )
        .into_response())
}

/// `claude stop|respawn|rm <short>` for a background session (422
/// `not_a_background_session` when it has no short job id).
async fn run_lifecycle(state: &AppState, action: &str, id: &str) -> ApiResult<Json<Value>> {
    let output = owned::lifecycle(state, action, id).await?;
    ws::publish_notice(
        state,
        NoticeLevel::Info,
        format!("{action} {id}: ok"),
        Some(id),
    );
    Ok(Json(json!({ "ok": true, "output": output })))
}

/// `POST /api/sessions/{id}/stop`.
pub async fn stop(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    // Bridge conversations have no ControlRegistry handle; close them through
    // the bridge so "stopped" is true.
    if crate::bridge::close_thread(&state, &id).await {
        pending::withdraw_session(&state, &id, "session stopped").await;
        ws::publish_notice(
            &state,
            NoticeLevel::Info,
            format!("stopped {id}"),
            Some(&id),
        );
        return Ok(ok());
    }
    if owned::has_handle(&state, &id) {
        owned::kill(&state, &id).await?;
        pending::withdraw_session(&state, &id, "session stopped").await;
        ws::publish_notice(
            &state,
            NoticeLevel::Info,
            format!("stopped {id}"),
            Some(&id),
        );
        return Ok(ok());
    }
    run_lifecycle(&state, "stop", &id).await
}

/// `POST /api/sessions/{id}/respawn`.
pub async fn respawn(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if owned::is_owned(&state, &id) {
        return Err(ApiError::unprocessable(
            "not_a_background_session",
            "Respawn isn't supported for app-owned sessions — stop it and start a new one.",
        ));
    }
    run_lifecycle(&state, "respawn", &id).await
}

/// `POST /api/sessions/{id}/rm` (L).
pub async fn rm(
    DangerousOk(_ctx): DangerousOk,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let owned_here = if crate::bridge::close_thread(&state, &id).await {
        true
    } else if owned::has_handle(&state, &id) || state.is_owned(&id).await {
        let _ = owned::kill(&state, &id).await;
        state.unmark_owned(&id).await;
        true
    } else {
        false
    };
    if owned_here {
        pending::withdraw_session(&state, &id, "session removed").await;
        ws::publish_notice(
            &state,
            NoticeLevel::Info,
            format!("removed {id}"),
            Some(&id),
        );
        return Ok(ok());
    }
    let out = run_lifecycle(&state, "rm", &id).await?;
    pending::withdraw_session(&state, &id, "session removed").await;
    Ok(out)
}

/// How long a sidecar prompt waits before defaulting to deny.
const SIDECAR_TIMEOUT: std::time::Duration = pending::DEFAULT_TIMEOUT;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PermissionRequestBody {
    /// Ignored: ids are always server-generated (a caller must never be able
    /// to replace another request by reusing its id).
    #[serde(alias = "requestId")]
    pub request_id: Option<String>,
    /// "permission" or "question".
    pub kind: String,
    pub tool: Option<String>,
    pub prompt: Option<String>,
    pub header: Option<String>,
    /// Strings or `{label, description}` objects.
    pub options: Vec<Value>,
    #[serde(alias = "multiSelect")]
    pub multi_select: bool,
    pub detail: Option<String>,
    pub input: Option<Value>,
    pub suggestions: Option<Value>,
    pub dangerous: bool,
}

fn sidecar_item(session_id: &str, body: PermissionRequestBody) -> PendingItem {
    let is_question = body.kind == "question";
    let tool = body.tool.clone();
    let title = body.prompt.clone().unwrap_or_else(|| match &tool {
        Some(t) => format!("Claude wants to use {t}."),
        None => "Claude is waiting for you.".into(),
    });
    let questions = is_question.then(|| {
        vec![Question {
            question: body.prompt.clone().unwrap_or_default(),
            header: body.header.clone(),
            options: body
                .options
                .iter()
                .filter_map(|o| match o {
                    Value::String(s) => Some(QuestionOption {
                        label: s.clone(),
                        description: None,
                    }),
                    _ => Some(QuestionOption {
                        label: o.get("label")?.as_str()?.to_string(),
                        description: o
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    }),
                })
                .collect(),
            multi_select: body.multi_select,
        }]
    });
    PendingItem {
        session_id: session_id.to_string(),
        kind: if is_question {
            PendingKind::Question
        } else {
            PendingKind::Permission
        },
        source: PendingSource::Sidecar,
        tool_name: tool,
        title,
        detail: body.detail,
        input: body.input,
        questions,
        suggestions: pending::suggestions_from_raw(body.suggestions.as_ref()),
        dangerous: body.dangerous,
        ..Default::default()
    }
}

/// `POST /api/sessions/{id}/permission-request` (Sidecar principal only):
/// raise a prompt and **block** until it is answered (or times out → deny).
/// Response (the Path A sidecar's contract):
/// `{behavior:"allow"|"deny"|"answer", answer?}`.
pub async fn permission_request(
    SidecarOnly(_ctx): SidecarOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<PermissionRequestBody>,
) -> Json<Value> {
    let item = sidecar_item(&id, body);
    let is_question = item.kind == PendingKind::Question;
    let (guard, rx) = pending::raise(&state, item).await;
    let end = pending::wait_end(rx, SIDECAR_TIMEOUT).await;
    if matches!(end, pending::WaitEnd::TimedOut) {
        guard.expire();
    } else {
        drop(guard);
    }
    let decision = match end {
        pending::WaitEnd::Decided(d) => Some(d),
        pending::WaitEnd::Closed | pending::WaitEnd::TimedOut => None,
    };
    Json(match decision {
        Some(d) if d.is_allow() && is_question => {
            json!({ "behavior": "answer", "answer": d.answer_text().unwrap_or_default() })
        }
        Some(d) if d.is_allow() => json!({ "behavior": "allow" }),
        Some(d) => json!({ "behavior": "deny", "message": d.message }),
        None => json!({ "behavior": "deny" }),
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LegacyPermissionBody {
    /// "allow" or "deny".
    pub decision: String,
    #[serde(alias = "requestId")]
    pub request_id: Option<String>,
}

fn pick_item(state: &AppState, session: &str, request_id: Option<&str>) -> Option<PendingItem> {
    match request_id {
        Some(r) => state.pending.get(r),
        None => state.pending.oldest_for(session),
    }
}

/// Legacy alias of `respond`: `POST /api/sessions/{id}/permission
/// {decision, requestId?}` → 204. Kept for one release.
pub async fn legacy_permission(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<LegacyPermissionBody>,
) -> ApiResult<Response> {
    let allow = body.decision == "allow";
    let Some(item) = pick_item(&state, &id, body.request_id.as_deref()) else {
        // Foreign background session driven by PTY: select the TUI option.
        let s = state
            .find_session(&id)
            .await
            .ok_or_else(|| ApiError::no_such_session(&id))?;
        if s.control.channel == ControlChannel::Pty {
            let is_plan = s.pending.iter().any(|p| p.kind == PendingKind::Plan);
            let key = match (is_plan, allow) {
                (true, true) => "2",
                (true, false) => "3",
                (false, true) => "1",
                (false, false) => "2",
            };
            foreign::pty_inject_text(&state, &ctx, &s, key).await?;
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
        return Err(ApiError::not_found(
            "request_not_found",
            "No pending request to resolve.",
        ));
    };
    let decision = if allow {
        Decision::allow()
    } else {
        Decision::deny(None)
    };
    pending::respond(&state, &id, &item.id, decision, &ctx)
        .await
        .map_err(|e| already_decided_with_resolution(&state, e, &item.id))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LegacyAnswerBody {
    pub answer: String,
    #[serde(alias = "requestId")]
    pub request_id: Option<String>,
}

/// Legacy alias of `respond`: `POST /api/sessions/{id}/answer
/// {answer, requestId?}` → 204. A question gets the answer; a permission is
/// denied with the text as the reason.
pub async fn legacy_answer(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<LegacyAnswerBody>,
) -> ApiResult<Response> {
    let Some(item) = pick_item(&state, &id, body.request_id.as_deref()) else {
        let s = state
            .find_session(&id)
            .await
            .ok_or_else(|| ApiError::no_such_session(&id))?;
        if s.control.channel == ControlChannel::Pty {
            foreign::pty_inject_text(&state, &ctx, &s, &body.answer).await?;
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
        return Err(ApiError::not_found(
            "request_not_found",
            "No pending question to answer.",
        ));
    };
    let decision = if item.kind == PendingKind::Question {
        let mut d = Decision::allow();
        let key = item
            .questions
            .as_ref()
            .and_then(|q| q.first())
            .map(|q| q.question.clone())
            .unwrap_or_default();
        d.answers = Some([(key, body.answer.clone())].into_iter().collect());
        d
    } else {
        Decision::deny(Some(body.answer.clone()))
    };
    pending::respond(&state, &id, &item.id, decision, &ctx)
        .await
        .map_err(|e| already_decided_with_resolution(&state, e, &item.id))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /api/sessions/{id}/context` → the CLI's context-usage breakdown
/// (owned sessions only).
pub async fn context_usage(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    match route_for(&state, &id).await? {
        Route::Owned => Ok(Json(owned::context_usage(&state, &id).await?)),
        other => Err(unsupported("Live context usage", &other)),
    }
}

/// `GET /api/sessions/{id}/mcp` → `{servers:[{name,status,scope}]}` (owned
/// sessions only; server config is never echoed).
pub async fn mcp_status(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    match route_for(&state, &id).await? {
        Route::Owned => Ok(Json(owned::mcp_status(&state, &id).await?)),
        other => Err(unsupported("MCP status", &other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_items_are_server_identified_and_typed() {
        let q = sidecar_item(
            "s1",
            serde_json::from_value(json!({
                "requestId": "attacker-chosen",
                "kind": "question",
                "prompt": "Which DB?",
                "header": "Storage",
                "options": ["pg", { "label": "sqlite", "description": "file" }],
                "multiSelect": true,
            }))
            .unwrap(),
        );
        assert!(q.id.is_empty(), "the store assigns ids");
        assert_eq!(q.kind, PendingKind::Question);
        assert_eq!(q.source, PendingSource::Sidecar);
        let qs = q.questions.unwrap();
        assert_eq!(qs[0].options.len(), 2);
        assert!(qs[0].multi_select);

        let p = sidecar_item(
            "s1",
            serde_json::from_value(
                json!({ "kind": "permission", "tool": "Bash", "dangerous": true }),
            )
            .unwrap(),
        );
        assert_eq!(p.kind, PendingKind::Permission);
        assert_eq!(p.title, "Claude wants to use Bash.");
        assert!(p.dangerous);
    }
}
