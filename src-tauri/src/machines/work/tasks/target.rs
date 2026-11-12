//! The computer that runs a task (`/peer/work/tasks/*`, docs/PLAN-MACHINES.md
//! Phase 4 decision 6): where the project is here and which account its
//! session would run as, the handed-over code coming in (the moves'
//! chunks), the task's own worktree at it, its owned session, its status
//! (long-polled by the asking computer), its results' files, its cancel and
//! its removal.
//!
//! Every route needs a paired computer holding this computer's `run` grant
//! for the path the request came over, and Machines on here (`403
//! machines_off`). A permission mode without approvals also needs this
//! computer's `dangerous` grant and the asking person's right there
//! (`ctx.may_dangerous()`, `X-MC-Acting`). A task is only ever reached by
//! the computer that asked for it; everything it sends is checked before
//! git or the file system sees it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    audit, clip, first_line, is_task_id, publish, Task, TaskRecord, TaskState, IN_DIR, MAX_TASK,
    RESULTS_DIR,
};
use crate::claude::control::SpawnOptions;
use crate::claude::owned::{self, Caller};
use crate::machines::chunks::{self, AppendError, Refusal};
use crate::machines::code::{self, TaskResult, TaskResultRequest};
use crate::machines::work::target::{check_dangerous, locate, FileSpec};
use crate::machines::work::Direction;
use crate::machines::{self as m, moves};
use crate::server::context::{ClientContext, Principal};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// At most this many files in a task's snapshot.
const MAX_FILES: usize = 10_000;
/// The largest file a snapshot may hold.
const MAX_FILE: u64 = 1 << 40;
/// The incoming files' list, in the task's `.in` folder.
const MANIFEST: &str = "manifest.json";
/// The longest a status request waits for news.
const MAX_WAIT_SECS: u64 = 25;
/// How long a stopped session gets to end.
const STOP_WAIT: Duration = Duration::from_secs(15);

fn bad(m: &str) -> ApiError {
    ApiError::unprocessable("task_bad_request", m.to_string())
}

/// The task `id` `from` asked for (`404 task_unknown` otherwise, as if
/// there were none).
fn task_of(state: &AppState, id: &str, from: &str) -> ApiResult<Arc<Task>> {
    let unknown = || ApiError::not_found("task_unknown", "No such task here.");
    if !is_task_id(id) {
        return Err(unknown());
    }
    state
        .machines
        .work
        .tasks
        .get(id)
        .filter(|t| t.peek(|r| r.direction == Direction::In && r.machine == from))
        .ok_or_else(unknown)
}

fn in_dir(state: &AppState, id: &str) -> ApiResult<PathBuf> {
    state
        .machines
        .work
        .tasks
        .task_dir(id)
        .map(|d| d.join(IN_DIR))
        .ok_or_else(|| ApiError::internal("This build has no data folder."))
}

/// A permission mode a task's session may start in.
fn check_mode(mode: Option<&str>) -> ApiResult<String> {
    let mode = mode.unwrap_or("default");
    if !owned::PERMISSION_MODES.contains(&mode) {
        return Err(bad("The permission mode."));
    }
    Ok(mode.to_string())
}

// ─── who it would run as ───────────────────────────────────────────────────────

/// The asking session's account, as it names it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountAsk {
    pub name: String,
    pub is_default: bool,
}

/// The account a task's session runs under here: the one of the same name
/// (without regard to case), else the Default — when it can run a session.
/// A team role another computer runs here is matched the same way.
pub(crate) fn task_account(state: &AppState, ask: &AccountAsk) -> ApiResult<(String, String)> {
    let all = state.accounts.accounts();
    let wanted = ask.name.trim();
    let found = all
        .iter()
        .find(|a| !wanted.is_empty() && a.name.trim().eq_ignore_ascii_case(wanted))
        .or_else(|| all.iter().find(|a| a.is_default()));
    let Some(a) = found else {
        return Err(ApiError::conflict(
            "account_missing",
            format!(
                "{} has no account to run the task's session.",
                m::this_name(state)
            ),
        ));
    };
    match moves::account_ready(state, &a.id, &a.name) {
        Ok(_) => Ok((a.id.clone(), a.name.clone())),
        Err(check) => {
            let message = match &check.fix {
                Some(fix) => format!("{} {fix}", check.message),
                None => check.message.clone(),
            };
            Err(ApiError::conflict(check.code.clone(), message))
        }
    }
}

