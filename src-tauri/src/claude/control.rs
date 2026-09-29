//! Two-way control of **owned** sessions — sessions Mother Claude launches and
//! therefore can drive live (design §5.1).
//!
//! **Path B "full"** (the default): the installed `claude` in print mode with
//! the stream-json control protocol, launched the way VS Code launches it:
//!
//! ```text
//! claude -p --output-format stream-json --input-format stream-json --verbose
//!   --permission-prompt-tool stdio --include-partial-messages --replay-user-messages
//!   --setting-sources user,project,local --permission-mode M [--model X] [--effort E]
//!   [--allow-dangerously-skip-permissions]
//!   (--session-id <uuid> | --resume <id> | --resume <id> --fork-session --session-id <new>)
//! ```
//!
//! * The environment is scrubbed of the parent's Claude Code session variables
//!   (an explicit denylist, [`SCRUBBED_ENV`]) and of `MOTHER_CLAUDE_TOKEN`;
//!   `MOTHER_CLAUDE_OWNED=1` tells the foreign-control hook scripts to stay
//!   out. `CLAUDE_CONFIG_DIR` is never set (only inherited when the user set
//!   it) — setting it breaks the CLI's credential lookup.
//! * An `initialize` control request goes first; its commands/models/agents
//!   (and `fast_mode_state`, and what `account` says about the credentials)
//!   are cached. `get_settings` (local, no API call) follows it, every
//!   model/effort/ultracode change and each `result`, so the dashboard shows
//!   the effort and ultracode the CLI really applies; an answer that predates
//!   a change never speaks for it ([`SettingsSync`]: what was asked for shows
//!   as requested until then).
//! * `rate_limit_event` lines feed the plan usage (`claude::usage`) when the
//!   session runs on the machine's own plan.
//! * The stdout reader answers every CLI → host control request:
//!   `can_use_tool` becomes a pending item (permission, native AskUserQuestion
//!   with `answers`, ExitPlanMode with a `setMode`), `elicitation` too when a
//!   dashboard is connected (else declined), `request_user_dialog` is
//!   cancelled, anything else gets `error: unsupported`.
//!   `control_cancel_request` withdraws the card. Stream events become `stream`
//!   frames and activity; `system/status` updates the permission mode;
//!   `task_*` feed the agents view; `result` ends the turn (push trigger).
//! * Outgoing control requests carry a request id and are awaited with a
//!   timeout; CLI `error` replies surface as [`ControlError::Cli`] (a 422).
//! * The reaper withdraws pending items, fails in-flight requests and clears
//!   `owned` when the child exits.
//!
//! **Path A** (the Agent SDK sidecar, `sidecar/src/agent-bridge.ts`) is kept
//! behind `MOTHER_CLAUDE_SIDECAR=1`; it only takes text and answers
//! approvals over HTTP, so live control requests are refused for it.

pub mod protocol;
pub mod vt;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{oneshot, Mutex as AsyncMutex, Notify};
use uuid::Uuid;

pub use protocol::{applied_settings, catalog_adaptive_thinking, user_message_json};
pub use protocol::{AppliedSettings, CliError};

use super::model::{
    Activity, ActivityKind, AgentInfo, AgentKind, AgentStatus, BackgroundTask, BackgroundTaskKind,
    CommandInfo, Delivery, ImageInput, LastError, ModelInfo, NoticeLevel, QueueStatus, QueueVia,
    QueuedMessage, SessionStatus, StreamFrame, StreamKind,
};
use crate::pending;
use crate::server::ws;
use crate::state::AppState;

/// How long a user-triggered control request waits for the CLI.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the first `initialize` may take (the CLI boots first).
const INIT_TIMEOUT: Duration = Duration::from_secs(60);
/// An owned approval is held at most this long, then denied.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);
/// Text/thinking deltas are coalesced for this long before publishing.
const DELTA_FLUSH: Duration = Duration::from_millis(60);
/// Grace period between SIGTERM and SIGKILL when stopping a child.
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_secs(3);
/// VS Code's setting sources.
pub const SETTING_SOURCES: &str = "user,project,local";
/// `MAX_THINKING_TOKENS` a spawn with thinking "on" gets.
pub const SPAWN_THINKING_BUDGET: u32 = 16_000;
/// `set_max_thinking_tokens` budget the thinking switch turns on with (VS
/// Code's value; adaptive-capable models ignore the budget).
pub const THINKING_ON_BUDGET: u32 = 31_999;
/// stderr lines kept for the exit notice.
const STDERR_TAIL: usize = 20;

/// Inherited variables removed from every child Mother Claude spawns (an
/// explicit denylist: provider and auth variables such as
/// `CLAUDE_CODE_USE_BEDROCK` or `CLAUDE_CODE_OAUTH_TOKEN` must survive, or the
/// child fails the way an exported `CLAUDE_CONFIG_DIR` does). Everything here
/// identifies the *parent* Claude Code session (set when Mother Claude itself
/// runs from a Claude Code terminal) or is our own credential.
pub const SCRUBBED_ENV: [&str; 12] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_PID",
    "CLAUDE_AGENT_SDK_VERSION",
    "MOTHER_CLAUDE_TOKEN",
];

/// Non-secret marker on every child Mother Claude spawns; the foreign-control
/// hook scripts exit immediately when it is set (no double cards).
pub const OWNED_ENV: &str = "MOTHER_CLAUDE_OWNED";

/// Scrub a child's environment ([`SCRUBBED_ENV`], plus any inherited
/// [`OWNED_ENV`]) without marking it owned. Used for the lifecycle commands
/// (`claude stop|respawn|rm|logs`): they can cold-start a transient
/// background daemon that would pass the marker on to every background
/// session it runs, silencing their hooks.
pub fn scrub_child_env(cmd: &mut tokio::process::Command) {
    for key in SCRUBBED_ENV {
        cmd.env_remove(key);
    }
    cmd.env_remove(OWNED_ENV);
}

/// Scrub a child's environment ([`scrub_child_env`]) and mark it owned. Used
/// for the Path B `claude`, the Path A sidecar and the bridge host.
pub fn apply_child_env(cmd: &mut tokio::process::Command) {
    scrub_child_env(cmd);
    cmd.env(OWNED_ENV, "1");
}

/// Which machinery drives an owned child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Driver {
    /// Path B: `claude -p` stream-json with the control protocol.
    #[default]
    Stream,
    /// Path A: the Agent SDK sidecar (text + approvals only).
    Sidecar,
}

/// Options for launching an owned session.
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    pub cwd: String,
    /// First message (may be empty).
    pub prompt: String,
    pub model: Option<String>,
    /// Reasoning effort: low | medium | high | xhigh | max.
    pub effort: Option<String>,
    /// Thinking override: "on" | "off" (None = model/settings default).
    pub thinking: Option<String>,
    /// Always passed explicitly (`None` ⇒ `default`); never inherited.
    pub permission_mode: Option<String>,
    /// Continue an existing conversation by id.
    pub resume: Option<String>,
    /// With `resume`: `--fork-session` under a new id (the original is left
    /// untouched — never two writers on one transcript).
    pub fork: bool,
    /// The id to use (fresh: `--session-id`; fork: the new id). Generated when
    /// `None`.
    pub session_id: Option<String>,
    /// Images on the first message (validated by the caller).
    pub images: Vec<ImageInput>,
    /// Pass `--allow-dangerously-skip-permissions` (the spawning caller may use
    /// dangerous actions).
    pub allow_bypass: bool,
    /// Whether the spawning caller may use dangerous actions: if the session
    /// still comes up in bypassPermissions without it, it is stopped.
    pub caller_may_dangerous: bool,
    /// `--setting-sources` (default [`SETTING_SOURCES`]; tests narrow it).
    pub setting_sources: Option<String>,
    /// Extra CLI arguments (tests only; never from a request).
    pub extra_args: Vec<String>,
}

/// Lightweight metadata for a live owned session, so the registry can list it
/// even before its transcript file exists.
#[derive(Debug, Clone)]
pub struct OwnedSessionMeta {
    pub id: String,
    pub cwd: String,
    pub started_at: i64,
}

/// One background task / subagent the session reported (`task_*`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OwnedTask {
    pub id: String,
    pub tool_use_id: Option<String>,
    pub description: String,
    /// `local_bash` | `local_agent` | `remote_agent` | …
    pub task_type: Option<String>,
    pub subagent_type: Option<String>,
    pub background: bool,
    /// `running` | `completed` | `failed` | `stopped` | `killed` | `pending`.
    pub status: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub tokens: Option<u64>,
    pub tool_uses: Option<u64>,
    pub last_tool: Option<String>,
    pub summary: Option<String>,
    pub output_file: Option<String>,
    /// Claude Code's own housekeeping, not the session's work.
    pub ambient: bool,
}

impl OwnedTask {
    fn running(&self) -> bool {
        matches!(self.status.as_str(), "running" | "pending")
    }

    /// As an agent row (subagents only).
    pub fn agent(&self, session_id: &str) -> Option<AgentInfo> {
        if self.task_type.as_deref() != Some("local_agent")
            && self.task_type.as_deref() != Some("remote_agent")
        {
            return None;
        }
        let activity = self.last_tool.as_ref().map(|t| {
            let mut a = Activity::new(ActivityKind::Tool, format!("Running {t}"));
            a.tool = Some(t.clone());
            a
        });
        Some(AgentInfo {
            id: self.id.clone(),
            kind: AgentKind::Subagent,
            session_id: session_id.to_string(),
            depth: 1,
            agent_type: self.subagent_type.clone(),
            label: self
                .summary
                .clone()
                .unwrap_or_else(|| self.description.clone()),
            status: task_status(&self.status),
            background: self.background,
            started_at: Some(self.started_at),
            ended_at: self.ended_at,
            activity: if self.running() { activity } else { None },
            tokens: self.tokens,
            tool_uses: self.tool_uses,
            tool_use_id: self.tool_use_id.clone(),
            task_id: Some(self.id.clone()),
            can_stop: self.running(),
            ..Default::default()
        })
    }

    /// As a background task row.
    pub fn background_task(&self) -> BackgroundTask {
        BackgroundTask {
            id: self.id.clone(),
            kind: match self.task_type.as_deref() {
                Some("local_agent" | "remote_agent") => BackgroundTaskKind::Agent,
                Some("local_workflow" | "workflow") => BackgroundTaskKind::Workflow,
                Some("monitor") => BackgroundTaskKind::Monitor,
                _ => BackgroundTaskKind::Bash,
            },
            label: self.description.clone(),
            status: self.status.clone(),
            started_at: Some(self.started_at),
            ended_at: self.ended_at,
            output_available: self.output_file.as_deref().is_some_and(|f| !f.is_empty()),
        }
    }
}

