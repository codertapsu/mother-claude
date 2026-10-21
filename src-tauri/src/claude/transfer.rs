//! Moving and copying sessions and projects between the config directories of
//! two accounts (docs/ACCOUNTS.md §5.9, D4).
//!
//! One of the explicit desktop actions that write under a Claude directory.
//! Everything is checked before anything is touched, each refusal with its own
//! code; nothing runs in the background on its own.
//!
//! **What moves with a session** (§1.4): `projects/<enc>/<id>.jsonl`,
//! `projects/<enc>/<id>/` (subagents, workflow journals, tool results),
//! `file-history/<id>/`, `session-env/<id>`, `todos/<id>*.json`. Never a
//! pidfile. Files are copied to a temporary name beside their destination,
//! fsynced, given the source's mtime, renamed into place and verified (size and
//! SHA-256 against the source).
//!
//! **Shared files** are written only when they must be, and then only while no
//! live process uses the directory (a pidfile with a living pid in its
//! `sessions/`, or a `.claude.json.lock` younger than 10 s): `.claude.json`
//! gets the project's entry only with the trust opt-in (the entry *is* the
//! trust: `hasTrustDialogAccepted`, allowed tools, MCP choices — a trusted
//! folder runs its hooks and MCP servers unasked) and never overwrites one;
//! `history.jsonl` gets a project's prompt lines (de-duplicated) when a project
//! moves. A copy of a session without the opt-in touches only new files, so it
//! runs while the target account has live sessions.
//!
//! **Undo** (seven days): `<app data dir>/undo/<job id>/` holds a manifest of
//! every file created (with its hash) and, in move mode, the source files
//! themselves (renamed there, so a move costs no extra space on one volume),
//! plus a copy of every shared file as it was before the job changed it
//! (`shared/`, for recovery by hand: undo itself takes back exactly the
//! `.claude.json` entry and the `history.jsonl` lines, so whatever Claude Code
//! wrote since stays). Undo refuses while a session of the job runs or, when it
//! changes shared files, while their directory is in use; it puts the moved
//! files back first and only then deletes what was created — refusing if any of
//! it changed since — and a retry after a failure finishes what is left.
//!
//! **A failed run** puts the source back the same way and removes what it made;
//! when even that fails, the undo directory and its manifest stay (the job says
//! so, and Undo finishes it): a moved file is never deleted.

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::home::{encode_cwd, ClaudeHome};

/// A `.claude.json.lock` younger than this means the CLI is writing it.
const LOCK_FRESH: Duration = Duration::from_secs(10);
/// How long an undo is kept.
pub const UNDO_KEEP: Duration = Duration::from_secs(7 * 24 * 3600);

/// What to move or copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "what")]
pub enum What {
    Session { id: String },
    Project { cwd: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Move,
    Copy,
}

/// Why a transfer was refused. Every precondition has its own code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    #[error("both accounts use the same directory: there is nothing to move")]
    SameDirectory,
    #[error("{0}")]
    NotFound(String),
    #[error("session {0} is running; stop it first")]
    SessionLive(String),
    #[error("the target account already has session {0}")]
    AlreadyExists(String),
    #[error("a Claude Code process is using {0}; quit its sessions first")]
    DirectoryInUse(String),
    #[error("Claude Code is writing {0} right now; try again in a few seconds")]
    ConfigLocked(String),
    #[error("{0}")]
    Invalid(String),
    #[error("a copied file did not match its source: {0}")]
    VerifyFailed(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    UndoConflict(String),
    #[error("session {0} changed while it was being moved; nothing was moved, try again")]
    SessionChanged(String),
    #[error("a backup, restore or transfer is running; try again when it has finished")]
    Busy,
    #[error("{0}")]
    Incomplete(String),
    /// A backup set's refusal (a move into or out of a storage, its undo),
    /// with the backup error's own code.
    #[error("{message}")]
    Backup { code: &'static str, message: String },
}

impl TransferError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Backup { code, .. } => code,
            Self::SameDirectory => "same_directory",
            Self::NotFound(_) => "not_found",
            Self::SessionLive(_) => "session_live",
            Self::AlreadyExists(_) => "already_exists",
            Self::DirectoryInUse(_) => "directory_in_use",
            Self::ConfigLocked(_) => "config_locked",
            Self::Invalid(_) => "invalid_transfer",
            Self::VerifyFailed(_) => "verify_failed",
            Self::Io(_) => "transfer_io",
            Self::UndoConflict(_) => "undo_conflict",
            Self::SessionChanged(_) => "session_changed",
            Self::Busy => "data_job_running",
            Self::Incomplete(_) => "transfer_incomplete",
        }
    }
}

fn io(what: &str, path: &Path, e: std::io::Error) -> TransferError {
    TransferError::Io(format!("{what} {}: {e}", path.display()))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ─────────────────────────────── the plan ────────────────────────────────

/// One file to copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCopy {
    pub from: PathBuf,
    pub to: PathBuf,
    pub size: u64,
}

/// One session's part of a transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlan {
    pub id: String,
    pub cwd: Option<String>,
    pub files: Vec<FileCopy>,
    /// Directories that hold only this session (removed from the source in
    /// move mode, created in the target).
    #[serde(skip)]
    pub dirs: Vec<(PathBuf, PathBuf)>,
}

/// The dry run: everything a transfer would write, and nothing written yet.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub what: What,
    pub mode: Mode,
    pub sessions: Vec<SessionPlan>,
    pub files: usize,
    pub bytes: u64,
    /// The project entry added to the target's `.claude.json` (trust opt-in).
    pub adds_project_entry: Option<String>,
    /// The project entry removed from the source's (a project move that
    /// carries it to the target).
    pub removes_project_entry: Option<String>,
    /// Prompt-history lines appended to the target (a project).
    pub history_lines: usize,
    /// The target's `.claude.json` or `history.jsonl` is written.
    pub writes_shared: bool,
    #[serde(skip)]
    from: PathBuf,
    #[serde(skip)]
    to: PathBuf,
    /// The two `.claude.json` files (the Default's is `~/.claude.json`,
    /// beside its directory, unless `CLAUDE_CONFIG_DIR` is set).
    #[serde(skip)]
    from_config: PathBuf,
    #[serde(skip)]
    to_config: PathBuf,
    /// `(cwd, entry)` added to the target's `.claude.json` (trust opt-in).
    #[serde(skip)]
    entries: Vec<(String, Value)>,
    /// Projects whose entry leaves the source's `.claude.json` (a move that
    /// carries it).
    #[serde(skip)]
    removes: Vec<String>,
    #[serde(skip)]
    history: Vec<String>,
    /// Projects whose prompt-history lines leave the source (a move of every
    /// conversation of them).
    #[serde(skip)]
    history_cwds: Vec<String>,
    /// The projects of a bulk request that go (some of their conversations
    /// at least).
    #[serde(skip)]
    pub projects: Vec<String>,
    /// What a bulk request should know before it runs.
    #[serde(skip)]
    pub warnings: Vec<String>,
    /// The customizations it copies.
    #[serde(skip)]
    pub customs: Option<custom::CustomPlan>,
    #[serde(skip)]
    snapshot: bool,
    #[serde(skip)]
    refresh: bool,
    /// The two accounts, when the app planned it (the job names them).
    #[serde(skip)]
    pub(crate) from_account: Option<String>,
    #[serde(skip)]
    pub(crate) to_account: Option<String>,
}

/// What the engine needs to know about the world: the two directories and
/// whether a session runs somewhere (any directory's pidfile, an owned handle,
/// a bridge conversation).
pub struct Context<'a> {
    pub from: &'a ClaudeHome,
    pub to: &'a ClaudeHome,
    pub is_live: &'a dyn Fn(&str) -> bool,
}

/// A request, as the routes take it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    #[serde(flatten)]
    pub what: What,
    pub mode: Mode,
    /// Copy the project's trust (and its entry) into the target too.
    #[serde(default)]
    pub trust: bool,
    /// A copy of what is there now, even while the session runs ("continue
    /// on", which forks from this point): an unfinished last line of a
    /// transcript is left out and each file is verified against the
    /// source's first bytes. Never from a request body.
    #[serde(skip)]
    pub snapshot: bool,
    /// With `snapshot`: a copy an earlier snapshot left in the target is
    /// brought up to date when nothing changed it there since (it is the
    /// start of the source now); one that changed there is left as it is
    /// (`already_exists`, so it is forked as it stands). Never from a body.
    #[serde(skip)]
    pub refresh: bool,
}

/// The transcripts a session id has under `projects/*/`.
pub(crate) fn transcripts_of(home: &ClaudeHome, id: &str) -> Vec<PathBuf> {
    let name = format!("{id}.jsonl");
    let mut out: Vec<PathBuf> = std::fs::read_dir(home.projects_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|d| d.file_type().is_ok_and(|t| t.is_dir()))
        .map(|d| d.path().join(&name))
        .filter(|p| is_plain_file(p))
        .collect();
    out.sort();
    out
}

/// A regular file, not through a symlink.
pub(crate) fn is_plain_file(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
}

/// A directory, not through a symlink.
pub(crate) fn is_plain_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

