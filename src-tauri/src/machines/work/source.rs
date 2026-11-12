//! The computer that asks (docs/PLAN-MACHINES.md Phase 4): a session's
//! `machine_run` — the calling session's project (its folder's repository
//! top), the other computer asked where the project is there, an approval
//! here unless a standing grant covers the command, the code snapshotted
//! and sent (a bundle of what the other computer lacks, chained on the
//! work folder's last snapshots), the run started there and followed
//! (`follow`: its state and output's tail, long-polled, into this
//! computer's record of it) — and what comes back: the exit code, the
//! duration, the output's last 64 KiB, its pages, the files it made.
//!
//! Every request carries `X-MC-Acting: machine` with the right this
//! computer judged: a dangerous approval allowed here (only someone with
//! the dangerous right can) — the other computer intersects it with its own
//! `dangerous` grant for this one.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::grants::suggested_prefix;
use super::target::{clean_cwd, clean_glob};
use super::{
    audit, is_run_id, publish, tail_of, Direction, FetchedArtifact, Run, RunRecord, RunState,
    DEFAULT_TIMEOUT_SECS, MAX_ARTIFACT, MAX_COMMAND, MAX_TIMEOUT_SECS, TAIL, WAIT_SECS,
};
use crate::claude::model::{PendingItem, PendingKind, PendingSource, PermissionSuggestion};
use crate::machines::chunks::{self, PutError};
use crate::machines::grants::Grant;
use crate::machines::store::PeerRecord;
use crate::machines::{self as m, code, moves};
use crate::mcp::{Caller, ToolError};
use crate::pending::{self, WaitEnd};
use crate::server::error::ApiError;
use crate::state::AppState;

/// How long an approval waits for someone to answer.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// The suggestion's kind on an approval card.
pub const GRANT_KIND: &str = "machineRunGrant";
/// A plain request's time.
const CALL: Duration = Duration::from_secs(30);
/// How long the other computer may take to take the code in.
const START: Duration = Duration::from_secs(10 * 60);
/// A status request waits this long for news there, and is given this
/// much longer here.
const POLL_WAIT: u64 = 20;
const POLL: Duration = Duration::from_secs(POLL_WAIT + 25);
/// How long a run may stay out of reach past its deadline before it is
/// given up.
const LOST_AFTER_MS: i64 = 10 * 60 * 1000;

fn tool_err(e: ApiError) -> ToolError {
    ToolError::new(e.code, e.message)
}

/// The header this computer sends for its runs.
fn acting(state: &AppState, dangerous: bool) -> String {
    m::acting::machine_header(&m::this_name(state), dangerous)
}

/// A JSON call to `machine` about a run, its refusal in its own words.
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
                "Update Mother Claude on {name}: it does not run commands for other computers yet."
            ),
        )),
        Ok((s, v)) => Err(tool_err(moves::refusal(s, &v, &name))),
        Err(e) => Err(ToolError::new(
            "unreachable",
            format!("{name} can't be reached right now: {e}"),
        )),
    }
}

// ─── who asks, and where ───────────────────────────────────────────────────────

/// The calling session and its project.
#[derive(Clone, Debug)]
pub struct Asker {
    pub session: String,
    pub title: Option<String>,
    /// The project's top (its repository's).
    pub top: PathBuf,
    /// The session's folder inside it (`""`: the top).
    pub rel: String,
}

/// The calling session, its folder and that folder's repository.
pub async fn asker(state: &AppState, caller: &Caller) -> Result<Asker, ToolError> {
    let Some(session) = caller.session_id.clone() else {
        return Err(ToolError::new(
            "unknown_session",
            "Mother Claude could not tell which Claude Code session is calling, so it does not \
             know whose code to run. Try again in a moment.",
        ));
    };
    let summary = state.find_session(&session).await;
    let cwd = caller
        .cwd
        .clone()
        .filter(|c| !c.is_empty())
        .or_else(|| {
            summary
                .as_ref()
                .map(|s| s.cwd.clone())
                .filter(|c| !c.is_empty())
        })
        .ok_or_else(|| {
            ToolError::new(
                "unknown_project",
                "Mother Claude does not know this session's folder.",
            )
        })?;
    let git = state.machines.moves.git();
    let c = PathBuf::from(&cwd);
    let top = tokio::task::spawn_blocking(move || code::top_of(&git, &c))
        .await
        .map_err(|e| ToolError::new("internal", e.to_string()))?
        .map_err(|e| match e {
            code::CodeError::NotARepository(_) => ToolError::new(
                "not_a_repository",
                format!(
                    "{cwd} is not in a git repository: machine_run sends the project's code as a \
                     git snapshot, so the session must work in one."
                ),
            ),
            other => ToolError::new("unknown_project", other.to_string()),
        })?;
    let rel = relative(&PathBuf::from(&cwd), &top);
    Ok(Asker {
        session,
        title: summary.and_then(|s| s.title),
        top,
        rel,
    })
}

/// `path` below `top`, with `/` (`""` when it is the top or outside it).
fn relative(path: &Path, top: &Path) -> String {
    let p = crate::claude::paths::canonical_or_given(path);
    let t = crate::claude::paths::canonical_or_given(top);
    if !crate::claude::paths::is_within(&p, &t) {
        return String::new();
    }
    let rest: Vec<String> = p
        .components()
        .skip(t.components().count())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    rest.join("/")
}

