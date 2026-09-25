//! "What is each session doing right now" (design §3.5).
//!
//! [`ActivityTracker`] keeps the live hints other subsystems report:
//! * B3 (owned sessions) calls [`ActivityTracker::note_owned`] with the owned
//!   stream state and [`ActivityTracker::note_permission_mode`] on
//!   `system/init` / `system/status`;
//! * B4 (hooks) calls [`ActivityTracker::note_hook`] for every hook event and
//!   [`ActivityTracker::note_permission_mode`] with the payload's
//!   `permission_mode`.
//!
//! [`derive`] combines those hints with the pidfile and the transcript tail
//! into a status + [`Activity`], in this priority:
//! answerable pending item ⇒ waiting; pidfile `waiting` ⇒ waiting (with
//! `waitingFor`); owned stream state; latest hook event (PreToolUse without
//! PostToolUse ⇒ that tool; PreCompact ⇒ compacting; UserPromptSubmit ⇒
//! thinking); pidfile busy + transcript tail (tool_use without result ⇒ tool,
//! prompt/tool_result ⇒ thinking, end of turn with background work ⇒
//! background, api_error ⇒ retrying); idle ⇒ idle.
//!
//! Labels never contain durations (clients render elapsed time from `since`),
//! so an unchanged activity hashes the same on every refresh.
//!
//! The tracker also holds the *observed* pending items of foreign sessions
//! (seen on disk, not answerable here) so `GET /inbox` and the session detail
//! can serve them in full while `SessionSummary.pending` carries previews.
//! AppState field: `state.activity`.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use super::index::{Mark, MarkKind, PendingTool};
use super::live::{JobState, LiveSession};
use super::model::{
    Activity, ActivityKind, PendingItem, PendingKind, PendingPreview, SessionStatus,
};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Debug, Clone, Default)]
struct HookHints {
    last_event: Option<(String, i64)>,
    /// PreToolUse without its PostToolUse: (tool, agent, since).
    tools: Vec<(String, Option<String>, i64)>,
    compacting_since: Option<i64>,
    prompt_since: Option<i64>,
    stopped_at: Option<i64>,
    /// Subagents started (SubagentStart) and not yet stopped.
    subagents: HashMap<String, i64>,
}

#[derive(Debug, Clone, Default)]
struct Hints {
    owned: Option<(Activity, i64)>,
    hook: HookHints,
    /// (mode, epoch ms, source) from hooks / owned control.
    mode: Option<(String, i64)>,
}

/// Live activity hints per session.
#[derive(Debug, Default)]
pub struct ActivityTracker {
    inner: Mutex<HashMap<String, Hints>>,
    observed: Mutex<HashMap<String, Vec<PendingItem>>>,
}