/// The cwd a transcript records first.
pub fn recorded_cwd(transcript: &Path) -> Option<String> {
    let file = std::fs::File::open(transcript).ok()?;
    std::io::BufReader::new(file)
        .lines()
        .take(200)
        .map_while(Result::ok)
        .find_map(|line| {
            let v: Value = serde_json::from_str(&line).ok()?;
            v.get("cwd")?.as_str().map(str::to_string)
        })
}

/// Every regular file under `dir` (symlinks are never followed).
pub(crate) fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let ft = e.file_type()?;
        if ft.is_dir() {
            walk(&e.path(), out)?;
        } else if ft.is_file() {
            out.push(e.path());
        }
    }
    Ok(())
}

/// The files of one session in `from`, mapped into `to`.
pub(crate) fn plan_session(
    from: &ClaudeHome,
    to: &ClaudeHome,
    id: &str,
) -> Result<SessionPlan, TransferError> {
    if !super::ids::is_session_id(id) {
        return Err(TransferError::Invalid(format!(
            "{id:?} is not a session id"
        )));
    }
    let transcripts = transcripts_of(from, id);
    if transcripts.is_empty() {
        return Err(TransferError::NotFound(format!(
            "no transcript of session {id} in {}",
            from.base().display()
        )));
    }
    let map = |p: &Path| -> Option<PathBuf> {
        p.strip_prefix(from.base())
            .ok()
            .map(|rel| to.base().join(rel))
    };
    let mut files: Vec<FileCopy> = Vec::new();
    let mut dirs: Vec<(PathBuf, PathBuf)> = Vec::new();
    let add = |p: &Path, files: &mut Vec<FileCopy>| -> Result<(), TransferError> {
        let meta = std::fs::symlink_metadata(p).map_err(|e| io("reading", p, e))?;
        if meta.is_file() {
            if let Some(dest) = map(p) {
                files.push(FileCopy {
                    from: p.to_path_buf(),
                    to: dest,
                    size: meta.len(),
                });
            }
        }
        Ok(())
    };
    let add_dir = |d: &Path,
                   files: &mut Vec<FileCopy>,
                   dirs: &mut Vec<(PathBuf, PathBuf)>|
     -> Result<(), TransferError> {
        // A symlink planted in a session's place is never followed.
        if !is_plain_dir(d) {
            return Ok(());
        }
        if let Some(dest) = map(d) {
            dirs.push((d.to_path_buf(), dest));
        }
        let mut found = Vec::new();
        walk(d, &mut found).map_err(|e| io("listing", d, e))?;
        for f in found {
            add(&f, files)?;
        }
        Ok(())
    };
    for t in &transcripts {
        add(t, &mut files)?;
        if let Some(parent) = t.parent() {
            add_dir(&parent.join(id), &mut files, &mut dirs)?;
        }
    }
    add_dir(&from.file_history_dir().join(id), &mut files, &mut dirs)?;
    let env = from.session_env_dir().join(id);
    if is_plain_dir(&env) {
        add_dir(&env, &mut files, &mut dirs)?;
    } else if is_plain_file(&env) {
        add(&env, &mut files)?;
    }
    for e in std::fs::read_dir(from.todos_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(id) && name.ends_with(".json") {
            add(&e.path(), &mut files)?;
        }
    }
    Ok(SessionPlan {
        id: id.to_string(),
        cwd: recorded_cwd(&transcripts[0]),
        files,
        dirs,
    })
}

/// A project's entry in a `.claude.json` (`projects[<cwd>]`).
pub(crate) fn project_entry(claude_json: &Path, cwd: &str) -> Option<Value> {
    let text = std::fs::read_to_string(claude_json).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("projects")?.get(cwd).cloned()
}

/// `history.jsonl` lines whose `project` is `cwd`.
pub(crate) fn history_lines(history: &Path, cwd: &str) -> Vec<String> {
    let Ok(file) = std::fs::File::open(history) else {
        return Vec::new();
    };
    // Line by line as bytes: a torn character skips its line, not the rest.
    std::io::BufReader::new(file)
        .split(b'\n')
        .map_while(Result::ok)
        .filter_map(|line| String::from_utf8(line).ok())
        .map(|line| line.trim_end_matches('\r').to_string())
        .filter(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|v| v.get("project").and_then(Value::as_str).map(|p| p == cwd))
                .unwrap_or(false)
        })
        .collect()
}

/// The `(timestamp, display)` a history line is de-duplicated by.
fn history_key(line: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(line).ok()?;
    Some((
        v.get("timestamp").map(Value::to_string).unwrap_or_default(),
        v.get("display")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    ))
}

/// Whether a live Claude Code process uses `home`: a pidfile with a living
/// pid, or a `.claude.json.lock` written in the last 10 s.
pub fn in_use(home: &ClaudeHome) -> Option<TransferError> {
    in_use_at(home.base(), &home.claude_json())
}

/// [`in_use`] for a directory and the `.claude.json` that goes with it (the
/// Default's is beside its directory).
fn in_use_at(base: &Path, claude_json: &Path) -> Option<TransferError> {
    for e in std::fs::read_dir(ClaudeHome::for_account_dir(base).sessions_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_suffix(".json")
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if super::live::pid_exists(pid) {
            return Some(TransferError::DirectoryInUse(base.display().to_string()));
        }
    }
    let lock = claude_json.with_extension("json.lock");
    let fresh = std::fs::metadata(&lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < LOCK_FRESH);
    fresh.then(|| TransferError::ConfigLocked(claude_json.display().to_string()))
}

/// Check every precondition and work out what would be written. Nothing is
/// touched. Blocking.
pub fn plan(ctx: &Context<'_>, req: &Request) -> Result<Plan, TransferError> {
    let same = |a: &Path, b: &Path| {
        let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
        let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
        a == b
    };
    if same(ctx.from.base(), ctx.to.base()) {
        return Err(TransferError::SameDirectory);
    }
    let (ids, cwd): (Vec<String>, Option<String>) = match &req.what {
        What::Session { id } => (vec![id.clone()], None),
        What::Project { cwd } => {
            let cwd = cwd.trim_end_matches('/').to_string();
            if !Path::new(&cwd).is_absolute() {
                return Err(TransferError::Invalid(
                    "a project is named by its absolute directory".into(),
                ));
            }
            let dir = ctx.from.transcript_dir(&cwd);
            let mut ids: Vec<String> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let id = name.strip_suffix(".jsonl")?.to_string();
                    // A look-alike folder encodes the same: only sessions
                    // that recorded exactly this cwd.
                    (super::ids::is_session_id(&id)
                        && recorded_cwd(&e.path()).as_deref() == Some(cwd.as_str()))
                    .then_some(id)
                })
                .collect();
            ids.sort();
            if ids.is_empty() {
                return Err(TransferError::NotFound(format!(
                    "no sessions of {cwd} in {}",
                    ctx.from.base().display()
                )));
            }
            (ids, Some(cwd))
        }
    };
    if req.snapshot && (req.mode != Mode::Copy || cwd.is_some()) {
        return Err(TransferError::Invalid(
            "a snapshot is a copy of one session".into(),
        ));
    }
    if req.refresh && !req.snapshot {
        return Err(TransferError::Invalid(
            "only a snapshot brings an earlier copy up to date".into(),
        ));
    }
    let mut sessions = Vec::new();
    for id in &ids {
        if !req.snapshot && (ctx.is_live)(id) {
            return Err(TransferError::SessionLive(id.clone()));
        }
        if !transcripts_of(ctx.to, id).is_empty() {
            if !req.refresh {
                return Err(TransferError::AlreadyExists(id.clone()));
            }
            // An earlier snapshot is brought up to date only when it is
            // behind the source and nothing changed it in the target.
            let planned = plan_session(ctx.from, ctx.to, id)?;
            let behind = planned
                .files
                .first()
                .is_some_and(|t| matches!(copy_state(&t.to, &t.from), Ok(CopyState::Behind)));
            if !behind {
                return Err(TransferError::AlreadyExists(id.clone()));
            }
            sessions.push(planned);
            continue;
        }
        sessions.push(plan_session(ctx.from, ctx.to, id)?);
    }
    let project_cwd = cwd
        .clone()
        .or_else(|| sessions.first().and_then(|s| s.cwd.clone()));
    // The project's entry, with the trust opt-in only, and never over one.
    let entry = match (&project_cwd, req.trust) {
        (Some(cwd), true) if project_entry(&ctx.to.claude_json(), cwd).is_none() => {
            project_entry(&ctx.from.claude_json(), cwd).map(|e| (cwd.clone(), e))
        }
        _ => None,
    };
    let history: Vec<String> = match &cwd {
        Some(cwd) => {
            let have: HashSet<(String, String)> = history_lines(&ctx.to.history_file(), cwd)
                .iter()
                .filter_map(|l| history_key(l))
                .collect();
            history_lines(&ctx.from.history_file(), cwd)
                .into_iter()
                .filter(|l| history_key(l).is_some_and(|k| !have.contains(&k)))
                .collect()
        }
        None => Vec::new(),
    };
    // A move takes the source's entry along only where it is carried to (the
    // trust opt-in, a target without one): it holds the project's MCP servers
    // and allowed tools, so without the opt-in it stays where it is.
    let removes = (req.mode == Mode::Move && entry.is_some())
        .then_some(cwd.clone())
        .flatten();
    let history_cwds: Vec<String> = cwd
        .iter()
        .filter(|_| req.mode == Mode::Move)
        .cloned()
        .collect();
    let writes_shared = entry.is_some() || !history.is_empty();
    if writes_shared {
        if let Some(e) = in_use(ctx.to) {
            return Err(e);
        }
    }
    if req.mode == Mode::Move && cwd.is_some() {
        if let Some(e) = in_use(ctx.from) {
            return Err(e);
        }
    }
    let files = sessions.iter().map(|s| s.files.len()).sum();
    let bytes = sessions
        .iter()
        .flat_map(|s| s.files.iter())
        .map(|f| f.size)
        .sum();
    Ok(Plan {
        what: req.what.clone(),
        mode: req.mode,
        sessions,
        files,
        bytes,
        adds_project_entry: entry.as_ref().map(|(c, _)| c.clone()),
        removes_project_entry: removes.clone(),
        history_lines: history.len(),
        writes_shared,
        from: ctx.from.base().to_path_buf(),
        to: ctx.to.base().to_path_buf(),
        from_config: ctx.from.claude_json(),
        to_config: ctx.to.claude_json(),
        entries: entry.into_iter().collect(),
        removes: removes.into_iter().collect(),
        history,
        history_cwds,
        projects: Vec::new(),
        warnings: Vec::new(),
        customs: None,
        snapshot: req.snapshot,
        refresh: req.refresh,
        from_account: None,
        to_account: None,
    })
}