fn task_status(s: &str) -> AgentStatus {
    match s {
        "running" | "pending" => AgentStatus::Running,
        "completed" => AgentStatus::Done,
        "failed" => AgentStatus::Failed,
        "stopped" | "killed" => AgentStatus::Stopped,
        _ => AgentStatus::Unknown,
    }
}

/// Thinking as Mother Claude set it on an owned session. Nothing in the CLI
/// reports it back, so this is what the host remembers it sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedThinking {
    Off,
    /// On with this budget (adaptive-capable models ignore it).
    On(u32),
}

impl OwnedThinking {
    /// From a spawn/continue `thinking` option (`"on"` / `"off"`).
    pub fn from_option(thinking: Option<&str>) -> Option<Self> {
        match thinking {
            Some("off") => Some(OwnedThinking::Off),
            Some("on") => Some(OwnedThinking::On(SPAWN_THINKING_BUDGET)),
            _ => None,
        }
    }
}

/// What Mother Claude knows about an owned session right now (from the
/// control protocol, not the transcript). Cloned out by [`ControlRegistry::state`].
#[derive(Debug, Clone, Default)]
pub struct OwnedLive {
    pub driver: Driver,
    pub cwd: String,
    pub started_at: i64,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    /// The effort Mother Claude last asked for (spawn option or change).
    pub effort: Option<String>,
    /// The last `get_settings.applied` (`None` until the CLI answered one).
    /// Parts of it are stale after a change it predates (see [`Self::sync`]).
    pub applied: Option<AppliedSettings>,
    /// Which of Mother Claude's setting changes `applied` reflects.
    pub sync: SettingsSync,
    /// The ultracode switch Mother Claude set that no `get_settings` answer
    /// reflects yet (cleared by the first one that does).
    pub ultracode_request: Option<bool>,
    /// Whose plan the session's requests go to, per `initialize.account`.
    pub account: Option<AccountFacts>,
    /// The session answered `get_usage` without plan usage: it runs on other
    /// credentials than the machine's plan (a project's own `env` or
    /// `apiKeyHelper`), so plan-usage reads go elsewhere.
    pub no_plan_usage: bool,
    /// Thinking as Mother Claude set it (`None` = the CLI default).
    pub thinking: Option<OwnedThinking>,
    /// `initialize` / `system/init` `fast_mode_state` (`on|off|cooldown`).
    pub fast_mode_state: Option<String>,
    /// From `initialize` / `commands_changed` (`/`-prefixed).
    pub commands: Vec<CommandInfo>,
    /// `initialize.models` (raw).
    pub models: Vec<Value>,
    /// `initialize.agents` (raw).
    pub agents: Vec<Value>,
    /// `initialize` answered.
    pub initialized: bool,
    /// `system/init` arrived.
    pub started: bool,
    /// The id `system/init` reported (differs from ours only on an old CLI).
    pub reported_session_id: Option<String>,
    pub capabilities: Vec<String>,
    pub turn_active: bool,
    pub turn_started_at: Option<i64>,
    pub activity: Activity,
    /// Our messages written while a turn ran and not answered yet.
    pub queued: Vec<QueuedMessage>,
    /// Ids in `queued` sent by callers without the dangerous right: taken
    /// back before the session switches to a mode without approvals.
    pub restricted_queued: Vec<String>,
    pub tasks: Vec<OwnedTask>,
    pub last_error: Option<LastError>,
    /// Launched with `--allow-dangerously-skip-permissions`.
    pub bypass_available: bool,
}

/// Mother Claude's setting changes on an owned session, counted, so a
/// `get_settings` answer that predates one is recognised: what was asked for
/// shows as requested until the CLI confirms it, even when the follow-up
/// `get_settings` fails or an older answer arrives late.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SettingsSync {
    /// Changes made so far.
    pub changes: u64,
    /// `changes` at the last model / effort / ultracode change.
    pub model_at: u64,
    pub effort_at: u64,
    pub ultracode_at: u64,
    /// `changes` when the `get_settings` behind `applied` was sent.
    pub applied_as_of: u64,
}

/// A setting change the CLI accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingChange {
    /// `set_model`: the applied effort and ultracode availability follow it.
    Model,
    /// `apply_flag_settings {effortLevel}` (ultracode kept).
    Effort,
    /// `apply_flag_settings {ultracode}` (the effort left alone).
    Ultracode,
}

/// What `initialize.account` says about an owned session's credentials. No
/// raw value is kept: the email only as a fingerprint to compare with the
/// machine's sign-in, in process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccountFacts {
    /// A third-party provider (`apiProvider` not `firstParty`) or an API key
    /// (`apiKeySource` other than `none`): no claude.ai plan behind it.
    pub not_plan: bool,
    /// [`crate::claude::usage::email_key`] of `account.email`, when reported.
    pub email_key: Option<u64>,
}

impl AccountFacts {
    /// From an `initialize` response (`None` without an `account` object).
    pub fn from_initialize(resp: &Value) -> Option<Self> {
        let a = resp.get("account")?.as_object()?;
        let text = |k: &str| a.get(k).and_then(Value::as_str).map(str::trim);
        let provider = text("apiProvider").filter(|p| !p.is_empty());
        let key_source = text("apiKeySource").filter(|s| !s.is_empty());
        Some(Self {
            not_plan: provider.is_some_and(|p| !p.eq_ignore_ascii_case("firstParty"))
                || key_source.is_some_and(|s| !s.eq_ignore_ascii_case("none")),
            email_key: text("email").and_then(crate::claude::usage::email_key),
        })
    }
}

impl OwnedLive {
    /// Record a setting change the CLI accepted (before `get_settings` is
    /// asked again).
    pub fn note_change(&mut self, change: SettingChange) {
        let s = &mut self.sync;
        s.changes += 1;
        match change {
            SettingChange::Model => s.model_at = s.changes,
            SettingChange::Effort => s.effort_at = s.changes,
            SettingChange::Ultracode => s.ultracode_at = s.changes,
        }
    }

    /// Keep a `get_settings` answer to a request sent after `as_of` changes.
    /// An answer older than the one kept is dropped. Returns whether what
    /// the summary shows may have changed.
    pub fn store_applied(&mut self, applied: AppliedSettings, as_of: u64) -> bool {
        if self.applied.is_some() && as_of < self.sync.applied_as_of {
            return false;
        }
        let changed = self.applied.as_ref() != Some(&applied) || self.sync.applied_as_of != as_of;
        self.applied = Some(applied);
        self.sync.applied_as_of = as_of;
        if as_of >= self.sync.ultracode_at {
            self.ultracode_request = None;
        }
        changed
    }

    /// `applied` when it reflects every change made up to `at`.
    fn applied_since(&self, at: u64) -> Option<&AppliedSettings> {
        self.applied
            .as_ref()
            .filter(|_| self.sync.applied_as_of >= at)
    }

    /// `applied`, for its effort: confirmed after every model and effort
    /// change.
    pub fn applied_effort(&self) -> Option<&AppliedSettings> {
        self.applied_since(self.sync.effort_at.max(self.sync.model_at))
    }

    /// `applied`, for ultracode: confirmed after every model change and
    /// ultracode switch.
    pub fn applied_ultracode(&self) -> Option<&AppliedSettings> {
        self.applied_since(self.sync.ultracode_at.max(self.sync.model_at))
    }

    /// `applied`, for the model: confirmed after every model change.
    pub fn applied_model(&self) -> Option<&AppliedSettings> {
        self.applied_since(self.sync.model_at)
    }

    /// Ultracode is on or requested: an effort change must keep it (an
    /// `effortLevel` sent without the key turns it off). That includes a
    /// request the model cannot run — the switch stays offered to withdraw
    /// it ([`Self::ultracode_switchable`]).
    pub fn ultracode_wanted(&self) -> bool {
        self.ultracode_request.unwrap_or_else(|| {
            self.applied
                .as_ref()
                .is_some_and(|a| a.ultracode || a.ultracode_requested)
        })
    }

    /// The ultracode switch has something to do: the CLI can run it, or it
    /// is requested (a model that cannot run it leaves the request standing,
    /// and only the switch withdraws it).
    pub fn ultracode_switchable(&self) -> bool {
        self.ultracode_request == Some(true)
            || self
                .applied
                .as_ref()
                .is_some_and(|a| a.ultracode_available || a.ultracode_requested)
    }

    /// Running subagents and all tasks, for the agents views.
    pub fn agents_of(&self, session_id: &str) -> Vec<AgentInfo> {
        self.tasks
            .iter()
            .filter(|t| !t.ambient)
            .filter_map(|t| t.agent(session_id))
            .collect()
    }

    pub fn background_tasks(&self) -> Vec<BackgroundTask> {
        self.tasks
            .iter()
            .filter(|t| !t.ambient && t.task_type.as_deref() != Some("local_agent"))
            .map(OwnedTask::background_task)
            .collect()
    }
}

/// Why a control operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("session {0} is not an owned, controllable session")]
    NotOwned(String),
    #[error("{0}")]
    NotSupported(String),
    #[error("{}", .0.message)]
    Cli(CliError),
    #[error("the session did not answer within {}s", .0.as_secs())]
    Timeout(Duration),
    #[error("the session's process has exited")]
    Closed,
    #[error("could not write to the session: {0}")]
    Io(#[from] std::io::Error),
}

type Reply = Result<Value, CliError>;

/// A CLI → host request we are answering.
struct Incoming {
    cancel: Option<oneshot::Sender<()>>,
}

struct OwnedHandle {
    id: String,
    cwd: String,
    started_at: i64,
    driver: Driver,
    caller_may_dangerous: bool,
    stdin: AsyncMutex<Option<ChildStdin>>,
    kill: Notify,
    alive: AtomicBool,
    live: Mutex<OwnedLive>,
    outgoing: Mutex<HashMap<String, oneshot::Sender<Reply>>>,
    incoming: Mutex<HashMap<String, Incoming>>,
    stderr_tail: Mutex<VecDeque<String>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl OwnedHandle {
    fn live(&self) -> MutexGuard<'_, OwnedLive> {
        lock(&self.live)
    }