impl ActivityTracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Hints>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// An owned session's stream state changed (B3). `Idle`/`Ended` clear it.
    pub fn note_owned(&self, session_id: &str, activity: Activity) {
        let mut g = self.lock();
        let h = g.entry(session_id.to_string()).or_default();
        if matches!(activity.kind, ActivityKind::Ended) {
            h.owned = None;
        } else {
            let since = activity.since.unwrap_or_else(now_ms);
            h.owned = Some((activity, since));
        }
    }

    /// A hook event arrived for a session (B4: event name, tool name, agent
    /// id). Only main-thread events (no agent id) drive the session activity.
    pub fn note_hook(
        &self,
        session_id: &str,
        event: &str,
        tool_name: Option<&str>,
        agent_id: Option<&str>,
    ) {
        let now = now_ms();
        let mut g = self.lock();
        let h = &mut g.entry(session_id.to_string()).or_default().hook;
        h.last_event = Some((event.to_string(), now));
        let agent = agent_id.map(str::to_string);
        match event {
            "PreToolUse" => {
                if let Some(t) = tool_name {
                    h.tools.push((t.to_string(), agent, now));
                    if h.tools.len() > 32 {
                        h.tools.remove(0);
                    }
                }
                h.compacting_since = None;
            }
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
                if let Some(i) = h
                    .tools
                    .iter()
                    .position(|(t, a, _)| Some(t.as_str()) == tool_name && *a == agent)
                {
                    h.tools.remove(i);
                }
            }
            "UserPromptSubmit" if agent.is_none() => {
                h.prompt_since = Some(now);
                h.stopped_at = None;
                h.tools.retain(|(_, a, _)| a.is_some());
            }
            "Stop" | "StopFailure" if agent.is_none() => {
                h.stopped_at = Some(now);
                h.prompt_since = None;
                h.compacting_since = None;
                h.tools.retain(|(_, a, _)| a.is_some());
            }
            "PreCompact" => h.compacting_since = Some(now),
            "PostCompact" => h.compacting_since = None,
            "SubagentStart" => {
                if let Some(a) = agent {
                    h.subagents.insert(a, now);
                }
            }
            "SubagentStop" => {
                if let Some(a) = &agent {
                    h.subagents.remove(a);
                    h.tools.retain(|(_, ta, _)| ta.as_ref() != Some(a));
                }
            }
            "SessionEnd" => *h = HookHints::default(),
            _ => {}
        }
    }

    /// The session's permission mode as reported by a hook payload or the
    /// owned stream (`system/init`, `system/status`).
    pub fn note_permission_mode(&self, session_id: &str, mode: &str) {
        if mode.is_empty() || mode.len() > 40 {
            return;
        }
        self.lock().entry(session_id.to_string()).or_default().mode =
            Some((mode.to_string(), now_ms()));
    }

    /// `(mode, epoch ms)` last noted by hooks / owned control.
    pub fn permission_mode_note(&self, session_id: &str) -> Option<(String, i64)> {
        self.lock().get(session_id).and_then(|h| h.mode.clone())
    }

    /// Forget a session's live hints (process gone).
    pub fn forget(&self, session_id: &str) {
        if let Some(h) = self.lock().get_mut(session_id) {
            h.owned = None;
            h.hook = HookHints::default();
        }
    }

    /// The owned stream hint.
    pub fn owned_hint(&self, session_id: &str) -> Option<Activity> {
        self.lock()
            .get(session_id)
            .and_then(|h| h.owned.as_ref())
            .map(|(a, since)| {
                let mut a = a.clone();
                a.since = a.since.or(Some(*since));
                a
            })
    }

    /// The hook-derived hint (main thread only).
    pub fn hook_hint(&self, session_id: &str) -> Option<Activity> {
        let g = self.lock();
        let h = &g.get(session_id)?.hook;
        if let Some(since) = h.compacting_since {
            let mut a = Activity::new(ActivityKind::Compacting, "Compacting context");
            a.since = Some(since);
            return Some(a);
        }
        if let Some((tool, _, since)) = h.tools.iter().rev().find(|(_, a, _)| a.is_none()) {
            let mut a = tool_activity(tool, None);
            a.since = Some(*since);
            return Some(a);
        }
        if let Some(since) = h.prompt_since {
            let mut a = Activity::new(ActivityKind::Thinking, "Thinking");
            a.since = Some(since);
            return Some(a);
        }
        None
    }

    /// The best current activity hint for a session, if any (owned stream
    /// state first, then hooks).
    pub fn current(&self, session_id: &str) -> Option<Activity> {
        self.owned_hint(session_id)
            .or_else(|| self.hook_hint(session_id))
    }

    /// Replace a session's observed (on-disk, unanswerable) pending items.
    pub fn set_observed(&self, session_id: &str, items: Vec<PendingItem>) {
        let mut g = self.observed.lock().unwrap_or_else(|p| p.into_inner());
        if items.is_empty() {
            g.remove(session_id);
        } else {
            g.insert(session_id.to_string(), items);
        }
    }

    /// A session's observed pending items.
    pub fn observed(&self, session_id: &str) -> Vec<PendingItem> {
        self.observed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Every observed pending item.
    pub fn observed_all(&self) -> Vec<PendingItem> {
        self.observed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .flatten()
            .cloned()
            .collect()
    }
}

/// `Running Bash` style activity for a tool (Agent/Task ⇒ delegating).
pub fn tool_activity(tool: &str, detail: Option<&str>) -> Activity {
    let mut a = match tool {
        "Agent" | "Task" => Activity::new(ActivityKind::Delegating, "Running an agent"),
        "Workflow" => Activity::new(ActivityKind::Delegating, "Running a workflow"),
        _ => Activity::new(ActivityKind::Tool, format!("Running {tool}")),
    };
    a.tool = Some(tool.to_string());
    a.detail = detail.filter(|d| !d.is_empty()).map(str::to_string);
    a
}

