//! The computer that runs a team role for another computer's lead
//! (`/peer/teams/roles/*`, docs/PLAN-MACHINES.md Phase 4 decision 9): where
//! the project is here and whether the role may start now (this computer's
//! own `capacity::admit`, with every role session it runs), the lead's code
//! coming in (the moves' chunks) and the role's worktree at it, its owned
//! session — started with the role's charter, settings, team tools and the
//! guard as a local role is — its status, the lead's messages to it, its
//! stop, its removal, and the results its `task_done` makes.
//!
//! Every route needs a paired computer holding this computer's `run` grant
//! for the path the request came over, and Machines on here (`403
//! machines_off`); a role that runs without permission prompts also needs
//! this computer's `dangerous` grant and the acting person's right there
//! (`X-MC-Acting`). A role is only ever reached by the computer whose lead
//! started it; everything it sends is checked before git, the file system
//! or a session sees it.
//!
//! The other way, [`relay`]: the role session's team tools here act on the
//! lead's board through the peer link, with the binding.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    audit, is_binding_id, liveness, publish, HostState, Hosted, HostedRecord, RoleSpec,
    AGENTS_FILE, IN_DIR, RESULTS_DIR,
};
use crate::claude::owned::{self, Caller};
use crate::machines::chunks::{self, AppendError, Refusal};
use crate::machines::code::{self, TaskResult, TaskResultRequest, TaskTip};
use crate::machines::work::target::{check_dangerous, locate, FileSpec};
use crate::machines::work::tasks::target::{task_account, AccountAsk};
use crate::machines::{self as m, moves};
use crate::mcp::{ToolError, ToolOutput};
use crate::orchestration::capacity::{self, Admission, StartRequest};
use crate::orchestration::launcher::{self, RoleLaunch};
use crate::orchestration::router::ModelChoice;
use crate::orchestration::templates;
use crate::server::context::{ClientContext, Principal};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// At most this many files in the lead's code.
const MAX_FILES: usize = 10_000;
/// The largest file it may hold.
const MAX_FILE: u64 = 1 << 40;
/// The incoming files' list, in the role's `.in` folder.
const MANIFEST: &str = "manifest.json";
/// How long a stopped session gets to end.
const STOP_WAIT: Duration = Duration::from_secs(15);
/// How long a board call through the lead may take (the tool's own limit
/// is 120 s).
const RELAY: Duration = Duration::from_secs(110);
/// The largest set of role definitions.
const MAX_AGENTS_BYTES: usize = 256 * 1024;

fn bad(m: &str) -> ApiError {
    ApiError::unprocessable("role_bad_request", m.to_string())
}

fn mismatch() -> ApiError {
    ApiError::conflict(
        "binding_mismatch",
        "That role binding belongs to another team, role or computer here.",
    )
}

/// The role `binding` that `from`'s lead started here (`404
/// binding_unknown` otherwise, as if there were none).
fn hosted_of(state: &AppState, binding: &str, from: &str) -> ApiResult<Arc<Hosted>> {
    let unknown = || ApiError::not_found("binding_unknown", "No such team role here.");
    if !is_binding_id(binding) {
        return Err(unknown());
    }
    state
        .machines
        .work
        .roles
        .get(binding)
        .filter(|h| h.peek(|r| r.lead == from))
        .ok_or_else(unknown)
}

/// Mother Claude's tools must be on here: the role reaches the board
/// through them.
fn tools_on(state: &AppState) -> ApiResult<()> {
    if state.mcp.enabled() {
        return Ok(());
    }
    Err(ApiError::conflict(
        "tools_off",
        format!(
            "Claude Code tools are off on {}: the role reaches the board through them. Turn \
             them on there (Settings › Services › Claude Code tools).",
            m::this_name(state)
        ),
    ))
}

