//! Unified session registry.
//!
//! Merges three views into one [`Session`] list:
//!  - `claude agents --json [--all]` — the authoritative roster of *running*
//!    sessions (pid, cwd, kind). On 2.1.185 this carries no state field.
//!  - transcript files under `projects/` — captures **every** surface (CLI,
//!    VS Code/JetBrains, Claude Desktop) and yields model, usage, activity,
//!    title and message counts.
//!  - `jobs/<id>/state.json` — an optional explicit state override.
//!
//! State is *derived* (working / needs-input / idle / completed / …) because the
//! CLI does not report it. `owned` sessions (those Mother Claude spawned) and any
//! live `pending` input are merged in from maps the higher layers maintain.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::control::OwnedSessionMeta;
use super::home::ClaudeHome;
use super::schema::{AgentEntry, ContentBlock, MessageContent, StateJson, TranscriptEvent};
use super::transcript::read_all;

/// Window (ms) within which recent transcript activity counts as "working".
const WORKING_WINDOW_MS: i64 = 20_000;

/// Editor/CLI surface a session was started from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    Cli,
    VsCode,
    JetBrains,
    Desktop,
    #[default]
    Unknown,
}

impl Surface {
    /// Map a transcript `entrypoint` value to a surface.
    pub fn from_entrypoint(entrypoint: &str) -> Self {
        let e = entrypoint.to_ascii_lowercase();
        if e == "cli" || e.contains("terminal") {
            Surface::Cli
        } else if e.contains("vscode") || e.contains("vs-code") {
            Surface::VsCode
        } else if e.contains("jetbrains") || e.contains("intellij") {
            Surface::JetBrains
        } else if e.contains("desktop") {
            Surface::Desktop
        } else {
            Surface::Unknown
        }
    }
}

/// Derived lifecycle state of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    Working,
    NeedsInput,
    Idle,
    Completed,
    Failed,
    Stopped,
    Unknown,
}

/// One selectable option of a pending question.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// Tolerant input: options arrive either as plain strings (legacy sidecar /
// simple askers) or as `{label, description}` objects (AskUserQuestion-shaped).
impl<'de> serde::Deserialize<'de> for QuestionOption {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Text(String),
            Full {
                label: String,
                #[serde(default)]
                description: Option<String>,
            },
        }
        Ok(match Repr::deserialize(deserializer)? {
            Repr::Text(label) => QuestionOption {
                label,
                description: None,
            },
            Repr::Full { label, description } => QuestionOption { label, description },
        })
    }
}

/// A pending question or permission prompt awaiting a human answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInput {
    pub kind: PendingKind,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub prompt: Option<String>,
    /// Very short topic chip (AskUserQuestion `header`), e.g. "Auth method".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub header: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<QuestionOption>,
    /// Whether several options may be selected together.
    #[serde(default)]
    pub multi_select: bool,
    /// Salient context: the Bash command / file path for permissions, or the
    /// plan text for plan approval. Pre-summarized, display-ready.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<String>,
    /// Correlates an answer back to the blocked request (owned sessions).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,
    /// Whether this can actually be answered live (false for foreign sessions).
    #[serde(default)]
    pub answerable: bool,
    /// Whether answering would grant a dangerous permission (bypass, etc.).
    #[serde(default)]
    pub dangerous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PendingKind {
    Permission,
    Question,
}

/// Aggregated token usage for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_tokens: u64,
}

/// The unified, dashboard-facing session record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub project_name: String,
    pub surface: Surface,
    pub owned: bool,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    pub running: bool,
    pub message_count: u64,
    pub usage: UsageSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingInput>,
    /// Background tasks (bash / agents / workflows) this session launched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<BackgroundTask>,
    /// Whether live answer/permission injection is possible for this session.
    /// True only for owned sessions (foreign sessions are monitor + lifecycle).
    pub can_inject: bool,
}

/// A background task/agent/workflow a session launched, with its lifecycle
/// derived from the transcript (launch tool_use → async-launch result →
/// `task-notification` completion event).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTask {
    /// Task id as notifications reference it (b* bash, w* workflow, a* agent).
    pub id: String,
    /// "bash" | "agent" | "workflow".
    pub kind: String,
    /// Human label: the command description, agent description, or workflow name.
    pub label: String,
    /// "running" | "completed" | "failed" | "killed".
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ended_at: Option<i64>,
}

/// Per-transcript summary derived from one `<id>.jsonl` file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSummary {
    pub id: String,
    pub cwd: Option<String>,
    pub surface: Surface,
    pub model: Option<String>,
    pub title: Option<String>,
    pub git_branch: Option<String>,
    pub started_at: Option<i64>,
    pub last_activity: Option<i64>,
    pub message_count: u64,
    pub usage: UsageSummary,
    /// A question / plan approval the transcript ends on, still unanswered —
    /// how foreign sessions' AskUserQuestion prompts become visible.
    pub pending: Option<PendingInput>,
    /// Background tasks the session launched (most recent last, capped).
    pub tasks: Vec<BackgroundTask>,
}

