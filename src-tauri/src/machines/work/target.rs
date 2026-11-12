//! The computer that runs (`/peer/work/*`, docs/PLAN-MACHINES.md Phase 4):
//! where the project is here, the snapshot coming in (the moves' chunks),
//! the run — waiting for its work folder, setting it to the snapshot,
//! running the command in this computer's shell — its status (long-polled
//! by the asking computer), its output's pages, its cancel, the files a
//! glob named, and the setup's echo.
//!
//! Every route needs a paired computer holding this computer's `run` grant
//! for the path the request came over, and Machines on here (`403
//! machines_off`) — but the echo, which the setup's readiness test uses
//! before Machines is on. A command the danger classifier flags, here or
//! there, also needs this computer's `dangerous` grant and the asking
//! person's right there (`ctx.may_dangerous()`, `X-MC-Acting`). A run is
//! only ever reached by the computer that asked for it; everything it sends
//! is checked before git or the file system sees it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::output::{self, Output};
use super::shell::{self, Ending, Shell};
use super::{
    audit, is_run_id, publish, tail_of, work_id, Artifact, Direction, Run, RunRecord, RunState,
    LIVE_TAIL, MAX_ARTIFACT, MAX_ARTIFACTS, MAX_COMMAND, MAX_TIMEOUT_SECS, TAIL,
};
use crate::claude::paths;
use crate::machines::chunks::{self, AppendError, Refusal};
use crate::machines::grants::Grant;
use crate::machines::{self as m, code, moves};
use crate::server::context::{ClientContext, Principal};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// At most this many files in a run's snapshot.
const MAX_FILES: usize = 10_000;
/// The largest file a snapshot may hold.
const MAX_FILE: u64 = 1 << 40;
/// The incoming files' list, in the run's `in/` folder.
const MANIFEST: &str = "manifest.json";
/// The longest a status request waits for news.
const MAX_WAIT_SECS: u64 = 25;
/// The longest glob.
const MAX_GLOB: usize = 512;
/// Entries a glob's walk looks at, at most.
const MAX_WALK: usize = 200_000;

// ─── who asks ──────────────────────────────────────────────────────────────────

fn peer_of(ctx: &ClientContext) -> ApiResult<(String, bool)> {
    match &ctx.principal {
        Principal::Peer { machine, lan, .. } => Ok((machine.clone(), *lan)),
        _ => Err(ApiError::forbidden(
            "peer_only",
            "Only a paired computer can call this.",
        )),
    }
}

fn grant_missing(state: &AppState, grant: Grant) -> ApiError {
    ApiError::forbidden(
        "grant_missing",
        format!(
            "{} has not given yours the \"{}\" permission.",
            m::this_name(state),
            grant.as_str()
        ),
    )
    .with("grant", grant.as_str())
}

/// A paired computer holding this one's `run` grant for the request's
/// path, while Machines is on here.
pub fn running_peer(state: &AppState, ctx: &ClientContext) -> ApiResult<String> {
    let (machine, lan) = peer_of(ctx)?;
    m::serving(state)?;
    if !state.machines.allows(&machine, Grant::Run, lan) {
        return Err(grant_missing(state, Grant::Run));
    }
    Ok(machine)
}

/// The run `id` `from` asked for (`404 run_unknown` otherwise, as if there
/// were none).
fn run_of(state: &AppState, id: &str, from: &str) -> ApiResult<Arc<Run>> {
    let unknown = || ApiError::not_found("run_unknown", "No such run here.");
    if !is_run_id(id) {
        return Err(unknown());
    }
    state
        .machines
        .work
        .get(id)
        .filter(|r| r.peek(|r| r.direction == Direction::In && r.machine == from))
        .ok_or_else(unknown)
}

/// A dangerous command needs this computer's `dangerous` grant for the
/// asking one and the asking person's right there (plan §5.5).
fn check_dangerous(state: &AppState, ctx: &ClientContext, dangerous: bool) -> ApiResult<()> {
    if !dangerous {
        return Ok(());
    }
    if !ctx.may_dangerous() {
        return Err(ApiError::forbidden(
            "dangerous_needed",
            format!(
                "This command is dangerous: running it on {} needs its Dangerous permission for \
                 your computer, and an approval with the dangerous right there.",
                m::this_name(state)
            ),
        )
        .with("grant", "dangerous"));
    }
    ctx.note_dangerous("run a dangerous command another computer asked for");
    Ok(())
}

