//! The computer that asks (docs/PLAN-MACHINES.md Phase 4 decision 6): a
//! session's `machine_task` — the calling session's project, the other
//! computer asked where the project is there and which account its session
//! would run as, an approval here (always: no standing grant covers a
//! task), the code snapshotted and sent, the task started there and
//! followed (`follow`: its state, long-polled, into this computer's record
//! of it) — and what comes back: each result fetched as a bundle and made
//! the branch `mother-claude/<machine>/<task>` here (never checked out,
//! never merged, moved only forward), with its session's summary.
//!
//! Every request carries `X-MC-Acting: machine` with the right this
//! computer judged: a permission mode without approvals was allowed here
//! (only someone with the dangerous right can) — the other computer
//! intersects it with its own `dangerous` grant for this one.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::target::AccountAsk;
use super::{
    audit, clean_text, clip, first_line, is_task_id, mirrored, publish, Task, TaskRecord,
    TaskState, IN_DIR, MAX_SUMMARY, MAX_TASK,
};
use crate::claude::model::{GrantOffer, PendingItem, PendingKind, PendingSource, RuleKind};
use crate::claude::owned;
use crate::machines::chunks::{self, PutError};
use crate::machines::code::{self, TaskBranchError, TaskTip};
use crate::machines::grants::Grant;
use crate::machines::store::PeerRecord;
use crate::machines::work::source::{
    acting, asker, duration_text, files_of, find_machine, tool_err, Asker, APPROVAL_TIMEOUT,
};
use crate::machines::work::target::FileSpec;
use crate::machines::work::{Direction, WAIT_SECS};
use crate::machines::{self as m, moves};
use crate::mcp::{Caller, ToolError};
use crate::pending::{self, WaitEnd};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// A plain request's time.
const CALL: Duration = Duration::from_secs(30);
/// How long the other computer may take to take the code in and start the
/// session.
const START: Duration = Duration::from_secs(10 * 60);
/// A status request waits this long for news there, and is given this
/// much longer here.
const POLL_WAIT: u64 = 20;
const POLL: Duration = Duration::from_secs(POLL_WAIT + 25);
/// A task that ended this long ago is looked at every few minutes only
/// (it runs again when someone sends its session a message there).
const IDLE_AFTER: Duration = Duration::from_secs(60 * 60);
const IDLE_PAUSE: Duration = Duration::from_secs(5 * 60);
/// The folder (in the task's own) the outgoing snapshot is written to.
const OUT_DIR: &str = ".out";

/// A JSON call to `machine` about a task, its refusal in its own words.
async fn call(
    state: &AppState,
    machine: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
    dangerous: bool,
    timeout: Duration,
) -> Result<Value, ToolError> {
    let name = m::peer_name(state, machine);
    let acting = acting(state, dangerous);
    match moves::call(state, machine, method, path, body, &acting, timeout).await {
        Ok((s, v)) if (200..300).contains(&s) => Ok(v),
        Ok((404, v)) if v.pointer("/error/code").is_none() => Err(ToolError::new(
            "update_needed",
            format!(
                "Update Mother Claude on {name}: it does not take tasks from other computers yet."
            ),
        )),
        Ok((s, v)) => Err(tool_err(moves::refusal(s, &v, &name))),
        Err(e) => Err(ToolError::new(
            "unreachable",
            format!("{name} can't be reached right now: {e}"),
        )),
    }
}

/// Whether this computer may hand `peer` a task now, or why not.
pub fn why_not(state: &AppState, peer: &PeerRecord) -> Option<String> {
    let name = peer.display_name();
    if !state.machines.enabled() {
        return Some("Computers is off on this computer: turn it on in Computers first.".into());
    }
    if !peer.capabilities.is_empty() && !peer.capabilities.iter().any(|c| c == super::CAPABILITY) {
        return Some(format!(
            "Update Mother Claude on {name}: it does not take tasks from other computers yet."
        ));
    }
    crate::machines::work::source::why_not(state, peer)
}

// ─── asking ────────────────────────────────────────────────────────────────────

