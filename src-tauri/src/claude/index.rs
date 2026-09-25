//! Incremental transcript index (design §3.2, amended §10 B2).
//!
//! One [`FileIndex`] per transcript file — main transcripts
//! (`projects/<enc>/<sid>.jsonl`) and every subagent / workflow-agent file
//! (`projects/<enc>/<sid>/subagents/**/agent-*.jsonl`). Each remembers its
//! inode, size, mtime and the byte offset just past the last complete line,
//! so a refresh never re-reads unchanged bytes.
//!
//! * **Location by session id**: discovery lists `projects/*/` and maps every
//!   `<uuid>.jsonl` and `<uuid>/` directory to its session, independent of the
//!   (drifting) cwd. When a session has transcripts in several project dirs,
//!   the largest is the main one; subagents are gathered from all of them.
//! * **Bootstrap**: live transcripts larger than 8 MB start with a tail-window
//!   scan (the last 8 MB: model, titles, pending tools, todos, context…); a
//!   background worker (concurrency 1) then runs the full pass for lifetime
//!   aggregates and swaps it in. Historical files are indexed by the same
//!   worker, most recent first.
//! * **Streaming**: lines are read one at a time with a bounded buffer and
//!   parsed into typed records that skip everything they do not need
//!   (`loose.rs`); lines up to 64 MB are parsed, longer ones skipped.
//! * **Checkpoints** (`index-cache.json` in the app data dir, 0600, written
//!   atomically every 30 s when dirty) hold `{path, inode, size, offset,
//!   aggregates}` of fully indexed files, so a restart resumes where it left
//!   off instead of rescanning ~1 GB.
//!
//! AppState field: `state.index`. All methods that touch the disk are
//! blocking: call them from `spawn_blocking` (the monitor does).

pub mod agg;
pub mod loose;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub use agg::{
    classify_path, stamp_of, AgentCall, Aggregates, BgTask, FileIndex, FileKind, LastAct, Mark,
    MarkKind, PendingTool, QueuedPrompt, Stamp, Totals,
};

use super::home::ClaudeHome;
use super::ids;

/// Live transcripts larger than this start from a tail window of this size.
pub const TAIL_WINDOW: u64 = 8 * 1024 * 1024;
/// A monitor tick scans at most this much of one file (the rest next tick).
pub const TICK_SCAN_MAX: u64 = 64 * 1024 * 1024;
/// A checkpoint further behind than this is caught up by the worker.
const CATCH_UP_INLINE_MAX: u64 = 32 * 1024 * 1024;
/// Subagent files touched this recently are indexed immediately.
const RECENT_SUBAGENT: Duration = Duration::from_secs(15 * 60);
/// Bumped when checkpointed aggregates change meaning, so finished files
/// (never re-scanned otherwise) are rebuilt once. 3: `UsageAcc::last_fold`
/// and `UsageAcc::recent`.
const CACHE_VERSION: u32 = 3;
const PERSIST_EVERY: Duration = Duration::from_secs(30);

/// A subagent / workflow-agent transcript of a session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubagentFile {
    pub agent_id: String,
    pub path: PathBuf,
    /// `agent-<id>.meta.json` next to it.
    pub meta: PathBuf,
    /// `wf_…` for workflow agents.
    pub run_id: Option<String>,
    pub mtime: i64,
    pub size: u64,
}

/// Everything discovery found for one session id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionFiles {
    pub id: String,
    /// The main transcript (largest candidate).
    pub main: Option<PathBuf>,
    pub main_size: u64,
    pub main_mtime: i64,
    /// `projects/<enc>` holding the main transcript.
    pub project_dir: Option<PathBuf>,
    /// `projects/<enc>/<sid>/` dirs (subagents, workflows, tool-results).
    pub session_dirs: Vec<PathBuf>,
    /// By agent id.
    pub subagents: BTreeMap<String, SubagentFile>,
    /// `subagents/workflows/<run>/journal.jsonl` by run id.
    pub journals: BTreeMap<String, PathBuf>,
    /// `workflows/<run>.json` (written when a run finishes) by run id.
    pub run_files: BTreeMap<String, PathBuf>,
}

/// New bytes consumed from a file by [`TranscriptIndex::update`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advance {
    pub session_id: String,
    /// `None` = the main transcript.
    pub agent_id: Option<String>,
    pub path: PathBuf,
    pub from: u64,
    pub to: u64,
}

