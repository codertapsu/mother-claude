//! Read-only dashboard data (design §4.5 "Data", amended §10 B2).
//!
//! Every id from a URL, query or frame is validated with `claude::ids` before
//! anything touches the disk, and files are located through the index (by
//! session id), never built from request input. Binary and plain-text
//! answers (media, task output) carry `X-Content-Type-Options: nosniff`,
//! `Content-Security-Policy: default-src 'none'; sandbox` and
//! `Cache-Control: private, no-store`.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::claude::model::{CommandInfo, PendingItem, ProjectInfo, SessionSummary, TranscriptPage};
use crate::claude::{self, ids, uievents};
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
    /// Cursor: the previous page's `nextBefore` — the last row's
    /// `lastActivityAt` (epoch ms) and id, `<ms>:<id>` (a bare `<ms>`, from an
    /// older client, still works, but skips rows that share that time).
    pub before: Option<String>,
    /// The list's search box: sessions whose title, name, project, folder,
    /// branch, last prompt, model, effort or id contain it, in any case
    /// ([`SessionSummary::matches_search`]).
    pub q: Option<String>,
    /// Only this surface (`vscode`, `terminal`, …).
    pub surface: Option<String>,
    /// Only this project (its name, as the list shows it).
    pub project: Option<String>,
    /// Only this account's sessions.
    pub account: Option<String>,
    /// `live`: only running sessions; `needs`: only those waiting on someone.
    pub only: Option<String>,
    /// `ids`: every match's id, unpaged (`{ids}`): what *Select all* takes.
    pub view: Option<String>,
}

/// A query value that says something (blank form fields say nothing).
fn given(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// Whether `s` passes the list's filters (all but the scope and the cursor).
/// `only_default`: the Default is the only account, so a session without an
/// account chip is its (with several, no chip means a directory nobody
/// registered, as the client's account filter reads it).
fn passes(s: &SessionSummary, q: &ListQuery, search: Option<&str>, only_default: bool) -> bool {
    if let Some(surface) = given(&q.surface) {
        if serde_json::to_value(s.surface)
            .ok()
            .as_ref()
            .and_then(Value::as_str)
            != Some(surface)
        {
            return false;
        }
    }
    if given(&q.project).is_some_and(|p| s.project_name != p) {
        return false;
    }
    if let Some(a) = given(&q.account) {
        let id = match &s.account {
            Some(r) => Some(r.id.as_str()),
            None => only_default.then_some(crate::claude::accounts::DEFAULT_ID),
        };
        if id != Some(a) {
            return false;
        }
    }
    let only = match given(&q.only) {
        Some("live") => s.live,
        Some("needs") => s.needs_you(),
        _ => true,
    };
    only && search.is_none_or(|t| s.matches_search(t))
}

/// The cursor of a page that ends with `s`: its time and its id, so rows
/// that share that time are never skipped across the page boundary.
fn cursor_of(s: &SessionSummary) -> String {
    format!("{}:{}", s.last_activity_at.unwrap_or(0), s.id)
}

/// A `before` cursor: the time, and the id when it carries one. `None` when
/// it is not a cursor this route made.
fn parse_cursor(before: &str) -> Option<(i64, Option<&str>)> {
    match before.split_once(':') {
        Some((t, id)) => Some((t.parse().ok()?, Some(id))),
        None => Some((before.parse().ok()?, None)),
    }
}

/// Whether `s` comes after the cursor in the list's order (newest first,
/// ties by id, as the snapshot keeps it).
fn after_cursor(s: &SessionSummary, (at, id): (i64, Option<&str>)) -> bool {
    let t = s.last_activity_at.unwrap_or(0);
    match id {
        Some(id) => t < at || (t == at && s.id.as_str() > id),
        None => t < at,
    }
}

/// `GET /api/sessions?scope=active|all&limit=&before=` →
/// `{sessions, nextBefore?}`, narrowed by `q`, `surface`, `project`,
/// `account` and `only` when given; `view=ids` → `{ids}`, every match
/// unpaged. Carries an `ETag`; a matching `If-None-Match` (the phone's
/// polling fallback) gets `304 Not Modified`.
pub async fn list_sessions(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(q): Query<ListQuery>,
) -> Response {
    let now = claude::registry::now_ms();
    let all = q.scope.as_deref() == Some("all");
    let search = given(&q.q).map(str::to_lowercase);
    let only_default = state.accounts.accounts().len() <= 1;
    let cursor = match given(&q.before).map(parse_cursor) {
        Some(None) => {
            return ApiError::bad_request("`before` must be a page's `nextBefore`.").into_response()
        }
        Some(Some(c)) => Some(c),
        None => None,
    };
    let sessions: Vec<SessionSummary> = state
        .sessions()
        .await
        .into_iter()
        .filter(|s| all || s.in_active_scope(now))
        .filter(|s| cursor.is_none_or(|c| after_cursor(s, c)))
        .filter(|s| passes(s, &q, search.as_deref(), only_default))
        .collect();
    let body = if given(&q.view) == Some("ids") {
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        json!({ "ids": ids })
    } else {
        let limit = q.limit.unwrap_or(if all { 50 } else { usize::MAX }).max(1);
        let next_before = (sessions.len() > limit).then(|| cursor_of(&sessions[limit - 1]));
        let page: Vec<SessionSummary> = sessions.into_iter().take(limit).collect();
        let mut body = json!({ "sessions": page });
        if let Some(nb) = next_before {
            body["nextBefore"] = json!(nb);
        }
        body
    };
    let text = body.to_string();
    let etag = format!("\"{}\"", &crate::server::auth::sha256_hex(&text)[..32]);
    let fresh = headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().trim_start_matches("W/"))
        .any(|t| t == etag || t == "*");
    let etag_value =
        HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("\"\""));
    if fresh {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag_value)]).into_response();
    }
    (
        [
            (header::ETAG, etag_value),
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ],
        text,
    )
        .into_response()
}