/// Label for a pidfile `waitingFor`.
pub fn waiting_label(waiting_for: Option<&str>) -> String {
    match waiting_for {
        Some("permission prompt") => "Waiting for approval".into(),
        Some("input needed") => "Waiting for your answer".into(),
        Some("dialog open") => "A dialog is open".into(),
        Some("sandbox request") => "Waiting for a sandbox approval".into(),
        Some("worker request") => "Waiting on a worker request".into(),
        Some(other) if !other.is_empty() => format!("Waiting: {other}"),
        _ => "Waiting for you".into(),
    }
}

/// Anything that looks like a pending item (store items and previews).
pub trait PendingLike {
    fn kind(&self) -> PendingKind;
    fn answerable(&self) -> bool;
    fn tool_name(&self) -> Option<&str>;
    fn created_at(&self) -> i64;
}

impl PendingLike for PendingItem {
    fn kind(&self) -> PendingKind {
        self.kind
    }
    fn answerable(&self) -> bool {
        self.answerable
    }
    fn tool_name(&self) -> Option<&str> {
        self.tool_name.as_deref()
    }
    fn created_at(&self) -> i64 {
        self.created_at
    }
}

impl PendingLike for PendingPreview {
    fn kind(&self) -> PendingKind {
        self.kind
    }
    fn answerable(&self) -> bool {
        self.answerable
    }
    fn tool_name(&self) -> Option<&str> {
        self.tool_name.as_deref()
    }
    fn created_at(&self) -> i64 {
        self.created_at
    }
}

/// The waiting activity for a pending item.
pub fn waiting_activity<P: PendingLike>(p: &P) -> Activity {
    let label = match p.kind() {
        PendingKind::Question => "Waiting for your answer",
        PendingKind::Plan => "Waiting for plan approval",
        PendingKind::Elicitation => "Waiting for input",
        PendingKind::Permission => "Waiting for approval",
    };
    let mut a = Activity::new(ActivityKind::Waiting, label);
    a.tool = p.tool_name().map(str::to_string);
    a.since = Some(p.created_at()).filter(|t| *t > 0);
    a
}

/// Everything [`derive`] looks at.
#[derive(Debug, Clone, Default)]
pub struct Inputs<'a> {
    /// The process is alive (pidfile + liveness, or an owned handle).
    pub live: bool,
    pub owned: bool,
    pub pidfile: Option<&'a LiveSession>,
    pub pending: &'a [PendingPreview],
    pub owned_hint: Option<Activity>,
    pub hook_hint: Option<Activity>,
    /// Last meaningful main-chain record of the transcript.
    pub mark: Option<&'a Mark>,
    /// Main-chain tool uses without a result.
    pub pending_tools: &'a [PendingTool],
    pub agents_running: u32,
    pub background_running: u32,
    pub job: Option<&'a JobState>,
    pub has_transcript: bool,
    /// Earliest timestamp that belongs to the current process (pidfile
    /// `startedAt`): older tool uses are leftovers of a previous run.
    pub process_started: Option<i64>,
    /// Resolved by the caller for a waiting session: `Some(Some(tool))` is
    /// the tool it waits on (a foreground subagent's own tool rather than
    /// the `Agent` call), `Some(None)` means a subagent waits on something
    /// unknown, `None` falls back to the newest main-chain tool.
    pub waiting_on: Option<Option<&'a PendingTool>>,
}

