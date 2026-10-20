//! Supervisor for the Node bridge host (`sidecar/dist/bridge-host.js`).
//!
//! One long-lived child process owns every Agent SDK conversation the bridge
//! serves; this module owns the child. Commands go down its stdin as NDJSON and
//! frames come back up its stdout, where a single reader task fans them out to
//! the operation logs, the pending-request table, and the dashboard's broadcast
//! bus.
//!
//! The host is started lazily on first use and respawned after a crash, so the
//! desktop app pays nothing for the bridge until something actually calls it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use super::error::{BridgeError, BridgeResult};
use super::ops::OperationStatus;
use crate::claude::model::{PendingItem, PendingKind, PendingSource, Question, QuestionOption};
use crate::pending::Decision;
use crate::state::AppState;

/// Default deadline for one host RPC.
pub const RPC_TIMEOUT: Duration = Duration::from_secs(120);
/// Creating a conversation spawns a CLI subprocess; the SDK's own load timeout
/// is 60s, so allow a little more before giving up on it.
pub const CREATE_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a tool approval or question waits for a human before it is denied.
/// Matches the dashboard's own `PENDING_TIMEOUT`.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// A tool approval or question waiting on a human.
#[derive(Debug)]
pub struct PendingRequest {
    pub id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub kind: String,
    /// The whole frame from the host, minus its envelope fields.
    pub params: Value,
    pub created_at: i64,
    /// The server's own classification ([`crate::pending::classify_dangerous`])
    /// OR-ed with the host's flag — never the host's heuristic alone.
    pub dangerous: bool,
    /// The dashboard item this request was raised as (classified), so the
    /// bridge's `/requests/{id}/respond` checks answers against the same input
    /// and suggestions the dashboard does.
    pub item: PendingItem,
    /// Taken by whichever surface answers first.
    responder: Option<oneshot::Sender<Value>>,
}

impl PendingRequest {
    pub fn snapshot(&self) -> Value {
        json!({
            "request_id": self.id,
            "thread_id": self.session_id,
            "turn_id": self.turn_id,
            "kind": self.kind,
            "params": self.params,
            "status": "pending",
            "dangerous": self.dangerous,
            "created_at": self.created_at,
        })
    }
}

/// An in-flight RPC: which host generation it was written to, and the channel
/// waiting for its reply.
type InFlight = (u64, oneshot::Sender<BridgeResult<Value>>);

struct Running {
    stdin: AsyncMutex<ChildStdin>,
    child: AsyncMutex<Child>,
    alive: AtomicBool,
    /// Which host process this is. In-flight RPCs are tagged with it so a dead
    /// generation can be cleaned up without touching its replacement's work.
    generation: u64,
}

/// The lazily-started host process and its in-flight RPCs.
#[derive(Default)]
pub struct RuntimeHost {
    running: AsyncMutex<Option<Arc<Running>>>,
    inflight: Mutex<HashMap<String, InFlight>>,
    seq: AtomicU64,
    generations: AtomicU64,
    /// Requests blocked on a human, keyed by request id.
    pub requests: Mutex<HashMap<String, PendingRequest>>,
}

impl std::fmt::Debug for RuntimeHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeHost").finish_non_exhaustive()
    }
}

