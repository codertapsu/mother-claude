//! Turning code sync on and off for a project (docs/PLAN-MACHINES.md
//! §5.15 "Turning it on"): this computer's desktop turns it on for its
//! checkout with one paired computer — which asks that computer (an asked
//! action, `setup::asks`), whose desktop confirms there — and turns it off
//! at once, telling the other.
//!
//! What both must hold before it is on: the project is a git repository's
//! top here; a root maps it to a folder there that is the top of a
//! repository of the same project (the same `origin`, or a commit in
//! common); each computer gave the other its `move` grant; Machines is on
//! on both; neither syncs the folder with a third computer.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{audit, is_sync_id, name_of, new_sync_id, update, Pair, Status, Trigger, MAX_PAIRS};
use crate::claude::paths::{self, PathForm};
use crate::machines::code;
use crate::machines::grants::Grant;
use crate::machines::setup::asks::{self, Ask, AskKind, AskState};
use crate::machines::setup::folders;
use crate::machines::{self as m, roots, transport};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// What an ask of kind `sync` carries (`AskRequest::sync`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncAsk {
    /// The pair's id, the same on both computers.
    pub id: String,
    /// The asking computer's folder.
    pub folder: String,
    /// The folder on the asked computer, as the asking one maps it.
    pub there: String,
    /// The asking checkout's `origin` (without credentials).
    #[serde(default)]
    pub origin: Option<String>,
    /// Commits it has (its first ones and HEAD): one in common makes two
    /// checkouts one project.
    #[serde(default)]
    pub commits: Vec<String>,
}

/// `PUT /api/machines/sync {project, machine, on}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetBody {
    pub project: String,
    pub machine: String,
    pub on: bool,
}

/// What a checkout tells about itself for the other computer's check.
struct Here {
    top: String,
    origin: Option<String>,
    commits: Vec<String>,
}