    async fn write_line(&self, line: &str) -> std::io::Result<()> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdin is closed")
        })?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await
    }

    async fn write_json(&self, v: &Value) -> std::io::Result<()> {
        self.write_line(&v.to_string()).await
    }

    /// Write a control request and return the receiver of its reply.
    async fn send_request(
        &self,
        request: Value,
    ) -> Result<(String, oneshot::Receiver<Reply>), ControlError> {
        if self.driver == Driver::Sidecar {
            return Err(ControlError::NotSupported(
                "This session runs through the Agent SDK sidecar (MOTHER_CLAUDE_SIDECAR=1), \
                 which does not take live control requests."
                    .into(),
            ));
        }
        if !self.alive.load(Ordering::SeqCst) {
            return Err(ControlError::Closed);
        }
        let rid = format!("mc_{}", Uuid::new_v4().simple());
        let (tx, rx) = oneshot::channel();
        lock(&self.outgoing).insert(rid.clone(), tx);
        if let Err(e) = self
            .write_json(&protocol::control_request(&rid, request))
            .await
        {
            lock(&self.outgoing).remove(&rid);
            return Err(e.into());
        }
        Ok((rid, rx))
    }

    async fn request(&self, request: Value, timeout: Duration) -> Result<Value, ControlError> {
        let (rid, rx) = self.send_request(request).await?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(ControlError::Cli(e)),
            Ok(Err(_)) => Err(ControlError::Closed),
            Err(_) => {
                lock(&self.outgoing).remove(&rid);
                Err(ControlError::Timeout(timeout))
            }
        }
    }
}

type HandleMap = Arc<Mutex<HashMap<String, Arc<OwnedHandle>>>>;

/// Registry of live owned-session subprocess handles.
#[derive(Default)]
pub struct ControlRegistry {
    handles: HandleMap,
    /// Mirror of `state.owned` (bridge threads included) for sync lookups.
    owned_mirror: Mutex<HashSet<String>>,
}

impl ControlRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// True if we hold a live control handle for this id.
    pub fn controls(&self, id: &str) -> bool {
        lock(&self.handles).contains_key(id)
    }

    /// Owned by Mother Claude in any way: a control handle (Path A or B) or a
    /// session marked owned (HTTP bridge conversations). Sync.
    pub fn is_owned(&self, id: &str) -> bool {
        self.controls(id) || lock(&self.owned_mirror).contains(id)
    }

    /// Record that `state.owned` gained / lost an id (called by
    /// `Inner::mark_owned` / `unmark_owned`).
    pub fn note_owned(&self, id: &str, owned: bool) {
        let mut m = lock(&self.owned_mirror);
        if owned {
            m.insert(id.to_string());
        } else {
            m.remove(id);
        }
    }

    /// Metadata for every live owned session (for registry inclusion).
    pub fn live(&self) -> Vec<OwnedSessionMeta> {
        lock(&self.handles)
            .iter()
            .map(|(id, h)| OwnedSessionMeta {
                id: id.clone(),
                cwd: h.cwd.clone(),
                started_at: h.started_at,
            })
            .collect()
    }

    /// The working directory of an owned session.
    pub fn cwd_of(&self, id: &str) -> Option<String> {
        self.get(id).map(|h| h.cwd.clone())
    }

    /// What the control protocol told us about an owned session.
    pub fn state(&self, id: &str) -> Option<OwnedLive> {
        self.get(id).map(|h| h.live().clone())
    }

    /// Which machinery drives an owned session.
    pub fn driver(&self, id: &str) -> Option<Driver> {
        self.get(id).map(|h| h.driver)
    }

    /// What the setting switches of an owned session depend on, without
    /// cloning its whole state: `(driven by the sidecar, the ultracode switch
    /// has something to do — see [`OwnedLive::ultracode_switchable`])`.
    pub fn switch_facts(&self, id: &str) -> Option<(bool, bool)> {
        self.get(id)
            .map(|h| (h.driver == Driver::Sidecar, h.live().ultracode_switchable()))
    }

    /// A live, initialised Path B session that can answer a control request
    /// on the account's behalf (plan usage), if any: the newest one whose
    /// credentials `plan_ok` accepts and that never answered without plan
    /// usage.
    pub fn usage_session(&self, plan_ok: impl Fn(Option<AccountFacts>) -> bool) -> Option<String> {
        let handles: Vec<Arc<OwnedHandle>> = lock(&self.handles).values().cloned().collect();
        handles
            .into_iter()
            .filter(|h| h.driver == Driver::Stream && h.alive.load(Ordering::SeqCst))
            .filter(|h| {
                let live = h.live();
                live.initialized && !live.no_plan_usage && plan_ok(live.account)
            })
            .max_by_key(|h| h.started_at)
            .map(|h| h.id.clone())
    }

    /// Re-read an owned session's applied settings (`get_settings`) and
    /// publish what changed. A no-op for the sidecar or a gone session.
    pub async fn refresh_settings(&self, state: &AppState, id: &str) {
        if let Some(h) = self.get(id) {
            refresh_settings(state, &h).await;
        }
    }

    fn get(&self, id: &str) -> Option<Arc<OwnedHandle>> {
        lock(&self.handles).get(id).cloned()
    }

    fn handle(&self, id: &str) -> Result<Arc<OwnedHandle>, ControlError> {
        self.get(id)
            .ok_or_else(|| ControlError::NotOwned(id.to_string()))
    }

    /// Launch an owned session and return its id (the new id for a fork).
    pub async fn spawn(&self, state: &AppState, opts: SpawnOptions) -> Result<String> {
        let id = match (&opts.session_id, &opts.resume) {
            (Some(sid), _) => sid.clone(),
            (None, Some(resume)) if !opts.fork => resume.clone(),
            _ => Uuid::new_v4().to_string(),
        };
        let permission_mode = opts
            .permission_mode
            .clone()
            .unwrap_or_else(|| "default".into());

        // One controller per session: a second spawn/continue would clobber
        // this handle and its reaper would then orphan the original child.
        if self.controls(&id) {
            anyhow::bail!("session {id} is already controlled by Mother Claude");
        }

        // Path A only when explicitly requested, and only for plain fresh
        // text sessions (it has no resume/fork/image support worth keeping).
        let entry = if opts.resume.is_none() && opts.images.is_empty() {
            sidecar_entry()
        } else {
            None
        };
        let mut driver = Driver::Stream;
        let mut child = match entry {
            Some(entry) => {
                tracing::info!(session = %id, "spawning owned session via sidecar (Path A)");
                let url = server_url(state);
                match sidecar_command(
                    &entry,
                    &id,
                    &opts,
                    &permission_mode,
                    &state.auth.sidecar_token,
                    &url,
                )
                .spawn()
                {
                    Ok(child) => {
                        driver = Driver::Sidecar;
                        child
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "sidecar failed to start; using Path B");
                        headless_command(&id, &opts, &permission_mode)
                            .spawn()
                            .context("failed to spawn claude (Path B fallback)")?
                    }
                }
            }
            None => headless_command(&id, &opts, &permission_mode)
                .spawn()
                .with_context(|| format!("failed to spawn `{}`", crate::claude::claude_bin()))?,
        };

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let started_at = crate::claude::registry::now_ms();

        let handle = Arc::new(OwnedHandle {
            id: id.clone(),
            cwd: opts.cwd.clone(),
            started_at,
            driver,
            caller_may_dangerous: opts.caller_may_dangerous,
            stdin: AsyncMutex::new(stdin),
            kill: Notify::new(),
            alive: AtomicBool::new(true),
            live: Mutex::new(OwnedLive {
                driver,
                cwd: opts.cwd.clone(),
                started_at,
                model: opts.model.clone(),
                permission_mode: Some(permission_mode.clone()),
                effort: opts.effort.clone(),
                thinking: OwnedThinking::from_option(opts.thinking.as_deref()),
                bypass_available: opts.allow_bypass,
                activity: Activity {
                    since: Some(started_at),
                    ..Activity::new(ActivityKind::Starting, "Starting")
                },
                ..Default::default()
            }),
            outgoing: Mutex::new(HashMap::new()),
            incoming: Mutex::new(HashMap::new()),
            stderr_tail: Mutex::new(VecDeque::new()),
        });

        {
            let mut map = lock(&self.handles);
            if map.contains_key(&id) {
                drop(map);
                let _ = child.start_kill();
                anyhow::bail!("session {id} is already controlled by Mother Claude");
            }
            map.insert(id.clone(), handle.clone());
        }
        // Owned before any output is read: a permission card or hook event
        // for this id must already see it as ours (and find its row).
        state.mark_owned(&id, &opts.cwd, started_at).await;

        if let Some(stderr) = stderr {
            let h = handle.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(session = %h.id, "claude stderr: {line}");
                    let mut tail = lock(&h.stderr_tail);
                    tail.push_back(line.chars().take(400).collect());
                    while tail.len() > STDERR_TAIL {
                        tail.pop_front();
                    }
                }
            });
        }
        let reader =
            stdout.map(|stdout| tokio::spawn(read_stdout(state.clone(), handle.clone(), stdout)));
        self.spawn_reaper(state, handle.clone(), child, reader);

        // `initialize` goes first so the CLI processes it before any message.
        if driver == Driver::Stream {
            match handle
                .send_request(json!({ "subtype": "initialize" }))
                .await
            {
                Ok((rid, rx)) => {
                    let h = handle.clone();
                    let st = state.clone();
                    tokio::spawn(async move {
                        match tokio::time::timeout(INIT_TIMEOUT, rx).await {
                            Ok(Ok(Ok(resp))) => {
                                on_initialized(&h, &resp);
                                // What the CLI will actually send (effort,
                                // ultracode) — no API call.
                                refresh_settings(&st, &h).await;
                            }
                            Ok(Ok(Err(e))) => {
                                tracing::warn!(session = %h.id, error = %e.message, "initialize refused")
                            }
                            _ => {
                                lock(&h.outgoing).remove(&rid);
                                tracing::warn!(session = %h.id, "initialize got no answer");
                            }
                        }
                    });
                }
                Err(e) => tracing::warn!(session = %id, error = %e, "could not send initialize"),
            }
        }

        // The first message goes over stdin like every other (a prompt as
        // argv could be parsed as a flag). The sidecar takes it from its env.
        if driver == Driver::Stream && (!opts.prompt.trim().is_empty() || !opts.images.is_empty()) {
            let uuid = Uuid::new_v4().to_string();
            if let Err(e) = write_user(&handle, &opts.prompt, &opts.images, &uuid).await {
                tracing::warn!(session = %id, error = %e, "could not write the first message");
            }
        } else if driver == Driver::Sidecar && !opts.prompt.trim().is_empty() {
            start_turn(&handle);
        }

        let live = handle.live().clone();
        let always_thinking = crate::claude::settings::always_thinking_enabled(&state.home);
        ws::patch_session(state, &id, |s| {
            s.permission_mode = live.permission_mode.clone();
            if let Some(m) = &live.model {
                s.model = Some(model_info(m));
            }
            crate::claude::owned::overlay_settings(s, &live, always_thinking);
        })
        .await;
        let verb = match (&opts.resume, opts.fork) {
            (Some(_), true) => "Forked",
            (Some(_), false) => "Continuing",
            (None, _) => "Spawned",
        };
        ws::publish_notice(
            state,
            NoticeLevel::Info,
            format!("{verb} owned session {id}"),
            Some(&id),
        );
        Ok(id)
    }

    fn spawn_reaper(
        &self,
        state: &AppState,
        handle: Arc<OwnedHandle>,
        mut child: Child,
        reader: Option<tokio::task::JoinHandle<()>>,
    ) {
        let st = state.clone();
        let map = self.handles.clone();
        tokio::spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s,
                _ = handle.kill.notified() => {
                    terminate(&mut child).await
                }
            };
            // Let the reader drain what the child wrote before it exited (its
            // last result), so nothing lands after the session is marked
            // ended. Bounded: a grandchild could hold the pipe open.
            if let Some(reader) = reader {
                let _ = tokio::time::timeout(Duration::from_secs(2), reader).await;
            }
            let sid = handle.id.clone();
            tracing::info!(session = %sid, ?status, "owned session exited");
            handle.alive.store(false, Ordering::SeqCst);
            handle.stdin.lock().await.take();
            // Fail in-flight requests and stop waiting on CLI requests.
            lock(&handle.outgoing).clear();
            for (_, inc) in lock(&handle.incoming).drain() {
                if let Some(c) = inc.cancel {
                    let _ = c.send(());
                }
            }
            {
                let mut live = handle.live();
                live.turn_active = false;
                for q in &mut live.queued {
                    q.status = QueueStatus::Failed;
                    q.cancellable = false;
                }
                live.activity = Activity::new(ActivityKind::Ended, "Ended");
            }
            let mine = {
                let mut m = lock(&map);
                let mine = m.get(&sid).is_some_and(|h| Arc::ptr_eq(h, &handle));
                if mine {
                    m.remove(&sid);
                }
                mine
            };
            if mine {
                // No longer drivable: clear ownership and withdraw its pending
                // prompts so no card waits on a dead process.
                st.unmark_owned(&sid).await;
            }
            let failed = status.as_ref().map(|s| !s.success()).unwrap_or(true);
            let tail: Vec<String> = lock(&handle.stderr_tail).iter().cloned().collect();
            let started = handle.live().started;
            if failed && !started && !tail.is_empty() {
                let text = tail.join("\n");
                ws::publish_notice(
                    &st,
                    NoticeLevel::Error,
                    format!("Session {sid} could not start: {text}"),
                    Some(&sid),
                );
                crate::server::push::on_session_failed(&st, &sid, &text);
            } else {
                ws::publish_notice(
                    &st,
                    NoticeLevel::Info,
                    format!("Session {sid} exited"),
                    Some(&sid),
                );
            }
        });
    }

    /// Send a user message (text + images) to an owned session. Returns the
    /// message id (its stream-json `uuid`) and whether it was sent now or
    /// queued behind the running turn.
    pub async fn send_user(
        &self,
        id: &str,
        text: &str,
        images: &[ImageInput],
    ) -> Result<(String, Delivery), ControlError> {
        let handle = self.handle(id)?;
        if handle.driver == Driver::Sidecar && !images.is_empty() {
            return Err(ControlError::NotSupported(
                "Images are not supported by the Agent SDK sidecar.".into(),
            ));
        }
        if !handle.alive.load(Ordering::SeqCst) {
            return Err(ControlError::Closed);
        }
        let uuid = Uuid::new_v4().to_string();
        let delivery = write_user(&handle, text, images, &uuid).await?;
        Ok((uuid, delivery))
    }

    /// Send a text message to an owned session's stdin (stream-json input).
    pub async fn send_message(&self, id: &str, text: &str) -> Result<()> {
        self.send_user(id, text, &[])
            .await
            .map(|_| ())
            .map_err(|e| anyhow!("{e}"))
    }

    /// Write a raw NDJSON line to an owned session's stdin.
    pub async fn send_raw(&self, id: &str, line: &str) -> Result<()> {
        let handle = self
            .get(id)
            .ok_or_else(|| anyhow!("session {id} is not controllable"))?;
        handle.write_line(line).await?;
        Ok(())
    }

    /// Send a control request (`{"subtype":…,…}`) and await its reply.
    pub async fn request(
        &self,
        id: &str,
        request: Value,
        timeout: Duration,
    ) -> Result<Value, ControlError> {
        self.handle(id)?.request(request, timeout).await
    }

    /// Update the cached live state after a successful control request.
    pub fn update<F: FnOnce(&mut OwnedLive)>(&self, id: &str, f: F) {
        if let Some(h) = self.get(id) {
            f(&mut h.live());
        }
    }

    /// Terminate an owned session's subprocess (SIGTERM, then SIGKILL after a
    /// grace period). Returns immediately; the reaper cleans up.
    pub async fn kill(&self, id: &str) -> Result<()> {
        if let Some(handle) = self.get(id) {
            handle.kill.notify_one();
        }
        Ok(())
    }
}

