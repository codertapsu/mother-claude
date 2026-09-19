//! Turns and their operations: start, run, steer, interrupt, poll, stream.
//!
//! Starting a turn is asynchronous by design — `POST /threads/{id}/turns`
//! returns `202` with an `operation_id` and the turn runs on. `POST /run` is
//! the same machinery with the wait built in, for callers that would rather
//! block than stream.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::threads::CreateThread;
use super::{accepted, ok, sse, validate_id, Body};
use crate::bridge::error::{BridgeError, BridgeResult};
use crate::bridge::host::RPC_TIMEOUT;
use crate::bridge::inputs;
use crate::bridge::ops::Operation;
use crate::bridge::RUN_TIMEOUT_SECS;
use crate::state::AppState;

/// One user turn.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TurnBody {
    /// A string, one typed item, or an array of them.
    pub input: Value,
    /// Steering priority: `now` | `next` | `later`.
    pub priority: Option<String>,
}

fn validate_priority(priority: &Option<String>) -> BridgeResult<()> {
    if let Some(p) = priority {
        if !["now", "next", "later"].contains(&p.as_str()) {
            return Err(BridgeError::invalid_field(
                "priority",
                format!("Unknown priority {p:?}; expected now, next or later."),
            ));
        }
    }
    Ok(())
}

/// Start a turn, returning its operation. Shared by `/turns`, `/run` and
/// `/chat` so all three behave identically.
pub(crate) async fn begin_turn(
    state: &AppState,
    thread_id: &str,
    body: &TurnBody,
) -> BridgeResult<std::sync::Arc<Operation>> {
    validate_id("thread_id", thread_id)?;
    validate_priority(&body.priority)?;
    let normalized = inputs::normalize(&body.input)?;

    let turn_id = Uuid::new_v4().to_string();
    let op = state
        .bridge
        .ops
        .create(Some(thread_id.to_string()), Some(turn_id.clone()))?;

    // Announce the turn on its own log before any model output, so a client
    // that streams from event 1 sees the turn open.
    op.emit(
        &state.bridge.ops.global,
        &json!({
            "method": "turn/started",
            "params": {
                "thread_id": thread_id,
                "turn_id": turn_id,
                "operation_id": op.id,
                "text": normalized.text,
                "image_count": normalized.image_count,
                "estimated_image_tokens": normalized.estimated_image_tokens,
            }
        }),
    );

    let mut params = json!({
        "sessionId": thread_id,
        "turnId": turn_id,
        "content": normalized.content(),
    });
    if let Some(priority) = &body.priority {
        params["priority"] = json!(priority);
    }

    let result = match state
        .bridge
        .host
        .call(state, "session.input", params, RPC_TIMEOUT)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            // A turn that never started must not hold an active slot.
            state.bridge.ops.discard(&op.id);
            return Err(e);
        }
    };

    // The host reports which turn actually accepted the input. A mismatch means
    // a turn was already running, and this input would have silently joined it
    // — steer explicitly instead of quietly merging two callers' work.
    let accepted_turn = result.get("turnId").and_then(Value::as_str);
    if accepted_turn != Some(turn_id.as_str()) {
        state.bridge.ops.discard(&op.id);
        return Err(BridgeError::busy(
            "That conversation already has a turn running; steer it or wait for it to finish.",
        )
        .with("thread_id", thread_id)
        .with("turn_id", accepted_turn.unwrap_or_default()));
    }

    state
        .bridge
        .set_active_turn(thread_id, Some(turn_id.clone()))
        .await;
    Ok(op)
}

/// `POST /threads/{id}/turns` — start a turn, return `202` immediately.
pub async fn start(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
    Body(body): Body<TurnBody>,
) -> BridgeResult<Response> {
    let op = begin_turn(&state, &thread_id, &body).await?;
    Ok(accepted(op.snapshot(), &op.id))
}

/// `POST /threads/{id}/run` — start a turn and wait for it.
pub async fn run(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
    Body(body): Body<TurnBody>,
) -> BridgeResult<Response> {
    let op = begin_turn(&state, &thread_id, &body).await?;
    Ok(await_operation(&state, op).await)
}

