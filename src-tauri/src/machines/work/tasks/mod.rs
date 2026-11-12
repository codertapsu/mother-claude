//! Handing a task to a Claude Code session on another computer
//! (docs/PLAN-MACHINES.md §5.11, Phase 4 decision 6): `machine_task`.
//!
//! **Who agrees.** The target's `run` grant for the source (starting a
//! session that runs code there is running code), and an approval here for
//! every task (`crate::pending`, source `machine`: the computer, its folder,
//! the task's first line). No standing grant ever covers one (a task is a
//! free-form prompt). A permission mode without approvals
//! (`bypassPermissions`, `dontAsk`) is a dangerous approval here, and the
//! target requires its own `dangerous` grant for this computer too (the
//! `X-MC-Acting` hop rule).
//!
//! **Where it works** (`target`). The asking computer snapshots its
//! session's project (`code::snapshot`, uncommitted and untracked files
//! included) and sends a bundle of what the target lacks; the target makes
//! the task its own worktree of its checkout at that snapshot,
//! `<app data>/machines/work/tasks/<task>/<project>/` (detached; the user's
//! checkout is never touched), and starts an **owned** Claude Code session
//! there — through the control registry's spawn, as a move's resume does,
//! under the account of the same name as the asking session's, else the
//! Default; in permission mode `default` unless asked otherwise — with the
//! task and a short header saying where it came from and how to finish.
//! It is an ordinary owned session: listed, driven and approved from either
//! computer. Until the app turns task sessions on
//! (`WorkState::enable_task_sessions`) a spawn is only recorded: tests never
//! start `claude`.
//!
//! **When it ends.** A task is done when its session's turn ends with no
//! approval waiting and no message queued ([`turn_ended`], from the stream's
//! `result`); it stays done unless someone sends its session another
//! message ([`turn_started`]), and a later end brings back a newer result.
//! What it did — its commits and what it left in the working tree — is a
//! snapshot of the worktree chained on the last result (`code::task_result`)
//! and comes back as a bundle; on the asking computer it becomes the branch
//! `mother-claude/<machine>/<task>` (`code::land_task_branch`): never checked
//! out, never merged, moved only forward. A task that changed nothing makes
//! no branch. The worktree stays there until the task is removed (*Remove*,
//! or 7 days after it last ended), so its session can be continued.
//!
//! **Records.** Each computer keeps its tasks — the ones it handed over and
//! the ones it took — under `<app data>/machines/work/tasks/<task>/task.json`
//! (0600); both audit them with the task's id, the computer, the folder and
//! the SHA-256 of the task's first line — never its text. Every change is a
//! `{t:"machines", what:"tasks"}` frame.

pub mod source;
pub mod target;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::target::FileSpec;
use super::Direction;
use crate::claude::model::{BackgroundTask, BackgroundTaskKind};
use crate::machines::code::TaskTip;
use crate::server::devices::audit::AuditEntry;
use crate::state::AppState;

/// The capability a computer lists when it takes tasks from others.
pub const CAPABILITY: &str = "tasks";
/// A task's longest text (bytes).
pub const MAX_TASK: usize = 64 * 1024;
/// What is kept of a session's last message.
pub const MAX_SUMMARY: usize = 16 * 1024;
/// How long a task is kept after it last ended (its worktree too).
pub const KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Tasks kept at most (the newest).
pub const MAX_KEPT: usize = 100;
/// The folder under `machines/work/` holding the tasks.
pub const DIR: &str = "tasks";
/// A task's record in its folder.
pub const RECORD: &str = "task.json";
/// The folder (in a task's own) holding what arrived: the handed-over
/// code there, a result here. Its leading dot keeps it apart from the
/// worktree, whose name never starts with one (`code::safe_name`).
pub const IN_DIR: &str = ".in";
/// The folder (there) holding the results made, one per number.
pub const RESULTS_DIR: &str = ".results";

// ─── ids ───────────────────────────────────────────────────────────────────────

/// `task_` and 16 lowercase hex characters.
pub fn is_task_id(s: &str) -> bool {
    s.strip_prefix("task_")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

pub fn new_task_id() -> String {
    let hex: String = crate::server::auth::random_bytes(8)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("task_{hex}")
}

/// A task's first line (what an approval and a list show), at most 200
/// characters.
pub fn first_line(task: &str) -> String {
    let line = task
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let clean: String = line.chars().filter(|c| !c.is_control()).collect();
    clip(&clean, 200)
}

pub(crate) fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(n.saturating_sub(1)).collect::<String>()
        )
    }
}