// ─── where it would work ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareBody {
    pub project: String,
    #[serde(default)]
    pub account: AccountAsk,
}

/// `POST /peer/work/tasks/prepare`: where the project is here, the account
/// its session would run as, and what the snapshot may leave out (nothing
/// is written).
pub async fn prepare(state: &AppState, from: &str, body: PrepareBody) -> ApiResult<Value> {
    let checkout = locate(state, from, &body.project).await?;
    let (_, account_name) = task_account(state, &body.account)?;
    let git = state.machines.moves.git();
    let c = checkout.clone();
    let haves = tokio::task::spawn_blocking(move || code::haves(&git, &c, None))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::unprocessable("task_failed", e.to_string()))?;
    Ok(json!({
        "checkout": checkout.display().to_string(),
        "os": crate::machines::protocol::this_os(),
        "account": account_name,
        "haves": haves.haves,
        "shallow": haves.shallow,
    }))
}

// ─── a task coming in ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginBody {
    pub task: String,
    pub project: String,
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub cwd: String,
    /// What the session is asked (the task's text).
    pub prompt: String,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub account: AccountAsk,
    /// The branch it will become there (named in the session's header).
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    /// Commits the asking computer has (results leave them out).
    #[serde(default)]
    pub haves: Vec<String>,
    pub files: Vec<FileSpec>,
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn check_begin(b: &BeginBody, dir: &Path) -> ApiResult<()> {
    if !is_task_id(&b.task) {
        return Err(bad("The task's id."));
    }
    if b.prompt.trim().is_empty() || b.prompt.len() > MAX_TASK || b.prompt.contains('\0') {
        return Err(bad("The task."));
    }
    if crate::machines::work::target::clean_cwd(&b.cwd).as_deref() != Some(b.cwd.as_str()) {
        return Err(bad("The session's folder."));
    }
    if let Some(model) = &b.model {
        if owned::validate_model(model).ok().as_deref() != Some(model.as_str()) {
            return Err(bad("The model."));
        }
    }
    if b.effort
        .as_deref()
        .is_some_and(|e| !owned::EFFORT_LEVELS.contains(&e))
    {
        return Err(bad("The effort."));
    }
    if b.account.name.len() > 200 || b.account.name.chars().any(char::is_control) {
        return Err(bad("The account."));
    }
    if b.branch.as_deref().is_some_and(|br| {
        !code::valid_branch(br) || !br.starts_with(code::TASK_BRANCH_PREFIX) || br.len() > 200
    }) {
        return Err(bad("The branch."));
    }
    if b.session
        .as_deref()
        .is_some_and(|s| !crate::claude::ids::is_session_id(s))
        || b.session_title.as_ref().is_some_and(|t| t.len() > 300)
        || b.project_name.len() > 300
    {
        return Err(bad("The session."));
    }
    if b.haves.len() > code::MAX_HAVES || b.haves.iter().any(|h| !code::is_oid(h)) {
        return Err(bad("The commits the asking computer has."));
    }
    if b.files.len() > MAX_FILES {
        return Err(bad("Too many files."));
    }
    let mut seen = std::collections::HashSet::new();
    for f in &b.files {
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
    if !b.files.iter().any(|f| f.name == code::META) {
        return Err(bad("The snapshot has no record."));
    }
    Ok(())
}

fn have_of(dir: &Path, name: &str) -> u64 {
    moves::file_path(dir, name).map_or(0, |p| chunks::have(&p))
}

/// The worktree's folder name: the project's, made safe (never a dot first,
/// so never one of the task's own folders).
fn tree_name(project_name: &str, project: &str) -> String {
    let name = if project_name.trim().is_empty() {
        crate::machines::setup::folders::name_of(project)
    } else {
        project_name.to_string()
    };
    code::safe_name(&name)
}

/// `POST /peer/work/tasks`: a task is coming, with the snapshot's files. A
/// second call for the same task (a transfer broken off) answers what
/// arrived of each.
pub async fn begin(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    b: BeginBody,
) -> ApiResult<Value> {
    let dir = in_dir(state, &b.task)?;
    check_begin(&b, &dir)?;
    let mode = check_mode(b.permission_mode.as_deref())?;
    let dangerous = owned::is_dangerous_mode(Some(&mode));
    check_dangerous(state, ctx, dangerous)?;
    let checkout = locate(state, from, &b.project).await?;
    let (account, account_name) = task_account(state, &b.account)?;
    let via_lan = matches!(ctx.principal, Principal::Peer { lan: true, .. });
    if let Some(t) = state.machines.work.tasks.get(&b.task) {
        let (mine, sending) = t.peek(|r| {
            (
                r.direction == Direction::In && r.machine == from,
                r.state == TaskState::Sending,
            )
        });
        if !mine || !sending {
            return Err(ApiError::conflict(
                "task_exists",
                "That task is known here already.",
            ));
        }
    } else {
        let task_dir = state
            .machines
            .work
            .tasks
            .task_dir(&b.task)
            .ok_or_else(|| ApiError::internal("This build has no data folder."))?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(e.to_string()))?;
        crate::claude::transfer::private_dir(&task_dir);
        crate::claude::transfer::private_dir(&dir);
        crate::persist::write_json_private(&dir.join(MANIFEST), &b.files)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let folder = task_dir.join(tree_name(&b.project_name, &b.project));
        let first = first_line(&b.prompt);
        let now = m::now();
        let record = TaskRecord {
            id: b.task.clone(),
            direction: Direction::In,
            machine: from.to_string(),
            machine_name: m::peer_name(state, from),
            os: crate::machines::protocol::this_os().to_string(),
            session: b.session.clone(),
            session_title: b.session_title.as_deref().map(|t| clip(t, 300)),
            project: b.project.clone(),
            project_name: if b.project_name.is_empty() {
                crate::machines::setup::folders::name_of(&b.project)
            } else {
                clip(&b.project_name, 200)
            },
            cwd: b.cwd.clone(),
            checkout: checkout.display().to_string(),
            folder: folder.display().to_string(),
            task: b.prompt.clone(),
            first_line_sha256: crate::claude::secrets::sha256_hex(first.as_bytes()),
            first_line: first,
            permission_mode: mode.clone(),
            model: b.model.clone(),
            effort: b.effort.clone(),
            account_name: Some(account_name),
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
            branch: b.branch.clone(),
            tip: None,
            commits: 0,
            files: 0,
            uncommitted: false,
            results: 0,
            landed: 0,
            branch_note: None,
            approved_by: None,
            dangerous,
            unreachable: false,
            gone_there: false,
            via_lan: Some(via_lan),
            idle_stopped_at: None,
            updated_at: now,
            account: Some(account),
            haves: b.haves.clone(),
            last: None,
            result_files: Vec::new(),
            their_state: None,
        };
        super::prune(state);
        let t =
            state.machines.work.tasks.insert(record).ok_or_else(|| {
                ApiError::conflict("task_exists", "That task is known here already.")
            })?;
        state.machines.work.tasks.save(&t);
        audit(state, "machine.task.asked", &t.record());
        publish(state);
    }
    let files: Vec<Value> = b
        .files
        .iter()
        .map(|f| json!({ "name": f.name, "have": have_of(&dir, &f.name).min(f.size) }))
        .collect();
    Ok(json!({ "files": files }))
}