/// Wait for an operation and render its terminal state, mirroring the Codex
/// bridge: `200` with the result, `502` with the error envelope, or `504` with
/// the operation id so the caller can fall back to polling or replay.
async fn await_operation(state: &AppState, op: std::sync::Arc<Operation>) -> Response {
    let finished = op.wait_done(Duration::from_secs(RUN_TIMEOUT_SECS)).await;
    let snapshot = op.snapshot();
    let headers = [("x-operation-id", op.id.clone())];

    if let Some(thread_id) = &op.thread_id {
        state.bridge.set_active_turn(thread_id, None).await;
    }

    if !finished {
        return (
            StatusCode::GATEWAY_TIMEOUT,
            headers,
            axum::Json(
                BridgeError::timeout(format!(
                    "The turn did not finish within {RUN_TIMEOUT_SECS}s. It is still running; \
                     poll /operations/{} or stream its events.",
                    op.id
                ))
                .with("operation_id", op.id.clone())
                .with("thread_id", op.thread_id.clone())
                .body(),
            ),
        )
            .into_response();
    }

    match snapshot.get("error") {
        Some(error) if !error.is_null() => (
            status_for_turn_error(error),
            headers,
            axum::Json(json!({ "error": error })),
        )
            .into_response(),
        _ => (
            StatusCode::OK,
            headers,
            axum::Json(json!({
                "operation_id": op.id,
                "thread_id": op.thread_id,
                "turn_id": op.turn_id,
                "status": snapshot["status"],
                "result": snapshot["result"],
            })),
        )
            .into_response(),
    }
}

/// A failed turn is a `502` by default, but some causes deserve better: a
/// signed-out Claude is a `401` the caller can act on, and a runtime that went
/// away is a `503`. Answering 502 for all three sends people to debug the wrong
/// thing — the model, rather than their own sign-in.
fn status_for_turn_error(error: &Value) -> StatusCode {
    match error.get("code").and_then(Value::as_str) {
        Some("claude_login_required") => StatusCode::UNAUTHORIZED,
        Some("runtime_unavailable") => StatusCode::SERVICE_UNAVAILABLE,
        Some("timeout") => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::BAD_GATEWAY,
    }
}

/// `POST /threads/{id}/turns/{turn}/steer` — add input to a running turn.
pub async fn steer(
    State(state): State<AppState>,
    Path((thread_id, turn_id)): Path<(String, String)>,
    Body(body): Body<TurnBody>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    validate_id("turn_id", &turn_id)?;
    validate_priority(&body.priority)?;

    let op = state
        .bridge
        .ops
        .for_turn(&thread_id, &turn_id)
        .ok_or_else(|| {
            BridgeError::not_found("turn_not_found", "No such turn on this conversation.")
                .with("thread_id", thread_id.clone())
                .with("turn_id", turn_id.clone())
        })?;
    if op.is_finished() {
        return Err(BridgeError::conflict(
            "invalid_state",
            "That turn has already finished; start a new one.",
        )
        .with("turn_id", turn_id));
    }

    let normalized = inputs::normalize(&body.input)?;
    let mut params = json!({
        "sessionId": thread_id,
        "turnId": turn_id,
        "content": normalized.content(),
    });
    // Steering defaults to `now` — the point is to redirect work in flight.
    params["priority"] = json!(body.priority.as_deref().unwrap_or("now"));

    state
        .bridge
        .host
        .call(&state, "session.input", params, RPC_TIMEOUT)
        .await?;

    op.emit(
        &state.bridge.ops.global,
        &json!({
            "method": "turn/steered",
            "params": {
                "thread_id": thread_id,
                "turn_id": turn_id,
                "text": normalized.text,
            }
        }),
    );
    Ok(ok(op.snapshot()))
}

/// `POST /threads/{id}/turns/{turn}/interrupt` — stop a running turn and keep
/// the conversation.
pub async fn interrupt(
    State(state): State<AppState>,
    Path((thread_id, turn_id)): Path<(String, String)>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    validate_id("turn_id", &turn_id)?;

    let op = state
        .bridge
        .ops
        .for_turn(&thread_id, &turn_id)
        .ok_or_else(|| {
            BridgeError::not_found("turn_not_found", "No such turn on this conversation.")
                .with("thread_id", thread_id.clone())
                .with("turn_id", turn_id.clone())
        })?;

    // A finished turn is not interruptible, and asking the host to interrupt
    // "whatever is running" would kill whichever turn started after this one.
    if op.is_finished() {
        return Ok(ok(op.snapshot()));
    }

    state
        .bridge
        .host
        .call(
            &state,
            "session.interrupt",
            // Naming the turn lets the host refuse if a different one is live.
            json!({ "sessionId": thread_id, "turnId": turn_id }),
            RPC_TIMEOUT,
        )
        .await?;
    state.bridge.set_active_turn(&thread_id, None).await;
    Ok(ok(op.snapshot()))
}

/// The Codex-compatible one-shot: create-or-continue plus a blocking turn.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ChatBody {
    /// Text shortcut; use `input` for images and documents.
    pub message: Option<String>,
    pub input: Option<Value>,
    /// Continue this specific conversation. Naming one that is not currently
    /// running resumes it from disk, so any past conversation can be picked up.
    pub thread_id: Option<String>,
    /// Start a fresh conversation instead of joining the one in use.
    ///
    /// Defaults to false, which is the point: a client that only ever sends
    /// `{"message": "…"}` gets one continuous conversation rather than a new
    /// one per message. Accepted as `createNewChat` too.
    #[serde(default, alias = "createNewChat")]
    pub create_new_chat: bool,
    #[serde(flatten)]
    pub options: CreateThread,
}

