//! Running commands on another computer (docs/PLAN-MACHINES.md §5.11, Phase
//! 4 and its decisions 1–5 and 7): a Claude Code session on this computer
//! builds and tests on another one — the Mac's session runs the Windows
//! build there — without moving and without anyone switching computers.
//!
//! **Who agrees.** The target's `run` grant for the source (off until the
//! target's desktop turns it on, LAN and internet apart), and an approval
//! here for every run (`crate::pending`, source `machine`: the computer,
//! the folder, the command) unless a standing grant (`grants`) covers it —
//! project + computer + command prefix, attached from the approval with the
//! dangerous right. A command `pending::sensitive_command` flags is a
//! dangerous approval, and the target then requires its own `dangerous`
//! grant for this computer too (`X-MC-Acting`, `machines::acting`).
//!
//! **Where it runs** (`target`, decision 1): a kept work folder per project
//! and source computer, `<app data>/machines/work/<wk_…>/`, a `git
//! worktree` of the target's own checkout (the folder a root maps the
//! project to). Before each run it is set to the session's code: this
//! computer snapshots its checkout (`code::snapshot`, chained, a bundle of
//! what the target lacks), the target fetches it and resets the work folder
//! to it — tracked and untracked files alike — keeping the files git
//! ignores, so caches survive (`code::reset_work_tree`). The user's own
//! checkout there is never touched. No checkout there: *Clone it there*.
//!
//! **How it runs** (`shell`, decision 3): macOS `/bin/zsh -lc`, Windows
//! PowerShell 7 when present, else Windows PowerShell (`-NoProfile
//! -NonInteractive`), the environment scrubbed as for owned children (no
//! tokens, no API keys) plus `MOTHER_CLAUDE_RUN=1`; one run at a time per
//! work folder (a second waits, within its timeout); a timeout (30 minutes
//! by default, 2 hours at most) or a cancel ends the whole process tree,
//! and so does the end of the command for anything it left running.
//!
//! **What comes back** (`output`, decision 4): the exit code, the duration
//! and the output's last 64 KiB; the whole output kept there a day, paged
//! (`machine_output`); files a glob names listed with their sizes and
//! fetched on request (64 MiB at most each) into the session's project
//! under `.mother-claude/artifacts/<run>/`, which git ignores.
//!
//! **Long runs** (decision 5): `machine_run` waits up to 10 minutes, then
//! answers *still running* with the run's id; `machine_wait`,
//! `machine_output`, `machine_cancel`. A run is a background task of its
//! session on the session page (`session_tasks`), its output read the way
//! a shell's is; every change is a `{t:"machines", what:"runs"}` frame.
//!
//! **Records** (decision 7): every run is audited on both computers with
//! the program's name and the SHA-256 of the command line (never the line
//! itself), the folder, the exit code and the duration. Each computer keeps
//! its runs — the ones it asked for and the ones it ran — under
//! `<app data>/machines/work/runs/<run>/` (0600) for a day after they end.
//!
//! Machines off blocks use: the target's `/peer/work/*` answers `403
//! machines_off`, this computer's tools `machines_off`; turning Machines
//! off, revoking a computer or taking back its `run` grant stops what that
//! computer runs here.

pub mod grants;
pub mod mcp;
pub mod output;
pub mod routes;
pub mod shell;
pub mod source;
pub mod target;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::claude::model::{BackgroundTask, BackgroundTaskKind};
use crate::server::devices::audit::AuditEntry;
use crate::state::AppState;