async fn session_or_404(state: &AppState, id: &str) -> ApiResult<SessionSummary> {
    ids::require_session_id(id)?;
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
    /// Polling fallback: events after this cursor (bounded; `reset` when too
    /// far behind).
    pub after: Option<String>,
    pub limit: Option<usize>,
    pub agent: Option<String>,
}

fn cursor(v: Option<&str>, name: &str) -> ApiResult<Option<u64>> {
    match v {
        Some(b) => uievents::parse_cursor(b).map(Some).ok_or_else(|| {
            ApiError::bad_request(format!("`{name}` must be a cursor from a previous page."))
        }),
        None => Ok(None),
    }
}

/// The file behind a transcript request: the session's main transcript or,
/// with `agent`, that subagent's file (validated id, located by discovery).
async fn transcript_file(
    state: &AppState,
    id: &str,
    agent: Option<&str>,
) -> ApiResult<Option<PathBuf>> {
    match agent {
        Some(a) => {
            if !ids::is_agent_id(a) {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_id",
                    "Not a valid agent id.",
                ));
            }
            Ok(Some(state.index.subagent_path(id, a).ok_or_else(|| {
                ApiError::not_found("agent_not_found", format!("No agent {a} in session {id}."))
            })?))
        }
        None => Ok(state.transcript_path(id).await),
    }
}