/// `POST /chat` — the smallest useful call: one message in, one answer out.
pub async fn chat(
    State(state): State<AppState>,
    connect: axum::extract::ConnectInfo<std::net::SocketAddr>,
    Body(body): Body<ChatBody>,
) -> BridgeResult<Response> {
    let input = match (&body.input, &body.message) {
        (Some(input), _) => input.clone(),
        (None, Some(message)) => Value::String(message.clone()),
        (None, None) => {
            return Err(BridgeError::invalid_request(
                "Send either `message` (text) or `input` (typed items).",
            ))
        }
    };

    // Validate before creating anything. Otherwise a malformed image costs a
    // conversation (and a subprocess) that is immediately orphaned.
    inputs::normalize(&input)?;

    let thread_id = resolve_chat_thread(&state, connect, &body).await?;

    let op = begin_turn(
        &state,
        &thread_id,
        &TurnBody {
            input,
            priority: None,
        },
    )
    .await?;

    let finished = op.wait_done(Duration::from_secs(RUN_TIMEOUT_SECS)).await;
    state.bridge.set_active_turn(&thread_id, None).await;
    let snapshot = op.snapshot();

    if !finished {
        return Err(BridgeError::timeout(
            "The turn did not finish in time; poll its operation instead.",
        )
        .with("operation_id", op.id.clone())
        .with("thread_id", thread_id));
    }
    if let Some(error) = snapshot.get("error").filter(|v| !v.is_null()) {
        return Ok((
            status_for_turn_error(error),
            axum::Json(json!({ "error": error })),
        )
            .into_response());
    }

    let result = &snapshot["result"];
    Ok(ok(json!({
        "thread_id": thread_id,
        "turn_id": op.turn_id,
        "operation_id": op.id,
        "status": snapshot["status"],
        "response": result.get("result").cloned().unwrap_or(Value::Null),
        "structured_output": result.get("structured_output").cloned().unwrap_or(Value::Null),
        "usage": result.get("usage").cloned().unwrap_or(Value::Null),
        "total_cost_usd": result.get("total_cost_usd").cloned().unwrap_or(Value::Null),
    })))
}

/// Decide which conversation a `/chat` message belongs to.
///
/// The rule, in order:
///
/// 1. an explicit `thread_id` always wins, and is resumed from disk if it is
///    not currently running;
/// 2. `create_new_chat: true` starts a fresh one and makes it the conversation
///    later messages join;
/// 3. otherwise the active conversation — set when the bridge was started, or
///    by the first message — continues;
/// 4. with no active conversation, one is created and becomes active.
///
/// Step 3 is the reason the flag defaults to false: a client that sends nothing
/// but `{"message": "…"}` should be having *a conversation*, not accumulating a
/// new one per message.
async fn resolve_chat_thread(
    state: &AppState,
    connect: axum::extract::ConnectInfo<std::net::SocketAddr>,
    body: &ChatBody,
) -> BridgeResult<String> {
    if let Some(id) = &body.thread_id {
        validate_id("thread_id", id)?;
        // Explicitly named: a failure to resume belongs to the caller.
        crate::bridge::ensure_thread_live(state, id).await?;
        return Ok(id.clone());
    }

    if !body.create_new_chat {
        if let Some(active) = state.bridge.active_thread.read().await.clone() {
            match crate::bridge::ensure_thread_live(state, &active).await {
                Ok(()) => return Ok(active),
                // The conversation we were pointing at is gone. That is not the
                // caller's problem — open a new one rather than failing a
                // message they addressed to nothing in particular.
                Err(e) => {
                    tracing::warn!(error = %e, thread_id = %active,
                        "the active conversation could not be resumed; starting a new one");
                    state.bridge.set_active_thread(None).await;
                }
            }
        }
    }

    let options = CreateThread {
        cwd: body.options.cwd.clone(),
        model: body.options.model.clone(),
        effort: body.options.effort.clone(),
        thinking: body.options.thinking.clone(),
        permission_mode: body.options.permission_mode.clone(),
        title: body.options.title.clone(),
        system_prompt_append: body.options.system_prompt_append.clone(),
        allowed_tools: body.options.allowed_tools.clone(),
        disallowed_tools: body.options.disallowed_tools.clone(),
        additional_directories: body.options.additional_directories.clone(),
        setting_sources: body.options.setting_sources.clone(),
        mcp_servers: body.options.mcp_servers.clone(),
        output_schema: body.options.output_schema.clone(),
        max_turns: body.options.max_turns,
        max_budget_usd: body.options.max_budget_usd,
        skills: body.options.skills.clone(),
        agents: body.options.agents.clone(),
        include_partial_messages: body.options.include_partial_messages,
        thread_id: body.options.thread_id.clone(),
        resume: body.options.resume.clone(),
        fork: body.options.fork,
    };
    let response = super::threads::create(State(state.clone()), connect, Body(options)).await?;
    let created = thread_id_from(response).await?;
    state.bridge.set_active_thread(Some(created.clone())).await;
    Ok(created)
}