/// Locate the built host entry point, mirroring how `control.rs` finds the
/// per-session sidecar (dev checkout, then the packaged resource dir).
pub fn host_entry() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("MOTHER_CLAUDE_BRIDGE_HOST_PATH") {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(dir) = std::env::var("MOTHER_CLAUDE_SIDECAR_PATH") {
        if let Some(parent) = PathBuf::from(dir).parent() {
            let p = parent.join("bridge-host.js");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let candidates = [
        std::env::current_dir()
            .ok()
            .map(|d| d.join("sidecar/dist/bridge-host.js")),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sidecar/dist/bridge-host.js")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

impl RuntimeHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a host process is currently up.
    pub async fn is_running(&self) -> bool {
        matches!(&*self.running.lock().await, Some(r) if r.alive.load(Ordering::SeqCst))
    }

    /// Start the host if it is not already up. Idempotent and racy-safe: the
    /// `running` mutex is the start lock.
    async fn ensure(&self, state: &AppState) -> BridgeResult<Arc<Running>> {
        let mut guard = self.running.lock().await;
        if let Some(running) = guard.as_ref() {
            if running.alive.load(Ordering::SeqCst) {
                return Ok(running.clone());
            }
            // Retire the corpse while still holding the lock, so a replacement
            // is never installed before its predecessor's work has been failed.
            // Doing it the other way round let a dead host's reader task, still
            // draining its pipe, wipe the new host's in-flight RPCs and running
            // turns.
            if let Some(dead) = guard.take() {
                self.retire_locked(&dead, state);
            }
        }

        let entry = host_entry().ok_or_else(|| {
            BridgeError::runtime_unavailable(
                "The bridge host is not built. Run `npm run sidecar:build` \
                 (or set MOTHER_CLAUDE_BRIDGE_HOST_PATH).",
            )
        })?;

        let mut command = tokio::process::Command::new("node");
        command
            .arg(&entry)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The host talks to us over stdio only: no Mother Claude credential
        // reaches it (or the `claude` children its SDK spawns, which inherit
        // its environment). Scrub the parent's Claude Code session variables
        // and mark the children owned so the foreign-control hook scripts
        // stay out of bridge conversations.
        crate::claude::control::apply_child_env(
            &mut command,
            &crate::claude::accounts::AccountEnv::Inherit,
        );

        // CLAUDE_CONFIG_DIR is deliberately NOT set here. Exporting it breaks
        // the CLI's credential resolution *even when it points at the default
        // `~/.claude`*: every turn then answers "Not logged in · Please run
        // /login" (verified against 2.1.186). The child inherits the variable
        // when the user set one themselves, which is the only case where an
        // override is wanted — so the bridge reads the same home the adapter
        // does without ever naming it.

        let mut child = command.spawn().map_err(|e| {
            BridgeError::runtime_unavailable(format!(
                "Could not start the bridge host ({}): {e}",
                entry.display()
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            BridgeError::internal("The bridge host was started without a stdin pipe.")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            BridgeError::internal("The bridge host was started without a stdout pipe.")
        })?;
        let stderr = child.stderr.take();

        let running = Arc::new(Running {
            stdin: AsyncMutex::new(stdin),
            child: AsyncMutex::new(child),
            alive: AtomicBool::new(true),
            generation: self.generations.fetch_add(1, Ordering::Relaxed),
        });

        // Reader: the one place host frames are interpreted.
        {
            let st = state.clone();
            let running = running.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Value>(&line) {
                        Ok(frame) => handle_frame(&st, frame).await,
                        Err(e) => {
                            tracing::warn!(error = %e, "bridge host emitted a non-JSON line")
                        }
                    }
                }
                running.alive.store(false, Ordering::SeqCst);
                st.bridge.host.retire(&running, &st).await;
                tracing::warn!("bridge host stdout closed; runtime marked down");
            });
        }

        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("bridge host stderr: {line}");
                }
            });
        }

        // Reap so a dead host is noticed even if stdout lingers.
        {
            let running_reap = running.clone();
            tokio::spawn(async move {
                let status = running_reap.child.lock().await.wait().await;
                running_reap.alive.store(false, Ordering::SeqCst);
                tracing::info!(?status, "bridge host exited");
            });
        }

        *guard = Some(running.clone());
        Ok(running)
    }

    /// Clean up after a host process, but only if it is still the current one.
    ///
    /// The reader task and `ensure()` both call this and either may get there
    /// first; identity is the tiebreak, so a late cleanup from a replaced
    /// generation is a no-op rather than collateral damage.
    async fn retire(&self, dead: &Arc<Running>, state: &AppState) {
        let mut guard = self.running.lock().await;
        let is_current = guard.as_ref().is_some_and(|r| Arc::ptr_eq(r, dead));
        if !is_current {
            // Already replaced; still release this generation's own RPCs.
            drop(guard);
            self.abandon_generation(dead.generation);
            return;
        }
        guard.take();
        self.retire_locked(dead, state);
    }

    /// The cleanup itself. Call with the `running` lock held (or after
    /// establishing that this generation is no longer installed).
    fn retire_locked(&self, dead: &Arc<Running>, state: &AppState) {
        self.abandon_generation(dead.generation);
        // Conversations do not survive their host, so every operation still
        // running against it is dead too.
        state
            .bridge
            .ops
            .fail_all("runtime_unavailable", "The bridge host exited.");
    }

    /// Fail the in-flight RPCs belonging to one host generation.
    fn abandon_generation(&self, generation: u64) {
        let mut map = match self.inflight.lock() {
            Ok(m) => m,
            Err(p) => p.into_inner(),
        };
        let ids: Vec<String> = map
            .iter()
            .filter(|(_, (gen, _))| *gen == generation)
            .map(|(id, _)| id.clone())
            .collect();
        let senders: Vec<_> = ids.iter().filter_map(|id| map.remove(id)).collect();
        drop(map);
        for (_, tx) in senders {
            let _ = tx.send(Err(BridgeError::runtime_unavailable(
                "The bridge host exited while this request was in flight.",
            )));
        }
    }

    /// Send one command and await its reply.
    pub async fn call(
        &self,
        state: &AppState,
        op: &str,
        mut params: Value,
        timeout: Duration,
    ) -> BridgeResult<Value> {
        let running = self.ensure(state).await?;
        let id = format!("r{}", self.seq.fetch_add(1, Ordering::Relaxed));

        let (tx, rx) = oneshot::channel();
        match self.inflight.lock() {
            Ok(mut m) => m.insert(id.clone(), (running.generation, tx)),
            Err(p) => p.into_inner().insert(id.clone(), (running.generation, tx)),
        };

        if let Some(obj) = params.as_object_mut() {
            obj.insert("id".into(), json!(id));
            obj.insert("op".into(), json!(op));
        }
        let mut line = params.to_string();
        line.push('\n');

        {
            let mut stdin = running.stdin.lock().await;
            if let Err(e) = stdin.write_all(line.as_bytes()).await {
                self.drop_inflight(&id);
                running.alive.store(false, Ordering::SeqCst);
                return Err(BridgeError::runtime_unavailable(format!(
                    "Could not write to the bridge host: {e}"
                )));
            }
            if let Err(e) = stdin.flush().await {
                self.drop_inflight(&id);
                return Err(BridgeError::runtime_unavailable(format!(
                    "Could not flush the bridge host: {e}"
                )));
            }
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.drop_inflight(&id);
                Err(BridgeError::runtime_unavailable(
                    "The bridge host dropped this request.",
                ))
            }
            Err(_) => {
                self.drop_inflight(&id);
                Err(BridgeError::timeout(format!(
                    "The bridge host did not answer `{op}` within {}s.",
                    timeout.as_secs()
                )))
            }
        }
    }

    fn drop_inflight(&self, id: &str) {
        match self.inflight.lock() {
            Ok(mut m) => m.remove(id),
            Err(p) => p.into_inner().remove(id),
        };
    }

    fn resolve_inflight(&self, id: &str, result: BridgeResult<Value>) {
        let entry = match self.inflight.lock() {
            Ok(mut m) => m.remove(id),
            Err(p) => p.into_inner().remove(id),
        };
        if let Some((_, tx)) = entry {
            let _ = tx.send(result);
        }
    }

    /// Send a command **without** starting the host if it is down.
    ///
    /// Used for fire-and-forget follow-ups such as delivering a tool-approval
    /// answer: if the runtime has gone away there is nothing left to answer,
    /// and spawning a fresh 216 MB runtime to tell it so is pure waste.
    pub async fn call_if_running(
        &self,
        state: &AppState,
        op: &str,
        params: Value,
        timeout: Duration,
    ) -> BridgeResult<Value> {
        if !self.is_running().await {
            return Err(BridgeError::runtime_unavailable(
                "The bridge host is not running.",
            ));
        }
        self.call(state, op, params, timeout).await
    }

    pub fn pending_requests(&self) -> Vec<Value> {
        let map = match self.requests.lock() {
            Ok(m) => m,
            Err(p) => p.into_inner(),
        };
        let mut out: Vec<Value> = map.values().map(PendingRequest::snapshot).collect();
        out.sort_by_key(|v| v["created_at"].as_i64().unwrap_or(0));
        out
    }

    pub fn pending_request(&self, id: &str) -> Option<Value> {
        let map = match self.requests.lock() {
            Ok(m) => m,
            Err(p) => p.into_inner(),
        };
        map.get(id).map(PendingRequest::snapshot)
    }

    /// The classified dashboard item behind a pending request.
    pub fn pending_request_item(&self, id: &str) -> Option<PendingItem> {
        let map = match self.requests.lock() {
            Ok(m) => m,
            Err(p) => p.into_inner(),
        };
        map.get(id).map(|r| r.item.clone())
    }

    /// Answer a pending request from the bridge's own `/requests` surface.
    /// Returns an error if it is unknown or already answered.
    pub fn respond(&self, id: &str, result: Value) -> BridgeResult<()> {
        let responder = {
            let mut map = match self.requests.lock() {
                Ok(m) => m,
                Err(p) => p.into_inner(),
            };
            let entry = map
                .get_mut(id)
                .ok_or_else(|| BridgeError::request_not_found(id))?;
            entry.responder.take().ok_or_else(|| {
                BridgeError::conflict("already_answered", "That request was already answered.")
                    .with("request_id", id)
            })?
        };
        responder.send(result).map_err(|_| {
            BridgeError::conflict("already_answered", "That request is no longer waiting.")
        })
    }

    fn take_request(&self, id: &str) -> Option<PendingRequest> {
        match self.requests.lock() {
            Ok(mut m) => m.remove(id),
            Err(p) => p.into_inner().remove(id),
        }
    }
}