/// Whether this computer's classifier flags `command` (run in `folder`).
fn flagged_here(command: &str, folder: &Path) -> bool {
    let env = crate::pending::DangerEnv::current(Some(&folder.to_string_lossy()));
    crate::pending::sensitive_command(command, &env).is_some()
}

// ─── where the project is here ─────────────────────────────────────────────────

fn no_checkout(state: &AppState, project: &str, why: String) -> ApiError {
    let name = crate::machines::setup::folders::name_of(project);
    ApiError::not_found(
        "work_no_checkout",
        format!(
            "{name} isn't on {}: {why} Clone it there first (Machines › Set up › Accounts and \
             folders: Clone it there), then run it again.",
            m::this_name(state)
        ),
    )
    .with("project", name)
}

/// This computer's checkout of the project at `project` on `from`: the
/// folder a root (or a folder pair) maps it to — else the same path — when
/// it is a git repository here.
pub async fn locate(state: &AppState, from: &str, project: &str) -> ApiResult<PathBuf> {
    if project.is_empty() || project.len() > 4096 || !paths::is_absolute_anywhere(project) {
        return Err(ApiError::unprocessable(
            "run_bad_request",
            "The project's folder is not a full path.",
        ));
    }
    let found = moves::folder::folder_there(state, from, project, None, None)
        .map_err(|e| ApiError::unprocessable("run_bad_request", e.0))?;
    let Some(folder) = found.path else {
        let why = match crate::machines::roots::map_here(state, from, project) {
            Some(p) => format!("{p} is not a folder here."),
            None => format!("no folder here maps {project}."),
        };
        return Err(no_checkout(state, project, why));
    };
    let git = state.machines.moves.git();
    let f = PathBuf::from(&folder);
    let top = tokio::task::spawn_blocking(move || code::top_of(&git, &f))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    match top {
        Ok(t) => Ok(t),
        Err(code::CodeError::NotARepository(_)) => Err(no_checkout(
            state,
            project,
            format!("{folder} is not a git repository."),
        )),
        Err(e) => Err(ApiError::unprocessable("run_failed", e.to_string())),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareBody {
    pub project: String,
}

/// `POST /peer/work/prepare`: where the project is here, its work folder,
/// this computer's shell and what the snapshot may leave out (nothing is
/// written).
pub async fn prepare(state: &AppState, from: &str, body: PrepareBody) -> ApiResult<Value> {
    let checkout = locate(state, from, &body.project).await?;
    let work = work_id(from, &body.project);
    let folder = state
        .machines
        .work
        .folder(&work)
        .ok_or_else(|| ApiError::internal("This build has no data folder."))?;
    let git = state.machines.moves.git();
    let (c, w) = (checkout.clone(), work.clone());
    let haves = tokio::task::spawn_blocking(move || code::haves(&git, &c, Some(&w)))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::unprocessable("run_failed", e.to_string()))?;
    let shell = Shell::here();
    Ok(json!({
        "work": work,
        "checkout": checkout.display().to_string(),
        "folder": folder.display().to_string(),
        "os": crate::machines::protocol::this_os(),
        "shell": shell.kind.as_str(),
        "shellLabel": shell.kind.label(),
        "haves": haves.haves,
        "shallow": haves.shallow,
    }))
}

// ─── a run coming in ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpec {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginBody {
    pub run: String,
    pub work: String,
    pub project: String,
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub cwd: String,
    pub command: String,
    pub timeout_secs: u64,
    #[serde(default)]
    pub artifacts: Option<String>,
    #[serde(default)]
    pub dangerous: bool,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    pub files: Vec<FileSpec>,
}

/// A folder inside the project, as a run names it: relative, `/` or `\`
/// between its parts, never `..` or `.git`. `""` is the top.
pub fn clean_cwd(raw: &str) -> Option<String> {
    let s = raw.trim().replace('\\', "/");
    let s = s.trim_matches('/');
    let s = s.strip_prefix("./").unwrap_or(s);
    if s.is_empty() || s == "." {
        return Some(String::new());
    }
    let parts: Vec<&str> = s
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let joined = parts.join("/");
    (!parts.is_empty() && code::safe_rel_path(&joined) && !paths::is_absolute_anywhere(raw.trim()))
        .then_some(joined)
}