/// What `machine_task` asks.
#[derive(Clone, Debug, Default)]
pub struct Ask {
    pub machine: String,
    pub task: String,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// The other computer's answer to *where would it work*.
#[derive(Clone, Debug)]
struct There {
    checkout: String,
    os: String,
    account: String,
    haves: Vec<String>,
}

fn there_of(v: &Value) -> Option<There> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Some(There {
        checkout: s("checkout").filter(|c| !c.is_empty())?,
        os: s("os").unwrap_or_default(),
        account: s("account").map(|a| clip(&a, 200)).unwrap_or_default(),
        haves: v
            .get("haves")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .filter(|h| code::is_oid(h))
                    .take(code::MAX_HAVES)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The approval card (decision 2): the computer, its folder, the task's
/// first line — the whole task below it — and what a permission could
/// remember of the answer (tasks from this session, this project or any,
/// on that computer; never one that would run without asking: a task is a
/// free-form prompt, so no pattern narrows it).
async fn approve(
    state: &AppState,
    asker: &Asker,
    peer: &PeerRecord,
    there: &There,
    ask: &Ask,
    mode: &str,
    dangerous: bool,
) -> Result<Option<String>, ToolError> {
    let name = peer.display_name().to_string();
    let project = asker.top.display().to_string();
    let first = first_line(&ask.task);
    let item = PendingItem {
        session_id: asker.session.clone(),
        kind: PendingKind::Permission,
        source: PendingSource::Machine,
        tool_name: Some("machine_task".into()),
        title: format!("Hand a task to {name}: {}", clip(&first, 120)),
        detail: Some(format!(
            "In its own worktree of {} on {name}, at this session's code.",
            there.checkout
        )),
        input: Some(json!({
            "kind": "task",
            "machine": peer.id,
            "machineName": name,
            "os": there.os,
            "checkout": there.checkout,
            "cwd": asker.rel,
            "task": ask.task,
            "firstLine": first,
            "permissionMode": mode,
            "model": ask.model,
            "effort": ask.effort,
            "account": there.account,
            "project": project,
            "projectName": crate::machines::setup::folders::name_of(&project),
        })),
        suggestions: Vec::new(),
        dangerous,
        dangerous_reason: dangerous.then(|| {
            format!(
                "Its session on {name} would run in {mode}: it would edit files and run commands \
                 there without asking anyone."
            )
        }),
        grant: Some(GrantOffer {
            kind: RuleKind::Task,
            tool: None,
            patterns: None,
            suggested: Vec::new(),
            session: asker.session.clone(),
            session_name: asker.title.clone(),
            project: Some(project.clone()),
            project_name: Some(crate::machines::setup::folders::name_of(&project)),
            machine: Some(peer.id.clone()),
            machine_name: Some(name.clone()),
            blocked: dangerous.then(|| {
                "A task that would run without asking always asks: this one can't be remembered."
                    .to_string()
            }),
        }),
        ..Default::default()
    };
    let (guard, rx) = pending::raise(state, item).await;
    let id = guard.id().to_string();
    match pending::wait_end(rx, APPROVAL_TIMEOUT).await {
        WaitEnd::Decided(d) if d.is_allow() => Ok(state
            .pending
            .resolution(&id)
            .and_then(|r| r.by)
            .and_then(|b| b.name)),
        WaitEnd::Decided(d) => {
            let note = d
                .message
                .filter(|m| !m.trim().is_empty())
                .map(|m| format!(" Their note: {}", clip(m.trim(), 500)))
                .unwrap_or_default();
            Err(ToolError::new(
                "denied",
                format!(
                    "The user denied handing this task to {name}. Don't retry it; ask the user \
                     what to do instead.{note}"
                ),
            ))
        }
        WaitEnd::TimedOut => {
            guard.expire();
            Err(ToolError::new(
                "approval_timeout",
                format!(
                    "Nobody approved handing this task to {name} within {} minutes. Ask the user \
                     to approve it in Mother Claude, then try again.",
                    APPROVAL_TIMEOUT.as_secs() / 60
                ),
            ))
        }
        WaitEnd::Closed => Err(ToolError::new(
            "cancelled",
            "The approval was withdrawn. Try again if it is still needed.",
        )),
    }
}

/// The asking session's account, as the other computer matches it.
fn account_ask(state: &AppState, session: &str) -> AccountAsk {
    let id = crate::claude::summaries::acting_account(state, session);
    AccountAsk {
        name: state.accounts.get(&id).map(|a| a.name).unwrap_or_default(),
        is_default: id == crate::claude::accounts::DEFAULT_ID,
    }
}

/// `machine_task`: hands `ask.task` to an owned Claude Code session on
/// `ask.machine`, in its own worktree at the calling session's code; waits
/// up to [`WAIT_SECS`] for its turn to end and answers what came back (the
/// branch, what it changed, its summary) — or *still running*.
pub async fn hand_over(state: &AppState, caller: &Caller, ask: Ask) -> Result<String, ToolError> {
    m::using(state).map_err(tool_err)?;
    let text = ask.task.trim().to_string();
    if text.is_empty() || ask.task.len() > MAX_TASK || text.contains('\0') {
        return Err(ToolError::invalid(format!(
            "The task is text of at most {} KiB: what the session there should do.",
            MAX_TASK / 1024
        )));
    }
    let mode = ask
        .permission_mode
        .clone()
        .unwrap_or_else(|| "default".into());
    if !owned::PERMISSION_MODES.contains(&mode.as_str()) {
        return Err(ToolError::invalid(format!(
            "permissionMode is one of {}.",
            owned::PERMISSION_MODES.join(", ")
        )));
    }
    let model = match ask
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        Some(m) => Some(owned::validate_model(m).map_err(|e| ToolError::invalid(e.message))?),
        None => None,
    };
    let effort = ask
        .effort
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_string);
    if effort
        .as_deref()
        .is_some_and(|e| !owned::EFFORT_LEVELS.contains(&e))
    {
        return Err(ToolError::invalid(format!(
            "effort is one of {}.",
            owned::EFFORT_LEVELS.join(", ")
        )));
    }
    let dangerous = owned::is_dangerous_mode(Some(&mode));
    let peer = find_machine(state, &ask.machine)?;
    if let Some(why) = why_not(state, &peer) {
        return Err(ToolError::new("not_allowed", why));
    }
    let name = peer.display_name().to_string();
    if dangerous {
        let lan = state.machines.link(&peer.id).path == Some(m::transport::PathKind::Lan);
        if peer
            .their_grants
            .is_some_and(|g| !g.allows(Grant::Dangerous, lan))
        {
            return Err(ToolError::new(
                "not_allowed",
                format!(
                    "A session that runs in {mode} on {name} needs {name}'s Dangerous permission \
                     for this computer: on {name}, open Computers, press Change on {this}'s card \
                     and tick Dangerous — or hand the task over in the default mode.",
                    this = m::this_name(state)
                ),
            ));
        }
    }
    let asker = asker(state, caller).await?;
    let project = asker.top.display().to_string();
    let account = account_ask(state, &asker.session);
    let v = call(
        state,
        &peer.id,
        "POST",
        "/peer/work/tasks/prepare",
        Some(&json!({ "project": project, "account": account })),
        false,
        CALL,
    )
    .await?;
    let there = there_of(&v)
        .ok_or_else(|| ToolError::new("machine_refused", format!("{name} answered oddly.")))?;
    let ask = Ask {
        task: text,
        model,
        effort,
        ..ask
    };
    // A permission covers it (never one that would run without asking).
    let top = asker.top.display().to_string();
    let covered = if dangerous {
        None
    } else {
        state.rules.covering(&crate::pending::rules::Ask {
            kind: RuleKind::Task,
            tool: None,
            session: &asker.session,
            folder: Some(&top),
            project: Some(&top),
            machine: Some(&peer.id),
            subject: None,
        })
    };
    let approved_by = match covered {
        Some(rule) => Some(format!(
            "your permission ({})",
            crate::pending::rules::describe(&rule)
        )),
        None => approve(state, &asker, &peer, &there, &ask, &mode, dangerous).await?,
    };
    // The task, here.
    let id = super::new_task_id();
    let branch = code::task_branch(&name, &id);
    let first = first_line(&ask.task);
    let now = m::now();
    let record = TaskRecord {
        id: id.clone(),
        direction: Direction::Out,
        machine: peer.id.clone(),
        machine_name: name.clone(),
        os: there.os.clone(),
        session: Some(asker.session.clone()),
        session_title: asker.title.as_deref().map(|t| clean_text(t, 300)),
        project: project.clone(),
        project_name: crate::machines::setup::folders::name_of(&project),
        cwd: asker.rel.clone(),
        checkout: there.checkout.clone(),
        folder: String::new(),
        task: ask.task.clone(),
        first_line_sha256: crate::claude::secrets::sha256_hex(first.as_bytes()),
        first_line: first,
        permission_mode: mode.clone(),
        model: ask.model.clone(),
        effort: ask.effort.clone(),
        account_name: Some(there.account.clone()).filter(|a| !a.is_empty()),
        task_session: None,
        state: TaskState::Sending,
        error: None,
        created_at: now,
        started_at: None,
        ended_at: None,
        turns: 0,
        summary: None,
        background: false,
        base: None,
        base_tree: None,
        branch,
        tip: None,
        commits: 0,
        files: 0,
        uncommitted: false,
        results: 0,
        landed: 0,
        branch_note: None,
        approved_by,
        dangerous,
        unreachable: false,
        gone_there: false,
        via_lan: None,
        idle_stopped_at: None,
        updated_at: now,
        account: None,
        haves: Vec::new(),
        last: None,
        result_files: Vec::new(),
        their_state: None,
    };
    super::prune(state);
    let task = state
        .machines
        .work
        .tasks
        .insert(record)
        .ok_or_else(|| ToolError::new("internal", "A task with that id exists."))?;
    state.machines.work.tasks.save(&task);
    audit(state, "machine.task.asked", &task.record());
    publish(state);
    if let Err(e) = send(state, &task, &asker, &there, &account).await {
        let now = m::now();
        task.update(|r| {
            r.state = TaskState::Error;
            r.error = Some(e.message.clone());
            r.ended_at = Some(now);
        });
        state.machines.work.tasks.save(&task);
        audit(state, "machine.task", &task.record());
        publish(state);
        return Err(e);
    }
    follow(state, task.clone());
    let r = task.wait_end(WAIT_SECS).await;
    Ok(describe(&r, WAIT_SECS))
}

/// The snapshot taken, sent, and the task started there.
async fn send(
    state: &AppState,
    task: &Arc<Task>,
    asker: &Asker,
    there: &There,
    account: &AccountAsk,
) -> Result<(), ToolError> {
    let r = task.record();
    let dir = state
        .machines
        .work
        .tasks
        .task_dir(&r.id)
        .map(|d| d.join(OUT_DIR))
        .ok_or_else(|| ToolError::new("internal", "This build has no data folder."))?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| ToolError::new("internal", e.to_string()))?;
    crate::claude::transfer::private_dir(&dir);
    let this = m::this_name(state);
    let mut req =
        code::SnapshotRequest::new(&r.id, m::now().max(1) as u64, &this, dir.join("code"));
    req.haves = there.haves.clone();
    req.chain = false;
    req.label = Some(format!(
        "{} handed over from {}",
        r.id,
        code::safe_name(&this)
    ));
    let git = state.machines.moves.git();
    let (g, top) = (git.clone(), asker.top.clone());
    let snapped = tokio::task::spawn_blocking(move || {
        let meta = code::snapshot(&g, &top, &req)?;
        // This computer's own commits: its results leave them out.
        let ours = code::haves(&g, &top, None)?;
        Ok::<_, code::CodeError>((meta, ours.haves))
    })
    .await
    .map_err(|e| ToolError::new("internal", e.to_string()))?
    .map_err(|e| {
        ToolError::new(
            "snapshot_failed",
            format!("The code could not be snapshotted: {e}"),
        )
    })?;
    let (meta, ours) = snapped;
    task.update(|r| {
        r.base = Some(meta.snap.clone());
        r.base_tree = Some(meta.wt_tree.clone());
    });
    state.machines.work.tasks.save(task);
    let d = dir.clone();
    let files = tokio::task::spawn_blocking(move || files_of(&d, &meta))
        .await
        .map_err(|e| ToolError::new("internal", e.to_string()))?
        .map_err(|e| ToolError::new("snapshot_failed", e))?;
    let begun = call(
        state,
        &r.machine,
        "POST",
        "/peer/work/tasks",
        Some(&json!({
            "task": r.id,
            "project": r.project,
            "projectName": r.project_name,
            "cwd": r.cwd,
            "prompt": r.task,
            "permissionMode": r.permission_mode,
            "model": r.model,
            "effort": r.effort,
            "account": account,
            "branch": r.branch,
            "session": r.session,
            "sessionTitle": r.session_title,
            "haves": ours,
            "files": files,
        })),
        r.dangerous,
        CALL,
    )
    .await?;
    let have = |name: &str| -> u64 {
        begun
            .get("files")
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|f| f["name"] == name))
            .and_then(|f| f["have"].as_u64())
            .unwrap_or(0)
    };
    let acting = acting(state, r.dangerous);
    for f in &files {
        let name = f["name"].as_str().unwrap_or_default().to_string();
        let size = f["size"].as_u64().unwrap_or(0);
        let Some(path) = moves::file_path(&dir, &name) else {
            continue;
        };
        let mut offset = have(&name).min(size);
        while offset < size {
            let n = ((size - offset) as usize).min(chunks::CHUNK);
            let p = path.clone();
            let bytes = tokio::task::spawn_blocking(move || chunks::read_chunk(&p, offset, n))
                .await
                .map_err(|e| ToolError::new("internal", e.to_string()))?
                .map_err(|e| ToolError::new("internal", e.to_string()))?;
            let url = format!("/peer/work/tasks/{}/files/{name}?offset={offset}", r.id);
            offset = match chunks::put(state, &r.machine, &url, bytes, &acting).await {
                Ok(s) => s,
                Err(PutError::Offline(e)) => {
                    return Err(ToolError::new(
                        "unreachable",
                        format!("{} can't be reached right now: {e}", r.machine_name),
                    ))
                }
                Err(PutError::Odd) => {
                    return Err(ToolError::new(
                        "machine_refused",
                        format!("{} answered oddly.", r.machine_name),
                    ))
                }
                Err(PutError::Refused(status, v)) => {
                    return Err(tool_err(moves::refusal(status, &v, &r.machine_name)))
                }
            };
        }
    }
    let started = call(
        state,
        &r.machine,
        "POST",
        &format!("/peer/work/tasks/{}/start", r.id),
        None,
        r.dangerous,
        START,
    )
    .await?;
    let _ = std::fs::remove_dir_all(&dir);
    take_status(state, task, &started);
    Ok(())
}

