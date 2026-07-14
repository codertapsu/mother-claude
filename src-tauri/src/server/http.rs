//! REST handlers. All dashboard data is served here (never via Tauri `invoke`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::claude::{self, PendingInput, PendingKind, QuestionOption, SpawnOptions};
use crate::server::auth;
use crate::state::{AppState, Resolution};

/// How long a blocked permission/question request waits before defaulting to deny.
const PENDING_TIMEOUT: Duration = Duration::from_secs(600);

/// Liveness + version probe.
pub async fn health() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "app": "mother-claude",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Served when the Angular build is not present on disk.
pub async fn no_frontend() -> impl IntoResponse {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [("content-type", "text/html; charset=utf-8")],
        "<h1>Mother Claude</h1><p>Frontend not built. Run <code>npm run build</code> \
         or set <code>MOTHER_CLAUDE_WEB_DIR</code>. The API is available under <code>/api</code>.</p>",
    )
}

/// Current session snapshot.
pub async fn list_sessions(State(state): State<AppState>) -> impl IntoResponse {
    let sessions = state.sessions.read().await.clone();
    Json(sessions)
}

/// One session by id.
pub async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.find_session(&id).await {
        Some(s) => Json(s).into_response(),
        None => (StatusCode::NOT_FOUND, "no such session").into_response(),
    }
}

/// Full transcript for a session (history). `?limit=N` keeps the last N events.
pub async fn get_transcript(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(session) = state.find_session(&id).await else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    if session.cwd.is_empty() {
        return Json(Vec::<claude::TranscriptEvent>::new()).into_response();
    }
    let path = state.home.transcript_path(&session.cwd, &id);
    let limit = params.get("limit").and_then(|s| s.parse::<usize>().ok());

    let events = tokio::task::spawn_blocking(move || claude::read_transcript(&path))
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default();

    let events = match limit {
        Some(n) if events.len() > n => events[events.len() - n..].to_vec(),
        _ => events,
    };
    Json(events).into_response()
}

/// Per-session Git overview (branch, diff stats, recent log, worktrees).
pub async fn get_diff(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let Some(session) = state.find_session(&id).await else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    if session.cwd.is_empty() {
        return Json(claude::GitOverview::default()).into_response();
    }
    let cwd = std::path::PathBuf::from(session.cwd);
    let overview = tokio::task::spawn_blocking(move || claude::git_overview(&cwd, 25))
        .await
        .unwrap_or_default();
    Json(overview).into_response()
}

/// Unified diff for a single file. `?path=<relative path>` required.
pub async fn get_file_patch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(session) = state.find_session(&id).await else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let Some(rel) = params.get("path").cloned() else {
        return (StatusCode::BAD_REQUEST, "missing ?path=").into_response();
    };
    let cwd = std::path::PathBuf::from(session.cwd);
    let patch = tokio::task::spawn_blocking(move || claude::file_patch(&cwd, &rel, 256_000))
        .await
        .ok()
        .flatten();
    Json(json!({ "patch": patch })).into_response()
}

/// MCP servers, daemon health, and background jobs.
pub async fn get_services(State(state): State<AppState>) -> impl IntoResponse {
    let mcp = read_user_mcp_servers();
    let daemon = daemon_status().await;
    let sessions = state.sessions.read().await.clone();
    let bg: Vec<Value> = sessions
        .iter()
        .filter(|s| s.kind.as_deref() == Some("background"))
        .map(|s| json!({ "id": s.id, "cwd": s.cwd, "state": s.state }))
        .collect();
    Json(json!({
        "mcpServers": mcp,
        "daemon": daemon,
        "backgroundJobs": bg,
    }))
}

/// Raw `claude daemon status` output.
pub async fn get_daemon() -> impl IntoResponse {
    Json(daemon_status().await)
}

/// Launch defaults + available choices for the spawn/continue forms. The
/// "default" for each knob is whatever the user last selected in VS Code or
/// the CLI (persisted in ~/.claude/settings.json) — passing no override keeps
/// exactly those settings.
pub async fn get_defaults(State(state): State<AppState>) -> impl IntoResponse {
    let d = claude::read_launch_defaults(&state.home);
    Json(json!({
        "model": d.model,
        "effort": d.effort,
        "models": d.models,
        "efforts": EFFORT_LEVELS,
    }))
}

