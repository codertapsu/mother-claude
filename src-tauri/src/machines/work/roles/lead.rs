//! The lead's computer (docs/PLAN-MACHINES.md Phase 4 decision 9): its
//! launcher starts a role whose template names another computer *there*
//! ([`start`]: the computer resolved, a binding minted, the lead's code
//! snapshotted and sent — part 2's transfer — and the computer asked to
//! admit and start it), forwards stops, idle stops and resumes ([`stop`],
//! [`start`] again), hands it messages ([`message`]), follows it
//! ([`refresh_all`]: its session's state into the team's view), takes back
//! what it did when it finishes a task ([`land`]: the branch
//! `mother-claude/<computer>/<team>/<role>`, forward only) and answers its
//! board calls ([`board_call`]: only with the binding).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    check_binding, is_binding_id, new_binding, secret_sha256, BindingClaim, BindingKey, RoleSpec,
    IN_DIR, OUT_DIR,
};
use crate::claude::owned;
use crate::machines::chunks::{self, PutError};
use crate::machines::code::{self, TaskBranchError, TaskTip};
use crate::machines::grants::Grant;
use crate::machines::work::target::FileSpec;
use crate::machines::work::tasks::target::AccountAsk;
use crate::machines::{self as m, moves};
use crate::orchestration::board::{RemoteWork, Task};
use crate::orchestration::launcher::{self, StartError};
use crate::orchestration::router;
use crate::orchestration::teams::{RemoteSlot, RoleSlot, Team};
use crate::orchestration::templates::RoleDef;
use crate::server::context::{ClientContext, Principal};
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// A plain request's time.
const CALL: Duration = Duration::from_secs(30);
/// How long the other computer may take to set the code up and start the
/// session.
const START: Duration = Duration::from_secs(10 * 60);
/// A running role is looked at this often; one that is not, less.
const REFRESH_MS: i64 = 2_500;
const REFRESH_IDLE_MS: i64 = 30_000;
/// How long a `task_done` waits for its branch before it finishes anyway
/// (the branch then lands in the background and the task records it).
const LAND_WAIT: Duration = Duration::from_secs(80);

fn now() -> i64 {
    m::now()
}

/// The header the lead sends for a team: the right of whoever started it
/// (a role without permission prompts needs it there too).
fn acting(state: &AppState, team: &Team) -> String {
    m::acting::machine_header(&m::this_name(state), team.starter.may_dangerous)
}

/// A JSON call to `machine` about a role: its answer, or the refusal in its
/// own words.
async fn call(
    state: &AppState,
    machine: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
    acting: &str,
    timeout: Duration,
) -> Result<Value, ApiError> {
    let name = m::peer_name(state, machine);
    match moves::call(state, machine, method, path, body, acting, timeout).await {
        Ok((s, v)) if (200..300).contains(&s) => Ok(v),
        Ok((404, v)) if v.pointer("/error/code").is_none() => Err(ApiError::conflict(
            "update_needed",
            format!("Update Mother Claude on {name}: it does not run team roles yet."),
        )),
        Ok((s, v)) => Err(moves::refusal(s, &v, &name)),
        Err(e) => Err(ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "machine_unreachable",
            format!("{name} can't be reached right now: {e}"),
        )),
    }
}

fn audit(state: &AppState, action: &str, team: &Team, role: &str, slot: &RemoteSlot, extra: Value) {
    let mut detail = json!({
        "team": team.id,
        "role": role,
        "binding": slot.binding,
        "machine": slot.machine,
    });
    if let (Some(d), Some(e)) = (detail.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            d.insert(k.clone(), v.clone());
        }
    }
    state.devices.audit.record(AuditEntry {
        action: action.to_string(),
        device_id: Some(slot.machine.clone()),
        device_name: Some(m::peer_name(state, &slot.machine)),
        detail: Some(detail),
        ..AuditEntry::now()
    });
}

/// Whether this computer may start a role on `peer` now, or why not.
fn why_not(state: &AppState, peer: &crate::machines::store::PeerRecord) -> Option<String> {
    let name = peer.display_name();
    if !peer.capabilities.is_empty() && !peer.capabilities.iter().any(|c| c == super::CAPABILITY) {
        return Some(format!(
            "Update Mother Claude on {name}: it does not run team roles yet."
        ));
    }
    crate::machines::work::source::why_not(state, peer)
}

/// The paired computer the role's `machine` names, ready for it, or why
/// not in the user's words.
fn resolve(
    state: &AppState,
    role: &RoleDef,
    slot: &RoleSlot,
) -> Result<crate::machines::store::PeerRecord, String> {
    let spec = role.machine.as_deref().unwrap_or_default();
    let Some(parsed) = super::parse_spec(spec) else {
        return Err(format!("The {} role names no computer.", role.title));
    };
    if super::names_this(state, spec) {
        return Err(format!(
            "The {} role names this computer: leave `machine` out to run it here.",
            role.title
        ));
    }
    let candidates = super::candidates(state);
    let prefer = slot.remote.as_ref().map(|r| r.machine.as_str());
    let chosen =
        super::pick(&parsed, &candidates, prefer).map_err(|u| super::unresolved_text(spec, &u))?;
    let peer = state
        .machines
        .store_read(|f| f.peer(&chosen.id).cloned())
        .ok_or_else(|| super::unresolved_text(spec, &super::Unresolved::Unknown))?;
    if let Some(why) = why_not(state, &peer) {
        return Err(why);
    }
    if crate::claude::owned::is_dangerous_mode(Some(role.mode())) {
        let lan = state.machines.link(&peer.id).path == Some(m::transport::PathKind::Lan);
        if peer
            .their_grants
            .is_some_and(|g| !g.allows(Grant::Dangerous, lan))
        {
            return Err(format!(
                "The {} role runs in {} on {name}, which needs {name}'s Dangerous permission for \
                 this computer: on {name}, open Machines, press Change on {this}'s card and tick \
                 Dangerous.",
                role.title,
                role.mode(),
                name = peer.display_name(),
                this = m::this_name(state)
            ));
        }
    }
    Ok(peer)
}

