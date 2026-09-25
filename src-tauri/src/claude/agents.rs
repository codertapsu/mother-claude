//! Subagents, workflow agents, background shells and monitors (design §3.4).
//!
//! Per session (`projects/<enc>/<sid>/`):
//! * **Agent-tool subagents** — `subagents/agent-<id>.meta.json` (`agentType`,
//!   `description`, `toolUseId`, `parentAgentId`, `spawnDepth`,
//!   `requestShape`, worktree) + `agent-<id>.jsonl` (indexed like any
//!   transcript: tokens de-duplicated by message id, tool uses, last action).
//!   Status comes from the parent's `Agent` call: foreground ⇒ running until
//!   its tool_result; background ⇒ running from `async_launched` until the
//!   `<task-notification>` for that agent id. "Running" additionally needs the
//!   session process alive, the file written since the process started, and
//!   either a write in the last 10 minutes or a tool call of its own still in
//!   flight.
//! * **Workflow agents** — `subagents/workflows/<runId>/journal.jsonl`
//!   (`started{agentId,label,phase}` / `result{agentId}`): running = started −
//!   result. `workflows/<runId>.json` appears only when the run finished and
//!   gives status, duration, tokens and phases.
//! * **Background shells / monitors** — from the transcript index (launch
//!   without a completion), bounded by the process start.
//!
//! Journals are read incrementally; meta and run files once per mtime.
//! AppState field: `state.agents`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::Deserialize;

use super::activity::tool_activity;
use super::index::loose::{parse_line, CapStr, LStr, Seq, LU64};
use super::index::{AgentCall, BgTask, LastAct, SessionFiles, TranscriptIndex};
use super::model::{
    Activity, ActivityKind, AgentCounts, AgentInfo, AgentKind, AgentStatus, FleetSession,
    FleetSummary, WorkflowAgentCounts, WorkflowPhase, WorkflowRun, WorkflowStatus,
};