/// A paired computer, by its id or its name (without regard to case).
pub fn find_machine(state: &AppState, wanted: &str) -> Result<PeerRecord, ToolError> {
    let w = wanted.trim();
    let found = state.machines.store_read(|f| {
        f.peer(w).cloned().or_else(|| {
            f.peers
                .iter()
                .find(|p| {
                    p.display_name().eq_ignore_ascii_case(w) || p.name.eq_ignore_ascii_case(w)
                })
                .cloned()
        })
    });
    found.ok_or_else(|| {
        let names: Vec<String> = state.machines.store_read(|f| {
            f.peers
                .iter()
                .map(|p| p.display_name().to_string())
                .collect()
        });
        ToolError::new(
            "unknown_machine",
            if names.is_empty() {
                "No other computer is paired with this one (Machines in Mother Claude).".to_string()
            } else {
                format!(
                    "No paired computer is called {w:?}. machine_list names them: {}.",
                    names.join(", ")
                )
            },
        )
    })
}

/// Whether this computer may run on `peer` now, or why not.
pub fn why_not(state: &AppState, peer: &PeerRecord) -> Option<String> {
    let name = peer.display_name();
    if !state.machines.enabled() {
        return Some(
            "Machines is off on this computer: turn it on on the Machines page first.".into(),
        );
    }
    if !peer.capabilities.is_empty() && !peer.capabilities.iter().any(|c| c == super::CAPABILITY) {
        return Some(format!(
            "Update Mother Claude on {name}: it does not run commands for other computers yet."
        ));
    }
    let link = state.machines.link(&peer.id);
    if !link.online {
        return Some(format!("{name} is offline."));
    }
    let lan = link.path == Some(m::transport::PathKind::Lan);
    if peer
        .their_grants
        .is_some_and(|g| !g.allows(Grant::Run, lan))
    {
        return Some(format!(
            "{name} has not given this computer the Run permission{}: on {name}, open Machines, \
             press Change on {this}'s card and tick Run.",
            if lan { "" } else { " over the internet" },
            this = m::this_name(state),
        ));
    }
    None
}

// ─── asking ────────────────────────────────────────────────────────────────────

/// What `machine_run` asks.
#[derive(Clone, Debug, Default)]
pub struct Ask {
    pub machine: String,
    pub command: String,
    pub cwd: Option<String>,
    pub timeout_secs: Option<u64>,
    pub artifacts: Option<String>,
}

/// The other computer's answer to *where would it run*.
#[derive(Clone, Debug)]
struct There {
    work: String,
    checkout: String,
    folder: String,
    os: String,
    shell: String,
    shell_label: String,
    haves: Vec<String>,
    /// Why the other computer calls the command dangerous, when it does.
    dangerous: Option<String>,
}

fn there_of(v: &Value) -> Option<There> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let work = s("work").filter(|w| super::is_work_id(w))?;
    Some(There {
        work,
        checkout: s("checkout").unwrap_or_default(),
        folder: s("folder").unwrap_or_default(),
        os: s("os").unwrap_or_default(),
        shell: s("shell").unwrap_or_default(),
        shell_label: s("shellLabel").unwrap_or_default(),
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
        dangerous: s("dangerous").map(|d| clip(&d, 300)),
    })
}

/// Who allowed a run.
#[derive(Clone, Debug, Default)]
struct Allowed {
    by: Option<String>,
    grant: Option<String>,
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(n.saturating_sub(1)).collect::<String>()
        )
    }
}