/// A glob naming files inside the project.
pub fn clean_glob(raw: &str) -> Option<String> {
    let s = raw.trim().replace('\\', "/");
    if s.is_empty()
        || s.len() > MAX_GLOB
        || s.contains('\0')
        || paths::is_absolute_anywhere(raw.trim())
        || s.split('/')
            .any(|p| p == ".." || p.eq_ignore_ascii_case(".git"))
    {
        return None;
    }
    Some(s.trim_start_matches("./").to_string())
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn bad(m: &str) -> ApiError {
    ApiError::unprocessable("run_bad_request", m.to_string())
}

fn check_begin(b: &BeginBody, in_dir: &Path) -> ApiResult<()> {
    if !is_run_id(&b.run) {
        return Err(bad("The run's id."));
    }
    let command = b.command.trim();
    if command.is_empty() || b.command.len() > MAX_COMMAND || b.command.contains('\0') {
        return Err(bad("The command."));
    }
    if b.timeout_secs == 0 || b.timeout_secs > MAX_TIMEOUT_SECS {
        return Err(bad("The timeout."));
    }
    if clean_cwd(&b.cwd).as_deref() != Some(b.cwd.as_str()) {
        return Err(bad("The folder to run in."));
    }
    if let Some(g) = &b.artifacts {
        if clean_glob(g).as_deref() != Some(g.as_str()) {
            return Err(bad("The artifacts' glob."));
        }
    }
    for s in [&b.session, &b.session_title] {
        if s.as_ref().is_some_and(|s| s.len() > 300) {
            return Err(bad("The session."));
        }
    }
    if b.files.len() > MAX_FILES {
        return Err(bad("Too many files."));
    }
    let mut seen = std::collections::HashSet::new();
    for f in &b.files {
        if moves::file_path(in_dir, &f.name).is_none()
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

fn in_dir(state: &AppState, id: &str) -> ApiResult<PathBuf> {
    state
        .machines
        .work
        .run_dir(id)
        .map(|d| d.join("in"))
        .ok_or_else(|| ApiError::internal("This build has no data folder."))
}

fn manifest(dir: &Path) -> Option<Vec<FileSpec>> {
    crate::persist::read_json(&dir.join(MANIFEST))
}

fn have_of(dir: &Path, name: &str) -> u64 {
    moves::file_path(dir, name).map_or(0, |p| chunks::have(&p))
}

/// `POST /peer/work/runs`: a run is coming, with the snapshot's files. A
/// second call for the same run (a transfer broken off) answers what
/// arrived of each.
pub async fn begin(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    b: BeginBody,
) -> ApiResult<Value> {
    let dir = in_dir(state, &b.run)?;
    check_begin(&b, &dir)?;
    if b.work != work_id(from, &b.project) {
        return Err(bad("The work folder."));
    }
    let checkout = locate(state, from, &b.project).await?;
    let dangerous = b.dangerous || flagged_here(&b.command, &checkout);
    check_dangerous(state, ctx, dangerous)?;
    if let Some(run) = state.machines.work.get(&b.run) {
        let (mine, sending) = run.peek(|r| {
            (
                r.direction == Direction::In && r.machine == from,
                r.state == RunState::Sending,
            )
        });
        if !mine || !sending {
            return Err(ApiError::conflict(
                "run_exists",
                "That run is known here already.",
            ));
        }
    } else {
        let folder = state
            .machines
            .work
            .folder(&b.work)
            .ok_or_else(|| ApiError::internal("This build has no data folder."))?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(e.to_string()))?;
        crate::claude::transfer::private_dir(&dir);
        crate::persist::write_json_private(&dir.join(MANIFEST), &b.files)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let shell = Shell::here();
        let record = RunRecord {
            id: b.run.clone(),
            direction: Direction::In,
            machine: from.to_string(),
            machine_name: m::peer_name(state, from),
            os: crate::machines::protocol::this_os().to_string(),
            shell: shell.kind.as_str().to_string(),
            session: b.session.clone(),
            session_title: b.session_title.clone(),
            project: b.project.clone(),
            project_name: if b.project_name.is_empty() {
                crate::machines::setup::folders::name_of(&b.project)
            } else {
                b.project_name.chars().take(200).collect()
            },
            work: b.work.clone(),
            folder: folder.display().to_string(),
            checkout: checkout.display().to_string(),
            cwd: b.cwd.clone(),
            command: b.command.clone(),
            program: super::program_of(&b.command),
            command_sha256: super::command_sha256(&b.command),
            dangerous,
            timeout_secs: b.timeout_secs,
            artifacts_glob: b.artifacts.clone(),
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
            approved_by: None,
            grant: None,
            unreachable: false,
            updated_at: m::now(),
        };
        state.machines.work.prune();
        let run =
            state.machines.work.insert(record).ok_or_else(|| {
                ApiError::conflict("run_exists", "That run is known here already.")
            })?;
        state.machines.work.save(&run);
        audit(state, "machine.run.asked", &run.record());
        publish(state);
    }
    let files: Vec<Value> = b
        .files
        .iter()
        .map(|f| json!({ "name": f.name, "have": have_of(&dir, &f.name).min(f.size) }))
        .collect();
    Ok(json!({ "files": files }))
}

/// `PUT /peer/work/runs/{run}/files/{name}?offset=N`: the next chunk.
pub async fn put_chunk(
    state: &AppState,
    from: &str,
    id: &str,
    name: &str,
    offset: u64,
    bytes: axum::body::Bytes,
) -> ApiResult<Value> {
    let run = run_of(state, id, from)?;
    if run.state() != RunState::Sending {
        return Err(ApiError::conflict(
            "run_started",
            "That run has its code already.",
        ));
    }
    let dir = in_dir(state, id)?;
    let f = manifest(&dir)
        .and_then(|l| l.into_iter().find(|f| f.name == name))
        .ok_or_else(|| ApiError::not_found("not_found", "No such file in this run."))?;
    let path = moves::file_path(&dir, name)
        .ok_or_else(|| ApiError::unprocessable("run_bad_request", "No such file."))?;
    let _one = state.machines.work.chunks.lock().await;
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
                    "run_bad_request",
                    "The file is longer than the run said.",
                ),
                AppendError::Refused(Refusal::TooLarge) => {
                    ApiError::unprocessable("run_bad_request", "A chunk is at most 8 MiB.")
                }
                AppendError::Io(e) => {
                    ApiError::internal(format!("The file could not be written here: {e}"))
                }
            })?;
    Ok(json!({ "name": name, "size": written }))
}

