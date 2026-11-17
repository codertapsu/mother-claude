//! The dashboard's side of the Claude API (docs/CLAUDE-API.md "Managing
//! clients"): the clients and conversations in each API server's folder —
//! listed, read and deleted.
//!
//! * `GET /api/bridge/clients` — every folder an API server keeps clients
//!   in, each client with its conversations' counts; `GET
//!   /api/bridge/clients/{client}?root=` its conversations; `GET
//!   /api/bridge/api-conversations/{id}` one, with its messages.
//! * `POST /api/bridge/api-conversations/delete {ids, transcripts}` and
//!   `DELETE /api/bridge/clients/{client}?root=&transcripts=` (`ManageData`):
//!   Mother Claude's records go (and the processes close); with
//!   `transcripts`, the Claude Code transcripts too, through the data
//!   delete (an undo copy for seven days, the dangerous right, an audit
//!   line, docs/ACCOUNTS.md §14.3).

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::store::{ClientRecord, Record};
use crate::server::auth::{self, Authed, ManageData};
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

// ─────────────────────────────── clients ───────────────────────────────

/// Every folder an API server keeps its clients in (the default one first),
/// with the servers that use it.
pub fn roots(state: &AppState) -> Vec<(PathBuf, Vec<String>)> {
    let mut out: Vec<(PathBuf, Vec<String>)> = Vec::new();
    if let Some(d) = &state.bridge.api.default_root {
        out.push((d.clone(), Vec::new()));
    }
    for server in state.bridge.servers.all() {
        for defaults in [server.defaults(), server.saved()] {
            let Some(root) = super::root_for(state, &defaults) else {
                continue;
            };
            match out.iter_mut().find(|(r, _)| *r == root) {
                Some((_, servers)) => {
                    if !servers.contains(&server.id) {
                        servers.push(server.id.clone());
                    }
                }
                None => out.push((root, vec![server.id.clone()])),
            }
        }
    }
    out
}

/// The root a request names (`?root=`), when it is one of [`roots`]; the
/// default one when it names none.
fn root_named(state: &AppState, asked: Option<&String>) -> ApiResult<PathBuf> {
    let known = roots(state);
    match asked {
        None => known
            .first()
            .map(|(r, _)| r.clone())
            .ok_or_else(|| ApiError::conflict("no_data_dir", "There is no API data folder.")),
        Some(asked) => known
            .into_iter()
            .map(|(r, _)| r)
            .find(|r| r.as_os_str() == asked.as_str())
            .ok_or_else(|| {
                ApiError::not_found("root_not_found", "No API server keeps clients there.")
            }),
    }
}

/// A client folder's name, as the store writes them.
fn client_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn client_record(dir: &FsPath) -> Option<ClientRecord> {
    crate::persist::read_json::<ClientRecord>(&dir.join("client.json"))
}

/// A conversation as the dashboard lists it (no messages).
fn conversation_row(state: &AppState, r: &Record) -> Value {
    let (busy, waiting) = state.bridge.api.state_of(&r.id).unwrap_or_default();
    json!({
        "id": r.id,
        "title": r.title,
        "model": r.model_used.clone().unwrap_or_else(|| r.model.clone()),
        "createdAt": r.created_at,
        "updatedAt": r.updated_at,
        "messages": r.messages.len(),
        "requests": r.requests,
        "usage": r.usage,
        "seeded": r.seeded,
        "forkedFrom": r.forked_from,
        "server": r.server,
        "account": r.account,
        "busy": busy,
        "waitingForTools": waiting,
        "live": state.bridge.api.with(&r.id, |c| c.open).unwrap_or(false),
    })
}

/// The answered conversations of a client folder, newest first.
fn answered(state: &AppState, dir: &FsPath) -> Vec<Record> {
    state
        .bridge
        .api
        .records_in(dir)
        .into_iter()
        .filter(|r| !r.history.is_empty())
        .collect()
}

