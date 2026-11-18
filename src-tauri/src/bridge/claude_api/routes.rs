//! `POST /v1/messages`: the Claude API's Messages call, answered by Claude
//! Code on this computer (docs/CLAUDE-API.md).
//!
//! **A request's conversation.** A client names one (`x-conversation-id`,
//! `conversation_id`, `metadata.conversation_id`; `latest` for its folder's
//! last one with the request's settings), or its history finds one:
//! the history it sends before its last message is one of its
//! conversations' (fingerprints, [`super::convert`]). Found with the same
//! system prompt, tools and output settings, the conversation goes on in
//! its Claude Code session; found with others, a new conversation forks it
//! (the transcript carried over, the new settings in force). Not found, a
//! new conversation begins — with the history written into its first
//! message when there is one.
//!
//! **A turn.** The last message goes into the session as a turn; the
//! session's own stream of the API's events becomes the response, streamed
//! or whole. A client tool the model calls ends the response with
//! `tool_use`, while the turn waits for the client's next request to bring
//! the result. The answer is recorded before its last event goes out, so a
//! client that sends its next request at once finds it.

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use super::convert::{self, ToolResult};
use super::store::{Busy, Client, GroupBy, Lookup, Record, Store};
use super::stream::{self, Event, ToolCall, Translator};
use super::wire::{self, ApiError, Message, MessagesRequest, Role};
use crate::bridge::ops::Operation;
use crate::bridge::{MountPolicy, ThreadRecord};
use crate::server::context::ClientContext;
use crate::state::AppState;

/// The header a client names a conversation with, and gets it back in.
pub const CONVERSATION_HEADER: &str = "x-conversation-id";
/// What a client names instead of an id for its folder's latest
/// conversation with the request's settings (a new one when it has none).
pub const LATEST: &str = "latest";

fn is_latest(named: &str) -> bool {
    named.eq_ignore_ascii_case(LATEST)
}

/// How long a request waits for a named conversation that is ending its
/// last turn (normally a moment), before it is `409`.
const SETTLING_WAIT: Duration = Duration::from_secs(35);
/// The same for `latest`: its client would rather start a one-off than
/// spend its own deadline on a turn that does not end.
const LATEST_WAIT: Duration = Duration::from_secs(10);
/// The header that asks for the Anthropic API instead of Claude Code.
pub const BACKEND_HEADER: &str = "x-mother-claude-backend";

/// How long a turn may take to end after its message did.
const SETTLE: Duration = Duration::from_secs(30);
/// How long a host may take to ask for a client tool's result.
const HOST_CALL_WAIT: Duration = Duration::from_secs(30);
/// A whole answer may take this long (a long one, with thinking).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60 * 30);
/// A stream says it is alive this often.
const PING_EVERY: Duration = Duration::from_secs(10);
/// Conversations whose Claude Code process stays open at once; past it, the
/// one used longest ago is closed (it resumes from its transcript).
pub const MAX_OPEN: usize = 8;

/// The request being answered.
#[derive(Clone)]
struct Cx {
    state: AppState,
    request_id: String,
}

impl Cx {
    fn fail(&self, e: ApiError) -> Response {
        e.respond(&self.request_id)
    }
}

/// The client a request is from, as its server groups them. `key`: by the
/// key a client sends — any key: on a server without one it is only a name
/// (told apart by its hash, never kept).
pub fn client_of(
    mount: &MountPolicy,
    ctx: &ClientContext,
    headers: &HeaderMap,
    user_id: Option<&str>,
) -> Client {
    let ip = || {
        let label = super::store::ip_label(ctx.client_ip);
        Client {
            id: label.clone(),
            kind: "ip".into(),
            label,
        }
    };
    match GroupBy::parse(mount.defaults.group_clients_by.as_deref()) {
        GroupBy::None => Client {
            id: "default".into(),
            kind: "default".into(),
            label: "All clients".into(),
        },
        GroupBy::Ip => ip(),
        GroupBy::Key => match super::access::presented(headers) {
            Some(raw) => {
                let short = crate::claude::secrets::sha256_hex(raw.as_bytes())[..12].to_string();
                Client {
                    id: format!("key-{short}"),
                    kind: "key".into(),
                    label: format!("Key {short}"),
                }
            }
            None => ip(),
        },
        GroupBy::User => match user_id.map(str::trim).filter(|u| !u.is_empty()) {
            Some(u) => Client {
                id: super::store::folder_label(&format!("user-{u}")),
                kind: "user".into(),
                label: u.chars().take(80).collect(),
            },
            None => ip(),
        },
        GroupBy::Header => match headers
            .get(super::CLIENT_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            // The application names its folder, as deep as it likes.
            Some(h) => Client {
                id: super::store::client_path(h),
                kind: "header".into(),
                label: h.chars().take(80).collect(),
            },
            None => ip(),
        },
    }
}

/// Where a server keeps its clients' folders.
pub fn root_of(state: &AppState, mount: &MountPolicy) -> Option<PathBuf> {
    super::root_for(state, &mount.defaults)
}