/// Parse an RFC3339 timestamp (e.g. `2026-06-23T08:59:08.494Z`) to epoch millis.
fn parse_ts_ms(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Current wall-clock time in epoch millis.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Summarize an already-parsed transcript into a [`TranscriptSummary`].
pub fn summarize_transcript(id: &str, events: &[TranscriptEvent]) -> TranscriptSummary {
    let mut summary = TranscriptSummary {
        id: id.to_string(),
        ..Default::default()
    };

    // Background-task launches awaiting their async-launch confirmation,
    // keyed by the launching tool_use id.
    let mut launches: HashMap<String, (String, String)> = HashMap::new();

    for ev in events {
        if let Some(ts) = ev.timestamp.as_deref().and_then(parse_ts_ms) {
            summary.started_at = Some(summary.started_at.map_or(ts, |s| s.min(ts)));
            summary.last_activity = Some(summary.last_activity.map_or(ts, |l| l.max(ts)));
        }
        // Subagent (sidechain) traffic shares the file but is not the main
        // conversation: it must not clear a pending main-chain question, nor
        // skew model/count. (Its timestamps above still count as activity.)
        if ev.is_sidechain == Some(true) {
            continue;
        }
        if let Some(cwd) = &ev.cwd {
            summary.cwd = Some(cwd.clone());
        }
        if let Some(branch) = &ev.git_branch {
            summary.git_branch = Some(branch.clone());
        }
        if let Some(entry) = ev.extra.get("entrypoint").and_then(|v| v.as_str()) {
            summary.surface = Surface::from_entrypoint(entry);
        }
        if ev.event_type == "ai-title" {
            if let Some(t) = ev.extra.get("aiTitle").and_then(|v| v.as_str()) {
                summary.title = Some(t.to_string());
            }
        }
        if ev.event_type == "user" || ev.event_type == "assistant" {
            summary.message_count += 1;
        }
        // A user event answers the open question only when it actually
        // addresses it: a tool_result for that question's tool_use id, or the
        // user typing a message. A parallel tool's result must not clear it —
        // and neither must an injected task-notification (not the user).
        if ev.event_type == "user" {
            if is_task_notification(ev) {
                apply_task_notification(ev, &mut summary.tasks);
            } else {
                record_task_launch_result(ev, &mut launches, &mut summary.tasks);
                if summary.pending.is_some()
                    && user_event_answers(
                        ev,
                        summary
                            .pending
                            .as_ref()
                            .and_then(|p| p.request_id.as_deref()),
                    )
                {
                    summary.pending = None;
                }
            }
        }
        if let Some(msg) = &ev.message {
            if ev.event_type == "assistant" {
                if let Some(model) = &msg.model {
                    summary.model = Some(model.clone());
                }
                if let Some(MessageContent::Blocks(blocks)) = &msg.content {
                    for block in blocks {
                        if block.block_type == "tool_use" {
                            if let Some(p) = pending_from_tool_use(block) {
                                summary.pending = Some(p);
                            }
                            if let (Some(id), Some(launch)) =
                                (&block.id, task_launch_from_tool_use(block))
                            {
                                launches.insert(id.clone(), launch);
                            }
                        }
                    }
                }
            }
            if let Some(usage) = &msg.usage {
                summary.usage.input_tokens += usage.input_tokens.unwrap_or(0);
                summary.usage.output_tokens += usage.output_tokens.unwrap_or(0);
                summary.usage.cache_read_tokens += usage.cache_read_input_tokens.unwrap_or(0);
                summary.usage.cache_creation_tokens +=
                    usage.cache_creation_input_tokens.unwrap_or(0);
            }
        }
    }
    summary.usage.total_tokens = summary.usage.input_tokens
        + summary.usage.output_tokens
        + summary.usage.cache_read_tokens
        + summary.usage.cache_creation_tokens;
    summary
}

/// How many background tasks to remember per session.
const TASKS_CAP: usize = 20;

/// True for the SDK-injected `task-notification` user events that report a
/// background task finishing.
fn is_task_notification(ev: &TranscriptEvent) -> bool {
    ev.extra
        .get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(|k| k.as_str())
        == Some("task-notification")
}

/// Extract `<tag>…</tag>` from a notification's text (no regex needed).
fn tag_value<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim())
}