/// SIGTERM, wait a little, then SIGKILL.
pub(crate) async fn terminate(child: &mut Child) -> std::io::Result<std::process::ExitStatus> {
    terminate_within(child, KILL_GRACE).await
}

/// SIGTERM, wait up to `grace`, then SIGKILL.
pub(crate) async fn terminate_within(
    child: &mut Child,
    grace: Duration,
) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(unix)]
    if let Some(pid) = child.id().and_then(|p| i32::try_from(p).ok()) {
        // SAFETY: plain syscall on our own child's pid; no memory is touched.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        if let Ok(status) = tokio::time::timeout(grace, child.wait()).await {
            return status;
        }
    }
    #[cfg(not(unix))]
    let _ = grace;
    let _ = child.start_kill();
    child.wait().await
}

/// Write a user message line and do the queue/turn bookkeeping.
async fn write_user(
    h: &Arc<OwnedHandle>,
    text: &str,
    images: &[ImageInput],
    uuid: &str,
) -> Result<Delivery, ControlError> {
    let line = match h.driver {
        // The sidecar flattens content to text and has no uuid tracking.
        Driver::Sidecar => protocol::user_message_json(text),
        Driver::Stream => {
            let mut content = Vec::with_capacity(1 + images.len());
            if !text.trim().is_empty() {
                content.push(protocol::text_block(text));
            }
            for img in images {
                content.push(protocol::image_block(&img.media_type, &img.data));
            }
            protocol::user_message(content, Some(uuid)).to_string()
        }
    };
    // Decide queued-vs-sent and record the message BEFORE writing, so a fast
    // answer cannot race the bookkeeping.
    let queued = {
        let mut live = h.live();
        let queued = live.turn_active || !live.queued.is_empty();
        if h.driver == Driver::Stream && queued {
            live.queued.push(QueuedMessage {
                id: uuid.to_string(),
                text: text.chars().take(2000).collect(),
                created_at: crate::claude::registry::now_ms(),
                via: QueueVia::Owned,
                status: QueueStatus::Queued,
                cancellable: true,
            });
        }
        queued
    };
    if let Err(e) = h.write_line(&line).await {
        h.live().queued.retain(|q| q.id != uuid);
        return Err(e.into());
    }
    start_turn(h);
    Ok(if queued {
        Delivery::Queued
    } else {
        Delivery::Sent
    })
}

fn start_turn(h: &OwnedHandle) {
    let mut live = h.live();
    if !live.turn_active {
        live.turn_active = true;
        live.turn_started_at = Some(crate::claude::registry::now_ms());
    }
}

// ─────────────────────────────── stdout reader ────────────────────────────────

/// A text/thinking delta being coalesced.
struct PendingDelta {
    frame: StreamFrame,
    since: Instant,
}

async fn read_stdout(state: AppState, h: Arc<OwnedHandle>, stdout: ChildStdout) {
    let mut lines = BufReader::new(stdout).lines();
    let mut tracker = protocol::StreamTracker::default();
    let mut delta: Option<PendingDelta> = None;
    loop {
        let next = if delta.is_some() {
            // `next_line` is cancel safe, so a timeout only flushes.
            match tokio::time::timeout(DELTA_FLUSH, lines.next_line()).await {
                Ok(r) => r,
                Err(_) => {
                    flush_delta(&state, &h, &mut delta);
                    continue;
                }
            }
        } else {
            lines.next_line().await
        };
        let line = match next {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(session = %h.id, error = %e, "owned session stdout failed");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(session = %h.id, error = %e, "non-JSON line from claude");
                continue;
            }
        };
        dispatch(&state, &h, v, &mut tracker, &mut delta).await;
    }
    flush_delta(&state, &h, &mut delta);
}

fn flush_delta(state: &AppState, h: &OwnedHandle, delta: &mut Option<PendingDelta>) {
    if let Some(d) = delta.take() {
        ws::publish_stream(state, &h.id, d.frame);
    }
}

fn publish_frame(
    state: &AppState,
    h: &OwnedHandle,
    frame: StreamFrame,
    delta: &mut Option<PendingDelta>,
) {
    let coalescable =
        matches!(frame.kind, StreamKind::Text | StreamKind::Thinking) && frame.delta.is_some();
    if !coalescable {
        flush_delta(state, h, delta);
        ws::publish_stream(state, &h.id, frame);
        return;
    }
    if let Some(d) = delta.as_mut() {
        if d.frame.kind == frame.kind
            && d.frame.message_id == frame.message_id
            && d.frame.index == frame.index
        {
            if let (Some(acc), Some(more)) = (d.frame.delta.as_mut(), frame.delta.as_deref()) {
                acc.push_str(more);
            }
            if d.since.elapsed() >= DELTA_FLUSH {
                flush_delta(state, h, delta);
            }
            return;
        }
    }
    flush_delta(state, h, delta);
    *delta = Some(PendingDelta {
        frame,
        since: Instant::now(),
    });
}