/// Dispatch one frame from the host.
async fn handle_frame(state: &AppState, frame: Value) {
    match frame.get("t").and_then(Value::as_str).unwrap_or_default() {
        "ready" => tracing::info!(pid = ?frame.get("pid"), "bridge host ready"),

        "reply" => {
            let Some(id) = frame.get("id").and_then(Value::as_str) else {
                return;
            };
            let result = if frame.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                Ok(frame.get("result").cloned().unwrap_or(Value::Null))
            } else {
                Err(host_error(frame.get("error")))
            };
            state.bridge.host.resolve_inflight(id, result);
        }

        "event" => {
            let session_id = frame.get("sessionId").and_then(Value::as_str);
            let turn_id = frame.get("turnId").and_then(Value::as_str);
            let message = frame.get("message").cloned().unwrap_or(Value::Null);
            // system/init and system/status carry the live permission mode.
            if let (Some(id), Some(mode)) = (
                session_id,
                (message.get("type").and_then(Value::as_str) == Some("system"))
                    .then(|| message.get("permissionMode").and_then(Value::as_str))
                    .flatten(),
            ) {
                super::note_thread_mode(state, id, mode).await;
            }
            let event = json!({
                "method": "message",
                "params": {
                    "thread_id": session_id,
                    "turn_id": turn_id,
                    "message": message,
                }
            });
            match (session_id, turn_id) {
                (Some(s), Some(t)) => match state.bridge.ops.for_turn(s, t) {
                    Some(op) => op.emit(&state.bridge.ops.global, &event),
                    None => {
                        state.bridge.ops.global.append(&event);
                    }
                },
                _ => {
                    state.bridge.ops.global.append(&event);
                }
            }
        }

        "turn" => {
            let (Some(session_id), Some(turn_id)) = (
                frame.get("sessionId").and_then(Value::as_str),
                frame.get("turnId").and_then(Value::as_str),
            ) else {
                return;
            };
            let Some(op) = state.bridge.ops.for_turn(session_id, turn_id) else {
                return;
            };
            let status = match frame.get("status").and_then(Value::as_str) {
                Some("completed") => OperationStatus::Completed,
                Some("interrupted") => OperationStatus::Interrupted,
                _ => OperationStatus::Failed,
            };
            let error = frame.get("error").filter(|v| !v.is_null()).cloned();
            let result = frame.get("result").filter(|v| !v.is_null()).cloned();
            op.finish(&state.bridge.ops.global, status, result, error);
        }

        "request" => raise_request(state, frame).await,

        "session" => {
            if let Some(id) = frame.get("sessionId").and_then(Value::as_str) {
                state.bridge.forget_thread(id).await;
                // Stop advertising it as an owned, injectable session — and
                // keep `owned` from growing for the life of the process.
                state.unmark_owned(id).await;
                state.bridge.ops.global.append(&json!({
                    "method": "thread/closed",
                    "params": { "thread_id": id, "reason": frame.get("reason") }
                }));
            }
        }

        "log" => {
            let message = frame
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match frame.get("level").and_then(Value::as_str) {
                Some("error") => tracing::error!("bridge host: {message}"),
                Some("warn") => tracing::warn!("bridge host: {message}"),
                _ => tracing::debug!("bridge host: {message}"),
            }
        }

        other => tracing::debug!("bridge host sent an unknown frame `{other}`"),
    }
}