/// Pull the thread id back out of a `POST /threads` response.
async fn thread_id_from(response: Response) -> BridgeResult<String> {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .map_err(|e| BridgeError::internal(format!("Could not read the created thread: {e}")))?;
    let value: Value = serde_json::from_slice(&bytes)?;
    value
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| BridgeError::internal("Creating the conversation returned no id."))
}

/// `GET /operations/{id}` — poll one operation.
pub async fn operation(
    State(state): State<AppState>,
    Path(operation_id): Path<String>,
) -> BridgeResult<Response> {
    validate_id("operation_id", &operation_id)?;
    Ok(ok(state.bridge.ops.get(&operation_id)?.snapshot()))
}

/// `GET /operations/{id}/events` — replayable SSE for one turn.
pub async fn operation_events(
    State(state): State<AppState>,
    Path(operation_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> BridgeResult<Response> {
    validate_id("operation_id", &operation_id)?;
    let op = state.bridge.ops.get(&operation_id)?;
    // An operation stream replays from the beginning by default, so a client
    // that connects after the turn finished still gets the whole thing.
    let cursor = super::resolve_cursor(&params, &headers, 0)?;
    sse(op.log.clone(), cursor).await
}

/// `GET /events` — everything, across every conversation.
pub async fn global_events(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> BridgeResult<Response> {
    // A fresh global subscriber starts at the oldest retained event rather than
    // at 0, which would 410 the moment the buffer had wrapped.
    let default = state.bridge.ops.global.oldest_cursor();
    let cursor = super::resolve_cursor(&params, &headers, default)?;
    sse(state.bridge.ops.global.clone(), cursor).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_failures_map_onto_actionable_statuses() {
        assert_eq!(
            status_for_turn_error(&json!({ "code": "claude_login_required" })),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status_for_turn_error(&json!({ "code": "runtime_unavailable" })),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status_for_turn_error(&json!({ "code": "timeout" })),
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            status_for_turn_error(&json!({ "code": "claude_error" })),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(status_for_turn_error(&json!({})), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn priority_is_constrained() {
        assert!(validate_priority(&None).is_ok());
        assert!(validate_priority(&Some("now".into())).is_ok());
        let err = validate_priority(&Some("urgent".into())).unwrap_err();
        assert_eq!(err.code, "invalid_field");
        assert_eq!(err.body()["error"]["field"], "priority");
    }

    #[test]
    fn turn_bodies_reject_unknown_fields() {
        assert!(serde_json::from_value::<TurnBody>(json!({ "input": "hi" })).is_ok());
        let err =
            serde_json::from_value::<TurnBody>(json!({ "input": "hi", "modl": "x" })).unwrap_err();
        assert!(err.to_string().starts_with("unknown field"));
    }

    #[test]
    fn create_new_chat_defaults_to_false_and_accepts_camel_case() {
        let body: ChatBody = serde_json::from_value(json!({ "message": "hi" })).unwrap();
        assert!(
            !body.create_new_chat,
            "a bare message must not open a new conversation"
        );

        let body: ChatBody =
            serde_json::from_value(json!({ "message": "hi", "create_new_chat": true })).unwrap();
        assert!(body.create_new_chat);

        // The spelling a JavaScript client would reach for.
        let body: ChatBody =
            serde_json::from_value(json!({ "message": "hi", "createNewChat": true })).unwrap();
        assert!(body.create_new_chat);
    }

    #[test]
    fn chat_accepts_message_or_input_and_flattens_create_options() {
        let body: ChatBody =
            serde_json::from_value(json!({ "message": "hi", "model": "haiku" })).unwrap();
        assert_eq!(body.message.as_deref(), Some("hi"));
        assert_eq!(body.options.model.as_deref(), Some("haiku"));

        let body: ChatBody = serde_json::from_value(json!({
            "input": [{ "type": "text", "text": "hi" }],
            "thread_id": "t1",
        }))
        .unwrap();
        assert!(body.input.is_some());
        assert_eq!(body.thread_id.as_deref(), Some("t1"));
    }

    #[tokio::test]
    async fn thread_id_is_recovered_from_a_create_response() {
        let response = super::super::created(json!({ "thread_id": "abc", "status": "created" }));
        assert_eq!(thread_id_from(response).await.unwrap(), "abc");

        let response = super::super::created(json!({ "status": "created" }));
        assert_eq!(
            thread_id_from(response).await.unwrap_err().code,
            "internal_error"
        );
    }
}