/// A label made safe to keep and show.
fn clean_label(s: &str, n: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(n)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The worktree's folder name: the project's, made safe (never a dot
/// first, so never one of the role's own folders).
fn tree_name(project_name: &str, project: &str) -> String {
    let name = if project_name.trim().is_empty() {
        crate::machines::setup::folders::name_of(project)
    } else {
        project_name.to_string()
    };
    code::safe_name(&name)
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// ─── may it start, and where ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareBody {
    pub binding: String,
    pub team: String,
    pub role: String,
    pub project: String,
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub account: AccountAsk,
    /// The model the lead's router chose (for admission).
    pub model: String,
    #[serde(default)]
    pub priority: u8,
    /// It goes on with a task it holds: its worktree stays as it is.
    #[serde(default)]
    pub held: bool,
    /// Its last session, resumed when its conversation is here.
    #[serde(default)]
    pub resume: Option<String>,
}

fn check_ids(binding: &str, team: &str, role: &str) -> ApiResult<()> {
    if !is_binding_id(binding) {
        return Err(bad("The role binding."));
    }
    if !crate::orchestration::teams::is_team_id(team) {
        return Err(bad("The team."));
    }
    if !templates::is_id(role, templates::ROLE_ID_MAX) {
        return Err(bad("The role."));
    }
    Ok(())
}

/// Whether the worktree holds nothing the lead has not got back: as its
/// last result left it, or as the code it got.
async fn unchanged(state: &AppState, r: &HostedRecord) -> bool {
    let (Some(base), Some(base_tree)) = (r.base.clone(), r.base_tree.clone()) else {
        return true;
    };
    let folder = PathBuf::from(&r.folder);
    if !folder.join(".git").is_file() {
        return true;
    }
    let (git, last) = (state.machines.moves.git(), r.last.clone());
    tokio::task::spawn_blocking(move || {
        code::task_unchanged(&git, &folder, &base, &base_tree, last.as_ref())
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(false)
}

/// Whether `id` was one of the role's sessions here and its conversation
/// can be resumed (its transcript is here, nothing else holds it).
async fn resumable(state: &AppState, r: &HostedRecord, id: &str) -> bool {
    let ours = r.session.as_deref() == Some(id) || r.previous.iter().any(|p| p == id);
    ours && owned::is_uuid(id)
        && state.transcript_path(id).await.is_some_and(|p| p.is_file())
        && !owned::live_elsewhere(state, id).await
}

/// `POST /peer/teams/roles/prepare`: may the role start here now (this
/// computer's admission, with every role session it runs), where its
/// worktree is, under which account, whether the lead's newer code would
/// be taken, and what a snapshot may leave out. Nothing is written but the
/// admission's own record.
pub async fn prepare(state: &AppState, from: &str, b: PrepareBody) -> ApiResult<Value> {
    check_ids(&b.binding, &b.team, &b.role)?;
    if owned::validate_model(&b.model).ok().as_deref() != Some(b.model.as_str()) {
        return Err(bad("The model."));
    }
    if b.priority > 3 || b.resume.as_deref().is_some_and(|r| !owned::is_uuid(r)) {
        return Err(bad("The priority or the session."));
    }
    tools_on(state)?;
    let checkout = locate(state, from, &b.project).await?;
    let (account, account_name) = task_account(state, &b.account)?;
    let known = state.machines.work.roles.get(&b.binding);
    if let Some(h) = &known {
        let r = h.record();
        if r.lead != from || r.team != b.team || r.role != b.role {
            return Err(mismatch());
        }
        if liveness(state, &r).0 {
            return Ok(json!({
                "admitted": true,
                "alive": true,
                "known": true,
                "session": r.session,
                "folder": r.folder,
                "checkout": r.checkout,
                "os": crate::machines::protocol::this_os(),
                "account": account_name,
            }));
        }
    }
    let running = launcher::running_workers(state).await;
    let req = StartRequest {
        project: checkout.display().to_string(),
        role: b.role.clone(),
        model: b.model.clone(),
        priority: b.priority,
        account: Some(account),
        lead: false,
    };
    if let Admission::Wait { reasons } = capacity::admit(state, &req, &running) {
        return Ok(json!({
            "admitted": false,
            "reasons": reasons,
            "known": known.is_some(),
        }));
    }
    let record = known.as_ref().map(|h| h.record());
    let folder = match record.as_ref().filter(|r| !r.folder.is_empty()) {
        Some(r) => PathBuf::from(&r.folder),
        None => state
            .machines
            .work
            .roles
            .dir_of(&b.binding)
            .ok_or_else(|| ApiError::internal("This build has no data folder."))?
            .join(tree_name(&b.project_name, &b.project)),
    };
    let can_resume = match (&b.resume, &record) {
        (Some(id), Some(r)) => resumable(state, r, id).await,
        _ => false,
    };
    let wants_code = match &record {
        None => true,
        Some(r) => r.base.is_none() || (!b.held && unchanged(state, r).await),
    };
    let git = state.machines.moves.git();
    let (c, w) = (checkout.clone(), b.binding.clone());
    let haves = tokio::task::spawn_blocking(move || code::haves(&git, &c, Some(&w)))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::unprocessable("role_failed", e.to_string()))?;
    Ok(json!({
        "admitted": true,
        "alive": false,
        "known": record.is_some(),
        "resumable": can_resume,
        "wantsCode": wants_code,
        "checkout": checkout.display().to_string(),
        "folder": folder.display().to_string(),
        "os": crate::machines::protocol::this_os(),
        "account": account_name,
        "haves": haves.haves,
        "shallow": haves.shallow,
    }))
}

// ─── the lead's code coming in ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginBody {
    pub binding: String,
    /// The binding's secret, when the lead mints it (kept here; the lead
    /// keeps its hash). A binding this computer knows needs none.
    #[serde(default)]
    pub secret: Option<String>,
    pub team: String,
    #[serde(default)]
    pub team_label: String,
    pub role: String,
    #[serde(default)]
    pub role_title: String,
    pub project: String,
    #[serde(default)]
    pub project_name: String,
    /// Commits the lead has (results leave them out).
    #[serde(default)]
    pub haves: Vec<String>,
    pub files: Vec<FileSpec>,
}

fn check_files(files: &[FileSpec], dir: &Path) -> ApiResult<()> {
    if files.len() > MAX_FILES {
        return Err(bad("Too many files."));
    }
    let mut seen = std::collections::HashSet::new();
    for f in files {
        if moves::file_path(dir, &f.name).is_none()
            || f.name == crate::claude::backup::session_pack::FILE
            || f.name == moves::ignored::FILE
            || !seen.insert(f.name.as_str())
        {
            return Err(bad("A file this computer does not take."));
        }
        if f.size > MAX_FILE || !is_sha256(&f.sha256) {
            return Err(bad("A file's size or hash."));
        }
    }
    if !files.iter().any(|f| f.name == code::META) {
        return Err(bad("The code has no record."));
    }
    Ok(())
}

fn check_begin(b: &BeginBody) -> ApiResult<()> {
    check_ids(&b.binding, &b.team, &b.role)?;
    if b.secret.as_deref().is_some_and(|s| !is_sha256(s)) {
        return Err(bad("The binding's secret."));
    }
    if b.project.is_empty()
        || b.project.len() > 4096
        || !crate::claude::paths::is_absolute_anywhere(&b.project)
        || b.project_name.len() > 300
        || b.team_label.len() > 200
        || b.role_title.len() > 200
    {
        return Err(bad("The project or the team."));
    }
    if b.haves.len() > code::MAX_HAVES || b.haves.iter().any(|h| !code::is_oid(h)) {
        return Err(bad("The commits the lead has."));
    }
    Ok(())
}

fn have_of(dir: &Path, name: &str) -> u64 {
    moves::file_path(dir, name).map_or(0, |p| chunks::have(&p))
}

/// `POST /peer/teams/roles`: the lead's code is coming for a role (made
/// here the first time, with its binding). Answers what arrived of each
/// file.
pub async fn begin(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    b: BeginBody,
) -> ApiResult<Value> {
    check_begin(&b)?;
    let roles = &state.machines.work.roles;
    let dir = roles
        .dir_of(&b.binding)
        .ok_or_else(|| ApiError::internal("This build has no data folder."))?;
    let in_dir = dir.join(IN_DIR);
    check_files(&b.files, &in_dir)?;
    let checkout = locate(state, from, &b.project).await?;
    let via_lan = matches!(ctx.principal, Principal::Peer { lan: true, .. });
    match roles.get(&b.binding) {
        Some(h) => {
            let r = h.record();
            if r.lead != from
                || r.team != b.team
                || r.role != b.role
                || b.secret
                    .as_deref()
                    .is_some_and(|s| !super::same_text(&r.secret, s))
            {
                return Err(mismatch());
            }
            if liveness(state, &r).0 {
                return Err(ApiError::conflict(
                    "role_running",
                    "The role's session runs here: no new code while it works.",
                ));
            }
            h.update(|r| {
                r.haves = b.haves.clone();
                r.checkout = checkout.display().to_string();
                r.via_lan = Some(via_lan);
            });
            roles.save(&h);
        }
        None => {
            let Some(secret) = b.secret.clone() else {
                return Err(bad("A new role binding comes with its secret."));
            };
            let now = m::now();
            let folder = dir.join(tree_name(&b.project_name, &b.project));
            let record = HostedRecord {
                binding: b.binding.clone(),
                lead: from.to_string(),
                lead_name: m::peer_name(state, from),
                team: b.team.clone(),
                team_label: clean_label(&b.team_label, 64),
                role: b.role.clone(),
                role_title: clean_label(&b.role_title, 60),
                project: b.project.clone(),
                project_name: if b.project_name.is_empty() {
                    crate::machines::setup::folders::name_of(&b.project)
                } else {
                    clean_label(&b.project_name, 200)
                },
                checkout: checkout.display().to_string(),
                folder: folder.display().to_string(),
                session: None,
                previous: Vec::new(),
                state: HostState::Sending,
                error: None,
                permission_mode: String::new(),
                model: None,
                effort: None,
                account_name: None,
                paths: Vec::new(),
                read_only: false,
                dangerous: false,
                denials: 0,
                results: 0,
                result_task: None,
                landed: 0,
                kept_note: None,
                created_at: now,
                started_at: None,
                stopped_at: None,
                heard_at: now,
                via_lan: Some(via_lan),
                updated_at: now,
                secret,
                account: None,
                base: None,
                base_tree: None,
                haves: b.haves.clone(),
                last: None,
                result_files: Vec::new(),
                result_base: None,
                spec: None,
            };
            let h = roles
                .insert(record)
                .ok_or_else(|| ApiError::conflict("role_exists", "That role is known here."))?;
            roles.save(&h);
            audit(state, "machine.role.asked", &h.record());
            publish(state);
        }
    }
    let _ = std::fs::remove_dir_all(&in_dir);
    std::fs::create_dir_all(&in_dir).map_err(|e| ApiError::internal(e.to_string()))?;
    crate::claude::transfer::private_dir(&dir);
    crate::claude::transfer::private_dir(&in_dir);
    crate::persist::write_json_private(&in_dir.join(MANIFEST), &b.files)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let files: Vec<Value> = b
        .files
        .iter()
        .map(|f| json!({ "name": f.name, "have": have_of(&in_dir, &f.name).min(f.size) }))
        .collect();
    Ok(json!({ "files": files }))
}

/// `PUT /peer/teams/roles/{binding}/files/{name}?offset=N`: the next chunk.
pub async fn put_chunk(
    state: &AppState,
    from: &str,
    binding: &str,
    name: &str,
    offset: u64,
    bytes: axum::body::Bytes,
) -> ApiResult<Value> {
    let h = hosted_of(state, binding, from)?;
    let in_dir = state
        .machines
        .work
        .roles
        .dir_of(&h.binding)
        .map(|d| d.join(IN_DIR))
        .ok_or_else(|| ApiError::internal("This build has no data folder."))?;
    let manifest: Option<Vec<FileSpec>> = crate::persist::read_json(&in_dir.join(MANIFEST));
    let f = manifest
        .and_then(|l| l.into_iter().find(|f| f.name == name))
        .ok_or_else(|| ApiError::not_found("not_found", "No such file in this code."))?;
    let path = moves::file_path(&in_dir, name)
        .ok_or_else(|| ApiError::unprocessable("role_bad_request", "No such file."))?;
    let _one = state.machines.work.roles.chunks.lock().await;
    let written =
        tokio::task::spawn_blocking(move || chunks::append(&path, offset, &bytes, f.size))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .map_err(|e| match e {
                AppendError::Refused(Refusal::Offset(size)) => ApiError::conflict(
                    "offset_mismatch",
                    "This computer has another part of the file.",
                )
                .with("size", size),
                AppendError::Refused(Refusal::TooLong) => ApiError::unprocessable(
                    "role_bad_request",
                    "The file is longer than the lead said.",
                ),
                AppendError::Refused(Refusal::TooLarge) => {
                    ApiError::unprocessable("role_bad_request", "A chunk is at most 8 MiB.")
                }
                AppendError::Io(e) => {
                    ApiError::internal(format!("The file could not be written here: {e}"))
                }
            })?;
    Ok(json!({ "name": name, "size": written }))
}

/// The code that came in, fetched into this computer's checkout and the
/// role's worktree set to it — made the first time; later moved to the
/// lead's newer code only when it holds nothing the lead has not got back
/// and the role does not go on with a task it holds (else it keeps its own
/// work, and says so).
async fn take_code(state: &AppState, h: &Arc<Hosted>, held: bool) -> ApiResult<()> {
    let r = h.record();
    let in_dir = state
        .machines
        .work
        .roles
        .dir_of(&h.binding)
        .map(|d| d.join(IN_DIR))
        .ok_or_else(|| ApiError::internal("This build has no data folder."))?;
    let files: Vec<FileSpec> =
        crate::persist::read_json(&in_dir.join(MANIFEST)).ok_or_else(|| {
            ApiError::conflict(
                "role_incomplete",
                "The lead's code did not come here: send it again.",
            )
        })?;
    let d = in_dir.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        for f in &files {
            let path = moves::file_path(&d, &f.name).ok_or("a file has no place")?;
            chunks::finish(&path, f.size, &f.sha256)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|why| {
        ApiError::conflict(
            "role_incomplete",
            format!("The lead's code did not arrive whole: {why}."),
        )
    })?;
    let fresh = r.base.is_none() || !Path::new(&r.folder).join(".git").is_file();
    // Nothing in it the lead has not got back (as its last result left it,
    // or as the code it got)…
    let returned = !fresh && !held && unchanged(state, &r).await;
    let git = state.machines.moves.git();
    let (checkout, code_dir, folder, binding, last_tip) = (
        PathBuf::from(&r.checkout),
        in_dir.join("code"),
        PathBuf::from(&r.folder),
        h.binding.clone(),
        r.last.as_ref().map(|l| l.tip.clone()),
    );
    let made = tokio::task::spawn_blocking(move || {
        let f = code::fetch_snapshot(&git, &checkout, &code_dir, &binding)?;
        // …and the lead's newer code holds what came back (it merged the
        // role's branch): only then is the role's own work not lost.
        let merged = match &last_tip {
            Some(tip) => code::contains(&git, &checkout, tip, &f.meta.snap)?,
            None => true,
        };
        let moved = fresh || (returned && merged);
        if moved {
            code::reset_work_tree(&git, &checkout, &folder, &f.meta.snap)?;
        }
        Ok::<_, code::CodeError>((f.meta.snap, f.meta.wt_tree, moved))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let _ = std::fs::remove_dir_all(&in_dir);
    let (snap, tree, may_move) = match made {
        Ok(x) => x,
        Err(e) => {
            let why = format!("The lead's code could not be set up here: {e}");
            h.update(|r| {
                r.state = HostState::Error;
                r.error = Some(why.clone());
            });
            state.machines.work.roles.save(h);
            publish(state);
            return Err(ApiError::unprocessable("role_failed", why));
        }
    };
    h.update(|r| {
        if may_move {
            r.base = Some(snap.clone());
            r.base_tree = Some(tree.clone());
            // The next result still descends from the last one (the branch
            // only moves forward), and counts as unchanged until the role
            // changes something in the newer code.
            r.last = r.last.take().map(|l| TaskTip {
                head: Some(snap.clone()),
                wt_tree: tree.clone(),
                snap: l.snap,
                tip: l.tip,
            });
            r.kept_note = None;
        } else {
            r.kept_note = Some(if held {
                "It goes on with the task it holds, in its worktree as it left it.".into()
            } else {
                "Its worktree holds work the lead has not merged yet: it kept it rather than take \
                 the lead's newer code (once the lead merges the role's branch, its next start \
                 takes it)."
                    .into()
            });
        }
    });
    state.machines.work.roles.save(h);
    Ok(())
}

// ─── its session ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartBody {
    /// The id its session runs under: a new one, or `resume`'s.
    pub session_id: String,
    /// Resume this session's conversation (it must be here).
    #[serde(default)]
    pub resume: Option<String>,
    /// It goes on with a task it holds.
    #[serde(default)]
    pub held: bool,
    /// The lead sent its code (`POST /peer/teams/roles` and the files).
    #[serde(default)]
    pub code: bool,
    pub spec: RoleSpec,
    /// Its first message: its task packet, or nothing.
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub account: AccountAsk,
}

fn check_relative(list: &[String], globs: bool) -> bool {
    list.len() <= 100
        && list
            .iter()
            .all(|p| templates::is_inside_relative(p) && (globs || !p.contains(['*', '?', '['])))
}

/// A role spec from another computer: every field checked before a session
/// sees it.
fn check_spec(s: &RoleSpec) -> ApiResult<()> {
    let name_ok = !s.session_name.is_empty()
        && s.session_name.len() <= 120
        && !s.session_name.starts_with(['-', '/', '.'])
        && s.session_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    if !name_ok {
        return Err(bad("The session's name."));
    }
    if s.charter.chars().count() > templates::CHARTER_MAX + 10_000 || s.charter.contains('\0') {
        return Err(bad("The charter."));
    }
    if let Some(a) = &s.agents {
        let ok = a.as_object().is_some_and(|o| {
            o.len() <= templates::MAX_ROLES
                && o.iter()
                    .all(|(k, v)| templates::is_id(k, templates::ROLE_ID_MAX) && v.is_object())
        }) && serde_json::to_vec(a).is_ok_and(|b| b.len() <= MAX_AGENTS_BYTES);
        if !ok {
            return Err(bad("The role definitions."));
        }
    }
    if !check_relative(&s.paths, true) || !check_relative(&s.add_dirs, false) {
        return Err(bad("The role's paths."));
    }
    if !owned::PERMISSION_MODES.contains(&s.permission_mode.as_str()) {
        return Err(bad("The permission mode."));
    }
    if owned::validate_model(&s.model).ok().as_deref() != Some(s.model.as_str()) {
        return Err(bad("The model."));
    }
    if s.effort
        .as_deref()
        .is_some_and(|e| !owned::EFFORT_LEVELS.contains(&e))
    {
        return Err(bad("The effort."));
    }
    if s.fallback_model.as_deref().is_some_and(|f| {
        f.len() > 300
            || f.split(',')
                .any(|m| owned::validate_model(m.trim()).ok().as_deref() != Some(m.trim()))
    }) {
        return Err(bad("The fallback model."));
    }
    if s.budget_usd
        .is_some_and(|b| !(b > 0.0 && b <= templates::BUDGET_MAX_USD))
    {
        return Err(bad("The budget."));
    }
    if s.priority > 3 {
        return Err(bad("The priority."));
    }
    Ok(())
}

/// Writes the team's role definitions next to the role's worktree (0600).
fn write_agents(state: &AppState, binding: &str, agents: Option<&Value>) -> Option<PathBuf> {
    let a = agents?;
    let path = state.machines.work.roles.dir_of(binding)?.join(AGENTS_FILE);
    match crate::persist::write_json_private(&path, a) {
        Ok(()) => Some(path),
        Err(e) => {
            tracing::warn!(binding, error = %e, "a remote role's definitions could not be written");
            None
        }
    }
}

/// `POST /peer/teams/roles/{binding}/start`: the lead's code (when it sent
/// some) set up, this computer's admission asked again, then the role's
/// owned session started in its worktree — the same options a local role
/// gets (its name, charter, `{"crossSessionInbound":"accept"}`, the team
/// tools by exact name, the guard's callback, the role definitions) — or
/// resumed there. Records the spawn until the app turns task sessions on.
pub async fn start(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    binding: &str,
    b: StartBody,
) -> ApiResult<Value> {
    let h = hosted_of(state, binding, from)?;
    check_spec(&b.spec)?;
    if !owned::is_uuid(&b.session_id)
        || b.resume.as_deref().is_some_and(|r| r != b.session_id)
        || b.prompt.len() > owned::MAX_TEXT_BYTES
        || b.prompt.contains('\0')
        || b.account.name.len() > 200
    {
        return Err(bad("The session, its first message or the account."));
    }
    let dangerous = owned::is_dangerous_mode(Some(&b.spec.permission_mode));
    check_dangerous(state, ctx, dangerous)?;
    tools_on(state)?;
    let _one = h.work.lock().await;
    let r = h.record();
    if liveness(state, &r).0 {
        return Ok(json!({
            "state": "started",
            "session": r.session,
            "folder": r.folder,
            "status": status_view(state, &h),
        }));
    }
    if b.code {
        h.update(|r| r.state = HostState::Starting);
        publish(state);
        take_code(state, &h, b.held).await?;
    }
    let r = h.record();
    if r.base.is_none() {
        return Err(ApiError::conflict(
            "role_no_code",
            "The lead's code is not here yet: send it first.",
        ));
    }
    let (account, account_name) = task_account(state, &b.account)?;
    let running = launcher::running_workers(state).await;
    let req = StartRequest {
        project: r.checkout.clone(),
        role: r.role.clone(),
        model: b.spec.model.clone(),
        priority: b.spec.priority,
        account: Some(account.clone()),
        lead: false,
    };
    if let Admission::Wait { reasons } = capacity::admit(state, &req, &running) {
        h.update(|r| {
            if r.state != HostState::Error {
                r.state = HostState::Stopped;
            }
        });
        publish(state);
        return Ok(json!({ "state": "waiting", "reasons": reasons }));
    }
    if let Some(id) = &b.resume {
        if !resumable(state, &r, id).await {
            return Err(ApiError::conflict(
                "role_not_resumable",
                "That conversation can't be resumed here now.",
            ));
        }
    }
    let may_dangerous = dangerous && ctx.may_dangerous();
    let mode = owned::launch_mode(
        Some(&b.spec.permission_mode),
        Caller {
            may_dangerous,
            // The folder is the role's own worktree, in our data folder.
            desktop: true,
        },
    )?;
    let account_env = state.accounts.env(&account).map_err(owned::account_error)?;
    let folder = PathBuf::from(&r.folder);
    let launch = RoleLaunch {
        name: b.spec.session_name.clone(),
        charter: b.spec.charter.clone(),
        mode: mode.clone(),
        read_only: b.spec.read_only,
        add_dirs: b
            .spec
            .add_dirs
            .iter()
            .map(|d| folder.join(d).to_string_lossy().into_owned())
            .collect(),
        account: account.clone(),
        may_dangerous,
    };
    let agents = write_agents(state, binding, b.spec.agents.as_ref());
    let choice = ModelChoice::new(&b.spec.model, b.spec.effort.as_deref());
    let resume = b.resume.clone().map(|id| (id, false));
    let mut opts = launcher::launch_options(
        &launch,
        &choice,
        r.folder.clone(),
        resume.clone(),
        b.resume.is_none().then(|| b.session_id.clone()),
        None,
        agents.clone(),
        b.prompt.clone(),
        b.spec.budget_usd,
    );
    opts.account_env = account_env;
    opts.fallback_model = b.spec.fallback_model.clone();
    let session = b.session_id.clone();
    if !state.machines.work.tasks.sessions_live() {
        // Checked as it would be, then only recorded.
        crate::machines::lease::check_launch(state, &session)
            .map_err(|e| ApiError::conflict("role_not_started", e.to_string()))?;
        let argv = crate::claude::control::headless_args(&session, &opts, &mode);
        state.machines.work.roles.record_spawn(json!({
            "binding": h.binding,
            "team": r.team,
            "role": r.role,
            "session": session,
            "resume": b.resume,
            "cwd": opts.cwd,
            "folder": r.folder,
            "name": opts.name,
            "settings": opts.settings_json,
            "allowedTools": opts.allowed_tools,
            "disallowedTools": opts.disallowed_tools,
            "agentsFile": agents.as_ref().map(|p| p.display().to_string()),
            "charter": opts.append_system_prompt,
            "guardHooks": opts.guard_hooks,
            "paths": b.spec.paths,
            "readOnly": b.spec.read_only,
            "permissionMode": mode,
            "model": opts.model,
            "effort": opts.effort,
            "account": account,
            "accountName": account_name,
            "prompt": b.prompt,
            "argv": argv,
        }));
    } else {
        state
            .control
            .spawn(state, opts)
            .await
            .map_err(owned::spawn_error)?;
    }
    let now = m::now();
    let spec = b.spec.clone();
    h.update(|r| {
        if let Some(old) = r.session.take().filter(|o| *o != session) {
            r.previous.push(old);
            if r.previous.len() > 10 {
                let excess = r.previous.len() - 10;
                r.previous.drain(..excess);
            }
        }
        r.session = Some(session.clone());
        r.state = HostState::Running;
        r.error = None;
        r.started_at = Some(now);
        r.stopped_at = None;
        r.heard_at = now;
        r.permission_mode = mode.clone();
        r.model = Some(spec.model.clone());
        r.effort = spec.effort.clone();
        r.paths = spec.paths.clone();
        r.read_only = spec.read_only;
        r.dangerous = dangerous;
        r.account = Some(account.clone());
        r.account_name = Some(account_name.clone());
        r.via_lan = Some(matches!(ctx.principal, Principal::Peer { lan: true, .. }));
        r.spec = Some(spec);
    });
    state.machines.work.roles.save(&h);
    audit(state, "machine.role.start", &h.record());
    publish(state);
    Ok(json!({
        "state": "started",
        "session": session,
        "folder": r.folder,
        "base": h.peek(|r| r.base.clone()),
        "status": status_view(state, &h),
    }))
}

// ─── its status, messages, stop and removal ────────────────────────────────────

/// A role's status as the lead reads it.
pub fn status_view(state: &AppState, h: &Hosted) -> Value {
    let r = h.record();
    let (alive, busy) = liveness(state, &r);
    json!({
        "binding": r.binding,
        "state": r.state,
        "alive": alive,
        "busy": busy,
        "session": r.session,
        "results": r.results,
        "resultTask": r.result_task,
        "landed": r.landed,
        "denials": r.denials,
        "error": r.error,
        "keptNote": r.kept_note,
        "folder": r.folder,
        "checkout": r.checkout,
        "accountName": r.account_name,
        "startedAt": r.started_at,
        "stoppedAt": r.stopped_at,
    })
}

#[derive(Debug, Default, Deserialize)]
pub struct StatusQuery {
    /// The results the lead took.
    #[serde(default)]
    pub landed: Option<u32>,
}

/// `GET /peer/teams/roles/{binding}?landed=`: its status; `landed` tells
/// this computer which results the lead took.
pub async fn status(
    state: &AppState,
    from: &str,
    binding: &str,
    q: StatusQuery,
) -> ApiResult<Value> {
    let h = hosted_of(state, binding, from)?;
    let now = m::now();
    let landed = q
        .landed
        .filter(|n| h.peek(|r| *n > r.landed && *n <= r.results));
    h.update(|r| {
        r.heard_at = now;
        if let Some(n) = landed {
            r.landed = n;
        }
    });
    if landed.is_some() {
        state.machines.work.roles.save(&h);
    }
    Ok(status_view(state, &h))
}

#[derive(Debug, Deserialize)]
pub struct MessageBody {
    pub text: String,
}

/// `POST /peer/teams/roles/{binding}/message {text}`: the lead's message to
/// the role's session (its next task's packet, a board notice), queued
/// behind a running turn as any message to an owned session is.
pub async fn message(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    binding: &str,
    b: MessageBody,
) -> ApiResult<Value> {
    let h = hosted_of(state, binding, from)?;
    if b.text.trim().is_empty() || b.text.len() > owned::MAX_TEXT_BYTES || b.text.contains('\0') {
        return Err(bad("The message."));
    }
    let r = h.record();
    let session = match (liveness(state, &r).0, r.session.clone()) {
        (true, Some(s)) => s,
        _ => {
            return Err(ApiError::conflict(
                "role_not_running",
                "The role's session is not running here.",
            ))
        }
    };
    // Whatever it is told then runs with nobody approving it.
    check_dangerous(state, ctx, r.dangerous)?;
    let now = m::now();
    h.update(|r| r.heard_at = now);
    if state.machines.work.tasks.sessions_live() {
        owned::send(state, &session, &b.text, &[]).await?;
    } else {
        state.machines.work.roles.record_message(json!({
            "binding": h.binding,
            "session": session,
            "text": b.text,
        }));
    }
    Ok(json!({ "delivered": true }))
}

#[derive(Debug, Default, Deserialize)]
pub struct StopBody {
    #[serde(default)]
    pub why: String,
}

/// Stops a role's session here (between turns or not); its worktree and
/// record stay, so the lead can start it again.
pub(crate) async fn stop_here(state: &AppState, h: &Arc<Hosted>, why: &str) {
    let r = h.record();
    if let Some(s) = r.session.as_deref() {
        if liveness(state, &r).0 {
            if state.machines.work.tasks.sessions_live() {
                let _ = state.control.stop_and_wait(s, STOP_WAIT).await;
                crate::pending::withdraw_session(state, s, why).await;
            } else {
                state.machines.work.roles.record_stop(s);
            }
        }
    }
    let now = m::now();
    h.update(|r| {
        if r.state != HostState::Error {
            r.state = HostState::Stopped;
        }
        r.stopped_at = Some(now);
    });
    state.machines.work.roles.save(h);
    audit(state, "machine.role.stop", &h.record());
    publish(state);
}

/// `POST /peer/teams/roles/{binding}/stop {why}`: the lead stops it (a
/// stop by hand, its idle stop, the team's stop).
pub async fn stop(state: &AppState, from: &str, binding: &str, b: StopBody) -> ApiResult<Value> {
    let h = hosted_of(state, binding, from)?;
    let why = clean_label(&b.why, 200);
    let why = if why.is_empty() {
        "its team's lead stopped it".to_string()
    } else {
        why
    };
    stop_here(state, &h, &why).await;
    Ok(status_view(state, &h))
}

/// Takes a role away here: its session stopped, its worktree and refs
/// removed, its record and folder gone.
pub(crate) async fn remove_here(state: &AppState, h: &Arc<Hosted>) {
    stop_here(state, h, "its team no longer runs it here").await;
    let r = h.record();
    audit(state, "machine.role.remove", &r);
    state.machines.work.roles.forget(&h.binding);
    let git = state.machines.moves.git();
    let (checkout, folder, binding) = (
        PathBuf::from(&r.checkout),
        PathBuf::from(&r.folder),
        r.binding.clone(),
    );
    let done = tokio::task::spawn_blocking(move || {
        code::remove_task_tree(&git, &checkout, &folder, &binding)
    })
    .await;
    if let Ok(Err(e)) = done {
        tracing::warn!(binding = %r.binding, error = %e, "a role's worktree could not be removed");
    }
    if let Some(dir) = state.machines.work.roles.dir_of(&r.binding) {
        let _ = std::fs::remove_dir_all(dir);
    }
    publish(state);
}

/// `DELETE /peer/teams/roles/{binding}`: the lead's team no longer runs the
/// role here (it was removed).
pub async fn remove(state: &AppState, from: &str, binding: &str) -> ApiResult<Value> {
    let h = hosted_of(state, binding, from)?;
    remove_here(state, &h).await;
    Ok(json!({ "removed": true }))
}

/// Stops what `machine` runs here (all computers' with `None`): Machines was
/// turned off, the computer revoked, or its `run` grant taken back.
pub fn stop_from(state: &AppState, machine: Option<&str>, why: &str) {
    for h in state.machines.work.roles.all() {
        let hit = h.peek(|r| machine.is_none_or(|m| m == r.lead)) && liveness(state, &h.record()).0;
        if hit {
            let (st, why) = (state.clone(), why.to_string());
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move { stop_here(&st, &h, &why).await });
            }
        }
    }
}

/// After `machine`'s grants changed here: each role it runs here stops when
/// the `run` grant's column that let it in is gone.
pub fn grants_changed(state: &AppState, machine: &str) {
    use crate::machines::grants::Grant;
    for h in state.machines.work.roles.all() {
        let (hit, lan) = h.peek(|r| (r.lead == machine, r.via_lan));
        if !hit || !liveness(state, &h.record()).0 {
            continue;
        }
        let may = match lan {
            Some(lan) => state.machines.allows(machine, Grant::Run, lan),
            None => {
                state.machines.allows(machine, Grant::Run, true)
                    || state.machines.allows(machine, Grant::Run, false)
            }
        };
        if !may {
            let st = state.clone();
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move {
                    stop_here(&st, &h, "This computer took back the Run permission.").await
                });
            }
        }
    }
}