/// The team's account, as the other computer matches it (by name).
fn account_ask(state: &AppState, team: &Team) -> AccountAsk {
    AccountAsk {
        name: state
            .accounts
            .get(&team.account)
            .map(|a| a.name)
            .unwrap_or_default(),
        is_default: team.account == crate::claude::accounts::DEFAULT_ID,
    }
}

/// What the other computer answered to *may it start, and where*.
#[derive(Debug, Default)]
struct There {
    admitted: bool,
    reasons: Vec<String>,
    alive: bool,
    known: bool,
    resumable: bool,
    wants_code: bool,
    session: Option<String>,
    folder: String,
    os: String,
    haves: Vec<String>,
}

fn there_of(v: &Value) -> There {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    There {
        admitted: b("admitted"),
        reasons: v
            .get("reasons")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(|r| r.chars().filter(|c| !c.is_control()).take(300).collect())
                    .take(10)
                    .collect()
            })
            .unwrap_or_default(),
        alive: b("alive"),
        known: b("known"),
        resumable: b("resumable"),
        wants_code: b("wantsCode"),
        session: s("session").filter(|s| owned::is_uuid(s)),
        folder: s("folder")
            .map(|f| f.chars().filter(|c| !c.is_control()).take(4096).collect())
            .unwrap_or_default(),
        os: s("os").unwrap_or_default(),
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
    }
}

/// Where a role on another computer runs, for its charter.
pub struct Place {
    pub machine_name: String,
    pub folder: String,
    pub lead_machine: String,
}

/// Start (or resume) a role's session on the computer its `machine` names:
/// resolved (Machines on, online, its `run` grant for this computer), a
/// binding minted the first time, the lead's code snapshotted and sent when
/// that computer will take it, then that computer admits (its own
/// `capacity::admit`) and starts it. The task is claimed for the session
/// before it starts, as for a local role.
pub async fn start(
    state: &AppState,
    team: &Team,
    role: &RoleDef,
    slot: &RoleSlot,
    task: Option<&Task>,
    decision: &router::Decision,
) -> Result<String, StartError> {
    if let Err(e) = m::using(state) {
        return Err(StartError::Refused(e.message));
    }
    let peer = resolve(state, role, slot).map_err(StartError::Refused)?;
    let name = peer.display_name().to_string();
    let git = state.machines.moves.git();
    let project = PathBuf::from(&team.project);
    let top = {
        let g = git.clone();
        tokio::task::spawn_blocking(move || code::top_of(&g, &project))
            .await
            .map_err(|e| StartError::Failed(e.to_string()))?
            .map_err(|_| {
                StartError::Refused(format!(
                    "A role on another computer works at the project's code: {} must be in a git \
                     repository.",
                    team.project
                ))
            })?
    };
    let same = slot
        .remote
        .as_ref()
        .filter(|r| r.machine == peer.id && is_binding_id(&r.binding));
    let (mut binding, mut secret) = match same {
        Some(r) => (r.binding.clone(), None),
        None => {
            let (b, s) = new_binding();
            (b, Some(s))
        }
    };
    let held = task.is_some_and(|t| {
        t.status.held() && t.claimed_by.is_some() && t.claimed_by == slot.session_id
    });
    let acting = acting(state, team);
    let mut there = There::default();
    for _ in 0..2 {
        let resume = slot
            .session_id
            .clone()
            .filter(|_| secret.is_none())
            .filter(|id| owned::is_uuid(id));
        let v = call(
            state,
            &peer.id,
            "POST",
            "/peer/teams/roles/prepare",
            Some(&json!({
                "binding": binding,
                "team": team.id,
                "role": role.id,
                "project": team.project,
                "projectName": crate::machines::setup::folders::name_of(&team.project),
                "account": account_ask(state, team),
                "model": decision.choice.model,
                "priority": task.map(|t| t.priority).unwrap_or(2),
                "held": held,
                "resume": resume,
            })),
            &acting,
            CALL,
        )
        .await
        .map_err(|e| StartError::Refused(e.message))?;
        there = there_of(&v);
        if there.known || secret.is_some() {
            break;
        }
        // It no longer knows the binding (removed there): a new one.
        let (b, s) = new_binding();
        binding = b;
        secret = Some(s);
    }
    // What the slot keeps of the binding it had: everything while it is
    // the same one (what was taken back, the branch), nothing for a new one.
    let kept = same
        .filter(|r| r.binding == binding)
        .cloned()
        .unwrap_or_default();
    let slot_remote = |folder: &str| RemoteSlot {
        machine: peer.id.clone(),
        machine_name: name.clone(),
        os: if there.os.is_empty() {
            peer.os.clone()
        } else {
            there.os.clone()
        },
        binding: binding.clone(),
        secret_sha256: match &secret {
            Some(s) => secret_sha256(s),
            None => kept.secret_sha256.clone(),
        },
        folder: folder.to_string(),
        ..kept.clone()
    };
    if there.alive {
        // Its session runs there already (this computer restarted, or a
        // start crossed another): the role is up.
        let session = there
            .session
            .clone()
            .ok_or_else(|| StartError::Failed(format!("{name} answered oddly.")))?;
        bind(
            state,
            team,
            role,
            &session,
            slot_remote(&there.folder),
            decision,
        );
        note_status(state, &binding, true, false, &session);
        return Ok(session);
    }
    if !there.admitted {
        return Err(StartError::Waiting(
            there
                .reasons
                .iter()
                .map(|r| format!("on {name}: {r}"))
                .collect(),
        ));
    }
    // The id it runs under: its last session's when that resumes there.
    let resume = slot
        .session_id
        .clone()
        .filter(|_| there.resumable && secret.is_none());
    let session = resume
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let assigned = launcher::claim_for(state, team, task, &session, decision);
    let prompt = launcher::opening(state, team, role, task, resume.is_some());
    let mut code_sent = false;
    if there.wants_code || secret.is_some() {
        if let Err(e) = send_code(
            state,
            team,
            role,
            &peer.id,
            &binding,
            secret.as_deref(),
            &top,
            there.haves.clone(),
            &acting,
        )
        .await
        {
            launcher::give_back(state, team, &session, "its session did not start");
            return Err(e);
        }
        code_sent = true;
    }
    let place = Place {
        machine_name: name.clone(),
        folder: there.folder.clone(),
        lead_machine: m::this_name(state),
    };
    let spec = RoleSpec {
        session_name: team.session_name(&role.id),
        charter: launcher::charter_for(team, role, Some(&place)),
        agents: Some(launcher::agents_json(team)),
        paths: role.paths.clone(),
        read_only: role.read_only,
        add_dirs: role.add_dirs.clone(),
        permission_mode: role.mode().to_string(),
        model: decision.choice.model.clone(),
        effort: decision.choice.effort.clone(),
        fallback_model: launcher::fallback_models(state, &decision.choice),
        budget_usd: launcher::session_budget(state, team, role),
        priority: task.map(|t| t.priority).unwrap_or(2),
    };
    let started = call(
        state,
        &peer.id,
        "POST",
        &format!("/peer/teams/roles/{binding}/start"),
        Some(&json!({
            "sessionId": session,
            "resume": resume,
            "held": held,
            "code": code_sent,
            "spec": spec,
            "prompt": prompt,
            "account": account_ask(state, team),
        })),
        &acting,
        START,
    )
    .await;
    let v = match started {
        Ok(v) => v,
        Err(e) => {
            launcher::give_back(state, team, &session, "its session did not start");
            return Err(StartError::Failed(e.message));
        }
    };
    if v.get("state").and_then(Value::as_str) == Some("waiting") {
        launcher::give_back(state, team, &session, "its computer had no room for it yet");
        let reasons = there_of(&v).reasons;
        return Err(StartError::Waiting(
            reasons.iter().map(|r| format!("on {name}: {r}")).collect(),
        ));
    }
    let session = v
        .get("session")
        .and_then(Value::as_str)
        .filter(|s| owned::is_uuid(s))
        .map(str::to_string)
        .unwrap_or(session);
    let folder = v
        .get("folder")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or(there.folder.clone());
    let mut remote = slot_remote(&folder);
    if let Some(b) = v
        .get("base")
        .and_then(Value::as_str)
        .filter(|b| code::is_oid(b))
    {
        remote.base = Some(b.to_string());
    }
    bind(state, team, role, &session, remote.clone(), decision);
    note_status(state, &binding, true, false, &session);
    if let Some(c) = assigned {
        let tid = c.task.id;
        crate::orchestration::metrics::task_started(
            state,
            &crate::orchestration::delegation::outcome_id(&team.project, tid),
            &team.project,
            Some(&role.id),
            &session,
        );
        crate::orchestration::delegation::on_change(state, &team.id, c).await;
    }
    audit(
        state,
        "team.remote.start",
        team,
        &role.id,
        &remote,
        json!({ "session": session, "resumed": resume.is_some() }),
    );
    crate::server::ws::publish_notice(
        state,
        crate::claude::model::NoticeLevel::Info,
        format!(
            "Started the {} role of {} on {name} ({}{})",
            role.title,
            team.label,
            decision.choice.model,
            decision
                .choice
                .effort
                .as_deref()
                .map(|e| format!(" @ {e}"))
                .unwrap_or_default()
        ),
        None,
    );
    Ok(session)
}

