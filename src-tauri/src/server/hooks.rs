//! The foreign-control hook dispatcher (design §5.2, server side, with the
//! §10 review amendments).
//!
//! Owner: **B4 (foreign control)**. The plugin's `mc-hook` / `mc-wake`
//! scripts (see `claude::hooks`) call:
//!
//! * `POST /hooks/event` ([`post_event`], Hook principal: the `mch_…` token
//!   from a local client). Every event updates the session's hook state
//!   (process binding, turn state, in-flight tools, subagents, activity).
//!   PreToolUse / PostToolUse / Stop may answer with an interrupt
//!   (`{"continue":false}`) or a relayed message (`additionalContext`);
//!   SessionStart answers with the per-session relay marker; PermissionRequest
//!   is **held** as a [`crate::pending`] item (source `hook`) until someone
//!   answers it, it is withdrawn (answered at the desk, tool finished, turn
//!   over, process gone, client disconnected) or it times out.
//! * `POST /hooks/wait?pid=<claude pid>` ([`wait`], raw Stop payload as the
//!   body): the single wake listener per session (newest wins, older get 410),
//!   released with `200 <relay text>` when the session is idle and messages
//!   are queued, `204` after 50 s, `410` when it should stop (superseded,
//!   owned, not the session's process, process dead, control turned off).
//!
//! Owned session ids (Mother Claude's own children, the sidecar, bridge
//! threads) always get `{}` / 410: their approvals and messages go through
//! the owned channel. The pure state machine lives in `hooks/machine.rs`.
//!
//! Functions called by other agents (keep the signatures): [`is_active`]
//! (data layer, caps), [`enqueue_relay`], [`request_interrupt`],
//! [`cancel_queued`], [`queued`] (dispatchers), [`start`] (server).
//! AppState field: `state.hooks`.

mod machine;
mod payload;
mod relay;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;
use tokio::sync::oneshot;

pub use self::machine::{Arm, CancelError, WaitSignal, QUEUE_CAP};
pub use self::payload::{input_key, is_safe_id, is_uuid, HookPayload};
pub use self::relay::{MARKER_PREFIX, TEXT_MAX_BYTES as RELAY_TEXT_MAX};

use super::auth::HookOnly;
use super::context::ClientContext;
use super::error::{ApiError, ApiResult};
use crate::claude::model::{
    NoticeLevel, PendingItem, PendingKind, PendingSource, QueueStatus, QueuedMessage,
    SessionSummary, Surface, UiEvent, UiKind, UiRole,
};
use crate::pending::{self, Decision};
use crate::state::AppState;

/// Largest hook body (PostToolUse carries the whole tool response).
pub const HOOK_BODY_MAX: usize = 64 * 1024 * 1024;

/// How long a PermissionRequest is held for an interactive session (the
/// hook's own timeout is 3600 s, `mc-hook` gives curl 3590 s).
pub const HOLD_MAX: Duration = Duration::from_secs(3580);

/// Default hold for headless (`sdk-*`) sessions, and only while a dashboard
/// client is connected (`MOTHER_CLAUDE_HEADLESS_HOLD_SECS`).
pub const HEADLESS_HOLD_DEFAULT: Duration = Duration::from_secs(120);

/// How long a wake poll is parked before `204` (`mc-wake` gives curl 55 s).
pub const WAIT_HOLD: Duration = Duration::from_secs(50);

/// After this, a message to an idle session nobody listens for raises a
/// notice.
const NO_WAITER_NOTICE_AFTER: Duration = Duration::from_secs(10);

/// Pidfile watch interval.
const WATCH_EVERY: Duration = Duration::from_millis(300);

/// A permission prompt's input larger than this is not kept on the card.
const INPUT_KEEP_MAX: usize = 1024 * 1024;

/// Entrypoints of sessions that can show a prompt themselves (the hold runs
/// in parallel with their own dialog).
const INTERACTIVE_ENTRYPOINTS: [&str; 5] = [
    "cli",
    "claude-vscode",
    "claude-desktop",
    "jetbrains",
    "claude-jetbrains",
];

/// Per-session hook bookkeeping (the state machine plus timing knobs).
#[derive(Debug)]
pub struct HookHub {
    inner: Mutex<machine::Hub>,
    wait_hold_ms: AtomicU64,
    headless_hold_ms: AtomicU64,
    last_any: AtomicI64,
}