/// Why a bulk request left something out (§13.3 rule 2): a conversation
/// (`id`), a project (`cwd`) or a customization (`item`), with a code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub code: String,
    pub message: String,
}

impl Skipped {
    pub fn session(id: &str, code: &str, message: impl Into<String>) -> Self {
        Self {
            id: Some(id.to_string()),
            cwd: None,
            item: None,
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn project(cwd: &str, code: &str, message: impl Into<String>) -> Self {
        Self {
            id: None,
            cwd: Some(cwd.to_string()),
            item: None,
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn item(item: &str, code: &str, message: impl Into<String>) -> Self {
        Self {
            id: None,
            cwd: None,
            item: Some(item.to_string()),
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn of(id: &str, e: &TransferError) -> Self {
        Self::session(id, e.code(), e.to_string())
    }
}

/// A bulk request between two accounts in different directories (§13.4
/// `items`): conversations and projects, moved or copied in one job.
#[derive(Debug, Clone)]
pub struct Items {
    pub sessions: Vec<String>,
    /// Absolute folders: every conversation of each, its prompt history and
    /// (with `trust`) its `.claude.json` entry.
    pub projects: Vec<String>,
    pub mode: Mode,
    pub trust: bool,
    /// Customizations, copied only (§13.3 rule 7).
    pub customs: Vec<custom::Custom>,
    /// Customizations: replace what the target has.
    pub replace_existing: bool,
}

/// Whether a conversation of the source directory is the source account's
/// (in the shared folder only those attributed to it are):
/// `Err((code, message))` leaves it out.
pub type Belongs<'a> = &'a dyn Fn(&str) -> Result<(), (String, String)>;

/// The conversations of `from` that recorded exactly `cwd` (a look-alike
/// folder encodes the same).
fn project_sessions(from: &ClaudeHome, cwd: &str) -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(from.transcript_dir(cwd))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".jsonl")?.to_string();
            (super::ids::is_session_id(&id) && recorded_cwd(&e.path()).as_deref() == Some(cwd))
                .then_some(id)
        })
        .collect();
    ids.sort();
    ids
}

/// Plan a bulk request (§13.3 rules 1–3): what cannot go — a conversation
/// that runs, that the target has, that cannot be read or is not the source
/// account's; a project with no conversation here — is listed in `skipped`
/// with its code and the rest goes ahead. A project's prompt history and
/// entry leave the source only when every conversation of it goes. A
/// shared-file write meeting a directory in use still refuses the whole
/// request (D4). Nothing is touched. Blocking.
pub fn plan_items(
    ctx: &Context<'_>,
    req: &Items,
    belongs: Belongs<'_>,
) -> Result<(Plan, Vec<Skipped>), TransferError> {
    let same = |a: &Path, b: &Path| {
        let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
        let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
        a == b
    };
    if same(ctx.from.base(), ctx.to.base()) {
        return Err(TransferError::SameDirectory);
    }
    if req.mode == Mode::Move && !req.customs.is_empty() {
        return Err(TransferError::Invalid(
            "customizations are only copied, never moved".into(),
        ));
    }
    let mut skipped = Vec::new();
    let mut warnings = Vec::new();
    // The projects, each with the conversations of it that are the source
    // account's and whether those are all of them.
    let mut projects: Vec<(String, Vec<String>, bool)> = Vec::new();
    for raw in &req.projects {
        let cwd = match raw.trim_end_matches('/') {
            "" => "/".to_string(),
            c => c.to_string(),
        };
        if !Path::new(&cwd).is_absolute() {
            skipped.push(Skipped::project(
                raw,
                "invalid_transfer",
                "a project is named by its absolute folder",
            ));
            continue;
        }
        if projects.iter().any(|(c, _, _)| *c == cwd) {
            continue;
        }
        let all = project_sessions(ctx.from, &cwd);
        let mine: Vec<String> = all
            .iter()
            .filter(|id| belongs(id).is_ok())
            .cloned()
            .collect();
        if mine.is_empty() {
            skipped.push(Skipped::project(
                &cwd,
                "not_found",
                format!("no conversations of {cwd} here"),
            ));
            continue;
        }
        let complete = mine.len() == all.len();
        projects.push((cwd, mine, complete));
    }
    let mut sessions: Vec<SessionPlan> = Vec::new();
    let mut done: HashSet<String> = HashSet::new();
    let check = |id: &str, of_project: bool| -> Result<SessionPlan, Skipped> {
        if !super::ids::is_session_id(id) {
            return Err(Skipped::session(
                id,
                "invalid_transfer",
                format!("{id:?} is not a session id"),
            ));
        }
        if !of_project {
            belongs(id).map_err(|(code, message)| Skipped::session(id, &code, message))?;
        }
        if (ctx.is_live)(id) {
            return Err(Skipped::of(id, &TransferError::SessionLive(id.to_string())));
        }
        if !transcripts_of(ctx.to, id).is_empty() {
            return Err(Skipped::of(
                id,
                &TransferError::AlreadyExists(id.to_string()),
            ));
        }
        plan_session(ctx.from, ctx.to, id).map_err(|e| Skipped::of(id, &e))
    };
    for (_, ids, complete) in projects.iter_mut() {
        for id in ids.iter() {
            if !done.insert(id.clone()) {
                continue;
            }
            match check(id, true) {
                Ok(p) => sessions.push(p),
                Err(s) => {
                    skipped.push(s);
                    *complete = false;
                }
            }
        }
    }
    for id in &req.sessions {
        if !done.insert(id.clone()) {
            continue;
        }
        match check(id, false) {
            Ok(p) => sessions.push(p),
            Err(s) => skipped.push(s),
        }
    }
    let planned: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    // Only the projects something of goes.
    projects.retain(|(_, ids, _)| ids.iter().any(|id| planned.contains(id.as_str())));
    // The entries (trust opt-in, never over one): each project's, and the
    // folder of each conversation named on its own.
    let mut entries: Vec<(String, Value)> = Vec::new();
    if req.trust {
        let cwds = projects.iter().map(|(c, _, _)| c.clone()).chain(
            sessions
                .iter()
                .filter(|s| !projects.iter().any(|(_, ids, _)| ids.contains(&s.id)))
                .filter_map(|s| s.cwd.clone()),
        );
        for cwd in cwds {
            if entries.iter().any(|(c, _)| *c == cwd)
                || project_entry(&ctx.to.claude_json(), &cwd).is_some()
            {
                continue;
            }
            if let Some(e) = project_entry(&ctx.from.claude_json(), &cwd) {
                entries.push((cwd, e));
            }
        }
    }
    // Each project's prompt history the target lacks, once.
    let mut have: HashSet<(String, String)> = HashSet::new();
    let mut history: Vec<String> = Vec::new();
    for (cwd, _, _) in &projects {
        have.extend(
            history_lines(&ctx.to.history_file(), cwd)
                .iter()
                .filter_map(|l| history_key(l)),
        );
        for line in history_lines(&ctx.from.history_file(), cwd) {
            if history_key(&line).is_some_and(|k| have.insert(k)) {
                history.push(line);
            }
        }
    }
    // A move takes a project's history and carried entry out of the source
    // only when every conversation of it goes.
    let mut removes = Vec::new();
    let mut history_cwds = Vec::new();
    if req.mode == Mode::Move {
        for (cwd, _, complete) in &projects {
            if !complete {
                warnings.push(format!(
                    "Some conversations of {cwd} stay where they are, so its prompt history \
                     and folder settings stay there too."
                ));
                continue;
            }
            history_cwds.push(cwd.clone());
            if entries.iter().any(|(c, _)| c == cwd) {
                removes.push(cwd.clone());
            }
        }
    }
    let customs = if req.customs.is_empty() {
        None
    } else {
        let mut cp = custom::plan(ctx.from, ctx.to, &req.customs, req.replace_existing)?;
        skipped.append(&mut cp.skipped);
        warnings.append(&mut cp.warnings);
        // The whole history and a project's lines go once.
        for line in std::mem::take(&mut cp.history) {
            if history_key(&line).is_some_and(|k| have.insert(k)) {
                history.push(line);
            }
        }
        Some(cp)
    };
    let writes_shared = !entries.is_empty()
        || !history.is_empty()
        || customs
            .as_ref()
            .is_some_and(custom::CustomPlan::writes_shared);
    if writes_shared {
        if let Some(e) = in_use(ctx.to) {
            return Err(e);
        }
    }
    let source_lines = history_cwds
        .iter()
        .any(|c| !history_lines(&ctx.from.history_file(), c).is_empty());
    if !removes.is_empty() || source_lines {
        if let Some(e) = in_use(ctx.from) {
            return Err(e);
        }
    }
    let files = sessions.iter().map(|s| s.files.len()).sum::<usize>()
        + customs.as_ref().map_or(0, |c| c.files);
    let bytes = sessions
        .iter()
        .flat_map(|s| s.files.iter())
        .map(|f| f.size)
        .sum::<u64>()
        + customs.as_ref().map_or(0, |c| c.bytes);
    let what = match (projects.first(), sessions.first()) {
        (Some((cwd, _, _)), _) => What::Project { cwd: cwd.clone() },
        (None, Some(s)) => What::Session { id: s.id.clone() },
        (None, None) => What::Session { id: String::new() },
    };
    let plan = Plan {
        what,
        mode: req.mode,
        sessions,
        files,
        bytes,
        adds_project_entry: entries.first().map(|(c, _)| c.clone()),
        removes_project_entry: removes.first().cloned(),
        history_lines: history.len(),
        writes_shared,
        from: ctx.from.base().to_path_buf(),
        to: ctx.to.base().to_path_buf(),
        from_config: ctx.from.claude_json(),
        to_config: ctx.to.claude_json(),
        entries,
        removes,
        history,
        history_cwds,
        projects: projects.into_iter().map(|(c, _, _)| c).collect(),
        warnings,
        customs,
        snapshot: false,
        refresh: false,
        from_account: None,
        to_account: None,
    };
    Ok((plan, skipped))
}

impl Plan {
    /// The cwds whose `.claude.json` entry it adds to the target.
    pub fn added_entries(&self) -> Vec<String> {
        self.entries.iter().map(|(c, _)| c.clone()).collect()
    }

    /// The cwds whose entry it takes out of the source.
    pub fn removed_entries(&self) -> &[String] {
        &self.removes
    }

    /// Whether it changes the source's shared files (a move's prompt history
    /// or entries).
    fn removes_from_source(&self) -> bool {
        self.mode == Mode::Move && (!self.removes.is_empty() || !self.history_cwds.is_empty())
    }

    /// Whether running it would change anything.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
            && self.entries.is_empty()
            && self.history.is_empty()
            && !self.removes_from_source()
            && self
                .customs
                .as_ref()
                .is_none_or(custom::CustomPlan::is_empty)
    }
}

// ─────────────────────────────── running it ───────────────────────────────

/// A file the transfer created, as it was right after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
    /// A symbolic link the job made (a skill linked from a repository),
    /// pointing here; `size` and `sha256` are then empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<PathBuf>,
}