/// The repository whose top `project` is (or holds it), and what proves
/// it is the same project elsewhere.
async fn repo_here(state: &AppState, project: &str) -> ApiResult<Here> {
    let folder = paths::clean_folder(project, PathForm::here()).map_err(|why| {
        ApiError::unprocessable("sync_bad_project", format!("That project: {why}."))
    })?;
    let g = super::step::git(state);
    let f = PathBuf::from(&folder);
    let read = tokio::task::spawn_blocking(move || {
        let s = code::checkout_state(&g, &f)?;
        let mut commits = code::root_commits(&g, &f)?;
        if let Some(h) = &s.head {
            commits.push(h.clone());
        }
        Ok::<_, code::CodeError>((s, commits))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let (s, commits) = read.map_err(|_| {
        ApiError::unprocessable(
            "sync_not_a_repository",
            format!("{folder} is not in a git repository."),
        )
    })?;
    Ok(Here {
        top: s.top.to_string_lossy().into_owned(),
        origin: s.origin.as_deref().and_then(folders::sanitize_origin),
        commits,
    })
}

/// Both computers gave each other `move` (on either path): sync writes into
/// each other's checkout.
fn grants_both_ways(state: &AppState, machine: &str) -> ApiResult<()> {
    let name = m::peer_name(state, machine);
    let (ours, theirs) = state.machines.store_read(|f| {
        f.peer(machine)
            .map(|p| {
                (
                    p.grants.allows(Grant::Move, true) || p.grants.allows(Grant::Move, false),
                    p.their_grants
                        .map(|g| g.allows(Grant::Move, true) || g.allows(Grant::Move, false)),
                )
            })
            .unwrap_or((false, Some(false)))
    });
    if !ours {
        return Err(ApiError::conflict(
            "sync_no_grant",
            format!("This computer has not given {name} the Move permission: code sync writes into each other's project folder."),
        )
        .with("grant", "move"));
    }
    if theirs == Some(false) {
        return Err(ApiError::conflict(
            "sync_no_grant",
            format!("{name} has not given this computer the Move permission: code sync writes into each other's project folder."),
        )
        .with("grant", "move"));
    }
    Ok(())
}

/// Another computer's pair of this folder (a project syncs with one).
fn synced_elsewhere(state: &AppState, folder: &str, machine: &str) -> Option<String> {
    state.machines.sync.read(|f| {
        f.pairs
            .iter()
            .find(|p| p.machine != machine && p.here_on && paths::same_path(&p.folder, folder))
            .map(|p| m::peer_name(state, &p.machine))
    })
}

/// The open ask `machine` made of this computer to sync `folder`.
fn open_ask_from(state: &AppState, machine: &str, folder: &str) -> Option<Ask> {
    asks::view(state).into_iter().find(|a| {
        a.kind == AskKind::Sync
            && a.state == AskState::Asked
            && a.from == machine
            && a.folder
                .as_deref()
                .is_some_and(|f| paths::same_path(f, folder))
    })
}

/// `PUT /api/machines/sync`: turns a project's sync with `machine` on (it
/// asks that computer, whose desktop confirms) or off (at once here; the
/// other is told).
pub async fn set(state: &AppState, ctx: &ClientContext, body: SetBody) -> ApiResult<Value> {
    if !m::ids::is_machine_id(&body.machine)
        || !state
            .machines
            .store_read(|f| f.peer(&body.machine).is_some())
    {
        return Err(ApiError::not_found(
            "machines_unknown",
            "Not a paired computer.",
        ));
    }
    if body.on {
        turn_on(state, ctx, &body.project, &body.machine).await?;
    } else {
        turn_off(state, &body.project, &body.machine).await?;
    }
    Ok(super::view(state))
}

async fn turn_on(
    state: &AppState,
    ctx: &ClientContext,
    project: &str,
    machine: &str,
) -> ApiResult<()> {
    m::using(state)?;
    let here = repo_here(state, project).await?;
    let name = m::peer_name(state, machine);
    let existing = state.machines.sync.read(|f| {
        f.pairs
            .iter()
            .find(|p| p.machine == machine && paths::same_path(&p.folder, &here.top))
            .cloned()
    });
    if existing.as_ref().is_some_and(Pair::active) {
        return Ok(());
    }
    // It asked this computer already: turning it on here is the answer.
    if let Some(ask) = open_ask_from(state, machine, &here.top) {
        asks::answer(state, ctx, &ask.id, true).await?;
        return Ok(());
    }
    if let Some(other) = synced_elsewhere(state, &here.top, machine) {
        return Err(ApiError::conflict(
            "sync_elsewhere",
            format!(
                "{} is kept in sync with {other} already: turn that off first.",
                name_of(&here.top)
            ),
        ));
    }
    grants_both_ways(state, machine)?;
    let me = roots::here_id(state)
        .ok_or_else(|| ApiError::internal("This computer's id could not be read."))?;
    let there = roots::map_between(state, &me, machine, &here.top).ok_or_else(|| {
        ApiError::unprocessable(
            "sync_no_folder",
            format!(
                "No shared folder maps {} to {name}: add one in Machines › Set up › Accounts and folders.",
                name_of(&here.top)
            ),
        )
    })?;
    if state.machines.sync.read(|f| f.pairs.len()) >= MAX_PAIRS && existing.is_none() {
        return Err(ApiError::conflict(
            "sync_too_many",
            format!("At most {MAX_PAIRS} projects are kept in sync."),
        ));
    }
    let id = existing
        .as_ref()
        .map(|p| p.id.clone())
        .unwrap_or_else(new_sync_id);
    let ask = SyncAsk {
        id: id.clone(),
        folder: here.top.clone(),
        there: there.clone(),
        origin: here.origin.clone(),
        commits: here.commits.clone(),
    };
    let body = json!({ "check": "sync", "sync": ask });
    let (status, answer) = transport::request(
        state,
        machine,
        "POST",
        "/peer/setup/ask",
        Some(body),
        transport::REQUEST_TIMEOUT,
    )
    .await
    .map_err(|e| {
        ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "machine_unreachable",
            format!("{name} can't be reached right now: {e}"),
        )
    })?;
    if status >= 300 {
        let code = answer
            .pointer("/error/code")
            .and_then(Value::as_str)
            .unwrap_or("machines_refused")
            .to_string();
        let message = answer
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(|m| format!("{name}: {m}"))
            .unwrap_or_else(|| format!("{name} refused ({status})."));
        // Its refusal, as a conflict here (a 401 or 403 there is about this
        // computer, never the person asking here).
        let st = match axum::http::StatusCode::from_u16(status) {
            Ok(s)
                if s == axum::http::StatusCode::UNAUTHORIZED
                    || s == axum::http::StatusCode::FORBIDDEN =>
            {
                axum::http::StatusCode::CONFLICT
            }
            Ok(s) if s.is_client_error() => s,
            _ => axum::http::StatusCode::BAD_GATEWAY,
        };
        return Err(ApiError::new(st, code, message));
    }
    let now = m::now();
    let pair = Pair {
        id: id.clone(),
        machine: machine.to_string(),
        folder: here.top.clone(),
        there,
        here_on: true,
        peer_on: false,
        asked_at: Some(now),
        last: None,
        status: Status::Waiting,
        conflict: None,
        kept: Vec::new(),
        reset_ok: false,
        created_at: existing.as_ref().map(|p| p.created_at).unwrap_or(now),
        updated_at: now,
        extra: Default::default(),
    };
    state.machines.sync.write(|f| {
        f.pairs.retain(|p| p.id != id);
        f.pairs.push(pair.clone());
    });
    if let Some(kept) = answer
        .get("ask")
        .and_then(|a| serde_json::from_value::<Ask>(a.clone()).ok())
        .filter(|a| asks::is_ask_id(&a.id))
    {
        crate::machines::setup::note_asked(state, machine, kept);
    }
    audit(state, "machine.sync.asked", &pair, json!({}));
    super::publish(state);
    Ok(())
}

async fn turn_off(state: &AppState, project: &str, machine: &str) -> ApiResult<()> {
    let folder = paths::clean_folder(project, PathForm::here()).map_err(|why| {
        ApiError::unprocessable("sync_bad_project", format!("That project: {why}."))
    })?;
    // By the folder as asked, or the repository's top it is in.
    let top = repo_here(state, &folder).await.map(|h| h.top).ok();
    let found = state.machines.sync.read(|f| {
        f.pairs
            .iter()
            .find(|p| {
                p.machine == machine
                    && (paths::same_path(&p.folder, &folder)
                        || top
                            .as_deref()
                            .is_some_and(|t| paths::same_path(&p.folder, t)))
            })
            .cloned()
    });
    let Some(p) = found else {
        return Ok(());
    };
    forget(state, &p.id);
    audit(state, "machine.sync.off", &p, json!({ "by": "here" }));
    // Told there in the background (an offline computer learns when it
    // next asks: this one no longer knows the pair).
    if p.peer_on || p.asked_at.is_some() {
        let (st, id, mac) = (state.clone(), p.id.clone(), p.machine.clone());
        tokio::spawn(async move {
            let _ = transport::request(
                &st,
                &mac,
                "POST",
                &format!("/peer/sync/{id}/off"),
                Some(json!({ "reason": "off" })),
                transport::REQUEST_TIMEOUT,
            )
            .await;
        });
    }
    Ok(())
}

/// Drops a pair here (its watcher, its step files); its kept refs stay in
/// the repository, and the files git ignores its steps replaced stay in
/// its folder of the data dir.
pub(crate) fn forget(state: &AppState, id: &str) {
    state
        .machines
        .sync
        .write(|f| f.pairs.retain(|p| p.id != id));
    super::watch::refresh(state);
    if let Some(d) = state.machines.sync.dir_of(id) {
        for part in ["in", "out"] {
            let _ = std::fs::remove_dir_all(d.join(part));
        }
    }
    super::publish(state);
}

/// A paired computer was revoked: its pairs go.
pub fn forget_machine(state: &AppState, machine: &str) {
    let ids: Vec<String> = state.machines.sync.read(|f| {
        f.pairs
            .iter()
            .filter(|p| p.machine == machine)
            .map(|p| p.id.clone())
            .collect()
    });
    for id in ids {
        forget(state, &id);
    }
}

/// Machines turned off here: every pair says so; turned on, the app looks
/// again.
pub fn machines_turned(state: &AppState, on: bool) {
    let ids: Vec<String> = state.machines.sync.read(|f| {
        f.pairs
            .iter()
            .filter(|p| p.active())
            .map(|p| p.id.clone())
            .collect()
    });
    if on {
        super::step::on_peer_back_all(state);
        return;
    }
    let here = m::this_name(state);
    for id in ids {
        super::set_status(
            state,
            &id,
            Status::paused(
                super::Reason::MachinesOff,
                format!("Machines is off on {here}."),
            ),
        );
    }
}

// ─── the asked computer ────────────────────────────────────────────────────────

/// What an ask of kind `sync` holds once checked here: this computer's
/// folder, the asking computer's, and the project's name.
pub struct Checked {
    pub id: String,
    pub folder: String,
    pub there: String,
    pub title: String,
    pub origin: Option<String>,
}

/// Checks another computer's ask to sync against this computer's own
/// state before it is kept (`setup::asks::receive`): Machines on here, a
/// root mapping its folder to exactly the one it names here, that folder a
/// repository's top of the same project, `move` both ways, no other
/// computer syncing it, not asked here already.
pub async fn check_ask(state: &AppState, from: &str, a: &SyncAsk) -> ApiResult<Checked> {
    m::serving(state)?;
    if !is_sync_id(&a.id) {
        return Err(ApiError::bad_request("Malformed request."));
    }
    let name = m::peer_name(state, from);
    if !paths::is_absolute_anywhere(&a.folder)
        || a.folder.len() > 4096
        || a.there.len() > 4096
        || a.commits.len() > 20
        || a.commits.iter().any(|c| !code::is_oid(c))
    {
        return Err(ApiError::bad_request("Malformed request."));
    }
    // The folder here, as this computer's own roots map it.
    let mapped = roots::map_here(state, from, &a.folder).ok_or_else(|| {
        ApiError::unprocessable(
            "sync_no_folder",
            format!("No shared folder maps {name}'s project to this computer."),
        )
    })?;
    if !paths::same_path(&mapped, &a.there) {
        return Err(ApiError::unprocessable(
            "sync_no_folder",
            "The shared folders differ between the two computers: check them in Machines › Set up.",
        ));
    }
    let here = repo_here(state, &mapped).await.map_err(|_| {
        ApiError::unprocessable(
            "sync_not_here",
            format!(
                "{mapped} is not a git repository on this computer: clone the project there first."
            ),
        )
    })?;
    if !paths::same_path(&here.top, &mapped) {
        return Err(ApiError::unprocessable(
            "sync_not_here",
            format!("{mapped} is inside another git repository on this computer, not its top."),
        ));
    }
    // The same project: the same origin, or a commit in common.
    let same_origin = match (
        a.origin.as_deref().and_then(folders::origin_key),
        here.origin.as_deref().and_then(folders::origin_key),
    ) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    };
    if !same_origin {
        let (g, f, list) = (
            super::step::git(state),
            PathBuf::from(&here.top),
            a.commits.clone(),
        );
        let known = tokio::task::spawn_blocking(move || code::known_commits(&g, &f, &list))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        if known.is_empty() {
            return Err(ApiError::conflict(
                "sync_not_same_project",
                format!(
                    "{} on this computer is another project than {name}'s: they share no origin and no commit.",
                    here.top
                ),
            ));
        }
    }
    grants_both_ways(state, from)?;
    if let Some(other) = synced_elsewhere(state, &here.top, from) {
        return Err(ApiError::conflict(
            "sync_elsewhere",
            format!(
                "{} is kept in sync with {other} here already.",
                name_of(&here.top)
            ),
        ));
    }
    let asked_here = state.machines.sync.read(|f| {
        f.pairs.iter().any(|p| {
            p.machine == from
                && p.here_on
                && !p.peer_on
                && p.asked_at.is_some()
                && paths::same_path(&p.folder, &here.top)
        })
    });
    if asked_here {
        return Err(ApiError::conflict(
            "sync_asked_here",
            "This computer asked to keep it in sync already: confirm its request there.",
        ));
    }
    Ok(Checked {
        id: a.id.clone(),
        folder: here.top.clone(),
        there: a.folder.clone(),
        title: name_of(&here.top),
        origin: a.origin.clone(),
    })
}