/// What a conversation's process is started with, as one fingerprint: a
/// request asking for other settings forks the conversation.
fn settings_of(req: &MessagesRequest) -> String {
    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "schema": t.input_schema }))
        .collect();
    convert::digest(&json!({
        "system": system_prompt(req),
        "tools": tools,
        "toolChoiceNone": req.tool_choice.as_deref() == Some("none"),
        "thinking": req.thinking,
        "format": req.output_format,
        "effort": req.effort,
        "maxTokens": req.max_tokens,
    }))
}

/// The system prompt a conversation runs with: the client's, and what its
/// `tool_choice` asks (Claude Code has no way to force a tool call, so the
/// model is told).
fn system_prompt(req: &MessagesRequest) -> String {
    let hint = match req.tool_choice.as_deref() {
        Some("any") if !req.tools.is_empty() => {
            Some("Answer by calling one of your tools.".to_string())
        }
        Some("tool") => req
            .tool_name
            .as_deref()
            .map(|n| format!("Answer by calling the tool `{n}`.")),
        _ => None,
    };
    match (req.system.trim().is_empty(), hint) {
        (_, None) => req.system.clone(),
        (true, Some(h)) => h,
        (false, Some(h)) => format!("{}\n\n{h}", req.system),
    }
}

/// How the request goes on.
#[derive(Debug)]
enum Plan {
    /// A new conversation: forked from `fork_from`'s transcript, or begun
    /// (with the history written into `input` when `seeded`). `append`: the
    /// request carries only its last message, after `fork_from`'s history.
    New {
        input: Vec<Value>,
        seeded: bool,
        fork_from: Option<String>,
        append: bool,
    },
    /// The next message of a conversation. `append`: the request carries only
    /// it (the client keeps no history; the conversation does).
    Next { input: Vec<Value>, append: bool },
    /// The results of the client tools the conversation's turn waits on.
    Results {
        results: Vec<ToolResult>,
        calls: Vec<ToolCall>,
    },
}

impl Plan {
    fn append(&self) -> bool {
        matches!(
            self,
            Plan::New { append: true, .. } | Plan::Next { append: true, .. }
        )
    }
}

/// `POST /v1/messages` (and `/messages`).
pub async fn create(
    State(state): State<AppState>,
    ctx: ClientContext,
    mount: MountPolicy,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let cx = Cx {
        state: state.clone(),
        request_id: wire::request_id(),
    };
    let mut body: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return cx.fail(ApiError::invalid(format!(
                "There was an issue with your request body: {e}"
            )))
        }
    };
    let asked = headers
        .get(BACKEND_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_ascii_lowercase());
    let backend = asked
        .as_deref()
        .or(mount.defaults.messages_backend.as_deref())
        .unwrap_or(super::BACKEND_CLAUDE_CODE);
    match backend {
        super::BACKEND_ANTHROPIC => return pass_through(&cx, &headers, body).await,
        super::BACKEND_CLAUDE_CODE => {}
        other => {
            return cx.fail(ApiError::invalid(format!(
                "{BACKEND_HEADER}: {other:?} is not a backend; use claude-code or anthropic"
            )))
        }
    }
    // The server's default model, for a request that names none.
    if body.get("model").is_none_or(Value::is_null) {
        if let (Some(m), Some(obj)) = (&mount.defaults.model, body.as_object_mut()) {
            obj.insert("model".into(), json!(m));
        }
    }
    let req = match MessagesRequest::parse(&body, false) {
        Ok(r) => r,
        Err(e) => return cx.fail(e),
    };
    if req
        .messages
        .last()
        .is_some_and(|m| m.role == Role::Assistant)
    {
        return cx.fail(ApiError::invalid(
            "messages: a last message from the assistant (prefill) is not available when Claude \
             Code answers (Mother Claude); end with a user message",
        ));
    }
    if req.max_tokens == 0 {
        // The API's cache warm-up: nothing to generate.
        return json_response(&cx, empty_message(&req), Vec::new());
    }
    let Some(root) = root_of(&state, &mount) else {
        return cx.fail(ApiError::unavailable(
            "Mother Claude has no folder to keep conversations in.",
        ));
    };
    let client = client_of(&mount, &ctx, &headers, req.user_id.as_deref());
    let dir = match Store::client_dir(&root, &client) {
        Ok(d) => d,
        Err(e) => {
            return cx.fail(ApiError::api(format!(
                "Could not make the client's folder {}: {e}",
                root.join(&client.id).display()
            )))
        }
    };
    Store::note_client(&dir, &client, crate::claude::registry::now_ms());
    super::sweep::ensure(&state);

    let named = headers
        .get(CONVERSATION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| req.conversation.clone());
    let req = Arc::new(req);
    let (id, plan) = match resolve(&cx, &dir, &req, named.as_deref()).await {
        Ok(p) => p,
        Err(e) => return cx.fail(e),
    };
    // `latest` keeps one conversation going: the one it began or went on in
    // (never a one-off a request began without it).
    if named.as_deref().is_some_and(is_latest) {
        state.bridge.api.with(&id, |c| c.record.latest = true);
    }
    let account = state
        .bridge
        .api
        .with(&id, |c| c.record.account.clone())
        .filter(|a| !a.is_empty() && state.accounts.get(a).is_some())
        .or_else(|| mount.defaults.account.clone())
        .unwrap_or_else(|| state.accounts.account_for_bridge(&dir.to_string_lossy()));
    if let Some(only) = mount.defaults.only_account() {
        if account != only {
            state.bridge.api.release(&id);
            return cx.fail(ApiError::not_found(format!(
                "{CONVERSATION_HEADER}: no conversation {id} for this client"
            )));
        }
    }
    let (op, cursor, plan) =
        match prepare(&cx, &mount, &id, &dir, &client, &req, plan, &account).await {
            Ok(v) => v,
            Err(e) => {
                forget_unanswered(&state, &id).await;
                state.bridge.api.release(&id);
                return cx.fail(e);
            }
        };
    let mut extra = vec![(
        HeaderName::from_static(CONVERSATION_HEADER),
        HeaderValue::from_str(&id).unwrap_or_else(|_| HeaderValue::from_static("")),
    )];
    if let Some(h) = wire::ignored_header(&req.ignored) {
        extra.push(h);
    }

    let (done_tx, done_rx) = oneshot::channel();
    if req.stream {
        let (tx, rx) = mpsc::unbounded_channel();
        answer(state.clone(), id, req, plan, op, cursor, Some(tx), done_tx);
        sse_response(cx, rx, extra).await
    } else {
        answer(state.clone(), id, req, plan, op, cursor, None, done_tx);
        match done_rx.await {
            Ok(Ok(message)) => json_response(&cx, message, extra),
            Ok(Err(e)) => cx.fail(e),
            Err(_) => cx.fail(ApiError::api(
                "Claude Code ended the turn without an answer.",
            )),
        }
    }
}