impl Created {
    /// Whether it is still exactly what the job made (`None`: it is gone).
    fn state(&self) -> std::io::Result<Option<bool>> {
        if let Some(target) = &self.link {
            return match std::fs::read_link(&self.path) {
                Ok(t) => Ok(Some(t == *target)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            };
        }
        match sha256_file(&self.path) {
            Ok((size, sha)) => Ok(Some(size == self.size && sha == self.sha256)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// The set a move into or out of a storage used (docs/ACCOUNTS.md §13.2),
/// as its job recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageRef {
    /// `backup` or `stor_…`.
    pub storage: String,
    /// The set's name.
    pub set: String,
    /// The set's folder then.
    pub dir: PathBuf,
    /// Into the set (else out of it).
    pub into: bool,
}

impl Manifest {
    /// A manifest of nothing done yet.
    pub fn new(job_id: &str, mode: Mode, what: What, from_dir: PathBuf, to_dir: PathBuf) -> Self {
        Self {
            job_id: job_id.to_string(),
            created_at: now_ms(),
            mode,
            what,
            from_dir,
            to_dir,
            from_config: None,
            to_config: None,
            sessions: Vec::new(),
            created: Vec::new(),
            created_dirs: Vec::new(),
            moved: Vec::new(),
            moved_dirs: Vec::new(),
            added_entry: None,
            added_entries: Vec::new(),
            removed_entry: None,
            removed_entries: Vec::new(),
            added_history: Vec::new(),
            removed_history: Vec::new(),
            undone_at: None,
            kind: None,
            from_account: None,
            to_account: None,
            storage: None,
            archived_marks: Vec::new(),
            set_entries: Vec::new(),
            mcp: Vec::new(),
            assigned: Vec::new(),
            storage_to: None,
            added_set_entries: Vec::new(),
        }
    }

    /// Whether undoing it would change anything.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.moved.is_empty()
            && self.added_entry.is_none()
            && self.added_entries.is_empty()
            && self.removed_entry.is_none()
            && self.removed_entries.is_empty()
            && self.added_history.is_empty()
            && self.removed_history.is_empty()
            && self.archived_marks.is_empty()
            && self.set_entries.is_empty()
            && self.mcp.is_empty()
            && self.assigned.is_empty()
            && self.added_set_entries.is_empty()
    }

    /// The source's `.claude.json` (older manifests: inside its directory).
    pub fn from_config(&self) -> PathBuf {
        self.from_config
            .clone()
            .unwrap_or_else(|| self.from_dir.join(".claude.json"))
    }

    /// The target's `.claude.json`.
    pub fn to_config(&self) -> PathBuf {
        self.to_config
            .clone()
            .unwrap_or_else(|| self.to_dir.join(".claude.json"))
    }
}

/// A source file moved into the undo directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moved {
    pub original: PathBuf,
    /// Relative to the undo directory.
    pub saved: PathBuf,
}

/// Everything undo needs, kept beside the moved files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub job_id: String,
    pub created_at: i64,
    pub mode: Mode,
    pub what: What,
    pub from_dir: PathBuf,
    pub to_dir: PathBuf,
    /// The `.claude.json` files the entries were added to / removed from.
    #[serde(default)]
    pub from_config: Option<PathBuf>,
    #[serde(default)]
    pub to_config: Option<PathBuf>,
    pub sessions: Vec<String>,
    pub created: Vec<Created>,
    /// Directories created in the target (removed by undo when empty).
    pub created_dirs: Vec<PathBuf>,
    pub moved: Vec<Moved>,
    /// Source directories emptied by a move (re-created by undo).
    pub moved_dirs: Vec<PathBuf>,
    /// `(cwd, entry)` added to the target's `.claude.json`.
    pub added_entry: Option<(String, Value)>,
    /// More of them (a restore of sessions from several projects).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_entries: Vec<(String, Value)>,
    /// `(cwd, entry)` removed from the source's `.claude.json`.
    pub removed_entry: Option<(String, Value)>,
    /// More of them (a move of several projects).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_entries: Vec<(String, Value)>,
    /// Lines appended to the target's `history.jsonl`.
    pub added_history: Vec<String>,
    /// Lines removed from the source's `history.jsonl`.
    pub removed_history: Vec<String>,
    #[serde(default)]
    pub undone_at: Option<i64>,
    /// What made it, when not a move or copy (`restore`, `pull`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The accounts the job named (two may share a directory).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_account: Option<String>,
    /// The set of a move into or out of a storage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageRef>,
    /// The marks a move into a set made there (its undo clears them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_marks: Vec<(String, crate::claude::backup::Archived)>,
    /// The entries a move out of a set took out of it (its undo puts them
    /// back).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set_entries: Vec<crate::claude::backup::IndexEntry>,
    /// The MCP servers a copy of customizations wrote into the target's
    /// `.claude.json` (its undo sets each back).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp: Vec<custom::McpChange>,
    /// An assign between accounts that share a directory: each conversation
    /// and the account it was before (its undo puts that one back).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assigned: Vec<(String, String)>,
    /// The target set of a copy or move between two sets (`storage` is the
    /// source's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_to: Option<StorageRef>,
    /// The entries such a copy added to the target set, with their archives
    /// (its undo takes them out again).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_set_entries: Vec<(String, String)>,
}

/// Progress of a running transfer.
#[derive(Debug, Default)]
pub struct Progress {
    pub files_done: AtomicU64,
    pub bytes_done: AtomicU64,
}

pub(crate) fn sha256_file(path: &Path) -> std::io::Result<(u64, String)> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut n = 0u64;
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            break;
        }
        n += k as u64;
        h.update(&buf[..k]);
    }
    Ok((n, h.finalize().iter().map(|b| format!("{b:02x}")).collect()))
}