/// Text another computer sent, made safe to keep and show: no control
/// characters but line breaks and tabs, at most `n` bytes.
pub(crate) fn clean_text(s: &str, n: usize) -> String {
    let mut out = String::new();
    for c in s
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
    {
        if out.len() + c.len_utf8() > n {
            break;
        }
        out.push(c);
    }
    out
}

// ─── a task's state ────────────────────────────────────────────────────────────

/// Where a task is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskState {
    /// The asking computer snapshots the code and sends it.
    Sending,
    /// The code arrived: the worktree is made, the session starts.
    Starting,
    /// Its session works.
    Running,
    /// Its session waits for an answer there (an approval, a question).
    Waiting,
    /// Its turn ended: what it did is being taken (there) and brought
    /// back (here).
    Finishing,
    /// Its turn ended and what it did is back. A message to its session
    /// makes it run again.
    Done,
    /// Its session ended before its turn did (stopped there, or Mother
    /// Claude stopped); what it did is back. Continuing it runs it again.
    Stopped,
    /// Cancelled: its session was stopped. Nothing follows it any more.
    Cancelled,
    /// It could not start (said in `error`).
    Error,
    /// The asking computer lost track of it: the other computer no
    /// longer knows it.
    Lost,
}

impl TaskState {
    /// Over, as `machine_task` and `machine_wait` wait for it.
    pub fn ended(self) -> bool {
        matches!(
            self,
            TaskState::Done
                | TaskState::Stopped
                | TaskState::Cancelled
                | TaskState::Error
                | TaskState::Lost
        )
    }

    /// Nothing changes it any more: no turn of its session counts.
    pub fn is_final(self) -> bool {
        matches!(
            self,
            TaskState::Cancelled | TaskState::Error | TaskState::Lost
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TaskState::Sending => "sending",
            TaskState::Starting => "starting",
            TaskState::Running => "running",
            TaskState::Waiting => "waiting",
            TaskState::Finishing => "finishing",
            TaskState::Done => "done",
            TaskState::Stopped => "stopped",
            TaskState::Cancelled => "cancelled",
            TaskState::Error => "error",
            TaskState::Lost => "lost",
        }
    }

    /// As shown: a running task whose session waits for an answer waits.
    pub fn shown(self, asking: bool) -> TaskState {
        if asking && self == TaskState::Running {
            TaskState::Waiting
        } else {
            self
        }
    }
}

/// What a turn's end does to a task (there): `Some(Finishing)` when its
/// work is to be taken now. A turn that ends with a message queued or an
/// answer awaited goes on; one a session took up by itself after it was
/// done (a background job woke it) is taken too, and so is one that ended
/// while the last one's work was being taken (it is taken again after).
pub(crate) fn on_turn_end(s: TaskState, queued: bool, asking: bool) -> Option<TaskState> {
    match s {
        TaskState::Starting
        | TaskState::Running
        | TaskState::Waiting
        | TaskState::Finishing
        | TaskState::Done
        | TaskState::Stopped
            if !queued && !asking =>
        {
            Some(TaskState::Finishing)
        }
        TaskState::Done | TaskState::Stopped => Some(TaskState::Running),
        _ => None,
    }
}

/// What a message reaching its session does (there): a done task runs
/// again; nothing changes a cancelled one.
pub(crate) fn on_turn_start(s: TaskState) -> Option<TaskState> {
    match s {
        TaskState::Done | TaskState::Stopped | TaskState::Waiting => Some(TaskState::Running),
        _ => None,
    }
}

/// What its session's process ending does (there): a task in a turn
/// stops — its work is taken first (`Some(Stopped)` is where it ends).
pub(crate) fn on_exit(s: TaskState) -> Option<TaskState> {
    match s {
        TaskState::Starting | TaskState::Running | TaskState::Waiting => Some(TaskState::Stopped),
        _ => None,
    }
}

/// Where a task is once its work was taken: where it was going, unless a
/// new turn started meanwhile (then it runs).
pub(crate) fn after_result(then: TaskState, turned: bool) -> TaskState {
    if turned && then == TaskState::Done {
        TaskState::Running
    } else {
        then
    }
}

/// How the asking computer shows the other computer's state: an end whose
/// work is not here yet is still finishing.
pub(crate) fn mirrored(theirs: TaskState, results: u32, landed: u32) -> TaskState {
    if matches!(
        theirs,
        TaskState::Done | TaskState::Stopped | TaskState::Cancelled
    ) && landed < results
    {
        TaskState::Finishing
    } else {
        theirs
    }
}

// ─── the record ────────────────────────────────────────────────────────────────