/// If this tool_use launches background work, describe it as (kind, label).
fn task_launch_from_tool_use(block: &ContentBlock) -> Option<(String, String)> {
    const LABEL_MAX: usize = 120;
    let name = block.name.as_deref()?;
    let input = block.input.as_ref();
    let field = |key: &str| {
        input
            .and_then(|i| i.get(key))
            .and_then(|v| v.as_str())
            .map(|v| truncate_chars(v, LABEL_MAX))
    };
    match name {
        "Bash" => {
            let background = input
                .and_then(|i| i.get("run_in_background"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            background.then(|| {
                (
                    "bash".to_string(),
                    field("description")
                        .or_else(|| field("command"))
                        .unwrap_or_else(|| "background command".into()),
                )
            })
        }
        "Agent" => Some((
            "agent".to_string(),
            field("description")
                .or_else(|| field("name"))
                .unwrap_or_else(|| "subagent".into()),
        )),
        "Workflow" => Some((
            "workflow".to_string(),
            field("name").unwrap_or_else(|| "workflow".into()),
        )),
        _ => None,
    }
}

/// A user event carrying the async-launch confirmation: its wrapper
/// `toolUseResult` holds the task id the later notification will reference.
fn record_task_launch_result(
    ev: &TranscriptEvent,
    launches: &mut HashMap<String, (String, String)>,
    tasks: &mut Vec<BackgroundTask>,
) {
    let Some(result) = ev.extra.get("toolUseResult").and_then(|v| v.as_object()) else {
        return;
    };
    // Only ASYNC launches become background tasks. Synchronous Agent runs also
    // carry an agentId in their result (status: "completed") — recording those
    // as "running" would pin a false badge forever, since no task-notification
    // will ever arrive for them.
    let is_async = result.get("status").and_then(|v| v.as_str()) == Some("async_launched")
        || result.get("isAsync").and_then(|v| v.as_bool()) == Some(true)
        || result.contains_key("backgroundTaskId");
    if !is_async {
        return;
    }
    // Which launch does this confirm? Match the tool_result block that belongs
    // to a pending launch (defensive against multi-result events).
    let tool_use_id = ev
        .message
        .as_ref()
        .and_then(|m| m.content.as_ref())
        .and_then(|c| match c {
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter(|b| b.block_type == "tool_result")
                .find_map(|b| b.tool_use_id.clone().filter(|id| launches.contains_key(id))),
            MessageContent::Text(_) => None,
        });
    let Some(tool_use_id) = tool_use_id else {
        return;
    };
    let Some((kind, label)) = launches.remove(&tool_use_id) else {
        return;
    };
    let id = ["backgroundTaskId", "taskId", "agentId"]
        .iter()
        .find_map(|k| result.get(*k).and_then(|v| v.as_str()));
    let Some(id) = id else {
        return;
    };
    // Richer labels arrive with the confirmation (workflow name, description).
    let label = ["workflowName", "description"]
        .iter()
        .find_map(|k| result.get(*k).and_then(|v| v.as_str()))
        .map(|v| truncate_chars(v, 120))
        .unwrap_or(label);
    push_task(
        tasks,
        BackgroundTask {
            id: id.to_string(),
            kind,
            label,
            status: "running".into(),
            started_at: ev.timestamp.as_deref().and_then(parse_ts_ms),
            ended_at: None,
        },
    );
}

/// Apply a `task-notification` to the task list (or record an unseen task).
fn apply_task_notification(ev: &TranscriptEvent, tasks: &mut Vec<BackgroundTask>) {
    let Some(MessageContent::Text(content)) = ev.message.as_ref().and_then(|m| m.content.as_ref())
    else {
        return;
    };
    let (Some(id), Some(status)) = (tag_value(content, "task-id"), tag_value(content, "status"))
    else {
        return;
    };
    let ended_at = ev.timestamp.as_deref().and_then(parse_ts_ms);
    if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
        task.status = status.to_string();
        task.ended_at = ended_at;
        return;
    }
    // Completion for a launch outside what we saw (e.g. old transcript form).
    let kind = match id.chars().next() {
        Some('w') => "workflow",
        Some('a') => "agent",
        _ => "bash",
    };
    push_task(
        tasks,
        BackgroundTask {
            id: id.to_string(),
            kind: kind.into(),
            label: tag_value(content, "summary")
                .map(|v| truncate_chars(v, 120))
                .unwrap_or_default(),
            status: status.to_string(),
            started_at: None,
            ended_at,
        },
    );
}

fn push_task(tasks: &mut Vec<BackgroundTask>, task: BackgroundTask) {
    tasks.push(task);
    if tasks.len() > TASKS_CAP {
        let excess = tasks.len() - TASKS_CAP;
        tasks.drain(..excess);
    }
}

/// Whether a `user` event resolves the open transcript-derived prompt: it
/// carries a `tool_result` for the prompt's tool_use id (stored in
/// `request_id`), or the user typed actual text (the conversation moved on).
/// Without an id to correlate, any user event clears (conservative).
fn user_event_answers(ev: &TranscriptEvent, tool_use_id: Option<&str>) -> bool {
    let Some(tuid) = tool_use_id else {
        return true;
    };
    match ev.message.as_ref().and_then(|m| m.content.as_ref()) {
        Some(MessageContent::Text(t)) => !t.trim().is_empty(),
        Some(MessageContent::Blocks(blocks)) => blocks.iter().any(|b| {
            (b.block_type == "tool_result" && b.tool_use_id.as_deref() == Some(tuid))
                || (b.block_type == "text"
                    && b.text.as_deref().is_some_and(|t| !t.trim().is_empty()))
        }),
        None => true,
    }
}

/// If this `tool_use` block is a question or plan approval the user must act
/// on, describe it as a [`PendingInput`] (not answerable — transcript-derived
/// prompts belong to sessions we don't own; `request_id` carries the tool_use
/// id so the answer can be correlated).
///
/// Recognized: the native `AskUserQuestion` (`{questions: [{question, header,
/// multiSelect, options: [{label, description}]}]}`), our sidecar's
/// `mcp__mother-claude__ask_user` (`{question, header?, options?,
/// multiSelect?}`), and `ExitPlanMode` (plan approval).
fn pending_from_tool_use(block: &ContentBlock) -> Option<PendingInput> {
    const DETAIL_MAX: usize = 2_000;
    let name = block.name.as_deref()?;
    let input = block.input.as_ref()?;

    if name == "ExitPlanMode" {
        let plan = input.get("plan").and_then(|v| v.as_str()).unwrap_or("");
        return Some(PendingInput {
            kind: PendingKind::Permission,
            tool: Some(name.to_string()),
            prompt: Some(
                "Claude finished planning and asks approval to start implementing.".into(),
            ),
            header: Some("Plan ready".into()),
            options: Vec::new(),
            multi_select: false,
            detail: (!plan.is_empty()).then(|| truncate_chars(plan, DETAIL_MAX)),
            request_id: block.id.clone(),
            answerable: false,
            dangerous: false,
        });
    }

    if name != "AskUserQuestion" && !name.ends_with("__ask_user") {
        return None;
    }
    // AskUserQuestion nests under `questions[]`; ask_user is the object itself.
    let all = input.get("questions").and_then(|v| v.as_array());
    let extra_questions = all.map(|a| a.len().saturating_sub(1)).unwrap_or(0);
    let q = all.and_then(|a| a.first()).unwrap_or(input);
    let mut prompt = q.get("question").and_then(|v| v.as_str())?.to_string();
    if extra_questions > 0 {
        prompt.push_str(&format!(
            " (+{extra_questions} more question{} in the session)",
            if extra_questions == 1 { "" } else { "s" }
        ));
    }
    let options = q
        .get("options")
        .and_then(|v| v.as_array())
        .map(|opts| {
            opts.iter()
                .filter_map(|o| {
                    if let Some(s) = o.as_str() {
                        return Some(QuestionOption {
                            label: s.to_string(),
                            description: None,
                        });
                    }
                    Some(QuestionOption {
                        label: o.get("label")?.as_str()?.to_string(),
                        description: o
                            .get("description")
                            .and_then(|d| d.as_str())
                            .map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(PendingInput {
        kind: PendingKind::Question,
        tool: Some(name.to_string()),
        prompt: Some(prompt),
        header: q.get("header").and_then(|v| v.as_str()).map(String::from),
        options,
        multi_select: q
            .get("multiSelect")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        detail: None,
        request_id: block.id.clone(),
        answerable: false,
        dangerous: false,
    })
}

/// Truncate to at most `max` characters on a char boundary, appending `…`.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// Scan every transcript under `projects/` into summaries. I/O errors per
/// file are logged and skipped.
pub fn scan_transcripts(home: &ClaudeHome) -> Vec<TranscriptSummary> {
    let mut out = Vec::new();
    let projects = home.projects_dir();
    let Ok(project_dirs) = std::fs::read_dir(&projects) else {
        return out;
    };
    for project in project_dirs.filter_map(|e| e.ok()) {
        let dir = project.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.filter_map(|e| e.ok()) {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            match read_all(&path) {
                Ok(events) if !events.is_empty() => out.push(summarize_transcript(id, &events)),
                Ok(_) => {}
                Err(e) => tracing::debug!(path = %path.display(), error = %e, "skip transcript"),
            }
        }
    }
    out
}

/// Read every `jobs/<id>/state.json` into a map keyed by session id.
pub fn read_state_jsons(home: &ClaudeHome) -> HashMap<String, StateJson> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(home.jobs_dir()) else {
        return map;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(id) = dir.file_name().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        let state_path = dir.join("state.json");
        if let Ok(text) = std::fs::read_to_string(&state_path) {
            match serde_json::from_str::<StateJson>(&text) {
                Ok(state) => {
                    map.insert(id, state);
                }
                Err(e) => {
                    tracing::debug!(path = %state_path.display(), error = %e, "skip state.json")
                }
            }
        }
    }
    map
}

/// Run `claude agents --json [--all]`. Never errors out the dashboard: on any
/// failure (CLI missing, non-zero exit, bad JSON) it logs and returns empty.
pub async fn query_agents(all: bool) -> Vec<AgentEntry> {
    let mut cmd = tokio::process::Command::new(super::claude_bin());
    cmd.arg("agents").arg("--json");
    if all {
        cmd.arg("--all");
    }
    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "failed to run `claude agents --json`");
            return Vec::new();
        }
    };
    if !output.status.success() {
        tracing::warn!(
            status = ?output.status.code(),
            stderr = %String::from_utf8_lossy(&output.stderr),
            "`claude agents --json` exited non-zero"
        );
        return Vec::new();
    }
    match serde_json::from_slice::<Vec<AgentEntry>>(&output.stdout) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "could not parse `claude agents --json` output");
            Vec::new()
        }
    }
}

