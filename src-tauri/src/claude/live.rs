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
//! * **Accounts** (docs/ACCOUNTS.md §5.2): [`LiveRoster::refresh_homes`]
//!   scans every account's `sessions/` and `jobs/` and tags each record with
//!   the account whose directory holds it. With the argv, a process's
//!   `CLAUDE_CODE_OAUTH_TOKEN` is hashed where the platform still exposes
//!   another process's environment (Linux); macOS 27 does not, and there the
//!   hooks plugin reports the hash.
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

/// One `~/.claude/sessions/<pid>.json` record. Every field optional and
/// loosely typed (a field whose type drifts reads as `None` instead of failing
/// the whole record); unknown keys land in `extra`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LiveSession {
    #[serde(deserialize_with = "loose_u32")]
    pub pid: Option<u32>,
    #[serde(deserialize_with = "loose_string")]
    pub session_id: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub cwd: Option<String>,
    #[serde(deserialize_with = "loose_i64")]
    pub started_at: Option<i64>,
    #[serde(deserialize_with = "loose_string")]
    pub proc_start: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub version: Option<String>,
    /// `interactive` | `bg` | `background` | …
    #[serde(deserialize_with = "loose_string")]
    pub kind: Option<String>,
    /// `claude-vscode` | `cli` | `sdk-cli` | …
    #[serde(deserialize_with = "loose_string")]
    pub entrypoint: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub name: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub name_source: Option<String>,
    /// `idle` | `busy` | `waiting` | `shell` | …
    #[serde(deserialize_with = "loose_string")]
    pub status: Option<String>,
    #[serde(deserialize_with = "loose_i64")]
    pub status_updated_at: Option<i64>,
    /// e.g. `permission prompt`, `input needed`, `dialog open`.
    #[serde(deserialize_with = "loose_string")]
    pub waiting_for: Option<String>,
    /// Short background job id.
    #[serde(deserialize_with = "loose_string")]
    pub job_id: Option<String>,
    /// tmux target `session:@win.%pane`.
    #[serde(deserialize_with = "loose_string")]
    pub tmux: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub bridge_session_id: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub state: Option<String>,
    #[serde(deserialize_with = "loose_string")]
    pub detail: Option<String>,
    pub tempo: Option<Value>,
    pub needs: Option<Value>,
    #[serde(deserialize_with = "loose_i64")]
    pub updated_at: Option<i64>,
    /// `darwin` on macOS; `procStart` is only comparable there.
    #[serde(deserialize_with = "loose_string")]
    pub pid_domain: Option<String>,
    #[serde(deserialize_with = "loose_strings")]
    pub peer_features: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
    /// The account whose config directory holds the pidfile (set by a
    /// refresh; docs/ACCOUNTS.md §5.2).
    #[serde(skip)]
    pub home_account: Option<String>,
    /// SHA-256 of the process's `CLAUDE_CODE_OAUTH_TOKEN`, when its
    /// environment has one (D8: read once per process, hashed, the value
    /// dropped). Never serialized.
    #[serde(skip)]
    pub token_sha256: Option<String>,
    /// The pidfile the record was read from.
    #[serde(skip)]
    pub source: Option<PathBuf>,
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
    /// The account whose config directory holds the job (set by a refresh).
    #[serde(skip)]
    pub account: Option<String>,
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

fn loose_u32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| {
                n.as_f64()
                    .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                    .map(|f| f as u64)
            })
            .and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.trim().parse::<u32>().ok(),
        _ => None,
    })
}

/// A list of strings: an array (non-strings skipped), a single string, or an
/// object (its keys).
fn loose_strings<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(a) => Some(
            a.into_iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s),
                    _ => None,
                })
                .collect(),
        ),
        Value::String(s) => Some(vec![s]),
        Value::Object(m) => Some(m.into_iter().map(|(k, _)| k).collect()),
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
    /// Consecutive read/parse failures (reset by a good read).
    failures: u8,
    /// The account whose `sessions/` holds it.
    account: String,
}

/// A stamp no file has: the entry is re-read on the next refresh.
const RETRY_STAMP: (u64, i64) = (u64::MAX, i64::MIN);
/// After this many consecutive failed reads of a changed pidfile its stamp is
/// accepted (the last good record is kept) so a persistently broken file is
/// not re-read, with its 50 ms torn-read retry, on every refresh.
const MAX_READ_FAILURES: u8 = 3;

/// The outcome of reading one pidfile.
#[derive(Debug)]
enum PidfileRead {
    Parsed(Box<LiveSession>),
    /// The file no longer exists.
    Gone,
    /// Unreadable or unparseable (a torn rewrite, or drift serde can't read).
    Failed,
}

#[derive(Debug, Clone)]
struct CachedJob {
    stamp: (u64, i64),
    state: Option<JobState>,
}

/// The Default account's id, for a roster refreshed with one home.
const DEFAULT_ACCOUNT: &str = super::accounts::DEFAULT_ID;