/// Status + activity for one session.
pub fn derive(i: &Inputs) -> (SessionStatus, Activity) {
    if !i.live {
        let (status, label) = match i.job.and_then(|j| j.state.as_deref()) {
            Some("failed") => (SessionStatus::Failed, "Failed"),
            Some("stopped") => (SessionStatus::Stopped, "Stopped"),
            _ => (SessionStatus::Ended, "Ended"),
        };
        return (status, Activity::new(ActivityKind::Ended, label));
    }
    if let Some(p) = i.pending.iter().find(|p| p.answerable) {
        return (SessionStatus::Waiting, waiting_activity(p));
    }
    let pid_status = i.pidfile.and_then(|p| p.status.as_deref());
    let status_since = i.pidfile.and_then(|p| p.status_updated_at);
    if pid_status == Some("waiting") {
        let mut a = Activity::new(
            ActivityKind::Waiting,
            waiting_label(i.pidfile.and_then(|p| p.waiting_for.as_deref())),
        );
        let tools = current_tools(i);
        let tool = match i.waiting_on {
            Some(resolved) => resolved,
            // Never name the `Agent` call as what waits: its subagent does.
            None => tools.last().copied().filter(|t| !is_delegation(t)),
        };
        if let Some(t) = tool {
            a.tool = Some(t.name.clone());
            a.detail = Some(t.summary.clone()).filter(|s| !s.is_empty());
        } else if tools.last().is_some_and(|t| is_delegation(t)) {
            a.detail = Some("A subagent is waiting".to_string());
        } else if let Some(p) = i.pending.first() {
            a.tool = p.tool_name.clone();
        }
        a.since = status_since;
        return (SessionStatus::Waiting, a);
    }
    if i.owned {
        if let Some(a) = &i.owned_hint {
            let status = match a.kind {
                ActivityKind::Idle => SessionStatus::Idle,
                ActivityKind::Starting => SessionStatus::Starting,
                ActivityKind::Waiting => SessionStatus::Waiting,
                _ => SessionStatus::Working,
            };
            return (status, a.clone());
        }
        if i.pidfile.is_none() && !i.has_transcript {
            return (
                SessionStatus::Starting,
                Activity::new(ActivityKind::Starting, "Starting"),
            );
        }
    }
    let busy = match pid_status {
        Some("busy" | "shell") => true,
        Some("idle") => false,
        // No pidfile (owned child before registration, or a CLI that does not
        // write pidfiles): trust the transcript tail.
        _ => matches!(
            i.mark.map(|m| m.kind),
            Some(MarkKind::Prompt | MarkKind::ToolResult | MarkKind::ToolUse | MarkKind::ApiError)
        ),
    };
    if !busy {
        let mut a = Activity::new(ActivityKind::Idle, "Idle");
        a.since = status_since.or_else(|| i.mark.and_then(|m| m.ts));
        return (SessionStatus::Idle, a);
    }
    if let Some(a) = &i.hook_hint {
        return (SessionStatus::Working, a.clone());
    }
    (SessionStatus::Working, tail_activity(i, status_since))
}

fn is_delegation(t: &PendingTool) -> bool {
    matches!(t.name.as_str(), "Agent" | "Task")
}

fn current_tools<'a>(i: &'a Inputs) -> Vec<&'a PendingTool> {
    i.pending_tools
        .iter()
        .filter(|t| {
            i.process_started
                .is_none_or(|start| t.ts.is_none_or(|ts| ts + 5_000 >= start))
        })
        .filter(|t| t.name != "TodoWrite")
        .collect()
}

fn tail_activity(i: &Inputs, status_since: Option<i64>) -> Activity {
    let tools = current_tools(i);
    let delegating: Vec<&&PendingTool> = tools
        .iter()
        .filter(|t| matches!(t.name.as_str(), "Agent" | "Task"))
        .collect();
    if let Some(t) = tools
        .iter()
        .rev()
        .find(|t| !matches!(t.name.as_str(), "Agent" | "Task"))
    {
        let mut a = tool_activity(&t.name, Some(&t.summary));
        a.since = t.ts;
        return a;
    }
    if let Some(first) = delegating.first() {
        let mut a = if delegating.len() > 1 {
            Activity::new(
                ActivityKind::Delegating,
                format!("Running {} agents", delegating.len()),
            )
        } else {
            tool_activity(&first.name, None)
        };
        a.tool = Some(first.name.clone());
        a.detail = Some(first.summary.clone()).filter(|s| !s.is_empty());
        a.since = first.ts;
        return a;
    }
    let mark = i.mark.cloned().unwrap_or_default();
    let mut a = match mark.kind {
        MarkKind::ApiError => {
            let mut a = Activity::new(ActivityKind::Retrying, "Retrying after an API error");
            if let Some((n, max)) = mark.retry {
                a.detail = Some(format!("attempt {n} of {max}"));
            }
            a
        }
        MarkKind::Prompt | MarkKind::ToolResult | MarkKind::Compact => {
            Activity::new(ActivityKind::Thinking, "Thinking")
        }
        MarkKind::EndTurn | MarkKind::TurnEnd | MarkKind::Interrupted
            if i.agents_running + i.background_running > 0 =>
        {
            let mut a = Activity::new(ActivityKind::Background, "Background work");
            a.detail = Some(background_detail(i.agents_running, i.background_running));
            a
        }
        _ if i.agents_running + i.background_running > 0 => {
            let mut a = Activity::new(ActivityKind::Background, "Background work");
            a.detail = Some(background_detail(i.agents_running, i.background_running));
            a
        }
        _ => Activity::new(ActivityKind::Thinking, "Working"),
    };
    a.since = mark.ts.or(status_since);
    a
}