async fn dispatch(
    state: &AppState,
    h: &Arc<OwnedHandle>,
    v: Value,
    tracker: &mut protocol::StreamTracker,
    delta: &mut Option<PendingDelta>,
) {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or_default();
    match kind {
        "control_request" => {
            flush_delta(state, h, delta);
            // Register the cancel slot of a request that waits for a human
            // *before* spawning its handler: a `control_cancel_request`
            // buffered right behind it is dispatched before the spawned task
            // first runs, and must still find it.
            let cancel = match cli_request_ids(&v) {
                Some((rid, true)) => {
                    let mut incoming = lock(&h.incoming);
                    if incoming.contains_key(&rid) {
                        return; // duplicate delivery of an in-flight request
                    }
                    let (tx, rx) = oneshot::channel();
                    incoming.insert(rid, Incoming { cancel: Some(tx) });
                    Some(rx)
                }
                _ => None,
            };
            let (st, hh) = (state.clone(), h.clone());
            tokio::spawn(async move { handle_cli_request(&st, &hh, v, cancel).await });
        }
        "control_cancel_request" => {
            if let Some(rid) = v.get("request_id").and_then(Value::as_str) {
                if let Some(inc) = lock(&h.incoming).remove(rid) {
                    if let Some(c) = inc.cancel {
                        let _ = c.send(());
                    }
                }
            }
        }
        "control_response" => {
            if let Some((rid, reply)) = protocol::parse_control_response(&v) {
                if let Some(tx) = lock(&h.outgoing).remove(&rid) {
                    let _ = tx.send(reply);
                }
            }
        }
        "stream_event" => {
            mark_answered(state, h, &v).await;
            if let Some(event) = v.get("event") {
                let out = tracker.on_event(event);
                for frame in out.frames {
                    publish_frame(state, h, frame, delta);
                }
                if let Some(a) = out.activity {
                    note_activity(state, h, a).await;
                }
            }
        }
        "system" => {
            flush_delta(state, h, delta);
            on_system(state, h, &v).await;
        }
        "assistant" => {
            flush_delta(state, h, delta);
            mark_answered(state, h, &v).await;
            on_assistant(state, h, &v).await;
        }
        "user" => {
            // Tool results (and, with --replay-user-messages, our own message
            // echoed back): the model is thinking again.
            let is_tool_result = v
                .pointer("/message/content")
                .and_then(Value::as_array)
                .is_some_and(|c| {
                    c.iter()
                        .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
                });
            if is_tool_result && h.live().turn_active {
                note_activity(state, h, Activity::new(ActivityKind::Thinking, "Thinking")).await;
            }
        }
        "result" => {
            flush_delta(state, h, delta);
            mark_answered(state, h, &v).await;
            on_result(state, h, &v).await;
            // The turn may have changed what the CLI applies (a `/model`, the
            // per-turn effort latch): re-read it off the reader task, which
            // must keep draining stdout for the answer to arrive.
            let (st, hh) = (state.clone(), h.clone());
            tokio::spawn(async move { refresh_settings(&st, &hh).await });
        }
        // Plan usage for free: the CLI emits one when a window's rounded
        // percentage or reset time moves (5-hour and weekly only) — from a
        // session on the machine's own plan only.
        "rate_limit_event" => {
            if let Some(update) = crate::claude::usage::parse_rate_limit_event(&v) {
                let account = h.live().account;
                if crate::claude::usage::plan_session_ok(state, account) {
                    crate::claude::usage::merge_stream(state, update);
                } else {
                    tracing::debug!(session = %h.id, "rate_limit_event from other credentials ignored");
                }
            }
        }
        _ => {}
    }
}

/// Ask the CLI what it applies (`get_settings`: local, no API call), cache
/// `applied` and publish the summary fields it changes. Must never run on the
/// stdout reader task (it awaits a reply that task delivers).
async fn refresh_settings(state: &AppState, h: &Arc<OwnedHandle>) {
    if h.driver != Driver::Stream || !h.alive.load(Ordering::SeqCst) {
        return;
    }
    // Every change counted now was answered by the CLI before this request
    // goes out, so the answer reflects it (a failed or late answer leaves
    // what was asked for showing as requested: `OwnedLive::sync`).
    let as_of = h.live().sync.changes;
    let resp = match h
        .request(json!({ "subtype": "get_settings" }), REQUEST_TIMEOUT)
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::debug!(session = %h.id, error = %e, "get_settings failed");
            return;
        }
    };
    let Some(applied) = protocol::applied_settings(&resp) else {
        tracing::debug!(session = %h.id, "get_settings answered without `applied`");
        return;
    };
    let changed = h.live().store_applied(applied, as_of);
    if changed {
        crate::claude::owned::publish_settings(state, &h.id).await;
    }
}

/// Our messages a frame says it answers are no longer queued.
async fn mark_answered(state: &AppState, h: &OwnedHandle, v: &Value) {
    let answered = protocol::answered_uuids(v);
    if answered.is_empty() {
        return;
    }
    let changed = {
        let mut live = h.live();
        let before = live.queued.len();
        live.queued.retain(|q| !answered.contains(&q.id));
        before != live.queued.len()
    };
    if changed {
        let n = h.live().queued.len() as u32;
        ws::patch_session(state, &h.id, |s| s.queued = n).await;
    }
}

async fn note_activity(state: &AppState, h: &OwnedHandle, mut a: Activity) {
    let now = crate::claude::registry::now_ms();
    let changed = {
        let mut live = h.live();
        let same = live.activity.kind == a.kind
            && live.activity.label == a.label
            && live.activity.tool == a.tool;
        if same {
            false
        } else {
            a.since = Some(now);
            live.activity = a.clone();
            true
        }
    };
    if !changed {
        return;
    }
    state.activity.note_owned(&h.id, a.clone());
    let turn_active = h.live().turn_active;
    ws::patch_session(state, &h.id, |s| {
        if s.pending.iter().any(|p| p.answerable) {
            return; // waiting on a human wins
        }
        s.activity = a;
        s.status = if turn_active {
            SessionStatus::Working
        } else {
            SessionStatus::Idle
        };
        s.last_activity_at = Some(now);
    })
    .await;
}

fn model_info(id: &str) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        label: id.to_string(),
        context_window: None,
    }
}

fn on_initialized(h: &OwnedHandle, resp: &Value) {
    {
        let mut live = h.live();
        live.initialized = true;
        let commands = protocol::commands_from(resp);
        if !commands.is_empty() {
            live.commands = commands;
        }
        if let Some(models) = resp.get("models").and_then(Value::as_array) {
            live.models = models.clone();
        }
        if let Some(agents) = resp.get("agents").and_then(Value::as_array) {
            live.agents = agents.clone();
        }
        if let Some(mode) = resp.get("current_permission_mode").and_then(Value::as_str) {
            live.permission_mode = Some(mode.to_string());
        }
        if let Some(fast) = resp.get("fast_mode_state").and_then(Value::as_str) {
            live.fast_mode_state = Some(fast.chars().take(20).collect());
        }
        // A project's own credentials (`env`, `apiKeyHelper`) are not the
        // machine's plan: plan usage never comes from such a session.
        live.account = AccountFacts::from_initialize(resp);
    }
    tracing::debug!(session = %h.id, "owned session initialized");
}

async fn on_system(state: &AppState, h: &Arc<OwnedHandle>, v: &Value) {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    match v.get("subtype").and_then(Value::as_str).unwrap_or_default() {
        "init" => {
            let mode = text("permissionMode");
            let model = text("model");
            {
                let mut live = h.live();
                live.started = true;
                if mode.is_some() {
                    live.permission_mode = mode.clone();
                }
                if model.is_some() {
                    live.model = model.clone();
                }
                live.reported_session_id = text("session_id");
                if let Some(fast) = text("fast_mode_state") {
                    live.fast_mode_state = Some(fast.chars().take(20).collect());
                }
                if let Some(caps) = v.get("capabilities").and_then(Value::as_array) {
                    live.capabilities = caps
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect();
                }
            }
            if let Some(reported) = text("session_id").filter(|r| r != &h.id) {
                tracing::warn!(session = %h.id, %reported, "the CLI reported a different session id");
            }
            // A caller without dangerous rights must never end up driving a
            // session that runs tools unapproved (e.g. a user-settings
            // defaultMode of bypassPermissions leaking through).
            if mode.as_deref() == Some("bypassPermissions") && !h.caller_may_dangerous {
                tracing::warn!(session = %h.id, "owned session came up in bypassPermissions; stopping it");
                ws::publish_notice(
                    state,
                    NoticeLevel::Warning,
                    "Stopped a session that started without permission prompts: the device \
                     that launched it may not use dangerous actions.",
                    Some(&h.id),
                );
                pending::withdraw_session(state, &h.id, "session stopped (bypassPermissions)")
                    .await;
                h.kill.notify_one();
                return;
            }
            if let Some(m) = &mode {
                state.activity.note_permission_mode(&h.id, m);
            }
            ws::patch_session(state, &h.id, |s| {
                if let Some(m) = &mode {
                    s.permission_mode = Some(m.clone());
                }
                if let Some(m) = &model {
                    s.model = Some(model_info(m));
                }
                if let Some(v) = v.get("claude_code_version").and_then(Value::as_str) {
                    s.version = Some(v.to_string());
                }
            })
            .await;
        }
        "status" => {
            let mode = text("permissionMode");
            if let Some(m) = &mode {
                h.live().permission_mode = Some(m.clone());
                state.activity.note_permission_mode(&h.id, m);
                let m = m.clone();
                ws::patch_session(state, &h.id, |s| s.permission_mode = Some(m)).await;
            }
            let status = text("status");
            if status.as_deref() == Some("compacting") {
                note_activity(
                    state,
                    h,
                    Activity::new(ActivityKind::Compacting, "Compacting"),
                )
                .await;
            } else if v.get("status").is_some_and(Value::is_null) && h.live().turn_active {
                note_activity(state, h, Activity::new(ActivityKind::Thinking, "Thinking")).await;
            }
            // Only a real status (e.g. `compacting`); `null` just means the
            // turn runs normally and would render as an empty bubble.
            if let Some(status) = status {
                ws::publish_stream(
                    state,
                    &h.id,
                    StreamFrame {
                        kind: StreamKind::Status,
                        status: Some(status),
                        ..Default::default()
                    },
                );
            }
        }
        "api_retry" => {
            note_activity(state, h, Activity::new(ActivityKind::Retrying, "Retrying")).await;
        }
        "commands_changed" => {
            let commands = protocol::commands_from(v);
            if !commands.is_empty() {
                h.live().commands = commands;
            }
        }
        "thinking_tokens" => mark_answered(state, h, v).await,
        sub @ ("task_started"
        | "task_progress"
        | "task_updated"
        | "task_notification"
        | "background_tasks_changed") => {
            on_task(h, sub, v);
            publish_agents(state, h);
        }
        _ => {}
    }
}