#[derive(Debug, Default)]
struct RosterInner {
    sessions_dir_seen: bool,
    files: HashMap<PathBuf, CachedFile>,
    jobs_files: HashMap<PathBuf, CachedJob>,
    /// `(pid, procStart)` → (same process?, checked at).
    proc_checks: HashMap<(u32, String), (bool, Instant)>,
    /// `(pid, procStart)` → what the process argv says (read once).
    argv_flags: HashMap<(u32, String), LaunchFlags>,
    /// `(pid, procStart)` → the SHA-256 of its OAuth token, if any (read
    /// with the argv).
    proc_tokens: HashMap<(u32, String), Option<String>>,
    /// Live sessions by session id.
    live: HashMap<String, LiveSession>,
    /// Background job state by session id.
    jobs: HashMap<String, JobState>,
    /// Working directories whose processes are never sessions (Mother
    /// Claude's own plan-usage helper), without a trailing `/`.
    ignored_cwds: Vec<String>,
}

/// Thinking as a process was launched with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchThinking {
    /// `--thinking disabled`, `--max-thinking-tokens 0`.
    Off,
    /// `--thinking adaptive|enabled`.
    Adaptive,
    /// `--max-thinking-tokens N` with N > 0: on with a budget (the CLI
    /// ignores the budget on adaptive-capable models).
    Budget(u32),
}

/// What a live process's argv says about its settings. Spawn-time only: a
/// later in-app change (VS Code's thinking toggle, `/effort`) is invisible
/// here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchFlags {
    /// `--permission-mode X`, `--dangerously-skip-permissions`.
    pub permission_mode: Option<String>,
    /// `--effort <low|medium|high|xhigh|max>` (`ultracode` runs as xhigh).
    pub effort: Option<String>,
    /// `--thinking …` wins over `--max-thinking-tokens N`.
    pub thinking: Option<LaunchThinking>,
    /// `--effort ultracode`, or an inline `--settings '{"ultracode":…}'`.
    pub ultracode: Option<bool>,
}

/// The value of `--name X` / `--name=X` in split argv words (the last one
/// wins, like the CLI's parser).
fn flag_value<'a>(words: &[&'a str], name: &str) -> Option<&'a str> {
    let mut found = None;
    let eq = format!("{name}=");
    let mut i = 0;
    while i < words.len() {
        if words[i] == name {
            if let Some(v) = words.get(i + 1) {
                found = Some(*v);
                i += 1;
            }
        } else if let Some(v) = words[i].strip_prefix(&eq) {
            found = Some(v);
        }
        i += 1;
    }
    found
}

/// The JSON object an inline `--settings {…}` starts with in a raw command
/// line (`ps` prints each argument as given, so the braces balance).
fn inline_settings(cmdline: &str) -> Option<serde_json::Value> {
    let mut rest = cmdline;
    let mut found = None;
    while let Some(at) = rest.find("--settings") {
        let after = &rest[at + "--settings".len()..];
        let value = after
            .strip_prefix('=')
            .or_else(|| after.strip_prefix(' '))
            .map(str::trim_start);
        if let Some(v) = value.filter(|v| v.starts_with('{')) {
            if let Some(obj) = balanced_object(v) {
                found = serde_json::from_str::<serde_json::Value>(obj)
                    .ok()
                    .filter(serde_json::Value::is_object)
                    .or(found);
            }
        }
        rest = after;
    }
    found
}