/// The approval card (decision 2): the computer, the folder, the command;
/// *Always allow `<prefix>` on <computer> for this project* when a grant
/// could cover it (choosing it is a dangerous answer: a standing grant).
#[allow(clippy::too_many_arguments)]
async fn approve(
    state: &AppState,
    asker: &Asker,
    peer: &PeerRecord,
    there: &There,
    command: &str,
    cwd: &str,
    timeout: u64,
    artifacts: Option<&str>,
    dangerous: Option<String>,
) -> Result<Allowed, ToolError> {
    let name = peer.display_name().to_string();
    let project_name = crate::machines::setup::folders::name_of(&asker.top.display().to_string());
    let suggestion = if dangerous.is_none() {
        suggested_prefix(command).map(|prefix| PermissionSuggestion {
            index: 0,
            label: format!("Always allow `{prefix}` on {name} for this project"),
            kind: GRANT_KIND.into(),
            destination: Some("machines".into()),
            standing: true,
            raw: json!({
                "type": GRANT_KIND,
                "prefix": prefix,
                "machine": peer.id,
                "project": asker.top.display().to_string(),
            }),
        })
    } else {
        None
    };
    let folder_there = if cwd.is_empty() {
        there.checkout.clone()
    } else {
        format!("{} › {cwd}", there.checkout)
    };
    let title = format!("Run on {name}: {}", clip(command, 120));
    let item = PendingItem {
        session_id: asker.session.clone(),
        kind: PendingKind::Permission,
        source: PendingSource::Machine,
        tool_name: Some("machine_run".into()),
        title: title.clone(),
        detail: Some(format!(
            "In {folder_there} on {name} ({}), in a work folder at this session's code.",
            there.shell_label
        )),
        input: Some(json!({
            "machine": peer.id,
            "machineName": name,
            "os": there.os,
            "shell": there.shell,
            "shellLabel": there.shell_label,
            "checkout": there.checkout,
            "folder": there.folder,
            "cwd": cwd,
            "command": command,
            "timeoutSecs": timeout,
            "artifacts": artifacts,
            "project": asker.top.display().to_string(),
            "projectName": project_name,
        })),
        suggestions: suggestion.iter().cloned().collect(),
        dangerous: dangerous.is_some(),
        dangerous_reason: dangerous.map(|why| format!("The command {why}.")),
        ..Default::default()
    };
    let (guard, rx) = pending::raise(state, item).await;
    let id = guard.id().to_string();
    let end = pending::wait_end(rx, APPROVAL_TIMEOUT).await;
    match end {
        WaitEnd::Decided(d) if d.is_allow() => {
            let by = state
                .pending
                .resolution(&id)
                .and_then(|r| r.by)
                .and_then(|b| b.name);
            let mut allowed = Allowed {
                by: by.clone(),
                grant: None,
            };
            if let (Some(_), Some(s)) = (d.suggestion, &suggestion) {
                let prefix = s.raw["prefix"].as_str().unwrap_or_default();
                let by = by.unwrap_or_else(|| crate::claude::paths::machine_noun().into());
                if let Some(g) = state.machines.work.grants.add(
                    &asker.top.display().to_string(),
                    &project_name,
                    &peer.id,
                    prefix,
                    &by,
                ) {
                    state
                        .devices
                        .audit
                        .record(crate::server::devices::audit::AuditEntry {
                            action: "machine.run.grant.add".into(),
                            device_id: Some(peer.id.clone()),
                            device_name: Some(name.clone()),
                            detail: Some(json!({
                                "grant": g,
                                "program": super::program_of(prefix),
                                "prefixSha256": super::command_sha256(prefix),
                                "project": project_name,
                                "by": by,
                            })),
                            ..crate::server::devices::audit::AuditEntry::now()
                        });
                    m::publish(state, "runGrants");
                    allowed.grant = Some(g);
                }
            }
            Ok(allowed)
        }
        WaitEnd::Decided(d) => {
            let note = d
                .message
                .filter(|m| !m.trim().is_empty())
                .map(|m| format!(" Their note: {}", clip(m.trim(), 500)))
                .unwrap_or_default();
            Err(ToolError::new(
                "denied",
                format!(
                    "The user denied running this on {name}. Don't retry it; ask the user what \
                     to do instead.{note}"
                ),
            ))
        }
        WaitEnd::TimedOut => {
            guard.expire();
            Err(ToolError::new(
                "approval_timeout",
                format!(
                    "Nobody approved running this on {name} within {} minutes. Ask the user to \
                     approve it in Mother Claude, then try again.",
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

/// The file sizes and hashes of what the snapshot wrote in `dir`.
fn files_of(dir: &Path, meta: &code::SnapshotMeta) -> Result<Vec<Value>, String> {
    let mut names = vec![code::META.to_string()];
    if meta.bundle.is_some() {
        names.push(code::BUNDLE.to_string());
    }
    if let Some(l) = &meta.lfs {
        names.extend(l.carried.iter().map(|o| format!("lfs.{o}")));
    }
    let mut files = Vec::new();
    for name in names {
        let path = moves::file_path(dir, &name).ok_or("a file has no place")?;
        let (size, sha256) =
            crate::claude::transfer::sha256_file(&path).map_err(|e| format!("{name}: {e}"))?;
        files.push(json!({ "name": name, "size": size, "sha256": sha256 }));
    }
    Ok(files)
}

/// `machine_run`: asks `ask.machine` to run `ask.command` at the calling
/// session's code, waits up to [`WAIT_SECS`], and answers what came back
/// (or *still running*).
pub async fn run(state: &AppState, caller: &Caller, ask: Ask) -> Result<String, ToolError> {
    m::using(state).map_err(tool_err)?;
    let command = ask.command.trim().to_string();
    if command.is_empty() || ask.command.len() > MAX_COMMAND || command.contains('\0') {
        return Err(ToolError::invalid(format!(
            "The command must be one line of text of at most {MAX_COMMAND} bytes."
        )));
    }
    let timeout = ask.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    if timeout == 0 || timeout > MAX_TIMEOUT_SECS {
        return Err(ToolError::invalid(format!(
            "The timeout is 1 to {MAX_TIMEOUT_SECS} seconds (2 hours)."
        )));
    }
    let artifacts = match ask
        .artifacts
        .as_deref()
        .map(str::trim)
        .filter(|g| !g.is_empty())
    {
        Some(g) => Some(clean_glob(g).ok_or_else(|| {
            ToolError::invalid(
                "The artifacts glob names files inside the project (relative, no `..`).",
            )
        })?),
        None => None,
    };
    let peer = find_machine(state, &ask.machine)?;
    if let Some(why) = why_not(state, &peer) {
        return Err(ToolError::new("not_allowed", why));
    }
    let asker = asker(state, caller).await?;
    let cwd = match ask.cwd.as_deref() {
        Some(c) => clean_cwd(c).ok_or_else(|| {
            ToolError::invalid("cwd is a folder inside the project, relative to its top (no `..`).")
        })?,
        None => asker.rel.clone(),
    };
    let project = asker.top.display().to_string();
    let project_name = crate::machines::setup::folders::name_of(&project);
    // Where it would run there (nothing is written there yet).
    let v = call(
        state,
        &peer.id,
        "POST",
        "/peer/work/prepare",
        Some(&json!({ "project": project, "command": command })),
        false,
        CALL,
    )
    .await?;
    let there = there_of(&v).ok_or_else(|| {
        ToolError::new(
            "machine_refused",
            format!("{} answered oddly.", peer.display_name()),
        )
    })?;
    // Who agrees: a standing grant, or a person here. Dangerous when this
    // computer's classifier or the other's says so (each reads paths its
    // own way).
    let env = crate::pending::DangerEnv::current(Some(&project));
    let dangerous = crate::pending::sensitive_command(&command, &env).or_else(|| {
        there
            .dangerous
            .clone()
            .map(|why| format!("{why} (on {})", peer.display_name()))
    });
    let covered = if dangerous.is_none() {
        state
            .machines
            .work
            .grants
            .covering(&project, &peer.id, &command)
    } else {
        None
    };
    let allowed = match &covered {
        Some(g) => Allowed {
            by: Some(g.created_by.clone()),
            grant: Some(g.id.clone()),
        },
        None => {
            approve(
                state,
                &asker,
                &peer,
                &there,
                &command,
                &cwd,
                timeout,
                artifacts.as_deref(),
                dangerous.clone(),
            )
            .await?
        }
    };
    let right = dangerous.is_some();
    // The run, here.
    let id = super::new_run_id();
    state.machines.work.prune();
    let record = RunRecord {
        id: id.clone(),
        direction: Direction::Out,
        machine: peer.id.clone(),
        machine_name: peer.display_name().to_string(),
        os: there.os.clone(),
        shell: there.shell.clone(),
        session: Some(asker.session.clone()),
        session_title: asker.title.clone(),
        project: project.clone(),
        project_name: project_name.clone(),
        work: there.work.clone(),
        folder: there.folder.clone(),
        checkout: there.checkout.clone(),
        cwd: cwd.clone(),
        command: command.clone(),
        program: super::program_of(&command),
        command_sha256: super::command_sha256(&command),
        dangerous: right,
        timeout_secs: timeout,
        artifacts_glob: artifacts.clone(),
        state: RunState::Sending,
        error: None,
        exit_code: None,
        created_at: m::now(),
        started_at: None,
        ended_at: None,
        duration_ms: None,
        output_size: 0,
        truncated: false,
        tail: String::new(),
        artifacts: Vec::new(),
        fetched: Vec::new(),
        approved_by: allowed.by.clone(),
        grant: allowed.grant.clone(),
        unreachable: false,
        via_lan: None,
        updated_at: m::now(),
    };
    let run = state
        .machines
        .work
        .insert(record)
        .ok_or_else(|| ToolError::new("internal", "A run with that id exists."))?;
    state.machines.work.save(&run);
    audit(state, "machine.run.asked", &run.record());
    publish(state);
    if let Err(e) = send(state, &run, &asker, &there).await {
        let now = m::now();
        run.update(|r| {
            r.state = RunState::Error;
            r.error = Some(e.message.clone());
            r.ended_at = Some(now);
        });
        state.machines.work.save(&run);
        audit(state, "machine.run", &run.record());
        publish(state);
        return Err(e);
    }
    follow(state, run.clone());
    let r = run.wait_end(WAIT_SECS).await;
    Ok(describe(&r, WAIT_SECS))
}

/// The snapshot taken, sent and started there.
async fn send(
    state: &AppState,
    run: &Arc<Run>,
    asker: &Asker,
    there: &There,
) -> Result<(), ToolError> {
    let r = run.record();
    let dir = state
        .machines
        .work
        .run_dir(&r.id)
        .map(|d| d.join("out"))
        .ok_or_else(|| ToolError::new("internal", "This build has no data folder."))?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| ToolError::new("internal", e.to_string()))?;
    crate::claude::transfer::private_dir(&dir);
    let epoch = m::now().max(1) as u64;
    let mut req =
        code::SnapshotRequest::new(&there.work, epoch, &m::this_name(state), dir.join("code"));
    req.haves = there.haves.clone();
    // Every 32nd goes unchained, so old snapshots' commits can go.
    req.chain = !epoch.is_multiple_of(32);
    let git = state.machines.moves.git();
    let (g, top) = (git.clone(), asker.top.clone());
    let meta = tokio::task::spawn_blocking(move || code::snapshot(&g, &top, &req))
        .await
        .map_err(|e| ToolError::new("internal", e.to_string()))?
        .map_err(|e| {
            ToolError::new(
                "snapshot_failed",
                format!("The code could not be snapshotted: {e}"),
            )
        })?;
    let (g, top, work) = (git.clone(), asker.top.clone(), there.work.clone());
    let _ = tokio::task::spawn_blocking(move || {
        code::prune_snapshots(&g, &top, &work, super::KEEP_SNAPSHOTS)
    })
    .await;
    let d = dir.clone();
    let files = tokio::task::spawn_blocking(move || files_of(&d, &meta))
        .await
        .map_err(|e| ToolError::new("internal", e.to_string()))?
        .map_err(|e| ToolError::new("snapshot_failed", e))?;
    let begun = call(
        state,
        &r.machine,
        "POST",
        "/peer/work/runs",
        Some(&json!({
            "run": r.id,
            "work": r.work,
            "project": r.project,
            "projectName": r.project_name,
            "cwd": r.cwd,
            "command": r.command,
            "timeoutSecs": r.timeout_secs,
            "artifacts": r.artifacts_glob,
            "dangerous": r.dangerous,
            "session": r.session,
            "sessionTitle": r.session_title,
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
            let url = format!("/peer/work/runs/{}/files/{name}?offset={offset}", r.id);
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
        &format!("/peer/work/runs/{}/start", r.id),
        None,
        r.dangerous,
        START,
    )
    .await?;
    let _ = std::fs::remove_dir_all(&dir);
    take_status(state, run, &started);
    Ok(())
}

// ─── following it ──────────────────────────────────────────────────────────────

/// What the other computer said of the run, into this computer's record.
fn take_status(state: &AppState, run: &Run, v: &Value) {
    let Some(s) = v.get("run") else { return };
    let new_state: Option<RunState> = s
        .get("state")
        .and_then(|x| serde_json::from_value(x.clone()).ok());
    let before = run.state();
    let num = |k: &str| s.get(k).and_then(Value::as_i64);
    run.update(|r| {
        r.unreachable = false;
        if let Some(ns) = new_state {
            // An end it learned here (a cancel answered, the code refused)
            // stays.
            if !r.state.ended() {
                r.state = ns;
            }
        }
        if let Some(c) = s.get("exitCode") {
            r.exit_code = c.as_i64().and_then(|c| i32::try_from(c).ok());
        }
        if let Some(t) = num("startedAt") {
            r.started_at = Some(t);
        }
        if let Some(t) = num("endedAt") {
            r.ended_at = Some(t);
        }
        if let Some(d) = s.get("durationMs").and_then(Value::as_u64) {
            r.duration_ms = Some(d);
        }
        if let Some(n) = s.get("outputSize").and_then(Value::as_u64) {
            r.output_size = n;
        }
        if let Some(t) = s.get("truncated").and_then(Value::as_bool) {
            r.truncated = t;
        }
        if let Some(e) = s.get("error").and_then(Value::as_str) {
            r.error = Some(clip(e, 2000));
        }
        if let Some(a) = s.get("artifacts") {
            if let Ok(list) = serde_json::from_value::<Vec<super::Artifact>>(a.clone()) {
                r.artifacts = list
                    .into_iter()
                    .filter(|a| code::safe_rel_path(&a.path))
                    .take(super::MAX_ARTIFACTS)
                    .collect();
            }
        }
        if let Some(t) = s.get("tail").and_then(Value::as_str) {
            r.tail = tail_of(t, TAIL).to_string();
        }
    });
    let after = run.state();
    if after != before {
        if after.ended() && run.peek(|r| r.ended_at.is_none()) {
            let now = m::now();
            run.update(|r| r.ended_at = Some(now));
        }
        state.machines.work.save(run);
        if after.ended() {
            audit(state, "machine.run", &run.record());
        }
        publish(state);
    }
}

fn give_up(state: &AppState, run: &Run, why: String) {
    let now = m::now();
    run.update(|r| {
        r.state = RunState::Lost;
        r.error = Some(why);
        r.ended_at = Some(now);
    });
    state.machines.work.save(run);
    audit(state, "machine.run", &run.record());
    publish(state);
}

/// Follows a run there until it ends (or stays out of reach well past its
/// deadline): its state and output's tail into this computer's record.
pub fn follow(state: &AppState, run: Arc<Run>) {
    let st = state.clone();
    tokio::spawn(async move { follower(st, run).await });
}

async fn follower(state: AppState, run: Arc<Run>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let (id, machine, s, size, created, timeout, name) = run.peek(|r| {
            (
                r.id.clone(),
                r.machine.clone(),
                r.state,
                r.output_size,
                r.created_at,
                r.timeout_secs,
                r.machine_name.clone(),
            )
        });
        if s.ended() {
            break;
        }
        let path = format!(
            "/peer/work/runs/{id}?wait={POLL_WAIT}&size={size}&state={}",
            s.as_str()
        );
        let acting = acting(&state, false);
        match moves::call(&state, &machine, "GET", &path, None, &acting, POLL).await {
            Ok((200, v)) => {
                take_status(&state, &run, &v);
                backoff = Duration::from_secs(1);
                continue;
            }
            Ok((404, _)) => {
                give_up(&state, &run, format!("{name} no longer knows this run."));
                break;
            }
            Ok((403, v)) => {
                let message = v
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("it refused")
                    .to_string();
                give_up(
                    &state,
                    &run,
                    format!("{name} stopped answering about it: {message}"),
                );
                break;
            }
            Ok(_) | Err(_) => {
                let was = run.peek(|r| r.unreachable);
                if !was {
                    run.update(|r| r.unreachable = true);
                    publish(&state);
                }
                let deadline = created + timeout as i64 * 1000 + LOST_AFTER_MS;
                if m::now() > deadline {
                    give_up(
                        &state,
                        &run,
                        format!("{name} stayed out of reach past the run's time."),
                    );
                    break;
                }
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

// ─── what comes back ───────────────────────────────────────────────────────────

fn size_text(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1 << 20 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64)
    }
}

fn duration_text(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{} min {} s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

/// A run as the model reads it: where and how it ran, how it ended, its
/// output's last 64 KiB, the files it made — or *still running*.
pub fn describe(r: &RunRecord, waited_secs: u64) -> String {
    let shell = super::shell::ShellKind::parse(&r.shell)
        .map(|k| k.label())
        .unwrap_or("its shell");
    let place = if r.cwd.is_empty() {
        "the project's top".to_string()
    } else {
        r.cwd.clone()
    };
    let head = format!(
        "{} on {} ({shell}), in {place} of a work folder at this session's code ({}).",
        r.id, r.machine_name, r.folder
    );
    let mut out = head;
    out.push('\n');
    if !r.state.ended() {
        let so_far = if r.output_size > 0 {
            format!(" Output so far: {}.", size_text(r.output_size))
        } else {
            String::new()
        };
        let waiting = match r.state {
            RunState::Sending => "Its code is still on its way there.",
            RunState::Queued => "It waits for another run in its work folder to end.",
            RunState::Preparing => "Its work folder is being set to the code.",
            _ => "It is still running.",
        };
        out.push_str(&format!(
            "{waiting}{so_far} (waited {} min.) Wait for it with machine_wait {{\"id\":\"{}\"}}, \
             read its output with machine_output, or stop it with machine_cancel.\n",
            waited_secs / 60,
            r.id
        ));
        if r.unreachable {
            out.push_str(&format!(
                "{} can't be reached right now; Mother Claude keeps asking.\n",
                r.machine_name
            ));
        }
        if !r.tail.is_empty() {
            out.push_str("Its latest output:\n");
            out.push_str(tail_of(&r.tail, 4096));
            if !r.tail.ends_with('\n') {
                out.push('\n');
            }
        }
        return out;
    }
    let took = r
        .duration_ms
        .map(|d| format!(" after {}", duration_text(d)))
        .unwrap_or_default();
    let how = match r.state {
        RunState::Done | RunState::Failed => format!(
            "It ended with exit code {}{took}.",
            r.exit_code
                .map_or("unknown (a signal ended it)".to_string(), |c| c.to_string())
        ),
        RunState::Cancelled => {
            format!("It was cancelled{took}; its whole process tree was stopped.")
        }
        RunState::TimedOut => format!(
            "It ran out of time ({} s){took}; its whole process tree was stopped.",
            r.timeout_secs
        ),
        RunState::Error => format!(
            "It could not run: {}",
            r.error.as_deref().unwrap_or("an error")
        ),
        RunState::Lost => format!(
            "Mother Claude lost track of it: {}",
            r.error
                .as_deref()
                .unwrap_or("the other computer did not answer")
        ),
        _ => String::new(),
    };
    out.push_str(&how);
    out.push('\n');
    if r.output_size == 0 && r.tail.is_empty() {
        if matches!(r.state, RunState::Done | RunState::Failed) {
            out.push_str("It printed nothing.\n");
        }
    } else {
        let shown = r.tail.len() as u64;
        if r.output_size > shown {
            out.push_str(&format!(
                "Output (its last {} of {}; machine_output {{\"id\":\"{}\",\"from\":0}} reads it \
                 from the start):\n",
                size_text(shown),
                size_text(r.output_size),
                r.id
            ));
        } else {
            out.push_str(&format!("Output ({}):\n", size_text(r.output_size)));
        }
        out.push_str(&r.tail);
        if !r.tail.ends_with('\n') {
            out.push('\n');
        }
    }
    if !r.artifacts.is_empty() {
        out.push_str(&format!(
            "Files it made (bring one into this project's .mother-claude/artifacts/{}/ with \
             machine_output {{\"id\":\"{}\",\"artifact\":\"<path>\"}}):\n",
            r.id, r.id
        ));
        for a in &r.artifacts {
            let note = if a.size > MAX_ARTIFACT {
                " — too large to bring here"
            } else {
                ""
            };
            out.push_str(&format!("- {} ({}){note}\n", a.path, size_text(a.size)));
        }
    } else if r.artifacts_glob.is_some() && matches!(r.state, RunState::Done | RunState::Failed) {
        out.push_str(&format!(
            "No file matched {}.\n",
            r.artifacts_glob.as_deref().unwrap_or_default()
        ));
    }
    out
}

/// The run `id` the calling session may follow: one it asked for, or one
/// asked for in its project.
pub async fn own_run(state: &AppState, caller: &Caller, id: &str) -> Result<Arc<Run>, ToolError> {
    let unknown = || ToolError::new("not_found", format!("No run {id} from this computer."));
    if !is_run_id(id) {
        return Err(unknown());
    }
    let run = state
        .machines
        .work
        .get(id)
        .filter(|r| r.peek(|r| r.direction == Direction::Out))
        .ok_or_else(unknown)?;
    let (session, project) = run.peek(|r| (r.session.clone(), r.project.clone()));
    if caller.session_id.is_some() && session == caller.session_id {
        return Ok(run);
    }
    match asker(state, caller).await {
        Ok(a) if crate::claude::paths::same_path(&a.top.display().to_string(), &project) => Ok(run),
        _ => Err(ToolError::new(
            "denied",
            format!("Run {id} belongs to another session and project."),
        )),
    }
}

/// `machine_wait`.
pub async fn wait(
    state: &AppState,
    caller: &Caller,
    id: &str,
    secs: Option<u64>,
) -> Result<String, ToolError> {
    let run = own_run(state, caller, id).await?;
    let secs = secs.unwrap_or(WAIT_SECS).clamp(1, WAIT_SECS);
    let r = run.wait_end(secs).await;
    Ok(describe(&r, secs))
}

/// `machine_output {id, from, max}`: a page of the whole output, kept on
/// the computer that ran it for a day.
pub async fn output(
    state: &AppState,
    caller: &Caller,
    id: &str,
    from: Option<i64>,
    max: Option<u64>,
) -> Result<String, ToolError> {
    let run = own_run(state, caller, id).await?;
    let (machine, name) = run.peek(|r| (r.machine.clone(), r.machine_name.clone()));
    let max = max
        .unwrap_or(TAIL as u64)
        .clamp(1024, super::output::MAX_PAGE as u64);
    let path = format!(
        "/peer/work/runs/{id}/output?from={}&max={max}",
        from.unwrap_or(0)
    );
    let v = call(state, &machine, "GET", &path, None, false, CALL).await?;
    let num = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let (f, to, size) = (num("from"), num("to"), num("size"));
    let more = v.get("more").and_then(Value::as_bool).unwrap_or(false);
    let text = v.get("text").and_then(Value::as_str).unwrap_or_default();
    let truncated = v.get("truncated").and_then(Value::as_bool).unwrap_or(false);
    let mut out = format!("{id} on {name}: output bytes {f}–{to} of {size}");
    if more {
        out.push_str(&format!(
            "; the next page: machine_output {{\"id\":\"{id}\",\"from\":{to}}}"
        ));
    }
    if truncated {
        out.push_str(" (the run printed more than is kept; this is what is)");
    }
    out.push_str(".\n");
    out.push_str(text);
    Ok(out)
}

/// `machine_output {id, artifact}`: brings one of the run's files into the
/// session's project, under `.mother-claude/artifacts/<run>/` (git ignores
/// that folder: a `.gitignore` in it says `*`).
pub async fn fetch(
    state: &AppState,
    caller: &Caller,
    id: &str,
    rel: &str,
) -> Result<String, ToolError> {
    let run = own_run(state, caller, id).await?;
    let r = run.record();
    let rel = rel.trim().replace('\\', "/");
    let rel = rel.trim_start_matches("./");
    if !r.artifacts.iter().any(|a| a.path == rel) || !code::safe_rel_path(rel) {
        return Err(ToolError::invalid(format!(
            "{rel} is not one of the files run {id} listed."
        )));
    }
    let query = url::form_urlencoded::byte_serialize(rel.as_bytes()).collect::<String>();
    let info = call(
        state,
        &r.machine,
        "GET",
        &format!("/peer/work/runs/{id}/artifact?path={query}"),
        None,
        false,
        CALL,
    )
    .await?;
    let size = info.get("size").and_then(Value::as_u64).unwrap_or(0);
    let sha = info
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if size > MAX_ARTIFACT {
        return Err(ToolError::new(
            "too_large",
            "Files of more than 64 MiB stay on the computer that made them.",
        ));
    }
    let dest_dir =
        artifacts_dir(Path::new(&r.project), id).map_err(|e| ToolError::new("not_written", e))?;
    let dest = rel.split('/').fold(dest_dir, |p, s| p.join(s));
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ToolError::new("not_written", e.to_string()))?;
    }
    let part = chunks::part_of(&dest);
    let _ = std::fs::remove_file(&part);
    let mut offset = 0u64;
    let acting = acting(state, false);
    while offset < size {
        let path = format!("/peer/work/runs/{id}/artifact/bytes?path={query}&offset={offset}");
        let got = m::transport::request_raw(
            state,
            &r.machine,
            "GET",
            &path,
            &[(m::acting::HEADER, acting.clone())],
            axum::body::Bytes::new(),
            chunks::CHUNK_TIMEOUT,
        )
        .await;
        let bytes = match got {
            Ok((200, _, b)) if !b.is_empty() => b,
            Ok((s, _, b)) => {
                let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
                let _ = std::fs::remove_file(&part);
                return Err(tool_err(moves::refusal(s, &v, &r.machine_name)));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                return Err(ToolError::new(
                    "unreachable",
                    format!("{} can't be reached right now: {e}", r.machine_name),
                ));
            }
        };
        let p = part.clone();
        let n = bytes.len() as u64;
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&p)?;
            f.write_all(&bytes)
        })
        .await
        .map_err(|e| ToolError::new("internal", e.to_string()))?
        .map_err(|e| ToolError::new("not_written", e.to_string()))?;
        offset += n;
    }
    let (d, p) = (dest.clone(), part.clone());
    tokio::task::spawn_blocking(move || chunks::finish(&d, size, &sha).map(|_| p))
        .await
        .map_err(|e| ToolError::new("internal", e.to_string()))?
        .map_err(|e| {
            let _ = std::fs::remove_file(&part);
            ToolError::new("not_written", e)
        })?;
    let to = dest.display().to_string();
    let now = m::now();
    run.update(|r| {
        r.fetched.retain(|f| f.path != rel);
        r.fetched.push(FetchedArtifact {
            path: rel.to_string(),
            to: to.clone(),
            size,
            at: now,
        });
    });
    state.machines.work.save(&run);
    publish(state);
    Ok(format!(
        "{rel} ({}) from {} is now at {to}.",
        size_text(size),
        r.machine_name
    ))
}

/// `<project>/.mother-claude/artifacts/<run>/`, made with the `.gitignore`
/// that keeps git from seeing it — never through a link.
pub fn artifacts_dir(project: &Path, run: &str) -> Result<PathBuf, String> {
    let mut dir = project.to_path_buf();
    for part in [".mother-claude", "artifacts", run] {
        dir = dir.join(part);
        match std::fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                return Err(format!(
                    "{} is not a folder of this project.",
                    dir.display()
                ))
            }
            Ok(_) => {}
            Err(_) => std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?,
        }
        if part == "artifacts" {
            let ignore = dir.join(".gitignore");
            if !ignore.exists() {
                std::fs::write(
                    &ignore,
                    "# Files Mother Claude brought back from runs on other computers.\n*\n",
                )
                .map_err(|e| format!("{}: {e}", ignore.display()))?;
            }
        }
    }
    Ok(dir)
}

/// Cancels a run: there (the asking side), or here (the side that runs
/// it). Answers its record.
pub async fn cancel_run(state: &AppState, run: &Arc<Run>) -> Result<RunRecord, ApiError> {
    let r = run.record();
    if r.state.ended() {
        return Ok(r);
    }
    match r.direction {
        Direction::In => {
            run.ask_cancel();
            if r.state == RunState::Sending {
                let now = m::now();
                run.update(|r| {
                    r.state = RunState::Cancelled;
                    r.ended_at = Some(now);
                });
                state.machines.work.save(run);
                audit(state, "machine.run", &run.record());
                publish(state);
            } else {
                let _ = tokio::time::timeout(Duration::from_secs(10), async {
                    let mut rx = run.watch();
                    while !run.state().ended() {
                        if rx.changed().await.is_err() {
                            break;
                        }
                    }
                })
                .await;
            }
            Ok(run.record())
        }
        Direction::Out => {
            let acting = acting(state, false);
            let path = format!("/peer/work/runs/{}/cancel", r.id);
            match moves::call(state, &r.machine, "POST", &path, None, &acting, CALL).await {
                Ok((200, v)) => {
                    take_status(state, run, &v);
                    Ok(run.record())
                }
                Ok((404, _)) if r.state == RunState::Sending => {
                    // It never got there.
                    let now = m::now();
                    run.update(|r| {
                        r.state = RunState::Cancelled;
                        r.ended_at = Some(now);
                    });
                    state.machines.work.save(run);
                    publish(state);
                    Ok(run.record())
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

/// `machine_cancel`.
pub async fn cancel(state: &AppState, caller: &Caller, id: &str) -> Result<String, ToolError> {
    let run = own_run(state, caller, id).await?;
    let r = cancel_run(state, &run).await.map_err(tool_err)?;
    Ok(describe(&r, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(state: RunState) -> RunRecord {
        RunRecord {
            id: "run_0123456789abcdef".into(),
            direction: Direction::Out,
            machine: "0123456789abcdef0123456789abcdef".into(),
            machine_name: "DESKTOP-PC".into(),
            os: "windows".into(),
            shell: "pwsh".into(),
            session: None,
            session_title: None,
            project: "/Users/me/app".into(),
            project_name: "app".into(),
            work: "wk_0123456789abcdef".into(),
            folder: r"C:\data\machines\work\wk_0123456789abcdef".into(),
            checkout: r"D:\dev\app".into(),
            cwd: String::new(),
            command: "cargo test".into(),
            program: "cargo".into(),
            command_sha256: String::new(),
            dangerous: false,
            timeout_secs: 1800,
            artifacts_glob: None,
            state,
            error: None,
            exit_code: None,
            created_at: 1,
            started_at: Some(1),
            ended_at: None,
            duration_ms: None,
            output_size: 0,
            truncated: false,
            tail: String::new(),
            artifacts: Vec::new(),
            fetched: Vec::new(),
            approved_by: None,
            grant: None,
            unreachable: false,
            via_lan: None,
            updated_at: 1,
        }
    }

    #[test]
    fn a_result_says_where_how_it_ended_and_what_it_printed() {
        let mut r = record(RunState::Failed);
        r.exit_code = Some(101);
        r.duration_ms = Some(12_345);
        r.output_size = 5;
        r.tail = "boom\n".into();
        let t = describe(&r, 600);
        assert!(t.contains("DESKTOP-PC (PowerShell 7)"), "{t}");
        assert!(t.contains("exit code 101 after 12.3 s"), "{t}");
        assert!(t.contains("Output (5 B):\nboom\n"), "{t}");
        // Longer than its tail: says how to read the rest.
        r.output_size = 3 << 20;
        let t = describe(&r, 600);
        assert!(
            t.contains("machine_output {\"id\":\"run_0123456789abcdef\",\"from\":0}"),
            "{t}"
        );
        // Still running.
        let mut r = record(RunState::Running);
        r.output_size = 2048;
        r.tail = "compiling…".into();
        let t = describe(&r, 600);
        assert!(t.contains("still running"), "{t}");
        assert!(
            t.contains("machine_wait {\"id\":\"run_0123456789abcdef\"}"),
            "{t}"
        );
        assert!(t.contains("compiling…"), "{t}");
        // Files it made.
        let mut r = record(RunState::Done);
        r.exit_code = Some(0);
        r.artifacts = vec![super::super::Artifact {
            path: "target/release/app.exe".into(),
            size: 3 << 20,
        }];
        let t = describe(&r, 0);
        assert!(t.contains("- target/release/app.exe (3.0 MiB)"), "{t}");
        assert!(t.contains("It printed nothing."), "{t}");
    }

    #[test]
    fn artifacts_land_in_an_ignored_folder_of_the_project_never_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("app");
        std::fs::create_dir_all(&project).unwrap();
        let d = artifacts_dir(&project, "run_0123456789abcdef").unwrap();
        assert!(d.ends_with(Path::new(".mother-claude/artifacts/run_0123456789abcdef")));
        let ignore =
            std::fs::read_to_string(project.join(".mother-claude/artifacts/.gitignore")).unwrap();
        assert!(ignore.lines().any(|l| l == "*"));
        // A file where a folder should be: refused.
        let other = dir.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join(".mother-claude"), "x").unwrap();
        assert!(artifacts_dir(&other, "run_0123456789abcdef").is_err());
    }

    #[test]
    fn a_folder_below_the_top_is_relative_with_slashes() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path().join("app");
        std::fs::create_dir_all(top.join("src").join("ui")).unwrap();
        assert_eq!(relative(&top.join("src").join("ui"), &top), "src/ui");
        assert_eq!(relative(&top, &top), "");
        assert_eq!(relative(dir.path(), &top), "");
    }
}