impl Default for HookHub {
    fn default() -> Self {
        let headless = std::env::var("MOTHER_CLAUDE_HEADLESS_HOLD_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|s| *s > 0)
            .map(Duration::from_secs)
            .unwrap_or(HEADLESS_HOLD_DEFAULT)
            .min(HOLD_MAX);
        Self {
            inner: Mutex::new(machine::Hub::new()),
            wait_hold_ms: AtomicU64::new(WAIT_HOLD.as_millis() as u64),
            headless_hold_ms: AtomicU64::new(headless.as_millis() as u64),
            last_any: AtomicI64::new(0),
        }
    }
}

impl HookHub {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, machine::Hub> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Record that `session_id` sent a hook event at `now_ms`.
    pub fn touch(&self, session_id: &str, now_ms: i64) {
        self.lock().mark_seen(session_id, now_ms);
        self.last_any.fetch_max(now_ms, Ordering::Relaxed);
    }

    /// Epoch ms of the session's last hook event.
    pub fn last_event_at(&self, session_id: &str) -> Option<i64> {
        self.lock()
            .session(session_id)
            .map(|s| s.last_event_ms)
            .filter(|t| *t > 0)
    }

    /// Epoch ms of the most recent hook event from any session.
    pub fn last_any_event_at(&self) -> Option<i64> {
        Some(self.last_any.load(Ordering::Relaxed)).filter(|t| *t > 0)
    }

    /// How long a wake poll is parked (tests shorten it).
    pub fn wait_hold(&self) -> Duration {
        Duration::from_millis(self.wait_hold_ms.load(Ordering::Relaxed))
    }

    pub fn set_wait_hold(&self, d: Duration) {
        self.wait_hold_ms
            .store(d.as_millis() as u64, Ordering::Relaxed);
    }

    /// How long a headless session's prompt is held.
    pub fn headless_hold(&self) -> Duration {
        Duration::from_millis(self.headless_hold_ms.load(Ordering::Relaxed))
    }

    pub fn set_headless_hold(&self, d: Duration) {
        self.headless_hold_ms
            .store(d.as_millis() as u64, Ordering::Relaxed);
    }

    /// The session's relay marker, once it has hook state.
    pub fn marker(&self, session_id: &str) -> Option<String> {
        self.lock().session(session_id).map(|s| s.marker.clone())
    }

    /// Whether the session's current turn is running (hook view).
    pub fn turn_active(&self, session_id: &str) -> bool {
        self.lock()
            .session(session_id)
            .is_some_and(|s| s.turn_active)
    }

    /// Held PermissionRequests of a session.
    pub fn held_count(&self, session_id: &str) -> usize {
        self.lock().held_count(session_id)
    }

    /// Ids of every session with hook state.
    pub fn session_ids(&self) -> Vec<String> {
        self.lock().session_ids()
    }