/// Merge one `task_*` message into the task list.
fn on_task(h: &OwnedHandle, sub: &str, v: &Value) {
    let now = crate::claude::registry::now_ms();
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let mut live = h.live();
    if sub == "background_tasks_changed" {
        let listed: Vec<Value> = v
            .get("tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for t in &listed {
            let Some(id) = t.get("task_id").and_then(Value::as_str) else {
                continue;
            };
            let entry = task_entry(&mut live.tasks, id, now);
            if let Some(d) = t.get("description").and_then(Value::as_str) {
                entry.description = d.to_string();
            }
            entry.ambient = t.get("ambient").and_then(Value::as_bool).unwrap_or(false);
            entry.background = true;
            if entry.status.is_empty() {
                entry.status = "running".into();
            }
        }
        return;
    }
    let Some(id) = text("task_id") else {
        return;
    };
    let entry = task_entry(&mut live.tasks, &id, now);
    if let Some(d) = text("description") {
        entry.description = d;
    }
    if let Some(t) = text("tool_use_id") {
        entry.tool_use_id = Some(t);
    }
    if let Some(a) = v.get("ambient").and_then(Value::as_bool) {
        entry.ambient = a;
    }
    let usage = v.get("usage");
    let num = |k: &str| usage.and_then(|u| u.get(k)).and_then(Value::as_u64);
    match sub {
        "task_started" => {
            entry.task_type = text("task_type");
            entry.background = v
                .get("is_backgrounded")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            entry.status = "running".into();
            entry.started_at = now;
        }
        "task_progress" => {
            entry.subagent_type = text("subagent_type").or(entry.subagent_type.take());
            entry.last_tool = text("last_tool_name").or(entry.last_tool.take());
            entry.summary = text("summary").or(entry.summary.take());
            entry.tokens = num("total_tokens").or(entry.tokens);
            entry.tool_uses = num("tool_uses").or(entry.tool_uses);
            if entry.status.is_empty() {
                entry.status = "running".into();
            }
        }
        "task_updated" => {
            if let Some(patch) = v.get("patch") {
                if let Some(s) = patch.get("status").and_then(Value::as_str) {
                    entry.status = s.to_string();
                }
                if let Some(d) = patch.get("description").and_then(Value::as_str) {
                    entry.description = d.to_string();
                }
                if let Some(b) = patch.get("is_backgrounded").and_then(Value::as_bool) {
                    entry.background = b;
                }
                if let Some(e) = patch.get("end_time").and_then(Value::as_i64) {
                    entry.ended_at = Some(e);
                }
            }
        }
        "task_notification" => {
            entry.status = text("status").unwrap_or_else(|| "completed".into());
            entry.summary = text("summary").or(entry.summary.take());
            entry.output_file = text("output_file").filter(|f| !f.is_empty());
            entry.tokens = num("total_tokens").or(entry.tokens);
            entry.tool_uses = num("tool_uses").or(entry.tool_uses);
            entry.ended_at.get_or_insert(now);
        }
        _ => {}
    }
    // Bound memory: keep every running task and the 50 most recent others.
    if live.tasks.len() > 100 {
        let mut finished: Vec<usize> = live
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.running())
            .map(|(i, _)| i)
            .collect();
        let excess = live.tasks.len() - 100;
        finished.truncate(excess);
        for i in finished.into_iter().rev() {
            live.tasks.remove(i);
        }
    }
}

fn task_entry<'a>(tasks: &'a mut Vec<OwnedTask>, id: &str, now: i64) -> &'a mut OwnedTask {
    if let Some(pos) = tasks.iter().position(|t| t.id == id) {
        return &mut tasks[pos];
    }
    tasks.push(OwnedTask {
        id: id.to_string(),
        started_at: now,
        ..Default::default()
    });
    let last = tasks.len() - 1;
    &mut tasks[last]
}

/// Publish the session's agents: the data layer's view (from disk) merged with
/// what the control protocol reported, so owned subagents show up live.
fn publish_agents(state: &AppState, h: &OwnedHandle) {
    let (mut agents, workflows) = state.agents.for_session(&h.id);
    let mine = h.live().agents_of(&h.id);
    for a in mine {
        let dup = agents
            .iter()
            .any(|b| b.id == a.id || (a.tool_use_id.is_some() && b.tool_use_id == a.tool_use_id));
        if !dup {
            agents.push(a);
        }
    }
    ws::publish_agents(state, &h.id, agents, workflows);
}

async fn on_assistant(state: &AppState, h: &OwnedHandle, v: &Value) {
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        let text = v
            .pointer("/message/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or(err)
            .chars()
            .take(500)
            .collect::<String>();
        let at = crate::claude::registry::now_ms();
        h.live().last_error = Some(LastError {
            kind: err.to_string(),
            text: text.clone(),
            at,
        });
        let kind = err.to_string();
        ws::patch_session(state, &h.id, |s| {
            s.last_error = Some(LastError { kind, text, at });
        })
        .await;
    }
    // Without partial messages (the sidecar) this is the only activity hint.
    let last_tool = v
        .pointer("/message/content")
        .and_then(Value::as_array)
        .and_then(|c| {
            c.iter()
                .rev()
                .find(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        })
        .and_then(|b| b.get("name").and_then(Value::as_str))
        .map(str::to_string);
    if let Some(name) = last_tool {
        let mut a = Activity::new(ActivityKind::Tool, format!("Running {name}"));
        a.tool = Some(name);
        note_activity(state, h, a).await;
    }
    // Claude asked to notify the user (PushNotification tool): push it now,
    // keyed by the tool-use id so the transcript copy is not pushed again.
    for b in v
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter(|b| b.get("name").and_then(Value::as_str) == Some("PushNotification"))
    {
        crate::server::push::on_claude_notification(
            state,
            &h.id,
            b.pointer("/input/message").and_then(Value::as_str),
            b.get("id").and_then(Value::as_str),
        );
    }
    if let Some(m) = v.pointer("/message/model").and_then(Value::as_str) {
        if m != "<synthetic>" {
            h.live().model = Some(m.to_string());
        }
    }
}

async fn on_result(state: &AppState, h: &OwnedHandle, v: &Value) {
    let now = crate::claude::registry::now_ms();
    let (worked_ms, still_busy) = {
        let mut live = h.live();
        // `turn_started_at` is the start of the busy period: queued messages
        // chain turns without it being reset.
        let started = live.turn_started_at.unwrap_or(now);
        let wall = u64::try_from(now - started).unwrap_or(0);
        let worked = v
            .get("duration_ms")
            .and_then(Value::as_u64)
            .map_or(wall, |d| d.max(wall));
        // Messages still queued start the next turn right away.
        let busy = !live.queued.is_empty();
        live.turn_active = busy;
        if !busy {
            live.turn_started_at = None;
        } else if live.turn_started_at.is_none() {
            live.turn_started_at = Some(now);
        }
        (worked, busy)
    };
    let subtype = v.get("subtype").and_then(Value::as_str).unwrap_or_default();
    let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    if is_error && subtype != "error_during_execution" {
        let text = v
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or(subtype)
            .chars()
            .take(500)
            .collect::<String>();
        let kind = if subtype.is_empty() { "error" } else { subtype }.to_string();
        let at = now;
        h.live().last_error = Some(LastError {
            kind: kind.clone(),
            text: text.clone(),
            at,
        });
        ws::patch_session(state, &h.id, |s| {
            s.last_error = Some(LastError {
                kind,
                text: text.clone(),
                at,
            })
        })
        .await;
        crate::server::push::on_session_failed(state, &h.id, &text);
    }
    // Only when the session is really done: queued messages keep it working,
    // and the push fired at the end covers the whole busy period.
    if !still_busy {
        crate::server::push::on_turn_finished(state, &h.id, worked_ms);
    }
    if still_busy {
        note_activity(state, h, Activity::new(ActivityKind::Thinking, "Thinking")).await;
    } else {
        note_activity(state, h, Activity::new(ActivityKind::Idle, "Idle")).await;
    }
    let queued = h.live().queued.len() as u32;
    ws::patch_session(state, &h.id, |s| s.queued = queued).await;
    let name = state
        .find_session(&h.id)
        .await
        .map(|s| s.name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("Session {}", h.id.chars().take(8).collect::<String>()));
    ws::publish_notice(
        state,
        NoticeLevel::Success,
        format!("{name} finished a turn"),
        Some(&h.id),
    );
}

// ───────────────────────── CLI → host control requests ─────────────────────────

enum Outcome {
    Decided(pending::Decision),
    Cancelled,
    Gone,
    TimedOut,
}

/// The request id of a CLI → host `control_request`, and whether it waits
/// for a human (`can_use_tool`, `elicitation`: those can be cancelled).
fn cli_request_ids(v: &Value) -> Option<(String, bool)> {
    let rid = v.get("request_id").and_then(Value::as_str)?.to_string();
    let subtype = v
        .pointer("/request/subtype")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some((rid, matches!(subtype, "can_use_tool" | "elicitation")))
}

async fn handle_cli_request(
    state: &AppState,
    h: &Arc<OwnedHandle>,
    v: Value,
    cancel: Option<oneshot::Receiver<()>>,
) {
    let Some(rid) = v
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let req = v.get("request").cloned().unwrap_or(Value::Null);
    let subtype = req
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let reply = match (subtype.as_str(), cancel) {
        ("can_use_tool", Some(cancel)) => {
            let item = protocol::item_from_can_use_tool(&h.id, &req);
            let tool_use_id = req.get("tool_use_id").cloned();
            match ask_human(state, h, &rid, item.clone(), cancel).await {
                Some(Outcome::Decided(d)) => {
                    let mut out = pending::permission_result(&item, &d);
                    if let Some(t) = tool_use_id.filter(|t| !t.is_null()) {
                        out["toolUseID"] = t;
                    }
                    Some(protocol::control_success(&rid, out))
                }
                Some(Outcome::TimedOut) => Some(protocol::control_success(
                    &rid,
                    json!({ "behavior": "deny", "message": "Nobody answered in time." }),
                )),
                Some(Outcome::Gone) => Some(protocol::control_success(
                    &rid,
                    json!({ "behavior": "deny", "message": "The request was withdrawn in Mother Claude." }),
                )),
                // The CLI cancelled it (or the process is gone): no reply.
                Some(Outcome::Cancelled) | None => None,
            }
        }
        ("elicitation", Some(cancel)) => {
            // Only worth a card when someone can see it.
            if state.hub.client_count() == 0 {
                lock(&h.incoming).remove(&rid);
                Some(protocol::control_success(
                    &rid,
                    json!({ "action": "decline" }),
                ))
            } else {
                let item = protocol::item_from_elicitation(&h.id, &req);
                match ask_human(state, h, &rid, item.clone(), cancel).await {
                    Some(Outcome::Decided(d)) => Some(protocol::control_success(
                        &rid,
                        pending::elicitation_result(&d),
                    )),
                    Some(Outcome::TimedOut | Outcome::Gone) => Some(protocol::control_success(
                        &rid,
                        json!({ "action": "decline" }),
                    )),
                    Some(Outcome::Cancelled) | None => None,
                }
            }
        }
        // Hosts must answer unknown dialog kinds with `cancelled`.
        ("request_user_dialog", _) => Some(protocol::control_success(
            &rid,
            json!({ "behavior": "cancelled" }),
        )),
        (other, _) => {
            tracing::debug!(session = %h.id, subtype = %other, "unsupported CLI control request");
            Some(protocol::control_error(&rid, "unsupported"))
        }
    };
    if let Some(reply) = reply {
        if let Err(e) = h.write_json(&reply).await {
            tracing::debug!(session = %h.id, error = %e, "could not answer a control request");
        }
    }
}

/// Raise a pending item for CLI request `rid` and wait for a human.
/// `cancel_rx` is the slot [`dispatch`] registered in `incoming` before this
/// handler was spawned. `None` when the process is gone.
async fn ask_human(
    state: &AppState,
    h: &Arc<OwnedHandle>,
    rid: &str,
    item: pending::PendingItem,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Option<Outcome> {
    // Cancelled (or the process exited) before this task first ran: no card.
    let already = !matches!(
        cancel_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    if already || !h.alive.load(Ordering::SeqCst) {
        lock(&h.incoming).remove(rid);
        return h.alive.load(Ordering::SeqCst).then_some(Outcome::Cancelled);
    }
    let (guard, rx) = pending::raise(state, item).await;
    let pending_id = guard.id().to_string();
    let outcome = tokio::select! {
        d = tokio::time::timeout(PROMPT_TIMEOUT, rx) => match d {
            Ok(Ok(d)) => Outcome::Decided(d),
            Ok(Err(_)) => Outcome::Gone,
            Err(_) => Outcome::TimedOut,
        },
        _ = cancel_rx => Outcome::Cancelled,
    };
    lock(&h.incoming).remove(rid);
    match &outcome {
        Outcome::Cancelled => {
            pending::withdraw(state, &pending_id, "cancelled by the session").await;
        }
        Outcome::TimedOut => {
            pending::withdraw(state, &pending_id, "timeout").await;
        }
        _ => {}
    }
    drop(guard);
    if matches!(outcome, Outcome::Cancelled) && !h.alive.load(Ordering::SeqCst) {
        return None;
    }
    Some(outcome)
}

// ────────────────────────────── spawn commands ───────────────────────────────

/// Whether foreign-session injection (PTY-driving `claude attach`) is available.
/// Requires the `experimental` capability to be compiled in (on by default) and
/// is enabled at runtime unless `MOTHER_CLAUDE_FOREIGN_INJECTION` is `0`/off.
pub fn foreign_injection_enabled() -> bool {
    if !cfg!(feature = "experimental") {
        return false;
    }
    match std::env::var("MOTHER_CLAUDE_FOREIGN_INJECTION") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "off" || v == "no")
        }
        Err(_) => true,
    }
}