/// `GET /api/sessions/{id}/transcript?before=&after=&limit=&agent=` →
/// `{events, before?, tail, reset?}`.
pub async fn transcript(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TranscriptQuery>,
) -> ApiResult<Response> {
    ids::require_session_id(&id)?;
    if state.find_session(&id).await.is_none() && state.index.session_files(&id).is_none() {
        return Err(ApiError::no_such_session(&id));
    }
    let agent = q.agent.clone().filter(|a| !a.is_empty());
    let Some(path) = transcript_file(&state, &id, agent.as_deref()).await? else {
        return Ok(Json(TranscriptPage {
            tail: "0".into(),
            ..Default::default()
        })
        .into_response());
    };
    let before = cursor(q.before.as_deref(), "before")?;
    let after = cursor(q.after.as_deref(), "after")?;
    let limit = q.limit.unwrap_or(150);
    let sid = id.clone();
    let agent_for_read = agent.clone();
    let page = tokio::task::spawn_blocking(move || -> std::io::Result<TranscriptPage> {
        match after {
            Some(from) => {
                let r = uievents::read_range(&sid, &path, from, None, agent_for_read.as_deref())?;
                Ok(TranscriptPage {
                    events: r.events,
                    before: None,
                    tail: r.tail.to_string(),
                    reset: r.reset.then_some(true),
                })
            }
            None => uievents::read_page_opts(
                &sid,
                &path,
                uievents::PageOpts {
                    before,
                    limit,
                    agent: agent_for_read.as_deref(),
                },
            ),
        }
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(format!("Could not read the transcript: {e}")))?;
    let mut page = page;
    if agent.is_none() && after.is_none() {
        state.timeline.merge_into(&id, &mut page, before.is_none());
    }
    Ok(Json(page).into_response())
}

/// Headers for anything served from transcripts or task files: never
/// sniffed, never rendered as a document, never cached. B3/B4 use it for
/// terminal text too.
pub fn locked_down(mut resp: Response, content_type: &'static str) -> Response {
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    resp
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AgentQuery {
    pub agent: Option<String>,
}

/// `GET /api/sessions/{id}/media/{offset}/{uuid}/{block}?agent=` — one
/// transcript image: PNG/JPEG/GIF/WebP only (by magic bytes), else 415.
pub async fn media(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path((id, offset, uuid, block)): Path<(String, String, String, String)>,
    Query(q): Query<AgentQuery>,
) -> ApiResult<Response> {
    ids::require_session_id(&id)?;
    let offset: u64 = offset
        .parse()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid_id", "Bad media offset."))?;
    if uuid != "_" && !ids::is_uuid(&uuid) && !ids::is_simple_id(&uuid) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_id",
            "Bad record id.",
        ));
    }
    let block_path = ids::parse_block_path(&block)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid_id", "Bad block path."))?;
    let agent = q.agent.filter(|a| !a.is_empty());
    let path = transcript_file(&state, &id, agent.as_deref())
        .await?
        .ok_or_else(|| ApiError::not_found("media_not_found", "No transcript for this session."))?;
    let read = tokio::task::spawn_blocking(move || {
        uievents::read_media(&path, offset, &uuid, &block_path)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    match read {
        Ok((mime, bytes)) => Ok(locked_down(bytes.into_response(), mime)),
        Err(uievents::MediaError::NotFound) => Err(ApiError::not_found(
            "media_not_found",
            "No image at that position.",
        )),
        Err(uievents::MediaError::Unsupported) => Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media",
            "Only PNG, JPEG, GIF and WebP images are served.",
        )),
        Err(uievents::MediaError::TooLarge) => Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "media_too_large",
            "The image is too large to serve.",
        )),
        Err(uievents::MediaError::Io(e)) => {
            Err(ApiError::internal(format!("Could not read the image: {e}")))
        }
    }
}

/// Compute a session's agents if nobody has yet (history sessions).
async fn ensure_agents(state: &AppState, s: &SessionSummary) {
    if state.agents.has_session(&s.id) {
        return;
    }
    let st = state.clone();
    let (sid, live, owned, started) = (s.id.clone(), s.live, s.owned, s.started_at);
    let _ = tokio::task::spawn_blocking(move || {
        st.agents.recompute(
            &st.index,
            &sid,
            &claude::agents::AgentCtx {
                live,
                process_started: started,
                owned,
                now: claude::registry::now_ms(),
            },
        )
    })
    .await;
}

/// `GET /api/sessions/{id}/agents` → `{agents, workflows}`.
pub async fn session_agents(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let s = session_or_404(&state, &id).await?;
    ensure_agents(&state, &s).await;
    let (agents, workflows) = state.agents.for_session(&id);
    Ok(Json(json!({ "agents": agents, "workflows": workflows })).into_response())
}

/// `GET /api/agents` → `{sessions: FleetSession[], summary}`.
pub async fn fleet(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "sessions": state.agents.fleet(),
        "summary": state.agents.fleet_summary(),
    }))
}

/// Largest task output tail served.
const OUTPUT_TAIL: u64 = 16 * 1024;

/// Directories background task output may live in: the CLI's per-user temp
/// root (`/tmp/claude-<uid>`, also under `$TMPDIR`).
fn claude_tmp_roots() -> Vec<PathBuf> {
    #[cfg(unix)]
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    #[cfg(not(unix))]
    let uid = 0u32;
    let name = format!("claude-{uid}");
    let mut roots = vec![
        PathBuf::from("/tmp").join(&name),
        std::env::temp_dir().join(&name),
    ];
    if let Ok(t) = std::env::var("CLAUDE_CODE_TMPDIR") {
        roots.push(PathBuf::from(t).join(&name));
    }
    roots
}

