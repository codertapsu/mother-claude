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
//! themselves (renamed there, so a move costs no extra space on one volume).
//! Undo deletes what was created — refusing if any of it changed since — puts
//! the moved files back, and takes back exactly the `.claude.json` entry and the
//! `history.jsonl` lines that were added.

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
}

impl TransferError {
    pub fn code(&self) -> &'static str {
        match self {
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
    /// The project entry removed from the source's (a project move).
    pub removes_project_entry: Option<String>,
    /// Prompt-history lines appended to the target (a project).
    pub history_lines: usize,
    /// The target's `.claude.json` or `history.jsonl` is written.
    pub writes_shared: bool,
    #[serde(skip)]
    from: PathBuf,
    #[serde(skip)]
    to: PathBuf,
    #[serde(skip)]
    entry: Option<(String, Value)>,
    #[serde(skip)]
    history: Vec<String>,
    #[serde(skip)]
    snapshot: bool,
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
}

/// The transcripts a session id has under `projects/*/`.
fn transcripts_of(home: &ClaudeHome, id: &str) -> Vec<PathBuf> {
    let name = format!("{id}.jsonl");
    let mut out: Vec<PathBuf> = std::fs::read_dir(home.projects_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|d| d.path().join(&name))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out
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
fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
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
fn plan_session(
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
        if !d.is_dir() {
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
    if env.is_dir() {
        add_dir(&env, &mut files, &mut dirs)?;
    } else if env.is_file() {
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
fn project_entry(claude_json: &Path, cwd: &str) -> Option<Value> {
    let text = std::fs::read_to_string(claude_json).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("projects")?.get(cwd).cloned()
}

/// `history.jsonl` lines whose `project` is `cwd`.
fn history_lines(history: &Path, cwd: &str) -> Vec<String> {
    let Ok(file) = std::fs::File::open(history) else {
        return Vec::new();
    };
    std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
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
    for e in std::fs::read_dir(home.sessions_dir())
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
            return Some(TransferError::DirectoryInUse(
                home.base().display().to_string(),
            ));
        }
    }
    let lock = home.claude_json().with_extension("json.lock");
    let fresh = std::fs::metadata(&lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < LOCK_FRESH);
    fresh.then(|| TransferError::ConfigLocked(home.claude_json().display().to_string()))
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
    let mut sessions = Vec::new();
    for id in &ids {
        if !req.snapshot && (ctx.is_live)(id) {
            return Err(TransferError::SessionLive(id.clone()));
        }
        if !transcripts_of(ctx.to, id).is_empty() {
            return Err(TransferError::AlreadyExists(id.clone()));
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
    let removes = (req.mode == Mode::Move)
        .then_some(cwd.clone())
        .flatten()
        .filter(|c| project_entry(&ctx.from.claude_json(), c).is_some());
    let writes_shared = entry.is_some() || !history.is_empty();
    if writes_shared {
        if let Some(e) = in_use(ctx.to) {
            return Err(e);
        }
    }
    if removes.is_some() || (req.mode == Mode::Move && cwd.is_some()) {
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
        removes_project_entry: removes,
        history_lines: history.len(),
        writes_shared,
        from: ctx.from.base().to_path_buf(),
        to: ctx.to.base().to_path_buf(),
        entry,
        history,
        snapshot: req.snapshot,
    })
}

// ─────────────────────────────── running it ───────────────────────────────

/// A file the transfer created, as it was right after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
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
    pub sessions: Vec<String>,
    pub created: Vec<Created>,
    /// Directories created in the target (removed by undo when empty).
    pub created_dirs: Vec<PathBuf>,
    pub moved: Vec<Moved>,
    /// Source directories emptied by a move (re-created by undo).
    pub moved_dirs: Vec<PathBuf>,
    /// `(cwd, entry)` added to the target's `.claude.json`.
    pub added_entry: Option<(String, Value)>,
    /// `(cwd, entry)` removed from the source's `.claude.json`.
    pub removed_entry: Option<(String, Value)>,
    /// Lines appended to the target's `history.jsonl`.
    pub added_history: Vec<String>,
    /// Lines removed from the source's `history.jsonl`.
    pub removed_history: Vec<String>,
    #[serde(default)]
    pub undone_at: Option<i64>,
}

/// Progress of a running transfer.
#[derive(Debug, Default)]
pub struct Progress {
    pub files_done: AtomicU64,
    pub bytes_done: AtomicU64,
}

fn sha256_file(path: &Path) -> std::io::Result<(u64, String)> {
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
/// CLI is appending to).
fn trim_partial_line(path: &Path) -> std::io::Result<()> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() || bytes.ends_with(b"\n") {
        return Ok(());
    }
    let keep = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    std::fs::File::options()
        .write(true)
        .open(path)?
        .set_len(keep as u64)
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
    })
}

fn tmp_is_jsonl(dest: &Path) -> bool {
    dest.extension().is_some_and(|e| e == "jsonl")
}

/// Rewrite a JSON file atomically (temp + rename), keeping its mode.
fn rewrite_json(path: &Path, value: &Value) -> Result<(), TransferError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| TransferError::Io(e.to_string()))?;
    rewrite(path, &bytes)
}

