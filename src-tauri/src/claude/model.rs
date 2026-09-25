//! The dashboard API contract (design §4): every type the REST API and the
//! WebSocket serve, serialized as camelCase JSON. The TypeScript mirror lives
//! in `src/app/core/models.ts`.
//!
//! These are *output* types built by the data layer; they are not parsed from
//! `~/.claude` (that is `schema.rs`). All of them implement `Default` so a
//! producer can fill what it knows and leave the rest empty, and all of them
//! also `Deserialize` (tolerantly) so tests and clients can round-trip them.
//!
//! Owner: foundation. Extending a type with a new OPTIONAL field is fine for
//! any agent; renaming or removing a field is a contract change.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ───────────────────────────── 4.1 SessionSummary ─────────────────────────────

/// Coarse lifecycle state shown as the status pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionStatus {
    Working,
    Waiting,
    #[default]
    Idle,
    Starting,
    Ended,
    Failed,
    Stopped,
}

/// Where the session runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    Vscode,
    Terminal,
    Tmux,
    Desktop,
    Jetbrains,
    Background,
    MotherClaude,
    Sdk,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivityKind {
    Thinking,
    Tool,
    Waiting,
    Delegating,
    Background,
    Compacting,
    Retrying,
    #[default]
    Idle,
    Starting,
    Ended,
}

/// "What is it doing right now" (design §3.5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Activity {
    pub kind: ActivityKind,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Epoch ms the current activity started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
}

impl Activity {
    pub fn new(kind: ActivityKind, label: impl Into<String>) -> Self {
        Self {
            kind,
            label: label.into(),
            ..Default::default()
        }
    }
}