/// The Anthropic API answers instead (`messagesBackend: anthropic`): the
/// request as it came, its `anthropic-beta` header carried along.
async fn pass_through(cx: &Cx, headers: &HeaderMap, mut body: Value) -> Response {
    if let (Some(beta), Some(obj)) = (
        headers.get("anthropic-beta").and_then(|v| v.to_str().ok()),
        body.as_object_mut(),
    ) {
        if !obj.contains_key("betas") {
            obj.insert("betas".into(), json!(beta));
        }
    }
    match crate::bridge::routes::messages::create(
        State(cx.state.clone()),
        crate::bridge::routes::Body(body),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => cx.fail(e.into()),
    }
}

/// `max_tokens: 0`: the cache warm-up the API does, nothing generated.
fn empty_message(req: &MessagesRequest) -> Value {
    json!({
        "id": format!("msg_mc{}", &uuid::Uuid::new_v4().simple().to_string()[..20]),
        "type": "message",
        "role": "assistant",
        "model": req.model,
        "content": [],
        "stop_reason": "max_tokens",
        "stop_sequence": null,
        "usage": { "input_tokens": 0, "output_tokens": 0 },
    })
}

fn json_response(cx: &Cx, body: Value, extra: Vec<(HeaderName, HeaderValue)>) -> Response {
    let mut resp = (StatusCode::OK, axum::Json(body)).into_response();
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&cx.request_id) {
        h.insert("request-id", v);
    }
    for (k, v) in extra {
        h.insert(k, v);
    }
    resp
}

/// Waits for a conversation that is ending its last turn, up to a moment
/// past [`SETTLE`]. Whether it was freed in time.
async fn wait_released(store: &Store, id: &str, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        let released = store.released.notified();
        if store.state_of(id).is_none_or(|(busy, _)| !busy) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        let _ = tokio::time::timeout(Duration::from_millis(500), released).await;
    }
}

/// Which conversation the request goes on in, and how — claimed (busy).
async fn resolve(
    cx: &Cx,
    dir: &FsPath,
    req: &MessagesRequest,
    named: Option<&str>,
) -> Result<(String, Plan), ApiError> {
    let store = &cx.state.bridge.api;
    let settings = settings_of(req);
    let last_at = req.messages.len() - 1;
    let history = convert::fingerprints(&req.messages[..last_at]);

    // `latest`: the folder's conversation `latest` used last with these
    // settings; when it has none, the request goes on as one that names none.
    let via_latest = named.is_some_and(is_latest);
    let named = match named {
        Some(_) if via_latest => store.latest(dir, &settings),
        other => other.map(str::to_string),
    };
    if let Some(id) = named.as_deref() {
        if !crate::claude::ids::is_uuid(id) {
            return Err(ApiError::invalid(format!(
                "{CONVERSATION_HEADER}: {id:?} is not a conversation id"
            )));
        }
        let mut claimed = store.claim(dir, id);
        let wait = if via_latest {
            LATEST_WAIT
        } else {
            SETTLING_WAIT
        };
        if claimed == Err(Busy::Settling) && wait_released(store, id, wait).await {
            claimed = store.claim(dir, id);
        }
        match claimed {
            Ok(()) => {}
            Err(Busy::Missing) => {
                return Err(ApiError::not_found(format!(
                    "{CONVERSATION_HEADER}: no conversation {id} for this client"
                )))
            }
            Err(_) => {
                return Err(ApiError::conflict(
                    "This conversation is answering another request; wait for it, or start a new \
                     one.",
                ))
            }
        }
        let (known, same_settings) = store
            .with(id, |c| {
                (c.record.history.clone(), c.record.settings == settings)
            })
            .unwrap_or_default();
        let append = req.messages.len() == 1 && !known.is_empty();
        if history == known || append {
            if same_settings {
                return next(cx, id, req, append);
            }
            // Other settings: a new conversation goes on from this one.
            store.release(id);
            return new_conversation(cx, dir, req, settings, Some(id), append);
        }
        // Another history than the conversation's: a branch of its own.
        store.release(id);
        return new_conversation(cx, dir, req, settings, None, false);
    }

    let mut found = store.lookup(dir, &settings, &history);
    if let Lookup::Settling(id) = &found {
        wait_released(store, id, SETTLING_WAIT).await;
        found = store.lookup(dir, &settings, &history);
    }
    match found {
        Lookup::Claimed(id) => next(cx, &id, req, false),
        Lookup::Fork(from) => new_conversation(cx, dir, req, settings, Some(&from), false),
        Lookup::Settling(_) | Lookup::Nothing => {
            new_conversation(cx, dir, req, settings, None, false)
        }
    }
}