/// A queued background indexing job.
#[derive(Debug, Clone)]
pub struct Job {
    pub path: PathBuf,
    pub kind: FileKind,
    pub session_id: String,
    /// A checkpoint to continue from.
    pub base: Option<FileIndex>,
    prio: u8,
    mtime: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CacheFile {
    version: u32,
    files: Vec<FileIndex>,
}

type Shared = Arc<Mutex<FileIndex>>;

#[derive(Debug, Default)]
struct Idx {
    cache_loaded: bool,
    /// Checkpoints not yet adopted.
    cache: HashMap<PathBuf, FileIndex>,
    sessions: HashMap<String, SessionFiles>,
    files: HashMap<PathBuf, Shared>,
    jobs: Vec<Job>,
    queued: HashSet<PathBuf>,
    dirty: bool,
    last_persist: Option<Instant>,
}

/// The index.
#[derive(Debug, Default)]
pub struct TranscriptIndex {
    /// `<app data dir>/index-cache.json`, or `None` (tests: in-memory).
    pub cache_path: Option<PathBuf>,
    inner: Mutex<Idx>,
    notify: tokio::sync::Notify,
}

fn lock_file(f: &Shared) -> MutexGuard<'_, FileIndex> {
    f.lock().unwrap_or_else(|p| p.into_inner())
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    stamp_of(meta).mtime
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Run a job to completion (blocking): continue from the checkpoint, or scan
/// from the start.
pub fn run_job(job: &Job) -> FileIndex {
    let mut fi = job
        .base
        .clone()
        .unwrap_or_else(|| FileIndex::new(job.path.clone(), job.kind, &job.session_id));
    if let Err(e) = fi.scan(None) {
        tracing::debug!(error = %e, path = %job.path.display(), "indexing failed");
    }
    fi
}

impl TranscriptIndex {
    /// Tolerant constructor; `None` ⇒ no persisted checkpoints. Nothing is
    /// read here: the checkpoint file is loaded by [`Self::load_cache`].
    pub fn new(data_dir: Option<&Path>) -> Self {
        Self {
            cache_path: data_dir.map(|d| d.join("index-cache.json")),
            ..Default::default()
        }
    }