/// Validate optional launch knobs: unknown efforts/thinking values are
/// rejected rather than silently passed to the CLI.
fn validate_launch(effort: &Option<String>, thinking: &Option<String>) -> Result<(), String> {
    if let Some(e) = effort {
        if !EFFORT_LEVELS.contains(&e.as_str()) {
            return Err(format!(
                "unknown effort level {e:?} (expected one of {EFFORT_LEVELS:?})"
            ));
        }
    }
    if let Some(t) = thinking {
        if t != "on" && t != "off" {
            return Err(format!(
                "unknown thinking value {t:?} (expected \"on\" or \"off\")"
            ));
        }
    }
    Ok(())
}

const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

#[derive(Deserialize)]
pub struct SpawnBody {
    pub cwd: String,
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
}

/// Spawn a new **owned** session (full two-way control).
pub async fn post_spawn(
    State(state): State<AppState>,
    Json(body): Json<SpawnBody>,
) -> impl IntoResponse {
    if let Err(e) = validate_launch(&body.effort, &body.thinking) {
        return (StatusCode::UNPROCESSABLE_ENTITY, e).into_response();
    }
    let opts = SpawnOptions {
        cwd: body.cwd,
        prompt: body.prompt,
        model: body.model,
        effort: body.effort,
        thinking: body.thinking,
        permission_mode: None,
        resume: None,
    };
    match state.control.spawn(&state, opts).await {
        Ok(id) => (StatusCode::CREATED, Json(json!({ "id": id }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ContinueBody {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
}

/// Continue a session's conversation **in place** (same id, same transcript)
/// as an **owned** session that can be driven from anywhere — the supported
/// way to take over a foreign (e.g. VS Code) session from your phone.
pub async fn post_continue(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ContinueBody>,
) -> impl IntoResponse {
    let Some(cwd) = state.find_session(&id).await.map(|s| s.cwd) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    if let Err(e) = validate_launch(&body.effort, &body.thinking) {
        return (StatusCode::UNPROCESSABLE_ENTITY, e).into_response();
    }
    let opts = SpawnOptions {
        cwd,
        prompt: body.prompt.unwrap_or_default(),
        model: body.model,
        effort: body.effort,
        thinking: body.thinking,
        permission_mode: None,
        resume: Some(id),
    };
    match state.control.spawn(&state, opts).await {
        Ok(sid) => (StatusCode::CREATED, Json(json!({ "id": sid }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct MessageBody {
    pub text: String,
}

/// Send an instruction to a session. Owned sessions are driven over stdin;
/// running foreign sessions are driven via PTY injection when it is enabled.
pub async fn post_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<MessageBody>,
) -> impl IntoResponse {
    if state.is_owned(&id).await {
        return match state.control.send_message(&id, &body.text).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        };
    }
    if claude::foreign_injection_enabled() {
        return match foreign_inject(&state, &id, &body.text).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
        };
    }
    (
        StatusCode::FORBIDDEN,
        "foreign session injection is disabled; lifecycle only",
    )
        .into_response()
}

/// Inject text (plus Enter) into a foreign session by PTY-driving
/// `claude attach`. Attaches on first use; subsequent calls reuse the PTY.
/// Only known, running, non-owned sessions are eligible — never attach to an
/// unknown id (it would spawn a stray `claude attach`).
#[cfg(feature = "experimental")]
async fn foreign_inject(state: &AppState, id: &str, text: &str) -> Result<(), String> {
    let session = state
        .find_session(id)
        .await
        .ok_or_else(|| format!("unknown session {id}"))?;
    if session.owned {
        return Err("owned session is driven over stdin, not PTY".into());
    }
    if !session.running {
        return Err(format!("session {id} is not running; cannot attach"));
    }
    state
        .pty
        .attach(state, id, &session.cwd)
        .map_err(|e| e.to_string())?;
    state.pty.inject(id, text).map_err(|e| e.to_string())
}

#[cfg(not(feature = "experimental"))]
async fn foreign_inject(_state: &AppState, _id: &str, _text: &str) -> Result<(), String> {
    Err("foreign-session injection is not compiled into this build".into())
}

#[derive(Deserialize)]
pub struct PermissionRequestBody {
    #[serde(default, alias = "requestId")]
    pub request_id: Option<String>,
    /// "permission" or "question".
    pub kind: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    /// Very short topic chip for questions (AskUserQuestion `header`).
    #[serde(default)]
    pub header: Option<String>,
    /// Strings or `{label, description}` objects (QuestionOption is tolerant).
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default, alias = "multiSelect")]
    pub multi_select: bool,
    /// Salient tool input (the Bash command, the file path, …), display-ready.
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub dangerous: bool,
}

/// Cleanup for a blocked prompt that also runs when the request future is
/// CANCELLED (sidecar crash / connection drop / stop): removes the resolver
/// and clears the session's pending card — but only if the card still belongs
/// to this request, so a newer prompt that replaced it survives.
struct PromptGuard {
    state: AppState,
    id: String,
    request_id: String,
}

impl Drop for PromptGuard {
    fn drop(&mut self) {
        let _ = self.state.take_resolver(&self.request_id);
        let (state, id, request_id) =
            (self.state.clone(), self.id.clone(), self.request_id.clone());
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { state.clear_pending_if(&id, &request_id).await });
        }
    }
}

/// Sidecar-facing: raise a permission/question prompt and **block** until the
/// dashboard resolves it (or it times out → deny). This is the canUseTool /
/// ask_user bridge for owned (Path A) sessions.
pub async fn post_permission_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PermissionRequestBody>,
) -> impl IntoResponse {
    let request_id = body
        .request_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let kind = if body.kind == "question" {
        PendingKind::Question
    } else {
        PendingKind::Permission
    };
    let pending = PendingInput {
        kind,
        tool: body.tool,
        prompt: body.prompt,
        header: body.header,
        options: body.options,
        multi_select: body.multi_select,
        detail: body.detail,
        request_id: Some(request_id.clone()),
        answerable: true,
        dangerous: body.dangerous,
    };

    let (tx, rx) = oneshot::channel();
    state.register_resolver(request_id.clone(), tx, pending.dangerous);
    state.set_pending(&id, Some(pending)).await;
    let _guard = PromptGuard {
        state: state.clone(),
        id: id.clone(),
        request_id: request_id.clone(),
    };

    let resolution = match tokio::time::timeout(PENDING_TIMEOUT, rx).await {
        Ok(Ok(r)) => r,
        _ => Resolution::Deny,
    };
    // _guard's Drop removes the resolver + conditionally clears the card
    // (here on the normal path, and equally if this future is cancelled).

    let payload = match resolution {
        Resolution::Allow => json!({ "behavior": "allow" }),
        Resolution::Deny => json!({ "behavior": "deny" }),
        Resolution::Answer(answer) => json!({ "behavior": "answer", "answer": answer }),
    };
    Json(payload).into_response()
}

#[derive(Deserialize)]
pub struct PermissionBody {
    /// "allow" or "deny".
    pub decision: String,
    #[serde(default, alias = "requestId")]
    pub request_id: Option<String>,
}

/// Dashboard: approve or deny a pending permission. Dangerous approvals are
/// gated to the local desktop unless remote-dangerous is explicitly enabled.
pub async fn post_permission(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    Json(body): Json<PermissionBody>,
) -> impl IntoResponse {
    let pending = state.pending.read().await.get(&id).cloned();
    let request_id = body
        .request_id
        .or_else(|| pending.as_ref().and_then(|p| p.request_id.clone()));
    // Dangerousness must describe the request actually being resolved — prefer
    // the blocked resolver's own flag over whatever occupies the session slot.
    let dangerous = request_id
        .as_deref()
        .and_then(|r| state.resolver_dangerous(r))
        .or_else(|| pending.as_ref().map(|p| p.dangerous))
        .unwrap_or(false);
    let allow = body.decision == "allow";

    if allow
        && auth::dangerous_blocked(
            dangerous,
            auth::is_loopback(&peer),
            state.auth.allow_remote_dangerous,
        )
    {
        return (
            StatusCode::FORBIDDEN,
            "dangerous approvals are restricted to the local desktop",
        )
            .into_response();
    }

    // If a request is blocked on a resolver (owned/sidecar), answer it.
    let resolution = if allow {
        Resolution::Allow
    } else {
        Resolution::Deny
    };
    if try_resolve(&state, request_id.as_deref(), resolution) {
        return StatusCode::NO_CONTENT.into_response();
    }

    // Otherwise it's a foreign TUI prompt — best-effort select the option.
    // Selection layouts vary, so this is a hint. The plan-approval prompt has
    // THREE options (1 = yes + auto-accept, 2 = yes + manual approve, 3 = no),
    // so map it specially — and never remotely pick auto-accept.
    if claude::foreign_injection_enabled() {
        let is_plan = state
            .pending
            .read()
            .await
            .get(&id)
            .map(|p| p.tool.as_deref() == Some("ExitPlanMode"))
            .unwrap_or(false);
        let key = match (is_plan, allow) {
            (true, true) => "2",
            (true, false) => "3",
            (false, true) => "1",
            (false, false) => "2",
        };
        return match foreign_inject(&state, &id, key).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
        };
    }
    (StatusCode::NOT_FOUND, "no pending request to resolve").into_response()
}

#[derive(Deserialize)]
pub struct AnswerBody {
    pub answer: String,
    #[serde(default, alias = "requestId")]
    pub request_id: Option<String>,
}

/// Dashboard: answer a pending question — resolve a blocked owned/sidecar
/// request if one exists, otherwise inject into the foreign session's TUI.
pub async fn post_answer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AnswerBody>,
) -> impl IntoResponse {
    let request_id = match body.request_id.clone() {
        Some(r) => Some(r),
        None => state
            .pending
            .read()
            .await
            .get(&id)
            .and_then(|p| p.request_id.clone()),
    };
    if try_resolve(
        &state,
        request_id.as_deref(),
        Resolution::Answer(body.answer.clone()),
    ) {
        return StatusCode::NO_CONTENT.into_response();
    }
    if claude::foreign_injection_enabled() {
        return match foreign_inject(&state, &id, &body.answer).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
        };
    }
    (StatusCode::NOT_FOUND, "no pending question to answer").into_response()
}