/// The role's slot, bound to its session there.
fn bind(
    state: &AppState,
    team: &Team,
    role: &RoleDef,
    session: &str,
    remote: RemoteSlot,
    decision: &router::Decision,
) {
    let now = now();
    state.teams.bind(&team.id, &role.id, session, false, now);
    let choice = decision.choice.clone();
    let reason = decision.reason.clone();
    state.teams.update(&team.id, |t| {
        if let Some(s) = t.roles.get_mut(&role.id) {
            s.model = Some(choice);
            s.model_reason = Some(reason);
            s.wanted = false;
            s.worktree_path = Some(remote.folder.clone()).filter(|f| !f.is_empty());
            s.remote = Some(remote);
        }
    });
}

/// What the lead knows of a role's session there now.
fn note_status(state: &AppState, binding: &str, alive: bool, busy: bool, session: &str) {
    let at = now();
    state.teams.set_remote_status(binding, |s| {
        s.alive = alive;
        s.busy = busy;
        s.session = Some(session.to_string());
        s.unreachable = false;
        s.gone = false;
        s.at = at;
    });
}

/// The lead's code, snapshotted and sent (part 2's transfer): `begin`, then
/// each file in chunks from what arrived there.
#[allow(clippy::too_many_arguments)]
async fn send_code(
    state: &AppState,
    team: &Team,
    role: &RoleDef,
    machine: &str,
    binding: &str,
    secret: Option<&str>,
    top: &Path,
    haves: Vec<String>,
    acting: &str,
) -> Result<(), StartError> {
    let failed = |e: String| StartError::Failed(e);
    let dir = state
        .machines
        .work
        .roles
        .dir_of(binding)
        .map(|d| d.join(OUT_DIR))
        .ok_or_else(|| failed("This build has no data folder.".into()))?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| failed(e.to_string()))?;
    crate::claude::transfer::private_dir(&dir);
    let this = m::this_name(state);
    let mut req = code::SnapshotRequest::new(binding, now().max(1) as u64, &this, dir.join("code"));
    req.haves = haves;
    req.chain = false;
    req.label = Some(format!(
        "{} team role {} from {}",
        code::safe_name(&team.label),
        role.id,
        code::safe_name(&this)
    ));
    let git = state.machines.moves.git();
    let (g, t) = (git.clone(), top.to_path_buf());
    let (meta, ours) = tokio::task::spawn_blocking(move || {
        let meta = code::snapshot(&g, &t, &req)?;
        let ours = code::haves(&g, &t, None)?;
        Ok::<_, code::CodeError>((meta, ours.haves))
    })
    .await
    .map_err(|e| failed(e.to_string()))?
    .map_err(|e| failed(format!("The project's code could not be snapshotted: {e}")))?;
    let d = dir.clone();
    let files =
        tokio::task::spawn_blocking(move || crate::machines::work::source::files_of(&d, &meta))
            .await
            .map_err(|e| failed(e.to_string()))?
            .map_err(failed)?;
    let begun = call(
        state,
        machine,
        "POST",
        "/peer/teams/roles",
        Some(&json!({
            "binding": binding,
            "secret": secret,
            "team": team.id,
            "teamLabel": team.label,
            "role": role.id,
            "roleTitle": role.title,
            "project": team.project,
            "projectName": crate::machines::setup::folders::name_of(&team.project),
            "haves": ours,
            "files": files,
        })),
        acting,
        CALL,
    )
    .await
    .map_err(|e| failed(e.message))?;
    let have = |name: &str| -> u64 {
        begun
            .get("files")
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|f| f["name"] == name))
            .and_then(|f| f["have"].as_u64())
            .unwrap_or(0)
    };
    let peer = m::peer_name(state, machine);
    for f in &files {
        let fname = f["name"].as_str().unwrap_or_default().to_string();
        let size = f["size"].as_u64().unwrap_or(0);
        let Some(path) = moves::file_path(&dir, &fname) else {
            continue;
        };
        let mut offset = have(&fname).min(size);
        while offset < size {
            let n = ((size - offset) as usize).min(chunks::CHUNK);
            let p = path.clone();
            let bytes = tokio::task::spawn_blocking(move || chunks::read_chunk(&p, offset, n))
                .await
                .map_err(|e| failed(e.to_string()))?
                .map_err(|e| failed(e.to_string()))?;
            let url = format!("/peer/teams/roles/{binding}/files/{fname}?offset={offset}");
            offset = match chunks::put(state, machine, &url, bytes, acting).await {
                Ok(s) => s,
                Err(PutError::Offline(e)) => {
                    return Err(failed(format!("{peer} can't be reached right now: {e}")))
                }
                Err(PutError::Odd) => return Err(failed(format!("{peer} answered oddly."))),
                Err(PutError::Refused(status, v)) => {
                    return Err(failed(moves::refusal(status, &v, &peer).message))
                }
            };
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// The role's session there stopped (by hand, its idle stop, the team's
/// stop). Whether that computer said so.
pub async fn stop(state: &AppState, team: &Team, role: &str, why: &str) -> bool {
    let slot = team.slot(role);
    let Some(r) = slot.remote.clone() else {
        return false;
    };
    let path = format!("/peer/teams/roles/{}/stop", r.binding);
    let ok = call(
        state,
        &r.machine,
        "POST",
        &path,
        Some(&json!({ "why": why })),
        &acting(state, team),
        CALL,
    )
    .await
    .is_ok();
    let at = now();
    state.teams.set_remote_status(&r.binding, |s| {
        s.alive = false;
        s.busy = false;
        s.at = at;
    });
    audit(
        state,
        "team.remote.stop",
        team,
        role,
        &r,
        json!({ "delivered": ok }),
    );
    ok
}

/// A message to the role's session there (a task's packet, a board
/// notice). Whether it was delivered.
pub async fn message(state: &AppState, team: &Team, role: &str, text: &str) -> bool {
    let slot = team.slot(role);
    let Some(r) = slot.remote.clone() else {
        return false;
    };
    let path = format!("/peer/teams/roles/{}/message", r.binding);
    call(
        state,
        &r.machine,
        "POST",
        &path,
        Some(&json!({ "text": text })),
        &acting(state, team),
        CALL,
    )
    .await
    .is_ok()
}

/// A direct message to a role session of this computer's teams that runs
/// on another computer, while that session is stopped there (its idle
/// stop, a stop by hand): the lead's launcher resumes it there with its
/// role's options — `launcher::start_role`, the path new work takes, so
/// that computer's admission, the charter, the settings, the team tools
/// and the guard as at every start — then the message goes to it.
/// `None`: not such a session, or it runs there (the message goes to it as
/// any message does). A role that runs without approvals needs the
/// sender's dangerous right.
pub async fn resume_with(
    state: &AppState,
    machine: &str,
    session: &str,
    text: &str,
    may_dangerous: bool,
) -> ApiResult<Option<crate::claude::model::MessageReceipt>> {
    if !owned::is_uuid(session) {
        return Ok(None);
    }
    let found = state.teams.all().into_iter().find_map(|t| {
        let role = t
            .roles
            .iter()
            .find(|(_, s)| {
                s.session_id.as_deref() == Some(session)
                    && s.remote.as_ref().is_some_and(|r| r.machine == machine)
            })?
            .0
            .clone();
        Some((t, role))
    });
    let Some((team, role)) = found else {
        return Ok(None);
    };
    let Some(def) = team.role(&role).cloned() else {
        return Ok(None);
    };
    // How it is there now: the last word may be minutes old.
    refresh(state, &team.id, &role).await;
    let binding = team
        .slot(&role)
        .remote
        .map(|r| r.binding)
        .unwrap_or_default();
    if state.teams.remote_status(&binding).is_some_and(|s| s.alive) {
        return Ok(None);
    }
    if owned::is_dangerous_mode(Some(def.mode())) && !may_dangerous {
        return Err(ApiError::dangerous_blocked(
            "This role's session runs without approvals: resuming it needs the dangerous right.",
        ));
    }
    if text.trim().is_empty() || text.len() > owned::MAX_TEXT_BYTES || text.contains('\0') {
        return Err(ApiError::unprocessable(
            "invalid_message",
            "The message is empty or too long.",
        ));
    }
    // For the task it held, as the launcher's own resume does ("you were
    // working on this"), then the message.
    let held = state.teams.boards.read(&team.project, |b| {
        b.tasks
            .iter()
            .filter(|t| t.status.held() && t.claimed_by.as_deref() == Some(session))
            .min_by_key(|t| (t.priority, t.id))
            .map(|t| t.id)
    });
    let running = launcher::running_workers(state).await;
    match launcher::start_role(state, &team.id, &role, held, &running).await {
        Ok(_) => {}
        Err(StartError::Waiting(reasons)) => {
            return Err(ApiError::conflict(
                "role_waiting",
                format!("The {} role waits: {}", def.title, reasons.join("; ")),
            )
            .with("reasons", json!(reasons)))
        }
        Err(e) => {
            return Err(ApiError::conflict(
                "role_not_resumed",
                format!("The {} role could not be resumed: {e}", def.title),
            ))
        }
    }
    let team = state
        .teams
        .get(&team.id)
        .ok_or_else(|| ApiError::conflict("role_not_resumed", "The team was removed meanwhile."))?;
    if !message(state, &team, &role, text).await {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "role_not_reached",
            format!(
                "The {} role's session was resumed but did not take the message: send it again.",
                def.title
            ),
        ));
    }
    if let Some(r) = team.slot(&role).remote {
        audit(
            state,
            "team.remote.resume",
            &team,
            &role,
            &r,
            json!({ "why": "message" }),
        );
    }
    Ok(Some(crate::claude::model::MessageReceipt {
        delivery: crate::claude::model::Delivery::Sent,
        message_id: uuid::Uuid::new_v4().to_string(),
    }))
}

