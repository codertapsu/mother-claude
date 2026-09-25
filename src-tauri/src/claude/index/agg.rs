//! Per-file aggregates of a transcript (design §3.2 / §3.3, amended §10 B2)
//! and the streaming scanner that folds lines into them.
//!
//! One [`FileIndex`] per `*.jsonl` (main transcript or subagent file). It
//! remembers the byte offset just past the last complete line, so the next
//! scan resumes there and unchanged bytes are never read again. Aggregates are
//! bounded: every list is capped, text is clipped, and usage is folded into
//! totals (only the last few API messages stay "open" for de-duplication).
//!
//! **Usage de-duplication.** Every content block of one API message is its own
//! record carrying the message's `usage`; subagent files write blocks one by
//! one with *partial* usage. The rule verified against `cost-state` is: count
//! each `message.id` once, with the LAST record's usage. Records of a message
//! are contiguous, so a small window of open messages (keyed by id, newest
//! wins) is exact; older messages are folded into totals.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::loose::{
    clip, parse_line, CapStr, Keys, LBool, LStr, LineCount, Obj, PatchCount, Present, Seq,
    SmallValue, LF64, LU64,
};
use crate::claude::model::{Todo, TodoStatus};
use crate::claude::pricing::{self, UsageDetail};

// ───────────────────────────── record shapes (input) ─────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Rec {
    #[serde(rename = "type")]
    pub ty: LStr,
    pub subtype: LStr,
    pub uuid: LStr,
    pub timestamp: LStr,
    #[serde(rename = "isSidechain")]
    pub is_sidechain: LBool,
    #[serde(rename = "isMeta")]
    pub is_meta: LBool,
    pub cwd: LStr,
    #[serde(rename = "gitBranch")]
    pub git_branch: LStr,
    pub version: LStr,
    pub entrypoint: LStr,
    #[serde(rename = "permissionMode")]
    pub permission_mode: LStr,
    pub effort: LStr,
    pub message: Obj<Msg>,
    #[serde(rename = "toolUseResult")]
    pub tool_use_result: Obj<Tur>,
    pub attachment: Obj<Att>,
    pub origin: Obj<Origin>,
    #[serde(rename = "customTitle")]
    pub custom_title: CapStr<512>,
    #[serde(rename = "aiTitle")]
    pub ai_title: CapStr<512>,
    #[serde(rename = "agentName")]
    pub agent_name: CapStr<512>,
    #[serde(rename = "lastPrompt")]
    pub last_prompt: CapStr<1024>,
    #[serde(rename = "prNumber")]
    pub pr_number: LU64,
    #[serde(rename = "prUrl")]
    pub pr_url: CapStr<1024>,
    #[serde(rename = "prRepository")]
    pub pr_repository: CapStr<512>,
    pub operation: LStr,
    /// `queue-operation` / `system` string content.
    pub content: CapStr<8192>,
    #[serde(rename = "totalCostUSD")]
    pub total_cost_usd: LF64,
    #[serde(rename = "isApiErrorMessage")]
    pub is_api_error: LBool,
    pub error: ErrField,
    #[serde(rename = "quotaLimits")]
    pub quota_limits: Present,
    #[serde(rename = "retryAttempt")]
    pub retry_attempt: LU64,
    #[serde(rename = "maxRetries")]
    pub max_retries: LU64,
    #[serde(rename = "trackingPath")]
    pub tracking_path: CapStr<4096>,
    pub snapshot: Obj<SnapshotRec>,
    #[serde(rename = "bridgeSessionId")]
    pub bridge_session_id: LStr,
    #[serde(rename = "agentId")]
    pub agent_id: LStr,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Msg {
    pub id: LStr,
    pub model: LStr,
    pub role: LStr,
    pub stop_reason: LStr,
    pub usage: Obj<UsageRec>,
    pub content: Content,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UsageRec {
    pub input_tokens: LU64,
    pub output_tokens: LU64,
    pub cache_read_input_tokens: LU64,
    pub cache_creation_input_tokens: LU64,
    pub cache_creation: Obj<CacheCreation>,
    pub server_tool_use: Obj<ServerToolUse>,
    pub inference_geo: LStr,
    pub speed: LStr,
    pub output_tokens_details: Obj<OutputDetails>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CacheCreation {
    pub ephemeral_1h_input_tokens: LU64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ServerToolUse {
    pub web_search_requests: LU64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct OutputDetails {
    pub thinking_tokens: LU64,
}

/// `message.content`: a string or content blocks.
#[derive(Debug, Default)]
pub enum Content {
    #[default]
    None,
    Text(CapStr<8192>),
    Blocks(Vec<Block>),
}

impl<'de> Deserialize<'de> for Content {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, SeqAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Content;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("content")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Content, E> {
                let kept = clip(v, 8192);
                Ok(Content::Text(CapStr {
                    text: Some(kept.to_string()),
                    cut: kept.len() < v.len(),
                    len: v.len(),
                }))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Content, A::Error> {
                let mut out = Vec::new();
                while let Some(b) = seq.next_element::<Block>()? {
                    if out.len() < 512 {
                        out.push(b);
                    }
                }
                Ok(Content::Blocks(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Content, A::Error> {
                while map
                    .next_entry::<de::IgnoredAny, de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(Content::None)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Content, E> {
                Ok(Content::None)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Content, E> {
                Ok(Content::None)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Content, E> {
                Ok(Content::None)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Content, E> {
                Ok(Content::None)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Content, E> {
                Ok(Content::None)
            }
            fn visit_none<E: de::Error>(self) -> Result<Content, E> {
                Ok(Content::None)
            }
        }
        d.deserialize_any(V)
    }
}

/// One content block (index view: no tool_result bodies, no image data, no
/// thinking text).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Block {
    #[serde(rename = "type")]
    pub ty: LStr,
    pub id: LStr,
    pub name: LStr,
    pub text: CapStr<4096>,
    /// tool_use input; dropped when larger than 64 KB of JSON.
    pub input: SmallValue<65536>,
    pub tool_use_id: LStr,
    pub is_error: LBool,
}

/// `toolUseResult` (object form; string results are ignored).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Tur {
    pub status: LStr,
    #[serde(rename = "agentId")]
    pub agent_id: LStr,
    #[serde(rename = "agentType")]
    pub agent_type: LStr,
    #[serde(rename = "backgroundTaskId")]
    pub background_task_id: LStr,
    #[serde(rename = "taskId")]
    pub task_id: LStr,
    /// TaskStop / KillShell results name the task in snake case.
    #[serde(rename = "task_id")]
    pub task_id_snake: LStr,
    #[serde(rename = "runId")]
    pub run_id: LStr,
    #[serde(rename = "workflowName")]
    pub workflow_name: CapStr<512>,
    pub description: CapStr<512>,
    #[serde(rename = "filePath")]
    pub file_path: CapStr<4096>,
    #[serde(rename = "structuredPatch")]
    pub structured_patch: PatchCount,
    #[serde(rename = "type")]
    pub ty: LStr,
    /// Write: the created file's content (only its line count is kept).
    pub content: LineCount,
    #[serde(rename = "newTodos")]
    pub new_todos: Seq<TodoRec>,
    #[serde(rename = "outputFile")]
    pub output_file: CapStr<4096>,
    #[serde(rename = "totalTokens")]
    pub total_tokens: LU64,
    #[serde(rename = "isAsync")]
    pub is_async: LBool,
    #[serde(rename = "bashEditDiff")]
    pub bash_edit_diff: Obj<BashEditDiff>,
    pub persistent: LBool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct BashEditDiff {
    #[serde(rename = "changedFiles")]
    pub changed_files: Seq<ChangedFile>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ChangedFile {
    #[serde(rename = "filePath")]
    pub file_path: CapStr<4096>,
    pub hunks: PatchCount,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TodoRec {
    pub content: CapStr<1024>,
    pub status: LStr,
    #[serde(rename = "activeForm")]
    pub active_form: CapStr<1024>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Att {
    #[serde(rename = "type")]
    pub ty: LStr,
    pub identity: Obj<Identity>,
    #[serde(rename = "taskId")]
    pub task_id: LStr,
    #[serde(rename = "taskType")]
    pub task_type: LStr,
    pub description: CapStr<512>,
    pub status: LStr,
    #[serde(rename = "outputFilePath")]
    pub output_file_path: CapStr<4096>,
    pub shell: Obj<ShellRec>,
    /// `queued_command`: the queued prompt (task notifications).
    pub prompt: CapStr<8192>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Identity {
    #[serde(rename = "modelId")]
    pub model_id: LStr,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ShellRec {
    pub command: CapStr<512>,
    #[serde(rename = "toolUseId")]
    pub tool_use_id: LStr,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Origin {
    pub kind: LStr,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SnapshotRec {
    #[serde(rename = "trackedFileBackups")]
    pub tracked_file_backups: Keys,
}

/// `error`: a string (`"rate_limit"`) or `{message, formatted, status}`.
#[derive(Debug, Default)]
pub struct ErrField(pub Option<String>);

impl<'de> Deserialize<'de> for ErrField {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct E {
            formatted: CapStr<512>,
            message: CapStr<512>,
        }
        let v = Value::deserialize(d)?;
        Ok(ErrField(match v {
            Value::String(s) => Some(clip(&s, 512).to_string()),
            Value::Object(_) => serde_json::from_value::<E>(v)
                .ok()
                .and_then(|e| e.formatted.text.or(e.message.text)),
            _ => None,
        }))
    }
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

fn is_false(v: &bool) -> bool {
    !*v
}

pub fn parse_ts(ts: Option<&str>) -> Option<i64> {
    ts.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| d.timestamp_millis())
}

// ───────────────────────────── aggregates (output) ───────────────────────────

/// Token/cost totals of a set of API messages.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Totals {
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub input: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub output: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub cache_read: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub cache_write: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub thinking: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub web_search: u64,
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub cost: f64,
    /// Messages with a known price.
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub priced: u64,
    /// Messages whose model has no known price.
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub unpriced: u64,
    /// Messages in a non-standard speed tier (priced with the standard table).
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub nonstandard: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub messages: u64,
}

impl Totals {
    pub fn add(&mut self, o: &Totals) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.thinking += o.thinking;
        self.web_search += o.web_search;
        self.cost += o.cost;
        self.priced += o.priced;
        self.unpriced += o.unpriced;
        self.nonstandard += o.nonstandard;
        self.messages += o.messages;
    }

    fn of(model: &str, u: &UsageRec) -> Totals {
        let detail = UsageDetail {
            input: u.input_tokens.n(),
            output: u.output_tokens.n(),
            cache_read: u.cache_read_input_tokens.n(),
            cache_creation: u.cache_creation_input_tokens.n(),
            cache_1h: u
                .cache_creation
                .get()
                .map(|c| c.ephemeral_1h_input_tokens.n())
                .unwrap_or(0),
            web_search: u
                .server_tool_use
                .get()
                .map(|s| s.web_search_requests.n())
                .unwrap_or(0),
            us_geo: u.inference_geo.is("us"),
            standard_speed: u.speed.get().is_none_or(|s| s == "standard"),
        };
        let cost = pricing::message_cost(model, &detail);
        Totals {
            input: detail.input,
            output: detail.output,
            cache_read: detail.cache_read,
            cache_write: detail.cache_creation,
            thinking: u
                .output_tokens_details
                .get()
                .map(|o| o.thinking_tokens.n())
                .unwrap_or(0),
            web_search: detail.web_search,
            cost: cost.unwrap_or(0.0),
            priced: cost.is_some() as u64,
            unpriced: cost.is_none() as u64,
            nonstandard: (!detail.standard_speed) as u64,
            messages: 1,
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Totals::default()
    }

    /// All tokens (what the fleet view shows as "tokens").
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// An API message still open for de-duplication.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenMsg {
    pub id: String,
    pub totals: Totals,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    /// Seen after the last `cost-state` record.
    #[serde(skip_serializing_if = "is_false")]
    pub after_cost_state: bool,
}

/// Messages kept open for de-duplication (the records of one message are
/// contiguous, so a small window is exact).
const OPEN_WINDOW: usize = 4;
/// Cost-curve points kept per file.
const CURVE_POINTS: usize = 12;

/// De-duplicated usage of one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageAcc {
    pub folded: Totals,
    /// Folded messages written after the last `cost-state` record.
    #[serde(skip_serializing_if = "Totals::is_empty")]
    pub after: Totals,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<OpenMsg>,
    /// `(ts, cumulative folded cost)` samples, for "cost after T".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub curve: Vec<(i64, f64)>,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub curve_stride: u64,
}

impl UsageAcc {
    fn fold(&mut self, m: OpenMsg) {
        self.folded.add(&m.totals);
        if m.after_cost_state {
            self.after.add(&m.totals);
        }
        let stride = self.curve_stride.max(1);
        if let Some(ts) = m.ts {
            if self.folded.messages.is_multiple_of(stride) {
                // Micro-dollar precision keeps checkpoints small.
                self.curve
                    .push((ts, (self.folded.cost * 1e6).round() / 1e6));
                if self.curve.len() > CURVE_POINTS {
                    let kept: Vec<(i64, f64)> =
                        self.curve.iter().skip(1).step_by(2).copied().collect();
                    self.curve = kept;
                    self.curve_stride = stride * 2;
                }
            }
        }
    }

    fn record(&mut self, id: &str, totals: Totals, ts: Option<i64>, after_cost_state: bool) {
        if let Some(open) = self.open.iter_mut().find(|m| m.id == id) {
            open.totals = totals;
            open.ts = ts.or(open.ts);
            open.after_cost_state = after_cost_state;
            return;
        }
        self.open.push(OpenMsg {
            id: id.to_string(),
            totals,
            ts,
            after_cost_state,
        });
        while self.open.len() > OPEN_WINDOW {
            let m = self.open.remove(0);
            self.fold(m);
        }
    }

    /// Fold every open message but the newest (checkpoints: the newest may
    /// still grow when the file is appended after a restart).
    pub fn compact(&mut self) {
        while self.open.len() > 1 {
            let m = self.open.remove(0);
            self.fold(m);
        }
    }

    /// Fold everything open (a `cost-state` record closes the books).
    fn close_all(&mut self) {
        for m in std::mem::take(&mut self.open) {
            self.fold(m);
        }
    }

    pub fn total(&self) -> Totals {
        let mut t = self.folded.clone();
        for m in &self.open {
            t.add(&m.totals);
        }
        t
    }

    /// Usage written after the last `cost-state` record.
    pub fn after_cost_state(&self) -> Totals {
        let mut t = self.after.clone();
        for m in self.open.iter().filter(|m| m.after_cost_state) {
            t.add(&m.totals);
        }
        t
    }

    /// Priced cost of messages written after `ts` (approximated from the
    /// cost curve for folded messages; exact for open ones).
    pub fn cost_after(&self, ts: i64) -> f64 {
        let before = self
            .curve
            .iter()
            .rev()
            .find(|(t, _)| *t <= ts)
            .map(|(_, c)| *c)
            .unwrap_or(0.0);
        let folded_after = (self.folded.cost - before).max(0.0);
        folded_after
            + self
                .open
                .iter()
                .filter(|m| m.ts.is_none_or(|t| t > ts))
                .map(|m| m.totals.cost)
                .sum::<f64>()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PrLinkRec {
    pub number: u64,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CostStateRec {
    pub total: f64,
    /// Timestamp of the last timestamped record before the `cost-state` line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts_before: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContextRec {
    pub used: u64,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueuedPrompt {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    /// A task-notification (not a human prompt).
    #[serde(skip_serializing_if = "is_false")]
    pub notification: bool,
}

/// A tool_use without a tool_result yet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PendingTool {
    pub id: String,
    pub name: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    /// The input, when small (≤ 16 KB of JSON); AskUserQuestion / ExitPlanMode
    /// inputs are what the observed pending items are built from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

/// A background task (bash, agent, workflow, monitor).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BgTask {
    pub id: String,
    /// `bash` | `agent` | `workflow` | `monitor`.
    pub kind: String,
    pub label: String,
    /// `running` | `completed` | `failed` | `killed` | `stopped` | …
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// An `Agent`/`Task` tool call of this file (maps subagent metas to status).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentCall {
    pub tool_use_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub background: bool,
    /// The tool_result arrived (sync: finished; async: launched).
    #[serde(skip_serializing_if = "is_false")]
    pub result_seen: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub is_error: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub async_launched: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

/// A `Workflow` launch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowLaunch {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LastErrorRec {
    pub kind: String,
    pub text: String,
    pub at: i64,
}

/// The last meaningful record of the main chain (for "what is it doing").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkKind {
    #[default]
    None,
    /// A human (or task-notification) prompt.
    Prompt,
    ToolResult,
    /// An assistant message that ended on tool_use.
    ToolUse,
    /// An assistant message that ended the turn.
    EndTurn,
    /// Any other assistant record (partial, max_tokens…).
    Assistant,
    ApiError,
    Compact,
    /// `stop_hook_summary` / `turn_duration`: the turn is over.
    TurnEnd,
    /// `[Request interrupted by user]`.
    Interrupted,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mark {
    #[serde(skip_serializing_if = "MarkKind::is_none")]
    pub kind: MarkKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
    /// `(attempt, max)` for API errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<(u64, u64)>,
}

impl Mark {
    pub fn is_none(&self) -> bool {
        self.kind == MarkKind::None && self.ts.is_none()
    }
}

impl MarkKind {
    pub fn is_none(&self) -> bool {
        *self == MarkKind::None
    }
}

/// The last thing a (sub)agent did, for its activity line.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LastAct {
    /// `tool` | `thinking` | `text` | `result`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

const PENDING_CAP: usize = 32;
const TASK_CAP: usize = 200;
const CALL_CAP: usize = 256;
const QUEUE_CAP: usize = 20;
const FILES_CAP: usize = 500;
const WORKFLOW_CAP: usize = 256;
const TEXT_MAX: usize = 240;
const PENDING_INPUT_MAX: usize = 4 * 1024;
/// Only the newest pending tool uses keep their input.
const PENDING_WITH_INPUT: usize = 4;

/// Aggregates of one transcript file. Every field is bounded.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Aggregates {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_ts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ts: Option<i64>,
    pub usage: UsageAcc,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub prompts: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub tool_uses: u64,
    /// Latest `model` attachment `identity.modelId` (includes `[1m]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_attachment: Option<String>,
    /// Latest non-synthetic `message.model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// `(mode, ts)`: latest human-prompt `permissionMode`, overridden by a
    /// later `permission-mode` record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<(String, i64)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_link: Option<PrLinkRec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_state: Option<CostStateRec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextRec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todos: Option<Vec<Todo>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<QueuedPrompt>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pending_tools: Vec<PendingTool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<BgTask>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agent_calls: Vec<AgentCall>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub workflow_runs: Vec<WorkflowLaunch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<LastErrorRec>,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub compactions: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_compact_ts: Option<i64>,
    #[serde(skip_serializing_if = "is_false")]
    pub remote_control: bool,
    /// path → (added, removed).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub files_changed: BTreeMap<String, (u64, u64)>,
    #[serde(skip_serializing_if = "Mark::is_none")]
    pub last_mark: Mark,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_act: Option<LastAct>,
}

fn clip_chars(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Extract `<tag>…</tag>` from task-notification text.
pub fn xml_tag<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim())
}

/// A parsed `<task-notification>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskNote {
    pub task_id: String,
    pub tool_use_id: Option<String>,
    pub status: String,
    pub summary: Option<String>,
    pub output_file: Option<String>,
}

pub fn parse_task_notification(text: &str) -> Option<TaskNote> {
    if !text.contains("<task-notification>") {
        return None;
    }
    Some(TaskNote {
        task_id: xml_tag(text, "task-id")?.to_string(),
        tool_use_id: xml_tag(text, "tool-use-id").map(str::to_string),
        status: xml_tag(text, "status").unwrap_or("completed").to_string(),
        summary: xml_tag(text, "summary").map(|s| clip_chars(s, TEXT_MAX)),
        output_file: xml_tag(text, "output-file").map(str::to_string),
    })
}

fn todos_of(v: &Value) -> Option<Vec<Todo>> {
    let arr = v.get("todos")?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|t| {
                Some(Todo {
                    content: clip_chars(t.get("content")?.as_str()?, 512),
                    status: match t.get("status").and_then(Value::as_str) {
                        Some("completed") => TodoStatus::Completed,
                        Some("in_progress") => TodoStatus::InProgress,
                        _ => TodoStatus::Pending,
                    },
                    active_form: t
                        .get("activeForm")
                        .and_then(Value::as_str)
                        .map(|s| clip_chars(s, 512)),
                })
            })
            .take(200)
            .collect(),
    )
}

fn small_input(v: &Value) -> Option<Value> {
    let size = serde_json::to_vec(v).map(|b| b.len()).unwrap_or(usize::MAX);
    if size <= PENDING_INPUT_MAX {
        return Some(v.clone());
    }
    // Keep the scalar fields that identify the call (path, command, url…).
    let obj = v.as_object()?;
    let mut out = serde_json::Map::new();
    for (k, val) in obj {
        if let Some(s) = val.as_str() {
            out.insert(k.clone(), Value::String(clip(s, 1024).to_string()));
        } else if val.is_number() || val.is_boolean() {
            out.insert(k.clone(), val.clone());
        }
    }
    Some(Value::Object(out))
}

impl Aggregates {
    fn touch_ts(&mut self, ts: Option<i64>) {
        if let Some(t) = ts {
            self.first_ts = Some(self.first_ts.map_or(t, |f| f.min(t)));
            self.last_ts = Some(self.last_ts.map_or(t, |l| l.max(t)));
        }
    }

    fn upsert_task(&mut self, task: BgTask) {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == task.id) {
            if t.label.is_empty() {
                t.label = task.label;
            }
            if task.output_file.is_some() {
                t.output_file = task.output_file;
            }
            if task.tool_use_id.is_some() && t.tool_use_id.is_none() {
                t.tool_use_id = task.tool_use_id;
            }
            if task.run_id.is_some() {
                t.run_id = task.run_id;
            }
            return;
        }
        self.tasks.push(task);
        if self.tasks.len() > TASK_CAP {
            // Drop the oldest finished task first, else the oldest.
            let idx = self
                .tasks
                .iter()
                .position(|t| t.status != "running")
                .unwrap_or(0);
            self.tasks.remove(idx);
        }
    }

    fn file_changed(&mut self, path: &str, added: u64, removed: u64) {
        if path.is_empty() {
            return;
        }
        if let Some(e) = self.files_changed.get_mut(path) {
            e.0 += added;
            e.1 += removed;
        } else if self.files_changed.len() < FILES_CAP {
            self.files_changed
                .insert(path.to_string(), (added, removed));
        }
    }

    /// Fold one parsed record in.
    pub fn apply(&mut self, rec: Rec) {
        let ts = parse_ts(rec.timestamp.get());
        let ty = rec.ty.get().unwrap_or("").to_string();
        // Identity fields ride on most records.
        if let Some(c) = rec.cwd.get().filter(|c| !c.is_empty()) {
            if self.cwd.as_deref() != Some(c) {
                self.cwd = Some(c.to_string());
            }
        }
        if let Some(b) = rec.git_branch.get().filter(|b| !b.is_empty()) {
            if self.git_branch.as_deref() != Some(b) {
                self.git_branch = Some(b.to_string());
            }
        }
        if let Some(v) = rec.version.get() {
            if self.version.as_deref() != Some(v) {
                self.version = Some(v.to_string());
            }
        }
        if let Some(e) = rec.entrypoint.get() {
            if self.entrypoint.as_deref() != Some(e) {
                self.entrypoint = Some(e.to_string());
            }
        }
        match ty.as_str() {
            "assistant" => self.apply_assistant(rec, ts),
            "user" => self.apply_user(rec, ts),
            "attachment" => self.apply_attachment(rec, ts),
            "system" => self.apply_system(rec, ts),
            "last-prompt" => {
                if let Some(p) = rec.last_prompt.get().filter(|p| !p.trim().is_empty()) {
                    self.last_prompt = Some(clip_chars(p, TEXT_MAX));
                }
            }
            "custom-title" => {
                if let Some(t) = rec.custom_title.text {
                    self.custom_title = Some(clip_chars(&t, 200)).filter(|t| !t.is_empty());
                }
            }
            "ai-title" => {
                if let Some(t) = rec.ai_title.text {
                    self.ai_title = Some(clip_chars(&t, 200)).filter(|t| !t.is_empty());
                }
            }
            "agent-name" => {
                if let Some(t) = rec.agent_name.text {
                    self.agent_name = Some(clip_chars(&t, 200)).filter(|t| !t.is_empty());
                }
            }
            "permission-mode" => {
                if let Some(m) = rec.permission_mode.take() {
                    let at = self.last_ts.unwrap_or(0);
                    self.permission_mode = Some((m, at));
                }
            }
            "pr-link" => {
                if let (Some(n), Some(url)) = (rec.pr_number.0, rec.pr_url.text) {
                    self.pr_link = Some(PrLinkRec {
                        number: n,
                        url,
                        repo: rec.pr_repository.text,
                    });
                }
            }
            "bridge-session" => {
                if rec.bridge_session_id.get().is_some() {
                    self.remote_control = true;
                }
            }
            "cost-state" => {
                if let Some(total) = rec.total_cost_usd.0 {
                    self.usage.close_all();
                    self.usage.after = Totals::default();
                    self.cost_state = Some(CostStateRec {
                        total,
                        ts_before: self.last_ts,
                    });
                }
            }
            "queue-operation" => {
                let text = rec.content.text.unwrap_or_default();
                match rec.operation.get() {
                    Some("enqueue") => {
                        // Every background completion is enqueued, even when
                        // it is later absorbed mid-turn (no user record).
                        if let Some(note) = parse_task_notification(&text) {
                            self.complete_task(&note, ts);
                        }
                        self.queue.push(QueuedPrompt {
                            notification: text.contains("<task-notification>"),
                            text: clip_chars(&text, TEXT_MAX),
                            ts,
                        });
                        if self.queue.len() > QUEUE_CAP {
                            self.queue.remove(0);
                        }
                    }
                    Some("dequeue") | Some("remove") | Some("popAll") => {
                        let clipped = clip_chars(&text, TEXT_MAX);
                        if let Some(i) = (!text.is_empty())
                            .then(|| self.queue.iter().position(|q| q.text == clipped))
                            .flatten()
                        {
                            self.queue.remove(i);
                        } else if rec.operation.is("popAll") {
                            self.queue.clear();
                        } else if !self.queue.is_empty() {
                            self.queue.remove(0);
                        }
                    }
                    _ => {}
                }
            }
            "file-history-snapshot" => {
                if let Some(s) = rec.snapshot.get() {
                    for k in &s.tracked_file_backups.0 {
                        self.file_changed(k, 0, 0);
                    }
                }
            }
            "file-history-delta" => {
                if let Some(p) = rec.tracking_path.text.as_deref() {
                    self.file_changed(p, 0, 0);
                }
            }
            _ => {}
        }
        // Only timestamped records move the clock (metadata is re-appended
        // after every turn without timestamps).
        self.touch_ts(ts);
    }

    fn apply_assistant(&mut self, rec: Rec, ts: Option<i64>) {
        let Some(msg) = rec.message.0 else { return };
        let model = msg.model.get().unwrap_or("").to_string();
        let synthetic = pricing::is_synthetic(&model);
        if let Some(e) = rec.effort.get() {
            self.effort = Some(e.to_string());
        }
        if rec.is_api_error.yes() {
            let text = match &msg.content {
                Content::Blocks(b) => b
                    .iter()
                    .find_map(|b| b.text.text.clone())
                    .unwrap_or_default(),
                Content::Text(t) => t.text.clone().unwrap_or_default(),
                Content::None => String::new(),
            };
            let kind = if rec.quota_limits.0 {
                "quota".to_string()
            } else {
                rec.error.0.clone().unwrap_or_else(|| "api_error".into())
            };
            self.last_error = Some(LastErrorRec {
                kind,
                text: clip_chars(&text, TEXT_MAX),
                at: ts.or(self.last_ts).unwrap_or(0),
            });
            self.last_mark = Mark {
                kind: MarkKind::ApiError,
                ts,
                retry: None,
            };
            return;
        }
        if synthetic {
            return; // placeholder messages: no usage, no model, no context fill
        }
        if !model.is_empty() && self.model.as_deref() != Some(model.as_str()) {
            self.model = Some(model.clone());
        }
        self.last_error = None;
        if let (Some(id), Some(u)) = (msg.id.get(), msg.usage.get()) {
            let totals = Totals::of(&model, u);
            let after = self.cost_state.is_some();
            self.usage.record(id, totals, ts, after);
            if !rec.is_sidechain.yes() {
                let used = u.input_tokens.n()
                    + u.cache_read_input_tokens.n()
                    + u.cache_creation_input_tokens.n();
                if used > 0 {
                    self.context = Some(ContextRec {
                        used,
                        model: model.clone(),
                        ts,
                    });
                }
            }
        }
        let mut ended_on_tool = false;
        if let Content::Blocks(blocks) = msg.content {
            for b in blocks {
                match b.ty.get() {
                    Some("text") => {
                        if let Some(t) = b.text.get().filter(|t| !t.trim().is_empty()) {
                            self.last_text = Some(clip_chars(t, TEXT_MAX));
                            self.last_act = Some(LastAct {
                                kind: "text".into(),
                                tool: None,
                                summary: Some(clip_chars(t, 120)),
                                ts,
                            });
                        }
                    }
                    Some("thinking") | Some("redacted_thinking") => {
                        self.last_act = Some(LastAct {
                            kind: "thinking".into(),
                            tool: None,
                            summary: None,
                            ts,
                        });
                    }
                    Some("tool_use") | Some("server_tool_use") => {
                        ended_on_tool = true;
                        self.tool_uses += 1;
                        let name = b.name.get().unwrap_or("").to_string();
                        let id = b.id.get().unwrap_or("").to_string();
                        let input = b.input.0;
                        let summary = crate::claude::uievents::tool_summary(&name, input.as_ref());
                        self.last_act = Some(LastAct {
                            kind: "tool".into(),
                            tool: Some(name.clone()),
                            summary: Some(summary.clone()),
                            ts,
                        });
                        if name == "TodoWrite" {
                            if let Some(todos) = input.as_ref().and_then(todos_of) {
                                self.todos = Some(todos);
                            }
                        }
                        if matches!(name.as_str(), "Agent" | "Task") && !id.is_empty() {
                            let get = |k: &str| {
                                input
                                    .as_ref()
                                    .and_then(|i| i.get(k))
                                    .and_then(Value::as_str)
                                    .map(|s| clip_chars(s, 200))
                            };
                            let background = input
                                .as_ref()
                                .and_then(|i| i.get("run_in_background"))
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            if !self.agent_calls.iter().any(|c| c.tool_use_id == id) {
                                self.agent_calls.push(AgentCall {
                                    tool_use_id: id.clone(),
                                    description: get("description"),
                                    subagent_type: get("subagent_type"),
                                    background,
                                    ts,
                                    ..Default::default()
                                });
                                if self.agent_calls.len() > CALL_CAP {
                                    self.agent_calls.remove(0);
                                }
                            }
                        }
                        if !id.is_empty() && !self.pending_tools.iter().any(|p| p.id == id) {
                            self.pending_tools.push(PendingTool {
                                id,
                                name,
                                summary,
                                ts,
                                input: input.as_ref().and_then(small_input),
                            });
                            if self.pending_tools.len() > PENDING_CAP {
                                self.pending_tools.remove(0);
                            }
                            let n = self.pending_tools.len();
                            if n > PENDING_WITH_INPUT {
                                self.pending_tools[n - PENDING_WITH_INPUT - 1].input = None;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let kind = match msg.stop_reason.get() {
            Some("tool_use") => MarkKind::ToolUse,
            Some("end_turn") | Some("stop_sequence") => MarkKind::EndTurn,
            _ if ended_on_tool => MarkKind::ToolUse,
            _ => MarkKind::Assistant,
        };
        self.last_mark = Mark {
            kind,
            ts,
            retry: None,
        };
    }

    fn apply_user(&mut self, rec: Rec, ts: Option<i64>) {
        let origin = rec
            .origin
            .get()
            .and_then(|o| o.kind.get())
            .map(str::to_string);
        let Some(msg) = rec.message.0 else { return };
        let mut text_blocks: Vec<String> = Vec::new();
        let mut results: Vec<(String, bool)> = Vec::new();
        match &msg.content {
            Content::Text(t) => {
                if let Some(t) = t.text.clone() {
                    text_blocks.push(t);
                }
            }
            Content::Blocks(blocks) => {
                for b in blocks {
                    match b.ty.get() {
                        Some("text") => {
                            if let Some(t) = b.text.text.clone() {
                                text_blocks.push(t);
                            }
                        }
                        Some("tool_result") => {
                            if let Some(id) = b.tool_use_id.get() {
                                results.push((id.to_string(), b.is_error.yes()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Content::None => {}
        }
        let joined = text_blocks.join("\n");
        // Task notifications (background completions).
        let is_notification = origin.as_deref() == Some("task-notification")
            || joined.contains("<task-notification>");
        if is_notification {
            if let Some(note) = parse_task_notification(&joined) {
                self.complete_task(&note, ts);
            }
        }
        if !results.is_empty() {
            let tur = rec.tool_use_result.0;
            for (id, is_error) in &results {
                let pending = self
                    .pending_tools
                    .iter()
                    .position(|p| &p.id == id)
                    .map(|i| self.pending_tools.remove(i));
                if let Some(p) = &pending {
                    self.last_act = Some(LastAct {
                        kind: "result".into(),
                        tool: Some(p.name.clone()),
                        summary: Some(p.summary.clone()),
                        ts,
                    });
                }
                if let Some(call) = self.agent_calls.iter_mut().find(|c| &c.tool_use_id == id) {
                    call.result_seen = true;
                    call.is_error = *is_error;
                    if let Some(t) = &tur {
                        if let Some(a) = t.agent_id.get() {
                            call.agent_id = Some(a.to_string());
                        }
                        if t.status.is("async_launched") {
                            call.async_launched = true;
                        }
                    }
                }
                if let Some(t) = &tur {
                    self.apply_tool_result(id, pending.as_ref(), t, ts);
                }
            }
            self.last_mark = Mark {
                kind: MarkKind::ToolResult,
                ts,
                retry: None,
            };
            return;
        }
        if rec.is_meta.yes() || text_blocks.is_empty() {
            return;
        }
        if joined.contains("[Request interrupted by user") {
            self.last_mark = Mark {
                kind: MarkKind::Interrupted,
                ts,
                retry: None,
            };
            return;
        }
        if !is_notification {
            self.prompts += 1;
            if let Some(m) = rec.permission_mode.take() {
                self.permission_mode = Some((m, ts.or(self.last_ts).unwrap_or(0)));
            }
            if !joined.starts_with("<command-") && !joined.starts_with("<local-command") {
                self.last_prompt = Some(clip_chars(&joined, TEXT_MAX));
            }
        }
        self.last_mark = Mark {
            kind: MarkKind::Prompt,
            ts,
            retry: None,
        };
    }

    fn complete_task(&mut self, note: &TaskNote, ts: Option<i64>) {
        let status = match note.status.as_str() {
            "completed" | "success" => "completed",
            "failed" | "error" => "failed",
            "killed" => "killed",
            "stopped" | "cancelled" => "stopped",
            other => other,
        }
        .to_string();
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == note.task_id) {
            t.status = status;
            t.ended_at = ts;
            t.summary = note.summary.clone();
            if note.output_file.is_some() {
                t.output_file = note.output_file.clone();
            }
        } else {
            let kind = match note.task_id.chars().next() {
                Some('w') => "workflow",
                Some('a') => "agent",
                _ => "bash",
            };
            self.upsert_task(BgTask {
                id: note.task_id.clone(),
                kind: kind.into(),
                label: note.summary.clone().unwrap_or_default(),
                status,
                ended_at: ts,
                output_file: note.output_file.clone(),
                tool_use_id: note.tool_use_id.clone(),
                summary: note.summary.clone(),
                ..Default::default()
            });
        }
    }

    fn apply_tool_result(
        &mut self,
        tool_use_id: &str,
        p: Option<&PendingTool>,
        t: &Tur,
        ts: Option<i64>,
    ) {
        let name = p.map(|p| p.name.as_str()).unwrap_or("");
        let label = || p.map(|p| p.summary.clone()).unwrap_or_default();
        // A background task stopped by the model.
        if matches!(name, "TaskStop" | "KillShell" | "KillBash") {
            let id = t.task_id_snake.get().map(str::to_string).or_else(|| {
                p.and_then(|p| p.input.as_ref()).and_then(|i| {
                    i.get("task_id")
                        .or_else(|| i.get("shell_id"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
            });
            if let Some(id) = id {
                if let Some(task) = self.tasks.iter_mut().find(|x| x.id == id) {
                    if task.status == "running" {
                        task.status = "stopped".into();
                        task.ended_at = ts;
                    }
                }
            }
        }
        // Background bash.
        if let Some(task) = t.background_task_id.get() {
            self.upsert_task(BgTask {
                id: task.to_string(),
                kind: "bash".into(),
                label: label(),
                status: "running".into(),
                started_at: ts,
                tool_use_id: Some(tool_use_id.to_string()),
                output_file: t.output_file.text.clone(),
                ..Default::default()
            });
        }
        // Async agent.
        if t.status.is("async_launched") {
            if let Some(run) = t.run_id.get() {
                let task_id = t.task_id.get().map(str::to_string);
                let name = t
                    .workflow_name
                    .text
                    .clone()
                    .unwrap_or_else(|| run.to_string());
                if let Some(task) = &task_id {
                    self.upsert_task(BgTask {
                        id: task.clone(),
                        kind: "workflow".into(),
                        label: name.clone(),
                        status: "running".into(),
                        started_at: ts,
                        tool_use_id: Some(tool_use_id.to_string()),
                        run_id: Some(run.to_string()),
                        ..Default::default()
                    });
                }
                if !self.workflow_runs.iter().any(|w| w.run_id == run) {
                    self.workflow_runs.push(WorkflowLaunch {
                        run_id: run.to_string(),
                        task_id,
                        name,
                        ts,
                    });
                    if self.workflow_runs.len() > WORKFLOW_CAP {
                        self.workflow_runs.remove(0);
                    }
                }
            } else if let Some(agent) = t.agent_id.get() {
                self.upsert_task(BgTask {
                    id: agent.to_string(),
                    kind: "agent".into(),
                    label: t.description.text.clone().unwrap_or_else(label),
                    status: "running".into(),
                    started_at: ts,
                    tool_use_id: Some(tool_use_id.to_string()),
                    output_file: t.output_file.text.clone(),
                    ..Default::default()
                });
            }
        }
        // Monitor.
        if name == "Monitor" {
            if let Some(task) = t.task_id.get() {
                self.upsert_task(BgTask {
                    id: task.to_string(),
                    kind: "monitor".into(),
                    label: label(),
                    status: "running".into(),
                    started_at: ts,
                    tool_use_id: Some(tool_use_id.to_string()),
                    ..Default::default()
                });
            }
        }
        // Files changed.
        if let Some(path) = t.file_path.text.as_deref() {
            let (mut a, r) = (t.structured_patch.added, t.structured_patch.removed);
            if t.ty.is("create") && a == 0 {
                a = t.content.0;
            }
            self.file_changed(path, a, r);
        }
        if let Some(diff) = t.bash_edit_diff.get() {
            for f in &diff.changed_files.0 {
                if let Some(path) = f.file_path.text.as_deref() {
                    self.file_changed(path, f.hunks.added, f.hunks.removed);
                }
            }
        }
        // Todos (result form).
        if !t.new_todos.0.is_empty() {
            self.todos = Some(
                t.new_todos
                    .0
                    .iter()
                    .filter_map(|r| {
                        Some(Todo {
                            content: clip_chars(r.content.get()?, 512),
                            status: match r.status.get() {
                                Some("completed") => TodoStatus::Completed,
                                Some("in_progress") => TodoStatus::InProgress,
                                _ => TodoStatus::Pending,
                            },
                            active_form: r.active_form.get().map(|s| clip_chars(s, 512)),
                        })
                    })
                    .take(200)
                    .collect(),
            );
        }
    }

    fn apply_attachment(&mut self, rec: Rec, ts: Option<i64>) {
        let Some(att) = rec.attachment.0 else { return };
        match att.ty.get() {
            Some("model") => {
                if let Some(m) = att.identity.get().and_then(|i| i.model_id.get()) {
                    if !pricing::is_synthetic(m) {
                        self.model_attachment = Some(m.to_string());
                    }
                }
            }
            Some("queued_command") => {
                if let Some(note) = att.prompt.get().and_then(parse_task_notification) {
                    self.complete_task(&note, ts);
                }
            }
            Some("task_status") => {
                if let Some(task) = att.task_id.get() {
                    let kind = match att.task_type.get() {
                        Some(t) if t.contains("workflow") => "workflow",
                        Some(t) if t.contains("agent") => "agent",
                        Some(t) if t.contains("monitor") => "monitor",
                        _ => "bash",
                    };
                    let label = att
                        .description
                        .text
                        .clone()
                        .or_else(|| att.shell.get().and_then(|s| s.command.text.clone()))
                        .unwrap_or_default();
                    let status = att.status.get().unwrap_or("running").to_string();
                    self.upsert_task(BgTask {
                        id: task.to_string(),
                        kind: kind.into(),
                        label: clip_chars(&label, TEXT_MAX),
                        status: status.clone(),
                        started_at: ts,
                        output_file: att.output_file_path.text.clone(),
                        tool_use_id: att
                            .shell
                            .get()
                            .and_then(|s| s.tool_use_id.get().map(str::to_string)),
                        ..Default::default()
                    });
                    if status != "running" {
                        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == task) {
                            t.status = status;
                            t.ended_at = t.ended_at.or(ts);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn apply_system(&mut self, rec: Rec, ts: Option<i64>) {
        match rec.subtype.get() {
            Some("compact_boundary") => {
                self.compactions += 1;
                self.last_compact_ts = ts;
                self.last_mark = Mark {
                    kind: MarkKind::Compact,
                    ts,
                    retry: None,
                };
            }
            Some("api_error") => {
                self.last_error = Some(LastErrorRec {
                    kind: "api_error".into(),
                    text: rec.error.0.unwrap_or_else(|| "API error".into()),
                    at: ts.or(self.last_ts).unwrap_or(0),
                });
                self.last_mark = Mark {
                    kind: MarkKind::ApiError,
                    ts,
                    retry: rec
                        .retry_attempt
                        .0
                        .map(|a| (a, rec.max_retries.0.unwrap_or(0))),
                };
            }
            Some("stop_hook_summary") | Some("turn_duration")
                if !matches!(self.last_mark.kind, MarkKind::Prompt | MarkKind::ToolResult) =>
            {
                self.last_mark = Mark {
                    kind: MarkKind::TurnEnd,
                    ts,
                    retry: None,
                };
            }
            _ => {}
        }
    }

    /// A copy without the large lists (tasks, agent calls, workflow runs,
    /// files changed) and without the usage accumulator — what a summary
    /// needs, cheap enough to take on every refresh.
    pub fn light_clone(&self) -> Aggregates {
        Aggregates {
            cwd: self.cwd.clone(),
            git_branch: self.git_branch.clone(),
            version: self.version.clone(),
            entrypoint: self.entrypoint.clone(),
            first_ts: self.first_ts,
            last_ts: self.last_ts,
            usage: UsageAcc::default(),
            prompts: self.prompts,
            tool_uses: self.tool_uses,
            model_attachment: self.model_attachment.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            permission_mode: self.permission_mode.clone(),
            custom_title: self.custom_title.clone(),
            ai_title: self.ai_title.clone(),
            agent_name: self.agent_name.clone(),
            last_prompt: self.last_prompt.clone(),
            last_text: self.last_text.clone(),
            pr_link: self.pr_link.clone(),
            cost_state: self.cost_state.clone(),
            context: self.context.clone(),
            todos: self.todos.clone(),
            queue: self.queue.clone(),
            pending_tools: self.pending_tools.clone(),
            tasks: Vec::new(),
            agent_calls: Vec::new(),
            workflow_runs: Vec::new(),
            last_error: self.last_error.clone(),
            compactions: self.compactions,
            last_compact_ts: self.last_compact_ts,
            remote_control: self.remote_control,
            files_changed: BTreeMap::new(),
            last_mark: self.last_mark.clone(),
            last_act: self.last_act.clone(),
        }
    }

    /// The model to show: the `model` attachment (carries `[1m]`), else the
    /// last message model.
    pub fn display_model(&self) -> Option<&str> {
        self.model_attachment.as_deref().or(self.model.as_deref())
    }
}

// ─────────────────────────────── the file index ──────────────────────────────

/// Largest line kept in memory for parsing; longer lines are skipped.
pub const LINE_CAP: usize = 64 * 1024 * 1024;
/// Line buffer capacity kept between lines (bigger buffers are released).
const BUF_KEEP: usize = 1024 * 1024;

/// Kind of transcript file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileKind {
    #[default]
    Main,
    Subagent,
}

/// Incremental state of one transcript file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileIndex {
    pub path: PathBuf,
    pub kind: FileKind,
    pub session_id: String,
    /// Subagent files: the agent id (`agent-<id>.jsonl`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Workflow agent files: the run id (`wf_…`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub ino: u64,
    /// Size at the last scan.
    pub size: u64,
    pub mtime: i64,
    /// Just past the last complete line consumed.
    pub offset: u64,
    /// Where the scan began: 0 for a full pass, else a tail-window bootstrap
    /// (lifetime aggregates incomplete).
    pub start: u64,
    pub agg: Aggregates,
    /// Highest offset already published to transcript subscribers (not
    /// persisted).
    #[serde(skip)]
    pub ui_cursor: u64,
}

/// File identity + size + mtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub ino: u64,
    pub size: u64,
    pub mtime: i64,
}

pub fn stamp_of(meta: &std::fs::Metadata) -> Stamp {
    #[cfg(unix)]
    let ino = std::os::unix::fs::MetadataExt::ino(meta);
    #[cfg(not(unix))]
    let ino = 0u64;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Stamp {
        ino,
        size: meta.len(),
        mtime,
    }
}

/// Outcome of reading one line.
enum LineRead {
    Line,
    /// A line longer than [`LINE_CAP`] was skipped.
    Skipped,
    /// EOF (possibly with a partial line not consumed).
    Eof,
}

/// Read one `\n`-terminated line into `buf` (cleared first). A partial line at
/// EOF is reported as `Eof` and must not be consumed: the caller keeps its
/// offset at the line start. Lines over `cap` are skipped without buffering.
fn read_line_capped<R: BufRead>(
    r: &mut R,
    buf: &mut Vec<u8>,
    cap: usize,
    consumed: &mut u64,
) -> std::io::Result<LineRead> {
    buf.clear();
    let mut skipping = false;
    let mut total: u64 = 0;
    loop {
        let avail = match r.fill_buf() {
            Ok(a) => a,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if avail.is_empty() {
            // EOF: an incomplete line stays unconsumed (offset not advanced).
            return Ok(LineRead::Eof);
        }
        match memchr_nl(avail) {
            Some(i) => {
                if !skipping {
                    buf.extend_from_slice(&avail[..=i]);
                }
                total += (i + 1) as u64;
                r.consume(i + 1);
                *consumed += total;
                return Ok(if skipping {
                    LineRead::Skipped
                } else {
                    LineRead::Line
                });
            }
            None => {
                let n = avail.len();
                if !skipping {
                    if buf.len() + n > cap {
                        skipping = true;
                        buf.clear();
                    } else {
                        buf.extend_from_slice(avail);
                    }
                }
                total += n as u64;
                r.consume(n);
            }
        }
    }
}

fn memchr_nl(s: &[u8]) -> Option<usize> {
    s.iter().position(|b| *b == b'\n')
}

impl FileIndex {
    pub fn new(path: PathBuf, kind: FileKind, session_id: &str) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let agent_id = (kind == FileKind::Subagent)
            .then(|| {
                name.strip_prefix("agent-")
                    .and_then(|n| n.strip_suffix(".jsonl"))
                    .map(str::to_string)
            })
            .flatten();
        let run_id = (kind == FileKind::Subagent)
            .then(|| {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .filter(|n| n.starts_with("wf_"))
                    .map(str::to_string)
            })
            .flatten();
        Self {
            path,
            kind,
            session_id: session_id.to_string(),
            agent_id,
            run_id,
            ..Default::default()
        }
    }

    /// Lifetime aggregates cover the whole file.
    pub fn is_complete(&self) -> bool {
        self.start == 0
    }

    /// Scan from the current offset up to EOF (or `limit` bytes). Returns the
    /// bytes consumed. A shrunk or replaced file restarts from 0.
    pub fn scan(&mut self, limit: Option<u64>) -> std::io::Result<u64> {
        let file = File::open(&self.path)?;
        let meta = file.metadata()?;
        let st = stamp_of(&meta);
        if (self.ino != 0 && st.ino != 0 && st.ino != self.ino) || st.size < self.offset {
            // Replaced or truncated: start over.
            let (path, kind, sid) = (self.path.clone(), self.kind, self.session_id.clone());
            let ui = self.ui_cursor;
            *self = FileIndex::new(path, kind, &sid);
            self.ui_cursor = ui.min(st.size);
        }
        self.ino = st.ino;
        self.mtime = st.mtime;
        self.size = st.size;
        if st.size == self.offset {
            return Ok(0);
        }
        let mut reader = BufReader::with_capacity(256 * 1024, file);
        reader.seek(SeekFrom::Start(self.offset))?;
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        let start = self.offset;
        loop {
            if let Some(l) = limit {
                if self.offset - start >= l {
                    break;
                }
            }
            let mut consumed = 0u64;
            match read_line_capped(&mut reader, &mut buf, LINE_CAP, &mut consumed)? {
                LineRead::Eof => break,
                LineRead::Skipped => {
                    self.offset += consumed;
                    tracing::debug!(path = %self.path.display(), bytes = consumed, "oversized transcript line skipped");
                }
                LineRead::Line => {
                    self.offset += consumed;
                    if let Some(rec) = parse_line::<Rec>(&buf) {
                        self.agg.apply(rec);
                    }
                    if buf.capacity() > BUF_KEEP {
                        buf = Vec::with_capacity(64 * 1024);
                    }
                }
            }
        }
        Ok(self.offset - start)
    }

    /// Start at `from` (aligned to the next line start) instead of 0: a
    /// tail-window bootstrap for "now" state.
    pub fn seek_window(&mut self, window: u64) -> std::io::Result<()> {
        let meta = std::fs::metadata(&self.path)?;
        let size = meta.len();
        if size <= window {
            return Ok(());
        }
        let mut f = File::open(&self.path)?;
        let from = size - window;
        f.seek(SeekFrom::Start(from.saturating_sub(1)))?;
        let mut reader = BufReader::with_capacity(64 * 1024, f);
        // Skip to just past the next newline (the byte at from-1 may be one).
        let mut skipped: Vec<u8> = Vec::new();
        let n = reader.read_until(b'\n', &mut skipped)? as u64;
        self.offset = from.saturating_sub(1) + n;
        self.start = self.offset.max(1);
        Ok(())
    }
}

/// Session id and agent id from a transcript path, if it is one we index.
pub fn classify_path(projects: &Path, path: &Path) -> Option<(String, FileKind, Option<String>)> {
    let rel = path.strip_prefix(projects).ok()?;
    let parts: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
    match parts.as_slice() {
        [_enc, file] => {
            let sid = file.strip_suffix(".jsonl")?;
            crate::claude::ids::is_session_id(sid).then(|| (sid.to_string(), FileKind::Main, None))
        }
        [_enc, sid, "subagents", file] | [_enc, sid, "subagents", "workflows", _, file] => {
            let agent = file.strip_prefix("agent-")?.strip_suffix(".jsonl")?;
            (crate::claude::ids::is_session_id(sid) && crate::claude::ids::is_agent_id(agent))
                .then(|| (sid.to_string(), FileKind::Subagent, Some(agent.to_string())))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scan_str(lines: &str) -> FileIndex {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, lines).unwrap();
        let mut fi = FileIndex::new(path, FileKind::Main, "s");
        fi.scan(None).unwrap();
        fi
    }

    fn asst(id: &str, model: &str, out: u64, extra: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-09-25T10:00:00.000Z","message":{{"id":"{id}","model":"{model}","role":"assistant","content":[{{"type":"text","text":"hi"}}],"usage":{{"input_tokens":100,"output_tokens":{out},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}{extra}}}}}"#
        ) + "\n"
    }

    #[test]
    fn usage_dedups_by_message_id_keeping_the_last_record() {
        // Three block records of one message with growing (partial) usage,
        // then a second message.
        let lines = asst("m1", "claude-haiku-4-5", 8, "")
            + &asst("m1", "claude-haiku-4-5", 100, "")
            + &asst("m1", "claude-haiku-4-5", 288, "")
            + &asst("m2", "claude-haiku-4-5", 10, "");
        let fi = scan_str(&lines);
        let t = fi.agg.usage.total();
        assert_eq!(t.messages, 2);
        assert_eq!(t.output, 298);
        assert_eq!(t.input, 200);
        let expect = (200.0 * 1.0 + 298.0 * 5.0) / 1e6;
        assert!((t.cost - expect).abs() < 1e-12);
    }

    #[test]
    fn dedup_survives_the_open_window() {
        let mut lines = String::new();
        for i in 0..40 {
            lines += &asst(&format!("m{i}"), "claude-haiku-4-5", 1, "");
            lines += &asst(&format!("m{i}"), "claude-haiku-4-5", 2, "");
        }
        let fi = scan_str(&lines);
        let t = fi.agg.usage.total();
        assert_eq!(t.messages, 40);
        assert_eq!(t.output, 80);
        assert!(fi.agg.usage.open.len() <= OPEN_WINDOW);
    }

    #[test]
    fn synthetic_records_are_skipped() {
        let lines = asst("m1", "<synthetic>", 999, "") + &asst("m2", "claude-opus-5", 1, "");
        let fi = scan_str(&lines);
        assert_eq!(fi.agg.usage.total().messages, 1);
        assert_eq!(fi.agg.model.as_deref(), Some("claude-opus-5"));
    }

    #[test]
    fn cost_state_splits_before_and_after() {
        let lines = asst("m1", "claude-haiku-4-5", 100, "")
            + r#"{"type":"cost-state","totalCostUSD":1.5}"#
            + "\n"
            + &asst("m2", "claude-haiku-4-5", 200, "");
        let fi = scan_str(&lines);
        assert_eq!(fi.agg.cost_state.as_ref().unwrap().total, 1.5);
        let after = fi.agg.usage.after_cost_state();
        assert_eq!(after.messages, 1);
        assert_eq!(after.output, 200);
        assert_eq!(fi.agg.usage.total().messages, 2);
    }

    #[test]
    fn nonstandard_speed_is_flagged() {
        let lines = asst("m1", "claude-opus-5", 1, r#","speed":"fast""#);
        // `speed` belongs inside usage; build it by hand.
        let lines = lines.replace(
            r#""cache_creation_input_tokens":0}"#,
            r#""cache_creation_input_tokens":0,"speed":"fast"}"#,
        );
        let fi = scan_str(&lines);
        assert_eq!(fi.agg.usage.total().nonstandard, 1);
    }

    #[test]
    fn metadata_tools_todos_and_tasks() {
        let lines = concat!(
            r#"{"type":"user","timestamp":"2026-09-25T10:00:00.000Z","permissionMode":"default","origin":{"kind":"human"},"cwd":"/p","gitBranch":"main","version":"2.1.282","entrypoint":"cli","message":{"role":"user","content":"please fix"}}"#,
            "\n",
            r#"{"type":"attachment","attachment":{"type":"model","identity":{"modelId":"claude-opus-5[1m]"}}}"#,
            "\n",
            r#"{"type":"assistant","timestamp":"2026-09-25T10:00:01.000Z","effort":"xhigh","message":{"id":"m1","model":"claude-opus-5","stop_reason":"tool_use","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"npm run dev","description":"Start dev server","run_in_background":true}},{"type":"tool_use","id":"t2","name":"TodoWrite","input":{"todos":[{"content":"a","status":"completed"},{"content":"b","status":"in_progress","activeForm":"Doing b"}]}}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":1000,"cache_creation_input_tokens":20}}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-09-25T10:00:02.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"started"}]},"toolUseResult":{"stdout":"","backgroundTaskId":"bx9ov4bnx"}}"#,
            "\n",
            r#"{"type":"assistant","timestamp":"2026-09-25T10:00:03.000Z","message":{"id":"m2","model":"claude-opus-5","stop_reason":"tool_use","content":[{"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"/p/a.rs","old_string":"x","new_string":"y"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-09-25T10:00:04.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t3","content":"ok"}]},"toolUseResult":{"filePath":"/p/a.rs","structuredPatch":[{"lines":["-x","+y","+z"]}],"originalFile":"HUGE"}}"#,
            "\n",
            r#"{"type":"queue-operation","operation":"enqueue","content":"next thing","timestamp":"2026-09-25T10:00:05.000Z"}"#,
            "\n",
            r#"{"type":"queue-operation","operation":"enqueue","content":"<task-notification>x</task-notification>"}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-09-25T10:00:06.000Z","origin":{"kind":"task-notification"},"message":{"role":"user","content":"<task-notification>\n<task-id>bx9ov4bnx</task-id>\n<tool-use-id>t1</tool-use-id>\n<output-file>/private/tmp/claude-501/x/tasks/bx9ov4bnx.output</output-file>\n<status>completed</status>\n<summary>Dev server exited</summary>\n</task-notification>"}}"#,
            "\n",
            r#"{"type":"last-prompt","lastPrompt":"please fix"}"#,
            "\n",
            r#"{"type":"ai-title","aiTitle":"Fix the thing"}"#,
            "\n",
            r#"{"type":"custom-title","customTitle":"My title"}"#,
            "\n",
            r#"{"type":"permission-mode","permissionMode":"auto"}"#,
            "\n",
            r#"{"type":"pr-link","prNumber":15,"prUrl":"https://github.com/o/r/pull/15","prRepository":"o/r"}"#,
            "\n",
            r#"{"type":"bridge-session","bridgeSessionId":"cse_1","ownerAccountUuid":"secret"}"#,
            "\n",
            r#"{"type":"system","subtype":"api_error","timestamp":"2026-09-25T10:00:07.000Z","error":{"formatted":"529 Overloaded"},"retryAttempt":1,"maxRetries":10}"#,
            "\n",
            "not json at all\n",
        );
        let fi = scan_str(lines);
        let a = &fi.agg;
        assert_eq!(a.cwd.as_deref(), Some("/p"));
        assert_eq!(a.git_branch.as_deref(), Some("main"));
        assert_eq!(a.display_model(), Some("claude-opus-5[1m]"));
        assert_eq!(a.effort.as_deref(), Some("xhigh"));
        assert_eq!(a.permission_mode.as_ref().unwrap().0, "auto");
        assert_eq!(a.custom_title.as_deref(), Some("My title"));
        assert_eq!(a.ai_title.as_deref(), Some("Fix the thing"));
        assert_eq!(a.last_prompt.as_deref(), Some("please fix"));
        assert_eq!(a.pr_link.as_ref().unwrap().number, 15);
        assert!(a.remote_control);
        assert_eq!(a.todos.as_ref().unwrap().len(), 2);
        // Context fill = the LAST main-chain assistant message (m2).
        assert_eq!(a.context.as_ref().unwrap().used, 1);
        // TodoWrite never got a result: it is still pending; Bash and Edit got theirs.
        assert_eq!(a.pending_tools.len(), 1);
        assert_eq!(a.pending_tools[0].name, "TodoWrite");
        let task = a.tasks.iter().find(|t| t.id == "bx9ov4bnx").unwrap();
        assert_eq!(task.status, "completed");
        assert_eq!(task.label, "Start dev server");
        assert_eq!(
            task.output_file.as_deref(),
            Some("/private/tmp/claude-501/x/tasks/bx9ov4bnx.output")
        );
        assert_eq!(a.files_changed.get("/p/a.rs"), Some(&(2, 1)));
        assert_eq!(a.queue.iter().filter(|q| !q.notification).count(), 1);
        assert_eq!(a.last_error.as_ref().unwrap().text, "529 Overloaded");
        assert_eq!(a.last_mark.kind, MarkKind::ApiError);
        assert_eq!(a.last_mark.retry, Some((1, 10)));
        assert_eq!(a.prompts, 1);
    }

    #[test]
    fn incremental_scan_never_rereads_and_buffers_partial_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let first = asst("m1", "claude-haiku-4-5", 1, "");
        std::fs::write(&path, &first).unwrap();
        let mut fi = FileIndex::new(path.clone(), FileKind::Main, "s");
        assert_eq!(fi.scan(None).unwrap(), first.len() as u64);
        assert_eq!(fi.scan(None).unwrap(), 0, "unchanged file: nothing read");
        // Append a partial line: not consumed.
        let second = asst("m2", "claude-haiku-4-5", 1, "");
        let (a, b) = second.split_at(20);
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(a.as_bytes()).unwrap();
        f.flush().unwrap();
        assert_eq!(fi.scan(None).unwrap(), 0);
        assert_eq!(fi.offset, first.len() as u64);
        f.write_all(b.as_bytes()).unwrap();
        f.flush().unwrap();
        assert_eq!(fi.scan(None).unwrap(), second.len() as u64);
        assert_eq!(fi.agg.usage.total().messages, 2);
        // Truncation restarts.
        std::fs::write(&path, &first).unwrap();
        fi.scan(None).unwrap();
        assert_eq!(fi.agg.usage.total().messages, 1);
    }

    #[test]
    fn oversized_lines_are_skipped_without_losing_sync() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let big = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"image","source":{{"data":"{}"}}}}]}}}}"#,
            "A".repeat(300)
        );
        let text = big.clone() + "\n" + &asst("m1", "claude-haiku-4-5", 1, "");
        std::fs::write(&path, &text).unwrap();
        let mut buf = Vec::new();
        let mut r = BufReader::with_capacity(16, File::open(&path).unwrap());
        let mut consumed = 0;
        // With a tiny cap the big line is skipped, the next one read.
        assert!(matches!(
            read_line_capped(&mut r, &mut buf, 64, &mut consumed).unwrap(),
            LineRead::Skipped
        ));
        assert_eq!(consumed, big.len() as u64 + 1);
        let mut consumed = 0;
        assert!(matches!(
            read_line_capped(&mut r, &mut buf, 64 * 1024, &mut consumed).unwrap(),
            LineRead::Line
        ));
    }

    #[test]
    fn tail_window_starts_at_a_line_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let mut text = String::new();
        for i in 0..100 {
            text += &asst(&format!("m{i}"), "claude-haiku-4-5", 1, "");
        }
        std::fs::write(&path, &text).unwrap();
        let mut fi = FileIndex::new(path, FileKind::Main, "s");
        fi.seek_window(1000).unwrap();
        assert!(!fi.is_complete());
        let off = fi.offset as usize;
        assert_eq!(&text.as_bytes()[off - 1..off], b"\n");
        fi.scan(None).unwrap();
        let n = fi.agg.usage.total().messages;
        assert!(n > 0 && n < 100);
    }

    #[test]
    fn paths_are_classified() {
        let projects = Path::new("/h/projects");
        let sid = "e2b6972f-7126-4f6e-a490-5ce75cb6a9f9";
        assert_eq!(
            classify_path(projects, &projects.join(format!("-p/{sid}.jsonl"))),
            Some((sid.to_string(), FileKind::Main, None))
        );
        assert_eq!(
            classify_path(
                projects,
                &projects.join(format!("-p/{sid}/subagents/agent-a12.jsonl"))
            ),
            Some((sid.to_string(), FileKind::Subagent, Some("a12".into())))
        );
        assert_eq!(
            classify_path(
                projects,
                &projects.join(format!(
                    "-p/{sid}/subagents/workflows/wf_x-1/agent-a13.jsonl"
                ))
            ),
            Some((sid.to_string(), FileKind::Subagent, Some("a13".into())))
        );
        assert_eq!(
            classify_path(projects, &projects.join("-p/agent-a1b2.jsonl")),
            None
        );
        assert_eq!(
            classify_path(projects, &projects.join("-p/not-a-uuid.jsonl")),
            None
        );
        assert_eq!(
            classify_path(projects, Path::new("/elsewhere/x.jsonl")),
            None
        );
    }

    #[test]
    fn absorbed_notifications_and_task_stop_finish_tasks() {
        let lines = concat!(
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-haiku-4-5","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"sleep 1000","run_in_background":true}},{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"tail -f log","run_in_background":true}}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"bg"}]},"toolUseResult":{"backgroundTaskId":"b1"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"bg"}]},"toolUseResult":{"backgroundTaskId":"b2"}}"#,
            "\n",
            // Completion delivered only as a queue enqueue (absorbed mid-turn).
            r#"{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>\n<summary>done</summary>\n</task-notification>"}"#,
            "\n",
            r#"{"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","content":"<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>\n<summary>done</summary>\n</task-notification>"}"#,
            "\n",
            r#"{"type":"assistant","message":{"id":"m2","model":"claude-haiku-4-5","content":[{"type":"tool_use","id":"t3","name":"TaskStop","input":{"task_id":"b2"}}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","content":"stopped"}]},"toolUseResult":{"message":"stopped","task_id":"b2"}}"#,
            "\n",
        );
        let fi = scan_str(lines);
        let t = |id: &str| {
            fi.agg
                .tasks
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .status
                .clone()
        };
        assert_eq!(t("b1"), "completed");
        assert_eq!(t("b2"), "stopped");
        assert!(fi.agg.queue.is_empty());
    }

    #[test]
    fn task_notification_parsing() {
        let n = parse_task_notification(
            "<task-notification>\n<task-id>w59or2obz</task-id>\n<status>failed</status>\n<summary>boom</summary>\n</task-notification>",
        )
        .unwrap();
        assert_eq!(n.task_id, "w59or2obz");
        assert_eq!(n.status, "failed");
        assert_eq!(n.summary.as_deref(), Some("boom"));
        assert!(parse_task_notification("hello").is_none());
    }
}
