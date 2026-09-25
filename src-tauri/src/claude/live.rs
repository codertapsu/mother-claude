//! Live session roster from `~/.claude/sessions/<pid>.json` (design §3.1,
//! amended §10 B2).
//!
//! Every running Claude Code process (CLI, VS Code, JetBrains, Desktop, SDK,
//! `--bg` workers) writes `sessions/<pid>.json` and removes it on a clean exit.
//! Next to it sits `<pid>.<64hex>.key`, a secret: this module only ever opens
//! `*.json` files and never lists key contents.
//!
//! * **Liveness**: `kill(pid, 0)` (EPERM counts as alive) plus, when
//!   `pidDomain == "darwin"`, the pidfile's `procStart` compared with
//!   `ps -o lstart= -p <pid>` run with `TZ=UTC LC_ALL=C` (whitespace collapsed
//!   and trimmed on both sides — `ps` pads days and appends spaces). The `ps`
//!   answer is cached per `(pid, procStart)`. If `ps` fails or prints nothing
//!   parseable the check falls back to `kill` only: never "dead" on doubt.
//! * **Torn reads**: the CLI rewrites the file in place, so a JSON error is
//!   retried once after 50 ms.
//! * `spare` pidfiles (pre-spawned background workers) are ignored.
//! * **Background jobs**: `jobs/<short>/state.json` gives the short job id
//!   (`daemonShort`) and the lifecycle (`working|blocked|done|failed|stopped`)
//!   of `--bg` sessions, including stopped ones that have no process.
//! * The `claude agents --json` spawn of the old monitor is gone.
//! * Status transitions (`idle`/`busy`/`waiting`) are broadcast on
//!   [`LiveRoster::subscribe`] (B4 uses them for edge-triggered withdrawal).
//!
//! AppState field: `state.live`. Reading happens in [`LiveRoster::refresh`]
//! (blocking I/O — call it from `spawn_blocking`); the accessors are cheap.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use tokio::sync::broadcast;

use super::home::ClaudeHome;

/// One `~/.claude/sessions/<pid>.json` record. Every field optional; unknown
/// keys land in `extra`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LiveSession {
    pub pid: Option<u32>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<i64>,
    pub proc_start: Option<String>,
    pub version: Option<String>,
    /// `interactive` | `bg` | `background` | …
    pub kind: Option<String>,
    /// `claude-vscode` | `cli` | `sdk-cli` | …
    pub entrypoint: Option<String>,
    pub name: Option<String>,
    pub name_source: Option<String>,
    /// `idle` | `busy` | `waiting` | `shell` | …
    pub status: Option<String>,
    pub status_updated_at: Option<i64>,
    /// e.g. `permission prompt`, `input needed`, `dialog open`.
    pub waiting_for: Option<String>,
    /// Short background job id.
    pub job_id: Option<String>,
    /// tmux target `session:@win.%pane`.
    pub tmux: Option<String>,
    pub bridge_session_id: Option<String>,
    pub state: Option<String>,
    pub detail: Option<String>,
    pub tempo: Option<Value>,
    pub needs: Option<Value>,
    pub updated_at: Option<i64>,
    /// `darwin` on macOS; `procStart` is only comparable there.
    pub pid_domain: Option<String>,
    pub peer_features: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl LiveSession {
    /// A pre-spawned background worker (`spare: true`): not a session yet.
    pub fn is_spare(&self) -> bool {
        match self.extra.get("spare") {
            Some(Value::Bool(b)) => *b,
            Some(Value::Null) | None => false,
            Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
            Some(Value::String(s)) => !s.is_empty() && s != "false" && s != "0",
            Some(_) => true,
        }
    }

    /// A `claude --bg` worker (pidfile kind `bg`, agents JSON `background`).
    pub fn is_background(&self) -> bool {
        matches!(self.kind.as_deref(), Some("bg" | "background"))
    }
}

/// `~/.claude/jobs/<short>/state.json` of a background job. Only the fields
/// the dashboard shows are read; unknown keys (e.g. `providerEnv`, which may
/// carry credentials) are deliberately NOT captured.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JobState {
    /// Directory name — the short job id.
    #[serde(skip_deserializing)]
    pub short: String,
    /// `working` | `blocked` | `done` | `failed` | `stopped`.
    #[serde(deserialize_with = "loose_string")]
    pub state: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub detail: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub tempo: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub session_id: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub daemon_short: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub name: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub cwd: Option<String>,
    #[serde(deserialize_with = "loose_i64")]
    pub created_at: Option<i64>,
    #[serde(deserialize_with = "loose_i64")]
    pub updated_at: Option<i64>,
    #[serde(deserialize_with = "loose_string")]
    pub cli_version: Option<String>,
    #[serde(deserialize_with = "loose_i64")]
    pub tokens: Option<i64>,
}