/// Token counts (de-duplicated by message id in the v2 index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContextFill {
    pub used: u64,
    pub window: u64,
    /// 0–100.
    pub pct: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TodoSummary {
    pub total: u32,
    pub done: u32,
    pub in_progress: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentCounts {
    pub running: u32,
    pub total: u32,
    pub workflows_running: u32,
    pub background_running: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ControlChannel {
    Owned,
    Hooks,
    Tmux,
    Pty,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageCap {
    Direct,
    /// Delivered through a hook or tmux.
    Relay,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InterruptCap {
    Immediate,
    NextTool,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SetModeCap {
    Any,
    OnPrompt,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LifecycleCaps {
    pub stop: bool,
    pub respawn: bool,
    pub rm: bool,
}

/// What the dashboard can do with a session (design §4.1, computed in
/// `claude::caps`). The UI must be honest about these.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ControlCaps {
    pub channel: ControlChannel,
    pub message: MessageCap,
    pub approve: bool,
    pub answer: bool,
    pub interrupt: InterruptCap,
    pub set_model: bool,
    pub set_mode: SetModeCap,
    pub set_effort: bool,
    pub slash_commands: bool,
    pub attach_images: bool,
    pub stop_task: bool,
    pub fork: bool,
    pub continue_owned: bool,
    pub lifecycle: LifecycleCaps,
    /// Hook events were seen from this session recently.
    pub hooks_active: bool,
    /// Human-readable reasons/limits for the UI.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelInfo {
    /// Model id as the CLI reports it (e.g. `claude-opus-4-8`).
    pub id: String,
    /// Display label (e.g. `Opus 4.8`).
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PrLink {
    pub number: u64,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LastError {
    pub kind: String,
    pub text: String,
    pub at: i64,
}

/// One row of the session list (design §4.1).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionSummary {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub project_name: String,
    pub cwd: String,
    pub surface: Surface,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    pub owned: bool,
    pub live: bool,
    pub status: SessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<String>,
    pub activity: Activity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_link: Option<PrLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Short background job id (for `claude stop|respawn|rm|logs|attach`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmux_pane: Option<String>,
    pub usage: Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_final: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextFill>,
    pub message_count: u64,
    /// ≤ 240 chars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_prompt: Option<String>,
    /// ≤ 240 chars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todos: Option<TodoSummary>,
    pub agents: AgentCounts,
    pub queued: u32,
    pub pending: Vec<PendingItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<LastError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_control: Option<bool>,
    pub control: ControlCaps,
}

impl SessionSummary {
    /// Whether the session is in the `active` scope: live, waiting on a human,
    /// or active within the last 24 h.
    pub fn in_active_scope(&self, now_ms: i64) -> bool {
        const DAY_MS: i64 = 24 * 60 * 60 * 1000;
        self.live
            || !self.pending.is_empty()
            || self
                .last_activity_at
                .or(self.started_at)
                .is_some_and(|t| now_ms - t < DAY_MS)
    }
}

// ─────────────────────────── 4.2 PendingItem & respond ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuestionOption {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Question {
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

/// One "always allow …" option offered with a permission prompt.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PermissionSuggestion {
    pub index: usize,
    pub label: String,
    /// `addRules` | `setMode` | `addDirectories` | … (the raw `type`).
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    /// Outlives this request (written to settings, bypass mode, new dirs):
    /// dangerous ⇒ local or allowDangerous device only.
    pub standing: bool,
    /// The CLI's own `PermissionUpdate`, echoed back verbatim when chosen.
    pub raw: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PendingKind {
    #[default]
    Permission,
    Question,
    Plan,
    Elicitation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PendingSource {
    /// Path B stream-json control request.
    #[default]
    Owned,
    /// Foreign session via the hooks plugin (held PermissionRequest).
    Hook,
    /// The HTTP bridge's Agent SDK host.
    Bridge,
    /// The Path A sidecar.
    Sidecar,
    /// Seen on disk, not answerable here.
    Observed,
}

/// Something a human must act on (design §4.2).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PendingItem {
    pub id: String,
    pub session_id: String,
    pub kind: PendingKind,
    pub source: PendingSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub questions: Option<Vec<Question>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub suggestions: Vec<PermissionSuggestion>,
    pub dangerous: bool,
    pub answerable: bool,
    pub created_at: i64,
}

// ─────────────────────────────────── 4.3 Agents ───────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    #[default]
    Subagent,
    WorkflowAgent,
    Bash,
    Monitor,
    Teammate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentStatus {
    Running,
    Done,
    Failed,
    Stopped,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentInfo {
    pub id: String,
    pub kind: AgentKind,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub depth: u32,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    pub status: AgentStatus,
    pub background: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<Activity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_uses: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub can_stop: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowStatus {
    #[default]
    Running,
    Completed,
    Failed,
    Killed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowPhase {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowAgentCounts {
    pub total: u32,
    pub running: u32,
    pub done: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowRun {
    pub run_id: String,
    pub name: String,
    pub status: WorkflowStatus,
    pub phases: Vec<WorkflowPhase>,
    pub agents: WorkflowAgentCounts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Todo {
    pub content: String,
    pub status: TodoStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundTaskKind {
    #[default]
    Bash,
    Agent,
    Workflow,
    Monitor,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackgroundTask {
    pub id: String,
    pub kind: BackgroundTaskKind,
    pub label: String,
    /// `running` | `completed` | `failed` | `killed` | … (free text).
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    pub output_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueVia {
    #[default]
    Owned,
    HookBusy,
    HookIdle,
    Tmux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueStatus {
    #[default]
    Queued,
    Delivered,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueuedMessage {
    pub id: String,
    pub text: String,
    pub created_at: i64,
    pub via: QueueVia,
    pub status: QueueStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileChanged {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<u64>,
}

/// `GET /api/sessions/{id}`: the summary plus everything the detail view needs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionDetail {
    #[serde(flatten)]
    pub summary: SessionSummary,
    pub agents_detail: Vec<AgentInfo>,
    pub workflows: Vec<WorkflowRun>,
    pub tasks: Vec<BackgroundTask>,
    pub todos_detail: Vec<Todo>,
    pub queued_messages: Vec<QueuedMessage>,
    pub files_changed: Vec<FileChanged>,
}

/// One session's running agents for `GET /api/agents` and the `fleet` frame.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FleetSession {
    pub session_id: String,
    pub name: String,
    pub agents: Vec<AgentInfo>,
}

// ────────────────────────────────── 4.4 UiEvent ───────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiRole {
    User,
    #[default]
    Assistant,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiKind {
    Prompt,
    #[default]
    Text,
    Thinking,
    Tool,
    Result,
    Image,
    Notice,
    Compact,
    Error,
    Task,
    Relayed,
    Todos,
    Question,
    Plan,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiTool {
    pub id: String,
    pub name: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiPatch {
    pub path: String,
    pub hunks: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiResult {
    pub tool_use_id: String,
    pub is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<UiPatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiImage {
    pub media_type: String,
    /// `/api/sessions/<id>/media/<uuid>/<blockIdx>` — never inline base64.
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiQuestion {
    pub questions: Vec<Question>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiPlan {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiTask {
    pub task_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// One normalised transcript entry (design §4.4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiEvent {
    /// Stable across reads (the record uuid plus block index), so clients can
    /// dedupe REST pages against live WS appends.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    pub role: UiRole,
    pub kind: UiKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<UiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<UiResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<UiImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todos: Option<Vec<Todo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<UiQuestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<UiPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<UiTask>,
}

/// `GET /api/sessions/{id}/transcript` page: newest-last.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptPage {
    pub events: Vec<UiEvent>,
    /// Cursor for the previous page (opaque byte offset), absent at the start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// Cursor for `sub{since}` / live appends (opaque byte offset of EOF).
    pub tail: String,
}

// ─────────────────────────────── 4.6 live frames ──────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StreamKind {
    #[default]
    Text,
    Thinking,
    Tool,
    Status,
}

/// An owned session's live delta (`stream` WS frame).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamFrame {
    pub kind: StreamKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoticeLevel {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

// ───────────────────────────────────── misc ───────────────────────────────────

/// `GET /api/me` and the `me` of the WS `hello` frame.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Me {
    /// `desktop` | `device` | `local-script`.
    pub principal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `loopback` | `lan` | `tunnel` | `bridge-loopback`.
    pub ingress: String,
    pub local: bool,
    /// Whether THIS request may perform dangerous actions.
    pub allow_dangerous: bool,
    /// The ingress used for policy: `tunnel` for anything proxied (B1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_ingress: Option<String>,
    /// Device platform as given at pairing (B1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// When this device was paired, epoch ms (B1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paired_at: Option<i64>,
    /// The device's LAN dangerous grant (B1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_dangerous_lan: Option<bool>,
    /// The device's internet dangerous grant (B1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_dangerous_internet: Option<bool>,
    /// The client address this server sees (non-local callers only) (B1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<String>,
}

/// `GET /api/sessions/{id}/commands` entry.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

/// `GET /api/projects` entry (spawn picker).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectInfo {
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<i64>,
}

/// An image attached to a message or spawn (`images?: [{mediaType, data}]`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageInput {
    /// `image/png` | `image/jpeg` | `image/gif` | `image/webp`.
    pub media_type: String,
    /// Base64 (no data: prefix).
    pub data: String,
}

/// How `POST /sessions/{id}/message` delivered the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Delivery {
    /// Written to an owned session now.
    #[default]
    Sent,
    /// Accepted but waiting (owned session busy / hook relay pending).
    Queued,
    /// Handed to a foreign session through tmux or a hook.
    Relayed,
}

/// Response of `POST /sessions/{id}/message`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageReceipt {
    pub delivery: Delivery,
    pub message_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detail_flattens_summary_and_renames_detail_fields() {
        let d = SessionDetail {
            summary: SessionSummary {
                id: "s1".into(),
                ..Default::default()
            },
            agents_detail: vec![AgentInfo::default()],
            todos_detail: vec![Todo::default()],
            ..Default::default()
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["id"], "s1");
        assert!(v["agentsDetail"].is_array());
        assert!(v["todosDetail"].is_array());
        assert!(v["queuedMessages"].is_array());
        assert!(v["filesChanged"].is_array());
        assert!(v.get("agents_detail").is_none());
        // Summary fields keep camelCase through the flatten.
        assert!(v.get("projectName").is_some());
        assert!(v.get("messageCount").is_some());
    }

    #[test]
    fn enums_use_the_contract_spellings() {
        assert_eq!(json!(Surface::MotherClaude), json!("mother-claude"));
        assert_eq!(json!(Surface::Vscode), json!("vscode"));
        assert_eq!(json!(InterruptCap::NextTool), json!("next-tool"));
        assert_eq!(json!(SetModeCap::OnPrompt), json!("on-prompt"));
        assert_eq!(json!(AgentKind::WorkflowAgent), json!("workflow-agent"));
        assert_eq!(json!(TodoStatus::InProgress), json!("in_progress"));
        assert_eq!(json!(QueueVia::HookBusy), json!("hook-busy"));
        assert_eq!(json!(SessionStatus::Waiting), json!("waiting"));
    }

    #[test]
    fn tolerant_deserialize_and_renames() {
        let s: SessionSummary =
            serde_json::from_value(json!({ "id": "x", "unknown": 1, "lastActivityAt": 5 }))
                .unwrap();
        assert_eq!(s.last_activity_at, Some(5));
        let a: AgentInfo = serde_json::from_value(json!({ "type": "Explore" })).unwrap();
        assert_eq!(a.agent_type.as_deref(), Some("Explore"));
        let img = serde_json::to_value(UiImage {
            media_type: "image/png".into(),
            reference: "/r".into(),
        })
        .unwrap();
        assert_eq!(img["ref"], "/r");
        assert_eq!(img["mediaType"], "image/png");
    }

    #[test]
    fn active_scope() {
        let now = 10 * 24 * 60 * 60 * 1000;
        let mut s = SessionSummary::default();
        assert!(!s.in_active_scope(now));
        s.last_activity_at = Some(now - 1000);
        assert!(s.in_active_scope(now));
        s.last_activity_at = Some(now - 25 * 60 * 60 * 1000);
        assert!(!s.in_active_scope(now));
        s.live = true;
        assert!(s.in_active_scope(now));
    }
}