/// Inputs to [`build_registry`]; grouped to keep the signature tidy.
pub struct RegistryInputs<'a> {
    pub agents: &'a [AgentEntry],
    pub transcripts: &'a [TranscriptSummary],
    pub owned: &'a HashSet<String>,
    pub pending: &'a HashMap<String, PendingInput>,
    pub states: &'a HashMap<String, StateJson>,
    pub now_ms: i64,
    /// Whether running foreign sessions can be driven via PTY injection.
    pub foreign_injection: bool,
    /// Live owned sessions, so freshly-spawned/resumed ones are listed even
    /// before their transcript file exists.
    pub owned_live: &'a [OwnedSessionMeta],
}

fn project_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .map(String::from)
        .unwrap_or_else(|| cwd.to_string())
}

fn map_explicit_state(state: &StateJson) -> Option<SessionState> {
    let raw = state
        .state
        .as_deref()
        .or(state.status.as_deref())?
        .to_ascii_lowercase();
    Some(match raw.as_str() {
        "working" | "running" | "active" | "busy" => SessionState::Working,
        "needs-input" | "needs_input" | "waiting" | "blocked" => SessionState::NeedsInput,
        "idle" | "ready" => SessionState::Idle,
        "completed" | "done" | "finished" | "exited" => SessionState::Completed,
        "failed" | "error" | "errored" => SessionState::Failed,
        "stopped" | "killed" | "cancelled" | "canceled" => SessionState::Stopped,
        _ => return None,
    })
}