impl JobState {
    /// The id `claude stop|respawn|rm|logs|attach` accept.
    pub fn short_id(&self) -> &str {
        self.daemon_short.as_deref().unwrap_or(&self.short)
    }

    /// `done` / `failed` / `stopped`.
    pub fn is_terminal(&self) -> bool {
        matches!(self.state.as_deref(), Some("done" | "failed" | "stopped"))
    }
}

fn loose_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn loose_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.parse::<i64>().ok().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|t| t.timestamp_millis())
        }),
        _ => None,
    })
}

/// A pidfile status change (`from`/`to` are pidfile `status` values; `to` is
/// `None` when the process went away).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTransition {
    pub session_id: String,
    pub pid: u32,
    pub from: Option<String>,
    pub to: Option<String>,
    /// The pidfile's `statusUpdatedAt` of the new state (epoch ms).
    pub status_updated_at: Option<i64>,
    pub waiting_for: Option<String>,
}

/// What one refresh changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveChange {
    /// Anything visible changed (a session appeared/disappeared, a status or
    /// field changed, a job state changed).
    pub changed: bool,
    pub appeared: Vec<String>,
    pub gone: Vec<String>,
    pub transitions: Vec<LiveTransition>,
}

#[derive(Debug, Clone)]
struct CachedFile {
    stamp: (u64, i64),
    record: Option<LiveSession>,
}

#[derive(Debug, Clone)]
struct CachedJob {
    stamp: (u64, i64),
    state: Option<JobState>,
}

#[derive(Debug, Default)]
struct RosterInner {
    sessions_dir_seen: bool,
    files: HashMap<PathBuf, CachedFile>,
    jobs_files: HashMap<PathBuf, CachedJob>,
    /// `(pid, procStart)` → (same process?, checked at).
    proc_checks: HashMap<(u32, String), (bool, Instant)>,
    /// `(pid, procStart)` → permission mode from the process argv.
    argv_modes: HashMap<(u32, String), Option<String>>,
    /// Live sessions by session id.
    live: HashMap<String, LiveSession>,
    /// Background job state by session id.
    jobs: HashMap<String, JobState>,
}

/// The live roster.
#[derive(Debug)]
pub struct LiveRoster {
    inner: Mutex<RosterInner>,
    tx: broadcast::Sender<LiveTransition>,
}

impl Default for LiveRoster {
    fn default() -> Self {
        Self::new()
    }
}

/// How long a `ps` identity check stays valid before it is repeated.
const PROC_CHECK_TTL: Duration = Duration::from_secs(120);

fn stamp(meta: &std::fs::Metadata) -> (u64, i64) {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    (meta.len(), mtime)
}

/// Collapse runs of whitespace and trim (for `lstart` comparisons).
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `kill(pid, 0)`: the process exists (EPERM = exists but not ours).
#[cfg(unix)]
pub fn pid_exists(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: kill with signal 0 performs only the existence/permission check
    // and never delivers a signal; the pid is a plain integer.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if r == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Non-unix fallback: trust the pidfile.
#[cfg(not(unix))]
pub fn pid_exists(pid: u32) -> bool {
    pid != 0
}

/// `ps -o <field>= -p <pid>` with a C locale in UTC. `None` when `ps` failed
/// or printed nothing.
#[cfg(unix)]
fn ps_field(pid: u32, field: &str) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(not(unix))]
fn ps_field(_pid: u32, _field: &str) -> Option<String> {
    None
}

/// Extract the permission mode a process was launched with from its argv.
pub fn permission_mode_from_argv(cmdline: &str) -> Option<String> {
    let mut words = cmdline.split_whitespace();
    let mut mode = None;
    while let Some(w) = words.next() {
        if w == "--dangerously-skip-permissions" {
            mode = Some("bypassPermissions".to_string());
        } else if w == "--permission-mode" {
            if let Some(m) = words.next() {
                mode = Some(m.to_string());
            }
        } else if let Some(m) = w.strip_prefix("--permission-mode=") {
            mode = Some(m.to_string());
        }
    }
    mode.filter(|m| !m.is_empty() && m.len() <= 40)
}