/// `POST /peer/teams/roles/wake`: the computer that runs a role of this
/// computer's teams got a message for its stopped session — the lead's to
/// resume. Accepted with the binding this computer minted (as a board
/// call); then as [`resume_with`]; a session that runs meanwhile just gets
/// the message.
#[derive(Debug, Deserialize)]
pub struct WakeCall {
    pub binding: String,
    #[serde(default)]
    pub secret: String,
    pub team: String,
    pub role: String,
    pub session: String,
    pub text: String,
}

pub async fn wake(state: &AppState, ctx: &ClientContext, b: WakeCall) -> ApiResult<Value> {
    let Principal::Peer { machine: from, .. } = &ctx.principal else {
        return Err(ApiError::forbidden(
            "peer_only",
            "Only a paired computer can call this.",
        ));
    };
    m::serving(state)?;
    if !crate::orchestration::teams::is_team_id(&b.team) {
        return Err(refused());
    }
    let team = state.teams.get(&b.team).ok_or_else(refused)?;
    let slot = team.slot(&b.role);
    let r = slot.remote.clone().ok_or_else(refused)?;
    let key = BindingKey {
        binding: &r.binding,
        machine: &r.machine,
        team: &team.id,
        role: &b.role,
        secret_sha256: &r.secret_sha256,
        session: slot.session_id.as_deref(),
    };
    let claim = BindingClaim {
        from,
        binding: &b.binding,
        secret: &b.secret,
        team: &b.team,
        role: &b.role,
        session: &b.session,
    };
    if let Err(why) = check_binding(&key, &claim) {
        tracing::warn!(?why, team = %team.id, role = %b.role, "a wake call's binding was refused");
        state.devices.audit.record(AuditEntry {
            action: "team.remote.refused".into(),
            device_id: Some(from.clone()),
            device_name: Some(m::peer_name(state, from)),
            detail: Some(json!({ "team": team.id, "role": b.role, "call": "wake" })),
            ..AuditEntry::now()
        });
        return Err(refused());
    }
    let receipt = match resume_with(state, from, &b.session, &b.text, ctx.may_dangerous()).await? {
        Some(r) => r,
        // It runs there after all: the message goes to it.
        None => {
            if !message(state, &team, &b.role, &b.text).await {
                return Err(ApiError::new(
                    axum::http::StatusCode::BAD_GATEWAY,
                    "role_not_reached",
                    "The role's session did not take the message: send it again.",
                ));
            }
            crate::claude::model::MessageReceipt {
                delivery: crate::claude::model::Delivery::Sent,
                message_id: uuid::Uuid::new_v4().to_string(),
            }
        }
    };
    Ok(json!({ "receipt": receipt }))
}