    /// Tell every wake listener to stop (foreign control turned off).
    pub fn release_all_waiters(&self) -> usize {
        self.lock().release_all_waiters()
    }
}

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

/// Whether `pid` names a live process (`kill(pid, 0)`; EPERM counts as
/// alive).
pub fn pid_alive(pid: u32) -> bool {
    let Ok(p) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if p <= 1 {
        return false;
    }
    // SAFETY: signal 0 performs only the existence/permission check.
    let r = unsafe { libc::kill(p, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether hooks from `session_id`'s current process were seen (drives
/// `ControlCaps.hooksActive` and the message dispatcher). Process-scoped: a
/// hook event or wake poll since the current pid appeared, never decaying
/// while that process lives.
pub fn is_active(state: &AppState, session_id: &str) -> bool {
    hooks_active(state, session_id, None)
}

/// [`is_active`] with the claude pid the caller knows (live roster first).
pub fn hooks_active(state: &AppState, session_id: &str, pid_hint: Option<u32>) -> bool {
    let live_pid = state.live.get(session_id).and_then(|l| l.pid).or(pid_hint);
    let hook_pid = {
        let hub = state.hooks.lock();
        let Some(s) = hub.session(session_id) else {
            return false;
        };
        if !s.seen || s.ended {
            return false;
        }
        s.pid
    };
    if let (Some(a), Some(b)) = (hook_pid, live_pid) {
        if a != b {
            return false;
        }
    }
    hook_pid.is_none_or(pid_alive)
}

/// Whether a wake listener is connected for the session right now.
pub fn wake_armed(state: &AppState, session_id: &str) -> bool {
    let now = now_ms();
    state
        .hooks
        .lock()
        .session(session_id)
        .is_some_and(|s| s.wake(now))
}

/// The permission mode the session's hooks last reported.
pub fn permission_mode(state: &AppState, session_id: &str) -> Option<String> {
    state
        .hooks
        .lock()
        .session(session_id)
        .and_then(|s| s.permission_mode.clone())
}

/// Whether the session runs in bypassPermissions: the latest hook report,
/// else the summary's mode, else the process argv
/// (`--dangerously-skip-permissions`, `--permission-mode bypassPermissions`).
///
/// A stand-in for the data layer's `effective_permission_mode` helper.
pub async fn effective_bypass(state: &AppState, session: &SessionSummary) -> bool {
    const BYPASS: &str = crate::claude::caps::BYPASS_MODE;
    if let Some(mode) = permission_mode(state, &session.id) {
        return mode == BYPASS;
    }
    if let Some(mode) = &session.permission_mode {
        return mode == BYPASS;
    }
    let pid = state
        .live
        .get(&session.id)
        .and_then(|l| l.pid)
        .or(session.pid);
    match pid {
        Some(pid) => argv_says_bypass(pid).await,
        None => false,
    }
}

async fn argv_says_bypass(pid: u32) -> bool {
    let out = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new("ps")
            .args(["-o", "args=", "-p", &pid.to_string()])
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await;
    let Ok(Ok(out)) = out else {
        return false;
    };
    argv_is_bypass(&String::from_utf8_lossy(&out.stdout))
}

/// Whether a command line launches without permission prompts.
pub fn argv_is_bypass(args: &str) -> bool {
    let words: Vec<&str> = args.split_whitespace().collect();
    words.iter().enumerate().any(|(i, w)| {
        *w == "--dangerously-skip-permissions"
            || *w == "--permission-mode=bypassPermissions"
            || (*w == "--permission-mode" && words.get(i + 1) == Some(&"bypassPermissions"))
    })
}

async fn is_owned_any(state: &AppState, id: &str) -> bool {
    crate::claude::owned::is_owned(state, id)
        || state.is_owned(id).await
        || state.bridge.thread(id).await.is_some()
}

// ───────────────────────────── outcomes ─────────────────────────────

fn relayed_event(m: &QueuedMessage) -> UiEvent {
    UiEvent {
        id: format!("relay:{}", m.id),
        ts: Some(now_ms()),
        role: UiRole::User,
        kind: UiKind::Relayed,
        text: Some(m.text.clone()),
        ..Default::default()
    }
}

fn record_delivered(state: &AppState, session: &str, msgs: &[QueuedMessage]) {
    for m in msgs {
        tracing::info!(session = %session, message = %m.id, "relay delivered");
        state.timeline.push(session, relayed_event(m));
    }
}

/// Carry out what the state machine decided; returns the hook response.
async fn apply(state: &AppState, session: &str, out: machine::Outcome) -> Value {
    for (id, reason) in &out.withdraw {
        pending::withdraw(state, id, reason).await;
    }
    record_delivered(state, session, &out.delivered);
    if let Some(ms) = out.turn_finished_ms {
        crate::server::push::on_turn_finished(state, session, ms);
    }
    if out.turn_failed {
        crate::server::push::on_session_failed(state, session, "The turn ended with an error.");
    }
    if out.caps_changed || !out.delivered.is_empty() {
        crate::claude::caps::refresh(state, session).await;
    }
    out.response
}

/// Feed the live roster's view of a session's pidfile (the watch loop calls
/// this; tests call it directly).
pub async fn observe_pidfile(state: &AppState, session: &str, status: &str, updated_at: i64) {
    let out = state
        .hooks
        .lock()
        .observe_pidfile(session, status, updated_at, now_ms());
    apply(state, session, out).await;
}

// ─────────────────────────────── events ───────────────────────────────

/// `POST /hooks/event`.
pub async fn post_event(
    HookOnly(_ctx): HookOnly,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(p) = HookPayload::parse(&body) else {
        return Json(relay::empty()).into_response();
    };
    drop(body);
    let pid = headers
        .get("x-mc-pid")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|p| *p > 1);
    Json(dispatch(&state, p, pid).await).into_response()
}

/// Handle one hook event; returns the hook's stdout JSON.
pub async fn dispatch(state: &AppState, p: HookPayload, pid: Option<u32>) -> Value {
    if !is_uuid(&p.session_id) {
        return relay::empty();
    }
    let Ok(agent) = p.agent() else {
        return relay::empty();
    };
    if is_owned_any(state, &p.session_id).await {
        return relay::empty();
    }
    let now = now_ms();
    let name = p.hook_event_name.as_str();
    // Never the payload: tool inputs, prompts and responses can hold secrets.
    tracing::debug!(
        event = name,
        session = %p.session_id,
        tool = p.tool_name.as_deref().unwrap_or(""),
        "hook event"
    );
    state.hooks.last_any.fetch_max(now, Ordering::Relaxed);
    state
        .activity
        .note_hook(&p.session_id, name, p.tool_name.as_deref(), agent);
    let key = p.tool_input.as_ref().map(input_key);
    let ev = machine::Event {
        name,
        session: &p.session_id,
        agent_id: agent,
        agent_type: p.agent_type.as_deref(),
        tool_use_id: p.tool_use(),
        tool_name: p.tool_name.as_deref(),
        input_key: key,
        prompt_id: p.prompt_id.as_deref(),
        prompt: p.prompt.as_deref(),
        permission_mode: p.permission_mode.as_deref(),
        pid,
    };
    let out = state.hooks.lock().on_event(&ev, now);
    let response = apply(state, &p.session_id, out).await;
    if name == "PermissionRequest" {
        return permission_request(state, &p, agent, key.unwrap_or_default()).await;
    }
    response
}

/// How long to hold a PermissionRequest of `session`, or `None` to answer
/// `{}` at once. Sessions that show their own prompt (terminal, VS Code,
/// desktop, JetBrains, tmux) are held for up to [`HOLD_MAX`] in parallel
/// with it; headless runs (`sdk-*`, unknown) only while a dashboard client
/// is connected, briefly — they would otherwise block on us.
async fn hold_policy(state: &AppState, session: &str) -> Option<Duration> {
    let live = state.live.get(session);
    let summary = state.find_session(session).await;
    let entrypoint = live
        .as_ref()
        .and_then(|l| l.entrypoint.clone())
        .or_else(|| summary.as_ref().and_then(|s| s.entrypoint.clone()));
    let tmux = live.as_ref().is_some_and(|l| l.tmux.is_some())
        || summary.as_ref().is_some_and(|s| s.tmux_pane.is_some());
    let interactive = match entrypoint.as_deref() {
        Some(e) if e.starts_with("sdk") => false,
        Some(e) => INTERACTIVE_ENTRYPOINTS.contains(&e),
        None => summary.as_ref().is_some_and(|s| {
            matches!(
                s.surface,
                Surface::Vscode
                    | Surface::Terminal
                    | Surface::Tmux
                    | Surface::Desktop
                    | Surface::Jetbrains
                    | Surface::Background
            )
        }),
    };
    if interactive || tmux {
        return Some(HOLD_MAX);
    }
    (state.hub.client_count() > 0).then(|| state.hooks.headless_hold())
}

/// The pending card for a PermissionRequest.
fn build_item(p: &HookPayload, agent: Option<&str>, tool_use_id: Option<String>) -> PendingItem {
    let tool = p.tool_name.clone().unwrap_or_default();
    let input = p
        .tool_input
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let (kind, title, questions, plan) = match tool.as_str() {
        "AskUserQuestion" => {
            let qs = pending::questions_from_input(&input);
            let title = qs
                .first()
                .map(|q| q.question.clone())
                .filter(|q| !q.trim().is_empty())
                .unwrap_or_else(|| "Claude has a question for you.".into());
            (PendingKind::Question, title, Some(qs), None)
        }
        "ExitPlanMode" => (
            PendingKind::Plan,
            "Claude has a plan ready for review.".to_string(),
            None,
            input
                .get("plan")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        _ => (
            PendingKind::Permission,
            format!("Claude wants to use {tool}."),
            None,
            None,
        ),
    };
    let detail = match kind {
        PendingKind::Permission => match (p.mcp_server.as_deref(), tool.rsplit("__").next()) {
            (Some(server), Some(name)) if tool.starts_with("mcp__") => {
                Some(format!("{server} · {name}"))
            }
            _ => Some(crate::claude::uievents::tool_summary(&tool, Some(&input)))
                .filter(|d| d != &tool),
        },
        _ => None,
    };
    let keep_input = kind != PendingKind::Permission
        || serde_json::to_vec(&input).is_ok_and(|b| b.len() <= INPUT_KEEP_MAX);
    PendingItem {
        session_id: p.session_id.clone(),
        kind,
        source: PendingSource::Hook,
        tool_name: Some(tool),
        tool_use_id,
        agent_id: agent.map(str::to_string),
        agent_type: p.agent_type.clone().filter(|t| !t.is_empty()),
        title,
        detail,
        input: keep_input.then_some(input),
        questions,
        plan,
        suggestions: pending::suggestions_from_raw(p.permission_suggestions.as_ref()),
        // The server-side classifier (pending store) raises this when needed.
        dangerous: false,
        ..Default::default()
    }
}

/// The PermissionRequest answer for a decision. A plain permission is
/// answered without `updatedInput` (the CLI keeps its own input); questions
/// carry their answers in it, plans may carry a mode switch.
pub fn hook_decision(item: &PendingItem, d: &Decision) -> Value {
    let mut decision = pending::permission_result(item, d);
    if item.kind == PendingKind::Permission {
        if let Some(o) = decision.as_object_mut() {
            o.remove("updatedInput");
        }
    }
    relay::permission_decision(decision)
}

/// Removes the held entry when the request ends, however it ends.
struct HeldGuard {
    state: AppState,
    id: String,
}

impl Drop for HeldGuard {
    fn drop(&mut self) {
        self.state.hooks.lock().unhold(&self.id);
    }
}

enum Answer {
    Decision(Decision),
    Interrupt,
}

async fn permission_request(
    state: &AppState,
    p: &HookPayload,
    agent: Option<&str>,
    key: u64,
) -> Value {
    let session = p.session_id.as_str();
    let Some(tool_name) = p.tool_name.as_deref().filter(|t| !t.is_empty()) else {
        return relay::empty();
    };
    let now = now_ms();
    let (tool_use_id, interrupted) = {
        let mut hub = state.hooks.lock();
        let t = hub.correlate(session, agent, tool_name, key, now);
        (t, agent.is_none() && hub.interrupt_pending(session))
    };
    if interrupted {
        return relay::interrupt_decision();
    }
    let Some(hold_for) = hold_policy(state, session).await else {
        return relay::empty();
    };
    let item = build_item(p, agent, tool_use_id.clone());
    let (guard, rx) = pending::raise(state, item.clone()).await;
    let id = guard.id().to_string();
    // The store's copy (the server may have adjusted it, e.g. classified it
    // dangerous) is what the answer is built from.
    let item = state.pending.get(&id).unwrap_or(item);
    let (itx, irx) = oneshot::channel::<()>();
    let fire_now = state.hooks.lock().hold(
        id.clone(),
        machine::Held {
            session: session.to_string(),
            agent_id: agent.map(str::to_string),
            tool_use_id,
            tool_name: tool_name.to_string(),
            input_key: key,
            created_ms: item.created_at,
            interrupt: Some(itx),
        },
    );
    if let Some(tx) = fire_now {
        let _ = tx.send(());
    }
    let _held = HeldGuard {
        state: state.clone(),
        id: id.clone(),
    };
    let interrupt = async {
        match irx.await {
            Ok(()) => (),
            // Dropped without firing: never resolves this branch.
            Err(_) => std::future::pending::<()>().await,
        }
    };
    let answer = tokio::select! {
        d = pending::wait(rx, hold_for) => d.map(Answer::Decision),
        () = interrupt => Some(Answer::Interrupt),
    };
    drop(guard);
    match answer {
        Some(Answer::Decision(d)) => hook_decision(&item, &d),
        Some(Answer::Interrupt) => relay::interrupt_decision(),
        None => relay::empty(),
    }
}

// ──────────────────────────────── wake ────────────────────────────────

/// Clears the session's waiter slot when the poll ends.
struct WaiterGuard {
    state: AppState,
    session: String,
    id: u64,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        self.state
            .hooks
            .lock()
            .disarm(&self.session, self.id, now_ms());
    }
}

fn gone(why: &'static str) -> Response {
    (StatusCode::GONE, why).into_response()
}

/// `POST /hooks/wait?pid=<claude pid>` with the raw Stop payload as body.
pub async fn wait(
    HookOnly(_ctx): HookOnly,
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    let p = HookPayload::parse(&body).unwrap_or_default();
    drop(body);
    let session = if p.session_id.is_empty() {
        q.get("session").cloned().unwrap_or_default()
    } else {
        p.session_id
    };
    if !is_uuid(&session) {
        return gone("invalid session");
    }
    if is_owned_any(&state, &session).await {
        return gone("owned session");
    }
    if state.plugin.disabled_cached() {
        return gone("foreign control is turned off");
    }
    let Some(pid) = q
        .get("pid")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|p| *p > 1)
    else {
        return gone("no pid");
    };
    if !pid_alive(pid) {
        return gone("process gone");
    }
    if state
        .live
        .get(&session)
        .and_then(|l| l.pid)
        .is_some_and(|live| live != pid)
    {
        return gone("not the session's process");
    }
    let arm = state.hooks.lock().arm_waiter(&session, Some(pid), now_ms());
    match arm {
        Arm::Immediate(text, msgs) => {
            record_delivered(&state, &session, &msgs);
            crate::claude::caps::refresh(&state, &session).await;
            released(text)
        }
        Arm::Parked(id, rx) => {
            let _guard = WaiterGuard {
                state: state.clone(),
                session: session.clone(),
                id,
            };
            crate::claude::caps::refresh(&state, &session).await;
            match tokio::time::timeout(state.hooks.wait_hold(), rx).await {
                Ok(Ok(WaitSignal::Release(text))) => released(text),
                Ok(Ok(WaitSignal::Superseded)) => gone("superseded"),
                Ok(Ok(WaitSignal::Gone)) | Ok(Err(_)) => gone("session gone"),
                Err(_) => StatusCode::NO_CONTENT.into_response(),
            }
        }
    }
}

fn released(text: String) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        text,
    )
        .into_response()
}