/// Merge all inputs into a sorted (most-recent-first) session list. Pure and
/// deterministic given `now_ms`, so it is fully unit-testable.
pub fn build_registry(inputs: RegistryInputs) -> Vec<Session> {
    let RegistryInputs {
        agents,
        transcripts,
        owned,
        pending,
        states,
        now_ms,
        foreign_injection,
        owned_live,
    } = inputs;

    let agent_by_id: HashMap<&str, &AgentEntry> = agents
        .iter()
        .filter_map(|a| a.session_id.as_deref().map(|id| (id, a)))
        .collect();
    let transcript_by_id: HashMap<&str, &TranscriptSummary> =
        transcripts.iter().map(|t| (t.id.as_str(), t)).collect();

    // Union of all known session ids.
    let mut ids: HashSet<String> = HashSet::new();
    ids.extend(agent_by_id.keys().map(|s| s.to_string()));
    ids.extend(transcript_by_id.keys().map(|s| s.to_string()));

    let mut sessions: Vec<Session> = ids
        .into_iter()
        .map(|id| {
            let agent = agent_by_id.get(id.as_str()).copied();
            let transcript = transcript_by_id.get(id.as_str()).copied();
            let running = agent.is_some();
            let owned_flag = owned.contains(&id);
            // Only daemon *background* jobs can be PTY-attached (`claude attach`);
            // interactive foreign sessions (VS Code, CLI) cannot be injected.
            let is_background = agent.and_then(|a| a.kind.as_deref()) == Some("background");

            let cwd = agent
                .and_then(|a| a.cwd.clone())
                .or_else(|| transcript.and_then(|t| t.cwd.clone()))
                .unwrap_or_default();

            let last_activity = transcript.and_then(|t| t.last_activity);
            // Live (owned/sidecar) pending wins, but only while the session
            // can still consume an answer (running, or owned — our own
            // headless children may be absent from the agents roster).
            // Otherwise a running FOREIGN session whose transcript ends on an
            // unanswered question surfaces that; owned transcripts are skipped
            // (their prompts are only trustworthy from the live map — the
            // trailing tool_use looks unanswered until the SDK appends the
            // result and would flap the just-answered card back open).
            let pending_input = pending
                .get(&id)
                .cloned()
                .filter(|_| running || owned_flag)
                .or_else(|| {
                    if running && !owned_flag {
                        transcript.and_then(|t| t.pending.clone())
                    } else {
                        None
                    }
                });

            // Derive state: explicit state.json > pending > running+recency > rest.
            let state = if let Some(s) = states.get(&id).and_then(map_explicit_state) {
                s
            } else if pending_input.is_some() {
                SessionState::NeedsInput
            } else if running {
                match last_activity {
                    Some(ts) if now_ms - ts <= WORKING_WINDOW_MS => SessionState::Working,
                    _ => SessionState::Idle,
                }
            } else if transcript.is_some() {
                SessionState::Completed
            } else {
                SessionState::Unknown
            };

            Session {
                id: id.clone(),
                project_name: project_name(&cwd),
                cwd,
                surface: transcript.map(|t| t.surface).unwrap_or(Surface::Unknown),
                owned: owned_flag,
                state,
                model: transcript.and_then(|t| t.model.clone()),
                title: transcript.and_then(|t| t.title.clone()),
                started_at: agent
                    .and_then(|a| a.started_at)
                    .or_else(|| transcript.and_then(|t| t.started_at)),
                last_activity,
                pid: agent.and_then(|a| a.pid),
                kind: agent.and_then(|a| a.kind.clone()),
                git_branch: transcript.and_then(|t| t.git_branch.clone()),
                running,
                message_count: transcript.map(|t| t.message_count).unwrap_or(0),
                usage: transcript.map(|t| t.usage.clone()).unwrap_or_default(),
                pending: pending_input,
                tasks: transcript.map(|t| t.tasks.clone()).unwrap_or_default(),
                // Owned sessions are driven over stdin; running foreign *background*
                // jobs can be driven via PTY injection when it is enabled.
                can_inject: owned_flag || (foreign_injection && running && is_background),
            }
        })
        .collect();

    // Include freshly-spawned/resumed owned sessions that have no agent entry or
    // transcript yet, so they appear in the dashboard immediately.
    let present: HashSet<String> = sessions.iter().map(|s| s.id.clone()).collect();
    for meta in owned_live {
        if present.contains(&meta.id) {
            continue;
        }
        let pending_input = pending.get(&meta.id).cloned();
        sessions.push(Session {
            id: meta.id.clone(),
            project_name: project_name(&meta.cwd),
            cwd: meta.cwd.clone(),
            surface: Surface::Unknown,
            owned: true,
            state: if pending_input.is_some() {
                SessionState::NeedsInput
            } else {
                SessionState::Working
            },
            model: None,
            title: None,
            started_at: Some(meta.started_at),
            last_activity: Some(meta.started_at),
            pid: None,
            kind: None,
            git_branch: None,
            running: true,
            message_count: 0,
            usage: UsageSummary::default(),
            pending: pending_input,
            tasks: Vec::new(),
            can_inject: true,
        });
    }

    // Most recently active first; ties broken by id for determinism.
    sessions.sort_by(|a, b| {
        b.last_activity
            .cmp(&a.last_activity)
            .then_with(|| a.id.cmp(&b.id))
    });
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::schema::parse_transcript;

    fn agent(id: &str, pid: u32, cwd: &str) -> AgentEntry {
        agent_kind(id, pid, cwd, "interactive")
    }

    fn agent_kind(id: &str, pid: u32, cwd: &str, kind: &str) -> AgentEntry {
        AgentEntry {
            pid: Some(pid),
            cwd: Some(cwd.to_string()),
            kind: Some(kind.to_string()),
            started_at: Some(1000),
            session_id: Some(id.to_string()),
            ..Default::default()
        }
    }

    fn inputs<'a>(
        agents: &'a [AgentEntry],
        transcripts: &'a [TranscriptSummary],
        owned: &'a HashSet<String>,
        pending: &'a HashMap<String, PendingInput>,
        states: &'a HashMap<String, StateJson>,
        now: i64,
    ) -> RegistryInputs<'a> {
        RegistryInputs {
            agents,
            transcripts,
            owned,
            pending,
            states,
            now_ms: now,
            foreign_injection: false,
            owned_live: &[],
        }
    }

    #[test]
    fn surface_mapping() {
        assert_eq!(Surface::from_entrypoint("cli"), Surface::Cli);
        assert_eq!(Surface::from_entrypoint("claude-vscode"), Surface::VsCode);
        assert_eq!(Surface::from_entrypoint("jetbrains"), Surface::JetBrains);
        assert_eq!(Surface::from_entrypoint("claude-desktop"), Surface::Desktop);
        assert_eq!(Surface::from_entrypoint("weird"), Surface::Unknown);
    }

    #[test]
    fn summarize_extracts_model_usage_title_and_activity() {
        let blob = r#"{"type":"user","timestamp":"2026-06-23T08:00:00.000Z","cwd":"/x/app","gitBranch":"main","entrypoint":"claude-vscode","message":{"role":"user","content":"hi"}}
{"type":"ai-title","aiTitle":"My Session"}
{"type":"assistant","timestamp":"2026-06-23T08:00:05.000Z","message":{"role":"assistant","model":"claude-opus","content":[{"type":"text","text":"yo"}],"usage":{"input_tokens":10,"output_tokens":4,"cache_read_input_tokens":100}}}"#;
        let events = parse_transcript(blob);
        let s = summarize_transcript("sess-1", &events);
        assert_eq!(s.model.as_deref(), Some("claude-opus"));
        assert_eq!(s.title.as_deref(), Some("My Session"));
        assert_eq!(s.cwd.as_deref(), Some("/x/app"));
        assert_eq!(s.git_branch.as_deref(), Some("main"));
        assert_eq!(s.surface, Surface::VsCode);
        assert_eq!(s.message_count, 2);
        assert_eq!(s.usage.input_tokens, 10);
        assert_eq!(s.usage.total_tokens, 114);
        assert_eq!(
            s.started_at,
            Some(parse_ts_ms("2026-06-23T08:00:00.000Z").unwrap())
        );
        assert_eq!(
            s.last_activity,
            Some(parse_ts_ms("2026-06-23T08:00:05.000Z").unwrap())
        );
    }

    #[test]
    fn running_recent_is_working_old_is_idle() {
        let now = 1_000_000;
        let agents = vec![agent("a", 1, "/x/app")];
        let recent = TranscriptSummary {
            id: "a".into(),
            cwd: Some("/x/app".into()),
            last_activity: Some(now - 5_000),
            ..Default::default()
        };
        let owned = HashSet::new();
        let pending = HashMap::new();
        let states = HashMap::new();

        let r = build_registry(inputs(&agents, &[recent], &owned, &pending, &states, now));
        assert_eq!(r[0].state, SessionState::Working);
        assert!(r[0].running);
        assert!(!r[0].owned);

        let stale = TranscriptSummary {
            id: "a".into(),
            last_activity: Some(now - 60_000),
            ..Default::default()
        };
        let r = build_registry(inputs(&agents, &[stale], &owned, &pending, &states, now));
        assert_eq!(r[0].state, SessionState::Idle);
    }

    #[test]
    fn not_running_with_transcript_is_completed() {
        let now = 1_000_000;
        let t = TranscriptSummary {
            id: "gone".into(),
            last_activity: Some(now - 5_000),
            ..Default::default()
        };
        let (owned, pending, states) = (HashSet::new(), HashMap::new(), HashMap::new());
        let r = build_registry(inputs(&[], &[t], &owned, &pending, &states, now));
        assert_eq!(r[0].state, SessionState::Completed);
        assert!(!r[0].running);
    }

    #[test]
    fn pending_forces_needs_input_only_when_no_explicit_state() {
        let now = 1_000_000;
        let agents = vec![agent("a", 1, "/x/app")];
        let t = TranscriptSummary {
            id: "a".into(),
            last_activity: Some(now - 1_000),
            ..Default::default()
        };
        let owned: HashSet<String> = ["a".to_string()].into_iter().collect();
        let mut pending = HashMap::new();
        pending.insert(
            "a".to_string(),
            PendingInput {
                kind: PendingKind::Permission,
                tool: Some("Bash".into()),
                prompt: Some("run ls?".into()),
                header: None,
                options: Vec::new(),
                multi_select: false,
                detail: Some("ls -la".into()),
                request_id: Some("req1".into()),
                answerable: true,
                dangerous: false,
            },
        );
        let states = HashMap::new();
        let r = build_registry(inputs(&agents, &[t], &owned, &pending, &states, now));
        assert_eq!(r[0].state, SessionState::NeedsInput);
        assert!(r[0].owned);
        assert!(r[0].can_inject);
        assert!(r[0].pending.is_some());
    }

    #[test]
    fn explicit_state_json_overrides_derivation() {
        let now = 1_000_000;
        let agents = vec![agent("a", 1, "/x/app")];
        let t = TranscriptSummary {
            id: "a".into(),
            last_activity: Some(now - 1_000),
            ..Default::default()
        };
        let (owned, pending) = (HashSet::new(), HashMap::new());
        let mut states = HashMap::new();
        states.insert(
            "a".to_string(),
            StateJson {
                state: Some("failed".into()),
                ..Default::default()
            },
        );
        let r = build_registry(inputs(&agents, &[t], &owned, &pending, &states, now));
        assert_eq!(r[0].state, SessionState::Failed);
    }

    #[test]
    fn foreign_sessions_cannot_inject_by_default() {
        let now = 1_000_000;
        let agents = vec![agent("foreign", 9, "/x/app")];
        let (owned, pending, states) = (HashSet::new(), HashMap::new(), HashMap::new());
        let r = build_registry(inputs(&agents, &[], &owned, &pending, &states, now));
        assert!(!r[0].owned);
        assert!(!r[0].can_inject);
    }

    #[test]
    fn running_background_foreign_can_inject_when_enabled() {
        let now = 1_000_000;
        let agents = vec![agent_kind("foreign", 9, "/x/app", "background")];
        let (owned, pending, states) = (HashSet::new(), HashMap::new(), HashMap::new());
        let mut input = inputs(&agents, &[], &owned, &pending, &states, now);
        input.foreign_injection = true;
        let r = build_registry(input);
        assert!(!r[0].owned);
        assert!(r[0].running);
        assert!(
            r[0].can_inject,
            "running foreign background job should be injectable"
        );
    }

    #[test]
    fn owned_live_session_is_listed_before_any_transcript() {
        let now = 1_000_000;
        let (owned, pending, states) = (HashSet::new(), HashMap::new(), HashMap::new());
        let live = [OwnedSessionMeta {
            id: "fresh-owned".into(),
            cwd: "/x/app".into(),
            started_at: now - 500,
        }];
        let mut input = inputs(&[], &[], &owned, &pending, &states, now);
        input.owned_live = &live;
        let r = build_registry(input);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "fresh-owned");
        assert!(r[0].owned);
        assert!(r[0].running);
        assert!(r[0].can_inject);
        assert_eq!(r[0].project_name, "app");
    }

    #[test]
    fn interactive_foreign_cannot_inject_even_when_enabled() {
        let now = 1_000_000;
        // Interactive (e.g. VS Code) sessions can't be PTY-attached.
        let agents = vec![agent_kind("vscode", 9, "/x/app", "interactive")];
        let (owned, pending, states) = (HashSet::new(), HashMap::new(), HashMap::new());
        let mut input = inputs(&agents, &[], &owned, &pending, &states, now);
        input.foreign_injection = true;
        let r = build_registry(input);
        assert!(r[0].running);
        assert!(
            !r[0].can_inject,
            "interactive foreign session is not injectable"
        );
    }

    #[test]
    fn completed_foreign_cannot_inject_even_when_enabled() {
        let now = 1_000_000;
        // No agent entry => not running; only a transcript.
        let t = TranscriptSummary {
            id: "gone".into(),
            last_activity: Some(now - 5_000),
            ..Default::default()
        };
        let (owned, pending, states) = (HashSet::new(), HashMap::new(), HashMap::new());
        let transcripts = [t];
        let mut input = inputs(&[], &transcripts, &owned, &pending, &states, now);
        input.foreign_injection = true;
        let r = build_registry(input);
        assert!(!r[0].running);
        assert!(!r[0].can_inject, "a non-running session can't be attached");
    }

    #[test]
    fn sorts_most_recent_first() {
        let now = 1_000_000;
        let t1 = TranscriptSummary {
            id: "old".into(),
            last_activity: Some(100),
            ..Default::default()
        };
        let t2 = TranscriptSummary {
            id: "new".into(),
            last_activity: Some(900),
            ..Default::default()
        };
        let (owned, pending, states) = (HashSet::new(), HashMap::new(), HashMap::new());
        let r = build_registry(inputs(&[], &[t1, t2], &owned, &pending, &states, now));
        assert_eq!(r[0].id, "new");
        assert_eq!(r[1].id, "old");
    }

    #[test]
    fn background_task_lifecycle_from_transcript() {
        // launch (assistant tool_use) → async-launch result (user event with
        // toolUseResult) → completion notification (origin task-notification).
        let blob = r#"{"type":"assistant","timestamp":"2026-07-13T08:00:00.000Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"watch.sh","description":"Watch the CI run","run_in_background":true}}]}}
{"type":"user","timestamp":"2026-07-13T08:00:01.000Z","toolUseResult":{"stdout":"","backgroundTaskId":"bjrfky"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"Command running in background with ID: bjrfky."}]}}
{"type":"assistant","timestamp":"2026-07-13T08:00:02.000Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"tu2","name":"Workflow","input":{"script":"export const meta = {}"}}]}}
{"type":"user","timestamp":"2026-07-13T08:00:03.000Z","toolUseResult":{"status":"async_launched","taskId":"wky2ru","workflowName":"review-changes"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu2","content":"Workflow launched. Task ID: wky2ru"}]}}
{"type":"user","timestamp":"2026-07-13T08:05:00.000Z","origin":{"kind":"task-notification"},"message":{"role":"user","content":"<task-notification>\n<task-id>bjrfky</task-id>\n<status>completed</status>\n<summary>Background command finished</summary>\n</task-notification>"}}"#;
        let s = summarize_transcript("t", &parse_transcript(blob));
        assert_eq!(s.tasks.len(), 2);
        let bash = s.tasks.iter().find(|t| t.id == "bjrfky").unwrap();
        assert_eq!(bash.kind, "bash");
        assert_eq!(bash.label, "Watch the CI run");
        assert_eq!(bash.status, "completed");
        assert!(bash.ended_at.is_some());
        let wf = s.tasks.iter().find(|t| t.id == "wky2ru").unwrap();
        assert_eq!(wf.kind, "workflow");
        assert_eq!(wf.label, "review-changes");
        assert_eq!(wf.status, "running");
        assert!(wf.ended_at.is_none());
    }

    #[test]
    fn notification_does_not_clear_pending_question_and_orphan_is_recorded() {
        let blob = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"q1","name":"AskUserQuestion","input":{"questions":[{"question":"Pick?","options":["a"]}]}}]}}
{"type":"user","origin":{"kind":"task-notification"},"timestamp":"2026-07-13T08:06:00.000Z","message":{"role":"user","content":"<task-notification>\n<task-id>a12345</task-id>\n<status>failed</status>\n<summary>Agent \"x\" failed</summary>\n</task-notification>"}}"#;
        let s = summarize_transcript("t", &parse_transcript(blob));
        assert!(
            s.pending.is_some(),
            "notification must not answer the question"
        );
        let orphan = s.tasks.iter().find(|t| t.id == "a12345").unwrap();
        assert_eq!(orphan.kind, "agent");
        assert_eq!(orphan.status, "failed");
    }

    #[test]
    fn synchronous_agent_run_is_not_a_background_task() {
        // Sync Agent results also carry an agentId — without async gating the
        // task would be pinned "running" forever (no notification ever comes).
        let blob = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tu1","name":"Agent","input":{"description":"Explore code","prompt":"look around"}}]}}
{"type":"user","toolUseResult":{"status":"completed","agentId":"a9f0","content":"found it","usage":{}},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"found it"}]}}"#;
        let s = summarize_transcript("t", &parse_transcript(blob));
        assert!(s.tasks.is_empty(), "sync agent must not be a task");

        // The async form (isAsync/async_launched) IS a task.
        let blob = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tu2","name":"Agent","input":{"description":"Map sources"}}]}}
{"type":"user","timestamp":"2026-07-14T08:00:00.000Z","toolUseResult":{"status":"async_launched","isAsync":true,"agentId":"a2e5"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu2","content":"Async agent launched"}]}}"#;
        let s = summarize_transcript("t", &parse_transcript(blob));
        assert_eq!(s.tasks.len(), 1);
        assert_eq!(s.tasks[0].id, "a2e5");
        assert_eq!(s.tasks[0].status, "running");
    }

    #[test]
    fn foreground_bash_is_not_a_task() {
        let blob = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","toolUseResult":{"stdout":"ok"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"ok"}]}}"#;
        let s = summarize_transcript("t", &parse_transcript(blob));
        assert!(s.tasks.is_empty());
    }

    #[test]
    fn trailing_ask_user_question_becomes_pending() {
        let blob = r#"{"type":"user","timestamp":"2026-06-23T08:00:00.000Z","message":{"role":"user","content":"hi"}}
{"type":"assistant","timestamp":"2026-06-23T08:00:05.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Before I start:"},{"type":"tool_use","id":"tu1","name":"AskUserQuestion","input":{"questions":[{"question":"Which auth method?","header":"Auth","multiSelect":true,"options":[{"label":"OAuth","description":"Standards-based"},{"label":"API key"}]}]}}]}}"#;
        let s = summarize_transcript("q", &parse_transcript(blob));
        let p = s.pending.expect("question should be pending");
        assert_eq!(p.kind, PendingKind::Question);
        assert_eq!(p.prompt.as_deref(), Some("Which auth method?"));
        assert_eq!(p.header.as_deref(), Some("Auth"));
        assert!(p.multi_select);
        assert!(!p.answerable);
        assert_eq!(p.options.len(), 2);
        assert_eq!(p.options[0].label, "OAuth");
        assert_eq!(p.options[0].description.as_deref(), Some("Standards-based"));
        assert_eq!(p.options[1].description, None);
    }

    #[test]
    fn answered_question_clears_pending() {
        let blob = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tu1","name":"AskUserQuestion","input":{"questions":[{"question":"Pick one","options":["a","b"]}]}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"a"}]}}"#;
        let s = summarize_transcript("q", &parse_transcript(blob));
        assert!(s.pending.is_none());
    }

    #[test]
    fn sidecar_ask_user_and_plan_approval_detected() {
        let ask = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"mcp__mother-claude__ask_user","input":{"question":"Deploy now?","options":["Yes","No"]}}]}}"#;
        let s = summarize_transcript("q", &parse_transcript(ask));
        let p = s.pending.expect("ask_user should be pending");
        assert_eq!(p.kind, PendingKind::Question);
        assert_eq!(p.prompt.as_deref(), Some("Deploy now?"));
        assert_eq!(p.options.len(), 2);

        let plan = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"ExitPlanMode","input":{"plan":"1. Do X\n2. Do Y"}}]}}"#;
        let s = summarize_transcript("q", &parse_transcript(plan));
        let p = s.pending.expect("plan approval should be pending");
        assert_eq!(p.kind, PendingKind::Permission);
        assert_eq!(p.tool.as_deref(), Some("ExitPlanMode"));
        assert_eq!(p.detail.as_deref(), Some("1. Do X\n2. Do Y"));
    }

    #[test]
    fn question_options_accept_strings_and_objects() {
        let p: PendingInput = serde_json::from_value(serde_json::json!({
            "kind": "question",
            "prompt": "Pick",
            "options": ["plain", {"label": "rich", "description": "with text"}],
        }))
        .unwrap();
        assert_eq!(p.options[0].label, "plain");
        assert_eq!(p.options[1].label, "rich");
        assert_eq!(p.options[1].description.as_deref(), Some("with text"));
    }

    #[test]
    fn running_foreign_question_surfaces_needs_input_but_dead_session_does_not() {
        let now = 1_000_000;
        let t = TranscriptSummary {
            id: "f".into(),
            cwd: Some("/x/app".into()),
            last_activity: Some(now - 1_000),
            pending: Some(PendingInput {
                kind: PendingKind::Question,
                tool: Some("AskUserQuestion".into()),
                prompt: Some("Which one?".into()),
                header: None,
                options: Vec::new(),
                multi_select: false,
                detail: None,
                request_id: None,
                answerable: false,
                dangerous: false,
            }),
            ..Default::default()
        };
        let (owned, pending, states) = (HashSet::new(), HashMap::new(), HashMap::new());

        // Running foreign session → the transcript question surfaces.
        let agents = vec![agent("f", 7, "/x/app")];
        let r = build_registry(inputs(
            &agents,
            std::slice::from_ref(&t),
            &owned,
            &pending,
            &states,
            now,
        ));
        assert_eq!(r[0].state, SessionState::NeedsInput);
        assert!(r[0].pending.is_some());

        // Same transcript, process gone → no pending, completed.
        let r = build_registry(inputs(
            &[],
            std::slice::from_ref(&t),
            &owned,
            &pending,
            &states,
            now,
        ));
        assert_eq!(r[0].state, SessionState::Completed);
        assert!(r[0].pending.is_none());
    }
}