/// `PUT /peer/work/tasks/{task}/files/{name}?offset=N`: the next chunk.
pub async fn put_chunk(
    state: &AppState,
    from: &str,
    id: &str,
    name: &str,
    offset: u64,
    bytes: axum::body::Bytes,
) -> ApiResult<Value> {
    let t = task_of(state, id, from)?;
    if t.state() != TaskState::Sending {
        return Err(ApiError::conflict(
            "task_started",
            "That task has its code already.",
        ));
    }
    let dir = in_dir(state, id)?;
    let manifest: Option<Vec<FileSpec>> = crate::persist::read_json(&dir.join(MANIFEST));
    let f = manifest
        .and_then(|l| l.into_iter().find(|f| f.name == name))
        .ok_or_else(|| ApiError::not_found("not_found", "No such file in this task."))?;
    let path = moves::file_path(&dir, name)
        .ok_or_else(|| ApiError::unprocessable("task_bad_request", "No such file."))?;
    let _one = state.machines.work.tasks.chunks.lock().await;
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
                    "task_bad_request",
                    "The file is longer than the task said.",
                ),
                AppendError::Refused(Refusal::TooLarge) => {
                    ApiError::unprocessable("task_bad_request", "A chunk is at most 8 MiB.")
                }
                AppendError::Io(e) => {
                    ApiError::internal(format!("The file could not be written here: {e}"))
                }
            })?;
    Ok(json!({ "name": name, "size": written }))
}