/// `GET /api/bridge/clients` — `{roots: [{path, servers, default, clients:
/// [{id, kind, label, firstSeen, lastSeen, requests, conversations, active,
/// usage}]}]}`.
pub async fn list_clients(Authed(_ctx): Authed, State(state): State<AppState>) -> Response {
    let st = state.clone();
    let out = tokio::task::spawn_blocking(move || {
        let default = st.bridge.api.default_root.clone();
        roots(&st)
            .into_iter()
            .map(|(root, servers)| {
                let mut clients = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&root) {
                    for e in entries.flatten() {
                        let dir = e.path();
                        let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
                            continue;
                        };
                        if !dir.is_dir() || !client_id_ok(name) {
                            continue;
                        }
                        let Some(rec) = client_record(&dir) else {
                            continue;
                        };
                        let convs = answered(&st, &dir);
                        let mut usage = super::store::Usage::default();
                        for r in &convs {
                            usage.input_tokens += r.usage.input_tokens;
                            usage.output_tokens += r.usage.output_tokens;
                            usage.cache_read_input_tokens += r.usage.cache_read_input_tokens;
                            usage.cache_creation_input_tokens +=
                                r.usage.cache_creation_input_tokens;
                        }
                        let active = convs
                            .iter()
                            .filter(|r| st.bridge.api.state_of(&r.id).is_some_and(|(b, w)| b || w))
                            .count();
                        clients.push(json!({
                            "id": name,
                            "kind": rec.kind,
                            "label": rec.label,
                            "firstSeen": rec.first_seen,
                            "lastSeen": rec.last_seen,
                            "requests": rec.requests,
                            "conversations": convs.len(),
                            "active": active,
                            "usage": usage,
                            "folder": dir.to_string_lossy(),
                        }));
                    }
                }
                clients.sort_by_key(|c| std::cmp::Reverse(c["lastSeen"].as_i64().unwrap_or(0)));
                json!({
                    "path": root.to_string_lossy(),
                    "default": default.as_ref() == Some(&root),
                    "servers": servers,
                    "clients": clients,
                })
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    Json(json!({ "roots": out })).into_response()
}

/// `GET /api/bridge/clients/{client}?root=` — its conversations.
pub async fn get_client(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(client): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Response> {
    if !client_id_ok(&client) {
        return Err(ApiError::bad_request("Not a client."));
    }
    let root = root_named(&state, q.get("root"))?;
    let dir = root.join(&client);
    let Some(rec) = client_record(&dir) else {
        return Err(ApiError::not_found(
            "client_not_found",
            format!("No client {client}."),
        ));
    };
    let rows: Vec<Value> = answered(&state, &dir)
        .iter()
        .map(|r| conversation_row(&state, r))
        .collect();
    Ok(Json(json!({
        "client": {
            "id": client,
            "kind": rec.kind,
            "label": rec.label,
            "firstSeen": rec.first_seen,
            "lastSeen": rec.last_seen,
            "requests": rec.requests,
            "folder": dir.to_string_lossy(),
        },
        "conversations": rows,
    }))
    .into_response())
}

/// Finds a conversation's record among the known folders.
fn find(state: &AppState, id: &str) -> Option<Record> {
    if let Some(r) = state.bridge.api.with(id, |c| c.record.clone()) {
        return Some(r);
    }
    for (root, _) in roots(state) {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in entries.flatten() {
            if e.path()
                .join("conversations")
                .join(format!("{id}.json"))
                .is_file()
            {
                state.bridge.api.records_in(&e.path());
                return state.bridge.api.with(id, |c| c.record.clone());
            }
        }
    }
    None
}

/// `GET /api/bridge/api-conversations/{id}` — one, with its messages.
pub async fn get_conversation(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    if !crate::claude::ids::is_uuid(&id) {
        return Err(ApiError::bad_request("Not a conversation id."));
    }
    let st = state.clone();
    let found = tokio::task::spawn_blocking(move || find(&st, &id))
        .await
        .ok()
        .flatten();
    match found {
        Some(r) => {
            let mut row = conversation_row(&state, &r);
            row["client"] = json!(r.client);
            row["messageList"] = json!(r.messages);
            Ok(Json(row).into_response())
        }
        None => Err(ApiError::not_found(
            "conversation_not_found",
            "No such conversation.",
        )),
    }
}

/// Forgets a conversation: its process closed, its record gone.
pub async fn forget_conversation(state: &AppState, id: &str) {
    if state.bridge.api.with(id, |c| c.open).unwrap_or(false)
        || state.bridge.thread(id).await.is_some()
    {
        super::routes::close_conversation(state, id).await;
    }
    state.bridge.api.forget(id);
}

/// Starts the data delete of the transcripts of `ids` (the dangerous right
/// checked by the caller): the jobs' views.
async fn delete_transcripts(
    state: &AppState,
    ctx: &crate::server::context::ClientContext,
    ids: Vec<String>,
) -> ApiResult<Value> {
    if ids.is_empty() {
        return Ok(json!([]));
    }
    if crate::claude::transfer::ops::undo_root(state).is_none() {
        return Err(ApiError::conflict(
            "no_data_dir",
            "This build has no data directory to keep an undo in.",
        ));
    }
    let req = crate::claude::transfer::delete::DeleteRequest {
        from: None,
        items: crate::claude::transfer::delete::DeleteItems {
            sessions: ids,
            projects: Vec::new(),
        },
        undo: true,
        dry_run: false,
    };
    let plan = crate::claude::transfer::delete::plan(state, &req)
        .await
        .map_err(|e| ApiError::conflict("delete_failed", e.to_string()))?;
    let mut detail = crate::claude::transfer::delete::audit_detail(&plan, &[]);
    let jobs = crate::claude::transfer::delete::start(state, plan);
    detail["jobs"] = json!(jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>());
    state.devices.audit.record(AuditEntry {
        detail: Some(detail),
        ..AuditEntry::from_ctx(ctx, "data.delete")
    });
    Ok(json!(jobs))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteConversations {
    pub ids: Vec<String>,
    /// Their Claude Code transcripts too (the dangerous right).
    #[serde(default)]
    pub transcripts: bool,
}

/// `POST /api/bridge/api-conversations/delete` → `{deleted, busy, jobs}`:
/// one answering a request now is left (`busy`).
pub async fn delete_conversations(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<DeleteConversations>,
) -> ApiResult<Response> {
    if body.ids.is_empty() || body.ids.len() > 1000 {
        return Err(ApiError::bad_request("Name 1–1000 conversations."));
    }
    if body.ids.iter().any(|id| !crate::claude::ids::is_uuid(id)) {
        return Err(ApiError::bad_request("Not a conversation id."));
    }
    if body.transcripts {
        auth::require_dangerous(&ctx, "Deleting conversations' transcripts")?;
    }
    let (deleted, busy) = forget_many(&state, &body.ids).await;
    let jobs = if body.transcripts {
        delete_transcripts(&state, &ctx, deleted.clone()).await?
    } else {
        json!([])
    };
    auth::note_device_action(
        &state,
        &ctx,
        &format!("deleted {} HTTP API conversation(s)", deleted.len()),
    );
    Ok(Json(json!({ "deleted": deleted, "busy": busy, "jobs": jobs })).into_response())
}

/// Forgets the conversations of `ids` that are known and not answering a
/// request: those forgotten, those left.
async fn forget_many(state: &AppState, ids: &[String]) -> (Vec<String>, Vec<String>) {
    let mut deleted = Vec::new();
    let mut busy = Vec::new();
    for id in ids {
        let st = state.clone();
        let wanted = id.clone();
        let known = tokio::task::spawn_blocking(move || find(&st, &wanted).is_some())
            .await
            .unwrap_or(false);
        if !known {
            continue;
        }
        if state.bridge.api.state_of(id).is_some_and(|(b, _)| b) {
            busy.push(id.clone());
            continue;
        }
        forget_conversation(state, id).await;
        deleted.push(id.clone());
    }
    (deleted, busy)
}

/// `DELETE /api/bridge/clients/{client}?root=&transcripts=` — everything of
/// a client: its conversations, its folder's files (only the ones Mother
/// Claude wrote; the folder goes when that leaves it empty), and with
/// `transcripts=true` their Claude Code transcripts.
pub async fn delete_client(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(client): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Response> {
    if !client_id_ok(&client) {
        return Err(ApiError::bad_request("Not a client."));
    }
    let transcripts = q.get("transcripts").map(String::as_str) == Some("true");
    if transcripts {
        auth::require_dangerous(&ctx, "Deleting a client's transcripts")?;
    }
    let root = root_named(&state, q.get("root"))?;
    let dir = root.join(&client);
    if client_record(&dir).is_none() {
        return Err(ApiError::not_found(
            "client_not_found",
            format!("No client {client}."),
        ));
    }
    let ids: Vec<String> = state
        .bridge
        .api
        .records_in(&dir)
        .into_iter()
        .map(|r| r.id)
        .collect();
    let (deleted, busy) = forget_many(&state, &ids).await;
    if !busy.is_empty() {
        return Err(ApiError::conflict(
            "client_busy",
            "A conversation of this client is answering a request; try again when it is done.",
        )
        .with("busy", json!(busy)));
    }
    let jobs = if transcripts {
        delete_transcripts(&state, &ctx, deleted.clone()).await?
    } else {
        json!([])
    };
    remove_client_files(&dir);
    state.bridge.api.unload(&dir);
    auth::note_device_action(
        &state,
        &ctx,
        &format!("deleted the HTTP API client {client} and its data"),
    );
    Ok(Json(json!({ "deleted": deleted, "jobs": jobs })).into_response())
}

/// The files Mother Claude keeps for a client, and the folders they leave
/// empty. Nothing else in the folder is touched.
fn remove_client_files(dir: &FsPath) {
    let convs = dir.join("conversations");
    if let Ok(entries) = std::fs::read_dir(&convs) {
        for e in entries.flatten() {
            let p = e.path();
            let ours = p.extension().and_then(|x| x.to_str()) == Some("json")
                || p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains(".tmp-"));
            if ours && p.is_file() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    let _ = std::fs::remove_dir(&convs);
    let _ = std::fs::remove_file(dir.join("client.json"));
    let _ = std::fs::remove_dir(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ids_are_folder_names_only() {
        assert!(client_id_ok("127.0.0.1"));
        assert!(client_id_ok("key-key_0123"));
        assert!(!client_id_ok(".."));
        assert!(!client_id_ok("a/b"));
        assert!(!client_id_ok("a\\b"));
        assert!(!client_id_ok(""));
    }

    #[test]
    fn only_our_files_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let client = dir.path().join("c");
        std::fs::create_dir_all(client.join("conversations")).unwrap();
        std::fs::write(client.join("client.json"), "{}").unwrap();
        std::fs::write(client.join("conversations").join("x.json"), "{}").unwrap();
        std::fs::write(client.join("notes.txt"), "mine").unwrap();
        remove_client_files(&client);
        assert!(
            client.join("notes.txt").is_file(),
            "someone else's file stays"
        );
        assert!(!client.join("client.json").exists());
        assert!(!client.join("conversations").exists());

        std::fs::remove_file(client.join("notes.txt")).unwrap();
        std::fs::create_dir_all(client.join("conversations")).unwrap();
        std::fs::write(client.join("client.json"), "{}").unwrap();
        remove_client_files(&client);
        assert!(!client.exists(), "an emptied folder goes");
    }
}