/// SHA-256 of a file's first `len` bytes.
fn sha256_prefix(path: &Path, len: u64) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut reader = f.take(len);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let k = reader.read(&mut buf)?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Cut a JSONL file after its last complete line (a snapshot of a file the
/// CLI is appending to). Reads backwards from the end in blocks: a
/// transcript can be hundreds of megabytes.
fn trim_partial_line(path: &Path) -> std::io::Result<()> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::options().read(true).write(true).open(path)?;
    let len = f.metadata()?.len();
    let mut end = len;
    let mut buf = vec![0u8; 64 * 1024];
    let mut keep = 0u64;
    while end > 0 {
        let start = end.saturating_sub(buf.len() as u64);
        let n = (end - start) as usize;
        f.seek(SeekFrom::Start(start))?;
        f.read_exact(&mut buf[..n])?;
        if end == len && buf[n - 1] == b'\n' {
            return Ok(());
        }
        if let Some(i) = buf[..n].iter().rposition(|b| *b == b'\n') {
            keep = start + i as u64 + 1;
            break;
        }
        end = start;
    }
    f.set_len(keep)
}

/// Copy one file: a temp name beside the destination, fsync, the source's
/// mtime, rename into place, then verify against the source (in a snapshot,
/// against the source's first bytes: it may have grown since).
fn copy_file(c: &FileCopy, snapshot: bool) -> Result<Created, TransferError> {
    let dir =
        c.to.parent()
            .ok_or_else(|| TransferError::Invalid(format!("{} has no parent", c.to.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| io("creating", dir, e))?;
    if c.to.exists() {
        return Err(TransferError::AlreadyExists(c.to.display().to_string()));
    }
    let name = c.to.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.mc-{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<(), TransferError> {
        std::fs::copy(&c.from, &tmp).map_err(|e| io("copying", &c.from, e))?;
        if snapshot && tmp_is_jsonl(&c.to) {
            trim_partial_line(&tmp).map_err(|e| io("trimming", &tmp, e))?;
        }
        let f = std::fs::File::options()
            .write(true)
            .open(&tmp)
            .map_err(|e| io("opening", &tmp, e))?;
        if let Ok(mtime) = std::fs::metadata(&c.from).and_then(|m| m.modified()) {
            let _ = f.set_modified(mtime);
        }
        f.sync_all().map_err(|e| io("syncing", &tmp, e))?;
        std::fs::rename(&tmp, &c.to).map_err(|e| io("placing", &c.to, e))
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let (size, sha) = sha256_file(&c.to).map_err(|e| io("verifying", &c.to, e))?;
    let matches = if snapshot {
        sha256_prefix(&c.from, size).map_err(|e| io("verifying", &c.from, e))? == sha
    } else {
        let (src_size, src_sha) = sha256_file(&c.from).map_err(|e| io("verifying", &c.from, e))?;
        size == src_size && sha == src_sha
    };
    if !matches {
        let _ = std::fs::remove_file(&c.to);
        return Err(TransferError::VerifyFailed(c.to.display().to_string()));
    }
    Ok(Created {
        path: c.to.clone(),
        size,
        sha256: sha,
        link: None,
    })
}

fn tmp_is_jsonl(dest: &Path) -> bool {
    dest.extension().is_some_and(|e| e == "jsonl")
}

/// Rewrite a JSON file atomically (temp + rename), keeping its mode.
pub(crate) fn rewrite_json(path: &Path, value: &Value) -> Result<(), TransferError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| TransferError::Io(e.to_string()))?;
    rewrite(path, &bytes)
}

pub(crate) fn rewrite(path: &Path, bytes: &[u8]) -> Result<(), TransferError> {
    let dir = path
        .parent()
        .ok_or_else(|| TransferError::Invalid(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| io("creating", dir, e))?;
    let tmp = dir.join(format!(
        ".{}.mc-{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> Result<(), TransferError> {
        // Owner-only from its first byte (a `.claude.json` holds MCP secrets);
        // the original's mode is put back below.
        let mut opts = std::fs::File::options();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(|e| io("writing", &tmp, e))?;
        f.write_all(bytes).map_err(|e| io("writing", &tmp, e))?;
        f.sync_all().map_err(|e| io("syncing", &tmp, e))?;
        if let Ok(perm) = std::fs::metadata(path).map(|m| m.permissions()) {
            let _ = std::fs::set_permissions(&tmp, perm);
        } else {
            crate::persist::restrict(&tmp);
        }
        std::fs::rename(&tmp, path).map_err(|e| io("placing", path, e))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `.claude.json` with `projects[cwd]` changed by `f` (the rest as it is).
pub(crate) fn edit_projects(
    claude_json: &Path,
    f: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> Result<(), TransferError> {
    edit_claude_json(claude_json, |obj| {
        let projects = obj
            .entry("projects")
            .or_insert_with(|| Value::Object(Default::default()));
        let projects = projects.as_object_mut().ok_or_else(|| {
            TransferError::Io(format!("{} has no projects object", claude_json.display()))
        })?;
        f(projects);
        Ok(())
    })
}

/// `.claude.json` with its top-level object changed by `f` (the rest as it
/// is), rewritten atomically; a missing file starts empty, an unreadable one
/// is an error, never a file to write over.
pub(crate) fn edit_claude_json(
    claude_json: &Path,
    f: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<(), TransferError>,
) -> Result<(), TransferError> {
    let mut root: Value = match std::fs::read_to_string(claude_json) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            TransferError::Io(format!(
                "{} is not readable JSON: {e}",
                claude_json.display()
            ))
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Default::default()),
        Err(e) => return Err(io("reading", claude_json, e)),
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| TransferError::Io(format!("{} is not an object", claude_json.display())))?;
    f(obj)?;
    rewrite_json(claude_json, &root)
}

/// A file's bytes; only a missing file reads as empty (an unreadable one is
/// an error, never an empty file to write over).
fn read_or_empty(path: &Path) -> Result<Vec<u8>, TransferError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(io("reading", path, e)),
    }
}

/// A JSONL file's lines as bytes, without their `\n`.
fn byte_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split_inclusive(|b| *b == b'\n')
        .map(|l| l.strip_suffix(b"\n").unwrap_or(l))
}

/// Append lines to a JSONL file (atomically: the whole file rewritten, as
/// bytes, so a line that is not UTF-8 stays as it is).
pub(crate) fn append_lines(path: &Path, lines: &[String]) -> Result<(), TransferError> {
    let mut bytes = read_or_empty(path)?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    for l in lines {
        bytes.extend_from_slice(l.as_bytes());
        bytes.push(b'\n');
    }
    rewrite(path, &bytes)
}

/// Append those of `lines` the file does not have yet (counting repeats): an
/// undo that is retried adds nothing twice.
fn append_missing_lines(path: &Path, lines: &[String]) -> Result<(), TransferError> {
    let bytes = read_or_empty(path)?;
    let mut have: Vec<&[u8]> = byte_lines(&bytes).collect();
    let missing: Vec<String> = lines
        .iter()
        .filter(|l| match have.iter().position(|h| *h == l.as_bytes()) {
            Some(i) => {
                have.swap_remove(i);
                false
            }
            None => true,
        })
        .cloned()
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    append_lines(path, &missing)
}

/// Remove exactly these lines (once each) from a JSONL file; every other
/// byte stays. Returns the lines it found and removed.
fn remove_lines(path: &Path, lines: &[String]) -> Result<Vec<String>, TransferError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io("reading", path, e)),
    };
    let mut drop: Vec<&String> = lines.iter().collect();
    let mut removed = Vec::new();
    let mut out = Vec::with_capacity(bytes.len());
    for l in bytes.split_inclusive(|b| *b == b'\n') {
        let content = l.strip_suffix(b"\n").unwrap_or(l);
        let content = content.strip_suffix(b"\r").unwrap_or(content);
        if let Some(i) = drop.iter().position(|d| d.as_bytes() == content) {
            removed.push(drop.remove(i).clone());
            continue;
        }
        out.extend_from_slice(l);
    }
    if removed.is_empty() {
        return Ok(removed);
    }
    rewrite(path, &out)?;
    Ok(removed)
}

/// Keep a copy of a shared file as it is before the job's first change to it
/// (`<undo dir>/shared/<label>`), for recovery by hand.
fn keep_shared(path: &Path, undo_dir: &Path, label: &str) -> Result<(), TransferError> {
    let dest = undo_dir.join("shared").join(label);
    if dest.exists() || !path.exists() {
        return Ok(());
    }
    let dir = undo_dir.join("shared");
    std::fs::create_dir_all(&dir).map_err(|e| io("creating", &dir, e))?;
    private_dir(&dir);
    std::fs::copy(path, &dest).map_err(|e| io("keeping a copy of", path, e))?;
    crate::persist::restrict(&dest);
    Ok(())
}