/// The capability a computer lists when it runs commands for others.
pub const CAPABILITY: &str = "run";
/// A run's longest command line (bytes): PowerShell takes it encoded in one
/// argument of Windows' 32 767-character command line.
pub const MAX_COMMAND: usize = 8000;
/// A run's timeout unless it names one, and the longest it may name.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30 * 60;
pub const MAX_TIMEOUT_SECS: u64 = 2 * 60 * 60;
/// How long `machine_run` (and `machine_wait`) waits before answering
/// *still running*.
pub const WAIT_SECS: u64 = 10 * 60;
/// What comes back at once: the output's last 64 KiB.
pub const TAIL: usize = 64 * 1024;
/// What a list and the dashboards' live view show of it.
pub const LIVE_TAIL: usize = 16 * 1024;
/// The largest file a run may bring back.
pub const MAX_ARTIFACT: u64 = 64 << 20;
/// Files a glob may list at most.
pub const MAX_ARTIFACTS: usize = 100;
/// How long an ended run (its record, its output) is kept.
pub const KEEP_MS: i64 = 24 * 60 * 60 * 1000;
/// Runs kept at most (the newest).
pub const MAX_KEPT: usize = 200;
/// How many snapshots of a work folder each side keeps refs for (what the
/// next bundle chains on).
pub const KEEP_SNAPSHOTS: usize = 3;
/// The folder (under `machines/`) holding the work folders and the runs.
pub const DIR: &str = "work";

// ─── ids ───────────────────────────────────────────────────────────────────────

fn hex_id(prefix: &str, s: &str, len: usize) -> bool {
    s.strip_prefix(prefix).is_some_and(|h| {
        h.len() == len && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// `run_` and 16 lowercase hex characters.
pub fn is_run_id(s: &str) -> bool {
    hex_id("run_", s, 16)
}

pub fn new_run_id() -> String {
    let hex: String = crate::server::auth::random_bytes(8)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("run_{hex}")
}

/// `wk_` and 16 lowercase hex characters: a work folder's id.
pub fn is_work_id(s: &str) -> bool {
    hex_id("wk_", s, 16)
}

/// The work folder of the project at `project` (its top, as the source
/// computer writes it) on behalf of `source`: the same id on both
/// computers, so each keeps its snapshots under the same refs.
pub fn work_id(source: &str, project: &str) -> String {
    let key = crate::claude::paths::path_key(project);
    let digest = crate::claude::secrets::sha256_hex(format!("{source}\n{key}").as_bytes());
    format!("wk_{}", &digest[..16])
}

/// The program a command line starts (its first word, without a folder or
/// `.exe`), for records that never hold the line itself.
pub fn program_of(command: &str) -> String {
    let first = command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(['"', '\'', '&', '(', ')']);
    let base = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let base = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base);
    let clean: String = base.chars().filter(|c| !c.is_control()).take(64).collect();
    if clean.is_empty() {
        "(none)".into()
    } else {
        clean
    }
}

/// The SHA-256 of a command line (what the records keep of it).
pub fn command_sha256(command: &str) -> String {
    crate::claude::secrets::sha256_hex(command.as_bytes())
}

// ─── a run ─────────────────────────────────────────────────────────────────────

/// Where a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunState {
    /// The asking computer snapshots the code and sends it.
    Sending,
    /// It waits for its work folder (another run uses it).
    Queued,
    /// The work folder is being set to the snapshot.
    Preparing,
    Running,
    /// It ended with exit code 0.
    Done,
    /// It ended with another exit code.
    Failed,
    Cancelled,
    TimedOut,
    /// It could not run: the code did not arrive or land, the shell could
    /// not start (said in `error`).
    Error,
    /// The asking computer lost track of it: the other computer stayed out
    /// of reach, or no longer knows it.
    Lost,
}

impl RunState {
    pub fn ended(self) -> bool {
        matches!(
            self,
            RunState::Done
                | RunState::Failed
                | RunState::Cancelled
                | RunState::TimedOut
                | RunState::Error
                | RunState::Lost
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Sending => "sending",
            RunState::Queued => "queued",
            RunState::Preparing => "preparing",
            RunState::Running => "running",
            RunState::Done => "done",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
            RunState::TimedOut => "timedOut",
            RunState::Error => "error",
            RunState::Lost => "lost",
        }
    }
}

/// Which way a run went, from this computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    /// This computer asked another to run it.
    Out,
    /// Another computer asked this one.
    In,
}

/// A file a run produced (a glob named it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    /// Relative to the project's top, with `/`.
    pub path: String,
    pub size: u64,
}

/// An artifact brought into the asking session's project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedArtifact {
    pub path: String,
    pub to: String,
    pub size: u64,
    pub at: i64,
}