/// Send a resolution to a blocked request if one is registered for `request_id`.
/// Returns true if it resolved one.
fn try_resolve(state: &AppState, request_id: Option<&str>, resolution: Resolution) -> bool {
    let Some(req) = request_id else {
        return false;
    };
    match state.take_resolver(req) {
        Some(resolver) => {
            let _ = resolver.tx.send(resolution);
            true
        }
        None => false,
    }
}

/// Stop a session. Owned sessions are our own subprocesses (killed directly);
/// foreign sessions go through `claude stop` (background jobs only).
pub async fn post_stop(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    if state.is_owned(&id).await {
        let _ = state.control.kill(&id).await;
        // A killed session can't consume an answer — drop its stale prompt.
        state.set_pending(&id, None).await;
        state.broadcast(crate::state::ServerEvent::Notice(format!("stopped {id}")));
        return (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
    }
    run_lifecycle(&state, "stop", &id).await
}

/// Respawn (restart) a foreign background session. Owned sessions have no
/// persisted prompt to restart, so this is not supported for them.
pub async fn post_respawn(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if state.is_owned(&id).await {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "respawn isn't supported for app-owned sessions — stop it and start a new one",
        )
            .into_response();
    }
    run_lifecycle(&state, "respawn", &id).await
}

/// Remove a session and its worktree. Irreversible → gated to the local desktop
/// unless remote-dangerous is explicitly enabled.
pub async fn post_rm(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if auth::dangerous_blocked(
        true,
        auth::is_loopback(&peer),
        state.auth.allow_remote_dangerous,
    ) {
        return (
            StatusCode::FORBIDDEN,
            "removing a session is irreversible and restricted to the local desktop",
        )
            .into_response();
    }
    if state.is_owned(&id).await {
        let _ = state.control.kill(&id).await;
        state.owned.write().await.remove(&id);
        state.set_pending(&id, None).await;
        // Best-effort daemon cleanup; ignore "no job" for our own subprocesses.
        let _ = claude::control::run_lifecycle("rm", &id).await;
        state.broadcast(crate::state::ServerEvent::Notice(format!("removed {id}")));
        return (StatusCode::OK, Json(json!({ "ok": true }))).into_response();
    }
    state.set_pending(&id, None).await;
    run_lifecycle(&state, "rm", &id).await
}