// Test-only failure injection: `fail_at("stash", 2)` makes the third pass
// through `fail_point("stash")` on this thread fail.
#[cfg(test)]
thread_local! {
    static FAIL_AT: std::cell::RefCell<Vec<(&'static str, usize)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn fail_at(point: &'static str, after: usize) {
    FAIL_AT.with(|f| f.borrow_mut().push((point, after)));
}

#[cfg(test)]
pub(crate) fn clear_failures() {
    FAIL_AT.with(|f| f.borrow_mut().clear());
}

fn fail_point(point: &'static str) -> Result<(), TransferError> {
    #[cfg(test)]
    {
        let hit = FAIL_AT.with(|f| {
            let mut f = f.borrow_mut();
            match f.iter_mut().position(|(p, _)| *p == point) {
                Some(i) if f[i].1 == 0 => {
                    f.remove(i);
                    true
                }
                Some(i) => {
                    f[i].1 -= 1;
                    false
                }
                None => false,
            }
        });
        if hit {
            return Err(TransferError::Io(format!("injected failure at {point}")));
        }
    }
    let _ = point;
    Ok(())
}

/// Move a file into the undo directory: a rename on one volume, else a
/// copy and a delete.
pub(crate) fn stash(path: &Path, undo_dir: &Path, rel: &Path) -> Result<Moved, TransferError> {
    fail_point("stash")?;
    let dest = undo_dir.join("moved").join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io("creating", parent, e))?;
    }
    if std::fs::rename(path, &dest).is_err() {
        copy_entry(path, &dest).map_err(|e| io("saving", path, e))?;
        if let Err(e) = std::fs::remove_file(path) {
            let _ = std::fs::remove_file(&dest);
            return Err(io("removing", path, e));
        }
    }
    Ok(Moved {
        original: path.to_path_buf(),
        saved: PathBuf::from("moved").join(rel),
    })
}

/// Copy a file, or a symbolic link as a link to the same target (never what
/// it points to).
fn copy_entry(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(from)?.file_type().is_symlink() {
        return make_symlink(&std::fs::read_link(from)?, to);
    }
    std::fs::copy(from, to).map(|_| ())
}

/// Make a symbolic link at `link` pointing to `target` (on Windows a
/// directory link when the target is a directory).
pub(crate) fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
}

/// Put a saved file back: a rename on one volume, else a copy (the saved
/// copy is removed only once the original is back).
pub(crate) fn put_back(saved: &Path, original: &Path) -> Result<(), TransferError> {
    fail_point("put_back")?;
    if let Some(parent) = original.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io("creating", parent, e))?;
    }
    if std::fs::rename(saved, original).is_err() {
        copy_entry(saved, original).map_err(|e| io("restoring", original, e))?;
        let _ = std::fs::remove_file(saved);
    }
    Ok(())
}

/// Remove a directory tree's empty directories, deepest first: whatever a
/// session wrote there that the plan did not know stays.
pub(crate) fn remove_empty_dirs(dir: &Path) {
    let mut dirs = Vec::new();
    let _ = collect_dirs(dir, &mut dirs);
    for sub in dirs.iter().rev() {
        let _ = std::fs::remove_dir(sub);
    }
    let _ = std::fs::remove_dir(dir);
}

/// The size and mtime a file has now (`None` when it is gone).
fn file_stamp(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

/// The checks a plan made that its shared-file writes depend on, again: the
/// target's `.claude.json`/`history.jsonl` and, for a project move, the
/// source's, only while no Claude Code process uses that directory.
fn shared_checks(plan: &Plan) -> Result<(), TransferError> {
    if plan.writes_shared {
        if let Some(e) = in_use_at(&plan.to, &plan.to_config) {
            return Err(e);
        }
    }
    if plan.removes_from_source() {
        if let Some(e) = in_use_at(&plan.from, &plan.from_config) {
            return Err(e);
        }
    }
    Ok(())
}

/// A plan's preconditions once more, right before it runs (it may have
/// waited behind another data job): no session of it started since, the
/// target did not get one of them meanwhile, the shared files' directories
/// are free.
pub fn recheck(plan: &Plan, is_live: &dyn Fn(&str) -> bool) -> Result<(), TransferError> {
    let to = ClaudeHome::for_account_dir(&plan.to);
    for s in &plan.sessions {
        if !plan.snapshot && is_live(&s.id) {
            return Err(TransferError::SessionLive(s.id.clone()));
        }
        if !plan.refresh && !transcripts_of(&to, &s.id).is_empty() {
            return Err(TransferError::AlreadyExists(s.id.clone()));
        }
    }
    shared_checks(plan)
}

/// How a copy made earlier stands against its source now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyState {
    /// The same bytes (a source that only has an unfinished line more counts
    /// as the same: a snapshot leaves that out).
    Same,
    /// The start of the source: the source went on, the copy did not.
    Behind,
    /// Written to since it was copied (or never a copy of this source).
    Changed,
}

fn copy_state(copy: &Path, source: &Path) -> Result<CopyState, TransferError> {
    let (n, sha) = sha256_file(copy).map_err(|e| io("reading", copy, e))?;
    let src_len = std::fs::metadata(source)
        .map_err(|e| io("reading", source, e))?
        .len();
    if n > src_len || sha256_prefix(source, n).map_err(|e| io("reading", source, e))? != sha {
        return Ok(CopyState::Changed);
    }
    if n == src_len || !has_newline_after(source, n).map_err(|e| io("reading", source, e))? {
        return Ok(CopyState::Same);
    }
    Ok(CopyState::Behind)
}

/// Whether a file has a `\n` anywhere after byte `from`.
fn has_newline_after(path: &Path, from: u64) -> std::io::Result<bool> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(from))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            return Ok(false);
        }
        if buf[..k].contains(&b'\n') {
            return Ok(true);
        }
    }
}

/// Whether a session has a file now that the plan does not list (it was
/// resumed after the plan was made).
fn has_unplanned_files(s: &SessionPlan) -> bool {
    let planned: HashSet<&Path> = s.files.iter().map(|f| f.from.as_path()).collect();
    s.dirs.iter().any(|(dir, _)| {
        let mut found = Vec::new();
        let _ = walk(dir, &mut found);
        found.iter().any(|f| !planned.contains(f.as_path()))
    })
}

