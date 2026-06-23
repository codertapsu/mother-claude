//! REST handlers. All dashboard data is served here (never via Tauri `invoke`).

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::claude;
use crate::state::AppState;

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

/// Parse `$HOME/.claude.json` for configured MCP servers (tolerant).
fn read_user_mcp_servers() -> Value {
    let Some(home) = std::env::var_os("HOME") else {
        return json!({});
    };
    let path = std::path::PathBuf::from(home).join(".claude.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return json!({});
    };
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        return json!({});
    };
    root.get("mcpServers").cloned().unwrap_or_else(|| json!({}))
}