/// Whether Path A (the SDK sidecar) is requested: `MOTHER_CLAUDE_SIDECAR=1`.
/// Path B is the default.
pub fn sidecar_requested() -> bool {
    std::env::var("MOTHER_CLAUDE_SIDECAR")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "on" || v == "yes"
        })
        .unwrap_or(false)
}

/// The built sidecar entry point, when Path A is requested.
/// `MOTHER_CLAUDE_SIDECAR_PATH` (set at startup for packaged builds) wins.
fn sidecar_entry() -> Option<PathBuf> {
    if !sidecar_requested() {
        return None;
    }
    if let Ok(custom) = std::env::var("MOTHER_CLAUDE_SIDECAR_PATH") {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }
    let candidates = [
        std::env::current_dir()
            .ok()
            .map(|d| d.join("sidecar/dist/agent-bridge.js")),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sidecar/dist/agent-bridge.js")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

/// Loopback URL the sidecar uses to reach this server. The loopback listener
/// is always plain HTTP (TLS is only on the LAN addresses), so this is always
/// `http://` (design G3).
fn server_url(state: &AppState) -> String {
    format!("http://127.0.0.1:{}", state.config.port)
}

/// Shared stdio/env configuration for both spawn paths. No Mother Claude
/// credential is ever inherited by a `claude` child (design §2.5).
fn configure_common(cmd: &mut tokio::process::Command, cwd: &str) {
    apply_child_env(cmd);
    cmd.current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
}

/// The Path B argument list (without the binary).
pub fn headless_args(id: &str, opts: &SpawnOptions, permission_mode: &str) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--verbose",
        "--permission-prompt-tool",
        "stdio",
        "--include-partial-messages",
        "--replay-user-messages",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push("--setting-sources".into());
    a.push(
        opts.setting_sources
            .clone()
            .unwrap_or_else(|| SETTING_SOURCES.into()),
    );
    a.push("--permission-mode".into());
    a.push(permission_mode.to_string());
    if opts.allow_bypass {
        a.push("--allow-dangerously-skip-permissions".into());
    }
    if let Some(model) = &opts.model {
        a.push("--model".into());
        a.push(model.clone());
    }
    if let Some(effort) = &opts.effort {
        a.push("--effort".into());
        a.push(effort.clone());
    }
    match (&opts.resume, opts.fork) {
        (Some(resume), true) => {
            // The CLI: "--session-id can only be used with --continue or
            // --resume if --fork-session is also specified" — so the new id is
            // ours up front and marked owned before any hook fires.
            a.extend(["--resume".into(), resume.clone(), "--fork-session".into()]);
            a.extend(["--session-id".into(), id.to_string()]);
        }
        (Some(resume), false) => a.extend(["--resume".into(), resume.clone()]),
        (None, _) => a.extend(["--session-id".into(), id.to_string()]),
    }
    a.extend(opts.extra_args.iter().cloned());
    a
}

/// Path B: headless `claude -p` stream-json subprocess.
fn headless_command(
    id: &str,
    opts: &SpawnOptions,
    permission_mode: &str,
) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(crate::claude::claude_bin());
    c.args(headless_args(id, opts, permission_mode));
    // The MAX_THINKING_TOKENS env is what the CLI reads at startup (0
    // disables, a positive budget enables; adaptive-capable models ignore
    // the budget). Mid-session changes go through `set_max_thinking_tokens`.
    match OwnedThinking::from_option(opts.thinking.as_deref()) {
        Some(OwnedThinking::Off) => {
            c.env("MAX_THINKING_TOKENS", "0");
        }
        Some(OwnedThinking::On(budget)) => {
            c.env("MAX_THINKING_TOKENS", budget.to_string());
        }
        None => {}
    }
    // NOTE: the initial prompt is deliberately NOT passed as argv — it goes
    // over stdin as the first stream-json user message (see spawn()).
    configure_common(&mut c, &opts.cwd);
    c
}

/// Path A: the Node Agent SDK sidecar (canUseTool + ask_user).
fn sidecar_command(
    entry: &Path,
    id: &str,
    opts: &SpawnOptions,
    permission_mode: &str,
    sidecar_token: &str,
    server_url: &str,
) -> tokio::process::Command {
    let mut c = tokio::process::Command::new("node");
    c.arg(entry)
        .env("MC_SESSION_ID", id)
        .env("MC_CWD", &opts.cwd)
        .env("MC_PROMPT", &opts.prompt)
        .env("MC_PERMISSION_MODE", permission_mode)
        .env("MOTHER_CLAUDE_URL", server_url);
    if let Some(model) = &opts.model {
        c.env("MC_MODEL", model);
    }
    if let Some(effort) = &opts.effort {
        c.env("MC_EFFORT", effort);
    }
    if let Some(thinking) = &opts.thinking {
        c.env("MC_THINKING", thinking);
    }
    if let Some(resume) = &opts.resume {
        c.env("MC_RESUME", resume);
    }
    configure_common(&mut c, &opts.cwd);
    // The sidecar's own credential (`mcs_…`): valid only for
    // `/api/sessions/{id}/permission-request` from this machine. The sidecar
    // removes it from its environment and passes the SDK an explicit env
    // without it, so the `claude` child never sees it.
    c.env("MOTHER_CLAUDE_TOKEN", sidecar_token);
    c
}

// ──────────────────────────── background sessions ─────────────────────────────

/// `claude stop|respawn|rm|logs` take only the SHORT background job id (8 hex
/// chars, or a unique prefix of it); a session UUID never matches.
pub fn is_short_job_id(id: &str) -> bool {
    super::ids::is_short_job_id(id)
}

const LIFECYCLE_ACTIONS: [&str; 4] = ["stop", "respawn", "rm", "logs"];

async fn run_job_command(action: &str, short_id: &str) -> Result<std::process::Output> {
    if !LIFECYCLE_ACTIONS.contains(&action) {
        anyhow::bail!("unknown lifecycle action {action:?}");
    }
    if !is_short_job_id(short_id) {
        anyhow::bail!("{short_id:?} is not a background job id");
    }
    let mut cmd = tokio::process::Command::new(crate::claude::claude_bin());
    cmd.arg(action)
        .arg(short_id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Not owned: never mark a lifecycle command (see `scrub_child_env`).
    scrub_child_env(&mut cmd);
    tokio::time::timeout(Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| anyhow!("`claude {action} {short_id}` timed out"))?
        .with_context(|| format!("failed to run `claude {action} {short_id}`"))
}

/// Run a documented lifecycle subcommand (`stop` / `respawn` / `rm`) against a
/// background job by its SHORT id. Returns stdout on success.
pub async fn run_lifecycle(action: &str, short_id: &str) -> Result<String> {
    let out = run_job_command(action, short_id).await?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        Err(anyhow!("`claude {action} {short_id}` failed: {stderr}"))
    }
}

/// Width × height of the grid `claude logs` snapshots are replayed onto.
pub const LOGS_COLS: usize = 200;
pub const LOGS_ROWS: usize = 50;