/// Run a planned transfer: undo copy first, then the files, then the shared
/// files, then (move mode) the source. Blocking. The plan's checks are made
/// again where they matter: before the shared files are written and before
/// the source moves (a session that started meanwhile, a file that changed
/// since it was copied). On failure the source is put back and everything
/// created is removed; a moved file is never lost (see the module docs).
pub fn run(
    plan: &Plan,
    undo_root: &Path,
    job_id: &str,
    progress: &Progress,
    is_live: &dyn Fn(&str) -> bool,
) -> Result<Manifest, TransferError> {
    let undo_dir = undo_root.join(job_id);
    std::fs::create_dir_all(&undo_dir).map_err(|e| io("creating", &undo_dir, e))?;
    private_dir(undo_root);
    private_dir(&undo_dir);
    let mut m = Manifest {
        from_config: Some(plan.from_config.clone()),
        to_config: Some(plan.to_config.clone()),
        sessions: plan.sessions.iter().map(|s| s.id.clone()).collect(),
        from_account: plan.from_account.clone(),
        to_account: plan.to_account.clone(),
        ..Manifest::new(
            job_id,
            plan.mode,
            plan.what.clone(),
            plan.from.clone(),
            plan.to.clone(),
        )
    };
    let save = |m: &Manifest| -> Result<(), TransferError> {
        fail_point("save")?;
        crate::persist::write_json_private(&undo_dir.join("manifest.json"), m)
            .map_err(|e| io("writing", &undo_dir, e))
    };
    if let Err(e) = save(&m) {
        let _ = std::fs::remove_dir_all(&undo_dir);
        return Err(e);
    }
    // What each source file was right after its copy verified: it must
    // still be that when it moves, or the copy is not what moves.
    let mut stamps: std::collections::HashMap<PathBuf, Option<(u64, Option<SystemTime>)>> =
        std::collections::HashMap::new();
    let outcome = (|| -> Result<(), TransferError> {
        // The files, verified. Every directory made on the way is recorded,
        // so undo leaves the target as it found it.
        for s in &plan.sessions {
            for (_, dest) in &s.dirs {
                for d in missing_dirs(dest, &plan.to) {
                    std::fs::create_dir(&d).map_err(|e| io("creating", &d, e))?;
                    m.created_dirs.push(d);
                }
            }
            for f in &s.files {
                if let Some(parent) = f.to.parent() {
                    for d in missing_dirs(parent, &plan.to) {
                        std::fs::create_dir(&d).map_err(|e| io("creating", &d, e))?;
                        m.created_dirs.push(d);
                    }
                }
                if plan.refresh && is_plain_file(&f.to) {
                    // An earlier copy: replaced only when it is behind (the
                    // old one goes to the undo directory); one that changed
                    // in the target stays, and a changed transcript stops it.
                    match copy_state(&f.to, &f.from)? {
                        CopyState::Same => continue,
                        CopyState::Changed if f.to.extension().is_some_and(|e| e == "jsonl") => {
                            return Err(TransferError::SessionChanged(s.id.clone()));
                        }
                        CopyState::Changed => continue,
                        CopyState::Behind => {
                            let rel = f.to.strip_prefix(&plan.to).unwrap_or(&f.to);
                            m.moved
                                .push(stash(&f.to, &undo_dir, &Path::new("target").join(rel))?);
                        }
                    }
                }
                let created = copy_file(f, plan.snapshot)?;
                stamps.insert(f.from.clone(), file_stamp(&f.from));
                progress.files_done.fetch_add(1, Ordering::Relaxed);
                progress
                    .bytes_done
                    .fetch_add(created.size, Ordering::Relaxed);
                m.created.push(created);
            }
            save(&m)?;
        }
        // The target's shared files (trust opt-in; a project's history),
        // only while nothing uses its directory now.
        let to = ClaudeHome::for_account_dir(&plan.to);
        if plan.writes_shared {
            shared_checks(plan)?;
        }
        if !plan.entries.is_empty() {
            keep_shared(&plan.to_config, &undo_dir, "to-claude.json")?;
            let mut added = Vec::new();
            edit_projects(&plan.to_config, |p| {
                for (cwd, entry) in &plan.entries {
                    if !p.contains_key(cwd) {
                        p.insert(cwd.clone(), entry.clone());
                        added.push((cwd.clone(), entry.clone()));
                    }
                }
            })?;
            m.added_entries = added;
            save(&m)?;
        }
        if !plan.history.is_empty() {
            keep_shared(&to.history_file(), &undo_dir, "to-history.jsonl")?;
            append_lines(&to.history_file(), &plan.history)?;
            m.added_history = plan.history.clone();
            save(&m)?;
        }
        if let Some(cp) = &plan.customs {
            custom::run(cp, &to, &plan.to_config, &undo_dir, &mut m, progress, &save)?;
        }
        // Move mode: the source goes to the undo directory, only now that
        // every copy verified, and only while none of its sessions runs and
        // each file is still what was copied.
        if plan.mode == Mode::Move {
            for s in &plan.sessions {
                if is_live(&s.id) {
                    return Err(TransferError::SessionLive(s.id.clone()));
                }
                let changed = has_unplanned_files(s)
                    || s.files
                        .iter()
                        .any(|f| stamps.get(&f.from).cloned().flatten() != file_stamp(&f.from));
                if changed {
                    return Err(TransferError::SessionChanged(s.id.clone()));
                }
            }
            for s in &plan.sessions {
                for f in &s.files {
                    let rel = f.from.strip_prefix(&plan.from).unwrap_or(&f.from);
                    m.moved.push(stash(&f.from, &undo_dir, rel)?);
                }
                for (dir, _) in s.dirs.iter().rev() {
                    remove_empty_dirs(dir);
                    if !dir.exists() {
                        m.moved_dirs.push(dir.clone());
                    }
                }
                save(&m)?;
            }
            if plan.removes_from_source() {
                shared_checks(plan)?;
                let from = ClaudeHome::for_account_dir(&plan.from);
                if !plan.removes.is_empty() {
                    fail_point("edit_source")?;
                    keep_shared(&plan.from_config, &undo_dir, "from-claude.json")?;
                    let mut removed = Vec::new();
                    edit_projects(&plan.from_config, |p| {
                        for cwd in &plan.removes {
                            if let Some(e) = p.remove(cwd) {
                                removed.push((cwd.clone(), e));
                            }
                        }
                    })?;
                    m.removed_entries = removed;
                    save(&m)?;
                }
                let lines: Vec<String> = plan
                    .history_cwds
                    .iter()
                    .flat_map(|cwd| history_lines(&from.history_file(), cwd))
                    .collect();
                if !lines.is_empty() {
                    keep_shared(&from.history_file(), &undo_dir, "from-history.jsonl")?;
                    m.removed_history = remove_lines(&from.history_file(), &lines)?;
                }
                save(&m)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = outcome {
        // The source first (its moved files are only in the undo directory
        // now), then what was made in the target. If either fails, the undo
        // directory and its manifest stay, and Undo finishes the job.
        let back = restore_source(&m, &undo_dir).and_then(|()| rollback(&m));
        return match back {
            Ok(()) => {
                let _ = std::fs::remove_dir_all(&undo_dir);
                Err(e)
            }
            Err(back) => {
                let _ = crate::persist::write_json_private(&undo_dir.join("manifest.json"), &m);
                tracing::error!(
                    job = job_id,
                    error = %e,
                    rollback = %back,
                    "a failed transfer could not be put back; its undo directory stays"
                );
                Err(TransferError::Incomplete(format!(
                    "{e}. Putting things back failed too ({back}); nothing is lost: \
                     undo this job to finish."
                )))
            }
        };
    }
    Ok(m)
}

/// Undo a finished transfer from its manifest (no session is checked for
/// being live: [`undo_with`] does that).
pub fn undo(undo_root: &Path, job_id: &str) -> Result<Manifest, TransferError> {
    undo_with(undo_root, job_id, &|_| false)
}

/// Undo a transfer: refuse while one of its sessions runs, while a
/// directory whose shared files it changes back is in use, or when what it
/// created has been used since (a continued session); then put the source
/// back, and only then remove what was created. Safe to retry after a
/// failure: what is back already is skipped.
pub fn undo_with(
    undo_root: &Path,
    job_id: &str,
    is_live: &dyn Fn(&str) -> bool,
) -> Result<Manifest, TransferError> {
    undo_with_before(undo_root, job_id, is_live, &mut |_| Ok(()))
}

/// [`undo_with`] that runs `before` once every check passed and before
/// anything is put back: a move out of a backup set puts the set's entries
/// back there first, and when that fails nothing is undone.
pub fn undo_with_before(
    undo_root: &Path,
    job_id: &str,
    is_live: &dyn Fn(&str) -> bool,
    before: &mut dyn FnMut(&Manifest) -> Result<(), TransferError>,
) -> Result<Manifest, TransferError> {
    if !is_job_id(job_id) {
        return Err(TransferError::Invalid("not a transfer id".into()));
    }
    let undo_dir = undo_root.join(job_id);
    let mut m: Manifest = crate::persist::read_json(&undo_dir.join("manifest.json"))
        .ok_or_else(|| TransferError::NotFound("nothing to undo for that transfer".into()))?;
    if m.undone_at.is_some() {
        return Err(TransferError::UndoConflict(
            "this transfer was undone already".into(),
        ));
    }
    if let Some(id) = m.sessions.iter().find(|id| is_live(id)) {
        return Err(TransferError::SessionLive(id.clone()));
    }
    let target_shared = m.added_entry.is_some()
        || !m.added_entries.is_empty()
        || !m.added_history.is_empty()
        || !m.mcp.is_empty()
        // Customizations a copy replaced in the target go back there.
        || m.moved.iter().any(|mv| mv.original.starts_with(&m.to_dir));
    if target_shared {
        if let Some(e) = in_use_at(&m.to_dir, &m.to_config()) {
            return Err(e);
        }
    }
    if m.removed_entry.is_some() || !m.removed_entries.is_empty() || !m.removed_history.is_empty() {
        if let Some(e) = in_use_at(&m.from_dir, &m.from_config()) {
            return Err(e);
        }
    }
    // A moved file that is back already (an earlier attempt) has no saved
    // copy left; its place is the original again, not this job's file.
    let back: HashSet<&PathBuf> = m
        .moved
        .iter()
        .filter(|mv| !undo_dir.join(&mv.saved).exists() && mv.original.exists())
        .map(|mv| &mv.original)
        .collect();
    // Refuse when what was created has been used since (a continued
    // session): deleting it would lose that work.
    for c in m.created.iter().filter(|c| !back.contains(&c.path)) {
        match c.state() {
            Ok(Some(true)) | Ok(None) => {}
            Ok(Some(false)) => {
                return Err(TransferError::UndoConflict(format!(
                    "{} changed after the transfer",
                    c.path.display()
                )))
            }
            Err(e) => return Err(io("reading", &c.path, e)),
        }
    }
    // A moved file's place may hold only what this job put there (a pull
    // that replaced it), which goes first.
    let ours: HashSet<&PathBuf> = m.created.iter().map(|c| &c.path).collect();
    for mv in &m.moved {
        let saved = undo_dir.join(&mv.saved);
        if saved.exists() && mv.original.exists() && !ours.contains(&mv.original) {
            return Err(TransferError::UndoConflict(format!(
                "{} exists again",
                mv.original.display()
            )));
        }
        if !saved.exists() && !mv.original.exists() {
            return Err(TransferError::NotFound(format!(
                "the saved copy of {} is gone",
                mv.original.display()
            )));
        }
    }
    before(&m)?;
    restore_source(&m, &undo_dir)?;
    rollback(&m)?;
    m.undone_at = Some(now_ms());
    crate::persist::write_json_private(&undo_dir.join("manifest.json"), &m)
        .map_err(|e| io("writing", &undo_dir, e))?;
    Ok(m)
}

/// Put the source back as it was: the emptied directories, the moved files
/// (in place of this job's own file where one stands there, as after a pull
/// that replaced a session), the removed `.claude.json` entry and the removed
/// history lines. What is back already is skipped, so a retry finishes.
fn restore_source(m: &Manifest, undo_dir: &Path) -> Result<(), TransferError> {
    if m.moved.is_empty()
        && m.removed_entry.is_none()
        && m.removed_entries.is_empty()
        && m.removed_history.is_empty()
    {
        return Ok(());
    }
    // Emptied source directories first (an empty `session-env/<id>` is one).
    for d in &m.moved_dirs {
        std::fs::create_dir_all(d).map_err(|e| io("creating", d, e))?;
    }
    let ours: std::collections::HashMap<&PathBuf, &Created> =
        m.created.iter().map(|c| (&c.path, c)).collect();
    for mv in &m.moved {
        let saved = undo_dir.join(&mv.saved);
        if !saved.exists() {
            if mv.original.exists() {
                continue;
            }
            return Err(TransferError::NotFound(format!(
                "the saved copy of {} is gone",
                mv.original.display()
            )));
        }
        if mv.original.exists() {
            // Only this job's own, unchanged file may stand there.
            let mine = ours
                .get(&mv.original)
                .is_some_and(|c| matches!(c.state(), Ok(Some(true))));
            if !mine {
                return Err(TransferError::UndoConflict(format!(
                    "{} exists again",
                    mv.original.display()
                )));
            }
            std::fs::remove_file(&mv.original).map_err(|e| io("removing", &mv.original, e))?;
        }
        put_back(&saved, &mv.original)?;
    }
    let removed: Vec<&(String, Value)> = m.removed_entry.iter().chain(&m.removed_entries).collect();
    if !removed.is_empty() {
        edit_projects(&m.from_config(), |p| {
            for (cwd, entry) in removed {
                p.entry(cwd.clone()).or_insert_with(|| entry.clone());
            }
        })?;
    }
    if !m.removed_history.is_empty() {
        let from = ClaudeHome::for_account_dir(&m.from_dir);
        append_missing_lines(&from.history_file(), &m.removed_history)?;
    }
    Ok(())
}

/// Remove what a transfer created in the target (files, then the
/// directories it made when empty) and the shared-file changes it made there.
/// A created path that is a moved file's place holds the original again by
/// now ([`restore_source`] ran first) and stays.
fn rollback(m: &Manifest) -> Result<(), TransferError> {
    let originals: HashSet<&PathBuf> = m.moved.iter().map(|mv| &mv.original).collect();
    for c in m.created.iter().filter(|c| !originals.contains(&c.path)) {
        match std::fs::remove_file(&c.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io("removing", &c.path, e)),
        }
    }
    for d in m.created_dirs.iter().rev() {
        // Deepest first (they were made outermost first); only what is empty
        // now, so anything written there since stays.
        remove_empty_dirs(d);
    }
    let to = ClaudeHome::for_account_dir(&m.to_dir);
    let added: Vec<&(String, Value)> = m.added_entry.iter().chain(&m.added_entries).collect();
    if !added.is_empty() {
        edit_projects(&m.to_config(), |p| {
            for (cwd, entry) in added {
                if p.get(cwd) == Some(entry) {
                    p.remove(cwd);
                }
            }
        })?;
    }
    if !m.added_history.is_empty() {
        remove_lines(&to.history_file(), &m.added_history)?;
    }
    custom::undo_mcp(m)?;
    Ok(())
}

/// The directories from `stop` (exclusive, which exists) down to `dir` that
/// do not exist yet, outermost first.
pub(crate) fn missing_dirs(dir: &Path, stop: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if d == stop || d.exists() || !d.starts_with(stop) {
            break;
        }
        out.push(d.to_path_buf());
        cur = d.parent();
    }
    out.reverse();
    out
}

fn collect_dirs(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            out.push(e.path());
            collect_dirs(&e.path(), out)?;
        }
    }
    Ok(())
}