/// The team is removed: the other computer takes the role's worktree and
/// record away (when it can be reached), and this computer the binding's
/// folder and refs.
pub async fn remove(state: &AppState, team: &Team, role: &str) {
    let Some(r) = team.slot(role).remote else {
        return;
    };
    let path = format!("/peer/teams/roles/{}", r.binding);
    let ok = call(
        state,
        &r.machine,
        "DELETE",
        &path,
        None,
        &acting(state, team),
        CALL,
    )
    .await
    .is_ok();
    let git = state.machines.moves.git();
    let (project, binding) = (PathBuf::from(&team.project), r.binding.clone());
    let _ = tokio::task::spawn_blocking(move || {
        let top = code::top_of(&git, &project)?;
        code::drop_task_refs(&git, &top, &binding)
    })
    .await;
    if let Some(dir) = state.machines.work.roles.dir_of(&r.binding) {
        let _ = std::fs::remove_dir_all(dir);
    }
    state.teams.forget_remote_status(&r.binding);
    audit(
        state,
        "team.remote.remove",
        team,
        role,
        &r,
        json!({ "there": ok }),
    );
}

// ─── following it ──────────────────────────────────────────────────────────────

/// Looks at every role of every team that runs on another computer (the
/// launcher's loop calls it every few seconds): its session's state into
/// the team's view, and a result there not here yet taken.
pub fn refresh_all(state: &AppState) {
    let at = now();
    for team in state.teams.all() {
        for (role, slot) in &team.roles {
            let Some(r) = &slot.remote else {
                continue;
            };
            let due = match state.teams.remote_status(&r.binding) {
                None => true,
                Some(s) => {
                    let every = if s.alive { REFRESH_MS } else { REFRESH_IDLE_MS };
                    !s.refreshing && at - s.at >= every
                }
            };
            if !due {
                continue;
            }
            state
                .teams
                .set_remote_status(&r.binding, |s| s.refreshing = true);
            let (st, team_id, role) = (state.clone(), team.id.clone(), role.clone());
            tokio::spawn(async move { refresh(&st, &team_id, &role).await });
        }
    }
}