// ─── following it ──────────────────────────────────────────────────────────────

fn num(v: &Value, k: &str) -> Option<u32> {
    v.get(k)
        .and_then(Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

/// What the other computer said of the task, into this computer's record.
fn take_status(state: &AppState, t: &Task, v: &Value) {
    let Some(s) = v.get("task") else { return };
    let theirs: Option<TaskState> = s
        .get("state")
        .and_then(|x| serde_json::from_value(x.clone()).ok());
    let before = t.record();
    let text = |k: &str, n: usize| {
        s.get(k)
            .and_then(Value::as_str)
            .map(|x| clean_text(x, n))
            .filter(|x| !x.is_empty())
    };
    t.update(|r| {
        r.unreachable = false;
        if let Some(n) = num(s, "results") {
            r.results = n;
        }
        if let Some(ts) = theirs {
            r.their_state = Some(ts);
            if !r.state.is_final() {
                r.state = mirrored(ts, r.results, r.landed);
            }
        }
        if let Some(n) = num(s, "turns") {
            r.turns = n;
        }
        if let Some(x) = text("summary", MAX_SUMMARY) {
            r.summary = Some(x);
        }
        r.background = s
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        r.error = text("error", 2000);
        if let Some(x) = s.get("startedAt").and_then(Value::as_i64) {
            r.started_at = Some(x);
        }
        r.ended_at = s
            .get("endedAt")
            .and_then(Value::as_i64)
            .or(r.ended_at.filter(|_| r.state.ended()));
        if let Some(x) = s
            .get("taskSession")
            .and_then(Value::as_str)
            .filter(|x| crate::claude::ids::is_session_id(x))
        {
            r.task_session = Some(x.to_string());
        }
        if let Some(x) = text("folder", 4096) {
            r.folder = x;
        }
        if let Some(x) = text("checkout", 4096) {
            r.checkout = x;
        }
        if let Some(x) = text("accountName", 200) {
            r.account_name = Some(x);
        }
        r.idle_stopped_at = s.get("idleStoppedAt").and_then(Value::as_i64);
    });
    let after = t.record();
    if after.state != before.state
        || after.results != before.results
        || after.turns != before.turns
        || after.idle_stopped_at != before.idle_stopped_at
    {
        if after.state.ended() && after.ended_at.is_none() {
            let now = m::now();
            t.update(|r| r.ended_at = Some(now));
        }
        state.machines.work.tasks.save(t);
        if after.state.ended() && after.state != before.state {
            audit(state, "machine.task", &t.record());
        }
        publish(state);
    }
}

/// Follows a task there (in the background): its state into this
/// computer's record, and each new result made the branch here. A task
/// that ended is still looked at now and then: a message to its session
/// makes it run again.
pub fn follow(state: &AppState, t: Arc<Task>) {
    let st = state.clone();
    tokio::spawn(async move { follower(st, t).await });
}

async fn follower(state: AppState, t: Arc<Task>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        if state.machines.work.tasks.get(&t.id).is_none() {
            break;
        }
        let r = t.record();
        if r.state.is_final() || r.gone_there {
            break;
        }
        if state.machines.store_read(|f| f.peer(&r.machine).is_none()) {
            // Unpaired: nothing more comes from there.
            let now = m::now();
            t.update(|r| {
                if r.state.ended() {
                    r.gone_there = true;
                } else {
                    r.state = TaskState::Lost;
                    r.error = Some(format!(
                        "{} is no longer paired with this computer.",
                        r.machine_name
                    ));
                    r.ended_at = Some(now);
                }
            });
            state.machines.work.tasks.save(&t);
            publish(&state);
            break;
        }
        // A result there not here yet: taken first (when it can't be, the
        // status below says why — out of reach, or gone there).
        if r.results > r.landed && land(&state, &t).await.is_ok() {
            backoff = Duration::from_secs(1);
            continue;
        }
        let r = t.record();
        let theirs = r.their_state.unwrap_or(r.state);
        let path = format!(
            "/peer/work/tasks/{}?wait={POLL_WAIT}&state={}&results={}&landed={}&idle={}",
            r.id,
            theirs.as_str(),
            r.results,
            r.landed,
            r.idle_stopped_at.is_some()
        );
        let acting = acting(&state, false);
        match moves::call(&state, &r.machine, "GET", &path, None, &acting, POLL).await {
            Ok((200, v)) => {
                take_status(&state, &t, &v);
                backoff = Duration::from_secs(1);
                // A task that ended a while ago is looked at less often.
                let r = t.record();
                let idle = r
                    .ended_at
                    .is_some_and(|e| m::now() - e > IDLE_AFTER.as_millis() as i64);
                if r.state.ended() && r.results <= r.landed && idle {
                    tokio::time::sleep(IDLE_PAUSE).await;
                }
                continue;
            }
            Ok((404, _)) => {
                let now = m::now();
                t.update(|r| {
                    if r.state.ended() {
                        r.gone_there = true;
                    } else {
                        r.state = TaskState::Lost;
                        r.error = Some(format!("{} no longer knows this task.", r.machine_name));
                        r.ended_at = Some(now);
                    }
                });
                state.machines.work.tasks.save(&t);
                audit(&state, "machine.task", &t.record());
                publish(&state);
                break;
            }
            Ok((_, v)) => {
                // Machines off there, its Run permission taken back: said,
                // and asked again later (it may come back).
                let why = v
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(|s| clip(s, 500));
                unreachable(&state, &t, why);
            }
            Err(_) => unreachable(&state, &t, None),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

fn unreachable(state: &AppState, t: &Task, why: Option<String>) {
    let was = t.peek(|r| r.unreachable);
    if !was {
        t.update(|r| {
            r.unreachable = true;
            if why.is_some() && !r.state.ended() {
                r.error = why;
            }
        });
        publish(state);
    }
}

/// The newest result there, fetched and made the branch here. `Err(())`:
/// try again later (the other computer is out of reach, or a newer result
/// replaced it on the way).
async fn land(state: &AppState, t: &Arc<Task>) -> Result<(), ()> {
    let _one = t.work.lock().await;
    let r = t.record();
    let acting = acting(state, false);
    let info = match moves::call(
        state,
        &r.machine,
        "GET",
        &format!("/peer/work/tasks/{}/result", r.id),
        None,
        &acting,
        CALL,
    )
    .await
    {
        Ok((200, v)) => v,
        Ok((404, v)) if v.pointer("/error/code").and_then(Value::as_str) == Some("no_result") => {
            t.update(|r| r.landed = r.results);
            state.machines.work.tasks.save(t);
            return Ok(());
        }
        _ => return Err(()),
    };
    let seq = num(&info, "seq").unwrap_or(0);
    let last: Option<TaskTip> = info
        .get("last")
        .and_then(|l| serde_json::from_value(l.clone()).ok())
        .filter(|l: &TaskTip| l.validate().is_ok());
    let files: Vec<FileSpec> = info
        .get("files")
        .and_then(|f| serde_json::from_value(f.clone()).ok())
        .unwrap_or_default();
    let (Some(last), Some(dir), Some(base), Some(branch)) = (
        last,
        state.machines.work.tasks.task_dir(&r.id),
        r.base.clone(),
        r.branch.clone(),
    ) else {
        note_landed(
            state,
            t,
            seq.max(r.results),
            None,
            Some("The other computer's result could not be read, so no branch was made.".into()),
        );
        return Ok(());
    };
    let in_dir = dir.join(IN_DIR).join(seq.to_string());
    let _ = std::fs::remove_dir_all(&in_dir);
    if std::fs::create_dir_all(&in_dir).is_err() {
        return Err(());
    }
    crate::claude::transfer::private_dir(&in_dir);
    for f in &files {
        let ok_name = moves::file_path(&in_dir, &f.name).is_some()
            && f.name != crate::claude::backup::session_pack::FILE
            && f.name != moves::ignored::FILE;
        if !ok_name || f.sha256.len() != 64 {
            note_landed(
                state,
                t,
                seq,
                None,
                Some(
                    "The other computer's result named a file this computer does not take.".into(),
                ),
            );
            let _ = std::fs::remove_dir_all(&in_dir);
            return Ok(());
        }
        if fetch_file(state, &r, seq, f, &in_dir).await.is_err() {
            let _ = std::fs::remove_dir_all(&in_dir);
            return Err(());
        }
    }
    let git = state.machines.moves.git();
    let (top, code_dir, id, b, tip) = (
        PathBuf::from(&r.project),
        in_dir.join("code"),
        r.id.clone(),
        branch.clone(),
        last.tip.clone(),
    );
    let landed = tokio::task::spawn_blocking(move || {
        code::land_task_branch(&git, &top, &code_dir, &id, &b, &tip, &base)
    })
    .await;
    let _ = std::fs::remove_dir_all(&in_dir);
    match landed {
        Ok(Ok((_, l))) => {
            t.update(|r| {
                r.landed = seq;
                r.tip = Some(last.tip.clone());
                r.commits = l.commits;
                r.files = l.files;
                r.uncommitted = l.uncommitted;
                r.branch_note = None;
                r.last = Some(last);
                if let Some(theirs) = r.their_state {
                    if !r.state.is_final() {
                        r.state = mirrored(theirs, r.results, r.landed);
                    }
                }
            });
            state.machines.work.tasks.save(t);
            audit(state, "machine.task.result", &t.record());
            publish(state);
        }
        Ok(Err(e @ (TaskBranchError::CheckedOut { .. } | TaskBranchError::Diverged { .. }))) => {
            let short = &last.tip[..12.min(last.tip.len())];
            note_landed(
                state,
                t,
                seq,
                Some(last.clone()),
                Some(format!(
                    "{e} Its newer result is commit {short} (`git branch <name> {short}` makes a \
                     branch of it)."
                )),
            );
        }
        Ok(Err(TaskBranchError::Failed(e))) => {
            note_landed(
                state,
                t,
                seq,
                None,
                Some(format!("Its result could not be taken here: {e}")),
            );
        }
        Err(e) => {
            note_landed(state, t, seq, None, Some(e.to_string()));
        }
    }
    Ok(())
}

/// A result taken (or given up on) without moving the branch: why.
fn note_landed(state: &AppState, t: &Task, seq: u32, last: Option<TaskTip>, note: Option<String>) {
    t.update(|r| {
        r.landed = seq.max(r.landed);
        r.branch_note = note.map(|n| clip(&n, 1000));
        if last.is_some() {
            r.last = last;
        }
        if let Some(theirs) = r.their_state {
            if !r.state.is_final() {
                r.state = mirrored(theirs, r.results, r.landed);
            }
        }
    });
    state.machines.work.tasks.save(t);
    audit(state, "machine.task.result", &t.record());
    publish(state);
}

/// One file of result `seq`, in chunks, checked whole.
async fn fetch_file(
    state: &AppState,
    r: &TaskRecord,
    seq: u32,
    f: &FileSpec,
    in_dir: &std::path::Path,
) -> Result<(), ()> {
    let path = moves::file_path(in_dir, &f.name).ok_or(())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| ())?;
    }
    let part = chunks::part_of(&path);
    let _ = std::fs::remove_file(&part);
    let acting = acting(state, false);
    let mut offset = 0u64;
    while offset < f.size {
        let url = format!(
            "/peer/work/tasks/{}/result/files/{}?seq={seq}&offset={offset}",
            r.id, f.name
        );
        let got = m::transport::request_raw(
            state,
            &r.machine,
            "GET",
            &url,
            &[(m::acting::HEADER, acting.clone())],
            axum::body::Bytes::new(),
            chunks::CHUNK_TIMEOUT,
        )
        .await;
        let bytes = match got {
            Ok((200, _, b)) if !b.is_empty() && b.len() <= chunks::CHUNK => b,
            _ => {
                let _ = std::fs::remove_file(&part);
                return Err(());
            }
        };
        let p = part.clone();
        let n = bytes.len() as u64;
        if offset + n > f.size {
            let _ = std::fs::remove_file(&part);
            return Err(());
        }
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&p)?;
            file.write_all(&bytes)
        })
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
        offset += n;
    }
    let (d, size, sha) = (path.clone(), f.size, f.sha256.clone());
    tokio::task::spawn_blocking(move || chunks::finish(&d, size, &sha))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