// ───────────────────────────── dispatchers ─────────────────────────────

/// Queue `text` for delivery to a hooked foreign session (next main-thread
/// PostToolUse / Stop while it works; the wake listener when it is idle).
/// Returns the message: `delivered` when it went out at once, else `queued`.
pub async fn enqueue_relay(
    state: &AppState,
    session_id: &str,
    text: &str,
) -> ApiResult<QueuedMessage> {
    enqueue_inner(state, session_id, text, None).await
}

/// [`enqueue_relay`] for a dashboard caller: a session running without
/// permission prompts only takes messages from callers with the dangerous
/// right (a relayed message there runs tools nobody approves).
pub async fn enqueue_relay_as(
    state: &AppState,
    ctx: &ClientContext,
    session_id: &str,
    text: &str,
) -> ApiResult<QueuedMessage> {
    if !ctx.may_dangerous() {
        if let Some(s) = state.find_session(session_id).await {
            if effective_bypass(state, &s).await {
                return Err(ApiError::dangerous_blocked(
                    "This session runs without permission prompts (bypassPermissions); \
                     messages to it are restricted to this computer or a device allowed \
                     dangerous actions.",
                ));
            }
        }
    }
    let from = ctx
        .device_id()
        .map(str::to_string)
        .unwrap_or_else(|| ctx.principal.label().to_string());
    enqueue_inner(state, session_id, text, Some(from)).await
}