/// A task, as either computer keeps it (`task.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub direction: Direction,
    /// The other computer's id.
    pub machine: String,
    #[serde(default)]
    pub machine_name: String,
    /// The OS of the computer that runs it.
    #[serde(default)]
    pub os: String,
    /// The asking session (on the asking computer).
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    /// The project's top on the asking computer, as it writes it.
    pub project: String,
    #[serde(default)]
    pub project_name: String,
    /// The asking session's folder inside the project (`""`: the top).
    #[serde(default)]
    pub cwd: String,
    /// The checkout the task's worktree belongs to, and the worktree (on
    /// the computer that runs it).
    #[serde(default)]
    pub checkout: String,
    #[serde(default)]
    pub folder: String,
    /// The task, whole (what its session was asked).
    pub task: String,
    #[serde(default)]
    pub first_line: String,
    #[serde(default)]
    pub first_line_sha256: String,
    #[serde(default)]
    pub permission_mode: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// The account its session runs under there (its name).
    #[serde(default)]
    pub account_name: Option<String>,
    /// The task's session (on the computer that runs it).
    #[serde(default)]
    pub task_session: Option<String>,
    pub state: TaskState,
    #[serde(default)]
    pub error: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub started_at: Option<i64>,
    /// When it last ended.
    #[serde(default)]
    pub ended_at: Option<i64>,
    /// Turns its session ended.
    #[serde(default)]
    pub turns: u32,
    /// Its session's last message.
    #[serde(default)]
    pub summary: Option<String>,
    /// Its session still had background work at its last end (it may go
    /// on by itself).
    #[serde(default)]
    pub background: bool,
    /// The handed-over snapshot (the worktree's first commit, the branch's
    /// base) and its tree.
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub base_tree: Option<String>,
    /// The asking side: the branch (`refs/heads/…`) once made, and its tip.
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub tip: Option<String>,
    /// What the last result changed: the session's commits since the
    /// handed-over code, files, and whether some of it is uncommitted.
    #[serde(default)]
    pub commits: u32,
    #[serde(default)]
    pub files: u32,
    #[serde(default)]
    pub uncommitted: bool,
    /// Results made there (the computer that runs it says).
    #[serde(default)]
    pub results: u32,
    /// Results the asking computer took.
    #[serde(default)]
    pub landed: u32,
    /// The asking side: why the last result did not move the branch.
    #[serde(default)]
    pub branch_note: Option<String>,
    /// Who allowed it here (the asking side).
    #[serde(default)]
    pub approved_by: Option<String>,
    /// A permission mode without approvals.
    #[serde(default)]
    pub dangerous: bool,
    /// The asking side: the other computer did not answer the last time.
    #[serde(default)]
    pub unreachable: bool,
    /// The asking side: the other computer no longer knows it (removed
    /// there); nothing follows it.
    #[serde(default)]
    pub gone_there: bool,
    /// The side that runs it: whether it was asked for over the local
    /// network (the `run` grant's column that let it in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_lan: Option<bool>,
    #[serde(default)]
    pub updated_at: i64,
    // ── kept, never shown ──
    /// The side that runs it: the account's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The side that runs it: commits the asking computer has (its results'
    /// bundles leave them out).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub haves: Vec<String>,
    /// The last result made (there), or taken (here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<TaskTip>,
    /// The side that runs it: the last result's files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result_files: Vec<FileSpec>,
    /// The asking side: the state the other computer last said (its
    /// results not here yet show as finishing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub their_state: Option<TaskState>,
}

/// Fields that stay in the record.
const INTERNAL: [&str; 5] = ["account", "haves", "last", "resultFiles", "theirState"];

impl TaskRecord {
    /// What a list shows: no internals, the task's first 500 characters.
    pub fn summary(&self) -> Value {
        let mut v = self.full();
        if let Some(o) = v.as_object_mut() {
            o.insert("task".into(), json!(clip(&self.task, 500)));
            o.insert(
                "summary".into(),
                json!(self.summary.as_deref().map(|s| clip(s, 2000))),
            );
        }
        v
    }

    /// Everything but the internals (`GET /api/machines/tasks/{id}`), and
    /// `branchName` once a result made the branch (`mother-claude/…`, on
    /// the asking computer).
    pub fn full(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            for k in INTERNAL {
                o.remove(k);
            }
            o.insert(
                "branchName".into(),
                json!(self
                    .branch
                    .as_deref()
                    .filter(|_| self.tip.is_some())
                    .map(crate::machines::code::short_branch)),
            );
        }
        v
    }

    /// The task this is on its asking session's page.
    pub fn background_task(&self) -> BackgroundTask {
        let status = match self.state {
            TaskState::Sending
            | TaskState::Starting
            | TaskState::Running
            | TaskState::Waiting
            | TaskState::Finishing => "running",
            TaskState::Done => "completed",
            TaskState::Stopped | TaskState::Cancelled => "killed",
            TaskState::Error | TaskState::Lost => "failed",
        };
        let name = if self.machine_name.is_empty() {
            self.machine.chars().take(8).collect()
        } else {
            self.machine_name.clone()
        };
        BackgroundTask {
            id: self.id.clone(),
            kind: BackgroundTaskKind::Machine,
            label: format!("Task: {} · {name}", clip(&self.first_line, 100)),
            status: status.into(),
            started_at: Some(self.started_at.unwrap_or(self.created_at)),
            ended_at: self.ended_at.filter(|_| self.state.ended()),
            output_available: true,
        }
    }
}