/// A subagent file untouched this long is only "running" while one of its own
/// tool calls is still in flight.
const IDLE_PLAUSIBLE_MS: i64 = 10 * 60 * 1000;
/// Finished background shells kept in a session's agent list.
const FINISHED_SHELLS_KEPT: usize = 20;
/// Finished agents / workflow runs kept in a session's lists (the newest).
const FINISHED_AGENTS_KEPT: usize = 150;
const FINISHED_WORKFLOWS_KEPT: usize = 30;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Meta {
    #[serde(rename = "agentType")]
    agent_type: LStr,
    description: CapStr<512>,
    name: CapStr<256>,
    model: LStr,
    #[serde(rename = "toolUseId")]
    tool_use_id: LStr,
    #[serde(rename = "parentAgentId")]
    parent_agent_id: LStr,
    #[serde(rename = "spawnDepth")]
    spawn_depth: LU64,
    #[serde(rename = "requestShape")]
    request_shape: LStr,
    #[serde(rename = "worktreePath")]
    worktree_path: CapStr<4096>,
    #[serde(rename = "workflowPhase")]
    workflow_phase: CapStr<256>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct JournalRec {
    #[serde(rename = "type")]
    ty: LStr,
    #[serde(rename = "agentId")]
    agent_id: LStr,
    label: CapStr<256>,
    phase: CapStr<256>,
}

#[derive(Debug, Clone, Default)]
struct Journal {
    offset: u64,
    ino: u64,
    /// agent id → (label, phase), in start order.
    started: Vec<(String, String, Option<String>)>,
    results: HashSet<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct PhaseRec {
    title: CapStr<256>,
    detail: CapStr<512>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RunInfo {
    status: LStr,
    #[serde(rename = "workflowName")]
    workflow_name: CapStr<256>,
    #[serde(rename = "durationMs")]
    duration_ms: LU64,
    #[serde(rename = "totalTokens")]
    total_tokens: LU64,
    #[serde(rename = "agentCount")]
    agent_count: LU64,
    #[serde(rename = "startTime")]
    start_time: LU64,
    #[serde(rename = "taskId")]
    task_id: LStr,
    phases: Seq<PhaseRec>,
}

/// A session's computed agents.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionAgents {
    pub agents: Vec<AgentInfo>,
    pub workflows: Vec<WorkflowRun>,
    pub counts: AgentCounts,
    hash: u64,
}

/// What [`AgentTracker::recompute`] needs to know about the session.
#[derive(Debug, Clone, Default)]
pub struct AgentCtx {
    /// The session process is alive.
    pub live: bool,
    /// Pidfile `startedAt` (epoch ms) of the current process.
    pub process_started: Option<i64>,
    /// Mother Claude controls it (tasks can be stopped).
    pub owned: bool,
    pub now: i64,
}

#[derive(Debug, Default)]
struct Inner {
    sessions: HashMap<String, SessionAgents>,
    /// meta.json by path: (mtime, last checked, parsed).
    metas: HashMap<PathBuf, (i64, std::time::Instant, Meta)>,
    journals: HashMap<PathBuf, Journal>,
    runs: HashMap<PathBuf, (i64, RunInfo)>,
    fleet: Vec<FleetSession>,
    fleet_summary: FleetSummary,
    fleet_hash: u64,
}

/// Agent tracking state.
#[derive(Debug, Default)]
pub struct AgentTracker {
    inner: Mutex<Inner>,
}

fn hash_of<T: serde::Serialize>(v: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(v).unwrap_or_default().hash(&mut h);
    h.finish()
}

fn mtime_of(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()
        .map(|m| super::index::stamp_of(&m).mtime)
}

fn activity_of(last: &LastAct) -> Activity {
    let mut a = match last.kind.as_str() {
        "tool" => tool_activity(
            last.tool.as_deref().unwrap_or("tool"),
            last.summary.as_deref(),
        ),
        "result" => Activity::new(ActivityKind::Thinking, "Thinking"),
        "text" => {
            let mut a = Activity::new(ActivityKind::Thinking, "Writing");
            a.detail = last.summary.clone();
            a
        }
        _ => Activity::new(ActivityKind::Thinking, "Thinking"),
    };
    a.since = last.ts;
    a
}

/// Read a journal from its offset (restart when replaced/truncated).
fn advance_journal(path: &Path, j: &mut Journal) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let st = super::index::stamp_of(&meta);
    if (j.ino != 0 && st.ino != j.ino) || st.size < j.offset {
        *j = Journal::default();
    }
    j.ino = st.ino;
    if st.size == j.offset {
        return;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    let mut r = BufReader::new(file);
    if r.seek(SeekFrom::Start(j.offset)).is_err() {
        return;
    }
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = match r.read_until(b'\n', &mut buf) {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 || buf.last() != Some(&b'\n') {
            break; // EOF or partial line
        }
        j.offset += n as u64;
        let Some(rec) = parse_line::<JournalRec>(&buf) else {
            continue;
        };
        let Some(agent) = rec.agent_id.get().map(str::to_string) else {
            continue;
        };
        match rec.ty.get() {
            Some("started") => {
                if !j.started.iter().any(|(a, _, _)| *a == agent) && j.started.len() < 2000 {
                    j.started
                        .push((agent, rec.label.text.unwrap_or_default(), rec.phase.text));
                }
            }
            Some("result") => {
                j.results.insert(agent);
            }
            _ => {}
        }
    }
}

struct FileView {
    tokens: u64,
    tool_uses: u64,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
    model: Option<String>,
    last_act: Option<LastAct>,
    own_tools_in_flight: bool,
    calls: Vec<AgentCall>,
}

fn view(index: &TranscriptIndex, path: &Path) -> Option<FileView> {
    index.with_file(path, |f| FileView {
        tokens: f.agg.usage.total().tokens(),
        tool_uses: f.agg.tool_uses,
        first_ts: f.agg.first_ts,
        last_ts: f.agg.last_ts,
        model: f.agg.model.clone(),
        last_act: f.agg.last_act.clone(),
        own_tools_in_flight: !f.agg.pending_tools.is_empty(),
        calls: f.agg.agent_calls.clone(),
    })
}

impl AgentTracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A session's agents and workflow runs (`GET /sessions/{id}/agents`,
    /// `SessionDetail.agentsDetail/workflows`). Empty until computed.
    pub fn for_session(&self, session_id: &str) -> (Vec<AgentInfo>, Vec<WorkflowRun>) {
        self.lock()
            .sessions
            .get(session_id)
            .map(|s| (s.agents.clone(), s.workflows.clone()))
            .unwrap_or_default()
    }

    /// Whether the session's agents were computed at least once.
    pub fn has_session(&self, session_id: &str) -> bool {
        self.lock().sessions.contains_key(session_id)
    }

    /// The session's agent counts.
    pub fn counts(&self, session_id: &str) -> Option<AgentCounts> {
        self.lock().sessions.get(session_id).map(|s| s.counts)
    }

    /// Running agents across all sessions (`GET /agents`, `fleet` frame).
    pub fn fleet(&self) -> Vec<FleetSession> {
        self.lock().fleet.clone()
    }

    /// Cross-session counts for the fleet header.
    pub fn fleet_summary(&self) -> FleetSummary {
        self.lock().fleet_summary.clone()
    }

    /// Replace the fleet view; `true` when it changed.
    pub fn set_fleet(&self, fleet: Vec<FleetSession>, summary: FleetSummary) -> bool {
        let h = hash_of(&(&fleet, &summary));
        let mut g = self.lock();
        g.fleet = fleet;
        g.fleet_summary = summary;
        if g.fleet_hash == h {
            return false;
        }
        g.fleet_hash = h;
        true
    }

    /// Drop sessions that no longer exist.
    pub fn retain(&self, keep: &HashSet<String>) {
        self.lock().sessions.retain(|id, _| keep.contains(id));
    }

    fn meta(&self, path: &Path) -> Meta {
        // meta.json is written once at spawn: re-stat at most every 30 s.
        const RECHECK: std::time::Duration = std::time::Duration::from_secs(30);
        if let Some((_, checked, meta)) = self.lock().metas.get(path) {
            if checked.elapsed() < RECHECK {
                return meta.clone();
            }
        }
        let mtime = mtime_of(path).unwrap_or(0);
        if let Some((m, checked, meta)) = self.lock().metas.get_mut(path) {
            if *m == mtime {
                *checked = std::time::Instant::now();
                return meta.clone();
            }
        }
        let meta = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Meta>(&b).ok())
            .unwrap_or_default();
        self.lock().metas.insert(
            path.to_path_buf(),
            (mtime, std::time::Instant::now(), meta.clone()),
        );
        meta
    }

    fn run_info(&self, path: &Path) -> Option<RunInfo> {
        let mtime = mtime_of(path)?;
        if let Some((m, info)) = self.lock().runs.get(path) {
            if *m == mtime {
                return Some(info.clone());
            }
        }
        let file = std::fs::File::open(path).ok()?;
        // Streaming: `script`, `result`, `logs`, `workflowProgress` are skipped.
        let info: RunInfo = match serde_json::from_reader(BufReader::new(file)) {
            Ok(i) => i,
            Err(e) => {
                tracing::debug!(error = %e, path = %path.display(), "workflow run file unreadable");
                return None;
            }
        };
        self.lock()
            .runs
            .insert(path.to_path_buf(), (mtime, info.clone()));
        Some(info)
    }

    fn journal(&self, path: &Path) -> Journal {
        let mut j = self.lock().journals.get(path).cloned().unwrap_or_default();
        advance_journal(path, &mut j);
        self.lock().journals.insert(path.to_path_buf(), j.clone());
        j
    }

    /// Recompute one session's agents from the index and the side files.
    /// Returns `true` when they changed. Blocking (reads meta/journal/run
    /// files).
    pub fn recompute(&self, index: &TranscriptIndex, session_id: &str, ctx: &AgentCtx) -> bool {
        let files = index.session_files(session_id).unwrap_or_default();
        let computed = self.compute(index, session_id, &files, ctx);
        let mut g = self.lock();
        let changed = g
            .sessions
            .get(session_id)
            .is_none_or(|s| s.hash != computed.hash);
        g.sessions.insert(session_id.to_string(), computed);
        changed
    }

    fn compute(
        &self,
        index: &TranscriptIndex,
        sid: &str,
        files: &SessionFiles,
        ctx: &AgentCtx,
    ) -> SessionAgents {
        let main = files.main.as_deref().and_then(|p| view(index, p));
        let (main_tasks, main_wf) = files
            .main
            .as_deref()
            .and_then(|p| {
                index.with_file(p, |f| (f.agg.tasks.clone(), f.agg.workflow_runs.clone()))
            })
            .unwrap_or_default();
        let main_calls = main.as_ref().map(|m| m.calls.clone()).unwrap_or_default();
        let started_ok = |ts: Option<i64>| {
            ctx.process_started
                .is_none_or(|start| ts.is_none_or(|t| t + 5_000 >= start))
        };

        // Notified tasks (background agents finish with a task-notification).
        let task_status: HashMap<&str, &BgTask> =
            main_tasks.iter().map(|t| (t.id.as_str(), t)).collect();

        let mut agents: Vec<AgentInfo> = Vec::new();
        let mut views: HashMap<String, FileView> = HashMap::new();
        for a in files.subagents.values() {
            if let Some(v) = view(index, &a.path) {
                views.insert(a.agent_id.clone(), v);
            }
        }

        // ── Agent-tool subagents ──
        for a in files.subagents.values().filter(|a| a.run_id.is_none()) {
            let meta = self.meta(&a.meta);
            let v = views.get(&a.agent_id);
            let tool_use_id = meta.tool_use_id.get().map(str::to_string);
            let parent = meta.parent_agent_id.get().map(str::to_string);
            let calls: &[AgentCall] = match &parent {
                Some(p) => views.get(p).map(|v| v.calls.as_slice()).unwrap_or(&[]),
                None => &main_calls,
            };
            let call = tool_use_id
                .as_deref()
                .and_then(|t| calls.iter().find(|c| c.tool_use_id == t));
            let background = meta.request_shape.is("background")
                || call.is_some_and(|c| c.async_launched || c.background);
            let last_write = v.and_then(|v| v.last_ts).unwrap_or(a.mtime).max(a.mtime);
            let in_flight = v.is_some_and(|v| v.own_tools_in_flight);
            let fresh = ctx.now - last_write < IDLE_PLAUSIBLE_MS || in_flight;
            let this_process = started_ok(Some(a.mtime));
            let notified = task_status
                .get(a.agent_id.as_str())
                .filter(|t| t.status != "running")
                .map(|t| t.status.clone());
            let mut status = match (call, &notified) {
                (_, Some(s)) => finished_status(s),
                (Some(c), None) if c.result_seen && !c.async_launched => {
                    if c.is_error {
                        AgentStatus::Failed
                    } else {
                        AgentStatus::Done
                    }
                }
                (Some(_), None) => AgentStatus::Running,
                // No call record (parent not indexed yet / older CLI): go by
                // recency.
                (None, None) => {
                    if fresh {
                        AgentStatus::Running
                    } else {
                        AgentStatus::Unknown
                    }
                }
            };
            if status == AgentStatus::Running && !(ctx.live && this_process && fresh) {
                status = if ctx.live && this_process {
                    AgentStatus::Unknown
                } else {
                    AgentStatus::Stopped
                };
            }
            let running = status == AgentStatus::Running;
            let label = meta
                .description
                .text
                .clone()
                .or(meta.name.text.clone())
                .or_else(|| call.and_then(|c| c.description.clone()))
                .unwrap_or_else(|| a.agent_id.clone());
            let depth = meta
                .spawn_depth
                .0
                .unwrap_or(if parent.is_some() { 2 } else { 1 }) as u32;
            agents.push(AgentInfo {
                id: a.agent_id.clone(),
                kind: AgentKind::Subagent,
                session_id: sid.to_string(),
                parent_id: parent,
                depth,
                agent_type: meta
                    .agent_type
                    .0
                    .clone()
                    .or_else(|| call.and_then(|c| c.subagent_type.clone())),
                label,
                status,
                background,
                model: meta
                    .model
                    .0
                    .clone()
                    .or_else(|| v.and_then(|v| v.model.clone())),
                started_at: v.and_then(|v| v.first_ts).or(call.and_then(|c| c.ts)),
                ended_at: (!running && status != AgentStatus::Unknown)
                    .then(|| v.and_then(|v| v.last_ts))
                    .flatten(),
                last_activity_at: Some(last_write),
                activity: running
                    .then(|| v.and_then(|v| v.last_act.as_ref()).map(activity_of))
                    .flatten(),
                tokens: v.map(|v| v.tokens),
                tool_uses: v.map(|v| v.tool_uses),
                worktree: meta.worktree_path.text.clone(),
                tool_use_id,
                task_id: background.then(|| a.agent_id.clone()),
                can_stop: ctx.owned && running && background,
                ..Default::default()
            });
        }

        // ── Workflow runs and their agents ──
        let mut run_ids: BTreeMap<String, ()> = BTreeMap::new();
        for w in &main_wf {
            run_ids.insert(w.run_id.clone(), ());
        }
        for r in files.journals.keys().chain(files.run_files.keys()) {
            run_ids.insert(r.clone(), ());
        }
        let mut workflows: Vec<WorkflowRun> = Vec::new();
        for run_id in run_ids.keys() {
            let launch = main_wf.iter().find(|w| &w.run_id == run_id);
            let info = files.run_files.get(run_id).and_then(|p| self.run_info(p));
            let journal = files
                .journals
                .get(run_id)
                .map(|p| self.journal(p))
                .unwrap_or_default();
            // The run file is written when the run finishes, but may not be
            // discovered yet: the workflow task's notification in the main
            // transcript says the same.
            let notified = main_tasks.iter().find(|t| {
                t.kind == "workflow"
                    && t.status != "running"
                    && (t.run_id.as_deref() == Some(run_id.as_str())
                        || launch.and_then(|l| l.task_id.as_deref()) == Some(t.id.as_str()))
            });
            let finished = info.is_some() || notified.is_some();
            let status = match info.as_ref().and_then(|i| i.status.get()) {
                Some("completed") => WorkflowStatus::Completed,
                Some("failed") => WorkflowStatus::Failed,
                Some("killed") => WorkflowStatus::Killed,
                Some(_) => WorkflowStatus::Completed,
                None if notified.is_some() => match notified.map(|t| finished_status(&t.status)) {
                    Some(AgentStatus::Failed) => WorkflowStatus::Failed,
                    Some(AgentStatus::Stopped) => WorkflowStatus::Killed,
                    _ => WorkflowStatus::Completed,
                },
                None => {
                    // Launched by this process? From the launch record, else
                    // (older runs past the index's cap) from the journal's
                    // last write.
                    let launched_now = match launch {
                        Some(l) => started_ok(l.ts),
                        None => files
                            .journals
                            .get(run_id)
                            .and_then(|p| mtime_of(p))
                            .is_some_and(|m| started_ok(Some(m))),
                    };
                    if ctx.live && launched_now {
                        WorkflowStatus::Running
                    } else {
                        WorkflowStatus::Killed
                    }
                }
            };
            let run_running = status == WorkflowStatus::Running;
            let mut counts = WorkflowAgentCounts::default();
            let mut phases: Vec<WorkflowPhase> = info
                .as_ref()
                .map(|i| {
                    i.phases
                        .0
                        .iter()
                        .filter_map(|p| {
                            Some(WorkflowPhase {
                                title: p.title.text.clone()?,
                                detail: p.detail.text.clone(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (agent_id, label, phase) in &journal.started {
                counts.total += 1;
                if let Some(ph) = phase {
                    if !finished && !phases.iter().any(|p| &p.title == ph) {
                        phases.push(WorkflowPhase {
                            title: ph.clone(),
                            detail: None,
                        });
                    }
                }
                let done = journal.results.contains(agent_id);
                let v = views.get(agent_id);
                let fresh = v
                    .and_then(|v| v.last_ts)
                    .is_some_and(|t| ctx.now - t < IDLE_PLAUSIBLE_MS)
                    || v.is_some_and(|v| v.own_tools_in_flight);
                let astatus = if done {
                    counts.done += 1;
                    AgentStatus::Done
                } else if run_running {
                    counts.running += 1;
                    AgentStatus::Running
                } else {
                    match status {
                        WorkflowStatus::Failed => {
                            counts.failed += 1;
                            AgentStatus::Failed
                        }
                        _ => AgentStatus::Stopped,
                    }
                };
                let running = astatus == AgentStatus::Running;
                let file = files.subagents.get(agent_id);
                let meta = file.map(|f| self.meta(&f.meta)).unwrap_or_default();
                agents.push(AgentInfo {
                    id: agent_id.clone(),
                    kind: AgentKind::WorkflowAgent,
                    session_id: sid.to_string(),
                    depth: meta.spawn_depth.0.unwrap_or(1) as u32,
                    agent_type: meta.agent_type.0.clone(),
                    label: label.clone(),
                    phase: phase.clone().or(meta.workflow_phase.text.clone()),
                    workflow_run_id: Some(run_id.clone()),
                    status: astatus,
                    background: true,
                    model: v.and_then(|v| v.model.clone()),
                    started_at: v.and_then(|v| v.first_ts),
                    ended_at: (!running).then(|| v.and_then(|v| v.last_ts)).flatten(),
                    last_activity_at: v.and_then(|v| v.last_ts).or(file.map(|f| f.mtime)),
                    activity: (running && fresh)
                        .then(|| v.and_then(|v| v.last_act.as_ref()).map(activity_of))
                        .flatten(),
                    tokens: v.map(|v| v.tokens),
                    tool_uses: v.map(|v| v.tool_uses),
                    worktree: meta.worktree_path.text.clone(),
                    can_stop: false,
                    ..Default::default()
                });
            }
            if let Some(i) = &info {
                if counts.total == 0 {
                    counts.total = i.agent_count.0.unwrap_or(0) as u32;
                    counts.done = counts.total;
                }
            }
            workflows.push(WorkflowRun {
                run_id: run_id.clone(),
                name: info
                    .as_ref()
                    .and_then(|i| i.workflow_name.text.clone())
                    .or_else(|| launch.map(|l| l.name.clone()))
                    .unwrap_or_else(|| run_id.clone()),
                status,
                phases,
                agents: counts,
                started_at: info
                    .as_ref()
                    .and_then(|i| i.start_time.0.map(|t| t as i64))
                    .or(launch.and_then(|l| l.ts)),
                duration_ms: info.as_ref().and_then(|i| i.duration_ms.0),
                task_id: info
                    .as_ref()
                    .and_then(|i| i.task_id.0.clone())
                    .or_else(|| launch.and_then(|l| l.task_id.clone())),
                total_tokens: info.as_ref().and_then(|i| i.total_tokens.0),
            });
        }
        workflows.sort_by_key(|w| std::cmp::Reverse(w.started_at));

        // ── Background shells and monitors ──
        let mut shells_finished = 0usize;
        for t in main_tasks
            .iter()
            .rev()
            .filter(|t| matches!(t.kind.as_str(), "bash" | "monitor"))
        {
            let running = t.status == "running" && ctx.live && started_ok(t.started_at);
            let status = if running {
                AgentStatus::Running
            } else if t.status == "running" {
                AgentStatus::Stopped
            } else {
                finished_status(&t.status)
            };
            if !running {
                if shells_finished >= FINISHED_SHELLS_KEPT {
                    continue;
                }
                shells_finished += 1;
            }
            agents.push(AgentInfo {
                id: t.id.clone(),
                kind: if t.kind == "monitor" {
                    AgentKind::Monitor
                } else {
                    AgentKind::Bash
                },
                session_id: sid.to_string(),
                depth: 1,
                label: t.label.clone(),
                status,
                background: true,
                started_at: t.started_at,
                ended_at: t.ended_at,
                last_activity_at: t.ended_at.or(t.started_at),
                tool_use_id: t.tool_use_id.clone(),
                task_id: Some(t.id.clone()),
                can_stop: ctx.owned && running,
                ..Default::default()
            });
        }

        let mut counts = AgentCounts::default();
        for a in &agents {
            let running = a.status == AgentStatus::Running;
            match a.kind {
                AgentKind::Subagent | AgentKind::WorkflowAgent | AgentKind::Teammate => {
                    counts.total += 1;
                    counts.running += running as u32;
                }
                AgentKind::Bash | AgentKind::Monitor => {
                    counts.background_running += running as u32;
                }
            }
        }
        counts.workflows_running = workflows
            .iter()
            .filter(|w| w.status == WorkflowStatus::Running)
            .count() as u32;
        // Running first, then most recent; long histories keep only the
        // newest finished entries (counts above cover everything).
        agents.sort_by(|a, b| {
            (b.status == AgentStatus::Running)
                .cmp(&(a.status == AgentStatus::Running))
                .then(b.last_activity_at.cmp(&a.last_activity_at))
                .then(a.id.cmp(&b.id))
        });
        let running = agents
            .iter()
            .filter(|a| a.status == AgentStatus::Running)
            .count();
        agents.truncate(running + FINISHED_AGENTS_KEPT);
        let running_runs = workflows
            .iter()
            .filter(|w| w.status == WorkflowStatus::Running)
            .count();
        workflows.sort_by_key(|w| {
            (
                w.status != WorkflowStatus::Running,
                std::cmp::Reverse(w.started_at),
            )
        });
        workflows.truncate(running_runs + FINISHED_WORKFLOWS_KEPT);
        let hash = hash_of(&(&agents, &workflows));
        SessionAgents {
            agents,
            workflows,
            counts,
            hash,
        }
    }
}

fn finished_status(s: &str) -> AgentStatus {
    match s {
        "completed" | "success" | "done" => AgentStatus::Done,
        "failed" | "error" => AgentStatus::Failed,
        "killed" | "stopped" | "cancelled" => AgentStatus::Stopped,
        "running" => AgentStatus::Running,
        _ => AgentStatus::Unknown,
    }
}

/// Only running agents (and running workflows) of a session, for the fleet.
pub fn fleet_entry(
    session_id: &str,
    name: &str,
    agents: &[AgentInfo],
    workflows: &[WorkflowRun],
) -> Option<FleetSession> {
    let running: Vec<AgentInfo> = agents
        .iter()
        .filter(|a| a.status == AgentStatus::Running)
        .cloned()
        .collect();
    let wf: Vec<WorkflowRun> = workflows
        .iter()
        .filter(|w| w.status == WorkflowStatus::Running)
        .cloned()
        .collect();
    (!running.is_empty() || !wf.is_empty()).then(|| FleetSession {
        session_id: session_id.to_string(),
        name: name.to_string(),
        agents: running,
        workflows: wf,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::home::ClaudeHome;

    const SID: &str = "e2b6972f-7126-4f6e-a490-5ce75cb6a9f9";

    fn now() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    fn iso(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    fn setup() -> (tempfile::TempDir, ClaudeHome, TranscriptIndex) {
        let tmp = tempfile::tempdir().unwrap();
        let home = ClaudeHome::with_base(tmp.path());
        let proj = tmp.path().join("projects/-p");
        let sdir = proj.join(SID);
        let sub = sdir.join("subagents");
        std::fs::create_dir_all(sub.join("workflows/wf_run-1")).unwrap();
        let t = iso(now() - 1000);
        // Main transcript: a foreground Agent call (no result yet → running),
        // a finished one, a background agent (async_launched, no
        // notification → running), a background bash, and a workflow launch.
        let main = [
            format!(r#"{{"type":"assistant","timestamp":"{t}","message":{{"id":"m1","model":"claude-opus-5","stop_reason":"tool_use","content":[{{"type":"tool_use","id":"tu_fg","name":"Agent","input":{{"description":"Explore code","subagent_type":"Explore"}}}},{{"type":"tool_use","id":"tu_done","name":"Agent","input":{{"description":"Quick look"}}}},{{"type":"tool_use","id":"tu_bg","name":"Agent","input":{{"description":"Long task","run_in_background":true}}}},{{"type":"tool_use","id":"tu_sh","name":"Bash","input":{{"command":"npm run dev","run_in_background":true}}}},{{"type":"tool_use","id":"tu_wf","name":"Workflow","input":{{"name":"big-plan"}}}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#),
            format!(r#"{{"type":"user","timestamp":"{t}","message":{{"content":[{{"type":"tool_result","tool_use_id":"tu_done","content":"ok"}}]}},"toolUseResult":{{"status":"completed","agentId":"adone"}}}}"#),
            format!(r#"{{"type":"user","timestamp":"{t}","message":{{"content":[{{"type":"tool_result","tool_use_id":"tu_bg","content":"launched"}}]}},"toolUseResult":{{"status":"async_launched","agentId":"abg","description":"Long task","isAsync":true}}}}"#),
            format!(r#"{{"type":"user","timestamp":"{t}","message":{{"content":[{{"type":"tool_result","tool_use_id":"tu_sh","content":"started"}}]}},"toolUseResult":{{"backgroundTaskId":"bsh1"}}}}"#),
            format!(r#"{{"type":"user","timestamp":"{t}","message":{{"content":[{{"type":"tool_result","tool_use_id":"tu_wf","content":"launched"}}]}},"toolUseResult":{{"status":"async_launched","taskId":"wtask1","runId":"wf_run-1","workflowName":"big-plan"}}}}"#),
        ]
        .join("\n")
            + "\n";
        std::fs::write(proj.join(format!("{SID}.jsonl")), main).unwrap();
        let agent_line = |id: &str| {
            format!(
                r#"{{"type":"assistant","isSidechain":true,"timestamp":"{t}","message":{{"id":"{id}-m","model":"claude-haiku-4-5","content":[{{"type":"tool_use","id":"{id}-t","name":"Grep","input":{{"pattern":"TODO"}}}}],"usage":{{"input_tokens":10,"output_tokens":5}}}}}}"#
            ) + "\n"
        };
        for (id, tu, shape) in [
            ("afg", "tu_fg", "foreground"),
            ("adone", "tu_done", "foreground"),
            ("abg", "tu_bg", "background"),
        ] {
            std::fs::write(sub.join(format!("agent-{id}.jsonl")), agent_line(id)).unwrap();
            std::fs::write(
                sub.join(format!("agent-{id}.meta.json")),
                format!(r#"{{"agentType":"general-purpose","description":"desc {id}","toolUseId":"{tu}","spawnDepth":1,"requestShape":"{shape}"}}"#),
            )
            .unwrap();
        }
        // Nested agent under afg: its toolUseId lives in agent-afg.jsonl.
        std::fs::write(sub.join("agent-anest.jsonl"), agent_line("anest")).unwrap();
        std::fs::write(
            sub.join("agent-anest.meta.json"),
            r#"{"agentType":"Explore","description":"nested","toolUseId":"afg-t","parentAgentId":"afg","spawnDepth":2}"#,
        )
        .unwrap();
        // Workflow: 3 started, 1 result ⇒ 2 running.
        let wf = sub.join("workflows/wf_run-1");
        std::fs::write(
            wf.join("journal.jsonl"),
            concat!(
                r#"{"type":"launched"}"#,
                "\n",
                r#"{"type":"started","key":"k1","agentId":"aw1","label":"plan","phase":"Plan"}"#,
                "\n",
                r#"{"type":"started","key":"k2","agentId":"aw2","label":"build","phase":"Build"}"#,
                "\n",
                r#"{"type":"started","key":"k3","agentId":"aw3","label":"test","phase":"Build"}"#,
                "\n",
                r#"{"type":"result","key":"k1","agentId":"aw1","result":"big text"}"#,
                "\n",
            ),
        )
        .unwrap();
        for id in ["aw1", "aw2", "aw3"] {
            std::fs::write(wf.join(format!("agent-{id}.jsonl")), agent_line(id)).unwrap();
            std::fs::write(
                wf.join(format!("agent-{id}.meta.json")),
                r#"{"agentType":"workflow-subagent","spawnDepth":1}"#,
            )
            .unwrap();
        }
        let idx = TranscriptIndex::new(None);
        idx.discover(&home);
        idx.queue_all();
        idx.drain_jobs_blocking();
        (tmp, home, idx)
    }

    #[test]
    fn maps_subagents_workflows_and_shells() {
        let (_tmp, _home, idx) = setup();
        let tracker = AgentTracker::new();
        let ctx = AgentCtx {
            live: true,
            process_started: Some(now() - 60_000),
            owned: false,
            now: now(),
        };
        assert!(tracker.recompute(&idx, SID, &ctx));
        assert!(!tracker.recompute(&idx, SID, &ctx), "unchanged ⇒ no change");
        let (agents, workflows) = tracker.for_session(SID);
        let by = |id: &str| agents.iter().find(|a| a.id == id).unwrap();
        assert_eq!(by("afg").status, AgentStatus::Running);
        assert_eq!(by("afg").agent_type.as_deref(), Some("general-purpose"));
        assert_eq!(by("afg").tokens, Some(15));
        assert_eq!(by("afg").tool_uses, Some(1));
        assert_eq!(by("adone").status, AgentStatus::Done);
        assert_eq!(by("abg").status, AgentStatus::Running);
        assert!(by("abg").background);
        assert_eq!(by("abg").task_id.as_deref(), Some("abg"));
        let nested = by("anest");
        assert_eq!(nested.parent_id.as_deref(), Some("afg"));
        assert_eq!(nested.depth, 2);
        // The nested agent's call (afg-t) has no result in agent-afg.jsonl.
        assert_eq!(nested.status, AgentStatus::Running);
        assert!(by("afg")
            .activity
            .as_ref()
            .is_some_and(|a| a.label == "Running Grep"));
        // Workflow.
        assert_eq!(workflows.len(), 1);
        let w = &workflows[0];
        assert_eq!(w.name, "big-plan");
        assert_eq!(w.status, WorkflowStatus::Running);
        assert_eq!(w.agents.total, 3);
        assert_eq!(w.agents.running, 2);
        assert_eq!(w.agents.done, 1);
        assert_eq!(w.task_id.as_deref(), Some("wtask1"));
        assert_eq!(by("aw2").kind, AgentKind::WorkflowAgent);
        assert_eq!(by("aw2").phase.as_deref(), Some("Build"));
        // Shell.
        let sh = by("bsh1");
        assert_eq!(sh.kind, AgentKind::Bash);
        assert_eq!(sh.status, AgentStatus::Running);
        let c = tracker.counts(SID).unwrap();
        assert_eq!(c.running, 3 + 2); // afg, abg, anest + aw2, aw3
        assert_eq!(c.total, 4 + 3);
        assert_eq!(c.workflows_running, 1);
        assert_eq!(c.background_running, 1);
        // Fleet entry has only running things.
        let f = fleet_entry(SID, "s", &agents, &workflows).unwrap();
        assert!(f.agents.iter().all(|a| a.status == AgentStatus::Running));
        assert_eq!(f.workflows.len(), 1);
    }

    #[test]
    fn a_dead_process_stops_everything_and_a_finished_run_counts() {
        let (tmp, home, idx) = setup();
        let tracker = AgentTracker::new();
        tracker.recompute(
            &idx,
            SID,
            &AgentCtx {
                live: false,
                now: now(),
                ..Default::default()
            },
        );
        let (agents, workflows) = tracker.for_session(SID);
        assert!(agents.iter().all(|a| a.status != AgentStatus::Running));
        assert_eq!(workflows[0].status, WorkflowStatus::Killed);
        assert_eq!(tracker.counts(SID).unwrap().running, 0);

        // The run finishes: `workflows/<run>.json` appears.
        let wdir = tmp.path().join("projects/-p").join(SID).join("workflows");
        std::fs::create_dir_all(&wdir).unwrap();
        std::fs::write(
            wdir.join("wf_run-1.json"),
            r#"{"runId":"wf_run-1","status":"completed","workflowName":"big-plan","durationMs":1234,"totalTokens":99,"agentCount":3,"phases":[{"title":"Plan","detail":"p"},{"title":"Build"}],"result":"HUGE","script":"x"}"#,
        )
        .unwrap();
        idx.discover(&home);
        tracker.recompute(
            &idx,
            SID,
            &AgentCtx {
                live: true,
                now: now(),
                ..Default::default()
            },
        );
        let (_, workflows) = tracker.for_session(SID);
        let w = &workflows[0];
        assert_eq!(w.status, WorkflowStatus::Completed);
        assert_eq!(w.duration_ms, Some(1234));
        assert_eq!(w.total_tokens, Some(99));
        assert_eq!(w.phases.len(), 2);
        assert_eq!(w.agents.running, 0);
    }

    #[test]
    fn a_notified_workflow_is_finished_before_its_run_file_is_discovered() {
        let (tmp, _home, idx) = setup();
        let main = tmp.path().join("projects/-p").join(format!("{SID}.jsonl"));
        let ctx = AgentCtx {
            live: true,
            now: now(),
            ..Default::default()
        };
        let tracker = AgentTracker::new();
        tracker.recompute(&idx, SID, &ctx);
        assert_eq!(
            tracker.for_session(SID).1[0].status,
            WorkflowStatus::Running
        );
        // The workflow task's notification lands in the main transcript; the
        // run file is not discovered yet.
        let note = format!(
            r#"{{"type":"queue-operation","operation":"enqueue","timestamp":"{}","content":"<task-notification>\n<task-id>wtask1</task-id>\n<status>completed</status>\n<summary>done</summary>\n</task-notification>"}}"#,
            iso(now())
        ) + "\n";
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&main)
            .unwrap();
        std::io::Write::write_all(&mut f, note.as_bytes()).unwrap();
        idx.update(std::slice::from_ref(&main));
        tracker.recompute(&idx, SID, &ctx);
        let (agents, workflows) = tracker.for_session(SID);
        assert_eq!(workflows[0].status, WorkflowStatus::Completed);
        assert_eq!(workflows[0].agents.running, 0);
        assert!(!agents
            .iter()
            .any(|a| a.workflow_run_id.is_some() && a.status == AgentStatus::Running));
    }
}