    fn lock(&self) -> MutexGuard<'_, Idx> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Load `index-cache.json` once (tolerant: missing/corrupt ⇒ empty).
    pub fn load_cache(&self) {
        if self.lock().cache_loaded {
            return;
        }
        let mut cache = HashMap::new();
        if let Some(path) = &self.cache_path {
            match std::fs::read(path) {
                Ok(bytes) => match serde_json::from_slice::<CacheFile>(&bytes) {
                    Ok(c) if c.version == CACHE_VERSION => {
                        for f in c.files {
                            cache.insert(f.path.clone(), f);
                        }
                    }
                    Ok(_) => tracing::info!("index cache from another version ignored"),
                    Err(e) => tracing::warn!(error = %e, "index cache unreadable; rebuilding"),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!(error = %e, "index cache unreadable; rebuilding"),
            }
        }
        let mut g = self.lock();
        g.cache_loaded = true;
        g.cache = cache;
    }

    /// The main transcript of a session, located by id (independent of cwd).
    pub fn transcript_path(&self, session_id: &str) -> Option<PathBuf> {
        self.lock()
            .sessions
            .get(session_id)
            .and_then(|s| s.main.clone())
    }

    /// Every session id discovery knows.
    pub fn session_ids(&self) -> Vec<String> {
        self.lock().sessions.keys().cloned().collect()
    }

    /// What discovery found for a session.
    pub fn session_files(&self, session_id: &str) -> Option<SessionFiles> {
        self.lock().sessions.get(session_id).cloned()
    }

    /// Paths of indexed (adopted) files of a session.
    pub fn indexed_paths(&self, session_id: &str) -> Vec<PathBuf> {
        let g = self.lock();
        let Some(s) = g.sessions.get(session_id) else {
            return Vec::new();
        };
        s.main
            .iter()
            .chain(s.subagents.values().map(|a| &a.path))
            .filter(|p| g.files.contains_key(*p))
            .cloned()
            .collect()
    }

    fn shared(&self, path: &Path) -> Option<Shared> {
        self.lock().files.get(path).cloned()
    }

    /// Read a file's index under its lock.
    pub fn with_file<R>(&self, path: &Path, f: impl FnOnce(&FileIndex) -> R) -> Option<R> {
        let shared = self.shared(path)?;
        let g = lock_file(&shared);
        Some(f(&g))
    }

    /// Read a session's main index.
    pub fn with_main<R>(&self, session_id: &str, f: impl FnOnce(&FileIndex) -> R) -> Option<R> {
        let path = self.transcript_path(session_id)?;
        self.with_file(&path, f)
    }

    /// Read one subagent's index.
    pub fn with_subagent<R>(
        &self,
        session_id: &str,
        agent_id: &str,
        f: impl FnOnce(&FileIndex) -> R,
    ) -> Option<R> {
        let path = self
            .lock()
            .sessions
            .get(session_id)?
            .subagents
            .get(agent_id)?
            .path
            .clone();
        self.with_file(&path, f)
    }

    /// The transcript file of a subagent (validated id; located via
    /// discovery, never built from the id).
    pub fn subagent_path(&self, session_id: &str, agent_id: &str) -> Option<PathBuf> {
        if !ids::is_agent_id(agent_id) {
            return None;
        }
        self.lock()
            .sessions
            .get(session_id)?
            .subagents
            .get(agent_id)
            .map(|a| a.path.clone())
    }

    /// Background jobs still queued (0 once the bootstrap finished).
    pub fn pending_jobs(&self) -> usize {
        self.lock().jobs.len()
    }

    // ─────────────────────────────── discovery ──────────────────────────────

    /// List `projects/` and rebuild the session → files map. Returns the ids
    /// of sessions whose files changed. Blocking.
    pub fn discover(&self, home: &ClaudeHome) -> Vec<String> {
        let projects = home.projects_dir();
        let mut found: HashMap<String, SessionFiles> = HashMap::new();
        let Ok(dirs) = std::fs::read_dir(&projects) else {
            return Vec::new();
        };
        let mut session_dirs: Vec<(String, PathBuf)> = Vec::new();
        for dir in dirs.flatten() {
            let Ok(ft) = dir.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let project_dir = dir.path();
            let Ok(entries) = std::fs::read_dir(&project_dir) else {
                continue;
            };
            for e in entries.flatten() {
                let name = e.file_name();
                let Some(name) = name.to_str() else { continue };
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_file() {
                    let Some(sid) = name.strip_suffix(".jsonl") else {
                        continue;
                    };
                    if !ids::is_session_id(sid) {
                        continue;
                    }
                    let Ok(meta) = e.metadata() else { continue };
                    let entry = found
                        .entry(sid.to_string())
                        .or_insert_with(|| SessionFiles {
                            id: sid.to_string(),
                            ..Default::default()
                        });
                    if entry.main.is_none() || meta.len() > entry.main_size {
                        entry.main = Some(e.path());
                        entry.main_size = meta.len();
                        entry.main_mtime = mtime_ms(&meta);
                        entry.project_dir = Some(project_dir.clone());
                    }
                } else if ft.is_dir() && ids::is_session_id(name) {
                    session_dirs.push((name.to_string(), e.path()));
                }
            }
        }
        for (sid, dir) in session_dirs {
            let entry = found.entry(sid.clone()).or_insert_with(|| SessionFiles {
                id: sid.clone(),
                ..Default::default()
            });
            scan_session_dir(entry, &dir);
            entry.session_dirs.push(dir);
        }
        let mut g = self.lock();
        let mut changed: Vec<String> = found
            .iter()
            .filter(|(sid, s)| g.sessions.get(*sid) != Some(*s))
            .map(|(sid, _)| sid.clone())
            .collect();
        changed.extend(
            g.sessions
                .keys()
                .filter(|sid| !found.contains_key(*sid))
                .cloned(),
        );
        // Forget files that disappeared.
        let known: HashSet<PathBuf> = found
            .values()
            .flat_map(|s| {
                s.main
                    .iter()
                    .cloned()
                    .chain(s.subagents.values().map(|a| a.path.clone()))
            })
            .collect();
        g.files.retain(|p, _| known.contains(p));
        // Checkpoints of transcripts that are gone (Claude Code prunes them
        // after 30 days) would otherwise be carried into every cache write.
        let cached = g.cache.len();
        g.cache.retain(|p, _| known.contains(p));
        if g.cache.len() != cached {
            g.dirty = true;
        }
        g.jobs.retain(|j| known.contains(&j.path));
        let jobs_left: HashSet<PathBuf> = g.jobs.iter().map(|j| j.path.clone()).collect();
        g.queued = jobs_left;
        g.sessions = found;
        changed.sort();
        changed
    }

    /// Re-list one session's directories (after a watcher event named a file
    /// we did not know). Blocking.
    pub fn rediscover_session(&self, session_id: &str) {
        let dirs = self
            .lock()
            .sessions
            .get(session_id)
            .map(|s| s.session_dirs.clone())
            .unwrap_or_default();
        let mut updated = match self.lock().sessions.get(session_id).cloned() {
            Some(s) => s,
            None => return,
        };
        updated.subagents.clear();
        updated.journals.clear();
        updated.run_files.clear();
        for d in &dirs {
            scan_session_dir(&mut updated, d);
        }
        self.lock().sessions.insert(session_id.to_string(), updated);
    }

    // ─────────────────────────────── adoption ───────────────────────────────

    /// Make sure a session's files are indexed or queued. `live` sessions get
    /// their main transcript indexed now (tail window when large) and recent
    /// subagent files scanned immediately; everything else goes to the
    /// background worker. Blocking.
    pub fn ensure(&self, session_id: &str, live: bool) {
        let Some(files) = self.session_files(session_id) else {
            return;
        };
        let now = now_ms();
        if let Some(main) = &files.main {
            self.ensure_file(main, FileKind::Main, session_id, live, files.main_mtime);
        }
        for a in files.subagents.values() {
            let recent = now - a.mtime < RECENT_SUBAGENT.as_millis() as i64;
            self.ensure_file(
                &a.path,
                FileKind::Subagent,
                session_id,
                live && recent,
                a.mtime,
            );
        }
    }

    /// Queue every discovered file that is neither indexed nor queued
    /// (history bootstrap). Recent files first.
    pub fn queue_all(&self) {
        let sessions: Vec<SessionFiles> = self.lock().sessions.values().cloned().collect();
        for s in sessions {
            if let Some(main) = &s.main {
                self.ensure_file(main, FileKind::Main, &s.id, false, s.main_mtime);
            }
            for a in s.subagents.values() {
                self.ensure_file(&a.path, FileKind::Subagent, &s.id, false, a.mtime);
            }
        }
    }

    fn ensure_file(&self, path: &Path, kind: FileKind, sid: &str, now: bool, mtime: i64) {
        let checkpoint = {
            let mut g = self.lock();
            if g.files.contains_key(path) {
                return;
            }
            if g.queued.contains(path) && !now {
                return;
            }
            g.cache.remove(path)
        };
        let Ok(meta) = std::fs::metadata(path) else {
            return;
        };
        let st = stamp_of(&meta);
        // A checkpoint is usable when the file is the same inode and only grew.
        let checkpoint =
            checkpoint.filter(|c| (c.ino == 0 || c.ino == st.ino) && c.offset <= st.size);
        let prio = if now {
            0
        } else if now_ms() - mtime < 24 * 3600 * 1000 {
            1
        } else {
            2
        };
        match checkpoint {
            Some(mut c) if st.size - c.offset <= CATCH_UP_INLINE_MAX || !now => {
                if st.size - c.offset <= CATCH_UP_INLINE_MAX {
                    // Bytes written while we were away are consumed by the
                    // next `update` (reported as one advance).
                    c.ui_cursor = c.offset;
                    self.adopt(c);
                } else {
                    self.queue(Job {
                        path: path.to_path_buf(),
                        kind,
                        session_id: sid.to_string(),
                        base: Some(c),
                        prio,
                        mtime,
                    });
                }
            }
            Some(c) => {
                // Far behind and live: tail window now, catch-up in background.
                self.bootstrap_window(path, kind, sid, st.size);
                self.queue(Job {
                    path: path.to_path_buf(),
                    kind,
                    session_id: sid.to_string(),
                    base: Some(c),
                    prio: 0,
                    mtime,
                });
            }
            None if now => {
                if st.size > TAIL_WINDOW {
                    self.bootstrap_window(path, kind, sid, st.size);
                    self.queue(Job {
                        path: path.to_path_buf(),
                        kind,
                        session_id: sid.to_string(),
                        base: None,
                        prio: 0,
                        mtime,
                    });
                } else {
                    let mut fi = FileIndex::new(path.to_path_buf(), kind, sid);
                    if let Err(e) = fi.scan(None) {
                        tracing::debug!(error = %e, path = %path.display(), "indexing failed");
                    }
                    fi.ui_cursor = fi.offset;
                    self.adopt(fi);
                }
            }
            None => self.queue(Job {
                path: path.to_path_buf(),
                kind,
                session_id: sid.to_string(),
                base: None,
                prio,
                mtime,
            }),
        }
    }

    fn bootstrap_window(&self, path: &Path, kind: FileKind, sid: &str, size: u64) {
        // `size` is only informative: the scan reads up to the current EOF.
        let mut fi = FileIndex::new(path.to_path_buf(), kind, sid);
        if let Err(e) = fi.seek_window(TAIL_WINDOW) {
            tracing::debug!(error = %e, path = %path.display(), "tail window failed");
        }
        if let Err(e) = fi.scan(None) {
            tracing::debug!(error = %e, path = %path.display(), "tail window scan failed");
        }
        let _ = size;
        fi.ui_cursor = fi.offset;
        self.adopt(fi);
    }

    fn adopt(&self, fi: FileIndex) {
        let mut g = self.lock();
        g.dirty = true;
        if fi.is_complete() && g.queued.remove(&fi.path) {
            g.jobs.retain(|j| j.path != fi.path);
        }
        g.files.insert(fi.path.clone(), Arc::new(Mutex::new(fi)));
    }

    fn queue(&self, job: Job) {
        let mut g = self.lock();
        if g.queued.insert(job.path.clone()) {
            g.jobs.push(job);
        } else if job.prio == 0 {
            if let Some(j) = g.jobs.iter_mut().find(|j| j.path == job.path) {
                j.prio = 0;
            }
        }
        drop(g);
        self.notify.notify_one();
    }

    /// The next background job (highest priority, most recent first).
    pub fn next_job(&self) -> Option<Job> {
        let mut g = self.lock();
        let idx = g
            .jobs
            .iter()
            .enumerate()
            .min_by_key(|(_, j)| (j.prio, std::cmp::Reverse(j.mtime)))
            .map(|(i, _)| i)?;
        Some(g.jobs.swap_remove(idx))
    }

    /// Wait until a job may be available.
    pub async fn job_notified(&self) {
        self.notify.notified().await;
    }

    /// Install a finished background job, replacing a tail-window entry (its
    /// publish cursor is carried over so live appends are not re-sent).
    pub fn install(&self, mut fi: FileIndex) {
        let path = fi.path.clone();
        let existing = {
            let mut g = self.lock();
            g.queued.remove(&path);
            g.dirty = true;
            g.files.get(&path).cloned()
        };
        match existing {
            Some(shared) => {
                let mut cur = lock_file(&shared);
                if cur.is_complete() && cur.offset >= fi.offset && cur.ino == fi.ino {
                    return; // already at least as far along
                }
                fi.ui_cursor = cur.ui_cursor.max(cur.offset);
                *cur = fi;
            }
            None => {
                fi.ui_cursor = fi.offset;
                self.lock().files.insert(path, Arc::new(Mutex::new(fi)));
            }
        }
    }

    /// Run every queued job inline (tests and `monitor::refresh_once`).
    pub fn drain_jobs_blocking(&self) {
        while let Some(job) = self.next_job() {
            let fi = run_job(&job);
            self.install(fi);
        }
    }

    // ─────────────────────────────── updates ────────────────────────────────

    /// Scan the given files (only adopted ones; unchanged files cost a
    /// `stat`). Returns the byte ranges newly consumed. Blocking.
    pub fn update(&self, paths: &[PathBuf]) -> Vec<Advance> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for path in paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            let Some(shared) = self.shared(path) else {
                continue;
            };
            let Ok(meta) = std::fs::metadata(path) else {
                continue;
            };
            let st = stamp_of(&meta);
            let mut fi = lock_file(&shared);
            if st.ino == fi.ino
                && st.size == fi.size
                && st.mtime == fi.mtime
                && st.size == fi.offset
            {
                continue;
            }
            match fi.scan(Some(TICK_SCAN_MAX)) {
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!(error = %e, path = %path.display(), "tail scan failed");
                    continue;
                }
            }
            if fi.ui_cursor > st.size {
                fi.ui_cursor = fi.offset; // file was truncated/replaced
            }
            if fi.offset > fi.ui_cursor {
                out.push(Advance {
                    session_id: fi.session_id.clone(),
                    agent_id: fi.agent_id.clone(),
                    path: path.clone(),
                    from: fi.ui_cursor,
                    to: fi.offset,
                });
                fi.ui_cursor = fi.offset;
            }
            drop(fi);
            self.lock().dirty = true;
        }
        out
    }

    /// The files a refresh tick should stat for these sessions: their main
    /// transcripts plus the files written within `recent_ms` (all files when
    /// `recent_ms` is `None`).
    pub fn tick_paths(
        &self,
        sessions: &[String],
        now_ms: i64,
        recent_ms: Option<i64>,
    ) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for sid in sessions {
            for p in self.indexed_paths(sid) {
                let keep = match recent_ms {
                    None => true,
                    Some(window) => self
                        .with_file(&p, |f| {
                            f.kind == FileKind::Main || now_ms - f.mtime < window
                        })
                        .unwrap_or(false),
                };
                if keep {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Every adopted file of the given sessions (for periodic stat sweeps).
    pub fn files_of(&self, sessions: &[String]) -> Vec<PathBuf> {
        sessions
            .iter()
            .flat_map(|s| self.indexed_paths(s))
            .collect()
    }

    // ────────────────────────────── persistence ─────────────────────────────

    /// Write `index-cache.json` when dirty and due (or `force`). Only complete
    /// files are checkpointed. Blocking.
    pub fn persist(&self, force: bool) -> bool {
        let Some(path) = self.cache_path.clone() else {
            return false;
        };
        let shared: Vec<Shared> = {
            let mut g = self.lock();
            let due = g.last_persist.is_none_or(|t| t.elapsed() >= PERSIST_EVERY);
            if !g.dirty || (!force && !due) {
                return false;
            }
            g.dirty = false;
            g.last_persist = Some(Instant::now());
            g.files.values().cloned().collect()
        };
        let recent_from = now_ms() - agg::RECENT_KEEP_MS;
        let mut files: Vec<FileIndex> = shared
            .iter()
            .map(|s| lock_file(s).clone())
            .filter(|f| f.is_complete())
            .map(|mut f| {
                f.agg.usage.compact();
                f.agg.usage.forget_recent_before(recent_from);
                if f.kind == FileKind::Subagent {
                    // Not shown for subagents; keeps the checkpoint small.
                    f.agg.last_prompt = None;
                    f.agg.last_text = None;
                }
                f
            })
            .collect();
        // Keep unadopted checkpoints too (history not yet touched this run).
        {
            let g = self.lock();
            let adopted: HashSet<&PathBuf> = files.iter().map(|f| &f.path).collect();
            let extra: Vec<FileIndex> = g
                .cache
                .values()
                .filter(|c| !adopted.contains(&c.path))
                .cloned()
                .collect();
            drop(adopted);
            files.extend(extra);
        }
        let body = CacheFile {
            version: CACHE_VERSION,
            files,
        };
        match crate::persist::write_json_private(&path, &body) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "could not write the index cache");
                self.lock().dirty = true;
                false
            }
        }
    }
}