// ─── a task in memory ──────────────────────────────────────────────────────────

/// A task: its record, a counter waiters watch, and one piece of work on it
/// at a time (taking its result there, landing one here).
pub struct Task {
    pub id: String,
    record: Mutex<TaskRecord>,
    tick: tokio::sync::watch::Sender<u64>,
    pub(crate) work: tokio::sync::Mutex<()>,
    /// Turns its session started (there): a result taken while one started
    /// leaves it running.
    turns_started: AtomicU64,
}

impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Task").field("id", &self.id).finish()
    }
}

impl Task {
    fn new(record: TaskRecord) -> Arc<Self> {
        Arc::new(Self {
            id: record.id.clone(),
            record: Mutex::new(record),
            tick: tokio::sync::watch::channel(0).0,
            work: tokio::sync::Mutex::new(()),
            turns_started: AtomicU64::new(0),
        })
    }

    pub fn record(&self) -> TaskRecord {
        self.record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn state(&self) -> TaskState {
        self.peek(|r| r.state)
    }

    pub fn peek<R>(&self, f: impl FnOnce(&TaskRecord) -> R) -> R {
        f(&self.record.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Changes the record and wakes whoever waits on it.
    pub(crate) fn update<R>(&self, f: impl FnOnce(&mut TaskRecord) -> R) -> R {
        let out = {
            let mut r = self.record.lock().unwrap_or_else(|e| e.into_inner());
            let out = f(&mut r);
            r.updated_at = super::super::now();
            out
        };
        self.poke();
        out
    }

    pub(crate) fn poke(&self) {
        self.tick.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub(crate) fn watch(&self) -> tokio::sync::watch::Receiver<u64> {
        self.tick.subscribe()
    }

    pub(crate) fn note_turn_started(&self) {
        self.turns_started.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn turns_started(&self) -> u64 {
        self.turns_started.load(Ordering::SeqCst)
    }

    /// Waits until it ended, or `secs` went by; answers its record.
    pub async fn wait_end(&self, secs: u64) -> TaskRecord {
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

// ─── the store ─────────────────────────────────────────────────────────────────

/// Every task this computer keeps (`WorkState::tasks`).
pub struct Tasks {
    dir: Option<PathBuf>,
    all: Mutex<HashMap<String, Arc<Task>>>,
    /// Incoming chunks are written one at a time.
    pub(crate) chunks: tokio::sync::Mutex<()>,
    /// Task sessions start `claude` for real (the app); else (tests) a
    /// spawn is only recorded.
    live: std::sync::atomic::AtomicBool,
    spawns: Mutex<Vec<Value>>,
}

impl std::fmt::Debug for Tasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tasks").finish_non_exhaustive()
    }
}

impl Tasks {
    /// The tasks kept under `<work dir>/tasks` (7 days after they ended).
    pub fn load(work_dir: Option<&Path>) -> Self {
        let dir = work_dir.map(|d| d.join(DIR));
        let mut all = HashMap::new();
        if let Some(d) = &dir {
            for r in load_records(d) {
                all.insert(r.id.clone(), Task::new(r));
            }
        }
        Self {
            dir,
            all: Mutex::new(all),
            chunks: tokio::sync::Mutex::new(()),
            live: std::sync::atomic::AtomicBool::new(false),
            spawns: Mutex::new(Vec::new()),
        }
    }

    /// A task's own folder: its record, what arrives, its worktree.
    pub fn task_dir(&self, id: &str) -> Option<PathBuf> {
        is_task_id(id)
            .then(|| self.dir.as_ref().map(|d| d.join(id)))
            .flatten()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Task>> {
        self.all
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Every task, newest first.
    pub fn all(&self) -> Vec<Arc<Task>> {
        let mut out: Vec<(i64, Arc<Task>)> = self
            .all
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|t| (t.peek(|r| r.created_at), t.clone()))
            .collect();
        out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        out.into_iter().map(|(_, t)| t).collect()
    }

    /// The task whose session there is `session` (this computer runs it).
    pub fn of_session(&self, session: &str) -> Option<Arc<Task>> {
        self.all
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .find(|t| {
                t.peek(|r| {
                    r.direction == Direction::In && r.task_session.as_deref() == Some(session)
                })
            })
            .cloned()
    }

    /// Adds a task (refused when its id is taken).
    pub(crate) fn insert(&self, record: TaskRecord) -> Option<Arc<Task>> {
        let mut all = self.all.lock().unwrap_or_else(|e| e.into_inner());
        if all.contains_key(&record.id) {
            return None;
        }
        let t = Task::new(record);
        all.insert(t.id.clone(), t.clone());
        Some(t)
    }

    pub(crate) fn forget(&self, id: &str) {
        self.all
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// Writes a task's record (0600).
    pub(crate) fn save(&self, task: &Task) {
        let Some(dir) = self.task_dir(&task.id) else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(task = %task.id, error = %e, "a task's folder could not be made");
            return;
        }
        crate::claude::transfer::private_dir(&dir);
        let record = task.record();
        if let Err(e) = crate::persist::write_json_private(&dir.join(RECORD), &record) {
            tracing::warn!(task = %task.id, error = %e, "a task's record could not be saved");
        }
    }

    /// The app turns this on: a task another computer hands over starts a
    /// Claude Code session here. Until then (tests) the spawn is recorded.
    pub fn enable_sessions(&self) {
        self.live.store(true, Ordering::Relaxed);
    }

    pub(crate) fn sessions_live(&self) -> bool {
        self.live.load(Ordering::Relaxed)
    }

    pub(crate) fn record_spawn(&self, v: Value) {
        self.spawns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(v);
    }

    pub fn recorded_spawns(&self) -> Vec<Value> {
        self.spawns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

fn load_records(dir: &Path) -> Vec<TaskRecord> {
    let now = super::super::now();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_task_id(&name) {
            continue;
        }
        let record: Option<TaskRecord> = crate::persist::read_json(&e.path().join(RECORD));
        let Some(mut r) = record.filter(|r| r.id == name) else {
            // A folder without its record: what is left of a removal.
            continue;
        };
        if r.direction == Direction::In && !r.state.ended() {
            let now_ended = now;
            match r.state {
                TaskState::Sending | TaskState::Starting => {
                    r.state = TaskState::Error;
                    r.error = Some(
                        "Mother Claude stopped on this computer before the task's session started."
                            .into(),
                    );
                }
                _ => {
                    // Its session ended with the app; what it did is taken
                    // once the app runs again (`start`).
                    r.state = TaskState::Finishing;
                    r.error = Some(
                        "Mother Claude stopped on this computer while the task's session worked. \
                         Continue its session to go on."
                            .into(),
                    );
                }
            }
            r.ended_at = Some(now_ended);
            let _ = crate::persist::write_json_private(&e.path().join(RECORD), &r);
        }
        out.push(r);
    }
    out
}

// ─── shared ────────────────────────────────────────────────────────────────────

/// Tasks changed: dashboards refetch `/api/machines/tasks`.
pub fn publish(state: &AppState) {
    crate::machines::publish(state, "tasks");
}

/// A task's audit line, on the computer that writes it: its id, the other
/// computer, the folder, the SHA-256 of its first line — never its text.
pub fn audit(state: &AppState, action: &str, r: &TaskRecord) {
    state.devices.audit.record(AuditEntry {
        action: action.to_string(),
        device_id: Some(r.machine.clone()),
        device_name: Some(crate::machines::peer_name(state, &r.machine)),
        detail: Some(json!({
            "task": r.id,
            "direction": r.direction,
            "session": r.session,
            "taskSession": r.task_session,
            "folder": if r.folder.is_empty() { &r.checkout } else { &r.folder },
            "firstLineSha256": r.first_line_sha256,
            "permissionMode": r.permission_mode,
            "dangerous": r.dangerous,
            "state": r.state,
            "turns": r.turns,
            "results": r.results,
            "branch": r.branch.as_deref().map(crate::machines::code::short_branch),
            "commits": r.commits,
            "files": r.files,
        })),
        ..AuditEntry::now()
    });
}

/// Whether the task's session waits for an answer here.
pub(crate) fn asking(state: &AppState, r: &TaskRecord) -> bool {
    r.direction == Direction::In
        && r.task_session
            .as_deref()
            .is_some_and(|s| !state.pending.list(s).is_empty())
}

/// A task as a dashboard shows it (its state as shown).
fn shown_summary(state: &AppState, t: &Task) -> Value {
    let r = t.record();
    let mut v = r.summary();
    v["state"] = json!(r.state.shown(asking(state, &r)));
    v
}

/// `GET /api/machines/tasks`: both ways, newest first.
pub fn view(state: &AppState) -> Value {
    prune(state);
    let tasks: Vec<Value> = state
        .machines
        .work
        .tasks
        .all()
        .iter()
        .take(MAX_KEPT)
        .map(|t| shown_summary(state, t))
        .collect();
    json!({ "tasks": tasks })
}

/// One task whole (`GET /api/machines/tasks/{id}`).
pub fn one(state: &AppState, t: &Task) -> Value {
    let r = t.record();
    let mut v = r.full();
    v["state"] = json!(r.state.shown(asking(state, &r)));
    v
}

/// The session's tasks as background tasks (its page's *Background
/// tasks*).
pub fn session_tasks(state: &AppState, session: &str) -> Vec<BackgroundTask> {
    state
        .machines
        .work
        .tasks
        .all()
        .into_iter()
        .filter_map(|t| {
            t.peek(|r| {
                (r.direction == Direction::Out && r.session.as_deref() == Some(session))
                    .then(|| r.background_task())
            })
        })
        .take(50)
        .collect()
}

/// A session's task, by its id: what its task output and stop routes use.
pub fn session_task(state: &AppState, session: &str, id: &str) -> Option<Arc<Task>> {
    if !is_task_id(id) {
        return None;
    }
    state.machines.work.tasks.get(id).filter(|t| {
        t.peek(|r| r.direction == Direction::Out && r.session.as_deref() == Some(session))
    })
}

/// Forgets the tasks that ended more than 7 days ago — their worktrees
/// and records there, their records and refs here (never a branch) — and
/// the oldest past [`MAX_KEPT`].
pub fn prune(state: &AppState) {
    let now = super::super::now();
    let all = state.machines.work.tasks.all();
    for (i, t) in all.iter().enumerate() {
        let (ended, at) = t.peek(|r| (r.state.ended(), r.ended_at));
        let old = ended && at.is_some_and(|a| now - a > KEEP_MS);
        if old || (i >= MAX_KEPT && ended) {
            let st = state.clone();
            let task = t.clone();
            state.machines.work.tasks.forget(&t.id);
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move { clear(&st, &task).await });
            }
        }
    }
}

/// Takes a task away on this computer: its session stopped, its worktree
/// and refs removed (there); its refs removed, never its branch (here);
/// its folder and record gone.
pub(crate) async fn clear(state: &AppState, t: &Task) {
    let r = t.record();
    let git = state.machines.moves.git();
    match r.direction {
        Direction::In => {
            if let Some(s) = r.task_session.as_deref() {
                let _ = state
                    .control
                    .stop_and_wait(s, std::time::Duration::from_secs(10))
                    .await;
            }
            let (g, checkout, folder, id) = (
                git,
                PathBuf::from(&r.checkout),
                PathBuf::from(&r.folder),
                r.id.clone(),
            );
            let done = tokio::task::spawn_blocking(move || {
                crate::machines::code::remove_task_tree(&g, &checkout, &folder, &id)
            })
            .await;
            if let Ok(Err(e)) = done {
                tracing::warn!(task = %r.id, error = %e, "a task's worktree could not be removed");
            }
        }
        Direction::Out => {
            let (g, top, id) = (git, PathBuf::from(&r.project), r.id.clone());
            let _ = tokio::task::spawn_blocking(move || {
                crate::machines::code::drop_task_refs(&g, &top, &id)
            })
            .await;
        }
    }
    if let Some(dir) = state.machines.work.tasks.task_dir(&r.id) {
        let _ = std::fs::remove_dir_all(dir);
    }
    publish(state);
}

// ─── the session's hooks (the computer that runs it) ───────────────────────────

/// How a task session's turn ended, as its stream says (`result`).
#[derive(Clone, Debug, Default)]
pub struct TurnEnd {
    /// Its last message (the stream's `result` text).
    pub summary: Option<String>,
    /// A message is queued behind it: the next turn starts at once.
    pub queued: bool,
    /// Background work it started runs on (it may resume by itself).
    pub resumes: bool,
    /// The turn ended in an error (said here).
    pub error: Option<String>,
}

/// A task session's turn ended (the control stream's `result`, after its
/// own bookkeeping): with no approval waiting and nothing queued the task
/// is done — what it did is taken, and the other computer brings it back.
/// Cheap for every other session.
pub fn turn_ended(state: &AppState, session: &str, end: TurnEnd) {
    let Some(task) = state.machines.work.tasks.of_session(session) else {
        return;
    };
    let asking = !state.pending.list(session).is_empty();
    let s = task.state();
    if s.is_final() {
        return;
    }
    task.update(|r| {
        r.turns = r.turns.saturating_add(1);
        r.summary = end
            .summary
            .as_deref()
            .map(|s| clean_text(s.trim(), MAX_SUMMARY))
            .filter(|s| !s.is_empty())
            .or(r.summary.take());
        r.background = end.resumes;
        r.error = end.error.as_deref().map(|e| clip(e, 2000));
    });
    match on_turn_end(s, end.queued, asking) {
        Some(TaskState::Finishing) => {
            task.update(|r| r.state = TaskState::Finishing);
            target::collect_soon(state, task, TaskState::Done);
        }
        Some(next) => {
            task.update(|r| r.state = next);
            state.machines.work.tasks.save(&task);
            publish(state);
        }
        None => {
            state.machines.work.tasks.save(&task);
            publish(state);
        }
    }
}

/// A message reached a task session (a person's, from either computer):
/// a done task runs again. Cheap for every other session.
pub fn turn_started(state: &AppState, session: &str) {
    let Some(task) = state.machines.work.tasks.of_session(session) else {
        return;
    };
    task.note_turn_started();
    let s = task.state();
    if let Some(next) = on_turn_start(s) {
        task.update(|r| {
            r.state = next;
            r.ended_at = None;
        });
        state.machines.work.tasks.save(&task);
        publish(state);
    }
}

/// A task session's process ended: a task in a turn stops, and what it did
/// is taken. Cheap for every other session.
pub fn session_exited(state: &AppState, session: &str) {
    let Some(task) = state.machines.work.tasks.of_session(session) else {
        return;
    };
    if let Some(then) = on_exit(task.state()) {
        task.update(|r| {
            r.state = TaskState::Finishing;
            if r.error.is_none() {
                r.error = Some("Its session ended before its turn did.".into());
            }
        });
        target::collect_soon(state, task, then);
    }
}

/// At app start: tasks handed over that had not ended are followed again;
/// tasks this computer ran that Mother Claude's stop cut off have their
/// work taken.
pub fn start(state: &AppState) {
    for t in state.machines.work.tasks.all() {
        let (dir, s, gone) = t.peek(|r| (r.direction, r.state, r.gone_there));
        match dir {
            Direction::Out if !s.is_final() && !gone => source::follow(state, t),
            Direction::In if s == TaskState::Finishing => {
                target::collect_soon(state, t, TaskState::Stopped)
            }
            _ => {}
        }
    }
}

/// Cancels what `machine` asked of this computer (all computers' with
/// `None`): Machines was turned off, the computer revoked, or its `run`
/// grant taken back. A done task's session is left as it is.
pub fn stop_from(state: &AppState, machine: Option<&str>, why: &str) {
    for t in state.machines.work.tasks.all() {
        let hit = t.peek(|r| {
            r.direction == Direction::In
                && !r.state.ended()
                && machine.is_none_or(|m| m == r.machine)
        });
        if hit {
            cancel_soon(state, t, why);
        }
    }
}

/// After `machine`'s grants changed here: each of its running tasks is
/// cancelled when the `run` grant's column that let it in is gone.
pub fn grants_changed(state: &AppState, machine: &str) {
    use crate::machines::grants::Grant;
    for t in state.machines.work.tasks.all() {
        let (hit, lan) = t.peek(|r| {
            (
                r.direction == Direction::In && !r.state.ended() && r.machine == machine,
                r.via_lan,
            )
        });
        if !hit {
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
            cancel_soon(state, t, "This computer took back the Run permission.");
        }
    }
}

fn cancel_soon(state: &AppState, t: Arc<Task>, why: &str) {
    tracing::info!(task = %t.id, why, "a task from another computer is cancelled");
    let (st, why) = (state.clone(), why.to_string());
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move {
            target::cancel_here(&st, &t, Some(why)).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TaskState::*;

    #[test]
    fn ids_have_their_shape_and_are_told_from_runs() {
        let id = new_task_id();
        assert!(is_task_id(&id), "{id}");
        assert!(!super::super::is_run_id(&id));
        assert!(!is_task_id(&super::super::new_run_id()));
        assert!(!is_task_id("task_0123"));
        assert!(!is_task_id("task_0123456789ABCDEF"));
        assert_eq!(
            first_line("\n\n  Fix the Windows build\nthen run the tests"),
            "Fix the Windows build"
        );
        assert_eq!(first_line(""), "");
        assert_eq!(first_line(&"x".repeat(300)).chars().count(), 200);
        assert_eq!(clean_text("a\u{1b}[31mb\nc\td", 100), "a[31mb\nc\td");
        assert_eq!(clean_text("héllo", 2), "h", "never half a character");
    }

    #[test]
    fn a_task_runs_waits_ends_runs_again_and_is_cancelled() {
        // It runs; its turn ends with nothing waiting: its work is taken,
        // then it is done.
        assert_eq!(on_turn_end(Running, false, false), Some(Finishing));
        assert_eq!(after_result(Done, false), Done);
        // A turn that ends asking for an approval, or with a message queued,
        // goes on — shown as waiting while the approval waits.
        assert_eq!(on_turn_end(Running, false, true), None);
        assert_eq!(on_turn_end(Running, true, false), None);
        assert_eq!(Running.shown(true), Waiting);
        assert_eq!(Running.shown(false), Running);
        assert_eq!(Done.shown(true), Done);
        // A message makes a done task run again; a later end is taken anew.
        assert_eq!(on_turn_start(Done), Some(Running));
        assert_eq!(on_turn_start(Stopped), Some(Running));
        assert_eq!(on_turn_start(Running), None);
        assert_eq!(on_turn_end(Done, false, false), Some(Finishing));
        assert_eq!(on_turn_end(Done, true, false), Some(Running));
        // A turn that started while its work was being taken: running.
        assert_eq!(after_result(Done, true), Running);
        // Its process ends mid-turn: stopped, once its work is taken; an
        // idle one that ends stays done.
        assert_eq!(on_exit(Running), Some(Stopped));
        assert_eq!(on_exit(Waiting), Some(Stopped));
        assert_eq!(on_exit(Done), None);
        assert_eq!(after_result(Stopped, true), Stopped);
        // Cancelled: nothing moves it any more.
        assert_eq!(after_result(Cancelled, true), Cancelled);
        assert_eq!(on_turn_start(Cancelled), None);
        assert_eq!(on_turn_end(Cancelled, false, false), None);
        assert_eq!(on_exit(Cancelled), None);
        for s in [Cancelled, Error, Lost] {
            assert!(s.ended() && s.is_final(), "{s:?}");
        }
        for s in [Done, Stopped] {
            assert!(s.ended() && !s.is_final(), "{s:?}");
        }
        for s in [Sending, Starting, Running, Waiting, Finishing] {
            assert!(!s.ended(), "{s:?}");
        }
    }

    #[test]
    fn the_asking_computer_waits_for_the_work_before_it_says_done() {
        assert_eq!(mirrored(Done, 1, 0), Finishing);
        assert_eq!(mirrored(Done, 1, 1), Done);
        assert_eq!(mirrored(Done, 0, 0), Done, "nothing changed: done at once");
        assert_eq!(mirrored(Cancelled, 2, 1), Finishing);
        assert_eq!(mirrored(Running, 2, 1), Running);
    }

    fn record() -> TaskRecord {
        TaskRecord {
            id: "task_0123456789abcdef".into(),
            direction: Direction::Out,
            machine: "0123456789abcdef0123456789abcdef".into(),
            machine_name: "DESKTOP-PC".into(),
            os: "windows".into(),
            session: Some("a0ee0000-1111-4222-8333-444455556666".into()),
            session_title: None,
            project: "/Users/me/app".into(),
            project_name: "app".into(),
            cwd: String::new(),
            checkout: r"D:\dev\app".into(),
            folder: r"C:\data\machines\work\tasks\task_0123456789abcdef\app".into(),
            task: "Fix the Windows build\nThen run the tests.".into(),
            first_line: "Fix the Windows build".into(),
            first_line_sha256: crate::claude::secrets::sha256_hex(b"Fix the Windows build"),
            permission_mode: "default".into(),
            model: None,
            effort: None,
            account_name: Some("Default".into()),
            task_session: Some("b0ee0000-1111-4222-8333-444455556666".into()),
            state: Done,
            error: None,
            created_at: 1,
            started_at: Some(2),
            ended_at: Some(3),
            turns: 1,
            summary: Some("Fixed.".into()),
            background: false,
            base: Some("a".repeat(40)),
            base_tree: Some("b".repeat(40)),
            branch: Some("refs/heads/mother-claude/DESKTOP-PC/task_0123456789abcdef".into()),
            tip: Some("c".repeat(40)),
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
            updated_at: 3,
            account: Some("default".into()),
            haves: vec!["d".repeat(40)],
            last: None,
            result_files: Vec::new(),
            their_state: None,
        }
    }

    #[test]
    fn views_leave_out_what_is_kept_and_name_the_branch() {
        let r = record();
        let v = r.full();
        assert_eq!(
            v["branchName"],
            "mother-claude/DESKTOP-PC/task_0123456789abcdef"
        );
        for k in INTERNAL {
            assert!(v.get(k).is_none(), "{k}");
        }
        let t = r.background_task();
        assert_eq!(t.kind, BackgroundTaskKind::Machine);
        assert_eq!(t.status, "completed");
        assert_eq!(t.label, "Task: Fix the Windows build · DESKTOP-PC");
    }
}