/// The leading `{…}` of `s` (JSON strings respected), if it closes.
fn balanced_object(s: &str) -> Option<&str> {
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if in_str {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse the settings a process was launched with from its argv, each
/// argument whole as the process got it ([`process_argv`]): only an argument
/// that is the flag itself (or `--flag=value`) counts, its value is the next
/// argument, and nothing after `--` is a flag — so words inside a quoted
/// prompt (`claude "explain --thinking disabled"`) never are. An inline
/// `--settings` value is parsed as the JSON object it is.
pub fn launch_flags_from_args<S: AsRef<str>>(args: &[S]) -> LaunchFlags {
    let words: Vec<&str> = args
        .iter()
        .skip(1)
        .map(AsRef::as_ref)
        .take_while(|w| *w != "--")
        .collect();
    let settings = flag_value(&words, "--settings")
        .map(str::trim)
        .filter(|v| v.starts_with('{'))
        .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        .filter(serde_json::Value::is_object);
    flags_from_words(&words, settings)
}

/// [`launch_flags_from_args`] for a command line `ps` printed, where the
/// argument boundaries are lost: split on whitespace, a best guess (a quoted
/// prompt's words look like flags here). Only used when the real argv
/// cannot be read.
pub fn launch_flags_from_argv(cmdline: &str) -> LaunchFlags {
    let head = cmdline.split(" -- ").next().unwrap_or(cmdline);
    let words: Vec<&str> = head.split_whitespace().take_while(|w| *w != "--").collect();
    flags_from_words(&words, inline_settings(head))
}

fn flags_from_words(words: &[&str], settings: Option<serde_json::Value>) -> LaunchFlags {
    let mut flags = LaunchFlags::default();
    // Permission mode: the last of `--permission-mode` and the bypass flag.
    for (i, w) in words.iter().enumerate() {
        if *w == "--dangerously-skip-permissions" {
            flags.permission_mode = Some("bypassPermissions".to_string());
        } else if *w == "--permission-mode" {
            if let Some(m) = words.get(i + 1) {
                flags.permission_mode = Some(m.to_string());
            }
        } else if let Some(m) = w.strip_prefix("--permission-mode=") {
            flags.permission_mode = Some(m.to_string());
        }
    }
    flags.permission_mode = flags
        .permission_mode
        .filter(|m| !m.is_empty() && m.len() <= 40);
    match flag_value(words, "--effort").map(str::to_ascii_lowercase) {
        Some(e) if e == "ultracode" => {
            flags.effort = Some("xhigh".to_string());
            flags.ultracode = Some(true);
        }
        Some(e) if e == "med" => flags.effort = Some("medium".to_string()),
        Some(e) if super::owned::EFFORT_LEVELS.contains(&e.as_str()) => flags.effort = Some(e),
        _ => {}
    }
    flags.thinking = match flag_value(words, "--thinking") {
        Some("disabled") => Some(LaunchThinking::Off),
        Some("adaptive" | "enabled") => Some(LaunchThinking::Adaptive),
        _ => match flag_value(words, "--max-thinking-tokens").map(|n| n.parse::<u64>()) {
            Some(Ok(0)) => Some(LaunchThinking::Off),
            Some(Ok(n)) => Some(LaunchThinking::Budget(u32::try_from(n).unwrap_or(u32::MAX))),
            _ => None,
        },
    };
    if let Some(on) = settings
        .as_ref()
        .and_then(|s| s.get("ultracode"))
        .and_then(serde_json::Value::as_bool)
    {
        flags.ultracode = Some(on);
    }
    flags
}

/// A process's argv with its argument boundaries: `KERN_PROCARGS2` on macOS,
/// `/proc/<pid>/cmdline` on Linux; `None` elsewhere or when unreadable (the
/// caller falls back to `ps`).
#[cfg(target_os = "macos")]
pub fn process_argv(pid: u32) -> Option<Vec<String>> {
    parse_procargs2(&procargs2(pid)?)
}

/// What a process was started with, read once: its argv and, for D8, the
/// SHA-256 of its `CLAUDE_CODE_OAUTH_TOKEN` (the value itself is hashed where
/// it lies and never copied out). Our own user's processes only: the kernel
/// refuses anyone else's.
#[derive(Debug, Default)]
pub struct ProcFacts {
    pub args: Option<Vec<String>>,
    pub token_sha256: Option<String>,
}

#[cfg(target_os = "macos")]
pub fn process_facts(pid: u32) -> ProcFacts {
    match procargs2(pid) {
        Some(buf) => ProcFacts {
            args: parse_procargs2(&buf),
            token_sha256: env_sha256_in_procargs2(&buf, OAUTH_TOKEN_ENV),
        },
        None => ProcFacts::default(),
    }
}

#[cfg(target_os = "linux")]
pub fn process_facts(pid: u32) -> ProcFacts {
    let token_sha256 = std::fs::read(format!("/proc/{pid}/environ"))
        .ok()
        .and_then(|bytes| env_sha256_in(bytes.split(|b| *b == 0), OAUTH_TOKEN_ENV));
    ProcFacts {
        args: process_argv(pid),
        token_sha256,
    }
}

/// No environment read on other platforms (Windows: "unknown", §11.6).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn process_facts(pid: u32) -> ProcFacts {
    ProcFacts {
        args: process_argv(pid),
        token_sha256: None,
    }
}

/// The variable D8 looks for.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
const OAUTH_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// SHA-256 of `name`'s value among `NAME=value` strings (empty values count
/// as unset). The value is hashed in place.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn env_sha256_in<'a>(vars: impl Iterator<Item = &'a [u8]>, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    vars.filter_map(|v| v.strip_prefix(prefix.as_bytes()))
        .find(|value| !value.is_empty())
        .map(super::secrets::sha256_hex)
}

/// The environment after the arguments of a `KERN_PROCARGS2` buffer, as D8
/// needs it: the hash of one variable.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn env_sha256_in_procargs2(buf: &[u8], name: &str) -> Option<String> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let argc = usize::try_from(argc).ok().filter(|n| *n <= 65_536)?;
    let rest = &buf[4..];
    let path_end = rest.iter().position(|b| *b == 0)?;
    let rest = &rest[path_end..];
    let first = rest.iter().position(|b| *b != 0)?;
    // The environment ends at the first empty string after the arguments.
    let vars = rest[first..]
        .split(|b| *b == 0)
        .skip(argc)
        .take_while(|v| !v.is_empty());
    env_sha256_in(vars, name)
}

/// The raw `KERN_PROCARGS2` buffer of a process.
#[cfg(target_os = "macos")]
fn procargs2(pid: u32) -> Option<Vec<u8>> {
    let pid = libc::c_int::try_from(pid).ok().filter(|p| *p > 0)?;
    let mut argmax: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: sysctl writes at most `size` bytes (one c_int) into `argmax`.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut argmax as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    let argmax = usize::try_from(argmax).ok().filter(|n| rc == 0 && *n > 0)?;
    let mut buf = vec![0u8; argmax];
    let mut size = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: sysctl writes at most `size` bytes into `buf`, which is that long.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(size);
    Some(buf)
}