fn background_detail(agents: u32, shells: u32) -> String {
    let mut parts = Vec::new();
    if agents > 0 {
        parts.push(format!(
            "{agents} agent{}",
            if agents == 1 { "" } else { "s" }
        ));
    }
    if shells > 0 {
        parts.push(format!(
            "{shells} background task{}",
            if shells == 1 { "" } else { "s" }
        ));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pidfile(status: &str, waiting: Option<&str>) -> LiveSession {
        LiveSession {
            pid: Some(1),
            status: Some(status.into()),
            waiting_for: waiting.map(str::to_string),
            status_updated_at: Some(50),
            started_at: Some(0),
            ..Default::default()
        }
    }

    fn tool(name: &str, summary: &str, ts: i64) -> PendingTool {
        PendingTool {
            id: format!("t-{name}"),
            name: name.into(),
            summary: summary.into(),
            ts: Some(ts),
            input: None,
        }
    }

    #[test]
    fn ended_sessions_use_the_job_state() {
        let (s, a) = derive(&Inputs::default());
        assert_eq!(s, SessionStatus::Ended);
        assert_eq!(a.kind, ActivityKind::Ended);
        let job = JobState {
            state: Some("failed".into()),
            ..Default::default()
        };
        let (s, _) = derive(&Inputs {
            job: Some(&job),
            ..Default::default()
        });
        assert_eq!(s, SessionStatus::Failed);
    }

    #[test]
    fn answerable_pending_beats_everything() {
        let p = pidfile("busy", None);
        let pending = vec![PendingPreview {
            kind: PendingKind::Question,
            answerable: true,
            tool_name: Some("AskUserQuestion".into()),
            ..Default::default()
        }];
        let (s, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&p),
            pending: &pending,
            ..Default::default()
        });
        assert_eq!(s, SessionStatus::Waiting);
        assert_eq!(a.label, "Waiting for your answer");
    }

    #[test]
    fn pidfile_waiting_names_the_pending_tool() {
        let p = pidfile("waiting", Some("permission prompt"));
        let tools = vec![tool("Bash", "npm test", 10)];
        let (s, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&p),
            pending_tools: &tools,
            ..Default::default()
        });
        assert_eq!(s, SessionStatus::Waiting);
        assert_eq!(a.label, "Waiting for approval");
        assert_eq!(a.tool.as_deref(), Some("Bash"));
        assert_eq!(a.detail.as_deref(), Some("npm test"));
        assert_eq!(a.since, Some(50));
    }

    #[test]
    fn a_waiting_foreground_subagent_is_not_named_as_the_agent_call() {
        let p = pidfile("waiting", Some("permission prompt"));
        let tools = vec![tool("Agent", "Explore the repo", 10)];
        // Unresolved (or resolved to nothing): no tool is named.
        for waiting_on in [None, Some(None)] {
            let (s, a) = derive(&Inputs {
                live: true,
                pidfile: Some(&p),
                pending_tools: &tools,
                waiting_on,
                ..Default::default()
            });
            assert_eq!(s, SessionStatus::Waiting);
            assert_eq!(a.label, "Waiting for approval");
            assert_eq!(a.tool, None);
            assert_eq!(a.detail.as_deref(), Some("A subagent is waiting"));
        }
        // Resolved to the subagent's own tool.
        let bash = tool("Bash", "rm -rf build", 12);
        let (_, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&p),
            pending_tools: &tools,
            waiting_on: Some(Some(&bash)),
            ..Default::default()
        });
        assert_eq!(a.tool.as_deref(), Some("Bash"));
        assert_eq!(a.detail.as_deref(), Some("rm -rf build"));
    }

    #[test]
    fn busy_with_a_running_tool() {
        let p = pidfile("busy", None);
        let tools = vec![
            tool("Agent", "Explore the repo", 5),
            tool("Bash", "cargo test", 9),
        ];
        let (s, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&p),
            pending_tools: &tools,
            ..Default::default()
        });
        assert_eq!(s, SessionStatus::Working);
        assert_eq!(a.kind, ActivityKind::Tool);
        assert_eq!(a.label, "Running Bash");
        assert!(
            !a.label.chars().any(|c| c.is_ascii_digit()),
            "no durations in labels"
        );
        // Only agents in flight ⇒ delegating.
        let tools = vec![tool("Agent", "Explore the repo", 5)];
        let (_, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&p),
            pending_tools: &tools,
            ..Default::default()
        });
        assert_eq!(a.kind, ActivityKind::Delegating);
        assert_eq!(a.detail.as_deref(), Some("Explore the repo"));
    }

    #[test]
    fn stale_tool_uses_from_a_previous_process_are_ignored() {
        let mut p = pidfile("busy", None);
        p.started_at = Some(1_000_000);
        let tools = vec![tool("Bash", "old", 10)];
        let mark = Mark {
            kind: MarkKind::Prompt,
            ts: Some(1_000_100),
            retry: None,
        };
        let (_, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&p),
            pending_tools: &tools,
            mark: Some(&mark),
            process_started: Some(1_000_000),
            ..Default::default()
        });
        assert_eq!(a.kind, ActivityKind::Thinking);
    }

    #[test]
    fn tail_marks_map_to_thinking_retrying_background_idle() {
        let p = pidfile("busy", None);
        let err = Mark {
            kind: MarkKind::ApiError,
            ts: Some(3),
            retry: Some((2, 10)),
        };
        let (_, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&p),
            mark: Some(&err),
            ..Default::default()
        });
        assert_eq!(a.kind, ActivityKind::Retrying);
        assert_eq!(a.detail.as_deref(), Some("attempt 2 of 10"));
        let end = Mark {
            kind: MarkKind::EndTurn,
            ts: Some(4),
            retry: None,
        };
        let (s, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&p),
            mark: Some(&end),
            agents_running: 2,
            background_running: 1,
            ..Default::default()
        });
        assert_eq!(s, SessionStatus::Working);
        assert_eq!(a.kind, ActivityKind::Background);
        assert_eq!(a.detail.as_deref(), Some("2 agents, 1 background task"));
        let idle = pidfile("idle", None);
        let (s, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&idle),
            mark: Some(&end),
            ..Default::default()
        });
        assert_eq!(s, SessionStatus::Idle);
        assert_eq!(a.kind, ActivityKind::Idle);
    }

    #[test]
    fn hook_and_owned_hints() {
        let t = ActivityTracker::new();
        t.note_hook("s", "UserPromptSubmit", None, None);
        assert_eq!(t.current("s").unwrap().kind, ActivityKind::Thinking);
        t.note_hook("s", "PreToolUse", Some("Bash"), None);
        t.note_hook("s", "PreToolUse", Some("Read"), Some("a1"));
        let a = t.hook_hint("s").unwrap();
        assert_eq!(
            a.label, "Running Bash",
            "subagent tools do not drive the session"
        );
        t.note_hook("s", "PostToolUse", Some("Bash"), None);
        assert_eq!(t.hook_hint("s").unwrap().kind, ActivityKind::Thinking);
        t.note_hook("s", "PreCompact", None, None);
        assert_eq!(t.hook_hint("s").unwrap().kind, ActivityKind::Compacting);
        t.note_hook("s", "Stop", None, None);
        assert!(t.hook_hint("s").is_none());
        t.note_owned("s", Activity::new(ActivityKind::Tool, "Running Edit"));
        assert_eq!(t.current("s").unwrap().label, "Running Edit");
        t.note_owned("s", Activity::new(ActivityKind::Ended, "Ended"));
        assert!(t.owned_hint("s").is_none());
        t.note_permission_mode("s", "plan");
        assert_eq!(t.permission_mode_note("s").unwrap().0, "plan");

        // Busy pidfile + hook hint ⇒ the hint wins over the tail.
        let p = pidfile("busy", None);
        let (_, a) = derive(&Inputs {
            live: true,
            pidfile: Some(&p),
            hook_hint: Some(tool_activity("Grep", Some("TODO"))),
            ..Default::default()
        });
        assert_eq!(a.label, "Running Grep");
        // Owned sessions without a pidfile or transcript are starting.
        let (s, _) = derive(&Inputs {
            live: true,
            owned: true,
            ..Default::default()
        });
        assert_eq!(s, SessionStatus::Starting);
    }

    #[test]
    fn observed_items_are_kept_per_session() {
        let t = ActivityTracker::new();
        t.set_observed(
            "s",
            vec![PendingItem {
                id: "observed:1".into(),
                ..Default::default()
            }],
        );
        assert_eq!(t.observed("s").len(), 1);
        assert_eq!(t.observed_all().len(), 1);
        t.set_observed("s", Vec::new());
        assert!(t.observed_all().is_empty());
    }
}