/// chmod 0700 (no-op off unix): our own directories, owner only.
pub(crate) fn private_dir(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// `<timestamp>-<8 hex>`.
pub fn new_job_id() -> String {
    let hex: String = crate::server::auth::random_bytes(4)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{}-{hex}", chrono::Utc::now().format("%Y%m%dT%H%M%S"))
}

pub fn is_job_id(id: &str) -> bool {
    let Some((ts, hex)) = id.rsplit_once('-') else {
        return false;
    };
    ts.len() == 15
        && ts.bytes().all(|b| b.is_ascii_digit() || b == b'T')
        && hex.len() == 8
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Delete undo directories older than [`UNDO_KEEP`].
pub fn prune_undo(undo_root: &Path) {
    let cutoff = SystemTime::now() - UNDO_KEEP;
    for e in std::fs::read_dir(undo_root).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_job_id(&name) {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t < cutoff);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

// ─────────────────────────────── jobs ────────────────────────────────

/// Where a job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Running,
    Done,
    Failed,
    Undone,
}

/// A data job as clients see it (`GET /api/data/jobs/{id}`): moves and
/// copies here, backups and restores later (the same model).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    /// `move` | `copy` | `backup` | `restore` | `pull`.
    pub kind: String,
    pub state: JobState,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Until when undo works (move/copy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_until: Option<i64>,
    /// The sessions it moved, copied or restored.
    pub sessions: Vec<String>,
    /// The account whose backup set it used (backup, restore, pull).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The accounts it moved or copied from and to (restore: `to` is the
    /// account restored into).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// The storage a move or copy came from or went to (docs/ACCOUNTS.md
    /// §13.4), and its set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_storage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_storage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Storage to storage: the target's set (`set` is the source's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_set: Option<String>,
    /// A per-kind result (backup: what was archived; restore: warnings…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

struct Job {
    view: Mutex<JobView>,
    progress: Arc<Progress>,
}

/// Running and recent data jobs (in memory; a move's undo survives a restart
/// in its manifest).
#[derive(Default)]
pub struct Jobs {
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
    /// One transfer at a time.
    pub lock: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Jobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jobs").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Jobs {
    /// Register a running job.
    pub fn start(&self, id: &str, kind: &str, files_total: u64, bytes_total: u64) -> Arc<Progress> {
        let progress = Arc::new(Progress::default());
        let job = Arc::new(Job {
            view: Mutex::new(JobView {
                id: id.to_string(),
                kind: kind.to_string(),
                state: JobState::Running,
                started_at: now_ms(),
                finished_at: None,
                files_done: 0,
                files_total,
                bytes_done: 0,
                bytes_total,
                error: None,
                error_code: None,
                undo_until: None,
                sessions: Vec::new(),
                account: None,
                from: None,
                to: None,
                from_storage: None,
                to_storage: None,
                set: None,
                to_set: None,
                result: None,
            }),
            progress: progress.clone(),
        });
        let mut jobs = lock(&self.jobs);
        // Keep the last 50.
        while jobs.len() >= 50 {
            let oldest = jobs.keys().next().cloned();
            match oldest {
                Some(k) => {
                    jobs.remove(&k);
                }
                None => break,
            }
        }
        jobs.insert(id.to_string(), job);
        progress
    }

    /// A running job's progress counters.
    pub fn progress(&self, id: &str) -> Option<Arc<Progress>> {
        lock(&self.jobs).get(id).map(|j| j.progress.clone())
    }

    /// Change a job's view.
    pub fn update(&self, id: &str, f: impl FnOnce(&mut JobView)) {
        if let Some(job) = lock(&self.jobs).get(id) {
            f(&mut lock(&job.view));
        }
    }

    pub fn get(&self, id: &str) -> Option<JobView> {
        let jobs = lock(&self.jobs);
        let job = jobs.get(id)?;
        let mut v = lock(&job.view).clone();
        if v.state == JobState::Running {
            v.files_done = job.progress.files_done.load(Ordering::Relaxed);
            v.bytes_done = job.progress.bytes_done.load(Ordering::Relaxed);
        }
        Some(v)
    }

    pub fn list(&self) -> Vec<JobView> {
        let ids: Vec<String> = lock(&self.jobs).keys().cloned().collect();
        ids.iter().rev().filter_map(|id| self.get(id)).collect()
    }
}

/// A finished move's view, from its manifest (after a restart).
pub fn view_from_manifest(m: &Manifest) -> JobView {
    JobView {
        id: m.job_id.clone(),
        kind: m.kind.clone().unwrap_or_else(|| {
            match m.mode {
                Mode::Move => "move",
                Mode::Copy => "copy",
            }
            .to_string()
        }),
        state: if m.undone_at.is_some() {
            JobState::Undone
        } else {
            JobState::Done
        },
        started_at: m.created_at,
        finished_at: Some(m.created_at),
        files_done: m.created.len() as u64,
        files_total: m.created.len() as u64,
        bytes_done: m.created.iter().map(|c| c.size).sum(),
        bytes_total: m.created.iter().map(|c| c.size).sum(),
        error: None,
        error_code: None,
        undo_until: m
            .undone_at
            .is_none()
            .then(|| m.created_at + UNDO_KEEP.as_millis() as i64),
        sessions: m.sessions.clone(),
        account: None,
        from: m.from_account.clone(),
        to: m.to_account.clone(),
        from_storage: m
            .storage
            .as_ref()
            .filter(|s| !s.into)
            .map(|s| s.storage.clone()),
        to_storage: m
            .storage
            .as_ref()
            .filter(|s| s.into)
            .or(m.storage_to.as_ref())
            .map(|s| s.storage.clone()),
        set: m.storage.as_ref().map(|s| s.set.clone()),
        to_set: m.storage_to.as_ref().map(|s| s.set.clone()),
        result: None,
    }
}

/// The manifest of a transfer, if its undo directory is still there.
pub fn manifest(undo_root: &Path, job_id: &str) -> Option<Manifest> {
    if !is_job_id(job_id) {
        return None;
    }
    crate::persist::read_json(&undo_root.join(job_id).join("manifest.json"))
}

/// The `projects/<enc>` folder name a cwd gets (for the UI's dry run).
pub fn project_folder(cwd: &str) -> String {
    encode_cwd(cwd)
}

pub mod custom;
pub mod moves;
pub mod ops;

#[cfg(test)]
mod tests;