/// Read the last `max` bytes of a file, returning (text, truncated, size).
fn tail_file(path: &FsPath, max: u64) -> std::io::Result<(String, bool, u64)> {
    let mut f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    let start = size.saturating_sub(max);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((size - start) as usize);
    f.take(max).read_to_end(&mut buf)?;
    let mut text = String::from_utf8_lossy(&buf).to_string();
    if start > 0 {
        // Start at a line boundary when one is near.
        if let Some(i) = text.find('\n').filter(|i| *i < 1024) {
            text = text[i + 1..].to_string();
        }
    }
    Ok((text, start > 0, size))
}

/// `GET /api/sessions/{id}/tasks/{taskId}/output` → the end of a background
/// task's output, as `text/plain`. An owned session's CLI answers itself
/// (`get_task_output`: its own 8 KiB tail, Monitor tasks included, whatever
/// temp directory the shell was given); for every other session, and for a
/// CLI that predates the request, it is the last 16 KB of the file the
/// transcript names (`outputFile` / `<output-file>` / `task_status`), which
/// must lie under the CLI's temp root.
pub async fn task_output(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path((id, task)): Path<(String, String)>,
) -> ApiResult<Response> {
    ids::require_session_id(&id)?;
    if !ids::is_task_id(&task) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_id",
            "Not a valid task id.",
        ));
    }
    if claude::owned::has_handle(&state, &id) {
        if let Ok(v) = claude::owned::task_output(&state, &id, &task).await {
            if let Some(text) = v.get("output").and_then(Value::as_str) {
                let size = v
                    .get("total_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(text.len() as u64);
                let truncated = v.get("truncated").and_then(Value::as_bool).unwrap_or(false);
                return Ok(output_response(text.to_string(), truncated, size));
            }
        }
    }
    let recorded = state
        .index
        .with_main(&id, |f| {
            f.agg
                .tasks
                .iter()
                .find(|t| t.id == task)
                .and_then(|t| t.output_file.clone())
        })
        .flatten()
        .ok_or_else(|| ApiError::not_found("no_output", "No output is recorded for this task."))?;
    let out = tokio::task::spawn_blocking(move || {
        let candidate = PathBuf::from(&recorded);
        let real = claude_tmp_roots()
            .iter()
            .find_map(|root| ids::under_root(root, &candidate))?;
        tail_file(&real, OUTPUT_TAIL).ok()
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("no_output", "The task's output file is not available."))?;
    let (text, truncated, size) = out;
    Ok(output_response(text, truncated, size))
}

/// A task output tail as `text/plain`, with the whole output's size and
/// whether this is only its end.
fn output_response(text: String, truncated: bool, size: u64) -> Response {
    let mut resp = locked_down(text.into_response(), "text/plain; charset=utf-8");
    if let Ok(v) = HeaderValue::from_str(&size.to_string()) {
        resp.headers_mut().insert("x-output-size", v);
    }
    if truncated {
        resp.headers_mut()
            .insert("x-output-truncated", HeaderValue::from_static("true"));
    }
    resp
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
    let branch = match q.base.as_deref() {
        None | Some("working") => false,
        Some("branch") => true,
        Some(_) => {
            return Err(ApiError::bad_request(
                "`base` must be `working` or `branch`.",
            ))
        }
    };
    if session.cwd.is_empty() {
        return Ok(Json(claude::GitOverview::default()).into_response());
    }
    let cwd = PathBuf::from(session.cwd);
    let overview = tokio::task::spawn_blocking(move || {
        if branch {
            claude::git::branch_overview(&cwd, 25)
        } else {
            claude::git_overview(&cwd, 25)
        }
    })
    .await
    .unwrap_or_default();
    Ok(Json(overview).into_response())
}

/// `GET /api/sessions/{id}/file-patch?path=&base=` → `{patch}`.
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
    if !ids::is_safe_relative_path(&rel) {
        return Err(ApiError::bad_request(
            "`path` must be relative to the session's directory.",
        ));
    }
    let branch = params.get("base").map(String::as_str) == Some("branch");
    let cwd = PathBuf::from(session.cwd);
    let patch = tokio::task::spawn_blocking(move || {
        claude::git::file_patch_against(&cwd, &rel, 256_000, branch)
    })
    .await
    .ok()
    .flatten();
    Ok(Json(json!({ "patch": patch })).into_response())
}