#[cfg(target_os = "linux")]
pub fn process_argv(pid: u32) -> Option<Vec<String>> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(&bytes);
    (!bytes.is_empty()).then(|| {
        bytes
            .split(|b| *b == 0)
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect()
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn process_argv(_pid: u32) -> Option<Vec<String>> {
    None
}

/// The arguments in a `KERN_PROCARGS2` buffer: `argc` (a native int), the
/// executable path, NUL padding, then `argc` NUL-terminated arguments. The
/// environment that may follow is never copied out: only
/// [`env_sha256_in_procargs2`] looks at it, for one variable's hash.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_procargs2(buf: &[u8]) -> Option<Vec<String>> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let argc = usize::try_from(argc).ok().filter(|n| *n <= 65_536)?;
    let rest = &buf[4..];
    let path_end = rest.iter().position(|b| *b == 0)?;
    let rest = &rest[path_end..];
    let first = rest.iter().position(|b| *b != 0)?;
    let args: Vec<String> = rest[first..]
        .split(|b| *b == 0)
        .take(argc)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    (args.len() == argc).then_some(args)
}

/// A working directory as the roster compares it (no trailing `/`).
fn cwd_key(cwd: &str) -> String {
    match cwd.trim_end_matches('/') {
        "" => "/".to_string(),
        c => c.to_string(),
    }
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
    launch_flags_from_argv(cmdline).permission_mode
}

/// Read and parse one pidfile, retrying once after 50 ms on a torn read.
fn read_pidfile(path: &Path) -> PidfileRead {
    for attempt in 0..2 {
        let text = match std::fs::read(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PidfileRead::Gone,
            Err(e) => {
                tracing::debug!(error = %e, path = %path.display(), "pidfile unreadable");
                return PidfileRead::Failed;
            }
        };
        match serde_json::from_slice::<LiveSession>(&text) {
            Ok(rec) => return PidfileRead::Parsed(Box::new(rec)),
            Err(e) if attempt == 0 => {
                tracing::trace!(error = %e, "pidfile torn read; retrying");
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                tracing::debug!(error = %e, path = %path.display(), "pidfile unparseable; skipped");
            }
        }
    }
    PidfileRead::Failed
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
        self.launch_flags(session_id)
            .and_then(|f| f.permission_mode)
    }

    /// What the live process's argv says (permission mode, effort, thinking,
    /// ultracode), read once per process.
    pub fn launch_flags(&self, session_id: &str) -> Option<LaunchFlags> {
        let g = self.lock();
        let s = g.live.get(session_id)?;
        let key = (s.pid?, s.proc_start.clone().unwrap_or_default());
        g.argv_flags.get(&key).cloned()
    }

    /// Never list processes running in `dir` as sessions (Mother Claude's
    /// own plan-usage helper). Both the path as given and its canonical form
    /// are matched.
    pub fn ignore_cwd(&self, dir: &Path) {
        let mut keys = vec![cwd_key(&dir.to_string_lossy())];
        if let Ok(canonical) = std::fs::canonicalize(dir) {
            keys.push(cwd_key(&canonical.to_string_lossy()));
        }
        let mut g = self.lock();
        for k in keys {
            if !g.ignored_cwds.contains(&k) {
                g.ignored_cwds.push(k);
            }
        }
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
        let (pid, source) = {
            let g = self.lock();
            let rec = g.live.get(session_id)?;
            (rec.pid?, rec.source.clone())
        };
        let path = source.unwrap_or_else(|| home.sessions_dir().join(format!("{pid}.json")));
        let PidfileRead::Parsed(rec) = read_pidfile(&path) else {
            return None;
        };
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
        self.refresh_homes(&[(DEFAULT_ACCOUNT.to_string(), home.clone())])
    }

    /// [`Self::refresh`] over every account's config directory (the Default
    /// first): each record and job is tagged with the account it was found
    /// under (docs/ACCOUNTS.md §5.2).
    pub fn refresh_homes(&self, homes: &[(String, ClaudeHome)]) -> LiveChange {
        // 1. Pidfiles (outside the lock: may sleep 50 ms on a torn read).
        let mut dir_seen = false;
        let mut seen_paths: HashSet<PathBuf> = HashSet::new();
        let mut to_parse: Vec<(PathBuf, (u64, i64), String)> = Vec::new();
        for (index, (account, home)) in homes.iter().enumerate() {
            let listing = std::fs::read_dir(home.sessions_dir()).ok();
            if index == 0 {
                // Old CLIs without pidfiles: judged by the Default's.
                dir_seen = listing.is_some();
            }
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
                if g.files.get(&path).map(|c| (c.stamp, c.account.as_str()))
                    != Some((st, account.as_str()))
                {
                    to_parse.push((path, st, account.clone()));
                }
            }
        }
        let parsed: Vec<(PathBuf, (u64, i64), String, PidfileRead)> = to_parse
            .into_iter()
            .map(|(path, st, account)| {
                let read = read_pidfile(&path);
                (path, st, account, read)
            })
            .collect();

        // 2. Jobs.
        let mut job_seen: HashSet<PathBuf> = HashSet::new();
        let mut job_parse: Vec<(PathBuf, String, (u64, i64), String)> = Vec::new();
        for (account, home) in homes {
            let g = self.lock();
            for entry in std::fs::read_dir(home.jobs_dir())
                .into_iter()
                .flatten()
                .flatten()
            {
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
                    job_parse.push((path, short, st, account.clone()));
                }
            }
        }
        let jobs_parsed: Vec<(PathBuf, CachedJob)> = job_parse
            .into_iter()
            .map(|(path, short, st, account)| {
                let state = read_job(&path, &short).map(|mut j| {
                    j.account = Some(account);
                    j
                });
                (path, CachedJob { stamp: st, state })
            })
            .collect();

        // 3. Candidates → liveness (ps outside the lock).
        let candidates: Vec<LiveSession> = {
            let mut g = self.lock();
            g.sessions_dir_seen = dir_seen;
            for (p, st, account, read) in parsed {
                match read {
                    PidfileRead::Parsed(rec) => {
                        let mut rec = *rec;
                        rec.home_account = Some(account.clone());
                        rec.source = Some(p.clone());
                        g.files.insert(
                            p,
                            CachedFile {
                                stamp: st,
                                record: Some(rec),
                                failures: 0,
                                account,
                            },
                        );
                    }
                    PidfileRead::Gone => {
                        g.files.remove(&p);
                    }
                    PidfileRead::Failed => {
                        // Keep the last good record: a torn rewrite must not
                        // report the session gone. Retry on the next refresh
                        // unless this keeps failing.
                        let prior = g.files.remove(&p);
                        let failures = prior.as_ref().map_or(0, |c| c.failures).saturating_add(1);
                        let stamp = if failures >= MAX_READ_FAILURES {
                            st
                        } else {
                            RETRY_STAMP
                        };
                        g.files.insert(
                            p,
                            CachedFile {
                                stamp,
                                record: prior.and_then(|c| c.record),
                                failures,
                                account,
                            },
                        );
                    }
                }
            }
            g.files.retain(|p, _| seen_paths.contains(p));
            for (p, c) in jobs_parsed {
                g.jobs_files.insert(p, c);
            }
            g.jobs_files.retain(|p, _| job_seen.contains(p));
            let ignored = &g.ignored_cwds;
            g.files
                .values()
                .filter_map(|c| c.record.clone())
                .filter(|r| !r.is_spare())
                .filter(|r| {
                    r.session_id
                        .as_deref()
                        .is_some_and(super::ids::is_session_id)
                })
                .filter(|r| {
                    !r.cwd
                        .as_deref()
                        .is_some_and(|c| ignored.contains(&cwd_key(c)))
                })
                .collect()
        };
        let mut alive: Vec<LiveSession> = Vec::new();
        let mut new_checks: Vec<((u32, String), bool)> = Vec::new();
        let mut new_argv: Vec<((u32, String), LaunchFlags, Option<String>)> = Vec::new();
        for mut rec in candidates {
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
                    g.argv_flags.contains_key(&key),
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
            if cached.1 {
                rec.token_sha256 = self.lock().proc_tokens.get(&key).cloned().flatten();
            } else {
                // The real argv, whole arguments; `ps`'s joined text only
                // when that cannot be read. The environment's token (D8) in
                // the same read.
                let facts = process_facts(pid);
                let flags = facts
                    .args
                    .map(|args| launch_flags_from_args(&args))
                    .or_else(|| ps_field(pid, "command").map(|c| launch_flags_from_argv(&c)))
                    .unwrap_or_default();
                rec.token_sha256 = facts.token_sha256.clone();
                new_argv.push((key, flags, facts.token_sha256));
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
        for (k, f, token) in new_argv {
            g.proc_tokens.insert(k.clone(), token);
            g.argv_flags.insert(k, f);
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
        // Forget identity checks of pidfiles that are gone. A negative check
        // (a stale pidfile whose pid was reused) is kept for its TTL while
        // the file exists, or `ps` would run again on every refresh.
        let key_of = |s: &LiveSession| Some((s.pid?, s.proc_start.clone().unwrap_or_default()));
        let file_keys: HashSet<(u32, String)> = g
            .files
            .values()
            .filter_map(|c| c.record.as_ref())
            .filter_map(key_of)
            .collect();
        let live_keys: HashSet<(u32, String)> = g.live.values().filter_map(key_of).collect();
        g.proc_checks.retain(|k, _| file_keys.contains(k));
        g.argv_flags.retain(|k, _| live_keys.contains(k));
        g.proc_tokens.retain(|k, _| live_keys.contains(k));
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
    fn pidfile_type_drift_degrades_per_field() {
        // Every field in an unexpected shape: the record still parses, the
        // drifted fields read loosely or as None.
        let s: LiveSession = serde_json::from_str(
            r#"{"pid":"42","sessionId":"abc","startedAt":1790338119067.5,
                "statusUpdatedAt":"1790338119999","updatedAt":true,
                "status":7,"cwd":["/x"],"peerFeatures":{"ask":true,"push":1},
                "procStart":null,"pidDomain":"darwin"}"#,
        )
        .unwrap();
        assert_eq!(s.pid, Some(42));
        assert_eq!(s.session_id.as_deref(), Some("abc"));
        assert_eq!(s.started_at, Some(1790338119067));
        assert_eq!(s.status_updated_at, Some(1790338119999));
        assert_eq!(s.updated_at, None);
        assert_eq!(s.status.as_deref(), Some("7"));
        assert_eq!(s.cwd, None);
        let mut pf = s.peer_features.clone().unwrap();
        pf.sort();
        assert_eq!(pf, vec!["ask".to_string(), "push".to_string()]);
        assert_eq!(s.proc_start, None);
        assert_eq!(s.pid_domain.as_deref(), Some("darwin"));
        // A negative or oversized pid is no pid.
        let s: LiveSession = serde_json::from_str(r#"{"pid":-1}"#).unwrap();
        assert_eq!(s.pid, None);
        let s: LiveSession = serde_json::from_str(r#"{"pid":99999999999}"#).unwrap();
        assert_eq!(s.pid, None);
        let s: LiveSession = serde_json::from_str(r#"{"peerFeatures":["a",1,"b"]}"#).unwrap();
        assert_eq!(
            s.peer_features,
            Some(vec!["a".to_string(), "b".to_string()])
        );
        // Round trip through our own serialization.
        let back: LiveSession = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn a_failed_reread_keeps_the_last_good_record() {
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let sessions = tmp.path().join("sessions");
        let me = std::process::id();
        let good =
            |status: &str| format!(r#"{{"pid":{me},"sessionId":"{SID}","status":"{status}"}}"#);
        write(&sessions, &format!("{me}.json"), &good("busy"));
        let roster = LiveRoster::new();
        assert_eq!(roster.refresh(&home).appeared, vec![SID.to_string()]);
        // A torn rewrite (both attempts): the session is not reported gone.
        write(&sessions, &format!("{me}.json"), "{\"pid\":");
        let change = roster.refresh(&home);
        assert!(change.gone.is_empty());
        assert!(change.transitions.is_empty());
        assert_eq!(
            roster.get(SID).and_then(|s| s.status).as_deref(),
            Some("busy")
        );
        // The rewrite completes: picked up (the failed stamp was not cached).
        write(&sessions, &format!("{me}.json"), &good("idle"));
        let change = roster.refresh(&home);
        assert_eq!(change.transitions.len(), 1);
        assert_eq!(change.transitions[0].to.as_deref(), Some("idle"));
        // A persistently broken file keeps the last record and stops being
        // re-read after a few attempts.
        write(&sessions, &format!("{me}.json"), "not json at all");
        for _ in 0..MAX_READ_FAILURES {
            assert!(roster.refresh(&home).gone.is_empty());
        }
        let stamp = roster.lock().files.values().next().map(|c| c.stamp);
        assert_ne!(stamp, Some(RETRY_STAMP));
        assert!(roster.is_live(SID));
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

    #[test]
    fn launch_flags_from_argv_cover_effort_thinking_and_ultracode() {
        // VS Code: no effort, thinking budget at launch.
        let vs = launch_flags_from_argv(
            "/x/claude --output-format stream-json --verbose --max-thinking-tokens 31999 \
             --permission-prompt-tool stdio --permission-mode default",
        );
        assert_eq!(vs.permission_mode.as_deref(), Some("default"));
        assert_eq!(vs.effort, None);
        assert_eq!(vs.thinking, Some(LaunchThinking::Budget(31_999)));
        assert_eq!(vs.ultracode, None);
        // Terminal with explicit flags (`=` forms, the last one wins).
        let t = launch_flags_from_argv(
            "claude --effort=high --effort max --thinking disabled --max-thinking-tokens 8000",
        );
        assert_eq!(t.effort.as_deref(), Some("max"));
        assert_eq!(
            t.thinking,
            Some(LaunchThinking::Off),
            "--thinking wins over a budget"
        );
        assert_eq!(
            launch_flags_from_argv("claude --thinking=adaptive").thinking,
            Some(LaunchThinking::Adaptive)
        );
        assert_eq!(
            launch_flags_from_argv("claude --thinking enabled").thinking,
            Some(LaunchThinking::Adaptive)
        );
        assert_eq!(
            launch_flags_from_argv("claude --max-thinking-tokens 0").thinking,
            Some(LaunchThinking::Off)
        );
        assert_eq!(
            launch_flags_from_argv("claude --max-thinking-tokens lots").thinking,
            None
        );
        assert_eq!(
            launch_flags_from_argv("claude --effort med")
                .effort
                .as_deref(),
            Some("medium")
        );
        assert_eq!(launch_flags_from_argv("claude --effort auto").effort, None);
        assert_eq!(launch_flags_from_argv("claude --effort turbo").effort, None);
        // Ultracode: `--effort ultracode` (xhigh) or an inline --settings.
        let u = launch_flags_from_argv("claude --effort ultracode");
        assert_eq!(u.effort.as_deref(), Some("xhigh"));
        assert_eq!(u.ultracode, Some(true));
        assert_eq!(
            launch_flags_from_argv(r#"claude --settings {"ultracode":true,"x":{"y":"}"}} -p"#)
                .ultracode,
            Some(true)
        );
        assert_eq!(
            launch_flags_from_argv(r#"claude --settings={"ultracode": false} --verbose"#).ultracode,
            Some(false)
        );
        // A settings file path is never opened; broken JSON is ignored.
        assert_eq!(
            launch_flags_from_argv("claude --settings /tmp/s.json").ultracode,
            None
        );
        assert_eq!(
            launch_flags_from_argv(r#"claude --settings {"ultracode":tru"#).ultracode,
            None
        );
        assert_eq!(launch_flags_from_argv(""), LaunchFlags::default());
        // The permission-mode reader keeps working through it.
        assert_eq!(
            permission_mode_from_argv("claude --dangerously-skip-permissions --effort max")
                .as_deref(),
            Some("bypassPermissions")
        );
        // Nothing after `--` is a flag, in the `ps` fallback too.
        assert_eq!(
            launch_flags_from_argv("claude --effort low -- --thinking disabled"),
            LaunchFlags {
                effort: Some("low".into()),
                ..Default::default()
            }
        );
    }

    #[test]
    fn real_argv_keeps_prompts_whole() {
        let args = |a: &[&str]| launch_flags_from_args(a);
        // A prompt that mentions flags is one argument: never a flag.
        assert_eq!(
            args(&[
                "claude",
                "document the --thinking disabled flag, --effort low"
            ]),
            LaunchFlags::default()
        );
        assert_eq!(
            args(&[
                "claude",
                r#"why does --settings {"ultracode":true} fail?"#,
                "--dangerously-skip-permissions-please"
            ]),
            LaunchFlags::default()
        );
        // Real flags: the value is the next argument, `=` forms too.
        let f = args(&[
            "/x/claude",
            "fix it",
            "--effort",
            "max",
            "--thinking=disabled",
            "--permission-mode",
            "plan",
            "--settings",
            r#"{ "ultracode": true, "x": "a b" }"#,
        ]);
        assert_eq!(f.effort.as_deref(), Some("max"));
        assert_eq!(f.thinking, Some(LaunchThinking::Off));
        assert_eq!(f.permission_mode.as_deref(), Some("plan"));
        assert_eq!(f.ultracode, Some(true));
        assert_eq!(
            args(&[
                "claude",
                "--settings=/tmp/s.json",
                "--max-thinking-tokens",
                "0"
            ]),
            LaunchFlags {
                thinking: Some(LaunchThinking::Off),
                ..Default::default()
            }
        );
        // After `--` everything is the prompt.
        assert_eq!(
            args(&["claude", "--effort", "high", "--", "--thinking", "disabled"]),
            LaunchFlags {
                effort: Some("high".into()),
                ..Default::default()
            }
        );
        // argv[0] is the program, never a flag.
        assert_eq!(
            args(&["--dangerously-skip-permissions"]),
            LaunchFlags::default()
        );
        let bypass = args(&["claude", "-p", "--dangerously-skip-permissions"]);
        assert_eq!(bypass.permission_mode.as_deref(), Some("bypassPermissions"));
    }

    #[test]
    fn the_token_in_a_process_environment_is_only_hashed() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/local/bin/claude\0\0\0");
        buf.extend_from_slice(
            b"claude\0-p\0HOME=/Users/me\0CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-x\0\0",
        );
        buf.extend_from_slice(b"ptr_munge=abc\0CLAUDE_CODE_OAUTH_TOKEN=apple-string\0");
        assert_eq!(
            env_sha256_in_procargs2(&buf, OAUTH_TOKEN_ENV),
            Some(crate::claude::secrets::sha256_hex(b"sk-ant-oat01-x")),
            "the environment ends at the empty string; the apple strings after it are not read"
        );
        // An argument that merely looks like the variable is not the environment.
        let mut args_only = 2i32.to_ne_bytes().to_vec();
        args_only.extend_from_slice(b"/bin/claude\0claude\0CLAUDE_CODE_OAUTH_TOKEN=nope\0\0");
        assert_eq!(env_sha256_in_procargs2(&args_only, OAUTH_TOKEN_ENV), None);
        // Unset or empty.
        let mut none = 1i32.to_ne_bytes().to_vec();
        none.extend_from_slice(b"/bin/claude\0claude\0CLAUDE_CODE_OAUTH_TOKEN=\0HOME=/x\0\0");
        assert_eq!(env_sha256_in_procargs2(&none, OAUTH_TOKEN_ENV), None);
        assert_eq!(env_sha256_in_procargs2(&[1, 0], OAUTH_TOKEN_ENV), None);
    }

    /// D8 on a real process: on Linux a child started with a token in its
    /// environment is recognised by the token's hash. (macOS 27 no longer
    /// returns another process's environment from KERN_PROCARGS2, our own
    /// children's included; there the hooks plugin reports the hash.)
    #[cfg(target_os = "linux")]
    #[test]
    fn a_childs_token_is_read_from_its_environment() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .env("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-child-dummy")
            .spawn()
            .unwrap();
        let mut facts = ProcFacts::default();
        for _ in 0..50 {
            facts = process_facts(child.id());
            if facts.token_sha256.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(
            facts.token_sha256,
            Some(crate::claude::secrets::sha256_hex(
                b"sk-ant-oat01-child-dummy"
            ))
        );
        assert_eq!(facts.args.unwrap()[1], "30");
    }

    #[test]
    fn records_from_several_homes_carry_their_account() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let sid_b = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        for (dir, sid) in [(a.path(), SID), (b.path(), sid_b)] {
            std::fs::create_dir_all(dir.join("sessions")).unwrap();
            std::fs::write(
                dir.join("sessions").join(format!("{me}.json")),
                format!(r#"{{"pid":{me},"sessionId":"{sid}","cwd":"/x","status":"idle"}}"#),
            )
            .unwrap();
            std::fs::create_dir_all(dir.join("jobs/abcd1234")).unwrap();
            std::fs::write(
                dir.join("jobs/abcd1234/state.json"),
                format!(r#"{{"sessionId":"{sid}","state":"working"}}"#),
            )
            .unwrap();
        }
        let roster = LiveRoster::new();
        let change = roster.refresh_homes(&[
            ("default".into(), ClaudeHome::with_base(a.path())),
            (
                "acc_0123456789ab".into(),
                ClaudeHome::for_account_dir(b.path()),
            ),
        ]);
        assert_eq!(change.appeared.len(), 2);
        let one = roster.get(SID).unwrap();
        assert_eq!(one.home_account.as_deref(), Some("default"));
        assert!(one.source.unwrap().starts_with(a.path()));
        assert_eq!(
            roster.get(sid_b).unwrap().home_account.as_deref(),
            Some("acc_0123456789ab")
        );
        assert_eq!(
            roster.job(sid_b).unwrap().account.as_deref(),
            Some("acc_0123456789ab")
        );
        // A home that goes away takes its sessions with it.
        let change = roster.refresh_homes(&[("default".into(), ClaudeHome::with_base(a.path()))]);
        assert_eq!(change.gone, vec![sid_b.to_string()]);
        assert!(roster.job(sid_b).is_none());
    }

    #[test]
    fn procargs2_buffers() {
        let mut buf = 3i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/local/bin/claude\0\0\0\0");
        buf.extend_from_slice(b"claude\0say --effort low\0\0HOME=/Users/me\0SECRET=x\0");
        assert_eq!(
            parse_procargs2(&buf),
            Some(vec![
                "claude".to_string(),
                "say --effort low".to_string(),
                String::new()
            ]),
            "the environment is never read"
        );
        assert_eq!(parse_procargs2(&[1, 0]), None);
        let mut short = 5i32.to_ne_bytes().to_vec();
        short.extend_from_slice(b"/bin/x\0x\0");
        assert_eq!(parse_procargs2(&short), None, "fewer arguments than argc");
        assert_eq!(parse_procargs2(&(-1i32).to_ne_bytes()), None);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn own_argv_is_read_whole() {
        let ours: Vec<String> = std::env::args().collect();
        assert_eq!(process_argv(std::process::id()), Some(ours));
        assert_eq!(process_argv(0), None);
    }

    #[test]
    fn processes_in_an_ignored_cwd_are_never_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let helper = tmp.path().join("app data/usage-helper");
        std::fs::create_dir_all(&helper).unwrap();
        let me = std::process::id();
        let helper_cwd = std::fs::canonicalize(&helper).unwrap();
        write(
            &tmp.path().join("sessions"),
            &format!("{me}.json"),
            &format!(
                r#"{{"pid":{me},"sessionId":"{SID}","cwd":"{}/","status":"idle","entrypoint":"sdk-cli"}}"#,
                helper_cwd.display()
            ),
        );
        let roster = LiveRoster::new();
        roster.ignore_cwd(&helper);
        let change = roster.refresh(&home);
        assert!(change.appeared.is_empty(), "{change:?}");
        assert!(!roster.is_live(SID));
        // Any other directory is a session as usual.
        write(
            &tmp.path().join("sessions"),
            &format!("{me}.json"),
            &format!(r#"{{"pid":{me},"sessionId":"{SID}","cwd":"/work/app","status":"idle"}}"#),
        );
        roster.refresh(&home);
        assert!(roster.is_live(SID));
        // Launch flags are read for live sessions only.
        assert!(roster.launch_flags(SID).is_some());
        assert!(roster
            .launch_flags("0a1b2c3d-0000-4000-8000-000000000000")
            .is_none());
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
    fn a_negative_identity_check_is_cached_while_the_pidfile_exists() {
        let me = std::process::id();
        if ps_field(me, "lstart").is_none() {
            return; // no ps in this environment
        }
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let sessions = tmp.path().join("sessions");
        // A stale pidfile whose pid now belongs to another process (us).
        write(
            &sessions,
            &format!("{me}.json"),
            &format!(
                r#"{{"pid":{me},"sessionId":"{SID}","pidDomain":"darwin","procStart":"Thu Jan  1 00:00:00 1970"}}"#
            ),
        );
        let roster = LiveRoster::new();
        roster.refresh(&home);
        assert!(!roster.is_live(SID));
        let key = (me, "Thu Jan  1 00:00:00 1970".to_string());
        let checked_at = roster
            .lock()
            .proc_checks
            .get(&key)
            .map(|(ok, at)| (*ok, *at));
        assert!(
            matches!(checked_at, Some((false, _))),
            "negative result kept"
        );
        // The next refresh reuses it rather than running `ps` again.
        roster.refresh(&home);
        let again = roster
            .lock()
            .proc_checks
            .get(&key)
            .map(|(ok, at)| (*ok, *at));
        assert_eq!(again, checked_at);
        // Once the pidfile is gone, so is the check.
        std::fs::remove_file(sessions.join(format!("{me}.json"))).unwrap();
        roster.refresh(&home);
        assert!(roster.lock().proc_checks.is_empty());
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