/// One look at a role there.
pub async fn refresh(state: &AppState, team_id: &str, role: &str) {
    let Some(team) = state.teams.get(team_id) else {
        return;
    };
    let slot = team.slot(role);
    let Some(r) = slot.remote.clone() else {
        return;
    };
    let path = format!("/peer/teams/roles/{}?landed={}", r.binding, r.landed);
    let answer = moves::call(
        state,
        &r.machine,
        "GET",
        &path,
        None,
        &acting(state, &team),
        CALL,
    )
    .await;
    let at = now();
    let mut land_now = false;
    let before = state.teams.remote_status(&r.binding);
    match answer {
        Ok((200, v)) => {
            let results = v.get("results").and_then(Value::as_u64).unwrap_or(0) as u32;
            land_now = results > r.landed;
            state.teams.set_remote_status(&r.binding, |s| {
                s.alive = v.get("alive").and_then(Value::as_bool).unwrap_or(false);
                s.busy = v.get("busy").and_then(Value::as_bool).unwrap_or(false);
                s.session = v
                    .get("session")
                    .and_then(Value::as_str)
                    .filter(|x| owned::is_uuid(x))
                    .map(str::to_string);
                s.results = results;
                s.denials = v.get("denials").and_then(Value::as_u64).unwrap_or(0) as u32;
                s.error = v
                    .get("error")
                    .and_then(Value::as_str)
                    .map(|e| e.chars().take(500).collect());
                s.kept_note = v
                    .get("keptNote")
                    .and_then(Value::as_str)
                    .map(|e| e.chars().take(500).collect());
                s.unreachable = false;
                s.gone = false;
                s.refreshing = false;
                s.at = at;
            });
        }
        Ok((404, _)) => {
            // Removed there: the next start mints a new binding.
            state.teams.set_remote_status(&r.binding, |s| {
                s.alive = false;
                s.busy = false;
                s.gone = true;
                s.refreshing = false;
                s.at = at;
            });
        }
        _ => {
            state.teams.set_remote_status(&r.binding, |s| {
                s.unreachable = true;
                s.refreshing = false;
                s.at = at;
            });
        }
    }
    let after = state.teams.remote_status(&r.binding);
    let changed = before
        .as_ref()
        .map(|s| (s.alive, s.busy, s.unreachable, s.gone))
        != after
            .as_ref()
            .map(|s| (s.alive, s.busy, s.unreachable, s.gone));
    if changed {
        state.teams.nudge();
        crate::orchestration::delegation::publish_team(state, team_id).await;
    }
    if land_now {
        let (st, team_id, role) = (state.clone(), team_id.to_string(), role.to_string());
        tokio::spawn(async move {
            let _ = land(&st, &team_id, &role).await;
        });
    }
}

// ─── what comes back ───────────────────────────────────────────────────────────

/// One file of result `seq`, in chunks, checked whole.
async fn fetch_file(
    state: &AppState,
    r: &RemoteSlot,
    seq: u32,
    f: &FileSpec,
    in_dir: &Path,
) -> Result<(), String> {
    let path = moves::file_path(in_dir, &f.name).ok_or("a file has no place")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let part = chunks::part_of(&path);
    let _ = std::fs::remove_file(&part);
    let acting = m::acting::machine_header(&m::this_name(state), false);
    let mut offset = 0u64;
    while offset < f.size {
        let url = format!(
            "/peer/teams/roles/{}/result/files/{}?seq={seq}&offset={offset}",
            r.binding, f.name
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
            Ok((status, _, _)) => {
                let _ = std::fs::remove_file(&part);
                return Err(format!("it answered {status}"));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                return Err(e);
            }
        };
        let n = bytes.len() as u64;
        if offset + n > f.size {
            let _ = std::fs::remove_file(&part);
            return Err("a file is longer than it said".into());
        }
        let p = part.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&p)?;
            file.write_all(&bytes)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        offset += n;
    }
    let (d, size, sha) = (path.clone(), f.size, f.sha256.clone());
    tokio::task::spawn_blocking(move || chunks::finish(&d, size, &sha))
        .await
        .map_err(|e| e.to_string())?
}

/// The role's newest result there, fetched and made its branch here —
/// `refs/heads/mother-claude/<computer>/<team>/<role>`, never checked out,
/// never merged, moved only forward — and the board's task it was made for
/// records it. `Err`: why it is not the branch (yet).
pub async fn land(state: &AppState, team_id: &str, role: &str) -> Result<RemoteWork, String> {
    let team = state.teams.get(team_id).ok_or("The team is gone.")?;
    let r = team.slot(role).remote.ok_or("The role runs here.")?;
    {
        let mut busy = state
            .machines
            .work
            .roles
            .landing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !busy.insert(r.binding.clone()) {
            return Err("Its work is being taken already.".into());
        }
    }
    let out = land_inner(state, &team, role, &r).await;
    state
        .machines
        .work
        .roles
        .landing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&r.binding);
    out
}