// ─── what comes back ───────────────────────────────────────────────────────────

/// A task as the model reads it: where, how it stands, and — once it
/// ended — the branch, what it changed, how to look, and its summary.
pub fn describe(r: &TaskRecord, waited_secs: u64) -> String {
    let name = &r.machine_name;
    let mut out = format!(
        "{} on {name}: \"{}\" — in its own worktree of {} there, at this session's code.\n",
        r.id,
        clip(&r.first_line, 120),
        if r.checkout.is_empty() {
            "the project"
        } else {
            r.checkout.as_str()
        }
    );
    if !r.state.ended() {
        let now = match r.state {
            TaskState::Sending => "Its code is still on its way there.".to_string(),
            TaskState::Starting => "Its worktree is being made and its session started.".into(),
            TaskState::Waiting => format!(
                "Its session waits for an answer on {name} (an approval or a question): the \
                 user answers it in Mother Claude, where the Inbox shows {name}'s requests too."
            ),
            TaskState::Finishing => {
                "Its session's turn ended; what it did is on its way here.".into()
            }
            _ => "Its session is still working.".into(),
        };
        out.push_str(&format!(
            "{now} (waited {} min.) Wait for it with machine_wait {{\"id\":\"{}\"}}, or stop it \
             with machine_cancel.\n",
            waited_secs / 60,
            r.id
        ));
        if r.unreachable {
            out.push_str(&format!(
                "{name} can't be reached right now; Mother Claude keeps asking.\n"
            ));
        }
        return out;
    }
    let took = match (r.started_at, r.ended_at) {
        (Some(a), Some(b)) if b >= a => format!(" after {}", duration_text((b - a) as u64)),
        _ => String::new(),
    };
    match r.state {
        TaskState::Done => out.push_str(&format!("It is done: its session's turn ended{took}.\n")),
        TaskState::Stopped => out.push_str(&format!(
            "Its session ended before it finished{took}: {}\n",
            r.error.as_deref().unwrap_or("it was stopped")
        )),
        TaskState::Cancelled => out.push_str(&format!(
            "It was cancelled{took}; its session there was stopped.\n"
        )),
        TaskState::Error => {
            out.push_str(&format!(
                "It could not start: {}\n",
                r.error.as_deref().unwrap_or("an error")
            ));
            return out;
        }
        TaskState::Lost => {
            out.push_str(&format!(
                "Mother Claude lost track of it: {}\n",
                r.error
                    .as_deref()
                    .unwrap_or("the other computer did not answer")
            ));
            return out;
        }
        _ => {}
    }
    match (&r.branch, &r.tip) {
        (Some(b), Some(_)) => {
            let short = code::short_branch(b);
            let base = r
                .base
                .as_deref()
                .map(|b| &b[..12.min(b.len())])
                .unwrap_or("HEAD");
            let commits = match r.commits {
                0 => "no commit".to_string(),
                1 => "1 commit".to_string(),
                n => format!("{n} commits"),
            };
            let files = match r.files {
                1 => "1 file changed".to_string(),
                n => format!("{n} files changed"),
            };
            let left = if r.uncommitted {
                " and changes it left uncommitted"
            } else {
                ""
            };
            out.push_str(&format!(
                "Its work is the branch {short} in this repository: {commits}{left}, {files}. It \
                 is never checked out or merged here; look with\n  git log --oneline \
                 {base}..{short}\n  git diff {base}...{short}\n({base} is this session's code as \
                 it was handed over, uncommitted changes included.)\n"
            ));
        }
        _ if r.results == 0 => out.push_str("It changed nothing, so no branch was made.\n"),
        _ => {}
    }
    if let Some(note) = &r.branch_note {
        out.push_str(&format!("Note: {note}\n"));
    }
    if r.state == TaskState::Done {
        if let Some(e) = &r.error {
            out.push_str(&format!("Its last turn ended in an error: {e}\n"));
        }
    }
    if let Some(s) = &r.summary {
        out.push_str("Its summary (its session's last message):\n");
        out.push_str(s.trim_end());
        out.push('\n');
    }
    if r.background {
        out.push_str(
            "Its session still had background work running there: if that resumes it, a later \
             end moves the branch forward.\n",
        );
    }
    if matches!(r.state, TaskState::Done | TaskState::Stopped) {
        if let Some(s) = &r.task_session {
            out.push_str(&format!(
                "Its session on {name} is {s}: someone can send it another message there (or \
                 from this computer, in Mother Claude) to go on; a later end moves the branch \
                 forward. machine_wait {{\"id\":\"{}\"}} answers again then.\n",
                r.id
            ));
        }
    }
    out
}