/// The lead's launcher stops its roles' idle sessions itself (the
/// template's minutes). One whose lead has not been heard from for the
/// launcher's default minutes — its computer is off or away — and that sits
/// idle with nothing to do (no turn, approval, queued message, background
/// work or wakeup) is stopped here, by the same rule
/// (`launcher::idle_stop_due`); the lead starts it again when it has work.
/// Answers the bindings stopped.
pub async fn stop_unattended(state: &AppState, now: i64) -> Vec<String> {
    let minutes = templates::DEFAULT_IDLE_STOP_MINUTES;
    let mut out = Vec::new();
    for h in state.machines.work.roles.all() {
        let r = h.record();
        let Some(session) = r.session.clone() else {
            continue;
        };
        let heard = r.heard_at.max(r.started_at.unwrap_or(0));
        if !liveness(state, &r).0 || !launcher::idle_stop_due(heard, now, minutes) {
            continue;
        }
        let idle = state.pending.list(&session).is_empty()
            && (!state.machines.work.tasks.sessions_live()
                || (state.control.is_idle(&session) == Some(true)
                    && state.activity.between(&session).is_none()));
        if !idle {
            continue;
        }
        stop_here(
            state,
            &h,
            "its team's lead was not heard from while it sat idle",
        )
        .await;
        out.push(h.binding.clone());
    }
    out
}