fn rewrite(path: &Path, bytes: &[u8]) -> Result<(), TransferError> {
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
        let mut f = std::fs::File::create(&tmp).map_err(|e| io("writing", &tmp, e))?;
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
fn edit_projects(
    claude_json: &Path,
    f: impl FnOnce(&mut serde_json::Map<String, Value>),
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
    let projects = obj
        .entry("projects")
        .or_insert_with(|| Value::Object(Default::default()));
    let projects = projects.as_object_mut().ok_or_else(|| {
        TransferError::Io(format!("{} has no projects object", claude_json.display()))
    })?;
    f(projects);
    rewrite_json(claude_json, &root)
}

/// Append lines to a JSONL file (atomically: the whole file rewritten).
fn append_lines(path: &Path, lines: &[String]) -> Result<(), TransferError> {
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for l in lines {
        text.push_str(l);
        text.push('\n');
    }
    rewrite(path, text.as_bytes())
}

/// Remove exactly these lines (once each) from a JSONL file.
fn remove_lines(path: &Path, lines: &[String]) -> Result<(), TransferError> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let mut drop: Vec<&String> = lines.iter().collect();
    let mut out = String::new();
    for l in text.lines() {
        if let Some(i) = drop.iter().position(|d| d.as_str() == l) {
            drop.remove(i);
            continue;
        }
        out.push_str(l);
        out.push('\n');
    }
    rewrite(path, out.as_bytes())
}

/// Move a file into the undo directory: a rename on one volume, else a
/// copy and a delete.
fn stash(path: &Path, undo_dir: &Path, rel: &Path) -> Result<Moved, TransferError> {
    let dest = undo_dir.join("moved").join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io("creating", parent, e))?;
    }
    if std::fs::rename(path, &dest).is_err() {
        std::fs::copy(path, &dest).map_err(|e| io("saving", path, e))?;
        std::fs::remove_file(path).map_err(|e| io("removing", path, e))?;
    }
    Ok(Moved {
        original: path.to_path_buf(),
        saved: PathBuf::from("moved").join(rel),
    })
}