/// The next message of a claimed conversation: the tool results its turn
/// waits on, or a new turn.
fn next(
    cx: &Cx,
    id: &str,
    req: &MessagesRequest,
    append: bool,
) -> Result<(String, Plan), ApiError> {
    let store = &cx.state.bridge.api;
    let last_at = req.messages.len() - 1;
    let last = &req.messages[last_at];
    let at = format!("messages.{last_at}.content");
    let (waiting, live) = store
        .with(id, |c| {
            (
                c.waiting.clone(),
                c.op.as_ref().is_some_and(|o| !o.is_finished()),
            )
        })
        .unwrap_or_default();
    let (results, _) = convert::split_tool_results(&last.content);
    if !waiting.is_empty() && live {
        let missing: Vec<&str> = waiting
            .iter()
            .filter(|c| !results.iter().any(|r| r.tool_use_id == c.id))
            .map(|c| c.id.as_str())
            .collect();
        if !missing.is_empty() {
            store.release(id);
            return Err(ApiError::invalid(format!(
                "messages.{last_at}: `tool_use` ids were found without `tool_result` blocks \
                 immediately after: {}. Each `tool_use` block must have a corresponding \
                 `tool_result` block in the next message.",
                missing.join(", ")
            )));
        }
        return Ok((
            id.to_string(),
            Plan::Results {
                results,
                calls: waiting,
            },
        ));
    }
    if !waiting.is_empty() {
        // The turn that asked is gone: the results are read as a message.
        store.with(id, |c| {
            c.waiting.clear();
            c.op = None;
        });
    }
    let input = if results.is_empty() {
        convert::for_claude_code(&last.content, &at)
    } else {
        convert::describe_tool_results(&last.content, &at)
    };
    match input {
        Ok(input) => Ok((id.to_string(), Plan::Next { input, append })),
        Err(e) => {
            store.release(id);
            Err(e)
        }
    }
}