/// A run, as either computer keeps it (`run.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub direction: Direction,
    /// The other computer's id.
    pub machine: String,
    #[serde(default)]
    pub machine_name: String,
    /// The OS and the shell it runs in (the target's).
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub shell: String,
    /// The asking session (on the asking computer).
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    /// The project's top on the asking computer, as it writes it.
    pub project: String,
    #[serde(default)]
    pub project_name: String,
    /// The work folder's id, the work folder and the checkout it is a
    /// worktree of (on the computer that runs it).
    pub work: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub checkout: String,
    /// Where it runs, relative to the project's top (`""`: the top).
    #[serde(default)]
    pub cwd: String,
    pub command: String,
    #[serde(default)]
    pub program: String,
    #[serde(default)]
    pub command_sha256: String,
    #[serde(default)]
    pub dangerous: bool,
    pub timeout_secs: u64,
    #[serde(default)]
    pub artifacts_glob: Option<String>,
    pub state: RunState,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    pub created_at: i64,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub ended_at: Option<i64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Bytes of output so far.
    #[serde(default)]
    pub output_size: u64,
    /// The output went past what is kept of it.
    #[serde(default)]
    pub truncated: bool,
    /// The output's last 64 KiB (16 KiB while it runs, on the asking side).
    #[serde(default)]
    pub tail: String,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub fetched: Vec<FetchedArtifact>,
    /// Who allowed it here (the asking side): a person's name, or the
    /// standing grant's id in `grant`.
    #[serde(default)]
    pub approved_by: Option<String>,
    #[serde(default)]
    pub grant: Option<String>,
    /// The asking side: the other computer did not answer the last time.
    #[serde(default)]
    pub unreachable: bool,
    #[serde(default)]
    pub updated_at: i64,
}

/// `s`'s last `n` bytes, from a character's start.
pub fn tail_of(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut start = s.len() - n;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

impl RunRecord {
    /// What a list shows: everything but the output, of which its last
    /// 2 KiB.
    pub fn summary(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            o.insert("tail".into(), json!(tail_of(&self.tail, 2048)));
        }
        v
    }

    /// Everything (`GET /api/machines/runs/{id}`).
    pub fn full(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// The task this run is on its session's page.
    pub fn task(&self) -> BackgroundTask {
        let status = match self.state {
            RunState::Sending | RunState::Queued | RunState::Preparing | RunState::Running => {
                "running"
            }
            RunState::Done => "completed",
            RunState::Failed | RunState::Error | RunState::Lost => "failed",
            RunState::Cancelled | RunState::TimedOut => "killed",
        };
        let name = if self.machine_name.is_empty() {
            self.machine.chars().take(8).collect()
        } else {
            self.machine_name.clone()
        };
        let command: String = self.command.chars().take(120).collect();
        BackgroundTask {
            id: self.id.clone(),
            kind: BackgroundTaskKind::Machine,
            label: format!("{command} · {name}"),
            status: status.into(),
            started_at: Some(self.started_at.unwrap_or(self.created_at)),
            ended_at: self.ended_at,
            output_available: true,
        }
    }
}

/// A run in memory: its record, a counter waiters watch, and its cancel.
pub struct Run {
    pub id: String,
    record: Mutex<RunRecord>,
    tick: tokio::sync::watch::Sender<u64>,
    cancel: tokio::sync::watch::Sender<bool>,
    /// The computer that runs it: its output as it arrives.
    pub(crate) output: Mutex<Option<output::Output>>,
}

impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Run").field("id", &self.id).finish()
    }
}

impl Run {
    fn new(record: RunRecord) -> Arc<Self> {
        Arc::new(Self {
            id: record.id.clone(),
            record: Mutex::new(record),
            tick: tokio::sync::watch::channel(0).0,
            cancel: tokio::sync::watch::channel(false).0,
            output: Mutex::new(None),
        })
    }