/// Ends a task that never got to work (its code did not land, its
/// session did not start).
fn fail_early(state: &AppState, t: &Task, why: String) {
    let now = m::now();
    t.update(|r| {
        r.state = TaskState::Error;
        r.error = Some(why);
        r.ended_at = Some(now);
    });
    state.machines.work.tasks.save(t);
    audit(state, "machine.task", &t.record());
    publish(state);
}

/// `POST /peer/work/tasks/{task}/start`: every file whole, the snapshot
/// fetched into this computer's checkout, the task's worktree made at it,
/// then its session started. Answers its status.
pub async fn start(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    id: &str,
) -> ApiResult<Value> {
    let t = task_of(state, id, from)?;
    let r = t.record();
    if r.state != TaskState::Sending {
        return Ok(status_view(state, &t));
    }
    check_dangerous(state, ctx, r.dangerous)?;
    let dir = in_dir(state, id)?;
    let files: Vec<FileSpec> = crate::persist::read_json(&dir.join(MANIFEST)).ok_or_else(|| {
        ApiError::conflict(
            "task_incomplete",
            "The task's code did not come here: send it again.",
        )
    })?;
    let d = dir.clone();
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
            "task_incomplete",
            format!("The task's code did not arrive whole: {why}."),
        )
    })?;
    t.update(|r| r.state = TaskState::Starting);
    publish(state);
    // The code, into this computer's checkout (its objects and refs only),
    // then the task's own worktree at it.
    let git = state.machines.moves.git();
    let (g, checkout, code_dir, folder, task) = (
        git.clone(),
        PathBuf::from(&r.checkout),
        dir.join("code"),
        PathBuf::from(&r.folder),
        r.id.clone(),
    );
    let made = tokio::task::spawn_blocking(move || {
        let f = code::fetch_snapshot(&g, &checkout, &code_dir, &task)?;
        code::reset_work_tree(&g, &checkout, &folder, &f.meta.snap)?;
        Ok::<_, code::CodeError>(f.meta)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let meta = match made {
        Ok(meta) => meta,
        Err(e) => {
            let why = format!("The task's code could not be set up here: {e}");
            fail_early(state, &t, why.clone());
            return Err(ApiError::unprocessable("task_failed", why));
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    t.update(|r| {
        r.base = Some(meta.snap.clone());
        r.base_tree = Some(meta.wt_tree.clone());
    });
    state.machines.work.tasks.save(&t);
    if t.state().is_final() {
        // Cancelled (or stopped from here) while its code was set up.
        return Ok(status_view(state, &t));
    }
    let may_dangerous = r.dangerous && ctx.may_dangerous();
    if let Err(why) = start_session(state, &t, may_dangerous).await {
        let why = format!("Its Claude Code session could not start here: {why}");
        fail_early(state, &t, why.clone());
        return Err(ApiError::unprocessable("task_failed", why));
    }
    state.machines.work.tasks.save(&t);
    audit(state, "machine.task.start", &t.record());
    publish(state);
    Ok(status_view(state, &t))
}

/// What the task's session is asked: the task, after a short header saying
/// where it came from and how to finish.
pub fn prompt_of(source: &str, r: &TaskRecord) -> String {
    let branch = r
        .branch
        .as_deref()
        .map(|b| format!(" `{}`", code::short_branch(b)))
        .unwrap_or_default();
    format!(
        "[Mother Claude handed you this task from {source}, where a Claude Code session works \
         on {project}. You work in a git worktree of this computer's copy of {project}, set to \
         that session's code (its uncommitted changes are this worktree's first commit). \
         Finish by committing your work or by leaving your changes in the working tree: both \
         go back to {source} as the branch{branch}, with your last message as the summary, so \
         end with a short summary of what you did. There is no need to push.]\n\n{task}",
        project = r.project_name,
        task = r.task,
    )
}

/// The session's name on both computers' lists.
fn title_of(source: &str, r: &TaskRecord) -> String {
    clip(&format!("From {source}: {}", r.first_line), 80)
}

/// Where the task's session works: the asking session's folder in the
/// project (when the code has it), else the worktree's top.
fn session_cwd(r: &TaskRecord) -> String {
    let top = PathBuf::from(&r.folder);
    let inside = r.cwd.split('/').fold(top.clone(), |p, s| p.join(s));
    if !r.cwd.is_empty() && inside.is_dir() {
        inside
    } else {
        top
    }
    .display()
    .to_string()
}

/// The options of the task's session, with everything but what starts it
/// (`session_id`, `resume`); why not when the account or the mode refuse.
fn session_options(
    state: &AppState,
    r: &TaskRecord,
    may_dangerous: bool,
    prompt: String,
) -> Result<SpawnOptions, String> {
    let account = r
        .account
        .clone()
        .unwrap_or_else(|| crate::claude::accounts::DEFAULT_ID.to_string());
    let caller = Caller {
        may_dangerous,
        // The folder is the task's own worktree, in our data folder.
        desktop: true,
    };
    let mode = owned::launch_mode(Some(&r.permission_mode), caller).map_err(|e| e.message)?;
    let account_env = state
        .accounts
        .env(&account)
        .map_err(|e| owned::account_error(e).message)?;
    Ok(SpawnOptions {
        cwd: session_cwd(r),
        prompt,
        model: r.model.clone(),
        effort: r.effort.clone(),
        permission_mode: Some(mode),
        allow_bypass: may_dangerous,
        caller_may_dangerous: may_dangerous,
        account: Some(account),
        account_env,
        quiet: true,
        ..Default::default()
    })
}

/// Starts the task's owned session in its worktree (records the spawn
/// until the app turns task sessions on). Running from here: a turn that
/// ends at once is the task's.
async fn start_session(
    state: &AppState,
    t: &Arc<Task>,
    may_dangerous: bool,
) -> Result<String, String> {
    let r = t.record();
    let session = uuid::Uuid::new_v4().to_string();
    let source = m::peer_name(state, &r.machine);
    let prompt = prompt_of(&source, &r);
    let now = m::now();
    t.update(|r| {
        r.task_session = Some(session.clone());
        r.state = TaskState::Running;
        r.started_at = Some(now);
    });
    let mut opts = session_options(state, &r, may_dangerous, prompt)?;
    opts.session_id = Some(session.clone());
    if !state.machines.work.tasks.sessions_live() {
        // Checked as it would be, then only recorded.
        crate::machines::lease::check_launch(state, &session).map_err(|e| e.to_string())?;
        state.machines.work.tasks.record_spawn(json!({
            "task": r.id,
            "session": session,
            "cwd": opts.cwd,
            "folder": r.folder,
            "prompt": opts.prompt,
            "title": title_of(&source, &r),
            "permissionMode": opts.permission_mode,
            "model": r.model,
            "effort": r.effort,
            "account": opts.account,
            "accountName": r.account_name,
            "allowBypass": may_dangerous,
        }));
        return Ok(session);
    }
    state
        .control
        .spawn(state, opts)
        .await
        .map_err(|e| owned::spawn_error(e).message)?;
    // A name it is known by on both computers' lists.
    let (st, id, title) = (state.clone(), session.clone(), title_of(&source, &r));
    tokio::spawn(async move {
        if let Err(e) = owned::rename(&st, &id, &title).await {
            tracing::debug!(session = %id, error = %e.message, "a task session's name");
        }
    });
    Ok(session)
}

/// A message to a task session whose process is gone — stopped while idle
/// ([`super::stop_idle`]), or ended with the app — resumes it in its
/// worktree with the message (`--resume <id>`, in place; a fork under a new
/// id when another process holds its transcript), so a turn it takes then
/// moves the branch forward as before. `None`: the session is not one of a
/// task that ended here (the usual message routes take it). A session that
/// runs without approvals needs the sender's dangerous right.
pub async fn resume_with(
    state: &AppState,
    may_dangerous: bool,
    session: &str,
    text: &str,
    images: &[crate::claude::model::ImageInput],
) -> ApiResult<Option<crate::claude::model::MessageReceipt>> {
    if state.control.controls(session) {
        return Ok(None);
    }
    let Some(t) = state.machines.work.tasks.of_session(session) else {
        return Ok(None);
    };
    let r = t.record();
    if !matches!(r.state, TaskState::Done | TaskState::Stopped) {
        return Ok(None);
    }
    if r.dangerous && !may_dangerous {
        return Err(ApiError::dangerous_blocked(
            "This task's session runs without approvals: resuming it needs the dangerous right.",
        ));
    }
    if text.len() > owned::MAX_TEXT_BYTES {
        return Err(ApiError::unprocessable(
            "message_too_long",
            "The message is too long.",
        ));
    }
    let images = owned::validate_images(images)?;
    let mut opts = session_options(state, &r, r.dangerous && may_dangerous, text.to_string())
        .map_err(|why| ApiError::conflict("task_not_resumed", why))?;
    opts.images = images;
    let fork = owned::live_elsewhere(state, session).await;
    let id = if fork {
        uuid::Uuid::new_v4().to_string()
    } else {
        session.to_string()
    };
    opts.resume = Some(session.to_string());
    opts.fork = fork;
    opts.session_id = fork.then(|| id.clone());
    if fork {
        t.update(|r| r.task_session = Some(id.clone()));
    }
    if !state.machines.work.tasks.sessions_live() {
        crate::machines::lease::check_launch(state, &id)
            .map_err(|e| ApiError::conflict("task_not_resumed", e.to_string()))?;
        state.machines.work.tasks.record_spawn(json!({
            "task": r.id,
            "session": id,
            "resume": session,
            "fork": fork,
            "cwd": opts.cwd,
            "folder": r.folder,
            "prompt": opts.prompt,
            "permissionMode": opts.permission_mode,
            "model": r.model,
            "effort": r.effort,
            "account": opts.account,
            "accountName": r.account_name,
            "allowBypass": opts.allow_bypass,
        }));
        // What the session's first turn would report.
        super::turn_started(state, &id);
    } else {
        let spawned = state
            .control
            .spawn(state, opts)
            .await
            .map_err(owned::spawn_error)?;
        // The stream's first turn says so too; said now, the task runs at once.
        super::turn_started(state, &spawned);
    }
    audit(state, "machine.task.resume", &t.record());
    tracing::info!(task = %r.id, "a message resumed a task's session");
    Ok(Some(crate::claude::model::MessageReceipt {
        delivery: crate::claude::model::Delivery::Sent,
        message_id: uuid::Uuid::new_v4().to_string(),
    }))
}

// ─── its result ────────────────────────────────────────────────────────────────

/// Takes the task's work in the background ([`collect`]).
pub(crate) fn collect_soon(state: &AppState, t: Arc<Task>, then: TaskState) {
    let st = state.clone();
    tokio::spawn(async move { collect(&st, t, then).await });
}

/// What the task's session did since its last result, taken (a snapshot of
/// its worktree, its bundle ready for the asking computer), then the task
/// is where it was going (`then`) — unless a new turn started meanwhile, or
/// a cancel already ended it.
pub(crate) async fn collect(state: &AppState, t: Arc<Task>, then: TaskState) {
    let _one = t.work.lock().await;
    let turned_before = t.turns_started();
    let r = t.record();
    if r.state.is_final() {
        return;
    }
    let mut error: Option<String> = None;
    if let (Some(base), Some(base_tree), Some(dir)) = (
        r.base.clone(),
        r.base_tree.clone(),
        state.machines.work.tasks.task_dir(&r.id),
    ) {
        let seq = r.results + 1;
        let results = dir.join(RESULTS_DIR);
        let res_dir = results.join(seq.to_string());
        let _ = std::fs::remove_dir_all(&res_dir);
        // The asking computer has the last result once it took it.
        let mut haves = r.haves.clone();
        if r.landed >= r.results {
            if let Some(l) = &r.last {
                haves.insert(0, l.snap.clone());
                haves.insert(0, l.tip.clone());
            }
        }
        let req = TaskResultRequest {
            task: r.id.clone(),
            epoch: m::now().max(1) as u64,
            machine: m::this_name(state),
            out_dir: res_dir.join("code"),
            base,
            base_tree,
            last: r.last.clone(),
            haves,
        };
        let git = state.machines.moves.git();
        let folder = PathBuf::from(&r.folder);
        let rd = res_dir.clone();
        let taken = tokio::task::spawn_blocking(
            move || -> Result<Option<(code::TaskTip, Vec<FileSpec>)>, String> {
                std::fs::create_dir_all(&rd).map_err(|e| e.to_string())?;
                crate::claude::transfer::private_dir(&rd);
                let res = code::task_result(&git, &folder, &req).map_err(|e| e.to_string())?;
                match res {
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
        .await;
        match taken {
            Ok(Ok(Some((tip, files)))) => {
                t.update(|r| {
                    r.results = seq;
                    r.tip = Some(tip.tip.clone());
                    r.last = Some(tip);
                    r.result_files = files;
                });
                // Older results' files go: the asking computer takes the newest.
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
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => error = Some(format!("What its session did could not be taken: {e}")),
            Err(e) => error = Some(e.to_string()),
        }
    }
    let turned = t.turns_started() != turned_before;
    let next = super::after_result(then, turned);
    let now = m::now();
    t.update(|r| {
        if r.state.is_final() {
            return;
        }
        r.state = next;
        if next.ended() {
            r.ended_at = Some(now);
        }
        if let Some(e) = error {
            r.error = Some(clip(&e, 2000));
        }
    });
    state.machines.work.tasks.save(&t);
    audit(state, "machine.task", &t.record());
    publish(state);
}

// ─── its status, results, cancel, removal ──────────────────────────────────────

/// A task's status as the asking computer follows it.
pub fn status_view(state: &AppState, t: &Task) -> Value {
    let r = t.record();
    let shown = r.state.shown(super::asking(state, &r));
    json!({
        "task": {
            "id": r.id,
            "state": shown,
            "error": r.error,
            "turns": r.turns,
            "summary": r.summary,
            "background": r.background,
            "results": r.results,
            "landed": r.landed,
            "last": r.last,
            "startedAt": r.started_at,
            "endedAt": r.ended_at,
            "taskSession": r.task_session,
            "folder": r.folder,
            "checkout": r.checkout,
            "accountName": r.account_name,
            "os": r.os,
            "idleStoppedAt": r.idle_stopped_at,
        }
    })
}

#[derive(Debug, Default, Deserialize)]
pub struct StatusQuery {
    /// Seconds to wait for news (at most 25).
    #[serde(default)]
    pub wait: Option<u64>,
    /// The state the asking computer knows.
    #[serde(default)]
    pub state: Option<String>,
    /// The results it knows of.
    #[serde(default)]
    pub results: Option<u32>,
    /// The results it took.
    #[serde(default)]
    pub landed: Option<u32>,
    /// Whether it knows the task's session stopped while idle.
    #[serde(default)]
    pub idle: Option<bool>,
}

/// `GET /peer/work/tasks/{task}?wait=&state=&results=&landed=`: its status,
/// once it differs from what the asking computer knows (or `wait` went
/// by). `landed` tells this computer which results it took.
pub async fn status(state: &AppState, from: &str, id: &str, q: StatusQuery) -> ApiResult<Value> {
    let t = task_of(state, id, from)?;
    if let Some(n) = q.landed {
        let changed = t.peek(|r| n > r.landed && n <= r.results);
        if changed {
            t.update(|r| r.landed = n);
            state.machines.work.tasks.save(&t);
        }
    }
    let wait = Duration::from_secs(q.wait.unwrap_or(0).min(MAX_WAIT_SECS));
    let deadline = tokio::time::Instant::now() + wait;
    let mut rx = t.watch();
    loop {
        let r = t.record();
        let shown = r.state.shown(super::asking(state, &r));
        if q.state.as_deref() != Some(shown.as_str())
            || q.results != Some(r.results)
            || q.idle
                .is_some_and(|idle| idle != r.idle_stopped_at.is_some())
        {
            break;
        }
        // A second at a time: an approval coming or going changes it too.
        let until = deadline.min(tokio::time::Instant::now() + Duration::from_secs(1));
        let _ = tokio::time::timeout_at(until, rx.changed()).await;
        if tokio::time::Instant::now() >= deadline {
            break;
        }
    }
    Ok(status_view(state, &t))
}

/// `GET /peer/work/tasks/{task}/result`: the newest result's files and tip.
pub async fn result_info(state: &AppState, from: &str, id: &str) -> ApiResult<Value> {
    let t = task_of(state, id, from)?;
    let r = t.record();
    if r.results == 0 || r.last.is_none() {
        return Err(ApiError::not_found(
            "no_result",
            "The task has no result yet.",
        ));
    }
    Ok(json!({ "seq": r.results, "files": r.result_files, "last": r.last }))
}

#[derive(Debug, Deserialize)]
pub struct BytesQuery {
    pub seq: u32,
    #[serde(default)]
    pub offset: Option<u64>,
}

/// `GET /peer/work/tasks/{task}/result/files/{name}?seq=&offset=`: up to
/// 8 MiB of one of the newest result's files.
pub async fn result_bytes(
    state: &AppState,
    from: &str,
    id: &str,
    name: &str,
    q: BytesQuery,
) -> ApiResult<Vec<u8>> {
    let t = task_of(state, id, from)?;
    let r = t.record();
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
        .tasks
        .task_dir(id)
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

/// Cancels a task here: its session stopped (what it did is taken first,
/// so it comes back), then nothing follows it any more.
pub(crate) async fn cancel_here(state: &AppState, t: &Arc<Task>, why: Option<String>) {
    let r = t.record();
    if r.state.is_final() {
        return;
    }
    if r.state == TaskState::Sending || r.base.is_none() {
        let now = m::now();
        t.update(|r| {
            r.state = TaskState::Cancelled;
            r.ended_at = Some(now);
            if why.is_some() {
                r.error = why.clone();
            }
        });
        state.machines.work.tasks.save(t);
        audit(state, "machine.task", &t.record());
        publish(state);
        return;
    }
    // Finishing: its session's end now is no stop of its own.
    t.update(|r| {
        r.state = TaskState::Finishing;
        if why.is_some() {
            r.error = why.clone();
        }
    });
    publish(state);
    if let Some(s) = r.task_session.as_deref() {
        let _ = state.control.stop_and_wait(s, STOP_WAIT).await;
    }
    collect(state, t.clone(), TaskState::Cancelled).await;
}

/// `POST /peer/work/tasks/{task}/cancel`: stops it. Answers its status.
pub async fn cancel(state: &AppState, from: &str, id: &str) -> ApiResult<Value> {
    let t = task_of(state, id, from)?;
    cancel_here(state, &t, None).await;
    Ok(status_view(state, &t))
}

/// `DELETE /peer/work/tasks/{task}`: takes it away here (its session, its
/// worktree, its refs, its record). Refused while it runs.
pub async fn remove(state: &AppState, from: &str, id: &str) -> ApiResult<Value> {
    let t = task_of(state, id, from)?;
    remove_here(state, &t).await?;
    Ok(json!({ "removed": true }))
}

/// Removes a task this computer ran (the asking computer's ask, or this
/// computer's dashboard).
pub(crate) async fn remove_here(state: &AppState, t: &Arc<Task>) -> ApiResult<()> {
    let r = t.record();
    if !r.state.ended() {
        return Err(ApiError::conflict(
            "task_running",
            "The task is still running: cancel it first.",
        ));
    }
    audit(state, "machine.task.remove", &r);
    state.machines.work.tasks.forget(&t.id);
    super::clear(state, t).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_is_told_where_the_task_came_from_and_how_to_finish() {
        let r = TaskRecord {
            id: "task_0123456789abcdef".into(),
            direction: Direction::In,
            machine: "m".into(),
            machine_name: "MacBook".into(),
            os: "windows".into(),
            session: None,
            session_title: None,
            project: "/Users/me/app".into(),
            project_name: "app".into(),
            cwd: String::new(),
            checkout: String::new(),
            folder: String::new(),
            task: "Fix the Windows build.".into(),
            first_line: "Fix the Windows build.".into(),
            first_line_sha256: String::new(),
            permission_mode: "default".into(),
            model: None,
            effort: None,
            account_name: None,
            task_session: None,
            state: TaskState::Sending,
            error: None,
            created_at: 0,
            started_at: None,
            ended_at: None,
            turns: 0,
            summary: None,
            background: false,
            base: None,
            base_tree: None,
            branch: Some("refs/heads/mother-claude/DESKTOP-PC/task_0123456789abcdef".into()),
            tip: None,
            commits: 0,
            files: 0,
            uncommitted: false,
            results: 0,
            landed: 0,
            branch_note: None,
            approved_by: None,
            dangerous: false,
            unreachable: false,
            gone_there: false,
            via_lan: None,
            idle_stopped_at: None,
            updated_at: 0,
            account: None,
            haves: Vec::new(),
            last: None,
            result_files: Vec::new(),
            their_state: None,
        };
        let p = prompt_of("MacBook", &r);
        assert!(
            p.starts_with("[Mother Claude handed you this task from MacBook"),
            "{p}"
        );
        assert!(p.contains("committing your work"), "{p}");
        assert!(
            p.contains("`mother-claude/DESKTOP-PC/task_0123456789abcdef`"),
            "{p}"
        );
        assert!(p.ends_with("]\n\nFix the Windows build."), "{p}");
        assert_eq!(
            title_of("MacBook", &r),
            "From MacBook: Fix the Windows build."
        );
        assert_eq!(tree_name("", "/Users/me/my app"), "my-app");
        assert_eq!(tree_name(".hidden", "/x"), "hidden", "never a dot first");
        assert!(!tree_name("..", "/x").starts_with('.'));
    }
}