/// A new conversation for the request, claimed: forked from `fork_from`, or
/// begun with the request's history written into its first message.
fn new_conversation(
    cx: &Cx,
    dir: &FsPath,
    req: &MessagesRequest,
    settings: String,
    fork_from: Option<&str>,
    append: bool,
) -> Result<(String, Plan), ApiError> {
    let store = &cx.state.bridge.api;
    let last_at = req.messages.len() - 1;
    let last = &req.messages[last_at];
    let at = format!("messages.{last_at}.content");
    let (results, _) = convert::split_tool_results(&last.content);
    let first = |content: &[Value]| {
        if results.is_empty() {
            convert::for_claude_code(content, &at)
        } else {
            convert::describe_tool_results(content, &at)
        }
    };
    let (input, seeded) = match fork_from {
        Some(_) => (first(&last.content)?, false),
        None if req.messages.len() == 1 => (first(&last.content)?, false),
        None => (
            convert::seeded_first_message(&req.messages[..last_at], &last.content)?,
            true,
        ),
    };
    // A fork in which the client sends only its last message goes on from
    // the history its source kept.
    let (history, messages, account) = match fork_from {
        Some(from) if append => store
            .with(from, |c| {
                (
                    c.record.history.clone(),
                    c.record.messages.clone(),
                    c.record.account.clone(),
                )
            })
            .unwrap_or_default(),
        Some(from) => (
            Vec::new(),
            Vec::new(),
            store
                .with(from, |c| c.record.account.clone())
                .unwrap_or_default(),
        ),
        None => Default::default(),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::claude::registry::now_ms();
    store.insert(
        Record {
            id: id.clone(),
            created_at: now,
            updated_at: now,
            model: req.model.clone(),
            settings,
            title: req.first_text(),
            seeded,
            forked_from: fork_from.map(str::to_string),
            account,
            history,
            messages,
            ..Record::default()
        },
        dir.to_path_buf(),
    );
    Ok((
        id,
        Plan::New {
            input,
            seeded,
            fork_from: fork_from.map(str::to_string),
            append,
        },
    ))
}

/// Whether a conversation's transcript is where Claude Code resumes it
/// from.
fn transcript_exists(state: &AppState, account: &str, cwd: &str, id: &str) -> bool {
    state
        .accounts
        .home(account)
        .is_some_and(|h| h.transcript_path(cwd, id).is_file())
}

/// Opens the conversation's session when needed and starts its turn — or
/// hands the tools' results to the turn waiting on them: the operation to
/// read, where its events start, and the plan as it ran.
#[allow(clippy::too_many_arguments)]
async fn prepare(
    cx: &Cx,
    mount: &MountPolicy,
    id: &str,
    dir: &FsPath,
    client: &Client,
    req: &MessagesRequest,
    plan: Plan,
    account: &str,
) -> Result<(Arc<Operation>, u64, Plan), ApiError> {
    let state = &cx.state;
    let store = &state.bridge.api;
    let cwd = dir.to_string_lossy().into_owned();
    store.with(id, |c| {
        c.record.client = client.id.clone();
        c.record.client_kind = client.kind.clone();
        c.record.server = mount.server_id().to_string();
        c.record.account = account.to_string();
    });

    if let Plan::Results { results, calls } = &plan {
        let (op, cursor) = store
            .with(id, |c| (c.op.clone(), c.cursor))
            .unwrap_or((None, 0));
        let op = op
            .filter(|o| !o.is_finished())
            .ok_or_else(|| ApiError::api("The turn waiting for these tool results ended."))?;
        give_results(state, id, results, calls).await?;
        store.with(id, |c| c.waiting.clear());
        return Ok((op, cursor, plan));
    }

    let open =
        store.with(id, |c| c.open).unwrap_or(false) && state.bridge.thread(id).await.is_some();
    let mut plan = plan;
    if !open {
        make_room(state).await;
        let resume = match &plan {
            Plan::New {
                fork_from: Some(from),
                ..
            } if transcript_exists(state, account, &cwd, from) => Some((from.clone(), true)),
            Plan::Next { .. } if transcript_exists(state, account, &cwd, id) => {
                Some((id.to_string(), false))
            }
            _ => None,
        };
        if resume.is_none() {
            // Nothing to resume from (deleted, pruned, another account's):
            // the history is written into the message.
            plan = reseed(state, id, req, plan)?;
        }
        open_session(state, mount, id, &cwd, req, account, resume).await?;
        store.with(id, |c| c.open = true);
    } else if store
        .with(id, |c| c.record.model != req.model)
        .unwrap_or(false)
    {
        state
            .bridge
            .host
            .call(
                state,
                "session.setModel",
                json!({ "sessionId": id, "model": req.model }),
                Duration::from_secs(15),
            )
            .await
            .map_err(ApiError::from)?;
        state
            .bridge
            .update_thread(id, |r| r.model = Some(req.model.clone()))
            .await;
        store.with(id, |c| c.record.model = req.model.clone());
    }
    let input = match &plan {
        Plan::New { input, .. } | Plan::Next { input, .. } => input.clone(),
        Plan::Results { .. } => Vec::new(),
    };
    let op = crate::bridge::routes::turns::begin_turn_blocks(state, id, Value::Array(input))
        .await
        .map_err(ApiError::from)?;
    store.with(id, |c| c.op = Some(op.clone()));
    Ok((op, 0, plan))
}

/// A conversation that cannot resume from a transcript: its history,
/// written into the message, begins a session of its own.
fn reseed(state: &AppState, id: &str, req: &MessagesRequest, plan: Plan) -> Result<Plan, ApiError> {
    let last_at = req.messages.len() - 1;
    let last = &req.messages[last_at];
    let (input, append) = match plan {
        Plan::New {
            input,
            fork_from: None,
            seeded,
            append,
        } => {
            return Ok(Plan::New {
                input,
                seeded,
                fork_from: None,
                append,
            })
        }
        Plan::New { input, append, .. } | Plan::Next { input, append } => (input, append),
        p @ Plan::Results { .. } => return Ok(p),
    };
    let history: Vec<Message> = if append {
        state
            .bridge
            .api
            .with(id, |c| {
                c.record
                    .messages
                    .iter()
                    .filter_map(convert::from_record)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        req.messages[..last_at].to_vec()
    };
    if history.is_empty() {
        return Ok(Plan::New {
            input,
            seeded: false,
            fork_from: None,
            append,
        });
    }
    let (results, _) = convert::split_tool_results(&last.content);
    let last_blocks = if results.is_empty() {
        last.content.clone()
    } else {
        convert::describe_tool_results(&last.content, &format!("messages.{last_at}.content"))?
    };
    state.bridge.api.with(id, |c| c.record.seeded = true);
    Ok(Plan::New {
        input: convert::seeded_first_message(&history, &last_blocks)?,
        seeded: true,
        fork_from: None,
        append,
    })
}

/// Closes the open conversations used longest ago while [`MAX_OPEN`] are
/// open (never one answering or waiting on tool results).
async fn make_room(state: &AppState) {
    let open = state.bridge.api.open_ones();
    let mut over = (open.len() + 1).saturating_sub(MAX_OPEN);
    for (id, active) in open {
        if over == 0 {
            break;
        }
        if active {
            continue;
        }
        close_conversation(state, &id).await;
        over -= 1;
    }
}

/// Closes a conversation's session (its record stays: the next request
/// resumes it from its transcript). Tool calls it waits on are answered
/// with an error.
pub async fn close_conversation(state: &AppState, id: &str) {
    for call in state.bridge.api.drain_host_calls(id) {
        let _ = state
            .bridge
            .host
            .call_if_running(
                state,
                "request.respond",
                json!({
                    "requestId": call.request_id,
                    "result": {
                        "content": "The client did not send this tool's result.",
                        "is_error": true,
                    },
                }),
                Duration::from_secs(10),
            )
            .await;
    }
    state.bridge.api.with(id, |c| {
        c.open = false;
        c.op = None;
        c.cursor = 0;
        c.waiting.clear();
    });
    crate::bridge::close_thread(state, id).await;
}

/// Opens a conversation's Claude Code session: the model alone, with the
/// client's system prompt and tools. `resume`: the transcript it goes on
/// from (`true`: a fork of it).
#[allow(clippy::too_many_arguments)]
async fn open_session(
    state: &AppState,
    mount: &MountPolicy,
    id: &str,
    cwd: &str,
    req: &MessagesRequest,
    account: &str,
    resume: Option<(String, bool)>,
) -> Result<(), ApiError> {
    let env = state.accounts.env(account).map_err(|e| {
        ApiError::unavailable(format!(
            "The Claude account this API runs as cannot be used: {e}"
        ))
    })?;
    let mut options = json!({
        "bare": true,
        "cwd": cwd,
        "permissionMode": "default",
        "includePartialMessages": true,
        "model": req.model,
        "systemPrompt": system_prompt(req),
        "maxOutputTokens": req.max_tokens,
    });
    if let Some(title) = req.first_text() {
        options["title"] = json!(title);
    }
    if !req.tools.is_empty() && req.tool_choice.as_deref() != Some("none") {
        options["clientTools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    })
                })
                .collect(),
        );
    }
    let t = req.effective_thinking();
    let mut tc = json!({ "type": t.get("type").cloned().unwrap_or(json!("disabled")) });
    if let Some(b) = t.get("budget_tokens") {
        tc["budgetTokens"] = b.clone();
    }
    if let Some(d) = t.get("display") {
        tc["display"] = d.clone();
    }
    options["thinkingConfig"] = tc;
    if let Some(e) = &req.effort {
        options["effort"] = json!(e);
    }
    if let Some(f) = &req.output_format {
        options["outputFormat"] = json!({ "type": "json_schema", "schema": f["schema"] });
    }
    let persisted = resume.is_some();
    if let Some((from, fork)) = resume {
        options["resume"] = json!(from);
        if fork {
            options["forkSession"] = json!(true);
        }
    }
    state.bridge.remember_options(id, &options);
    if !env.is_inherit() {
        options["env"] = json!(env.overrides());
    }
    state
        .bridge
        .host
        .call(
            state,
            "session.create",
            json!({ "sessionId": id, "options": options }),
            crate::bridge::host::CREATE_TIMEOUT,
        )
        .await
        .map_err(ApiError::from)?;
    let now = crate::claude::registry::now_ms();
    state
        .bridge
        .insert_thread(ThreadRecord {
            thread_id: id.to_string(),
            cwd: cwd.to_string(),
            created_at: now,
            model: Some(req.model.clone()),
            effort: req.effort.clone(),
            permission_mode: "default".into(),
            title: req.first_text(),
            persisted,
            active_turn: None,
            account: account.to_string(),
            server: mount.server_id().to_string(),
        })
        .await;
    state.mark_owned(id, cwd, now).await;
    Ok(())
}

/// Hands the client's tool results to the calls the turn waits on.
async fn give_results(
    state: &AppState,
    id: &str,
    results: &[ToolResult],
    calls: &[ToolCall],
) -> Result<(), ApiError> {
    let store = &state.bridge.api;
    for call in calls {
        let Some(result) = results.iter().find(|r| r.tool_use_id == call.id) else {
            continue;
        };
        let deadline = Instant::now() + HOST_CALL_WAIT;
        let host_call = loop {
            let changed = store.calls_changed.notified();
            if let Some(c) = store.take_host_call(id, &call.name, &call.input) {
                break c;
            }
            if Instant::now() >= deadline {
                return Err(ApiError::api(format!(
                    "Claude Code did not ask for the result of {} ({}) in time.",
                    call.name, call.id
                )));
            }
            let _ = tokio::time::timeout(Duration::from_millis(500), changed).await;
        };
        state
            .bridge
            .host
            .call(
                state,
                "request.respond",
                json!({
                    "requestId": host_call.request_id,
                    "result": { "content": result.content, "is_error": result.is_error },
                }),
                Duration::from_secs(15),
            )
            .await
            .map_err(ApiError::from)?;
    }
    Ok(())
}

/// An error Claude Code reported for the API call, in the API's words.
fn sdk_error(msg: &Value) -> Option<ApiError> {
    match msg.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            let kind = msg.get("error").and_then(Value::as_str)?;
            let text = msg
                .pointer("/message/content/0/text")
                .and_then(Value::as_str)
                .unwrap_or(kind)
                .to_string();
            Some(match kind {
                "authentication_failed" => ApiError::authentication(format!(
                    "Claude Code on this computer is not signed in to Claude ({text}). Sign in on \
                     the Mother Claude desktop app (Accounts) or with `claude auth login`."
                )),
                "rate_limit" => ApiError::rate_limited(text),
                "billing_error" => {
                    ApiError::new(StatusCode::PAYMENT_REQUIRED, "billing_error", text)
                }
                "invalid_request" => ApiError::invalid(text),
                "max_output_tokens" => return None,
                _ => ApiError::api(text),
            })
        }
        Some("result") if msg.get("is_error").and_then(Value::as_bool) == Some(true) => {
            let text = msg
                .get("errors")
                .and_then(Value::as_array)
                .map(|e| {
                    e.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    msg.get("result")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "Claude Code could not answer.".into());
            Some(
                if text.contains("Not logged in") || text.contains("/login") {
                    ApiError::authentication(text)
                } else {
                    ApiError::api(text)
                },
            )
        }
        _ => None,
    }
}