/// Read and parse one pidfile, retrying once after 50 ms on a torn read.
fn read_pidfile(path: &Path) -> Option<LiveSession> {
    for attempt in 0..2 {
        let text = match std::fs::read(path) {
            Ok(t) => t,
            Err(e) => {
                tracing::debug!(error = %e, path = %path.display(), "pidfile unreadable");
                return None;
            }
        };
        match serde_json::from_slice::<LiveSession>(&text) {
            Ok(rec) => return Some(rec),
            Err(e) if attempt == 0 => {
                tracing::trace!(error = %e, "pidfile torn read; retrying");
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                tracing::debug!(error = %e, path = %path.display(), "pidfile unparseable; skipped");
            }
        }
    }
    None
}

fn read_job(path: &Path, short: &str) -> Option<JobState> {
    let text = std::fs::read(path).ok()?;
    match serde_json::from_slice::<JobState>(&text) {
        Ok(mut s) => {
            s.short = short.to_string();
            Some(s)
        }
        Err(e) => {
            tracing::debug!(error = %e, job = short, "job state unparseable; skipped");
            None
        }
    }
}

impl LiveRoster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            inner: Mutex::new(RosterInner::default()),
            tx,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RosterInner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Pidfile status transitions (and disappearances) as they are observed.
    pub fn subscribe(&self) -> broadcast::Receiver<LiveTransition> {
        self.tx.subscribe()
    }

    /// The live record of a session, if its process is alive.
    pub fn get(&self, session_id: &str) -> Option<LiveSession> {
        self.lock().live.get(session_id).cloned()
    }

    /// Every live session.
    pub fn all(&self) -> Vec<LiveSession> {
        self.lock().live.values().cloned().collect()
    }

    /// Whether the session's process is alive.
    pub fn is_live(&self, session_id: &str) -> bool {
        self.lock().live.contains_key(session_id)
    }

    /// Background job state of a session (live or not).
    pub fn job(&self, session_id: &str) -> Option<JobState> {
        self.lock().jobs.get(session_id).cloned()
    }

    /// Every known background job.
    pub fn jobs(&self) -> Vec<JobState> {
        self.lock().jobs.values().cloned().collect()
    }

    /// The short job id for `claude stop|respawn|rm|logs|attach`: pidfile
    /// `jobId`, else `jobs/<short>/state.json` (`daemonShort` / dir name).
    pub fn short_job_id(&self, session_id: &str) -> Option<String> {
        let g = self.lock();
        g.live
            .get(session_id)
            .and_then(|s| s.job_id.clone())
            .or_else(|| g.jobs.get(session_id).map(|j| j.short_id().to_string()))
            .filter(|s| super::ids::is_short_job_id(s))
    }

    /// Permission mode from the live process's argv (`--permission-mode X` /
    /// `--dangerously-skip-permissions`), when it was launched with one.
    pub fn argv_permission_mode(&self, session_id: &str) -> Option<String> {
        let g = self.lock();
        let s = g.live.get(session_id)?;
        let key = (s.pid?, s.proc_start.clone().unwrap_or_default());
        g.argv_modes.get(&key).cloned().flatten()
    }

    /// Re-read the session's pidfile right now (not the cached record):
    /// `(status, waitingFor)`. `None` when the roster knows no pid for it or
    /// the file is gone, unreadable or belongs to another session. Nothing
    /// is cached or broadcast (that stays the monitor's job). Blocking (may
    /// sleep 50 ms on a torn read): call from `spawn_blocking`.
    pub fn fresh_status(
        &self,
        home: &ClaudeHome,
        session_id: &str,
    ) -> Option<(Option<String>, Option<String>)> {
        let pid = self.lock().live.get(session_id)?.pid?;
        let path = home.base().join("sessions").join(format!("{pid}.json"));
        let rec = read_pidfile(&path)?;
        (rec.session_id.as_deref() == Some(session_id)).then_some((rec.status, rec.waiting_for))
    }

    /// Whether `~/.claude/sessions/` existed at the last refresh. Older CLIs
    /// did not write pidfiles; the data layer then falls back to transcript
    /// recency for liveness.
    pub fn has_sessions_dir(&self) -> bool {
        self.lock().sessions_dir_seen
    }

    /// Re-read the sessions and jobs directories (only files whose size/mtime
    /// changed are parsed), re-check liveness, and report what changed.
    /// Blocking: call from `spawn_blocking`.
    pub fn refresh(&self, home: &ClaudeHome) -> LiveChange {
        let sessions_dir = home.base().join("sessions");
        let jobs_dir = home.jobs_dir();

        // 1. Pidfiles (outside the lock: may sleep 50 ms on a torn read).
        let listing = std::fs::read_dir(&sessions_dir).ok();
        let dir_seen = listing.is_some();
        let mut seen_paths: HashSet<PathBuf> = HashSet::new();
        let mut to_parse: Vec<(PathBuf, (u64, i64))> = Vec::new();
        {
            let g = self.lock();
            for entry in listing.into_iter().flatten().flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                // Only `<pid>.json` — never `<pid>.<hex>.key` or anything else.
                let Some(stem) = name.strip_suffix(".json") else {
                    continue;
                };
                if stem.is_empty() || !stem.bytes().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if !meta.is_file() {
                    continue;
                }
                let st = stamp(&meta);
                seen_paths.insert(path.clone());
                if g.files.get(&path).map(|c| c.stamp) != Some(st) {
                    to_parse.push((path, st));
                }
            }
        }
        let parsed: Vec<(PathBuf, CachedFile)> = to_parse
            .into_iter()
            .map(|(path, st)| {
                let record = read_pidfile(&path);
                (path, CachedFile { stamp: st, record })
            })
            .collect();

        // 2. Jobs.
        let mut job_seen: HashSet<PathBuf> = HashSet::new();
        let mut job_parse: Vec<(PathBuf, String, (u64, i64))> = Vec::new();
        {
            let g = self.lock();
            for entry in std::fs::read_dir(&jobs_dir).into_iter().flatten().flatten() {
                let short = entry.file_name().to_string_lossy().to_string();
                if !super::ids::is_short_job_id(&short) {
                    continue;
                }
                let path = entry.path().join("state.json");
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue;
                };
                let st = stamp(&meta);
                job_seen.insert(path.clone());
                if g.jobs_files.get(&path).map(|c| c.stamp) != Some(st) {
                    job_parse.push((path, short, st));
                }
            }
        }
        let jobs_parsed: Vec<(PathBuf, CachedJob)> = job_parse
            .into_iter()
            .map(|(path, short, st)| {
                let state = read_job(&path, &short);
                (path, CachedJob { stamp: st, state })
            })
            .collect();

        // 3. Candidates → liveness (ps outside the lock).
        let candidates: Vec<LiveSession> = {
            let mut g = self.lock();
            g.sessions_dir_seen = dir_seen;
            for (p, c) in parsed {
                g.files.insert(p, c);
            }
            g.files.retain(|p, _| seen_paths.contains(p));
            for (p, c) in jobs_parsed {
                g.jobs_files.insert(p, c);
            }
            g.jobs_files.retain(|p, _| job_seen.contains(p));
            g.files
                .values()
                .filter_map(|c| c.record.clone())
                .filter(|r| !r.is_spare())
                .filter(|r| {
                    r.session_id
                        .as_deref()
                        .is_some_and(super::ids::is_session_id)
                })
                .collect()
        };
        let mut alive: Vec<LiveSession> = Vec::new();
        let mut new_checks: Vec<((u32, String), bool)> = Vec::new();
        let mut new_argv: Vec<((u32, String), Option<String>)> = Vec::new();
        for rec in candidates {
            let Some(pid) = rec.pid else { continue };
            if !pid_exists(pid) {
                continue;
            }
            let key = (pid, rec.proc_start.clone().unwrap_or_default());
            let cached = {
                let g = self.lock();
                (
                    g.proc_checks
                        .get(&key)
                        .filter(|(_, at)| at.elapsed() < PROC_CHECK_TTL)
                        .map(|(ok, _)| *ok),
                    g.argv_modes.contains_key(&key),
                )
            };
            let same_process = match cached.0 {
                Some(ok) => ok,
                None => {
                    let ok = match (&rec.pid_domain, &rec.proc_start) {
                        (Some(domain), Some(expected)) if domain == "darwin" => {
                            match ps_field(pid, "lstart") {
                                Some(actual) => collapse_ws(&actual) == collapse_ws(expected),
                                None => true, // ps failed: kill-only, never dead on doubt
                            }
                        }
                        _ => true,
                    };
                    new_checks.push((key.clone(), ok));
                    ok
                }
            };
            if !same_process {
                continue;
            }
            if !cached.1 {
                let mode = ps_field(pid, "command").and_then(|c| permission_mode_from_argv(&c));
                new_argv.push((key, mode));
            }
            alive.push(rec);
        }

        // 4. Swap in and diff.
        let mut change = LiveChange::default();
        let mut g = self.lock();
        let now = Instant::now();
        for (k, ok) in new_checks {
            g.proc_checks.insert(k, (ok, now));
        }
        for (k, m) in new_argv {
            g.argv_modes.insert(k, m);
        }
        let mut next: HashMap<String, LiveSession> = HashMap::new();
        for rec in alive {
            let Some(sid) = rec.session_id.clone() else {
                continue;
            };
            // Two pidfiles for one session (a resumed fork racing an exit):
            // keep the most recently started.
            match next.get(&sid) {
                Some(prev) if prev.started_at >= rec.started_at => {}
                _ => {
                    next.insert(sid, rec);
                }
            }
        }
        for (sid, rec) in &next {
            match g.live.get(sid) {
                None => {
                    change.appeared.push(sid.clone());
                    change.changed = true;
                    if rec.status.is_some() {
                        change.transitions.push(LiveTransition {
                            session_id: sid.clone(),
                            pid: rec.pid.unwrap_or(0),
                            from: None,
                            to: rec.status.clone(),
                            status_updated_at: rec.status_updated_at,
                            waiting_for: rec.waiting_for.clone(),
                        });
                    }
                }
                Some(prev) => {
                    if prev != rec {
                        change.changed = true;
                    }
                    if prev.status != rec.status || prev.pid != rec.pid {
                        change.transitions.push(LiveTransition {
                            session_id: sid.clone(),
                            pid: rec.pid.unwrap_or(0),
                            from: prev.status.clone(),
                            to: rec.status.clone(),
                            status_updated_at: rec.status_updated_at,
                            waiting_for: rec.waiting_for.clone(),
                        });
                    }
                }
            }
        }
        for (sid, prev) in &g.live {
            if !next.contains_key(sid) {
                change.gone.push(sid.clone());
                change.changed = true;
                change.transitions.push(LiveTransition {
                    session_id: sid.clone(),
                    pid: prev.pid.unwrap_or(0),
                    from: prev.status.clone(),
                    to: None,
                    status_updated_at: None,
                    waiting_for: None,
                });
            }
        }
        let jobs: HashMap<String, JobState> = g
            .jobs_files
            .values()
            .filter_map(|c| c.state.clone())
            .filter_map(|j| j.session_id.clone().map(|sid| (sid, j)))
            .collect();
        if jobs != g.jobs {
            change.changed = true;
        }
        g.jobs = jobs;
        g.live = next;
        // Forget identity checks of processes that are gone.
        let live_keys: HashSet<(u32, String)> = g
            .live
            .values()
            .filter_map(|s| Some((s.pid?, s.proc_start.clone().unwrap_or_default())))
            .collect();
        g.proc_checks.retain(|k, _| live_keys.contains(k));
        g.argv_modes.retain(|k, _| live_keys.contains(k));
        drop(g);
        change.appeared.sort();
        change.gone.sort();
        for t in &change.transitions {
            let _ = self.tx.send(t.clone());
        }
        change
    }

    /// Test/dev hook: insert a live record directly (no liveness check).
    pub fn insert_for_test(&self, rec: LiveSession) {
        if let Some(sid) = rec.session_id.clone() {
            self.lock().live.insert(sid, rec);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "e2b6972f-7126-4f6e-a490-5ce75cb6a9f9";

    #[test]
    fn pidfile_parses_tolerantly() {
        let s: LiveSession = serde_json::from_str(
            r#"{"pid":42,"sessionId":"abc","kind":"interactive","status":"busy",
                "waitingFor":"permission prompt","tmux":"main:@1.%3","novel":true}"#,
        )
        .unwrap();
        assert_eq!(s.pid, Some(42));
        assert_eq!(s.tmux.as_deref(), Some("main:@1.%3"));
        assert_eq!(s.extra["novel"], true);
        let empty: LiveSession = serde_json::from_str("{}").unwrap();
        assert!(empty.pid.is_none());
        let spare: LiveSession = serde_json::from_str(r#"{"spare":true}"#).unwrap();
        assert!(spare.is_spare());
        let not_spare: LiveSession = serde_json::from_str(r#"{"spare":false}"#).unwrap();
        assert!(!not_spare.is_spare());
    }

    #[test]
    fn lstart_comparison_collapses_whitespace() {
        // `ps` pads single-digit days and appends spaces.
        assert_eq!(
            collapse_ws("Fri Sep  5 10:17:00 2026    "),
            "Fri Sep 5 10:17:00 2026"
        );
        assert_eq!(
            collapse_ws("Fri Sep 25 10:17:00 2026    "),
            collapse_ws("Fri Sep 25 10:17:00 2026")
        );
    }

    #[test]
    fn argv_permission_modes() {
        assert_eq!(
            permission_mode_from_argv(
                "/x/claude --output-format stream-json --permission-mode bypassPermissions --permission-prompt-tool stdio"
            )
            .as_deref(),
            Some("bypassPermissions")
        );
        assert_eq!(
            permission_mode_from_argv("claude --permission-mode=plan").as_deref(),
            Some("plan")
        );
        assert_eq!(
            permission_mode_from_argv("claude --dangerously-skip-permissions").as_deref(),
            Some("bypassPermissions")
        );
        assert_eq!(permission_mode_from_argv("claude -p hi"), None);
    }

    #[cfg(unix)]
    #[test]
    fn own_process_is_alive_and_bogus_pid_is_not() {
        assert!(pid_exists(std::process::id()));
        assert!(!pid_exists(0));
        assert!(!pid_exists(u32::MAX));
    }

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn fresh_status_rereads_the_pidfile_without_touching_the_roster() {
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let sessions = tmp.path().join("sessions");
        let me = std::process::id();
        let body = |status: &str, waiting: &str| {
            format!(
                r#"{{"pid":{me},"sessionId":"{SID}","status":"{status}","waitingFor":"{waiting}"}}"#
            )
        };
        let roster = LiveRoster::new();
        assert_eq!(roster.fresh_status(&home, SID), None, "no pid known yet");
        write(&sessions, &format!("{me}.json"), &body("busy", ""));
        roster.refresh(&home);
        // The CLI opens a dialog; the monitor has not refreshed yet.
        write(
            &sessions,
            &format!("{me}.json"),
            &body("waiting", "permission prompt"),
        );
        assert_eq!(roster.get(SID).unwrap().status.as_deref(), Some("busy"));
        assert_eq!(
            roster.fresh_status(&home, SID),
            Some((
                Some("waiting".to_string()),
                Some("permission prompt".to_string())
            ))
        );
        assert_eq!(
            roster.get(SID).unwrap().status.as_deref(),
            Some("busy"),
            "cache untouched"
        );
        // The pid now belongs to another session: not ours.
        write(
            &sessions,
            &format!("{me}.json"),
            &format!(r#"{{"pid":{me},"sessionId":"other","status":"idle"}}"#),
        );
        assert_eq!(roster.fresh_status(&home, SID), None);
    }

    #[test]
    fn roster_reads_live_pidfiles_and_skips_keys_spares_and_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let sessions = tmp.path().join("sessions");
        let me = std::process::id();
        // Live: our own pid, no pidDomain (kill-only check).
        write(
            &sessions,
            &format!("{me}.json"),
            &format!(
                r#"{{"pid":{me},"sessionId":"{SID}","cwd":"/p","status":"waiting","waitingFor":"permission prompt","kind":"interactive","tmux":"s:@1.%2","jobId":"abcdef12"}}"#
            ),
        );
        // A key file must never be parsed (it isn't JSON anyway).
        write(&sessions, &format!("{me}.{}.key", "a".repeat(64)), "SECRET");
        // Spare worker.
        write(
            &sessions,
            "1.json",
            r#"{"pid":1,"sessionId":"11111111-1111-4111-8111-111111111111","spare":true}"#,
        );
        // Dead pid.
        write(
            &sessions,
            "999999.json",
            r#"{"pid":999999,"sessionId":"22222222-2222-4222-8222-222222222222"}"#,
        );
        // Torn/garbage file.
        write(&sessions, "12345.json", "{\"pid\":");
        // Our own pid with a mismatching procStart on darwin ⇒ PID reuse ⇒ dead.
        let roster = LiveRoster::new();
        let mut rx = roster.subscribe();
        let change = roster.refresh(&home);
        assert!(roster.has_sessions_dir());
        assert_eq!(change.appeared, vec![SID.to_string()]);
        let s = roster.get(SID).expect("live");
        assert_eq!(s.waiting_for.as_deref(), Some("permission prompt"));
        assert_eq!(roster.all().len(), 1);
        assert_eq!(roster.short_job_id(SID).as_deref(), Some("abcdef12"));
        let t = rx.try_recv().unwrap();
        assert_eq!(t.to.as_deref(), Some("waiting"));

        // Unchanged refresh: no change.
        assert!(!roster.refresh(&home).changed);

        // Status transition waiting → busy is reported.
        std::thread::sleep(Duration::from_millis(20));
        write(
            &sessions,
            &format!("{me}.json"),
            &format!(
                r#"{{"pid":{me},"sessionId":"{SID}","cwd":"/p","status":"busy","statusUpdatedAt":5,"kind":"interactive","extraPadding":"xxxxxxxx"}}"#
            ),
        );
        let change = roster.refresh(&home);
        assert!(change.changed);
        assert_eq!(change.transitions.len(), 1);
        assert_eq!(change.transitions[0].from.as_deref(), Some("waiting"));
        assert_eq!(change.transitions[0].to.as_deref(), Some("busy"));

        // procStart mismatch on darwin ⇒ treated as a reused pid (dead) when
        // `ps` works; if `ps` is unavailable the kill-only rule keeps it.
        write(
            &sessions,
            &format!("{me}.json"),
            &format!(
                r#"{{"pid":{me},"sessionId":"{SID}","pidDomain":"darwin","procStart":"Thu Jan  1 00:00:00 1970","status":"idle","pad":"yyyyyyyyyyyy"}}"#
            ),
        );
        let change = roster.refresh(&home);
        if ps_field(me, "lstart").is_some() {
            assert_eq!(change.gone, vec![SID.to_string()]);
            assert!(roster.get(SID).is_none());
        }

        // File removed ⇒ gone.
        std::fs::remove_file(sessions.join(format!("{me}.json"))).unwrap();
        roster.refresh(&home);
        assert!(roster.get(SID).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn matching_proc_start_keeps_the_session() {
        let me = std::process::id();
        let Some(lstart) = ps_field(me, "lstart") else {
            return; // no ps in this environment
        };
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        // Written the way the CLI writes it: no padding, single spaces.
        let proc_start = collapse_ws(&lstart);
        write(
            &tmp.path().join("sessions"),
            &format!("{me}.json"),
            &format!(
                r#"{{"pid":{me},"sessionId":"{SID}","pidDomain":"darwin","procStart":"{proc_start}"}}"#
            ),
        );
        let roster = LiveRoster::new();
        roster.refresh(&home);
        assert!(roster.is_live(SID));
    }

    #[test]
    fn job_state_gives_short_ids_and_terminal_states() {
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        write(
            &tmp.path().join("jobs/631b387d"),
            "state.json",
            &format!(
                r#"{{"state":"done","detail":"replied OK","tempo":"idle","sessionId":"{SID}","daemonShort":"631b387d","providerEnv":{{"SECRET":"x"}},"createdAt":1790338119067,"inFlight":{{"tasks":0}}}}"#
            ),
        );
        // Not a job dir.
        write(&tmp.path().join("jobs"), "pins.json", "[]");
        let roster = LiveRoster::new();
        let change = roster.refresh(&home);
        assert!(change.changed);
        let job = roster.job(SID).unwrap();
        assert_eq!(job.short_id(), "631b387d");
        assert!(job.is_terminal());
        assert_eq!(job.created_at, Some(1790338119067));
        assert_eq!(roster.short_job_id(SID).as_deref(), Some("631b387d"));
        assert!(!roster.is_live(SID));
        // Nothing secret is captured.
        assert!(!format!("{job:?}").contains("SECRET"));
    }

    #[test]
    fn missing_sessions_dir_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let roster = LiveRoster::new();
        roster.refresh(&ClaudeHome::with_base(tmp.path()));
        assert!(!roster.has_sessions_dir());
        assert!(roster.all().is_empty());
    }
}