    pub fn record(&self) -> RunRecord {
        let mut r = self
            .record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        // While it runs here, the output as it is now.
        if let Some(o) = self
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            r.output_size = o.size();
            r.truncated = o.truncated();
            r.tail = o.tail(TAIL);
        }
        r
    }

    pub fn state(&self) -> RunState {
        self.peek(|r| r.state)
    }

    /// Reads the stored record (without the live output: cheap).
    pub fn peek<R>(&self, f: impl FnOnce(&RunRecord) -> R) -> R {
        f(&self.record.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Bytes of output so far (the live count while it runs here).
    pub fn record_size(&self) -> u64 {
        if let Some(o) = self
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            return o.size();
        }
        self.peek(|r| r.output_size)
    }

    /// Changes the record and wakes whoever waits on it.
    pub(crate) fn update<R>(&self, f: impl FnOnce(&mut RunRecord) -> R) -> R {
        let out = {
            let mut r = self.record.lock().unwrap_or_else(|e| e.into_inner());
            let out = f(&mut r);
            r.updated_at = super::now();
            out
        };
        self.poke();
        out
    }

    /// Wakes whoever waits (the output grew).
    pub(crate) fn poke(&self) {
        self.tick.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub(crate) fn watch(&self) -> tokio::sync::watch::Receiver<u64> {
        self.tick.subscribe()
    }

    /// Asks it to stop (the computer that runs it kills its tree).
    pub(crate) fn ask_cancel(&self) {
        self.cancel.send_replace(true);
        self.poke();
    }

    pub(crate) fn cancelled(&self) -> tokio::sync::watch::Receiver<bool> {
        self.cancel.subscribe()
    }

    pub(crate) fn cancel_asked(&self) -> bool {
        *self.cancel.borrow()
    }

    /// Waits until it ended, or `secs` went by; answers its record.
    pub async fn wait_end(&self, secs: u64) -> RunRecord {
        let mut rx = self.watch();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
        loop {
            if self.state().ended() {
                break;
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => continue,
                _ => break,
            }
        }
        self.record()
    }
}

// ─── the state ─────────────────────────────────────────────────────────────────

/// Everything runs keep at run time (`MachinesState::work`).
pub struct WorkState {
    dir: Option<PathBuf>,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    /// One run at a time per work folder (the computer that runs them).
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// When each work folder was last set to a snapshot (its earlier runs'
    /// files are gone then).
    resets: Mutex<HashMap<String, i64>>,
    /// Incoming chunks are written one at a time.
    pub(crate) chunks: tokio::sync::Mutex<()>,
    /// The standing grants for runs this computer asks for.
    pub grants: grants::RunGrants,
}

impl std::fmt::Debug for WorkState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkState").finish_non_exhaustive()
    }
}

impl WorkState {
    /// The runs kept under `<data_dir>/machines/work/runs` (a day after they
    /// ended); one this computer ran that a restart cut off is an error now.
    pub fn load(data_dir: Option<&Path>) -> Self {
        let dir = data_dir.map(|d| d.join("machines").join(DIR));
        let mut runs = HashMap::new();
        if let Some(d) = &dir {
            for record in load_runs(&d.join("runs")) {
                runs.insert(record.id.clone(), Run::new(record));
            }
        }
        Self {
            grants: grants::RunGrants::load(data_dir),
            dir,
            runs: Mutex::new(runs),
            locks: Mutex::new(HashMap::new()),
            resets: Mutex::new(HashMap::new()),
            chunks: tokio::sync::Mutex::new(()),
        }
    }

    /// `<app data>/machines/work`.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// A work folder.
    pub fn folder(&self, work: &str) -> Option<PathBuf> {
        is_work_id(work)
            .then(|| self.dir.as_ref().map(|d| d.join(work)))
            .flatten()
    }