/// How a finished turn that sent no message failed.
fn op_error(event: &Value) -> ApiError {
    let p = event.get("params").cloned().unwrap_or_default();
    let message = p
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("Claude Code ended the turn without an answer.")
        .to_string();
    match p.pointer("/error/code").and_then(Value::as_str) {
        Some("claude_login_required") => ApiError::authentication(message),
        _ if p.get("status").and_then(Value::as_str) == Some("interrupted") => {
            ApiError::api("The turn was interrupted before it answered.")
        }
        _ => ApiError::api(message),
    }
}

/// Reads the turn's events from `cursor` until the client's message is
/// whole, handing each API event to `sink` (`None`: a while passed with
/// none; `false` back: the client is gone).
async fn drive(
    op: &Operation,
    cursor: &mut u64,
    tr: &mut Translator,
    mut sink: impl FnMut(Option<Event>) -> bool,
) -> Result<(), ApiError> {
    let started = Instant::now();
    loop {
        if started.elapsed() > ANSWER_TIMEOUT {
            return Err(ApiError::timeout(
                "Claude Code did not finish the answer in time.",
            ));
        }
        let out = op
            .log
            .read(*cursor, PING_EVERY)
            .await
            .map_err(ApiError::from)?;
        if out.events.is_empty() && !out.closed {
            if !sink(None) {
                return Ok(());
            }
            continue;
        }
        for e in &out.events {
            *cursor = e.seq;
            let Ok(v) = serde_json::from_str::<Value>(&e.data) else {
                continue;
            };
            match v.get("method").and_then(Value::as_str) {
                Some("message") => {
                    let msg = v.pointer("/params/message").cloned().unwrap_or_default();
                    if !tr.started() {
                        if let Some(err) = sdk_error(&msg) {
                            return Err(err);
                        }
                    }
                    for ev in tr.push(&msg) {
                        if !sink(Some(ev)) {
                            return Ok(());
                        }
                    }
                    if tr.ended() {
                        return Ok(());
                    }
                }
                Some("bridge/completed") | Some("bridge/error") => {
                    let completed =
                        v.pointer("/params/status").and_then(Value::as_str) == Some("completed");
                    if tr.started() && completed {
                        // Ended without the message's end: close it as it is.
                        for ev in tr.finish() {
                            if !sink(Some(ev)) {
                                return Ok(());
                            }
                        }
                        return Ok(());
                    }
                    return Err(op_error(&v));
                }
                _ => {}
            }
        }
        if out.closed {
            return Err(ApiError::api(
                "Claude Code ended the turn without an answer.",
            ));
        }
    }
}