// ─── what it did ───────────────────────────────────────────────────────────────

/// What the role's session did since the last result, taken for task `task`
/// (a snapshot of its worktree, its bundle ready for the lead): the newest
/// result the lead has not taken yet — this one, or an earlier one when
/// nothing changed since — or `None`.
pub(crate) async fn collect(
    state: &AppState,
    h: &Arc<Hosted>,
    task: Option<u64>,
) -> Result<Option<u32>, String> {
    let _one = h.work.lock().await;
    let r = h.record();
    let (Some(base), Some(base_tree), Some(dir)) = (
        r.base.clone(),
        r.base_tree.clone(),
        state.machines.work.roles.dir_of(&h.binding),
    ) else {
        return Ok(None);
    };
    let seq = r.results + 1;
    let results = dir.join(RESULTS_DIR);
    let res_dir = results.join(seq.to_string());
    let _ = std::fs::remove_dir_all(&res_dir);
    // The lead has the last result once it took it.
    let mut haves = r.haves.clone();
    if r.landed >= r.results {
        if let Some(l) = &r.last {
            haves.insert(0, l.snap.clone());
            haves.insert(0, l.tip.clone());
        }
    }
    let req = TaskResultRequest {
        task: h.binding.clone(),
        epoch: m::now().max(1) as u64,
        machine: m::this_name(state),
        out_dir: res_dir.join("code"),
        base: base.clone(),
        base_tree,
        last: r.last.clone(),
        haves,
    };
    let git = state.machines.moves.git();
    let folder = PathBuf::from(&r.folder);
    let rd = res_dir.clone();
    let taken = tokio::task::spawn_blocking(
        move || -> Result<Option<(TaskTip, Vec<FileSpec>)>, String> {
            std::fs::create_dir_all(&rd).map_err(|e| e.to_string())?;
            crate::claude::transfer::private_dir(&rd);
            match code::task_result(&git, &folder, &req).map_err(|e| e.to_string())? {
                TaskResult::Unchanged => {
                    let _ = std::fs::remove_dir_all(&rd);
                    Ok(None)
                }
                TaskResult::Made { meta, tip } => {
                    let files = crate::machines::work::source::files_of(&rd, &meta)?;
                    let files: Vec<FileSpec> = files
                        .into_iter()
                        .filter_map(|v| serde_json::from_value(v).ok())
                        .collect();
                    Ok(Some((tip, files)))
                }
            }
        },
    )
    .await
    .map_err(|e| e.to_string())??;
    let Some((tip, files)) = taken else {
        // Nothing new: the newest result, if the lead has not taken it.
        return Ok((r.results > r.landed).then_some(r.results));
    };
    h.update(|r| {
        r.results = seq;
        r.last = Some(tip);
        r.result_files = files;
        r.result_base = Some(base);
        r.result_task = task;
    });
    // Older results' files go: the lead takes the newest.
    if let Ok(entries) = std::fs::read_dir(&results) {
        for e in entries.flatten() {
            let older = e
                .file_name()
                .to_string_lossy()
                .parse::<u32>()
                .is_ok_and(|n| n < seq);
            if older {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    state.machines.work.roles.save(h);
    audit(state, "machine.role.result", &h.record());
    publish(state);
    Ok(Some(seq))
}

/// `GET /peer/teams/roles/{binding}/result`: the newest result's files, tip,
/// the code it was made on and the task it was made for.
pub async fn result_info(state: &AppState, from: &str, binding: &str) -> ApiResult<Value> {
    let h = hosted_of(state, binding, from)?;
    let r = h.record();
    if r.results == 0 || r.last.is_none() {
        return Err(ApiError::not_found(
            "no_result",
            "The role has no result yet.",
        ));
    }
    Ok(json!({
        "seq": r.results,
        "files": r.result_files,
        "last": r.last,
        "base": r.result_base,
        "task": r.result_task,
    }))
}

#[derive(Debug, Deserialize)]
pub struct BytesQuery {
    pub seq: u32,
    #[serde(default)]
    pub offset: Option<u64>,
}

/// `GET /peer/teams/roles/{binding}/result/files/{name}?seq=&offset=`: up to
/// 8 MiB of one of the newest result's files.
pub async fn result_bytes(
    state: &AppState,
    from: &str,
    binding: &str,
    name: &str,
    q: BytesQuery,
) -> ApiResult<Vec<u8>> {
    let h = hosted_of(state, binding, from)?;
    let r = h.record();
    if q.seq != r.results {
        return Err(ApiError::conflict(
            "result_replaced",
            "A newer result replaced that one: take the newest.",
        ));
    }
    let f = r
        .result_files
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| ApiError::not_found("not_found", "No such file in the result."))?;
    let dir = state
        .machines
        .work
        .roles
        .dir_of(binding)
        .map(|d| d.join(RESULTS_DIR).join(q.seq.to_string()))
        .ok_or_else(|| ApiError::internal("This build has no data folder."))?;
    let path = moves::file_path(&dir, name)
        .ok_or_else(|| ApiError::not_found("not_found", "No such file in the result."))?;
    let size = f.size;
    let offset = q.offset.unwrap_or(0).min(size);
    let n = ((size - offset) as usize).min(chunks::CHUNK);
    tokio::task::spawn_blocking(move || chunks::read_chunk(&path, offset, n))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::internal(e.to_string()))
}

// ─── the role's board calls, through the lead ──────────────────────────────────

/// A team tool the role session here called: done on the lead's board, on
/// the lead's computer, with the binding — and for `task_done`, what it did
/// taken first, so the lead can make it the role's branch.
pub async fn relay(
    state: &AppState,
    h: &Arc<Hosted>,
    tool: &str,
    args: Value,
) -> Result<ToolOutput, ToolError> {
    let r = h.record();
    let lead = m::peer_name(state, &r.lead);
    if let Err(e) = m::using(state) {
        return Err(ToolError::new(
            e.code,
            format!("{} The board is on {lead}.", e.message),
        ));
    }
    let mut body = json!({
        "binding": r.binding,
        "secret": r.secret,
        "team": r.team,
        "role": r.role,
        "session": r.session,
        "tool": tool,
        "args": args,
    });
    if tool == "task_done" {
        let task = crate::orchestration::mcp::task_id(&args).ok();
        match collect(state, h, task).await {
            Ok(Some(seq)) => body["result"] = json!(seq),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(binding = %r.binding, error = %e, "a remote role's work could not be taken")
            }
        }
    }
    let acting = m::acting::machine_header(&m::this_name(state), false);
    match moves::call(state, &r.lead, "POST", "/peer/teams/board", Some(&body), &acting, RELAY).await
    {
        Ok((200, v)) => {
            // The results the lead took (they are not sent again).
            let now = m::now();
            let landed = v
                .get("landed")
                .and_then(Value::as_u64)
                .map(|n| u32::try_from(n).unwrap_or(0))
                .filter(|n| h.peek(|r| *n > r.landed && *n <= r.results));
            h.update(|r| {
                r.heard_at = now;
                if let Some(n) = landed {
                    r.landed = n;
                }
            });
            if landed.is_some() {
                state.machines.work.roles.save(h);
            }
            relayed(v)
        }
        Ok((404, v)) if v.pointer("/error/code").is_none() => Err(ToolError::new(
            "update_needed",
            format!("Update Mother Claude on {lead}: its board does not take calls from other computers yet."),
        )),
        Ok((s, v)) => {
            let e = moves::refusal(s, &v, &lead);
            Err(ToolError::new(e.code, e.message))
        }
        Err(e) => Err(ToolError::new(
            "unreachable",
            format!(
                "The team's board is on {lead}, which can't be reached right now ({e}). Try again \
                 in a moment; your work stays here."
            ),
        )),
    }
}