async fn enqueue_inner(
    state: &AppState,
    session_id: &str,
    text: &str,
    from: Option<String>,
) -> ApiResult<QueuedMessage> {
    if !is_uuid(session_id) {
        return Err(ApiError::no_such_session(session_id));
    }
    let text = relay::sanitize(text)?;
    let digest = crate::server::auth::sha256_hex(&text);
    let now = now_ms();
    let res = state.hooks.lock().enqueue(session_id, text, now);
    let (msg, delivered) = res.map_err(|_| {
        ApiError::conflict(
            "queue_full",
            format!(
                "{QUEUE_CAP} messages are already waiting for this session; wait for them to \
                 be delivered or cancel some."
            ),
        )
    })?;
    // Audit: who relayed what (by hash; never the text).
    tracing::info!(
        session = %session_id,
        message = %msg.id,
        from = from.as_deref().unwrap_or("-"),
        text_sha256 = %&digest[..16],
        delivered = msg.status == QueueStatus::Delivered,
        "relay queued"
    );
    record_delivered(state, session_id, &delivered);
    crate::claude::caps::refresh(state, session_id).await;
    if msg.status == QueueStatus::Queued {
        watch_undelivered(state, session_id, &msg.id);
    }
    Ok(msg)
}

/// Tell the user when an idle session nobody listens for keeps a message.
fn watch_undelivered(state: &AppState, session_id: &str, message_id: &str) {
    let (state, session, message) = (
        state.clone(),
        session_id.to_string(),
        message_id.to_string(),
    );
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        return;
    };
    rt.spawn(async move {
        tokio::time::sleep(NO_WAITER_NOTICE_AFTER).await;
        let stuck = {
            let hub = state.hooks.lock();
            hub.still_queued(&session, &message)
                && hub
                    .session(&session)
                    .is_some_and(|s| !s.turn_active && !s.has_waiter())
        };
        if stuck {
            super::ws::publish_notice(
                &state,
                NoticeLevel::Warning,
                "Your message is waiting: this session has no wake listener yet (it has not \
                 finished a turn since foreign control loaded). It goes out at its next tool \
                 call or turn end, or continue the session here.",
                Some(&session),
            );
        }
    });
}