/// Ends a run that never got to run (the code did not land, a cancel or
/// the timeout while it waited).
fn finish_early(state: &AppState, run: &Arc<Run>, s: RunState, error: Option<String>) {
    let now = m::now();
    run.update(|r| {
        r.state = s;
        if r.error.is_none() {
            r.error = error;
        }
        r.ended_at = Some(now);
    });
    state.machines.work.save(run);
    audit(state, "machine.run", &run.record());
    publish(state);
}

/// `POST /peer/work/runs/{run}/start`: every file whole, the snapshot
/// fetched into this computer's checkout, then the run starts (it waits
/// for its work folder first). Answers its status.
pub async fn start(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    id: &str,
) -> ApiResult<Value> {
    let run = run_of(state, id, from)?;
    let r = run.record();
    if r.state != RunState::Sending {
        return Ok(status_view(&run, true));
    }
    check_dangerous(state, ctx, r.dangerous)?;
    let dir = in_dir(state, id)?;
    let files = manifest(&dir).ok_or_else(|| {
        ApiError::conflict(
            "run_incomplete",
            "The run's code did not come here: send it again.",
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
            "run_incomplete",
            format!("The run's code did not arrive whole: {why}."),
        )
    })?;
    // The code, into this computer's checkout (its objects and refs only).
    let git = state.machines.moves.git();
    let (g, checkout, code_dir, work) = (
        git.clone(),
        PathBuf::from(&r.checkout),
        dir.join("code"),
        r.work.clone(),
    );
    let fetched = tokio::task::spawn_blocking(move || {
        let f = code::fetch_snapshot(&g, &checkout, &code_dir, &work)?;
        // The newest snapshots stay: what the next bundle chains on.
        let _ = code::prune_snapshots(&g, &checkout, &work, super::KEEP_SNAPSHOTS);
        Ok::<_, code::CodeError>(f)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let fetched = match fetched {
        Ok(f) => f,
        Err(e) => {
            let why = format!("The code could not be taken in: {e}");
            finish_early(state, &run, RunState::Error, Some(why.clone()));
            return Err(ApiError::unprocessable("run_failed", why));
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    run.update(|r| r.state = RunState::Queued);
    state.machines.work.save(&run);
    audit(state, "machine.run.start", &run.record());
    publish(state);
    let st = state.clone();
    let rn = run.clone();
    tokio::spawn(async move { runner(st, rn, fetched.meta.snap).await });
    Ok(status_view(&run, true))
}

/// When a run must have ended, as an instant.
fn deadline_of(r: &RunRecord) -> tokio::time::Instant {
    let end = r.created_at + (r.timeout_secs as i64) * 1000;
    let left = (end - m::now()).max(0) as u64;
    tokio::time::Instant::now() + Duration::from_millis(left)
}

async fn until_true(rx: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// The run itself: its work folder's turn, the folder set to the
/// snapshot, the command in this computer's shell, the files its glob
/// names, the record kept and audited.
async fn runner(state: AppState, run: Arc<Run>, snap: String) {
    let r = run.record();
    let deadline = deadline_of(&r);
    let mut cancel = run.cancelled();
    // One run at a time per work folder.
    let lock = state.machines.work.lock_of(&r.work);
    let guard = tokio::select! {
        g = lock.lock_owned() => Some(g),
        _ = until_true(&mut cancel) => None,
        _ = tokio::time::sleep_until(deadline) => None,
    };
    let Some(guard) = guard else {
        let (s, why) = if run.cancel_asked() {
            (RunState::Cancelled, None)
        } else {
            (
                RunState::TimedOut,
                Some("Its time ran out while another run used the work folder.".to_string()),
            )
        };
        finish_early(&state, &run, s, why);
        return;
    };
    run.update(|r| r.state = RunState::Preparing);
    publish(&state);
    let git = state.machines.moves.git();
    let (g, checkout, folder) = (
        git.clone(),
        PathBuf::from(&r.checkout),
        PathBuf::from(&r.folder),
    );
    let s = snap.clone();
    let reset =
        tokio::task::spawn_blocking(move || code::reset_work_tree(&g, &checkout, &folder, &s))
            .await;
    match reset {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            finish_early(
                &state,
                &run,
                RunState::Error,
                Some(format!("The work folder could not be set to the code: {e}")),
            );
            return;
        }
        Err(e) => {
            finish_early(&state, &run, RunState::Error, Some(e.to_string()));
            return;
        }
    }
    state.machines.work.note_reset(&r.work);
    let dir = if r.cwd.is_empty() {
        PathBuf::from(&r.folder)
    } else {
        PathBuf::from(&r.folder).join(&r.cwd)
    };
    if !dir.is_dir() {
        finish_early(
            &state,
            &run,
            RunState::Error,
            Some(format!(
                "{} is not a folder of the project at this code.",
                r.cwd
            )),
        );
        return;
    }
    let out_path = state
        .machines
        .work
        .run_dir(&r.id)
        .map(|d| d.join(output::FILE));
    *run.output.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Output::new(out_path.as_deref(), TAIL));
    let started = m::now();
    run.update(|r| {
        r.state = RunState::Running;
        r.started_at = Some(started);
    });
    state.machines.work.save(&run);
    publish(&state);
    let sink_run = run.clone();
    let sink: shell::Sink = Arc::new(move |bytes: &[u8]| {
        if let Some(o) = sink_run
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            o.push(bytes);
        }
        sink_run.poke();
    });
    let ending = shell::execute(Shell::here(), &r.command, &dir, sink, cancel, deadline).await;
    let artifacts = match &r.artifacts_glob {
        Some(glob) if matches!(ending, Ok(Ending::Exited(_))) => {
            let (f, g) = (PathBuf::from(&r.folder), glob.clone());
            tokio::task::spawn_blocking(move || list_artifacts(&f, &g))
                .await
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    drop(guard);
    let mut out = run.output.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(o) = out.as_mut() {
        o.flush();
    }
    let ended = m::now();
    run.update(|r| {
        if let Some(o) = &out {
            r.output_size = o.size();
            r.truncated = o.truncated();
            r.tail = o.tail(TAIL);
        }
        r.ended_at = Some(ended);
        r.duration_ms = Some((ended - started).max(0) as u64);
        r.artifacts = artifacts;
        match ending {
            Ok(Ending::Exited(code)) => {
                r.exit_code = code;
                r.state = if code == Some(0) {
                    RunState::Done
                } else {
                    RunState::Failed
                };
            }
            Ok(Ending::Cancelled) => r.state = RunState::Cancelled,
            Ok(Ending::TimedOut) => r.state = RunState::TimedOut,
            Err(e) => {
                r.state = RunState::Error;
                r.error = Some(format!(
                    "{} could not start: {e}",
                    Shell::here().kind.label()
                ));
            }
        }
    });
    state.machines.work.save(&run);
    audit(&state, "machine.run", &run.record());
    publish(&state);
}

/// The files `glob` names under `top` (never inside `.git`, never through
/// a link), at most [`MAX_ARTIFACTS`], by path.
pub fn list_artifacts(top: &Path, glob: &str) -> Vec<Artifact> {
    let fold = cfg!(windows);
    // Start the walk at the glob's literal folders, when it has some.
    let g = glob.trim_start_matches('/');
    let mut base: Vec<String> = Vec::new();
    if g.contains('/') {
        let segs: Vec<&str> = g.split('/').collect();
        for s in &segs[..segs.len() - 1] {
            if s.contains(['*', '?', '[', '{']) || s.is_empty() {
                break;
            }
            base.push((*s).to_string());
        }
    }
    let start = base.iter().fold(top.to_path_buf(), |p, s| p.join(s));
    let mut out = Vec::new();
    let mut seen = 0usize;
    let mut stack = vec![(start, base)];
    while let Some((dir, comps)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            seen += 1;
            if seen > MAX_WALK || out.len() >= MAX_ARTIFACTS * 4 {
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if comps.is_empty() && name.eq_ignore_ascii_case(".git") {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
                continue;
            };
            let mut c = comps.clone();
            c.push(name);
            if meta.is_dir() {
                stack.push((e.path(), c));
            } else if meta.is_file() && crate::orchestration::guard::glob_matches(glob, &c, fold) {
                out.push(Artifact {
                    path: c.join("/"),
                    size: meta.len(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.truncate(MAX_ARTIFACTS);
    out
}

// ─── its status, output, cancel ────────────────────────────────────────────────

/// A run's status as the asking computer follows it: its record (its
/// output's last 64 KiB once it ended, 16 KiB while it runs, without it
/// when `tail` is false).
pub fn status_view(run: &Run, tail: bool) -> Value {
    let r = run.record();
    let mut v = r.summary();
    if let Some(o) = v.as_object_mut() {
        o.remove("tail");
        o.remove("fetched");
        o.remove("approvedBy");
        o.remove("grant");
        if tail {
            let n = if r.state.ended() { TAIL } else { LIVE_TAIL };
            o.insert("tail".into(), json!(tail_of(&r.tail, n)));
        }
    }
    json!({ "run": v })
}

#[derive(Debug, Default, Deserialize)]
pub struct StatusQuery {
    /// Seconds to wait for news (at most 25).
    #[serde(default)]
    pub wait: Option<u64>,
    /// The output size the asking computer has.
    #[serde(default)]
    pub size: Option<u64>,
    /// The state it knows.
    #[serde(default)]
    pub state: Option<String>,
}

/// `GET /peer/work/runs/{run}?wait=&size=&state=`: its status, once it
/// differs from what the asking computer knows (or `wait` went by); the
/// output's tail only when it grew.
pub async fn status(state: &AppState, from: &str, id: &str, q: StatusQuery) -> ApiResult<Value> {
    let run = run_of(state, id, from)?;
    let wait = Duration::from_secs(q.wait.unwrap_or(0).min(MAX_WAIT_SECS));
    let started = tokio::time::Instant::now();
    let deadline = started + wait;
    let mut rx = run.watch();
    loop {
        let r = run.peek(|r| r.state);
        let size = run.record_size();
        let state_news = q.state.as_deref() != Some(r.as_str());
        let size_news = q.size != Some(size);
        if r.ended() || state_news || (size_news && started.elapsed() >= Duration::from_secs(1)) {
            break;
        }
        let until = if size_news {
            // News in the output: answer within a second, not at once (a
            // chatty build would otherwise be asked about without end).
            deadline.min(started + Duration::from_secs(1))
        } else {
            deadline
        };
        if tokio::time::timeout_at(until, rx.changed()).await.is_err()
            && tokio::time::Instant::now() >= deadline
        {
            break;
        }
    }
    let grew = q.size != Some(run.record_size());
    let ended = run.state().ended();
    Ok(status_view(&run, grew || ended))
}

#[derive(Debug, Default, Deserialize)]
pub struct OutputQuery {
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub max: Option<usize>,
}

/// `GET /peer/work/runs/{run}/output?from=&max=`: a page of the whole
/// output (kept a day).
pub async fn output_page(
    state: &AppState,
    from: &str,
    id: &str,
    q: OutputQuery,
) -> ApiResult<Value> {
    let run = run_of(state, id, from)?;
    let truncated = run.record().truncated;
    let Some(path) = state
        .machines
        .work
        .run_dir(id)
        .map(|d| d.join(output::FILE))
    else {
        return Err(ApiError::internal("This build has no data folder."));
    };
    if let Some(o) = run
        .output
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
    {
        o.flush();
    }
    let (f, max) = (q.from.unwrap_or(0), q.max.unwrap_or(TAIL));
    let page = tokio::task::spawn_blocking(move || {
        if path.is_file() {
            output::read_page(&path, f, max).map(Some)
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(format!("The output could not be read: {e}")))?;
    let mut v = match page {
        Some(p) => serde_json::to_value(p).unwrap_or(Value::Null),
        None => json!({ "from": 0, "to": 0, "size": 0, "text": "", "more": false }),
    };
    v["truncated"] = json!(truncated);
    v["state"] = json!(run.state());
    Ok(v)
}

/// `POST /peer/work/runs/{run}/cancel`: stops it (its whole tree).
pub async fn cancel(state: &AppState, from: &str, id: &str) -> ApiResult<Value> {
    let run = run_of(state, id, from)?;
    let s = run.state();
    if !s.ended() {
        run.ask_cancel();
        if s == RunState::Sending {
            finish_early(state, &run, RunState::Cancelled, None);
        } else {
            // Its runner ends it; a moment for that to show.
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
    }
    Ok(status_view(&run, true))
}

// ─── the files it made ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ArtifactQuery {
    pub path: String,
    #[serde(default)]
    pub offset: Option<u64>,
}

/// One of the run's listed files, here.
fn artifact_path(state: &AppState, run: &Run, rel: &str) -> ApiResult<(PathBuf, u64)> {
    let r = run.record();
    let listed =
        r.artifacts.iter().find(|a| a.path == rel).ok_or_else(|| {
            ApiError::not_found("artifact_unknown", "The run listed no such file.")
        })?;
    if !code::safe_rel_path(rel) {
        return Err(ApiError::not_found(
            "artifact_unknown",
            "The run listed no such file.",
        ));
    }
    if state
        .machines
        .work
        .reset_since(&r.work, r.ended_at.unwrap_or(0))
    {
        return Err(ApiError::conflict(
            "artifact_gone",
            "Another run used the work folder since: run it again to get the file.",
        ));
    }
    let path = rel
        .split('/')
        .fold(PathBuf::from(&r.folder), |p, s| p.join(s));
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|_| ApiError::not_found("artifact_gone", "The file is not there any more."))?;
    if !meta.is_file() {
        return Err(ApiError::not_found(
            "artifact_gone",
            "The file is not there any more.",
        ));
    }
    if meta.len() > MAX_ARTIFACT {
        return Err(ApiError::new(
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "artifact_too_large",
            "Files of more than 64 MiB stay on the computer that made them.",
        ));
    }
    if meta.len() != listed.size {
        return Err(ApiError::conflict(
            "artifact_gone",
            "The file changed since the run listed it.",
        ));
    }
    Ok((path, meta.len()))
}

/// `GET /peer/work/runs/{run}/artifact?path=`: a listed file's size and
/// SHA-256.
pub async fn artifact_info(
    state: &AppState,
    from: &str,
    id: &str,
    q: ArtifactQuery,
) -> ApiResult<Value> {
    let run = run_of(state, id, from)?;
    let (path, size) = artifact_path(state, &run, &q.path)?;
    let (_, sha) = tokio::task::spawn_blocking(move || crate::claude::transfer::sha256_file(&path))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(json!({ "path": q.path, "size": size, "sha256": sha }))
}

/// `GET /peer/work/runs/{run}/artifact/bytes?path=&offset=`: up to 8 MiB
/// of a listed file.
pub async fn artifact_bytes(
    state: &AppState,
    from: &str,
    id: &str,
    q: ArtifactQuery,
) -> ApiResult<Vec<u8>> {
    let run = run_of(state, id, from)?;
    let (path, size) = artifact_path(state, &run, &q.path)?;
    let offset = q.offset.unwrap_or(0).min(size);
    let n = ((size - offset) as usize).min(chunks::CHUNK);
    tokio::task::spawn_blocking(move || chunks::read_chunk(&path, offset, n))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::internal(e.to_string()))
}

// ─── the setup's echo ──────────────────────────────────────────────────────────

/// `POST /peer/work/echo`: the readiness test's run here — a fixed `echo`
/// in a scratch folder, for a computer holding this one's `run` grant
/// (Machines may still be off: the setup runs before).
pub async fn echo(state: &AppState, ctx: &ClientContext) -> ApiResult<Value> {
    let (machine, lan) = peer_of(ctx)?;
    if !state.machines.allows(&machine, Grant::Run, lan) {
        return Err(grant_missing(state, Grant::Run));
    }
    let shell = Shell::here();
    match shell::echo().await {
        Ok((code, text, ms)) => Ok(json!({
            "ok": code == Some(0) && text.contains(shell::ECHO_SAYS),
            "exitCode": code,
            "output": tail_of(&text, 1024),
            "ms": ms,
            "shell": shell.kind.as_str(),
            "shellLabel": shell.kind.label(),
        })),
        Err(e) => Ok(json!({
            "ok": false,
            "error": e,
            "shell": shell.kind.as_str(),
            "shellLabel": shell.kind.label(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_to_run_in_stays_inside_the_project() {
        assert_eq!(clean_cwd("").as_deref(), Some(""));
        assert_eq!(clean_cwd(".").as_deref(), Some(""));
        assert_eq!(clean_cwd("./src").as_deref(), Some("src"));
        assert_eq!(clean_cwd(r"src\app\").as_deref(), Some("src/app"));
        assert!(
            clean_cwd("/src").is_none(),
            "a full path is not a folder of the project"
        );
        assert_eq!(clean_cwd("src/./app").as_deref(), Some("src/app"));
        assert!(clean_cwd("../other").is_none());
        assert!(clean_cwd("src/../../x").is_none());
        assert!(clean_cwd(".git/hooks").is_none());
        assert!(clean_cwd(r"C:\Windows").is_none());
        assert!(clean_glob("target/release/*.exe").is_some());
        assert!(clean_glob("**/*.log").is_some());
        assert!(clean_glob("../secrets/*").is_none());
        assert!(clean_glob(".git/config").is_none());
        assert!(clean_glob("/etc/passwd").is_none());
        assert!(clean_glob("").is_none());
    }

    #[test]
    fn a_glob_lists_the_files_it_names_and_never_git_s() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        for (p, n) in [
            ("target/release/app.exe", 3),
            ("target/release/app.pdb", 5),
            ("target/debug/app.exe", 7),
            ("dist/a.js", 1),
            (".git/HEAD", 1),
        ] {
            let f = top.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, vec![b'x'; n]).unwrap();
        }
        let got = list_artifacts(top, "target/release/*.exe");
        assert_eq!(
            got,
            vec![Artifact {
                path: "target/release/app.exe".into(),
                size: 3
            }]
        );
        let all: Vec<String> = list_artifacts(top, "*.exe")
            .into_iter()
            .map(|a| a.path)
            .collect();
        assert_eq!(all, vec!["target/debug/app.exe", "target/release/app.exe"]);
        let release: Vec<String> = list_artifacts(top, "target/release")
            .into_iter()
            .map(|a| a.path)
            .collect();
        assert_eq!(
            release,
            vec!["target/release/app.exe", "target/release/app.pdb"]
        );
        assert!(list_artifacts(top, "HEAD").is_empty(), "never inside .git");
        assert!(list_artifacts(top, "missing/*").is_empty());
    }
}