/// `GET /api/sessions/{id}/files?q=` → `{files}` (tracked files under the
/// session's cwd, fuzzy matched, ≤ 50).
pub async fn files(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Response> {
    let session = session_or_404(&state, &id).await?;
    let q = params.get("q").cloned().unwrap_or_default();
    if q.len() > 256 {
        return Err(ApiError::bad_request("`q` is too long."));
    }
    if session.cwd.is_empty() {
        return Ok(Json(json!({ "files": [] })).into_response());
    }
    let cwd = PathBuf::from(session.cwd);
    let files = tokio::task::spawn_blocking(move || {
        let all = claude::git::list_files(&cwd);
        claude::git::fuzzy_files(&all, &q, 50)
    })
    .await
    .unwrap_or_default();
    Ok(Json(json!({ "files": files })).into_response())
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
        ("usage", "Show plan usage limits", None),
        ("status", "Show version, model and account status", None),
        ("model", "Switch the model", Some("[model]")),
        (
            "effort",
            "Set reasoning effort",
            Some("[low|medium|high|xhigh|max]"),
        ),
        ("rename", "Rename the session", Some("<name>")),
        ("review", "Review the current changes", None),
        ("init", "Create a CLAUDE.md for this project", None),
        ("memory", "Edit memory files", None),
        ("todos", "Show the todo list", None),
        ("btw", "Ask a side question", Some("<question>")),
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

/// Temp directories are not offered as projects.
fn is_temp_dir(p: &str) -> bool {
    p.starts_with("/private/var/folders/")
        || p.starts_with("/var/folders/")
        || p.starts_with("/private/tmp/")
        || p.starts_with("/tmp/")
}

/// `GET /api/projects` → `{projects}` for the spawn picker: directories with
/// sessions plus the ones trusted in the CLI, most recent first; only
/// directories that still exist.
pub async fn projects(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    let sessions = state.sessions().await;
    let st = state.clone();
    let projects = tokio::task::spawn_blocking(move || {
        // Trusted in any account's directory (docs/ACCOUNTS.md P2.1).
        let mut trusted: Vec<String> = Vec::new();
        for (_, home) in st.accounts.homes() {
            for p in claude::settings::trusted_projects(&home) {
                if !trusted.contains(&p) {
                    trusted.push(p);
                }
            }
        }
        let mut by_path: BTreeMap<String, ProjectInfo> = BTreeMap::new();
        for s in &sessions {
            if s.cwd.is_empty() {
                continue;
            }
            let entry = by_path.entry(s.cwd.clone()).or_insert_with(|| ProjectInfo {
                path: s.cwd.clone(),
                name: s.project_name.clone(),
                last_activity_at: None,
                trusted: None,
                sessions: Some(0),
            });
            entry.last_activity_at = entry.last_activity_at.max(s.last_activity_at);
            entry.sessions = Some(entry.sessions.unwrap_or(0) + 1);
        }
        for t in &trusted {
            let path = t.trim_end_matches('/').to_string();
            let entry = by_path.entry(path.clone()).or_insert_with(|| ProjectInfo {
                name: FsPath::new(&path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&path)
                    .to_string(),
                path,
                ..Default::default()
            });
            entry.trusted = Some(true);
        }
        let mut projects: Vec<ProjectInfo> = by_path
            .into_values()
            .filter(|p| !is_temp_dir(&p.path))
            .filter(|p| FsPath::new(&p.path).is_dir())
            .collect();
        projects.sort_by_key(|p| std::cmp::Reverse(p.last_activity_at));
        projects
    })
    .await
    .unwrap_or_default();
    Json(json!({ "projects": projects }))
}

const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Launch defaults + available choices for the spawn/continue forms.
pub async fn defaults(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    let d = claude::read_launch_defaults(&state.home);
    Json(json!({
        "model": d.model,
        "effort": d.effort,
        "alwaysThinkingEnabled": d.always_thinking_enabled,
        "models": d.models,
        "efforts": EFFORT_LEVELS,
    }))
}

/// `GET /api/inbox` → `{items}`: every pending item across sessions in full —
/// the store's answerable ones plus observed ones — oldest first.
pub async fn inbox(Authed(_ctx): Authed, State(state): State<AppState>) -> impl IntoResponse {
    let mut items: Vec<PendingItem> = state.pending.all();
    items.extend(state.activity.observed_all());
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
        .filter(|s| {
            s.kind
                .as_deref()
                .is_some_and(|k| k == "background" || k == "bg")
        })
        .map(|s| json!({ "id": s.id, "jobId": s.job_id, "cwd": s.cwd, "state": s.status }))
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
        .stdin(std::process::Stdio::null())
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

/// Placeholder for a redacted value.
const REDACTED: &str = "…";

/// `scheme://host[:port]` of a URL: userinfo, path, query and fragment go
/// (tokens live in all of them, e.g. `…/s/<secret>/mcp` or `?api_key=`).
/// `None` when `raw` is not an absolute URL.
fn url_origin(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw.trim()).ok()?;
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

/// A flag whose value is (or may be) a credential: `--api-key`, `--token`,
/// `--secret`, `--password`, `--auth…`, `-p`-style short flags excluded.
fn is_secret_flag(flag: &str) -> bool {
    let name = flag.trim_start_matches('-').to_ascii_lowercase();
    flag.starts_with("--")
        && [
            "key",
            "token",
            "secret",
            "password",
            "passwd",
            "pass",
            "auth",
            "credential",
            "cookie",
        ]
        .iter()
        .any(|w| name.contains(w))
}

/// A long, random-looking string (an API key pasted as a bare argument).
fn looks_like_secret(arg: &str) -> bool {
    arg.len() >= 20
        && !arg.contains(['/', ' ', '.'])
        && arg.chars().any(|c| c.is_ascii_digit())
        && arg.chars().any(|c| c.is_ascii_alphabetic())
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '=' | '+'))
}

/// An MCP server's `args` with anything credential-shaped replaced: URLs cut
/// to their origin (a DSN's password, a tokenized path), the value of a
/// secret flag (`--api-key X`, `--token=X`), `KEY=value` pairs, and bare
/// random-looking strings.
fn redact_args(args: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(args.len());
    let mut secret_next = false;
    for arg in args {
        let Some(a) = arg.as_str() else {
            out.push(json!(REDACTED));
            continue;
        };
        if std::mem::take(&mut secret_next) {
            out.push(json!(REDACTED));
            continue;
        }
        if a.starts_with('-') {
            match a.split_once('=') {
                // `--flag=value`: the value may be anything; keep the flag.
                Some((flag, _)) => out.push(json!(format!("{flag}={REDACTED}"))),
                None => {
                    secret_next = is_secret_flag(a);
                    out.push(json!(a));
                }
            }
            continue;
        }
        if a.contains("://") {
            out.push(json!(url_origin(a).unwrap_or_else(|| REDACTED.to_string())));
            continue;
        }
        if let Some((k, _)) = a.split_once('=') {
            out.push(json!(format!("{k}={REDACTED}")));
            continue;
        }
        if (a.contains('@') && a.contains(':')) || looks_like_secret(a) {
            out.push(json!(REDACTED));
            continue;
        }
        out.push(json!(a));
    }
    out
}

/// One MCP server, rebuilt from an allowlist: `type`, the `command`'s file
/// name, redacted `args` ([`redact_args`]), the `url`'s origin, and the KEYS
/// of `env` / `headers` with every value `"…"`. Unknown fields are dropped.
fn redact_server(server: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(t) = server.get("type").and_then(Value::as_str) {
        out.insert("type".into(), json!(t));
    }
    if let Some(cmd) = server.get("command").and_then(Value::as_str) {
        let name = std::path::Path::new(cmd.trim())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.insert("command".into(), json!(name));
    }
    if let Some(args) = server.get("args").and_then(Value::as_array) {
        out.insert("args".into(), Value::Array(redact_args(args)));
    }
    if let Some(url) = server.get("url").and_then(Value::as_str) {
        out.insert(
            "url".into(),
            json!(url_origin(url).unwrap_or_else(|| REDACTED.to_string())),
        );
    }
    for field in ["env", "headers"] {
        if let Some(obj) = server.get(field).and_then(Value::as_object) {
            let keys: serde_json::Map<String, Value> =
                obj.keys().map(|k| (k.clone(), json!(REDACTED))).collect();
            out.insert(field.into(), Value::Object(keys));
        }
    }
    Value::Object(out)
}

/// Never serve secret values: every paired device can read this, so each
/// server is rebuilt from an allowlist ([`redact_server`]).
pub fn redact_mcp(servers: Value) -> Value {
    match servers {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(name, server)| (name.clone(), redact_server(server)))
                .collect(),
        ),
        _ => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page boundary between two sessions of the same time skips
    /// neither: the cursor carries the id; a bare time (an older client's)
    /// still pages as it did.
    #[test]
    fn a_page_cursor_keeps_rows_that_share_a_time() {
        let row = |id: &str, at: i64| SessionSummary {
            id: id.into(),
            last_activity_at: Some(at),
            ..Default::default()
        };
        // The snapshot's order: newest first, ties by id.
        let rows = [row("a", 30), row("b", 20), row("c", 20), row("d", 10)];
        let cursor = cursor_of(&rows[1]);
        assert_eq!(cursor, "20:b");
        let c = parse_cursor(&cursor).unwrap();
        let next: Vec<&str> = rows
            .iter()
            .filter(|s| after_cursor(s, c))
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(next, ["c", "d"], "c shares b's time and is not skipped");
        let bare = parse_cursor("20").unwrap();
        assert_eq!(
            rows.iter().filter(|s| after_cursor(s, bare)).count(),
            1,
            "a bare time keeps only what is older"
        );
        assert!(parse_cursor("soon").is_none() && parse_cursor("x:b").is_none());
    }

    #[test]
    fn mcp_secrets_are_redacted() {
        let v = redact_mcp(json!({
            "a": { "command": "/usr/local/bin/x", "env": { "TOKEN": "secret" },
                   "headers": { "Authorization": "Bearer s" }, "extraSecret": "secret" },
            "b": { "type": "http", "url": "https://x" },
            "pg": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-postgres",
                    "postgresql://user:hunter2secret@db.internal:5432/prod"] },
            "k": { "command": "tool", "args": ["--api-key", "sk-live-secret-1234567890",
                    "--token=ghp_secret123", "--verbose", "API_KEY=secret", "--port", "8080",
                    "abcdefghijklmnop1234567890secret"] },
            "z": { "type": "http",
                   "url": "https://user:pw-secret@mcp.zapier.com/api/mcp/s/secret-path/mcp?api_key=secret#frag" },
            "bad": "not an object",
        }));
        assert_eq!(v["a"]["env"]["TOKEN"], "…");
        assert_eq!(v["a"]["headers"]["Authorization"], "…");
        assert_eq!(v["a"]["command"], "x", "the file name only");
        assert!(
            v["a"].get("extraSecret").is_none(),
            "unknown fields are dropped"
        );
        assert_eq!(v["b"]["url"], "https://x");
        assert_eq!(v["b"]["type"], "http");
        // A DSN in args keeps only its origin.
        assert_eq!(v["pg"]["args"][1], "@modelcontextprotocol/server-postgres");
        assert_eq!(v["pg"]["args"][2], "postgresql://db.internal:5432");
        // Secret flags, KEY=value pairs and bare keys.
        let k = &v["k"]["args"];
        assert_eq!(k[0], "--api-key");
        assert_eq!(k[1], "…");
        assert_eq!(k[2], "--token=…");
        assert_eq!(k[3], "--verbose");
        assert_eq!(k[4], "API_KEY=…");
        assert_eq!(k[5], "--port");
        assert_eq!(k[6], "8080");
        assert_eq!(k[7], "…");
        // A URL with userinfo, a secret path, query and fragment: the origin.
        assert_eq!(v["z"]["url"], "https://mcp.zapier.com");
        assert!(v["bad"].as_object().unwrap().is_empty());
        let text = v.to_string();
        assert!(!text.contains("secret"), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("pw-"), "{text}");
    }

    #[test]
    fn common_commands_are_slash_prefixed() {
        assert!(common_commands().iter().all(|c| c.name.starts_with('/')));
    }

    #[test]
    fn tails_files_at_a_line_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.output");
        let mut s = String::new();
        for i in 0..5000 {
            s += &format!("line {i}\n");
        }
        std::fs::write(&p, &s).unwrap();
        let (text, truncated, size) = tail_file(&p, 1024).unwrap();
        assert!(truncated);
        assert_eq!(size, s.len() as u64);
        assert!(text.starts_with("line "));
        assert!(text.ends_with("line 4999\n"));
        let (small, truncated, _) = tail_file(&p, 1_000_000).unwrap();
        assert!(!truncated);
        assert_eq!(small, s);
    }

    #[test]
    fn temp_dirs_are_not_projects() {
        assert!(is_temp_dir("/private/var/folders/h5/x"));
        assert!(is_temp_dir("/tmp/abc"));
        assert!(!is_temp_dir("/Users/me/dev/app"));
    }
}