/// The task `id` the calling session may follow: one it handed over, or
/// one handed over from its project.
pub async fn own_task(state: &AppState, caller: &Caller, id: &str) -> Result<Arc<Task>, ToolError> {
    let unknown = || ToolError::new("not_found", format!("No task {id} from this computer."));
    if !is_task_id(id) {
        return Err(unknown());
    }
    let task = state
        .machines
        .work
        .tasks
        .get(id)
        .filter(|t| t.peek(|r| r.direction == Direction::Out))
        .ok_or_else(unknown)?;
    let (session, project) = task.peek(|r| (r.session.clone(), r.project.clone()));
    if caller.session_id.is_some() && session == caller.session_id {
        return Ok(task);
    }
    match asker(state, caller).await {
        Ok(a) if crate::claude::paths::same_path(&a.top.display().to_string(), &project) => {
            Ok(task)
        }
        _ => Err(ToolError::new(
            "denied",
            format!("Task {id} belongs to another session and project."),
        )),
    }
}

/// `machine_wait` for a task: until it ends (a turn of its session that
/// ends, with its work back here), or `secs` went by.
pub async fn wait(
    state: &AppState,
    caller: &Caller,
    id: &str,
    secs: Option<u64>,
) -> Result<String, ToolError> {
    let t = own_task(state, caller, id).await?;
    let secs = secs.unwrap_or(WAIT_SECS).clamp(1, WAIT_SECS);
    let r = t.wait_end(secs).await;
    Ok(describe(&r, secs))
}