/// The desktop here confirmed another computer's ask: it is on on both.
/// The pair is kept, the asking computer told, and the first step runs.
pub async fn accept(state: &AppState, ask: &Ask) -> Result<String, String> {
    let (Some(id), Some(folder), Some(there)) =
        (ask.sync.clone(), ask.folder.clone(), ask.there.clone())
    else {
        return Err("The request lost its project.".into());
    };
    m::using(state).map_err(|e| e.message)?;
    grants_both_ways(state, &ask.from).map_err(|e| e.message)?;
    let here = repo_here(state, &folder).await.map_err(|e| e.message)?;
    if !paths::same_path(&here.top, &folder) {
        return Err(format!(
            "{folder} is not a git repository's top here any more."
        ));
    }
    if let Some(other) = synced_elsewhere(state, &folder, &ask.from) {
        return Err(format!(
            "{} is kept in sync with {other} here already.",
            name_of(&folder)
        ));
    }
    let now = m::now();
    let pair = Pair {
        id: id.clone(),
        machine: ask.from.clone(),
        folder: here.top.clone(),
        there,
        here_on: true,
        peer_on: true,
        asked_at: None,
        last: None,
        status: Status::Syncing { since: now },
        conflict: None,
        kept: Vec::new(),
        reset_ok: false,
        created_at: now,
        updated_at: now,
        extra: Default::default(),
    };
    state.machines.sync.write(|f| {
        f.pairs.retain(|p| {
            p.id != id && !(p.machine == ask.from && paths::same_path(&p.folder, &here.top))
        });
        f.pairs.push(pair.clone());
    });
    super::watch::refresh(state);
    super::publish(state);
    audit(
        state,
        "machine.sync.on",
        &pair,
        json!({ "confirmedHere": true }),
    );
    let asker = m::peer_name(state, &ask.from);
    let told = transport::request(
        state,
        &ask.from,
        "POST",
        &format!("/peer/sync/{id}/confirm"),
        Some(json!({ "folder": here.top })),
        transport::REQUEST_TIMEOUT,
    )
    .await;
    if matches!(told, Ok((404, _))) {
        // It turned it off before this computer confirmed.
        forget(state, &id);
        return Err(format!("{asker} turned it off before it was confirmed."));
    }
    if !matches!(told, Ok((200, _))) {
        update(state, &id, |q| {
            q.status = Status::paused(
                super::Reason::Offline,
                format!("{asker} was not told yet: it is on as soon as it is back."),
            )
        });
        return Ok(format!(
            "On. {asker} can't be reached right now: the first sync runs when it is back."
        ));
    }
    let outcome = super::step::run(state, &id, Trigger::Now).await;
    let text = match outcome {
        super::step::Outcome::Sent
        | super::step::Outcome::Received
        | super::step::Outcome::Agreed
        | super::step::Outcome::Unchanged => format!("Kept in sync with {asker}."),
        super::step::Outcome::Conflict => format!(
            "On. {} differs on both computers: choose which one's to keep on the Machines page.",
            name_of(&folder)
        ),
        _ => match state.machines.sync.pair(&id).map(|p| p.status) {
            Some(Status::Paused { detail, .. }) => format!("On. The first sync waits: {detail}"),
            _ => format!("Kept in sync with {asker}."),
        },
    };
    Ok(text)
}

/// The desktop here said *Not now*: the asking computer is told.
pub fn declined(state: &AppState, ask: &Ask) {
    let Some(id) = ask.sync.clone() else { return };
    let (st, mac) = (state.clone(), ask.from.clone());
    tokio::spawn(async move {
        let _ = transport::request(
            &st,
            &mac,
            "POST",
            &format!("/peer/sync/{id}/off"),
            Some(json!({ "reason": "declined" })),
            transport::REQUEST_TIMEOUT,
        )
        .await;
    });
}