    /// A run's own folder (its record, output, incoming and outgoing files).
    pub fn run_dir(&self, id: &str) -> Option<PathBuf> {
        is_run_id(id)
            .then(|| self.dir.as_ref().map(|d| d.join("runs").join(id)))
            .flatten()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Run>> {
        self.runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Every run, newest first.
    pub fn all(&self) -> Vec<Arc<Run>> {
        let mut out: Vec<(i64, Arc<Run>)> = self
            .runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|r| (r.peek(|r| r.created_at), r.clone()))
            .collect();
        out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        out.into_iter().map(|(_, r)| r).collect()
    }

    /// Adds a run (refused when its id is taken).
    pub(crate) fn insert(&self, record: RunRecord) -> Option<Arc<Run>> {
        let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        if runs.contains_key(&record.id) {
            return None;
        }
        let run = Run::new(record);
        runs.insert(run.id.clone(), run.clone());
        Some(run)
    }

    pub(crate) fn remove(&self, id: &str) {
        self.runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// The lock of a work folder.
    pub(crate) fn lock_of(&self, work: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(work.to_string())
            .or_default()
            .clone()
    }

    /// A work folder was just set to a snapshot.
    pub(crate) fn note_reset(&self, work: &str) {
        self.resets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(work.to_string(), super::now());
    }

    /// Whether a work folder was set to another snapshot after `t` (ms).
    pub(crate) fn reset_since(&self, work: &str, t: i64) -> bool {
        self.resets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(work)
            .is_some_and(|at| *at > t)
    }

    /// Writes a run's record (0600).
    pub(crate) fn save(&self, run: &Run) {
        let Some(dir) = self.run_dir(&run.id) else {
            return;
        };
        let record = run.record();
        if let Err(e) = crate::persist::write_json_private(&dir.join("run.json"), &record) {
            tracing::warn!(run = %run.id, error = %e, "a run's record could not be saved");
        }
    }

    /// Forgets the runs that ended more than a day ago (their folders go),
    /// and the oldest past [`MAX_KEPT`].
    pub(crate) fn prune(&self) {
        let now = super::now();
        let all = self.all();
        let mut gone = Vec::new();
        for (i, run) in all.iter().enumerate() {
            let (ended, ended_at) = run.peek(|r| (r.state.ended(), r.ended_at));
            let old = ended && ended_at.is_some_and(|t| now - t > KEEP_MS);
            if old || (i >= MAX_KEPT && ended) {
                gone.push(run.id.clone());
            }
        }
        for id in gone {
            self.remove(&id);
            if let Some(dir) = self.run_dir(&id) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

fn load_runs(dir: &Path) -> Vec<RunRecord> {
    let now = super::now();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_run_id(&name) {
            continue;
        }
        let path = e.path();
        let record: Option<RunRecord> = crate::persist::read_json(&path.join("run.json"));
        let Some(mut r) = record.filter(|r| r.id == name) else {
            let _ = std::fs::remove_dir_all(&path);
            continue;
        };
        if r.ended_at.is_some_and(|t| now - t > KEEP_MS) && r.state.ended() {
            let _ = std::fs::remove_dir_all(&path);
            continue;
        }
        if r.direction == Direction::In && !r.state.ended() {
            // It ran here when Mother Claude stopped: whatever it started is
            // no longer watched.
            r.state = RunState::Error;
            r.error = Some("Mother Claude stopped on this computer while it ran.".into());
            r.ended_at = Some(now);
            let _ = crate::persist::write_json_private(&path.join("run.json"), &r);
        }
        out.push(r);
    }
    out
}

// ─── shared ────────────────────────────────────────────────────────────────────

/// Runs changed: dashboards refetch `/api/machines/runs` (and a session's
/// page its tasks).
pub fn publish(state: &AppState) {
    super::publish(state, "runs");
}

/// A run's audit line, on the computer that writes it: the program's name
/// and the command's hash (never the line: it can carry secrets), the
/// folder, the exit code and the duration.
pub fn audit(state: &AppState, action: &str, r: &RunRecord) {
    state.devices.audit.record(AuditEntry {
        action: action.to_string(),
        device_id: Some(r.machine.clone()),
        device_name: Some(super::peer_name(state, &r.machine)),
        detail: Some(json!({
            "run": r.id,
            "direction": r.direction,
            "session": r.session,
            "program": r.program,
            "commandSha256": r.command_sha256,
            "folder": r.folder,
            "cwd": r.cwd,
            "state": r.state,
            "exitCode": r.exit_code,
            "durationMs": r.duration_ms,
            "dangerous": r.dangerous,
            "grant": r.grant,
        })),
        ..AuditEntry::now()
    });
}

/// `GET /api/machines/runs`: both ways, newest first.
pub fn view(state: &AppState) -> Value {
    state.machines.work.prune();
    let runs: Vec<Value> = state
        .machines
        .work
        .all()
        .iter()
        .take(100)
        .map(|r| r.record().summary())
        .collect();
    json!({ "runs": runs })
}

/// The session's runs as background tasks (its page's *Background tasks*).
pub fn session_tasks(state: &AppState, session: &str) -> Vec<BackgroundTask> {
    state
        .machines
        .work
        .all()
        .into_iter()
        .filter_map(|r| {
            r.peek(|r| {
                (r.direction == Direction::Out && r.session.as_deref() == Some(session))
                    .then(|| r.task())
            })
        })
        .take(50)
        .collect()
}

/// A session's run, by its id: what its task output and stop routes use.
pub fn session_run(state: &AppState, session: &str, id: &str) -> Option<Arc<Run>> {
    state.machines.work.get(id).filter(|r| {
        r.peek(|r| r.direction == Direction::Out && r.session.as_deref() == Some(session))
    })
}

/// At app start: the runs this computer asked for that had not ended are
/// followed again (the other computer may still run them).
pub fn start(state: &AppState) {
    for run in state.machines.work.all() {
        if run.peek(|r| r.direction == Direction::Out && !r.state.ended()) {
            source::follow(state, run);
        }
    }
}

/// Stops what `machine` runs here (all computers' with `None`): Machines
/// was turned off, the computer revoked, or its `run` grant taken back.
pub fn stop_from(state: &AppState, machine: Option<&str>, why: &str) {
    for run in state.machines.work.all() {
        let hit = run.peek(|r| {
            r.direction == Direction::In
                && !r.state.ended()
                && machine.is_none_or(|m| m == r.machine)
        });
        if hit {
            tracing::info!(run = %run.id, why, "a run from another computer is stopped");
            run.update(|r| {
                if r.error.is_none() {
                    r.error = Some(why.to_string());
                }
            });
            run.ask_cancel();
        }
    }
}

/// After `machine`'s grants changed here: its runs stop when it may run
/// nothing here any more.
pub fn grants_changed(state: &AppState, machine: &str) {
    let may = state
        .machines
        .allows(machine, super::grants::Grant::Run, true)
        || state
            .machines
            .allows(machine, super::grants::Grant::Run, false);
    if !may {
        stop_from(
            state,
            Some(machine),
            "This computer took back the Run permission.",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_have_their_shapes() {
        let id = new_run_id();
        assert!(is_run_id(&id), "{id}");
        assert!(!is_run_id("run_XYZ"));
        assert!(!is_run_id("run_0123456789abcdef0"));
        let w = work_id("0123456789abcdef0123456789abcdef", "/Users/me/app");
        assert!(is_work_id(&w), "{w}");
        assert_eq!(
            w,
            work_id("0123456789abcdef0123456789abcdef", "/Users/me/app/"),
            "a trailing separator is the same project"
        );
        assert_ne!(
            w,
            work_id("fedcba9876543210fedcba9876543210", "/Users/me/app"),
            "one work folder per source computer"
        );
        assert_eq!(
            work_id("m", r"D:\Dev\App"),
            work_id("m", r"d:\dev\app"),
            "a Windows path in any case"
        );
    }

    #[test]
    fn the_program_is_the_first_word_without_its_folder() {
        assert_eq!(program_of("cargo test --lib"), "cargo");
        assert_eq!(program_of("  /usr/bin/make -j8"), "make");
        assert_eq!(program_of(r"C:\Tools\msbuild.exe app.sln"), "msbuild");
        assert_eq!(program_of(r#""C:\Program Files\x.exe" a"#), "Program");
        assert_eq!(program_of("& ./build.ps1"), "(none)");
        assert_eq!(program_of(""), "(none)");
        assert_eq!(
            command_sha256("cargo test"),
            crate::claude::secrets::sha256_hex(b"cargo test")
        );
        assert_ne!(command_sha256("cargo test"), command_sha256("cargo  test"));
    }

    #[test]
    fn a_tail_starts_at_a_character() {
        assert_eq!(tail_of("hello", 10), "hello");
        assert_eq!(tail_of("hello", 3), "llo");
        let s = "aé"; // é is two bytes
        assert_eq!(tail_of(s, 1), "");
        assert_eq!(tail_of(s, 2), "é");
    }
}