fn host_error(value: Option<&Value>) -> BridgeError {
    let code = value
        .and_then(|v| v.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("claude_error");
    let message = value
        .and_then(|v| v.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("The bridge host reported a failure.")
        .to_string();
    match code {
        "thread_not_found" => BridgeError::not_found("thread_not_found", message),
        "request_not_found" => BridgeError::not_found("request_not_found", message),
        "unknown_method" => BridgeError::not_found("unknown_method", message),
        "invalid_request" => BridgeError::invalid_request(message),
        "invalid_state" => BridgeError::conflict("invalid_state", message),
        "not_found" => BridgeError::not_found("not_found", message),
        "interrupted" => BridgeError::conflict("interrupted", message),
        "busy" => BridgeError::busy(message),
        "claude_login_required" => BridgeError::login_required(message),
        _ => BridgeError::claude_error(message),
    }
}

/// A tool approval or question arrived. Register it on **both** answer
/// surfaces — the bridge's `/requests` endpoints and the dashboard's existing
/// pending-prompt card — and forward whichever answer lands first.
///
/// The future is boxed deliberately. Answering a request calls back into
/// [`RuntimeHost::call`], which can start the host, whose reader task lands
/// here — a type cycle rustc refuses to infer. Erasing this one signature cuts
/// it without changing any behaviour.
fn raise_request(
    state: &AppState,
    frame: Value,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    let state = state.clone();
    Box::pin(raise_request_inner(state, frame))
}

async fn raise_request_inner(state: AppState, frame: Value) {
    let state = &state;
    let Some(request_id) = frame.get("requestId").and_then(Value::as_str) else {
        return;
    };
    let request_id = request_id.to_string();
    let session_id = frame
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let turn_id = frame
        .get("turnId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let kind = frame
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("permission")
        .to_string();
    // The host's own flag is a three-substring heuristic; it can only raise
    // the server's classification, never replace it.
    let host_dangerous = frame
        .get("dangerous")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // Strip the envelope; what remains is the request's own payload.
    let mut params = frame.clone();
    if let Some(obj) = params.as_object_mut() {
        for key in ["t", "requestId", "sessionId", "turnId", "kind"] {
            obj.remove(key);
        }
    }

    // Classify BEFORE the request becomes answerable on either surface, with
    // the same rules (and the conversation's cwd) as every other pending item.
    let mut item = pending_item(&request_id, &session_id, &kind, &params, host_dangerous);
    let cwd = match state.bridge.thread(&session_id).await {
        Some(t) if !t.cwd.is_empty() => Some(t.cwd),
        _ => state.control.cwd_of(&session_id),
    };
    classify_item(&mut item, cwd.as_deref());
    let dangerous = item.dangerous;

    let (bridge_tx, bridge_rx) = oneshot::channel::<Value>();

    let record = PendingRequest {
        id: request_id.clone(),
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        kind: kind.clone(),
        params: params.clone(),
        created_at: crate::claude::registry::now_ms(),
        dangerous,
        item: item.clone(),
        responder: Some(bridge_tx),
    };
    let snapshot = record.snapshot();
    match state.bridge.host.requests.lock() {
        Ok(mut m) => m.insert(request_id.clone(), record),
        Err(p) => p.into_inner().insert(request_id.clone(), record),
    };

    // Dashboard surface: the shared pending store, so an approval card appears
    // in the app and on the phone exactly as it does for any other session.
    // The guard withdraws the card whichever surface answers.
    let (guard, dash_rx) = crate::pending::raise(state, item.clone()).await;

    // Bridge surface: an SSE frame so a headless client can see it too.
    let event = json!({
        "method": "bridge/request",
        "params": snapshot,
    });
    match (session_id.as_str(), turn_id.as_deref()) {
        (s, Some(t)) => match state.bridge.ops.for_turn(s, t) {
            Some(op) => op.emit(&state.bridge.ops.global, &event),
            None => {
                state.bridge.ops.global.append(&event);
            }
        },
        _ => {
            state.bridge.ops.global.append(&event);
        }
    }

    let st = state.clone();
    tokio::spawn(async move {
        let (result, timed_out) = tokio::select! {
            v = bridge_rx => (v.ok(), false),
            d = dash_rx => (d.ok().map(|d| decision_to_result(&kind, &item, &d)), false),
            _ = tokio::time::sleep(REQUEST_TIMEOUT) => (None, true),
        };
        let result = result.unwrap_or_else(
            || json!({ "behavior": "deny", "message": "Timed out waiting for a human." }),
        );

        st.bridge.host.take_request(&request_id);
        // An approval that switches the conversation's mode (a plan approved
        // into bypassPermissions, an update chosen on /requests) changes what
        // the gates must assume before the host even applies it.
        if let Some(mode) = super::mode_set_by(&result) {
            super::note_thread_mode(&st, &session_id, &mode).await;
        }
        if timed_out {
            guard.expire();
        } else {
            drop(guard);
        }

        let payload = json!({ "requestId": request_id, "result": result });
        if let Err(e) = st
            .bridge
            .host
            .call_if_running(&st, "request.respond", payload, RPC_TIMEOUT)
            .await
        {
            tracing::warn!(error = %e, "could not deliver a request response to the bridge host");
        }
        st.notice(format!("bridge request {request_id} answered"));
    });
}

/// Apply the server's danger classification to a bridge item (it can only
/// raise the flag the host sent).
fn classify_item(item: &mut PendingItem, cwd: Option<&str>) {
    if let Some(why) = crate::pending::classify_dangerous(item, cwd) {
        item.dangerous = true;
        item.dangerous_reason.get_or_insert(why);
    }
}

/// Map a dashboard decision onto the host's expected result shape: questions
/// (the `ask_user` tool) take `{answer}`; permissions take the shared
/// permission result.
fn decision_to_result(kind: &str, item: &PendingItem, decision: &Decision) -> Value {
    if kind == "question" {
        return if decision.is_allow() {
            json!({ "answer": decision.answer_text().unwrap_or_default() })
        } else {
            json!({ "behavior": "deny" })
        };
    }
    crate::pending::permission_result(item, decision)
}

/// Build the dashboard's pending item from a host request payload.
fn pending_item(
    request_id: &str,
    session_id: &str,
    kind: &str,
    params: &Value,
    dangerous: bool,
) -> PendingItem {
    let text = |key: &str| params.get(key).and_then(Value::as_str).map(str::to_string);
    let tool = text("tool");
    let input = params.get("input").cloned().filter(|v| !v.is_null());
    let is_plan = tool.as_deref() == Some("ExitPlanMode");
    let pending_kind = match kind {
        "question" => PendingKind::Question,
        "elicitation" => PendingKind::Elicitation,
        _ if is_plan => PendingKind::Plan,
        _ => PendingKind::Permission,
    };
    let questions = (pending_kind == PendingKind::Question).then(|| {
        vec![Question {
            question: text("prompt").unwrap_or_default(),
            header: text("header"),
            options: params
                .get("options")
                .and_then(Value::as_array)
                .map(|items| {
                    items
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
                        .collect()
                })
                .unwrap_or_default(),
            multi_select: params
                .get("multiSelect")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }]
    });
    let plan = if is_plan {
        input
            .as_ref()
            .and_then(|i| i.get("plan"))
            .and_then(Value::as_str)
            .map(str::to_string)
    } else {
        None
    };
    PendingItem {
        id: request_id.to_string(),
        session_id: session_id.to_string(),
        kind: pending_kind,
        source: PendingSource::Bridge,
        tool_name: tool.clone(),
        tool_use_id: text("toolUseId"),
        title: text("prompt").unwrap_or_else(|| match &tool {
            Some(t) => format!("Claude wants to use {t}."),
            None => "Claude is waiting for you.".into(),
        }),
        detail: text("detail"),
        input,
        questions,
        plan,
        suggestions: crate::pending::suggestions_from_raw(params.get("suggestions")),
        dangerous,
        answerable: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_errors_map_onto_bridge_statuses() {
        let err = host_error(Some(
            &json!({ "code": "thread_not_found", "message": "gone" }),
        ));
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(err.code, "thread_not_found");

        let err = host_error(Some(&json!({ "code": "boom", "message": "kaboom" })));
        assert_eq!(err.status, axum::http::StatusCode::BAD_GATEWAY);
        assert_eq!(err.code, "claude_error");

        // A frame with no error object still produces something serviceable.
        assert_eq!(host_error(None).code, "claude_error");
    }

    #[test]
    fn decisions_map_to_host_results() {
        let item = pending_item(
            "r",
            "s",
            "permission",
            &json!({ "tool": "Bash", "input": { "command": "ls" } }),
            false,
        );
        let allow = decision_to_result("permission", &item, &Decision::allow());
        assert_eq!(allow["behavior"], "allow");
        assert_eq!(allow["updatedInput"]["command"], "ls");
        let deny = decision_to_result(
            "permission",
            &item,
            &Decision::deny(Some("no thanks".into())),
        );
        assert_eq!(deny["behavior"], "deny");
        assert_eq!(deny["message"], "no thanks");

        let q = pending_item(
            "r2",
            "s",
            "question",
            &json!({ "prompt": "Colour?" }),
            false,
        );
        let mut d = Decision::allow();
        d.answers = Some(
            [("Colour?".to_string(), "blue".to_string())]
                .into_iter()
                .collect(),
        );
        assert_eq!(decision_to_result("question", &q, &d)["answer"], "blue");
        assert_eq!(
            decision_to_result("question", &q, &Decision::deny(None))["behavior"],
            "deny"
        );
    }

    #[test]
    fn pending_items_carry_options_kind_and_suggestions() {
        let params = json!({
            "prompt": "Which database?",
            "header": "Storage",
            "options": ["postgres", { "label": "sqlite", "description": "local file" }],
            "multiSelect": true,
        });
        let card = pending_item("req-1", "t1", "question", &params, false);
        assert_eq!(card.kind, PendingKind::Question);
        assert_eq!(card.source, PendingSource::Bridge);
        let q = &card.questions.as_ref().unwrap()[0];
        assert_eq!(q.question, "Which database?");
        assert_eq!(q.header.as_deref(), Some("Storage"));
        assert_eq!(q.options.len(), 2);
        assert!(q.multi_select);
        assert_eq!(card.id, "req-1");
        assert_eq!(card.session_id, "t1");

        let permission = pending_item(
            "req-2",
            "t1",
            "permission",
            &json!({ "tool": "Bash", "toolUseId": "toolu_1",
                     "suggestions": [{ "type": "addRules", "destination": "userSettings",
                                        "rules": [{ "toolName": "Bash" }] }] }),
            true,
        );
        assert_eq!(permission.kind, PendingKind::Permission);
        assert!(permission.dangerous);
        assert_eq!(permission.tool_name.as_deref(), Some("Bash"));
        assert_eq!(permission.tool_use_id.as_deref(), Some("toolu_1"));
        assert!(permission.suggestions[0].standing);

        let plan = pending_item(
            "req-3",
            "t1",
            "permission",
            &json!({ "tool": "ExitPlanMode", "input": { "plan": "1. do it" } }),
            false,
        );
        assert_eq!(plan.kind, PendingKind::Plan);
        assert_eq!(plan.plan.as_deref(), Some("1. do it"));
    }

    #[test]
    fn the_server_classifies_bridge_requests_whatever_the_host_says() {
        // The host's heuristic only knows three substrings; a Write to a shell
        // startup file or a git hook is dangerous on the server's own rules.
        for input in [
            json!({ "file_path": "~/.zshrc", "content": "x" }),
            json!({ "file_path": ".git/hooks/pre-commit", "content": "x" }),
        ] {
            let params = json!({ "tool": "Write", "input": input });
            let mut item = pending_item("r", "t", "permission", &params, false);
            assert!(!item.dangerous);
            classify_item(&mut item, Some("/tmp/some-repo"));
            assert!(item.dangerous, "{params}");
            assert!(item.dangerous_reason.is_some());
        }
        // An ordinary write stays ordinary, and the host flag still counts.
        let params = json!({ "tool": "Write", "input": { "file_path": "notes.md" } });
        let mut item = pending_item("r", "t", "permission", &params, false);
        classify_item(&mut item, Some("/tmp/some-repo"));
        assert!(!item.dangerous);
        let mut item = pending_item("r", "t", "permission", &params, true);
        classify_item(&mut item, Some("/tmp/some-repo"));
        assert!(item.dangerous);
    }

    #[test]
    fn request_snapshot_uses_the_codex_field_names() {
        let (tx, _rx) = oneshot::channel();
        let record = PendingRequest {
            id: "r1".into(),
            session_id: "t1".into(),
            turn_id: Some("u1".into()),
            kind: "permission".into(),
            params: json!({ "tool": "Bash" }),
            created_at: 42,
            dangerous: false,
            item: PendingItem::default(),
            responder: Some(tx),
        };
        let snap = record.snapshot();
        assert_eq!(snap["request_id"], "r1");
        assert_eq!(snap["thread_id"], "t1");
        assert_eq!(snap["turn_id"], "u1");
        assert_eq!(snap["status"], "pending");
        assert_eq!(snap["params"]["tool"], "Bash");
    }

    #[tokio::test]
    async fn responding_twice_is_a_conflict() {
        let host = RuntimeHost::new();
        let (tx, rx) = oneshot::channel();
        host.requests.lock().unwrap().insert(
            "r1".into(),
            PendingRequest {
                id: "r1".into(),
                session_id: "t".into(),
                turn_id: None,
                kind: "permission".into(),
                params: Value::Null,
                created_at: 0,
                dangerous: false,
                item: PendingItem::default(),
                responder: Some(tx),
            },
        );

        host.respond("r1", json!({ "behavior": "allow" })).unwrap();
        assert_eq!(rx.await.unwrap()["behavior"], "allow");

        let err = host
            .respond("r1", json!({ "behavior": "deny" }))
            .unwrap_err();
        assert_eq!(err.code, "already_answered");
        assert_eq!(err.status, axum::http::StatusCode::CONFLICT);

        let err = host.respond("nope", Value::Null).unwrap_err();
        assert_eq!(err.code, "request_not_found");
    }

    #[tokio::test]
    async fn pending_requests_are_listed_oldest_first() {
        let host = RuntimeHost::new();
        for (id, created) in [("b", 200), ("a", 100)] {
            let (tx, _rx) = oneshot::channel();
            host.requests.lock().unwrap().insert(
                id.into(),
                PendingRequest {
                    id: id.into(),
                    session_id: "t".into(),
                    turn_id: None,
                    kind: "permission".into(),
                    params: Value::Null,
                    created_at: created,
                    dangerous: false,
                    item: PendingItem::default(),
                    responder: Some(tx),
                },
            );
        }
        let listed = host.pending_requests();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["request_id"], "a");
        assert_eq!(listed[1]["request_id"], "b");
        assert!(host.pending_request("a").is_some());
        assert!(host.pending_request("zzz").is_none());
    }
}