/// The answer is whole: recorded in its conversation (which a request that
/// continues it now finds), the conversation marked as ending its turn.
fn record_answer(
    state: &AppState,
    id: &str,
    req: &MessagesRequest,
    plan: &Plan,
    op: &Arc<Operation>,
    cursor: u64,
    tr: &Translator,
) {
    let calls = if tr.stop_reason().as_deref() == Some("tool_use") {
        tr.tool_calls()
    } else {
        Vec::new()
    };
    let answer = tr.message();
    let response = Message {
        role: Role::Assistant,
        content: answer
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    };
    let append = plan.append();
    state.bridge.api.with(id, |c| {
        let r = &mut c.record;
        if append {
            if let Some(last) = req.messages.last() {
                r.history.push(convert::fingerprint(last));
                r.messages.push(convert::for_record(last));
            }
        } else {
            r.history = convert::fingerprints(&req.messages);
            r.messages = req.messages.iter().map(convert::for_record).collect();
        }
        r.history.push(convert::fingerprint(&response));
        r.messages.push(convert::for_record(&response));
        r.requests += 1;
        r.updated_at = crate::claude::registry::now_ms();
        if let Some(m) = answer.get("model").and_then(Value::as_str) {
            r.model_used = Some(m.to_string());
        }
        if let Some(u) = answer.get("usage") {
            r.usage.add(u);
        }
        if calls.is_empty() {
            c.op = None;
            c.cursor = 0;
        } else {
            c.op = Some(op.clone());
            c.cursor = cursor;
        }
        c.waiting = calls;
        c.settling = true;
    });
}

/// After the answer went out: a turn that ends on its own is let end; one
/// cut short (a stop sequence, `max_tokens`, …) is interrupted; one waiting
/// on client tools waits. Then the conversation is free.
async fn settle(state: &AppState, id: &str, op: &Operation, tr: &Translator) {
    let waiting = state
        .bridge
        .api
        .with(id, |c| !c.waiting.is_empty())
        .unwrap_or(false);
    if !waiting {
        let natural = tr.stopped_at().is_none() && tr.stop_reason().as_deref() == Some("end_turn");
        if !natural || !op.wait_done(SETTLE).await {
            interrupt(state, id).await;
            op.wait_done(Duration::from_secs(10)).await;
        }
    }
    state.bridge.api.release(id);
}