/// Flag the session so its next main-thread PreToolUse/PostToolUse answers
/// `{"continue":false}`; a held main-thread prompt is denied with
/// `interrupt:true` at once.
pub async fn request_interrupt(state: &AppState, session_id: &str) -> ApiResult<()> {
    if !is_uuid(session_id) {
        return Err(ApiError::no_such_session(session_id));
    }
    let senders = state.hooks.lock().interrupt(session_id, now_ms());
    for tx in senders {
        let _ = tx.send(());
    }
    tracing::info!(session = %session_id, "interrupt requested through hooks");
    Ok(())
}

/// Cancel a still-queued relay message.
pub async fn cancel_queued(state: &AppState, session_id: &str, message_id: &str) -> ApiResult<()> {
    let res = state.hooks.lock().cancel(session_id, message_id);
    match res {
        Ok(_) => {
            crate::claude::caps::refresh(state, session_id).await;
            Ok(())
        }
        Err(CancelError::AlreadyDelivered) => Err(ApiError::conflict(
            "already_delivered",
            "The message was already handed to the session.",
        )),
        Err(CancelError::NotFound) => Err(ApiError::not_found(
            "message_not_found",
            "No such queued message.",
        )),
    }
}

/// Undelivered relay messages of a session (SessionDetail.queuedMessages).
pub fn queued(state: &AppState, session_id: &str) -> Vec<QueuedMessage> {
    state
        .hooks
        .lock()
        .session(session_id)
        .map(|s| s.queued())
        .unwrap_or_default()
}