/// Run a planned transfer: undo copy first, then the files, then the shared
/// files, then (move mode) the source. Blocking. On failure everything
/// created so far is removed again and the source is left as it was.
pub fn run(
    plan: &Plan,
    undo_root: &Path,
    job_id: &str,
    progress: &Progress,
) -> Result<Manifest, TransferError> {
    let undo_dir = undo_root.join(job_id);
    std::fs::create_dir_all(&undo_dir).map_err(|e| io("creating", &undo_dir, e))?;
    private_dir(undo_root);
    private_dir(&undo_dir);
    let mut m = Manifest {
        job_id: job_id.to_string(),
        created_at: now_ms(),
        mode: plan.mode,
        what: plan.what.clone(),
        from_dir: plan.from.clone(),
        to_dir: plan.to.clone(),
        sessions: plan.sessions.iter().map(|s| s.id.clone()).collect(),
        created: Vec::new(),
        created_dirs: Vec::new(),
        moved: Vec::new(),
        moved_dirs: Vec::new(),
        added_entry: None,
        removed_entry: None,
        added_history: Vec::new(),
        removed_history: Vec::new(),
        undone_at: None,
    };
    let save = |m: &Manifest| -> Result<(), TransferError> {
        crate::persist::write_json_private(&undo_dir.join("manifest.json"), m)
            .map_err(|e| io("writing", &undo_dir, e))
    };
    if let Err(e) = save(&m) {
        let _ = std::fs::remove_dir_all(&undo_dir);
        return Err(e);
    }
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
                let created = copy_file(f, plan.snapshot)?;
                progress.files_done.fetch_add(1, Ordering::Relaxed);
                progress
                    .bytes_done
                    .fetch_add(created.size, Ordering::Relaxed);
                m.created.push(created);
            }
            save(&m)?;
        }
        // The target's shared files (trust opt-in; a project's history).
        let to = ClaudeHome::for_account_dir(&plan.to);
        if let Some((cwd, entry)) = &plan.entry {
            let mut added = false;
            edit_projects(&to.claude_json(), |p| {
                if !p.contains_key(cwd) {
                    p.insert(cwd.clone(), entry.clone());
                    added = true;
                }
            })?;
            if added {
                m.added_entry = Some((cwd.clone(), entry.clone()));
            }
            save(&m)?;
        }
        if !plan.history.is_empty() {
            append_lines(&to.history_file(), &plan.history)?;
            m.added_history = plan.history.clone();
            save(&m)?;
        }
        // Move mode: the source goes to the undo directory, only now that
        // every copy verified.
        if plan.mode == Mode::Move {
            for s in &plan.sessions {
                for f in &s.files {
                    let rel = f.from.strip_prefix(&plan.from).unwrap_or(&f.from);
                    m.moved.push(stash(&f.from, &undo_dir, rel)?);
                }
                for (dir, _) in s.dirs.iter().rev() {
                    if std::fs::remove_dir_all(dir).is_ok() {
                        m.moved_dirs.push(dir.clone());
                    }
                }
                save(&m)?;
            }
            if let (What::Project { cwd }, Some(_)) = (&plan.what, &plan.removes_project_entry) {
                let from = ClaudeHome::for_account_dir(&plan.from);
                let mut removed = None;
                edit_projects(&from.claude_json(), |p| removed = p.remove(cwd))?;
                m.removed_entry = removed.map(|e| (cwd.clone(), e));
                let lines = history_lines(&from.history_file(), cwd);
                if !lines.is_empty() {
                    remove_lines(&from.history_file(), &lines)?;
                    m.removed_history = lines;
                }
                save(&m)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = outcome {
        // Leave the source as it was and remove what was made.
        let _ = rollback(&m);
        let _ = std::fs::remove_dir_all(&undo_dir);
        return Err(e);
    }
    Ok(m)
}

/// Undo a finished transfer from its manifest.
pub fn undo(undo_root: &Path, job_id: &str) -> Result<Manifest, TransferError> {
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
    // Refuse when what was created has been used since (a continued
    // session): deleting it would lose that work.
    for c in &m.created {
        match sha256_file(&c.path) {
            Ok((size, sha)) if size == c.size && sha == c.sha256 => {}
            Ok(_) => {
                return Err(TransferError::UndoConflict(format!(
                    "{} changed after the transfer",
                    c.path.display()
                )))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io("reading", &c.path, e)),
        }
    }
    for mv in &m.moved {
        if mv.original.exists() {
            return Err(TransferError::UndoConflict(format!(
                "{} exists again",
                mv.original.display()
            )));
        }
    }
    rollback(&m)?;
    // Emptied source directories first (an empty `session-env/<id>` is one),
    // then the files back into them.
    for d in &m.moved_dirs {
        std::fs::create_dir_all(d).map_err(|e| io("creating", d, e))?;
    }
    for mv in &m.moved {
        let saved = undo_dir.join(&mv.saved);
        if let Some(parent) = mv.original.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io("creating", parent, e))?;
        }
        if std::fs::rename(&saved, &mv.original).is_err() {
            std::fs::copy(&saved, &mv.original).map_err(|e| io("restoring", &mv.original, e))?;
        }
    }
    if let Some((cwd, entry)) = &m.removed_entry {
        let from = ClaudeHome::for_account_dir(&m.from_dir);
        edit_projects(&from.claude_json(), |p| {
            p.entry(cwd.clone()).or_insert_with(|| entry.clone());
        })?;
    }
    if !m.removed_history.is_empty() {
        let from = ClaudeHome::for_account_dir(&m.from_dir);
        append_lines(&from.history_file(), &m.removed_history)?;
    }
    m.undone_at = Some(now_ms());
    crate::persist::write_json_private(&undo_dir.join("manifest.json"), &m)
        .map_err(|e| io("writing", &undo_dir, e))?;
    Ok(m)
}

/// Remove what a transfer created in the target (files, then the
/// directories it made when empty) and the shared-file changes it made there.
fn rollback(m: &Manifest) -> Result<(), TransferError> {
    for c in &m.created {
        match std::fs::remove_file(&c.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io("removing", &c.path, e)),
        }
    }
    for d in m.created_dirs.iter().rev() {
        // Deepest first (they were made outermost first); only what is empty
        // now, so anything written there since stays.
        let mut dirs = Vec::new();
        let _ = collect_dirs(d, &mut dirs);
        for sub in dirs.iter().rev() {
            let _ = std::fs::remove_dir(sub);
        }
        let _ = std::fs::remove_dir(d);
    }
    let to = ClaudeHome::for_account_dir(&m.to_dir);
    if let Some((cwd, entry)) = &m.added_entry {
        edit_projects(&to.claude_json(), |p| {
            if p.get(cwd) == Some(entry) {
                p.remove(cwd);
            }
        })?;
    }
    if !m.added_history.is_empty() {
        remove_lines(&to.history_file(), &m.added_history)?;
    }
    Ok(())
}

/// The directories from `stop` (exclusive, which exists) down to `dir` that
/// do not exist yet, outermost first.
fn missing_dirs(dir: &Path, stop: &Path) -> Vec<PathBuf> {
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
fn private_dir(dir: &Path) {
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
        kind: match m.mode {
            Mode::Move => "move",
            Mode::Copy => "copy",
        }
        .to_string(),
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

pub mod ops;

#[cfg(test)]
mod tests;
