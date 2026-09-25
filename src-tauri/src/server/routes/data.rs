//! Read-only dashboard data (design §4.5 "Data").
//!
//! Owner: **B2 (data layer)**. The foundation implements what the existing
//! registry supports (sessions, detail, naive transcript pages, diff,
//! file-patch, defaults, inbox, services, daemon, projects, static command
//! list) and answers `501 not_implemented` for the rest (media, task output,
//! file search, per-agent transcripts, branch diffs).

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::claude::model::{CommandInfo, PendingItem, ProjectInfo, SessionSummary};
use crate::claude::{self, uievents};
use crate::server::auth::Authed;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Liveness + version probe (public).
pub async fn health() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "app": "mother-claude",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListQuery {
    /// `active` (default) | `all`.
    pub scope: Option<String>,
    pub limit: Option<usize>,
    /// Cursor: `lastActivityAt` (epoch ms) of the last row of the previous page.
    pub before: Option<i64>,
}

/// `GET /api/sessions?scope=active|all&limit=&before=` →
/// `{sessions, nextBefore?}`.
pub async fn list_sessions(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let now = claude::registry::now_ms();
    let all = q.scope.as_deref() == Some("all");
    let sessions: Vec<SessionSummary> = state
        .sessions()
        .await
        .into_iter()
        .filter(|s| all || s.in_active_scope(now))
        .filter(|s| q.before.is_none_or(|b| s.last_activity_at.unwrap_or(0) < b))
        .collect();
    let limit = q.limit.unwrap_or(if all { 50 } else { usize::MAX }).max(1);
    let next_before =
        (sessions.len() > limit).then(|| sessions[limit - 1].last_activity_at.unwrap_or(0));
    let page: Vec<SessionSummary> = sessions.into_iter().take(limit).collect();
    let mut body = json!({ "sessions": page });
    if let Some(nb) = next_before {
        body["nextBefore"] = json!(nb);
    }
    Json(body)
}

async fn session_or_404(state: &AppState, id: &str) -> ApiResult<SessionSummary> {
    state
        .find_session(id)
        .await
        .ok_or_else(|| ApiError::no_such_session(id))
}

/// `GET /api/sessions/{id}` → `SessionDetail`.
pub async fn get_session(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let summary = session_or_404(&state, &id).await?;
    Ok(Json(claude::summaries::detail(&state, summary).await).into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptQuery {
    pub before: Option<String>,
    pub limit: Option<usize>,
    pub agent: Option<String>,
}

/// `GET /api/sessions/{id}/transcript?before=&limit=&agent=` →
/// `{events, before?, tail}`.
pub async fn transcript(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TranscriptQuery>,
) -> ApiResult<Response> {
    if q.agent.is_some() {
        return Err(ApiError::not_implemented("Per-agent transcripts (B2)"));
    }
    session_or_404(&state, &id).await?;
    let Some(path) = state.transcript_path(&id).await else {
        return Ok(Json(crate::claude::model::TranscriptPage {
            tail: "0".into(),
            ..Default::default()
        })
        .into_response());
    };
    let before = match q.before.as_deref() {
        Some(b) => Some(uievents::parse_cursor(b).ok_or_else(|| {
            ApiError::bad_request("`before` must be a cursor from a previous page.")
        })?),
        None => None,
    };
    let limit = q.limit.unwrap_or(150);
    let sid = id.clone();
    let page = tokio::task::spawn_blocking(move || uievents::read_page(&sid, &path, before, limit))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::internal(format!("Could not read the transcript: {e}")))?;
    Ok(Json(page).into_response())
}

/// `GET /api/sessions/{id}/media/{uuid}/{idx}` — STUB (B2).
pub async fn media(
    Authed(_ctx): Authed,
    Path((_id, _uuid, _idx)): Path<(String, String, String)>,
) -> ApiError {
    ApiError::not_implemented("Transcript images (B2)")
}

/// `GET /api/sessions/{id}/agents` → `{agents, workflows}`.
pub async fn session_agents(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    session_or_404(&state, &id).await?;
    let (agents, workflows) = state.agents.for_session(&id);
    Ok(Json(json!({ "agents": agents, "workflows": workflows })).into_response())
}

/// `GET /api/agents` → `{sessions: FleetSession[]}`.
pub async fn fleet(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "sessions": state.agents.fleet() }))
}