/// The current screen of a background job (`claude logs <short>`), rendered
/// to plain lines (never raw terminal bytes).
pub async fn logs_screen(short_id: &str) -> Result<vt::Screen> {
    let out = run_job_command("logs", short_id).await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        anyhow::bail!("`claude logs {short_id}` failed: {stderr}");
    }
    Ok(vt::render(&out.stdout, LOGS_COLS, LOGS_ROWS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_empty() {
        let r = ControlRegistry::new();
        assert!(!r.controls("anything"));
        assert!(!r.is_owned("anything"));
        r.note_owned("bridge-thread", true);
        assert!(r.is_owned("bridge-thread"));
        assert!(!r.controls("bridge-thread"));
        r.note_owned("bridge-thread", false);
        assert!(!r.is_owned("bridge-thread"));
    }

    #[test]
    fn child_env_is_scrubbed_by_denylist() {
        let mut cmd = tokio::process::Command::new("true");
        cmd.env("CLAUDE_CODE_USE_BEDROCK", "1")
            .env("CLAUDE_CODE_OAUTH_TOKEN", "x");
        apply_child_env(&mut cmd);
        let envs: HashMap<String, Option<String>> = cmd
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        for key in SCRUBBED_ENV {
            assert_eq!(envs.get(key), Some(&None), "{key} must be removed");
        }
        assert_eq!(envs.get(OWNED_ENV), Some(&Some("1".to_string())));
        // Provider/auth variables survive; CLAUDE_CONFIG_DIR is never touched.
        assert_eq!(
            envs.get("CLAUDE_CODE_USE_BEDROCK"),
            Some(&Some("1".to_string()))
        );
        assert_eq!(
            envs.get("CLAUDE_CODE_OAUTH_TOKEN"),
            Some(&Some("x".to_string()))
        );
        assert!(!envs.contains_key("CLAUDE_CONFIG_DIR"));
    }

    #[test]
    fn lifecycle_commands_are_never_marked_owned() {
        // `claude stop|respawn|rm|logs` can cold-start a transient daemon; a
        // marker it inherited would silence every background session's hooks.
        let mut cmd = tokio::process::Command::new("true");
        cmd.env(OWNED_ENV, "1").env("CLAUDE_CODE_USE_BEDROCK", "1");
        scrub_child_env(&mut cmd);
        let envs: HashMap<String, Option<String>> = cmd
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert_eq!(envs.get(OWNED_ENV), Some(&None), "marker removed, not set");
        for key in SCRUBBED_ENV {
            assert_eq!(envs.get(key), Some(&None), "{key} must be removed");
        }
        assert_eq!(
            envs.get("CLAUDE_CODE_USE_BEDROCK"),
            Some(&Some("1".to_string()))
        );
    }

    #[test]
    fn headless_args_mirror_vs_code() {
        let opts = SpawnOptions {
            cwd: "/tmp".into(),
            model: Some("haiku".into()),
            effort: Some("low".into()),
            ..Default::default()
        };
        let a = headless_args("11111111-2222-3333-4444-555555555555", &opts, "plan");
        let s = a.join(" ");
        assert!(
            s.starts_with("-p --output-format stream-json --input-format stream-json --verbose")
        );
        assert!(s.contains("--permission-prompt-tool stdio"));
        assert!(s.contains("--include-partial-messages --replay-user-messages"));
        assert!(s.contains("--setting-sources user,project,local"));
        assert!(s.contains("--permission-mode plan"));
        assert!(s.contains("--model haiku --effort low"));
        assert!(s.ends_with("--session-id 11111111-2222-3333-4444-555555555555"));
        assert!(!s.contains("--allow-dangerously-skip-permissions"));

        let fork = SpawnOptions {
            resume: Some("old".into()),
            fork: true,
            allow_bypass: true,
            ..Default::default()
        };
        let s = headless_args("new", &fork, "default").join(" ");
        assert!(s.contains("--allow-dangerously-skip-permissions"));
        assert!(s.contains("--resume old --fork-session --session-id new"));

        let resume = SpawnOptions {
            resume: Some("old".into()),
            ..Default::default()
        };
        let s = headless_args("old", &resume, "default").join(" ");
        assert!(s.ends_with("--resume old"));
        assert!(!s.contains("--session-id"));
    }

    #[test]
    fn short_job_ids() {
        assert!(is_short_job_id("631b387d"));
        assert!(is_short_job_id("631b38"));
        assert!(!is_short_job_id("631b387d-a7e5-4e12-ab38-522cb612803b"));
        assert!(!is_short_job_id("--help"));
        assert!(!is_short_job_id("ABCDEF12"));
        assert!(!is_short_job_id("abc"));
    }

    #[tokio::test]
    async fn run_lifecycle_invokes_cli() {
        // Stand in for `claude` with `echo` so we exercise the shell-out path
        // without touching real sessions.
        std::env::set_var("MOTHER_CLAUDE_CLI", "echo");
        let out = run_lifecycle("stop", "631b387d").await;
        let bad = run_lifecycle("stop", "abc-123").await;
        let action = run_lifecycle("attach", "631b387d").await;
        std::env::remove_var("MOTHER_CLAUDE_CLI");
        let out = out.unwrap();
        assert!(out.contains("stop"));
        assert!(out.contains("631b387d"));
        assert!(bad.is_err(), "a UUID-ish id never reaches the CLI");
        assert!(action.is_err());
    }

    #[test]
    fn tasks_become_agents_and_background_rows() {
        let h = OwnedHandle {
            id: "s".into(),
            cwd: "/".into(),
            started_at: 0,
            driver: Driver::Stream,
            caller_may_dangerous: false,
            stdin: AsyncMutex::new(None),
            kill: Notify::new(),
            alive: AtomicBool::new(true),
            live: Mutex::new(OwnedLive::default()),
            outgoing: Mutex::new(HashMap::new()),
            incoming: Mutex::new(HashMap::new()),
            stderr_tail: Mutex::new(VecDeque::new()),
        };
        on_task(
            &h,
            "task_started",
            &json!({ "task_id": "t1", "description": "Explore the repo",
                     "task_type": "local_agent", "is_backgrounded": true, "tool_use_id": "toolu_1" }),
        );
        on_task(
            &h,
            "task_progress",
            &json!({ "task_id": "t1", "subagent_type": "Explore", "last_tool_name": "Grep",
                     "usage": { "total_tokens": 1200, "tool_uses": 3, "duration_ms": 10 } }),
        );
        on_task(
            &h,
            "task_started",
            &json!({ "task_id": "b1", "description": "npm test", "task_type": "local_bash",
                     "is_backgrounded": true }),
        );
        let live = h.live().clone();
        let agents = live.agents_of("s");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_type.as_deref(), Some("Explore"));
        assert_eq!(agents[0].tokens, Some(1200));
        assert!(agents[0].can_stop);
        assert_eq!(
            agents[0].activity.as_ref().unwrap().tool.as_deref(),
            Some("Grep")
        );
        let tasks = live.background_tasks();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].kind, BackgroundTaskKind::Bash);
        on_task(
            &h,
            "task_notification",
            &json!({ "task_id": "t1", "status": "completed", "summary": "done", "output_file": "" }),
        );
        let agents = h.live().agents_of("s");
        assert_eq!(agents[0].status, AgentStatus::Done);
        assert!(!agents[0].can_stop);
    }

    fn applied(effort: Option<&str>, requested: bool, available: bool) -> AppliedSettings {
        AppliedSettings {
            model: Some("claude-opus-5-5".into()),
            effort: effort.map(str::to_string),
            ultracode: requested && available,
            ultracode_requested: requested,
            ultracode_available: available,
        }
    }

    #[test]
    fn applied_settings_older_than_a_change_are_stale() {
        let mut live = OwnedLive::default();
        assert!(live.store_applied(applied(Some("high"), false, true), 0));
        assert!(live.applied_effort().is_some() && live.applied_ultracode().is_some());
        // The same answer again changes nothing.
        assert!(!live.store_applied(applied(Some("high"), false, true), 0));

        // An effort change: its effort is unconfirmed until an answer to a
        // request sent after it; ultracode and the model are unaffected.
        live.note_change(SettingChange::Effort);
        assert!(live.applied_effort().is_none());
        assert!(live.applied_ultracode().is_some() && live.applied_model().is_some());
        // An answer to a request sent before the change (late, or the only
        // one when the follow-up failed) does not confirm it…
        assert!(!live.store_applied(applied(Some("high"), false, true), 0));
        assert!(live.applied_effort().is_none());
        // …one sent after it does.
        assert!(live.store_applied(applied(Some("high"), false, true), 1));
        assert_eq!(
            live.applied_effort().unwrap().effort.as_deref(),
            Some("high")
        );
        // An answer older than the one kept is dropped.
        assert!(!live.store_applied(applied(Some("low"), false, true), 0));
        assert_eq!(
            live.applied.as_ref().unwrap().effort.as_deref(),
            Some("high")
        );

        // An ultracode switch: requested until confirmed, then cleared.
        live.ultracode_request = Some(true);
        live.note_change(SettingChange::Ultracode);
        assert!(live.applied_ultracode().is_none());
        assert!(live.applied_effort().is_some(), "the effort is left alone");
        assert!(
            live.ultracode_wanted(),
            "an effort change keeps the request"
        );
        assert!(live.store_applied(applied(Some("high"), true, true), 2));
        assert_eq!(live.ultracode_request, None);
        assert!(live.applied_ultracode().unwrap().ultracode);

        // A model change makes all three stale.
        live.note_change(SettingChange::Model);
        assert!(live.applied_effort().is_none());
        assert!(live.applied_ultracode().is_none());
        assert!(live.applied_model().is_none());
        assert!(live.store_applied(applied(None, true, false), 3));
        assert!(live.applied_effort().is_some_and(|a| a.effort.is_none()));
    }

    #[test]
    fn a_standing_ultracode_request_stays_switchable_and_is_kept() {
        let mut live = OwnedLive::default();
        assert!(!live.ultracode_switchable() && !live.ultracode_wanted());
        // Available: the switch turns it on.
        live.store_applied(applied(Some("high"), false, true), 0);
        assert!(live.ultracode_switchable() && !live.ultracode_wanted());
        // Requested, but this model cannot run it: still switchable (to
        // withdraw it), and an effort change keeps it.
        live.store_applied(applied(Some("high"), true, false), 0);
        assert!(live.ultracode_switchable() && live.ultracode_wanted());
        // Withdrawn and confirmed: nothing to switch on a model without it.
        live.ultracode_request = Some(false);
        live.note_change(SettingChange::Ultracode);
        assert!(!live.ultracode_wanted(), "the switch wins until confirmed");
        live.store_applied(applied(Some("high"), false, false), 1);
        assert!(!live.ultracode_switchable() && !live.ultracode_wanted());
        // Just switched on, not confirmed yet: switchable either way.
        live.ultracode_request = Some(true);
        live.note_change(SettingChange::Ultracode);
        assert!(live.ultracode_switchable() && live.ultracode_wanted());
    }

    #[test]
    fn initialize_account_facts() {
        let facts = |account: Value| AccountFacts::from_initialize(&json!({ "account": account }));
        let plan = facts(json!({ "email": "Me@Example.test", "organization": "Me",
            "subscriptionType": "max", "tokenSource": "claude.ai", "apiKeySource": "none",
            "apiProvider": "firstParty" }))
        .unwrap();
        assert!(!plan.not_plan);
        assert_eq!(
            plan.email_key,
            crate::claude::usage::email_key(" me@example.test ")
        );
        assert!(facts(json!({ "apiProvider": "bedrock" })).unwrap().not_plan);
        assert!(
            facts(json!({ "apiProvider": "firstParty", "apiKeySource": "apiKeyHelper" }))
                .unwrap()
                .not_plan
        );
        let unknown = facts(json!({})).unwrap();
        assert!(!unknown.not_plan && unknown.email_key.is_none());
        assert_eq!(
            AccountFacts::from_initialize(&json!({ "commands": [] })),
            None
        );
        // Nothing readable is kept.
        assert!(!format!("{plan:?}").contains("example"));
    }
}