async fn land_inner(
    state: &AppState,
    team: &Team,
    role: &str,
    r: &RemoteSlot,
) -> Result<RemoteWork, String> {
    let name = m::peer_name(state, &r.machine);
    let info = call(
        state,
        &r.machine,
        "GET",
        &format!("/peer/teams/roles/{}/result", r.binding),
        None,
        &m::acting::machine_header(&m::this_name(state), false),
        CALL,
    )
    .await
    .map_err(|e| e.message)?;
    let seq = info.get("seq").and_then(Value::as_u64).unwrap_or(0) as u32;
    if seq <= r.landed {
        // Here already: nothing new since its last task.
        return Ok(RemoteWork {
            machine: r.machine.clone(),
            machine_name: name,
            branch: r.branch.clone(),
            tip: r.tip.clone(),
            note: Some("Nothing new since its last result: its branch is as it was.".into()),
            at: now(),
            ..Default::default()
        });
    }
    let task = info.get("task").and_then(Value::as_u64);
    let last: TaskTip = info
        .get("last")
        .and_then(|l| serde_json::from_value(l.clone()).ok())
        .filter(|l: &TaskTip| l.validate().is_ok())
        .ok_or_else(|| format!("{name}'s result could not be read."))?;
    let base = info
        .get("base")
        .and_then(Value::as_str)
        .filter(|b| code::is_oid(b))
        .map(str::to_string)
        .ok_or_else(|| format!("{name}'s result could not be read."))?;
    let files: Vec<FileSpec> = info
        .get("files")
        .and_then(|f| serde_json::from_value(f.clone()).ok())
        .unwrap_or_default();
    let branch = code::role_branch(&name, &team.label, role)
        .ok_or("The role's branch could not be named.")?;
    let dir = state
        .machines
        .work
        .roles
        .dir_of(&r.binding)
        .ok_or("This build has no data folder.")?;
    let in_dir = dir.join(IN_DIR).join(seq.to_string());
    let _ = std::fs::remove_dir_all(&in_dir);
    std::fs::create_dir_all(&in_dir).map_err(|e| e.to_string())?;
    crate::claude::transfer::private_dir(&in_dir);
    for f in &files {
        let ok_name = moves::file_path(&in_dir, &f.name).is_some()
            && f.name != crate::claude::backup::session_pack::FILE
            && f.name != moves::ignored::FILE
            && f.sha256.len() == 64;
        if !ok_name {
            let _ = std::fs::remove_dir_all(&in_dir);
            return Err(format!(
                "{name}'s result named a file this computer does not take."
            ));
        }
        if let Err(e) = fetch_file(state, r, seq, f, &in_dir).await {
            let _ = std::fs::remove_dir_all(&in_dir);
            return Err(format!("{name}'s result could not be fetched: {e}"));
        }
    }
    let git = state.machines.moves.git();
    let (project, code_dir, binding, b, tip, base2) = (
        PathBuf::from(&team.project),
        in_dir.join("code"),
        r.binding.clone(),
        branch.clone(),
        last.tip.clone(),
        base.clone(),
    );
    let landed = tokio::task::spawn_blocking(move || {
        let top = code::top_of(&git, &project).map_err(TaskBranchError::Failed)?;
        code::land_task_branch(&git, &top, &code_dir, &binding, &b, &tip, &base2)
    })
    .await
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&in_dir);
    let short = code::short_branch(&branch).to_string();
    let mut work = RemoteWork {
        machine: r.machine.clone(),
        machine_name: name.clone(),
        branch: None,
        tip: None,
        commits: 0,
        files: 0,
        uncommitted: false,
        note: None,
        at: now(),
        ..Default::default()
    };
    match landed {
        Ok((_, l)) => {
            work.branch = Some(short.clone());
            work.tip = Some(last.tip.clone());
            work.commits = l.commits;
            work.files = l.files;
            work.uncommitted = l.uncommitted;
        }
        Err(e @ (TaskBranchError::CheckedOut { .. } | TaskBranchError::Diverged { .. })) => {
            let c = &last.tip[..12.min(last.tip.len())];
            work.tip = Some(last.tip.clone());
            work.note = Some(format!(
                "{e} Its newer work is commit {c} (`git branch <name> {c}` makes a branch of it)."
            ));
        }
        Err(TaskBranchError::Failed(e)) => {
            return Err(format!("Its work could not be taken here: {e}"));
        }
    }
    // Taken: the slot says so (and the other computer hears it with the
    // next look), and the task it was made for records it.
    let w = work.clone();
    let seq_landed = seq;
    state.teams.update(&team.id, |t| {
        if let Some(rs) = t.roles.get_mut(role).and_then(|s| s.remote.as_mut()) {
            if rs.binding == r.binding {
                rs.landed = rs.landed.max(seq_landed);
                if w.branch.is_some() {
                    rs.branch = w.branch.clone();
                    rs.tip = w.tip.clone();
                }
                rs.branch_note = w.note.clone();
            }
        }
    });
    if let Some(task) = task {
        let w = work.clone();
        let changed = state.teams.boards.mutate(&team.project, |b| {
            Ok::<_, crate::orchestration::board::BoardError>(b.set_remote(task, w, now()))
        });
        if let Ok(Some(t)) = changed {
            crate::orchestration::delegation::publish_task(state, &team.id, &t);
        }
    }
    audit(
        state,
        "team.remote.result",
        team,
        role,
        r,
        json!({
            "branch": work.branch,
            "commits": work.commits,
            "files": work.files,
            "task": task,
        }),
    );
    crate::orchestration::delegation::publish_team(state, &team.id).await;
    Ok(work)
}