/// The lead's answer to a board call: the tool's JSON, or its refusal.
fn relayed(v: Value) -> Result<ToolOutput, ToolError> {
    match v.get("error") {
        Some(e) => Err(ToolError::new(
            e.get("code")
                .and_then(Value::as_str)
                .unwrap_or("denied")
                .to_string(),
            e.get("message")
                .and_then(Value::as_str)
                .unwrap_or("The lead's board refused it.")
                .to_string(),
        )),
        None => Ok(ToolOutput::json(
            v.get("result").cloned().unwrap_or(Value::Null),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> RoleSpec {
        RoleSpec {
            session_name: "app/windows".into(),
            charter: "# Team role: Windows".into(),
            agents: Some(json!({ "lead": { "prompt": "x" }, "windows": { "prompt": "y" } })),
            paths: vec!["src/win/**".into()],
            read_only: false,
            add_dirs: vec!["tools".into()],
            permission_mode: "acceptEdits".into(),
            model: "sonnet".into(),
            effort: Some("medium".into()),
            fallback_model: None,
            budget_usd: Some(5.0),
            priority: 2,
        }
    }

    #[test]
    fn a_spec_from_another_computer_is_checked_field_by_field() {
        assert!(check_spec(&spec()).is_ok());
        let cases: Vec<RoleSpec> = vec![
            RoleSpec {
                session_name: "--danger".into(),
                ..spec()
            },
            RoleSpec {
                session_name: "app/win dows".into(),
                ..spec()
            },
            RoleSpec {
                paths: vec!["/etc/**".into()],
                ..spec()
            },
            RoleSpec {
                paths: vec!["../x/**".into()],
                ..spec()
            },
            RoleSpec {
                add_dirs: vec!["src/*".into()],
                ..spec()
            },
            RoleSpec {
                permission_mode: "yolo".into(),
                ..spec()
            },
            RoleSpec {
                model: "--model".into(),
                ..spec()
            },
            RoleSpec {
                effort: Some("turbo".into()),
                ..spec()
            },
            RoleSpec {
                fallback_model: Some("sonnet,--x".into()),
                ..spec()
            },
            RoleSpec {
                budget_usd: Some(0.0),
                ..spec()
            },
            RoleSpec {
                priority: 9,
                ..spec()
            },
            RoleSpec {
                agents: Some(json!(["not", "an", "object"])),
                ..spec()
            },
            RoleSpec {
                agents: Some(json!({ "Bad Id": {} })),
                ..spec()
            },
        ];
        for s in cases {
            assert!(check_spec(&s).is_err(), "{s:?}");
        }
    }

    #[test]
    fn the_worktree_is_named_for_the_project_never_with_a_dot_first() {
        assert_eq!(tree_name("", "/Users/me/my app"), "my-app");
        assert_eq!(tree_name(".hidden", "/x"), "hidden");
        assert_eq!(clean_label("a\u{1b}b  ", 10), "ab");
    }
}