/// Fill `entry` from one `projects/<enc>/<sid>/` directory.
fn scan_session_dir(entry: &mut SessionFiles, dir: &Path) {
    let subagents = dir.join("subagents");
    for e in std::fs::read_dir(&subagents)
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_file() {
            add_agent_file(entry, &e.path(), name, None);
        } else if ft.is_dir() && name == "workflows" {
            for run in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
                let run_name = run.file_name();
                let Some(run_id) = run_name.to_str() else {
                    continue;
                };
                if !ids::is_run_id(run_id) {
                    continue;
                }
                for f in std::fs::read_dir(run.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    let fname = f.file_name();
                    let Some(fname) = fname.to_str() else {
                        continue;
                    };
                    if fname == "journal.jsonl" {
                        entry.journals.insert(run_id.to_string(), f.path());
                    } else {
                        add_agent_file(entry, &f.path(), fname, Some(run_id));
                    }
                }
            }
        }
    }
    for e in std::fs::read_dir(dir.join("workflows"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(run) = name.strip_suffix(".json") {
            if ids::is_run_id(run) {
                entry.run_files.insert(run.to_string(), e.path());
            }
        }
    }
}

fn add_agent_file(entry: &mut SessionFiles, path: &Path, name: &str, run: Option<&str>) {
    let Some(agent) = name
        .strip_prefix("agent-")
        .and_then(|n| n.strip_suffix(".jsonl"))
    else {
        return;
    };
    if !ids::is_agent_id(agent) {
        return;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    entry.subagents.insert(
        agent.to_string(),
        SubagentFile {
            agent_id: agent.to_string(),
            path: path.to_path_buf(),
            meta: path.with_file_name(format!("agent-{agent}.meta.json")),
            run_id: run.map(str::to_string),
            mtime: mtime_ms(&meta),
            size: meta.len(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SID: &str = "e2b6972f-7126-4f6e-a490-5ce75cb6a9f9";

    fn asst(id: &str, out: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-09-25T10:00:00.000Z","cwd":"/p/app","message":{{"id":"{id}","model":"claude-haiku-4-5","content":[{{"type":"text","text":"hi"}}],"usage":{{"input_tokens":1,"output_tokens":{out}}}}}}}"#
        ) + "\n"
    }

    fn layout(base: &Path) -> (PathBuf, PathBuf) {
        let proj = base.join("projects/-p-app");
        let sdir = proj.join(SID);
        std::fs::create_dir_all(sdir.join("subagents/workflows/wf_abc-1")).unwrap();
        std::fs::create_dir_all(sdir.join("workflows")).unwrap();
        let main = proj.join(format!("{SID}.jsonl"));
        std::fs::write(&main, asst("m1", 10)).unwrap();
        std::fs::write(sdir.join("subagents/agent-a1.jsonl"), asst("s1", 3)).unwrap();
        std::fs::write(sdir.join("subagents/agent-a1.meta.json"), "{}").unwrap();
        std::fs::write(
            sdir.join("subagents/workflows/wf_abc-1/agent-a2.jsonl"),
            asst("s2", 4),
        )
        .unwrap();
        std::fs::write(sdir.join("subagents/workflows/wf_abc-1/journal.jsonl"), "").unwrap();
        std::fs::write(sdir.join("workflows/wf_abc-1.json"), "{}").unwrap();
        // Noise that must be ignored.
        std::fs::write(proj.join("agent-old.jsonl"), asst("x", 1)).unwrap();
        std::fs::write(proj.join("not-a-session.jsonl"), asst("x", 1)).unwrap();
        (main, sdir)
    }

    #[test]
    fn discovers_by_session_id_and_indexes() {
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let (main, _) = layout(tmp.path());
        let idx = TranscriptIndex::new(None);
        let changed = idx.discover(&home);
        assert_eq!(changed, vec![SID.to_string()]);
        let files = idx.session_files(SID).unwrap();
        assert_eq!(files.main.as_deref(), Some(main.as_path()));
        assert_eq!(files.subagents.len(), 2);
        assert_eq!(files.subagents["a2"].run_id.as_deref(), Some("wf_abc-1"));
        assert!(files.journals.contains_key("wf_abc-1"));
        assert!(files.run_files.contains_key("wf_abc-1"));
        assert_eq!(idx.session_ids().len(), 1);
        // Unchanged rediscovery reports nothing.
        assert!(idx.discover(&home).is_empty());

        // History bootstrap via the worker queue.
        idx.queue_all();
        assert_eq!(idx.pending_jobs(), 3);
        idx.drain_jobs_blocking();
        assert_eq!(idx.pending_jobs(), 0);
        let out = idx.with_main(SID, |f| f.agg.usage.total().output).unwrap();
        assert_eq!(out, 10);
        assert_eq!(
            idx.with_subagent(SID, "a2", |f| f.agg.usage.total().output),
            Some(4)
        );

        // Appends are consumed incrementally and reported once.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&main)
            .unwrap();
        f.write_all(asst("m2", 5).as_bytes()).unwrap();
        f.flush().unwrap();
        let adv = idx.update(std::slice::from_ref(&main));
        assert_eq!(adv.len(), 1);
        assert_eq!(adv[0].session_id, SID);
        assert!(adv[0].to > adv[0].from);
        assert!(idx.update(std::slice::from_ref(&main)).is_empty());
        assert_eq!(
            idx.with_main(SID, |f| f.agg.usage.total().messages),
            Some(2)
        );
    }

    #[test]
    fn cwd_drift_picks_the_largest_main_transcript() {
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let a = tmp.path().join("projects/-old-dir");
        let b = tmp.path().join("projects/-new-dir");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join(format!("{SID}.jsonl")), asst("m1", 1)).unwrap();
        std::fs::write(
            b.join(format!("{SID}.jsonl")),
            asst("m1", 1) + &asst("m2", 1),
        )
        .unwrap();
        std::fs::create_dir_all(a.join(SID).join("subagents")).unwrap();
        std::fs::write(a.join(SID).join("subagents/agent-z9.jsonl"), asst("s", 1)).unwrap();
        let idx = TranscriptIndex::new(None);
        idx.discover(&home);
        let files = idx.session_files(SID).unwrap();
        assert_eq!(
            files.main.as_deref(),
            Some(b.join(format!("{SID}.jsonl")).as_path())
        );
        // Subagents are found in the other dir.
        assert!(files.subagents.contains_key("z9"));
        assert_eq!(
            idx.transcript_path(SID),
            Some(b.join(format!("{SID}.jsonl")))
        );
    }

    #[test]
    fn live_large_files_bootstrap_from_a_tail_window_then_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let proj = tmp.path().join("projects/-p");
        std::fs::create_dir_all(&proj).unwrap();
        let main = proj.join(format!("{SID}.jsonl"));
        // ~9 MB: bigger than the tail window.
        let line = asst("m", 1);
        let mut text = String::with_capacity(9 * 1024 * 1024 + line.len());
        let mut i = 0;
        while (text.len() as u64) < TAIL_WINDOW + 1024 * 1024 {
            text += &asst(&format!("m{i}"), 1);
            i += 1;
        }
        std::fs::write(&main, &text).unwrap();
        let idx = TranscriptIndex::new(None);
        idx.discover(&home);
        idx.ensure(SID, true);
        let (complete, partial_msgs) = idx
            .with_main(SID, |f| (f.is_complete(), f.agg.usage.total().messages))
            .unwrap();
        assert!(!complete);
        assert!(partial_msgs < i as u64);
        assert_eq!(idx.pending_jobs(), 1);
        idx.drain_jobs_blocking();
        let (complete, msgs) = idx
            .with_main(SID, |f| (f.is_complete(), f.agg.usage.total().messages))
            .unwrap();
        assert!(complete);
        assert_eq!(msgs, i as u64);
        // Nothing new: no advance even though the index was replaced.
        assert!(idx.update(std::slice::from_ref(&main)).is_empty());
    }

    #[test]
    fn checkpoints_persist_and_resume_without_rescanning() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let (main, _) = layout(tmp.path());
        {
            let idx = TranscriptIndex::new(Some(data.path()));
            idx.load_cache();
            idx.discover(&home);
            idx.queue_all();
            idx.drain_jobs_blocking();
            assert!(idx.persist(true));
        }
        let cache = data.path().join("index-cache.json");
        assert!(cache.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&cache).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // Append, then restart: the checkpoint is adopted and only the new
        // bytes are read.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&main)
            .unwrap();
        f.write_all(asst("m2", 7).as_bytes()).unwrap();
        f.flush().unwrap();
        let idx = TranscriptIndex::new(Some(data.path()));
        idx.load_cache();
        idx.discover(&home);
        idx.ensure(SID, false);
        // Adopted from the checkpoint (no job needed for the main file).
        let before = idx.with_main(SID, |f| f.offset).unwrap();
        assert!(before > 0);
        let adv = idx.update(std::slice::from_ref(&main));
        assert_eq!(adv.len(), 1, "only the appended line is new");
        assert_eq!(idx.with_main(SID, |f| f.agg.usage.total().output), Some(17));
        // A corrupt cache is ignored.
        std::fs::write(&cache, "{not json").unwrap();
        let idx = TranscriptIndex::new(Some(data.path()));
        idx.load_cache();
        idx.discover(&home);
        idx.ensure(SID, false);
        assert!(idx.pending_jobs() > 0);
    }

    #[test]
    fn checkpoints_of_pruned_transcripts_are_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let (main, sdir) = layout(tmp.path());
        {
            let idx = TranscriptIndex::new(Some(data.path()));
            idx.load_cache();
            idx.discover(&home);
            idx.queue_all();
            idx.drain_jobs_blocking();
            assert!(idx.persist(true));
        }
        let cache = data.path().join("index-cache.json");
        let cached = || -> Vec<PathBuf> {
            let c: CacheFile = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
            c.files.into_iter().map(|f| f.path).collect()
        };
        assert!(cached().contains(&main));
        // Claude Code prunes the transcript; the next run drops its
        // checkpoints on the first discovery and rewrites the cache.
        std::fs::remove_file(&main).unwrap();
        std::fs::remove_dir_all(&sdir).unwrap();
        let idx = TranscriptIndex::new(Some(data.path()));
        idx.load_cache();
        idx.discover(&home);
        assert!(idx.persist(true), "pruning marks the cache dirty");
        assert!(cached().is_empty());
    }
}