/// Cancels a task: there (the asking side), or here (the side that runs
/// it). Answers its record.
pub async fn cancel_task(state: &AppState, t: &Arc<Task>) -> ApiResult<TaskRecord> {
    let r = t.record();
    if r.state.is_final() {
        return Ok(r);
    }
    match r.direction {
        Direction::In => {
            super::target::cancel_here(state, t, None).await;
            Ok(t.record())
        }
        Direction::Out => {
            let acting = acting(state, false);
            let path = format!("/peer/work/tasks/{}/cancel", r.id);
            match moves::call(state, &r.machine, "POST", &path, None, &acting, CALL).await {
                Ok((200, v)) => {
                    take_status(state, t, &v);
                    Ok(t.record())
                }
                Ok((404, _)) if r.state == TaskState::Sending => {
                    // It never got there.
                    let now = m::now();
                    t.update(|r| {
                        r.state = TaskState::Cancelled;
                        r.ended_at = Some(now);
                    });
                    state.machines.work.tasks.save(t);
                    audit(state, "machine.task", &t.record());
                    publish(state);
                    Ok(t.record())
                }
                Ok((s, v)) => Err(moves::refusal(s, &v, &r.machine_name)),
                Err(e) => Err(ApiError::new(
                    axum::http::StatusCode::BAD_GATEWAY,
                    "machine_unreachable",
                    format!("{} can't be reached right now: {e}", r.machine_name),
                )),
            }
        }
    }
}