/// Run a `claude` lifecycle subcommand and translate the result. A non-zero CLI
/// exit (e.g. "No job matching …" for a non-background session) is a 422 with the
/// CLI's message — not a 500 — so the dashboard can show *why* it failed.
async fn run_lifecycle(state: &AppState, action: &str, id: &str) -> axum::response::Response {
    match claude::control::run_lifecycle(action, id).await {
        Ok(output) => {
            state.broadcast(crate::state::ServerEvent::Notice(format!(
                "{action} {id}: ok"
            )));
            (
                StatusCode::OK,
                Json(json!({ "ok": true, "output": output })),
            )
                .into_response()
        }
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
    }
}

/// EXPERIMENTAL: attach to a foreign session via PTY (Stage 3, feature-gated).
/// Local-desktop only — this uses unsanctioned internals.
#[cfg(feature = "experimental")]
pub async fn post_pty_attach(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if auth::dangerous_blocked(
        true,
        auth::is_loopback(&peer),
        state.auth.allow_remote_dangerous,
    ) {
        return (
            StatusCode::FORBIDDEN,
            "experimental PTY is local-desktop only",
        )
            .into_response();
    }
    let cwd = state
        .find_session(&id)
        .await
        .map(|s| s.cwd)
        .unwrap_or_default();
    match state.pty.attach(&state, &id, &cwd) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// EXPERIMENTAL: inject keystrokes into a PTY-attached foreign session.
#[cfg(feature = "experimental")]
pub async fn post_pty_inject(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    Json(body): Json<MessageBody>,
) -> impl IntoResponse {
    if auth::dangerous_blocked(
        true,
        auth::is_loopback(&peer),
        state.auth.allow_remote_dangerous,
    ) {
        return (
            StatusCode::FORBIDDEN,
            "experimental PTY is local-desktop only",
        )
            .into_response();
    }
    match state.pty.inject(&id, &body.text) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

/// Hook ingestion: foreign (and owned) sessions POST tool/notification/stop
/// events here. We fan them out on the bus as a `hook` event. We do NOT block or
/// auto-approve here (the documented `allow`-suppression bug makes that
/// unreliable); hooks are for *events* and explicit *deny* gating only.
pub async fn post_hook_event(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    tracing::debug!(?payload, "hook event");
    state.broadcast(crate::state::ServerEvent::Hook(payload));
    // Empty object = "no decision", let Claude proceed as normal.
    Json(json!({}))
}

/// Install Mother Claude's hook block into the user's `~/.claude/settings.json`
/// so *foreign* sessions across all projects emit events to us. Writes a backup
/// first and embeds the literal token (the user's own machine; see SECURITY.md).
/// Restricted to the local desktop unless remote-dangerous is enabled.
pub async fn post_install_hooks(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    if auth::dangerous_blocked(
        true,
        auth::is_loopback(&peer),
        state.auth.allow_remote_dangerous,
    ) {
        return (
            StatusCode::FORBIDDEN,
            "installing hooks edits your settings and is restricted to the local desktop",
        )
            .into_response();
    }

    let path = state.home.user_settings();
    let url = format!("http://127.0.0.1:{}/hooks/event", state.config.port);
    let entry = json!([{
        "hooks": [{
            "type": "http",
            "url": url,
            "headers": { "Authorization": format!("Bearer {}", state.auth.token) }
        }]
    }]);

    let mut root: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    if !root.is_object() {
        root = json!({});
    }

    // Back up before mutating the user's settings.
    if path.exists() {
        let _ = std::fs::copy(&path, path.with_extension("json.mc-backup"));
    }

    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if let Some(hooks_obj) = hooks.as_object_mut() {
        for event in ["PreToolUse", "PostToolUse", "Notification", "Stop"] {
            hooks_obj.insert(event.to_string(), entry.clone());
        }
    }

    match std::fs::write(
        &path,
        serde_json::to_string_pretty(&root).unwrap_or_default(),
    ) {
        Ok(()) => {
            Json(json!({ "installed": true, "path": path.to_string_lossy() })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not write {}: {e}", path.display()),
        )
            .into_response(),
    }
}

/// Device pairing payload: QR (SVG), URL, token, and TLS fingerprint.
pub async fn get_pairing(State(state): State<AppState>) -> impl IntoResponse {
    let fingerprint = state
        .fingerprint
        .read()
        .await
        .clone()
        .unwrap_or_else(|| "n/a (http)".to_string());
    Json(auth::build_pairing(&state, &fingerprint))
}

async fn daemon_status() -> Value {
    let out = tokio::process::Command::new(claude::claude_bin())
        .arg("daemon")
        .arg("status")
        .output()
        .await;
    match out {
        Ok(o) => json!({
            "reachable": o.status.success(),
            "raw": String::from_utf8_lossy(&o.stdout).trim().to_string(),
            "stderr": String::from_utf8_lossy(&o.stderr).trim().to_string(),
        }),
        Err(e) => json!({ "reachable": false, "error": e.to_string() }),
    }
}

/// Parse `<home>/.claude.json` for configured MCP servers (tolerant).
fn read_user_mcp_servers() -> Value {
    let Some(home) = crate::claude::user_home_dir() else {
        return json!({});
    };
    let path = home.join(".claude.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return json!({});
    };
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        return json!({});
    };
    root.get("mcpServers").cloned().unwrap_or_else(|| json!({}))
}