// ───────────────────────────── background ─────────────────────────────

/// Start background work: the plugin's runtime files and status, and the
/// pidfile watch (withdraw prompts answered at the desk, end turns that
/// ended without a Stop, forget dead processes). Called once from
/// `server::serve`.
pub fn start(state: &AppState) {
    crate::claude::hooks::on_start(state);
    let st = state.clone();
    tokio::spawn(async move { watch_loop(st).await });
}

async fn watch_loop(state: AppState) {
    let mut tick = tokio::time::interval(WATCH_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut n: u64 = 0;
    loop {
        tick.tick().await;
        n = n.wrapping_add(1);
        watch_once(&state).await;
        if n.is_multiple_of(200) {
            let dropped = state.hooks.lock().gc(now_ms());
            if dropped > 0 {
                tracing::debug!(dropped, "forgot idle hook sessions");
            }
        }
    }
}

/// One pass of the pidfile watch.
pub async fn watch_once(state: &AppState) {
    let watched = state.hooks.lock().watched();
    for id in watched {
        let (hook_pid, ended) = state
            .hooks
            .lock()
            .session(&id)
            .map(|s| (s.pid, s.ended))
            .unwrap_or_default();
        if ended {
            continue;
        }
        if hook_pid.is_some_and(|p| !pid_alive(p)) {
            // The process died without a SessionEnd (killed): drop its prompts
            // and listener.
            let out = state.hooks.lock().on_event(
                &machine::Event {
                    name: "SessionEnd",
                    session: &id,
                    ..Default::default()
                },
                now_ms(),
            );
            apply(state, &id, out).await;
            continue;
        }
        let Some(live) = state.live.get(&id) else {
            continue;
        };
        let (Some(status), Some(at)) = (live.status, live.status_updated_at.or(live.updated_at))
        else {
            continue;
        };
        observe_pidfile(state, &id, &status, at).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bypass_is_read_from_argv() {
        assert!(argv_is_bypass(
            "claude --output-format stream-json --permission-mode bypassPermissions --x"
        ));
        assert!(argv_is_bypass("claude --dangerously-skip-permissions"));
        assert!(argv_is_bypass("claude --permission-mode=bypassPermissions"));
        assert!(!argv_is_bypass(
            "claude --allow-dangerously-skip-permissions --permission-mode default"
        ));
        assert!(!argv_is_bypass("claude --permission-mode"));
        assert!(!argv_is_bypass(""));
    }

    #[test]
    fn pids() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
        assert!(!pid_alive(1));
        assert!(!pid_alive(u32::MAX));
    }

    fn payload(v: Value) -> HookPayload {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn items_are_typed_from_the_tool() {
        let s = "0a1b2c3d-0000-4000-8000-000000000000";
        let q = build_item(
            &payload(json!({
                "session_id": s, "tool_name": "AskUserQuestion",
                "tool_input": { "questions": [{ "question": "Which color?", "options": [
                    { "label": "Red" }, { "label": "Blue" } ], "multiSelect": false }] }
            })),
            None,
            Some("toolu_1".into()),
        );
        assert_eq!(q.kind, PendingKind::Question);
        assert_eq!(q.title, "Which color?");
        assert_eq!(q.source, PendingSource::Hook);
        assert_eq!(q.questions.as_ref().unwrap()[0].options.len(), 2);
        assert_eq!(q.tool_use_id.as_deref(), Some("toolu_1"));

        let plan = build_item(
            &payload(json!({ "session_id": s, "tool_name": "ExitPlanMode",
                "tool_input": { "plan": "1. do it" } })),
            None,
            None,
        );
        assert_eq!(plan.kind, PendingKind::Plan);
        assert_eq!(plan.plan.as_deref(), Some("1. do it"));

        let bash = build_item(
            &payload(
                json!({ "session_id": s, "tool_name": "Bash", "agent_type": "Explore",
                "tool_input": { "command": "npm test", "description": "Run tests" },
                "permission_suggestions": [{ "type": "addRules", "behavior": "allow",
                    "destination": "localSettings",
                    "rules": [{ "toolName": "Bash", "ruleContent": "npm test:*" }] }] }),
            ),
            Some("a1"),
            None,
        );
        assert_eq!(bash.kind, PendingKind::Permission);
        assert_eq!(bash.title, "Claude wants to use Bash.");
        assert_eq!(bash.detail.as_deref(), Some("Run tests"));
        assert_eq!(bash.agent_id.as_deref(), Some("a1"));
        assert_eq!(bash.agent_type.as_deref(), Some("Explore"));
        assert_eq!(bash.suggestions.len(), 1);

        let mcp = build_item(
            &payload(
                json!({ "session_id": s, "tool_name": "mcp__github__create_issue",
                "mcp_server": "github", "tool_input": {} }),
            ),
            None,
            None,
        );
        assert_eq!(mcp.detail.as_deref(), Some("github · create_issue"));
    }

    #[test]
    fn decisions_have_the_exact_hook_shape() {
        let item = PendingItem {
            kind: PendingKind::Permission,
            input: Some(json!({ "command": "ls" })),
            suggestions: pending::suggestions_from_raw(Some(&json!([{
                "type": "addRules", "behavior": "allow", "destination": "session",
                "rules": [{ "toolName": "Bash", "ruleContent": "ls" }] }]))),
            ..Default::default()
        };
        assert_eq!(
            hook_decision(&item, &Decision::allow()),
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest",
                "decision": { "behavior": "allow" } } })
        );
        let mut always = Decision::allow();
        always.suggestion = Some(0);
        assert_eq!(
            hook_decision(&item, &always),
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest",
                "decision": { "behavior": "allow", "updatedPermissions": [{
                    "type": "addRules", "behavior": "allow", "destination": "session",
                    "rules": [{ "toolName": "Bash", "ruleContent": "ls" }] }] } } })
        );
        let mut deny = Decision::deny(Some("use echo instead".into()));
        deny.interrupt = true;
        assert_eq!(
            hook_decision(&item, &deny),
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest",
                "decision": { "behavior": "deny", "message": "use echo instead",
                              "interrupt": true } } })
        );
        let q = PendingItem {
            kind: PendingKind::Question,
            input: Some(json!({ "questions": [{ "question": "Which color?" }] })),
            ..Default::default()
        };
        let mut answer = Decision::allow();
        answer.answers = Some([("Which color?".to_string(), "Blue".to_string())].into());
        assert_eq!(
            hook_decision(&q, &answer),
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest",
                "decision": { "behavior": "allow", "updatedInput": {
                    "questions": [{ "question": "Which color?" }],
                    "answers": { "Which color?": "Blue" } } } } })
        );
        let plan = PendingItem {
            kind: PendingKind::Plan,
            input: Some(json!({ "plan": "p" })),
            ..Default::default()
        };
        let mut approve = Decision::allow();
        approve.mode = Some("acceptEdits".into());
        assert_eq!(
            hook_decision(&plan, &approve),
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest",
                "decision": { "behavior": "allow", "updatedInput": { "plan": "p" },
                    "updatedPermissions": [{ "type": "setMode", "mode": "acceptEdits",
                                             "destination": "session" }] } } })
        );
    }
}