/// `machine_cancel` for a task.
pub async fn cancel(state: &AppState, caller: &Caller, id: &str) -> Result<String, ToolError> {
    let t = own_task(state, caller, id).await?;
    let r = cancel_task(state, &t).await.map_err(tool_err)?;
    Ok(describe(&r, 0))
}

/// Removes a task (*Remove* on its card): it must have ended. A task this
/// computer handed over is removed there too when that computer can be
/// reached (else it goes there 7 days after it ended); its branch here
/// stays.
pub async fn remove_task(state: &AppState, t: &Arc<Task>) -> ApiResult<Value> {
    let r = t.record();
    match r.direction {
        Direction::In => {
            super::target::remove_here(state, t).await?;
            Ok(json!({ "removed": true }))
        }
        Direction::Out => {
            if !r.state.ended() {
                return Err(ApiError::conflict(
                    "task_running",
                    "The task is still running: cancel it first.",
                ));
            }
            let there = if r.gone_there {
                "gone"
            } else {
                let acting = acting(state, false);
                let path = format!("/peer/work/tasks/{}", r.id);
                match moves::call(state, &r.machine, "DELETE", &path, None, &acting, CALL).await {
                    Ok((200, _)) => "removed",
                    Ok((404, _)) => "gone",
                    Ok((409, v)) => {
                        return Err(moves::refusal(409, &v, &r.machine_name));
                    }
                    _ => "unreachable",
                }
            };
            audit(state, "machine.task.remove", &r);
            state.machines.work.tasks.forget(&t.id);
            super::clear(state, t).await;
            Ok(json!({ "removed": true, "there": there }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(state: TaskState) -> TaskRecord {
        TaskRecord {
            id: "task_0123456789abcdef".into(),
            direction: Direction::Out,
            machine: "0123456789abcdef0123456789abcdef".into(),
            machine_name: "DESKTOP-PC".into(),
            os: "windows".into(),
            session: None,
            session_title: None,
            project: "/Users/me/app".into(),
            project_name: "app".into(),
            cwd: String::new(),
            checkout: r"D:\dev\app".into(),
            folder: String::new(),
            task: "Fix the Windows build".into(),
            first_line: "Fix the Windows build".into(),
            first_line_sha256: String::new(),
            permission_mode: "default".into(),
            model: None,
            effort: None,
            account_name: None,
            task_session: Some("b0ee0000-1111-4222-8333-444455556666".into()),
            state,
            error: None,
            created_at: 1,
            started_at: Some(1_000),
            ended_at: Some(61_000),
            turns: 1,
            summary: Some("Fixed the path separators.".into()),
            background: false,
            base: Some("0123456789abcdef0123456789abcdef01234567".into()),
            base_tree: None,
            branch: Some("refs/heads/mother-claude/DESKTOP-PC/task_0123456789abcdef".into()),
            tip: Some("a".repeat(40)),
            commits: 2,
            files: 3,
            uncommitted: true,
            results: 1,
            landed: 1,
            branch_note: None,
            approved_by: None,
            dangerous: false,
            unreachable: false,
            gone_there: false,
            via_lan: None,
            idle_stopped_at: None,
            updated_at: 1,
            account: None,
            haves: Vec::new(),
            last: None,
            result_files: Vec::new(),
            their_state: None,
        }
    }

    #[test]
    fn a_done_task_names_its_branch_what_it_changed_and_how_to_look() {
        let t = describe(&record(TaskState::Done), 600);
        assert!(t.contains("It is done"), "{t}");
        assert!(
            t.contains("the branch mother-claude/DESKTOP-PC/task_0123456789abcdef"),
            "{t}"
        );
        assert!(
            t.contains("2 commits and changes it left uncommitted, 3 files changed"),
            "{t}"
        );
        assert!(
            t.contains("git diff 0123456789ab...mother-claude/DESKTOP-PC/task_0123456789abcdef"),
            "{t}"
        );
        assert!(t.contains("Fixed the path separators."), "{t}");
        assert!(t.contains("b0ee0000-1111-4222-8333-444455556666"), "{t}");
    }

    #[test]
    fn no_change_means_no_branch_and_a_running_task_says_how_to_wait() {
        let mut r = record(TaskState::Done);
        r.tip = None;
        r.results = 0;
        r.landed = 0;
        let t = describe(&r, 0);
        assert!(
            t.contains("It changed nothing, so no branch was made."),
            "{t}"
        );
        let mut r = record(TaskState::Running);
        r.tip = None;
        let t = describe(&r, 600);
        assert!(t.contains("still working"), "{t}");
        assert!(
            t.contains("machine_wait {\"id\":\"task_0123456789abcdef\"}"),
            "{t}"
        );
        let t = describe(&record(TaskState::Waiting), 600);
        assert!(t.contains("waits for an answer on DESKTOP-PC"), "{t}");
        let mut r = record(TaskState::Done);
        r.branch_note =
            Some("mother-claude/DESKTOP-PC/task_0123456789abcdef is checked out".into());
        assert!(
            describe(&r, 0).contains("Note: mother-claude/"),
            "{}",
            describe(&r, 0)
        );
    }
}