/// `GET /api/sessions/{id}/tasks/{taskId}/output` — STUB (B2).
pub async fn task_output(
    Authed(_ctx): Authed,
    Path((_id, _task)): Path<(String, String)>,
) -> ApiError {
    ApiError::not_implemented("Background task output (B2)")
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DiffQuery {
    /// `working` (default) | `branch`.
    pub base: Option<String>,
}

/// `GET /api/sessions/{id}/diff?base=working|branch` → `GitOverview`.
pub async fn diff(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> ApiResult<Response> {
    let session = session_or_404(&state, &id).await?;
    if q.base.as_deref() == Some("branch") {
        return Err(ApiError::not_implemented("Branch-vs-default diffs (B2)"));
    }
    if session.cwd.is_empty() {
        return Ok(Json(claude::GitOverview::default()).into_response());
    }
    let cwd = std::path::PathBuf::from(session.cwd);
    let overview = tokio::task::spawn_blocking(move || claude::git_overview(&cwd, 25))
        .await
        .unwrap_or_default();
    Ok(Json(overview).into_response())
}

/// `GET /api/sessions/{id}/file-patch?path=` → `{patch}`.
pub async fn file_patch(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Response> {
    let session = session_or_404(&state, &id).await?;
    let rel = params
        .get("path")
        .cloned()
        .ok_or_else(|| ApiError::bad_request("Missing ?path=."))?;
    let cwd = std::path::PathBuf::from(session.cwd);
    let patch = tokio::task::spawn_blocking(move || claude::file_patch(&cwd, &rel, 256_000))
        .await
        .ok()
        .flatten();
    Ok(Json(json!({ "patch": patch })).into_response())
}

/// `GET /api/sessions/{id}/files?q=` — STUB (B2).
pub async fn files(Authed(_ctx): Authed, Path(_id): Path<String>) -> ApiError {
    ApiError::not_implemented("File search (B2)")
}

/// Commands offered when the session cannot tell us its own list.
pub fn common_commands() -> Vec<CommandInfo> {
    [
        (
            "compact",
            "Summarize the conversation to free context",
            Some("[instructions]"),
        ),
        ("clear", "Start a fresh conversation", None),
        ("context", "Show context usage", None),
        ("cost", "Show token usage and cost", None),
        ("model", "Switch the model", Some("[model]")),
        (
            "effort",
            "Set reasoning effort",
            Some("[low|medium|high|xhigh|max]"),
        ),
        ("review", "Review the current changes", None),
        ("init", "Create a CLAUDE.md for this project", None),
        ("memory", "Edit memory files", None),
        ("todos", "Show the todo list", None),
    ]
    .into_iter()
    .map(|(name, description, hint)| CommandInfo {
        name: format!("/{name}"),
        description: description.to_string(),
        argument_hint: hint.map(str::to_string),
    })
    .collect()
}

/// `GET /api/sessions/{id}/commands` → `{commands}` (owned: from the CLI's
/// `initialize`, B3; otherwise a static common list).
pub async fn commands(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    session_or_404(&state, &id).await?;
    let commands = claude::owned::commands(&state, &id)
        .await
        .unwrap_or_else(common_commands);
    Ok(Json(json!({ "commands": commands })).into_response())
}

/// `GET /api/projects` → `{projects}` for the spawn picker. Foundation: the
/// distinct working directories of known sessions, most recent first.
pub async fn projects(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    let mut by_path: BTreeMap<String, ProjectInfo> = BTreeMap::new();
    for s in state.sessions().await {
        if s.cwd.is_empty() {
            continue;
        }
        let entry = by_path.entry(s.cwd.clone()).or_insert_with(|| ProjectInfo {
            path: s.cwd.clone(),
            name: s.project_name.clone(),
            last_activity_at: None,
        });
        entry.last_activity_at = entry.last_activity_at.max(s.last_activity_at);
    }
    let mut projects: Vec<ProjectInfo> = by_path.into_values().collect();
    projects.sort_by_key(|p| std::cmp::Reverse(p.last_activity_at));
    Json(json!({ "projects": projects }))
}

const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Launch defaults + available choices for the spawn/continue forms.
pub async fn defaults(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    let d = claude::read_launch_defaults(&state.home);
    Json(json!({
        "model": d.model,
        "effort": d.effort,
        "models": d.models,
        "efforts": EFFORT_LEVELS,
    }))
}

/// `GET /api/inbox` → `{items}`: every pending item across sessions — the
/// store's answerable ones plus observed ones from the snapshot.
pub async fn inbox(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    let mut items: Vec<PendingItem> = state.pending.all();
    for s in state.sessions().await {
        items.extend(s.pending.into_iter().filter(|p| !p.answerable));
    }
    items.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Json(json!({ "items": items }))
}

/// MCP servers (secrets redacted), daemon health, and background jobs.
pub async fn services(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    let mcp = redact_mcp(read_user_mcp_servers());
    let daemon = daemon_status().await;
    let bg: Vec<Value> = state
        .sessions()
        .await
        .iter()
        .filter(|s| s.kind.as_deref() == Some("background"))
        .map(|s| json!({ "id": s.id, "cwd": s.cwd, "state": s.status }))
        .collect();
    Json(json!({
        "mcpServers": mcp,
        "daemon": daemon,
        "backgroundJobs": bg,
    }))
}

/// Raw `claude daemon status` output.
pub async fn daemon(Authed(_ctx): Authed) -> impl IntoResponse {
    Json(daemon_status().await)
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
    let Ok(text) = std::fs::read_to_string(home.join(".claude.json")) else {
        return json!({});
    };
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        return json!({});
    };
    root.get("mcpServers").cloned().unwrap_or_else(|| json!({}))
}

/// Never serve secret values: `env` and `headers` keep their keys, every value
/// becomes `"…"`.
pub fn redact_mcp(mut servers: Value) -> Value {
    if let Some(map) = servers.as_object_mut() {
        for server in map.values_mut() {
            for field in ["env", "headers"] {
                if let Some(obj) = server.get_mut(field).and_then(Value::as_object_mut) {
                    for v in obj.values_mut() {
                        *v = json!("…");
                    }
                }
            }
        }
    }
    servers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_secrets_are_redacted() {
        let v = redact_mcp(json!({
            "a": { "command": "x", "env": { "TOKEN": "secret" }, "headers": { "Authorization": "Bearer s" } },
            "b": { "url": "https://x" }
        }));
        assert_eq!(v["a"]["env"]["TOKEN"], "…");
        assert_eq!(v["a"]["headers"]["Authorization"], "…");
        assert_eq!(v["a"]["command"], "x");
        assert_eq!(v["b"]["url"], "https://x");
        assert!(!v.to_string().contains("secret"));
    }

    #[test]
    fn common_commands_are_slash_prefixed() {
        assert!(common_commands().iter().all(|c| c.name.starts_with('/')));
    }
}