// ─── the role's board calls ────────────────────────────────────────────────────

/// `POST /peer/teams/board`: a team tool the role session on another
/// computer called, with its binding.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardCall {
    pub binding: String,
    #[serde(default)]
    pub secret: String,
    pub team: String,
    pub role: String,
    #[serde(default)]
    pub session: Option<String>,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
    /// `task_done`: the result its work became there (fetched first).
    #[serde(default)]
    pub result: Option<u32>,
}

fn refused() -> ApiError {
    ApiError::forbidden(
        "binding_refused",
        "This computer runs no such team role on yours.",
    )
}

/// A board call from a role on another computer: accepted only with the
/// binding this computer minted for that team, role and computer, and the
/// role's current session; then the board's normal rules, as for a role
/// session here. `task_done` first takes its work back as the role's
/// branch, which the task records. Answers `{result}` or `{error}` (the
/// tool's own refusal).
pub async fn board_call(state: &AppState, ctx: &ClientContext, b: BoardCall) -> ApiResult<Value> {
    let Principal::Peer { machine: from, .. } = &ctx.principal else {
        return Err(ApiError::forbidden(
            "peer_only",
            "Only a paired computer can call this.",
        ));
    };
    m::serving(state)?;
    if !crate::orchestration::teams::is_team_id(&b.team) {
        return Err(refused());
    }
    let team = state.teams.get(&b.team).ok_or_else(refused)?;
    let slot = team.slot(&b.role);
    let r = slot.remote.clone().ok_or_else(refused)?;
    let key = BindingKey {
        binding: &r.binding,
        machine: &r.machine,
        team: &team.id,
        role: &b.role,
        secret_sha256: &r.secret_sha256,
        session: slot.session_id.as_deref(),
    };
    let claim = BindingClaim {
        from,
        binding: &b.binding,
        secret: &b.secret,
        team: &b.team,
        role: &b.role,
        session: b.session.as_deref().unwrap_or_default(),
    };
    if let Err(why) = check_binding(&key, &claim) {
        tracing::warn!(?why, team = %team.id, role = %b.role, "a board call's binding was refused");
        state.devices.audit.record(AuditEntry {
            action: "team.remote.refused".into(),
            device_id: Some(from.clone()),
            device_name: Some(m::peer_name(state, from)),
            detail: Some(json!({ "team": team.id, "role": b.role, "tool": b.tool })),
            ..AuditEntry::now()
        });
        return Err(refused());
    }
    if !crate::orchestration::mcp::is_team_tool(&b.tool) {
        return Err(ApiError::not_found("unknown_tool", "No such team tool."));
    }
    let session = slot.session_id.clone().unwrap_or_default();
    let work = match (b.tool.as_str(), b.result) {
        ("task_done", Some(_)) => Some(land_for_done(state, &team.id, &b.role, &r).await),
        _ => None,
    };
    let call = crate::orchestration::mcp::RemoteCall {
        machine: from.clone(),
        machine_name: m::peer_name(state, from),
        work,
    };
    let out = crate::orchestration::mcp::run_remote(
        state,
        &team,
        &b.role,
        &session,
        call,
        &b.tool,
        b.args.clone(),
    )
    .await;
    if !crate::orchestration::mcp::is_read_only(&b.tool) {
        audit(
            state,
            "team.remote.board",
            &team,
            &b.role,
            &r,
            json!({
                "tool": b.tool,
                "task": b.args.get("id"),
                "ok": out.is_ok(),
            }),
        );
    }
    // What this computer took of its results (so they are not sent again).
    let landed = state
        .teams
        .get(&team.id)
        .and_then(|t| t.slot(&b.role).remote.map(|r| r.landed))
        .unwrap_or(r.landed);
    Ok(match out {
        Ok(v) => json!({ "result": v, "landed": landed }),
        Err(e) => json!({ "error": { "code": e.code, "message": e.message }, "landed": landed }),
    })
}

/// `task_done`'s work, made the role's branch before the task finishes —
/// within [`LAND_WAIT`]; past it the branch lands in the background and
/// the task records it then.
async fn land_for_done(state: &AppState, team_id: &str, role: &str, r: &RemoteSlot) -> RemoteWork {
    let (st, t, ro) = (state.clone(), team_id.to_string(), role.to_string());
    let job = tokio::spawn(async move { land(&st, &t, &ro).await });
    let name = m::peer_name(state, &r.machine);
    let pending = |note: String| RemoteWork {
        machine: r.machine.clone(),
        machine_name: name.clone(),
        note: Some(note),
        at: now(),
        ..Default::default()
    };
    match tokio::time::timeout(LAND_WAIT, job).await {
        Ok(Ok(Ok(w))) => w,
        Ok(Ok(Err(why))) => pending(why),
        Ok(Err(e)) => pending(e.to_string()),
        Err(_) => pending(format!(
            "Its work is still on its way from {name}: the task records its branch once it is here."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_from_another_computer_is_read_with_care() {
        let t = there_of(&json!({
            "admitted": false,
            "reasons": ["3 role sessions run already\u{1b}[31m", 7],
            "session": "../../etc",
            "haves": ["a".repeat(40), "--upload-pack=x"],
            "folder": "D:\\x\u{7}y",
        }));
        assert!(!t.admitted);
        assert_eq!(
            t.reasons,
            vec!["3 role sessions run already[31m".to_string()]
        );
        assert_eq!(t.session, None, "never an id that is not a session's");
        assert_eq!(t.haves, vec!["a".repeat(40)]);
        assert_eq!(t.folder, "D:\\xy");
    }
}