async fn interrupt(state: &AppState, id: &str) {
    let _ = state
        .bridge
        .host
        .call_if_running(
            state,
            "session.interrupt",
            json!({ "sessionId": id }),
            Duration::from_secs(15),
        )
        .await;
}

/// A turn that failed: interrupted if it still runs, the conversation freed
/// (and forgotten, when it never answered).
async fn abandon(state: &AppState, id: &str, op: &Operation) {
    if !op.is_finished() {
        interrupt(state, id).await;
    }
    state.bridge.api.with(id, |c| {
        c.op = None;
        c.cursor = 0;
        c.waiting.clear();
    });
    forget_unanswered(state, id).await;
    state.bridge.api.release(id);
}

/// A new conversation that never answered is not kept: closed and
/// forgotten.
async fn forget_unanswered(state: &AppState, id: &str) {
    let unanswered = state
        .bridge
        .api
        .with(id, |c| c.record.history.is_empty())
        .unwrap_or(false);
    if unanswered {
        if state.bridge.thread(id).await.is_some() {
            close_conversation(state, id).await;
        }
        state.bridge.api.forget(id);
    }
}

/// Answers the request from its turn, in a task of its own (a client that
/// goes away never leaves a conversation claimed): stream frames to
/// `frames` when it streams; the whole message (or the error) to `done`.
#[allow(clippy::too_many_arguments)]
fn answer(
    state: AppState,
    id: String,
    req: Arc<MessagesRequest>,
    plan: Plan,
    op: Arc<Operation>,
    cursor: u64,
    frames: Option<mpsc::UnboundedSender<Result<String, ApiError>>>,
    done: oneshot::Sender<Result<Value, ApiError>>,
) {
    tokio::spawn(async move {
        let mut cursor = cursor;
        let mut tr =
            Translator::new(req.stop_sequences.clone()).structured(req.output_format.is_some());
        // The message's last event waits until the answer is recorded.
        let mut last_event: Option<Event> = None;
        let result = drive(&op, &mut cursor, &mut tr, |ev| match (&frames, ev) {
            (_, Some(e)) if e.name == "message_stop" => {
                last_event = Some(e);
                true
            }
            (Some(tx), Some(e)) => tx.send(Ok(stream::sse_frame(&e))).is_ok(),
            (Some(tx), None) => tx.send(Ok(stream::PING_FRAME.to_string())).is_ok(),
            (None, _) => !done.is_closed(),
        })
        .await;
        match result {
            Ok(()) if tr.ended() => {
                record_answer(&state, &id, &req, &plan, &op, cursor, &tr);
                if let (Some(tx), Some(e)) = (&frames, &last_event) {
                    let _ = tx.send(Ok(stream::sse_frame(e)));
                }
                drop(frames);
                let _ = done.send(Ok(tr.message()));
                settle(&state, &id, &op, &tr).await;
            }
            Ok(()) => {
                // The client went away before the answer was whole.
                drop(frames);
                abandon(&state, &id, &op).await;
            }
            Err(e) => {
                if let Some(tx) = &frames {
                    let _ = tx.send(Err(e.clone()));
                }
                drop(frames);
                let _ = done.send(Err(e));
                abandon(&state, &id, &op).await;
            }
        }
    });
}

/// A streamed answer: the API's server-sent events. An error before the
/// first event is the response's status; after it, an `error` event.
async fn sse_response(
    cx: Cx,
    mut rx: mpsc::UnboundedReceiver<Result<String, ApiError>>,
    extra: Vec<(HeaderName, HeaderValue)>,
) -> Response {
    let first = match rx.recv().await {
        Some(Ok(frame)) => frame,
        Some(Err(e)) => return cx.fail(e),
        None => {
            return cx.fail(ApiError::api(
                "Claude Code ended the turn without an answer.",
            ))
        }
    };
    let request_id = cx.request_id.clone();
    let stream = futures_util::stream::unfold(
        (Some(first), rx, false, request_id.clone()),
        |(first, mut rx, done, request_id)| async move {
            if let Some(f) = first {
                return Some((
                    Ok::<_, std::convert::Infallible>(Bytes::from(f)),
                    (None, rx, false, request_id),
                ));
            }
            if done {
                return None;
            }
            match rx.recv().await {
                Some(Ok(frame)) => Some((Ok(Bytes::from(frame)), (None, rx, false, request_id))),
                Some(Err(e)) => {
                    let data =
                        json!({ "type": "error", "error": e.object(), "request_id": request_id });
                    Some((
                        Ok(Bytes::from(format!("event: error\ndata: {data}\n\n"))),
                        (None, rx, true, request_id),
                    ))
                }
                None => None,
            }
        },
    );
    let mut resp = Response::new(axum::body::Body::from_stream(stream));
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    if let Ok(v) = HeaderValue::from_str(&request_id) {
        h.insert("request-id", v);
    }
    for (k, v) in extra {
        h.insert(k, v);
    }
    resp
}
