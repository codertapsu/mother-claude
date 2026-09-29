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
    clip, parse_line, CapStr, Keys, LBool, LStr, LineCount, Obj, PatchCount, Seq, SmallValue, LF64,
    LU64,
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
    /// `output_config.effort` actually sent with this API request.
    pub effort: LStr,
    /// The per-turn effort layer (null before 2.1.282); the CLI rebuilds
    /// history as `perTurnEffort ?? effort`, so it wins when set.
    #[serde(rename = "perTurnEffort")]
    pub per_turn_effort: LStr,
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
    pub quota_limits: QuotaField,
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
    /// The summary a compaction left in place of the conversation.
    #[serde(rename = "isCompactSummary")]
    pub is_compact_summary: LBool,
    /// A queued message delivered without the per-prompt attachments (task
    /// notifications, relays).
    #[serde(rename = "queueSkipAttachments")]
    pub queue_skip_attachments: LBool,
    #[serde(rename = "queueTranscriptOnly")]
    pub queue_transcript_only: LBool,
    /// `compact_boundary`: how it was triggered and what it kept.
    #[serde(rename = "compactMetadata")]
    pub compact_metadata: Obj<CompactMeta>,
}

/// `compactMetadata` of a `compact_boundary` record.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CompactMeta {
    /// `auto` | `manual`.
    pub trigger: LStr,
    #[serde(rename = "preservedMessages")]
    pub preserved_messages: Obj<PreservedMessages>,
}

/// The messages a compaction kept after its summary.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PreservedMessages {
    pub uuids: Seq<LStr>,
    #[serde(rename = "allUuids")]
    pub all_uuids: Seq<LStr>,
}

impl CompactMeta {
    /// Only `auto` is known to drop earlier markers; an unknown trigger is
    /// treated like a manual `/compact`.
    fn is_auto(&self) -> bool {
        self.trigger.is("auto")
    }

    fn preserves(&self, uuid: &str) -> bool {
        self.preserved_messages.get().is_some_and(|p| {
            p.uuids
                .0
                .iter()
                .chain(p.all_uuids.0.iter())
                .any(|u| u.is(uuid))
        })
    }
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

/// `quotaLimits` of a rejected request (written only when a plan limit is
/// hit): present for any non-null value, with the fields the dashboard uses
/// when it is an object.
#[derive(Debug, Default)]
pub struct QuotaField {
    pub present: bool,
    pub detail: Option<QuotaRec>,
}

/// The `quotaLimits` fields kept (the rest — overage, upgrade paths — is
/// skipped).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct QuotaRec {
    /// `five_hour` | `seven_day` | …
    #[serde(rename = "rateLimitType")]
    pub rate_limit_type: LStr,
    /// Epoch seconds.
    #[serde(rename = "resetsAt")]
    pub resets_at: LF64,
    /// `rejected` | `allowed_warning` | …
    pub status: LStr,
}

impl<'de> Deserialize<'de> for QuotaField {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
        struct V;
        const PRESENT: QuotaField = QuotaField {
            present: true,
            detail: None,
        };
        impl<'de> Visitor<'de> for V {
            type Value = QuotaField;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("anything")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<QuotaField, A::Error> {
                let detail = QuotaRec::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(QuotaField {
                    present: true,
                    detail: Some(detail),
                })
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<QuotaField, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(PRESENT)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<QuotaField, E> {
                Ok(PRESENT)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<QuotaField, E> {
                Ok(PRESENT)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<QuotaField, E> {
                Ok(PRESENT)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<QuotaField, E> {
                Ok(PRESENT)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<QuotaField, E> {
                Ok(PRESENT)
            }
            fn visit_none<E: de::Error>(self) -> Result<QuotaField, E> {
                Ok(QuotaField::default())
            }
            fn visit_unit<E: de::Error>(self) -> Result<QuotaField, E> {
                Ok(QuotaField::default())
            }
            fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<QuotaField, D::Error> {
                QuotaField::deserialize(d)
            }
        }
        d.deserialize_any(V)
    }
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
    /// `(newest folded ts, cumulative folded cost)` as of the last fold of a
    /// timestamped message. The curve is sampled every `curve_stride` folds,
    /// so without this the unsampled tail would always count as "after" a
    /// cutoff that follows every folded message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fold: Option<(i64, f64)>,
    /// `(15-minute bucket start, cumulative folded cost before the bucket's
    /// first fold)` for the buckets folded in the last [`RECENT_KEEP_MS`]
    /// (sparse: only buckets that saw a message). Gives "cost since local
    /// midnight" exactly: every zone's midnight falls on a quarter hour.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<(i64, f64)>,
}

/// Below this (micro-dollar rounding of curve samples), a remainder is zero.
const COST_EPSILON: f64 = 1e-6;
/// Granularity of [`UsageAcc::recent`].
pub const COST_BUCKET_MS: i64 = 15 * 60 * 1000;
/// How far back [`UsageAcc::recent`] reaches (a day, plus DST slack).
pub const RECENT_KEEP_MS: i64 = 26 * 60 * 60 * 1000;

fn micro(c: f64) -> f64 {
    (c * 1e6).round() / 1e6
}

impl UsageAcc {
    fn fold(&mut self, m: OpenMsg) {
        if let Some(ts) = m.ts {
            let bucket = ts - ts.rem_euclid(COST_BUCKET_MS);
            // Out-of-order (older) messages count in the current bucket.
            if self.recent.last().is_none_or(|(b, _)| *b < bucket) {
                self.recent.push((bucket, micro(self.folded.cost)));
                let keep_from = bucket - RECENT_KEEP_MS;
                self.recent.retain(|(b, _)| *b >= keep_from);
            }
        }
        self.folded.add(&m.totals);
        if m.after_cost_state {
            self.after.add(&m.totals);
        }
        let stride = self.curve_stride.max(1);
        if let Some(ts) = m.ts {
            let newest = self.last_fold.map_or(ts, |(t, _)| t.max(ts));
            self.last_fold = Some((newest, micro(self.folded.cost)));
            if self.folded.messages.is_multiple_of(stride) {
                // Micro-dollar precision keeps checkpoints small.
                self.curve.push((ts, micro(self.folded.cost)));
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

    /// Forget [`recent`](Self::recent) buckets that start before `before`
    /// (checkpoints: history files carry none).
    pub fn forget_recent_before(&mut self, before: i64) {
        self.recent.retain(|(b, _)| *b >= before);
    }

    /// Priced cost of messages written at or after `since`, which must lie
    /// within [`RECENT_KEEP_MS`] of now (e.g. local midnight). Exact when
    /// `since` is on a quarter hour.
    pub fn cost_since(&self, since: i64) -> f64 {
        let folded = match self
            .recent
            .iter()
            .find(|(b, _)| *b + COST_BUCKET_MS > since)
        {
            Some((_, before)) => self.folded.cost - before,
            // No folded message since then.
            None => 0.0,
        };
        let folded = if folded < COST_EPSILON { 0.0 } else { folded };
        folded
            + self
                .open
                .iter()
                .filter(|m| m.ts.is_none_or(|t| t >= since))
                .map(|m| m.totals.cost)
                .sum::<f64>()
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
    /// cost curve for folded messages; exact for open ones, and exact (zero)
    /// when every timestamped folded message precedes `ts`).
    pub fn cost_after(&self, ts: i64) -> f64 {
        let before = match self.last_fold {
            // Every timestamped folded message is at or before `ts`: only
            // untimestamped ones folded since then can be "after".
            Some((newest, cost)) if newest <= ts => cost,
            _ => self
                .curve
                .iter()
                .rev()
                .find(|(t, _)| *t <= ts)
                .map(|(_, c)| *c)
                .unwrap_or(0.0),
        };
        let mut folded_after = self.folded.cost - before;
        if folded_after < COST_EPSILON {
            folded_after = 0.0;
        }
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

// ─────────────────────────────── ultracode ───────────────────────────────
//
// Claude Code (verified in 2.1.282–2.1.284) decides what to note about
// ultracode on each *regular* prompt of the main thread — a message someone
// sent, not a task notification, a relay, a `/loop` or cron fire, a prompt
// absorbed mid-turn or a local slash command. It walks back through its
// conversation to the last `ultra_effort_enter` / `ultra_effort_exit`
// attachment and writes, right after the prompt:
//
// * `ultra_effort_enter` "full" when ultracode is in effect and that last
//   marker is not an `enter` (it turned on, or the marker is gone);
// * `ultra_effort_enter` "sparse" when it stays on and about 10 messages
//   passed since that `enter` (task notifications count);
// * `ultra_effort_exit` when it is not in effect and that marker is `enter`;
// * nothing otherwise — so a regular prompt without a marker confirms what
//   the conversation already said (on after an `enter`, off after an `exit`
//   or with no marker at all).
//
// An automatic compaction hides the earlier markers (the next prompt with
// ultracode on writes "full" again); a manual `/compact` may keep them. A
// resumed session sees the markers of its history, but the new process starts
// with ultracode off (VS Code does not switch it back on), so the first
// prompt after a restart writes an `exit` unless it was switched on again.

/// The oldest Claude Code version seen writing ultracode markers: without a
/// marker, an older version's prompt says nothing.
const ULTRA_MIN_VERSION: (u32, u32, u32) = (2, 1, 246);

/// This Claude Code version writes ultracode markers.
pub fn notes_ultracode(version: &str) -> bool {
    version_triple(version).is_some_and(|v| v >= ULTRA_MIN_VERSION)
}

/// `major.minor.patch` of a Claude Code version (`2.1.284`, `2.1.284-rc.1`).
fn version_triple(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().splitn(3, '.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, minor, patch.parse().ok()?))
}

/// The marker Claude Code sees as the last one of its conversation, which
/// decides what it notes on the next regular prompt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UltraSeen {
    /// No marker (none yet, or an automatic compaction hid it).
    #[default]
    None,
    Enter,
    Exit,
    /// An `enter` followed by a manual `/compact`, which may or may not have
    /// kept it.
    Compacted,
    /// Not known: the history before a tail window could not be read.
    Unknown,
    /// A tail window not seeded yet with what came before it
    /// ([`UltraReplay::seed`]).
    Seed,
}

/// Why a regular prompt could not tell whether ultracode was on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UltraUnknown {
    /// A manual `/compact` may have hidden the last marker.
    Compacted,
    /// The history before the tail window has not been read yet.
    Reading,
    /// The prompt's Claude Code version predates ultracode markers.
    Unsupported,
}

/// Ultracode as of the latest regular prompt of the main chain.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UltraState {
    /// On or off as of `at`; `None` when that prompt could not tell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    /// Epoch ms of the prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    /// A marker at that prompt said so (else the prompt's silence did).
    #[serde(skip_serializing_if = "is_false")]
    pub marker: bool,
    /// Why `on` is `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown: Option<UltraUnknown>,
    /// A silent prompt of a tail window not seeded yet: `on` follows the seed.
    #[serde(skip_serializing_if = "is_false")]
    pub seeded: bool,
}

/// A regular prompt whose attachments are still being read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UltraPrompt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    /// Its Claude Code version writes ultracode markers.
    #[serde(skip_serializing_if = "is_false")]
    pub versioned: bool,
}

/// The ultracode replay of a main transcript (see the notes above): what
/// Claude Code sees, and the state as of the latest regular prompt. Bounded:
/// a few fields and one uuid.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UltraReplay {
    pub seen: UltraSeen,
    /// uuid of the last marker (a compaction that preserved it keeps it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<UltraPrompt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<UltraState>,
}

impl UltraReplay {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// A regular prompt: Claude Code notes ultracode right after it, or
    /// confirms what it sees by noting nothing.
    fn prompt(&mut self, at: Option<i64>, version: Option<&str>) {
        self.settle();
        let versioned = version.is_some_and(notes_ultracode);
        self.pending = Some(UltraPrompt { at, versioned });
    }

    /// A record past the pending prompt's attachments: it wrote no marker.
    pub fn settle(&mut self) {
        let Some(p) = self.pending.take() else { return };
        let mut st = UltraState {
            at: p.at,
            ..Default::default()
        };
        if !p.versioned {
            st.unknown = Some(UltraUnknown::Unsupported);
        } else {
            match self.seen {
                UltraSeen::Enter => st.on = Some(true),
                UltraSeen::Exit | UltraSeen::None => st.on = Some(false),
                UltraSeen::Compacted => st.unknown = Some(UltraUnknown::Compacted),
                UltraSeen::Unknown => st.unknown = Some(UltraUnknown::Reading),
                UltraSeen::Seed => {
                    st.unknown = Some(UltraUnknown::Reading);
                    st.seeded = true;
                }
            }
        }
        self.state = Some(st);
    }

    /// An `ultra_effort_enter` (`on`) or `ultra_effort_exit` of the main
    /// chain: the state as of the prompt it follows.
    fn marker(&mut self, on: bool, ts: Option<i64>, uuid: Option<String>) {
        let at = self.pending.take().and_then(|p| p.at).or(ts);
        self.seen = if on {
            UltraSeen::Enter
        } else {
            UltraSeen::Exit
        };
        self.marker_uuid = uuid;
        self.state = Some(UltraState {
            on: Some(on),
            at,
            marker: true,
            ..Default::default()
        });
    }

    /// A `compact_boundary` of the main chain. An automatic one hides the
    /// last marker unless it kept it; a manual one may or may not (an `exit`
    /// and no marker at all make Claude Code note the same, so only an
    /// `enter` becomes uncertain).
    fn compacted(&mut self, auto: bool, preserves: impl Fn(&str) -> bool) {
        self.settle();
        if self.marker_uuid.as_deref().is_some_and(preserves) {
            return;
        }
        if auto {
            self.seen = UltraSeen::None;
            self.marker_uuid = None;
        } else if matches!(self.seen, UltraSeen::Enter | UltraSeen::Seed) {
            self.seen = UltraSeen::Compacted;
        }
    }

    /// A tail window's replay still depends on the history before it.
    pub fn needs_seed(&self) -> bool {
        self.seen == UltraSeen::Seed || self.state.as_ref().is_none_or(|s| s.seeded)
    }

    /// Resolve a tail window's replay with `before`, the replay as of the
    /// window start ([`scan_back_ultra`], or a checkpoint).
    pub fn seed(&mut self, before: &UltraReplay) {
        if self.seen == UltraSeen::Seed {
            self.seen = before.seen;
            self.marker_uuid = before.marker_uuid.clone();
        }
        match &mut self.state {
            Some(st) if st.seeded => {
                st.seeded = false;
                (st.on, st.unknown) = match before.seen {
                    UltraSeen::Enter => (Some(true), None),
                    UltraSeen::Exit | UltraSeen::None => (Some(false), None),
                    UltraSeen::Compacted => (None, Some(UltraUnknown::Compacted)),
                    UltraSeen::Unknown | UltraSeen::Seed => (None, Some(UltraUnknown::Reading)),
                };
            }
            None => self.state = before.state.clone(),
            Some(_) => {}
        }
    }
}

/// The last request rejected by a plan limit (`quotaLimits`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaHit {
    /// The window that was hit (`five_hour`, `seven_day`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_type: Option<String>,
    /// Epoch ms the window resets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    /// `rejected` | `allowed_warning` | …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// When the rejection was recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

impl QuotaHit {
    fn from_rec(q: &QuotaRec, ts: Option<i64>) -> Self {
        let short = |s: Option<&str>| s.map(|s| clip_chars(s, 40)).filter(|s| !s.is_empty());
        QuotaHit {
            rate_limit_type: short(q.rate_limit_type.get()),
            resets_at: q
                .resets_at
                .0
                .filter(|s| *s > 0.0 && *s < 1e11)
                .map(|s| (s * 1000.0) as i64),
            status: short(q.status.get()),
            ts,
        }
    }

    /// One line for `lastError` when the rejected message carries no text.
    pub fn describe(&self) -> String {
        match self.rate_limit_type.as_deref() {
            Some("five_hour") => "Plan usage limit reached (5-hour window).".to_string(),
            Some(w) if w.starts_with("seven_day") => {
                "Plan usage limit reached (weekly window).".to_string()
            }
            _ => "Plan usage limit reached.".to_string(),
        }
    }
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
    /// The latest record's cwd (the shell may have moved during the session).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The first record's cwd (the directory the session was launched in).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_cwd: Option<String>,
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
    /// Effort of the last main-chain assistant record (`perTurnEffort ??
    /// effort`); `None` when that record carried none (a model without
    /// effort levels) or there is no such record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Timestamp of that record, set with or without an effort.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort_ts: Option<i64>,
    /// The last main-chain API message ran in fast mode (`usage.speed`).
    #[serde(skip_serializing_if = "is_false")]
    pub fast: bool,
    /// Ultracode as of the latest regular prompt (see [`UltraReplay`]).
    #[serde(skip_serializing_if = "UltraReplay::is_default")]
    pub ultra: UltraReplay,
    /// Timestamp of the last main-chain thinking block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_thinking_ts: Option<i64>,
    /// The last request a plan limit rejected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_quota: Option<QuotaHit>,
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

/// A regular prompt of the main chain: a message someone sent, after which
/// Claude Code notes ultracode ([`UltraReplay`]). Task notifications, relays
/// and wake-ups (`queueSkipAttachments`), `/loop` and cron fires (meta),
/// compaction summaries, local commands, interruptions and tool results
/// never make it check; prompts absorbed mid-turn have no record of their
/// own.
fn is_regular_prompt(rec: &Rec) -> bool {
    if !rec.ty.is("user")
        || rec.is_sidechain.yes()
        || rec.is_meta.yes()
        || rec.is_compact_summary.yes()
        || rec.queue_skip_attachments.yes()
        || rec.queue_transcript_only.yes()
        || !rec.origin.get().is_some_and(|o| o.kind.is("human"))
    {
        return false;
    }
    let Some(msg) = rec.message.get() else {
        return false;
    };
    let lead = match &msg.content {
        Content::Text(t) => t.get().unwrap_or(""),
        Content::Blocks(blocks) => {
            if blocks.iter().any(|b| b.ty.is("tool_result")) {
                return false;
            }
            blocks
                .iter()
                .filter(|b| b.ty.is("text"))
                .filter_map(|b| b.text.get())
                .find(|t| !t.trim().is_empty())
                .unwrap_or("")
        }
        Content::None => "",
    }
    .trim_start();
    !lead.starts_with("[Request interrupted")
        && !lead.starts_with("<command-")
        && !lead.starts_with("<local-command")
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
            if self.first_cwd.is_none() {
                self.first_cwd = Some(c.to_string());
            }
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
        if !rec.is_sidechain.yes() && matches!(ty.as_str(), "assistant" | "user" | "system") {
            // Past the attachments of a pending regular prompt (they follow
            // it directly): no marker among them confirms what Claude Code
            // saw. Settled before a compaction changes what it sees.
            self.ultra.settle();
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
                        // A content-less enqueue is a prompt whose content
                        // is not a plain string (e.g. with an image).
                        self.queue.push(QueuedPrompt {
                            notification: text.contains("<task-notification>"),
                            text: clip_chars(&text, TEXT_MAX),
                            ts,
                        });
                        if self.queue.len() > QUEUE_CAP {
                            // Stale notifications go first.
                            let i = self.queue.iter().position(|q| q.notification).unwrap_or(0);
                            self.queue.remove(i);
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
                            // `dequeue` never names what it takes. The CLI
                            // takes prompts typed while busy before task
                            // notifications, so take the oldest prompt, else
                            // the oldest notification (taking the head let a
                            // stale notification absorb a prompt's dequeue,
                            // leaving the prompt "queued" forever).
                            let i = self.queue.iter().position(|q| !q.notification).unwrap_or(0);
                            self.queue.remove(i);
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
        let main_chain = !rec.is_sidechain.yes();
        // The effort actually sent with the last real API request, as the CLI
        // itself rebuilds it (`perTurnEffort ?? effort`). A request without
        // one (a model with no effort levels, e.g. after `/model haiku`)
        // clears it: the session runs at no effort as of that record, which
        // the summary weighs against newer hook reports by `effort_ts`.
        // Subagents run at their own effort; placeholders and API errors
        // were never sent.
        if main_chain && !synthetic && !rec.is_api_error.yes() {
            self.effort = rec
                .per_turn_effort
                .get()
                .or(rec.effort.get())
                .filter(|e| !e.is_empty() && e.len() <= 20)
                .map(str::to_string);
            self.effort_ts = ts.or(self.last_ts);
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
            let quota = rec
                .quota_limits
                .present
                .then(|| match &rec.quota_limits.detail {
                    Some(q) => QuotaHit::from_rec(q, ts.or(self.last_ts)),
                    None => QuotaHit {
                        ts: ts.or(self.last_ts),
                        ..Default::default()
                    },
                });
            let kind = if quota.is_some() {
                "quota".to_string()
            } else {
                rec.error.0.clone().unwrap_or_else(|| "api_error".into())
            };
            let text = match &quota {
                Some(q) if text.trim().is_empty() => q.describe(),
                _ => text,
            };
            if quota.is_some() {
                self.last_quota = quota;
            }
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
        if main_chain {
            if let Some(u) = msg.usage.get() {
                self.fast = u.speed.is("fast");
            }
        }
        if let (Some(id), Some(u)) = (msg.id.get(), msg.usage.get()) {
            let totals = Totals::of(&model, u);
            let after = self.cost_state.is_some();
            self.usage.record(id, totals, ts, after);
            if main_chain {
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
                        if main_chain {
                            if let Some(t) = ts.or(self.last_ts) {
                                self.last_thinking_ts =
                                    Some(self.last_thinking_ts.map_or(t, |p| p.max(t)));
                            }
                        }
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
        if is_regular_prompt(&rec) {
            let version = rec.version.get().or(self.version.as_deref());
            self.ultra.prompt(ts.or(self.last_ts), version);
        }
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
        let main_chain = !rec.is_sidechain.yes();
        let Some(att) = rec.attachment.0 else { return };
        match att.ty.get() {
            // Standing ultracode (not the per-turn `ultrathink_effort` /
            // `workflow_keyword_request` keywords): see [`UltraReplay`].
            Some(kind @ ("ultra_effort_enter" | "ultra_effort_exit")) if main_chain => {
                self.ultra.marker(
                    kind == "ultra_effort_enter",
                    ts.or(self.last_ts),
                    rec.uuid.take(),
                );
            }
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
                if !rec.is_sidechain.yes() {
                    let meta = rec.compact_metadata.get();
                    self.ultra
                        .compacted(meta.is_some_and(CompactMeta::is_auto), |u| {
                            meta.is_some_and(|m| m.preserves(u))
                        });
                }
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

    /// The session's project directory: the cwd whose encoding names the
    /// `projects/<enc>` directory holding the transcript (where the CLI looks
    /// it up on `--resume`), else the first record's cwd. The latest cwd
    /// drifts when the shell moves during the session.
    pub fn project_cwd(&self, project_dir_name: Option<&str>) -> Option<&str> {
        let candidates = [self.first_cwd.as_deref(), self.cwd.as_deref()];
        project_dir_name
            .and_then(|enc| {
                candidates
                    .into_iter()
                    .flatten()
                    .find(|c| crate::claude::home::encode_cwd(c) == enc)
            })
            .or(self.first_cwd.as_deref())
            .or(self.cwd.as_deref())
    }

    /// A copy without the large lists (tasks, agent calls, workflow runs,
    /// files changed) and without the usage accumulator — what a summary
    /// needs, cheap enough to take on every refresh.
    pub fn light_clone(&self) -> Aggregates {
        Aggregates {
            cwd: self.cwd.clone(),
            first_cwd: self.first_cwd.clone(),
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
            effort_ts: self.effort_ts,
            fast: self.fast,
            ultra: self.ultra.clone(),
            last_thinking_ts: self.last_thinking_ts,
            last_quota: self.last_quota.clone(),
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
    /// [`Aggregates::project_cwd`] for this file's `projects/<enc>` parent
    /// (main transcripts).
    pub fn project_cwd(&self) -> Option<&str> {
        let enc = self
            .path
            .parent()
            .and_then(|d| d.file_name())
            .and_then(|n| n.to_str());
        self.agg.project_cwd(enc)
    }

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
        // What Claude Code saw before the window is not known yet.
        self.agg.ultra.seen = UltraSeen::Seed;
        Ok(())
    }

    /// Seed a main transcript's tail window with the ultracode history
    /// before it, so the dashboard is right at once rather than after the
    /// background full pass: a bounded backward scan from `window_start`
    /// (down to `base`, a checkpoint of this file, whose replay covers the
    /// rest). Nothing is read when the window alone decides it.
    pub fn seed_ultra(&mut self, window_start: u64, base: Option<&FileIndex>) {
        if self.kind != FileKind::Main || !self.agg.ultra.needs_seed() {
            return;
        }
        let floor = base.map_or(0, |b| b.offset.min(window_start));
        let before = match scan_back_ultra(&self.path, window_start, floor, ULTRA_SEED_BUDGET) {
            Ok(back) => back.replay(base.map(|b| &b.agg.ultra)),
            Err(e) => {
                tracing::debug!(error = %e, path = %self.path.display(), "ultracode seed scan failed");
                UltraReplay {
                    seen: UltraSeen::Unknown,
                    ..Default::default()
                }
            }
        };
        self.agg.ultra.seed(&before);
    }
}

/// How far back [`scan_back_ultra`] reads before giving up (the background
/// full pass settles the rest).
pub const ULTRA_SEED_BUDGET: u64 = 128 * 1024 * 1024;
/// Bytes read per step of the backward scan.
const SEED_CHUNK: u64 = 1024 * 1024;
/// How far a hit's line may reach before it (a prompt names its origin after
/// its text) and after it; a longer line is skipped.
const SEED_LINE_BACK: u64 = 1024 * 1024;
const SEED_LINE_AHEAD: u64 = 256 * 1024;
/// With their unescaped quotes these match JSON structure only, never text
/// quoted inside a string (where every `"` is escaped).
const MARKER_NEEDLE: &str = "\"type\":\"ultra_effort_";
const BOUNDARY_NEEDLE: &str = "\"subtype\":\"compact_boundary\"";
const PROMPT_NEEDLE: &str = "\"origin\":{\"kind\":\"human\"}";

/// Where a backward scan stopped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BackStop {
    /// At the last main-chain marker: `(on, ts, uuid)`.
    Marker(bool, Option<i64>, Option<String>),
    /// At the file start: no marker before.
    #[default]
    Start,
    /// At the floor (a checkpoint's offset) without a marker.
    Floor,
    /// Out of budget.
    Budget,
}

/// A record between the stop of a backward scan and its starting point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackEvent {
    /// A main-chain compaction: `(automatic, preserved uuids)`.
    Compaction(bool, Vec<String>),
    /// The last regular prompt: `(ts, version)`.
    Prompt(Option<i64>, Option<String>),
}

/// What [`scan_back_ultra`] found before a point of a transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UltraBack {
    pub stop: BackStop,
    /// The compactions and the last regular prompt after the stop, oldest
    /// first.
    pub events: Vec<BackEvent>,
}

impl UltraBack {
    /// The replay as of the scanned point; `base` is the checkpoint the
    /// scan stopped at (its pending prompt wrote no marker, or the scan
    /// would have stopped at it).
    pub fn replay(&self, base: Option<&UltraReplay>) -> UltraReplay {
        let mut r = match (&self.stop, base) {
            (BackStop::Marker(on, ts, uuid), _) => {
                let mut r = UltraReplay::default();
                r.marker(*on, *ts, uuid.clone());
                r
            }
            (BackStop::Start, _) => UltraReplay::default(),
            (BackStop::Floor, Some(b)) => b.clone(),
            (BackStop::Floor | BackStop::Budget, _) => UltraReplay {
                seen: UltraSeen::Unknown,
                ..Default::default()
            },
        };
        r.settle();
        for e in &self.events {
            match e {
                BackEvent::Compaction(auto, kept) => {
                    r.compacted(*auto, |u| kept.iter().any(|k| k == u))
                }
                // Anything later than the prompt is past its attachments.
                BackEvent::Prompt(at, version) => r.prompt(*at, version.as_deref()),
            }
        }
        r.settle();
        r
    }
}

/// Scan a transcript backwards from `end` (a line start) for its last
/// main-chain ultracode marker, collecting the compactions and the last
/// regular prompt after it; stops at `floor` or after `budget` bytes. Reads
/// 1 MB at a time, plus a bounded window around each hit's line.
pub fn scan_back_ultra(
    path: &Path,
    end: u64,
    floor: u64,
    budget: u64,
) -> std::io::Result<UltraBack> {
    const NEEDLES: [&str; 3] = [MARKER_NEEDLE, BOUNDARY_NEEDLE, PROMPT_NEEDLE];
    let mut f = File::open(path)?;
    let size = f.metadata()?.len();
    let end = end.min(size);
    let overlap = NEEDLES.iter().map(|n| n.len()).max().unwrap_or(1) as u64 - 1;
    let mut back = UltraBack::default();
    let mut buf: Vec<u8> = Vec::new();
    let mut line: Vec<u8> = Vec::new();
    let mut prompt_seen = false;
    // Every needle starting at `pos` or later was searched already.
    let mut pos = end;
    back.stop = loop {
        if pos <= floor {
            break if floor == 0 {
                BackStop::Start
            } else {
                BackStop::Floor
            };
        }
        if end - pos >= budget {
            break BackStop::Budget;
        }
        let from = pos.saturating_sub(SEED_CHUNK).max(floor);
        let to = (pos + overlap).min(end);
        read_range(&mut f, from, to, &mut buf)?;
        let mut hits: Vec<u64> = Vec::new();
        let mut at = 0usize;
        for chunk in buf.utf8_chunks() {
            let text = chunk.valid();
            for needle in NEEDLES {
                if needle == PROMPT_NEEDLE && prompt_seen {
                    continue;
                }
                hits.extend(
                    text.match_indices(needle)
                        .map(|(i, _)| from + (at + i) as u64)
                        .filter(|h| *h < pos),
                );
            }
            at += text.len() + chunk.invalid().len();
        }
        hits.sort_unstable_by(|a, b| b.cmp(a));
        for hit in hits {
            let Some(rec) = line_at(&mut f, hit, size, &mut line)? else {
                continue;
            };
            if rec.is_sidechain.yes() {
                continue;
            }
            match (rec.ty.get(), rec.subtype.get()) {
                (Some("attachment"), _) => {
                    let kind = rec.attachment.get().and_then(|a| a.ty.get());
                    if let Some(k @ ("ultra_effort_enter" | "ultra_effort_exit")) = kind {
                        back.events.reverse();
                        back.stop = BackStop::Marker(
                            k == "ultra_effort_enter",
                            parse_ts(rec.timestamp.get()),
                            rec.uuid.0.clone(),
                        );
                        return Ok(back);
                    }
                }
                (Some("system"), Some("compact_boundary")) => {
                    let meta = rec.compact_metadata.get();
                    let kept = meta
                        .and_then(|m| m.preserved_messages.get())
                        .map(|p| {
                            p.uuids
                                .0
                                .iter()
                                .chain(p.all_uuids.0.iter())
                                .filter_map(|u| u.get().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    back.events.push(BackEvent::Compaction(
                        meta.is_some_and(CompactMeta::is_auto),
                        kept,
                    ));
                }
                (Some("user"), _) if !prompt_seen && is_regular_prompt(&rec) => {
                    prompt_seen = true;
                    back.events.push(BackEvent::Prompt(
                        parse_ts(rec.timestamp.get()),
                        rec.version.0.clone(),
                    ));
                }
                _ => {}
            }
        }
        pos = from;
    };
    back.events.reverse();
    Ok(back)
}

/// Read `[from, to)` of `f` into `buf`.
fn read_range(f: &mut File, from: u64, to: u64, buf: &mut Vec<u8>) -> std::io::Result<()> {
    use std::io::Read;
    buf.clear();
    buf.resize(to.saturating_sub(from) as usize, 0);
    f.seek(SeekFrom::Start(from))?;
    f.read_exact(buf)
}

/// The record on the line holding byte `hit`, when that line starts at most
/// [`SEED_LINE_BACK`] before it and ends at most [`SEED_LINE_AHEAD`] after.
fn line_at(f: &mut File, hit: u64, size: u64, buf: &mut Vec<u8>) -> std::io::Result<Option<Rec>> {
    let lo = hit.saturating_sub(SEED_LINE_BACK);
    let hi = (hit + SEED_LINE_AHEAD).min(size);
    read_range(f, lo, hi, buf)?;
    let at = (hit - lo) as usize;
    let start = match buf[..at].iter().rposition(|b| *b == b'\n') {
        Some(i) => i + 1,
        None if lo == 0 => 0,
        None => return Ok(None),
    };
    let Some(len) = buf[at..].iter().position(|b| *b == b'\n') else {
        return Ok(None);
    };
    Ok(parse_line::<Rec>(&buf[start..at + len]))
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

    /// One priced message costing a dollar.
    fn dollar() -> Totals {
        Totals {
            cost: 1.0,
            priced: 1,
            messages: 1,
            ..Totals::default()
        }
    }

    #[test]
    fn project_cwd_ignores_a_drifted_shell_cwd() {
        let rec = |cwd: &str| {
            format!(r#"{{"type":"user","cwd":"{cwd}","message":{{"content":"x"}}}}"#) + "\n"
        };
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-w-zira-zira-client");
        std::fs::create_dir_all(&proj).unwrap();
        let path = proj.join("s.jsonl");
        std::fs::write(
            &path,
            rec("/w/zira/zira-client") + &rec("/w/zira/zira-server") + &rec("/w/zira/zira-server"),
        )
        .unwrap();
        let mut fi = FileIndex::new(path, FileKind::Main, "s");
        fi.scan(None).unwrap();
        assert_eq!(fi.agg.cwd.as_deref(), Some("/w/zira/zira-server"));
        assert_eq!(fi.agg.first_cwd.as_deref(), Some("/w/zira/zira-client"));
        assert_eq!(fi.project_cwd(), Some("/w/zira/zira-client"));
        // The latest cwd wins when it is the one naming the project dir.
        let a = Aggregates {
            first_cwd: Some("/a".into()),
            cwd: Some("/b".into()),
            ..Default::default()
        };
        assert_eq!(a.project_cwd(Some("-b")), Some("/b"));
        // No match (or no dir): the first cwd.
        assert_eq!(a.project_cwd(Some("-c")), Some("/a"));
        assert_eq!(a.project_cwd(None), Some("/a"));
    }

    #[test]
    fn cost_after_a_cutoff_past_every_message_is_zero() {
        // Enough messages that the curve is thinned and its last sample
        // trails the folded total.
        for n in [17_i64, 250] {
            let mut u = UsageAcc::default();
            for i in 1..=n {
                u.record(&format!("m{i}"), dollar(), Some(i * 1000), false);
            }
            u.compact();
            assert_eq!(u.total().messages, n as u64);
            // Only the newest (open) message is at n*1000.
            assert_eq!(u.cost_after(n * 1000), 0.0, "n={n}");
            assert_eq!(u.cost_after(n * 1000 + 1), 0.0, "n={n}");
            // Everything is after a cutoff before the first message.
            assert!((u.cost_after(0) - n as f64).abs() < 1e-9, "n={n}");
            // The folded messages all precede the open one.
            assert!((u.cost_after(n * 1000 - 1) - 1.0).abs() < 1e-9, "n={n}");
            // A checkpoint round trip keeps the answer.
            let back: UsageAcc = serde_json::from_str(&serde_json::to_string(&u).unwrap()).unwrap();
            assert_eq!(back.cost_after(n * 1000), 0.0, "n={n}");
        }
    }

    #[test]
    fn cost_after_ignores_out_of_order_and_counts_untimestamped_tail() {
        let mut u = UsageAcc::default();
        u.record("a", dollar(), Some(5_000), false);
        u.record("b", dollar(), Some(3_000), false);
        u.close_all();
        // Newest folded ts is 5000, whatever the fold order.
        assert_eq!(u.cost_after(5_000), 0.0);
        // An untimestamped message folded later is "after" any cutoff.
        u.record("c", dollar(), None, false);
        u.close_all();
        assert!((u.cost_after(10_000) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cost_since_counts_only_messages_from_that_point() {
        const H: i64 = 60 * 60 * 1000;
        let midnight = 1_790_294_400_000_i64; // a quarter-hour boundary
        assert_eq!(midnight % COST_BUCKET_MS, 0);
        let mut u = UsageAcc::default();
        // Yesterday: 300 messages across the evening (thins the curve).
        for i in 0..300 {
            u.record(
                &format!("y{i}"),
                dollar(),
                Some(midnight - 5 * H + i * 60_000),
                false,
            );
        }
        // One just before midnight, in the same quarter hour.
        u.record("late", dollar(), Some(midnight - 1), false);
        // Today: 7 messages.
        for i in 0..7 {
            u.record(
                &format!("t{i}"),
                dollar(),
                Some(midnight + H + i * 60_000),
                false,
            );
        }
        assert_eq!(u.total().messages, 308);
        assert!((u.cost_since(midnight) - 7.0).abs() < 1e-9);
        u.compact();
        assert!((u.cost_since(midnight) - 7.0).abs() < 1e-9);
        // Nothing yet today.
        assert_eq!(u.cost_since(midnight + 2 * H), 0.0);
        // Buckets stay bounded to about a day.
        assert!(u
            .recent
            .iter()
            .all(|(b, _)| *b >= midnight + H - RECENT_KEEP_MS));
        // A history checkpoint drops them; with none left nothing is "today".
        let mut old = u.clone();
        old.close_all();
        old.forget_recent_before(midnight + 3 * H);
        assert!(old.recent.is_empty());
        assert_eq!(old.cost_since(midnight + 3 * H), 0.0);
    }

    #[test]
    fn old_checkpoints_without_last_fold_still_load() {
        let u: UsageAcc =
            serde_json::from_str(r#"{"folded":{"cost":2.0,"messages":2},"curve":[[1,1.0]]}"#)
                .unwrap();
        assert!(u.last_fold.is_none());
        assert!((u.cost_after(1) - 1.0).abs() < 1e-9);
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
            r#"{"type":"assistant","timestamp":"2026-09-25T10:00:03.000Z","effort":"xhigh","message":{"id":"m2","model":"claude-opus-5","stop_reason":"tool_use","content":[{"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"/p/a.rs","old_string":"x","new_string":"y"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
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

    fn at(sec: u32) -> Option<i64> {
        parse_ts(Some(&format!("2026-09-25T10:00:{sec:02}.000Z")))
    }

    #[test]
    fn effort_ultracode_thinking_and_speed_evidence() {
        let lines = concat!(
            // perTurnEffort wins over effort (the CLI rebuilds history so).
            r#"{"type":"assistant","timestamp":"2026-09-25T10:00:01.000Z","effort":"xhigh","perTurnEffort":"max","message":{"id":"m1","model":"claude-opus-5","content":[{"type":"thinking","thinking":"","signature":"s"},{"type":"text","text":"hi"}],"usage":{"input_tokens":1,"output_tokens":1,"speed":"fast","output_tokens_details":{"thinking_tokens":5}}}}"#,
            "\n",
            // A subagent's record says nothing about the session.
            r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-09-25T10:00:02.000Z","effort":"low","message":{"id":"s1","model":"claude-haiku-4-5","content":[{"type":"thinking","thinking":"x"}],"usage":{"input_tokens":1,"output_tokens":1,"speed":"standard"}}}"#,
            "\n",
            r#"{"type":"attachment","timestamp":"2026-09-25T10:00:03.000Z","attachment":{"type":"ultra_effort_enter","reminderType":"full"}}"#,
            "\n",
            // Per-turn keywords are not the standing setting.
            r#"{"type":"attachment","timestamp":"2026-09-25T10:00:04.000Z","attachment":{"type":"ultrathink_effort"}}"#,
            "\n",
            r#"{"type":"attachment","timestamp":"2026-09-25T10:00:05.000Z","attachment":{"type":"workflow_keyword_request"}}"#,
            "\n",
            r#"{"type":"attachment","isSidechain":true,"timestamp":"2026-09-25T10:00:06.000Z","attachment":{"type":"ultra_effort_exit"}}"#,
            "\n",
        );
        let fi = scan_str(lines);
        let a = &fi.agg;
        assert_eq!(a.effort.as_deref(), Some("max"));
        assert_eq!(a.effort_ts, at(1));
        assert!(a.fast);
        // A marker with no prompt before it (in this excerpt) holds as of
        // its own time.
        assert_eq!(a.ultra.seen, UltraSeen::Enter);
        assert_eq!(a.ultra.state, Some(marked(true, at(3))));
        assert_eq!(a.last_thinking_ts, at(1), "main chain only");
        // The light copy and a checkpoint keep all of it.
        let light = a.light_clone();
        assert_eq!(
            (
                &light.effort,
                light.effort_ts,
                light.fast,
                &light.ultra,
                light.last_thinking_ts
            ),
            (&a.effort, a.effort_ts, a.fast, &a.ultra, a.last_thinking_ts)
        );
        let back: Aggregates = serde_json::from_str(&serde_json::to_string(a).unwrap()).unwrap();
        assert_eq!(&back, a);

        // A null perTurnEffort (2.1.267–2.1.280) falls back to effort; the
        // exit marker turns it off; standard speed clears fast.
        let more = String::from(lines)
            + r#"{"type":"assistant","timestamp":"2026-09-25T10:00:07.000Z","effort":"high","perTurnEffort":null,"message":{"id":"m2","model":"claude-opus-5","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1,"speed":"standard"}}}"#
            + "\n"
            + r#"{"type":"attachment","timestamp":"2026-09-25T10:00:08.000Z","attachment":{"type":"ultra_effort_exit"}}"#
            + "\n";
        let fi = scan_str(&more);
        assert_eq!(fi.agg.effort.as_deref(), Some("high"));
        assert_eq!(fi.agg.effort_ts, at(7));
        assert!(!fi.agg.fast);
        assert_eq!(fi.agg.ultra.state, Some(marked(false, at(8))));
        assert_eq!(fi.agg.last_thinking_ts, at(1));
        // A plan-limit rejection and a placeholder were never sent: the
        // effort of the last real request stays.
        let rejected = more.clone()
            + r#"{"type":"assistant","timestamp":"2026-09-25T10:00:09.000Z","isApiErrorMessage":true,"error":"rate_limit","message":{"id":"e1","model":"<synthetic>","content":[{"type":"text","text":"limit"}]}}"#
            + "\n";
        let fi = scan_str(&rejected);
        assert_eq!(fi.agg.effort.as_deref(), Some("high"));
        assert_eq!(fi.agg.effort_ts, at(7));
        // A request without effort (`/model haiku`: the model has none)
        // clears it, stamped, so an older hook report cannot bring it back.
        let haiku = more
            + r#"{"type":"assistant","timestamp":"2026-09-25T10:00:09.000Z","message":{"id":"m3","model":"claude-haiku-4-5","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#
            + "\n";
        let fi = scan_str(&haiku);
        assert_eq!(fi.agg.effort, None);
        assert_eq!(fi.agg.effort_ts, at(9));
        // A subagent's record never touches it.
        let sub = haiku
            + r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-09-25T10:00:10.000Z","effort":"max","message":{"id":"s2","model":"claude-opus-5","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#
            + "\n";
        let fi = scan_str(&sub);
        assert_eq!(fi.agg.effort, None);
        assert_eq!(fi.agg.effort_ts, at(9));
    }

    // ── ultracode replay: record shapes as Claude Code 2.1.284 writes them ──

    fn ts(sec: u32) -> String {
        format!("2026-09-25T10:{:02}:{:02}.000Z", sec / 60, sec % 60)
    }

    fn secs(sec: u32) -> Option<i64> {
        parse_ts(Some(&ts(sec)))
    }

    /// A message someone sent (VS Code: `promptSource` sdk).
    fn prompt(sec: u32, version: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"{}","version":"{version}","origin":{{"kind":"human"}},"promptSource":"sdk","message":{{"role":"user","content":"go on"}}}}"#,
            ts(sec)
        ) + "\n"
    }

    /// `full` | `sparse` (an `ultra_effort_enter`) or `exit`.
    fn ultra(sec: u32, kind: &str, uuid: &str) -> String {
        let body = match kind {
            "exit" => r#""type":"ultra_effort_exit""#.to_string(),
            r => format!(r#""type":"ultra_effort_enter","reminderType":"{r}""#),
        };
        format!(
            r#"{{"type":"attachment","uuid":"{uuid}","timestamp":"{}","attachment":{{{body}}}}}"#,
            ts(sec)
        ) + "\n"
    }

    /// Another attachment of the prompt's block.
    fn other_attachment(sec: u32) -> String {
        format!(
            r#"{{"type":"attachment","timestamp":"{}","attachment":{{"type":"date"}}}}"#,
            ts(sec)
        ) + "\n"
    }

    fn reply(sec: u32) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{}","message":{{"id":"r{sec}","model":"claude-opus-5-5","content":[{{"type":"text","text":"ok"}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#,
            ts(sec)
        ) + "\n"
    }

    fn boundary(sec: u32, trigger: &str, kept: &str) -> String {
        format!(
            r#"{{"type":"system","subtype":"compact_boundary","timestamp":"{}","compactMetadata":{{"trigger":"{trigger}","preTokens":1,"preservedSegment":{{"headUuid":"h","anchorUuid":"a","tailUuid":"t"}},"preservedMessages":{{"anchorUuid":"a","uuids":["{kept}"],"allUuids":["{kept}","z"]}}}}}}"#,
            ts(sec)
        ) + "\n"
    }

    fn marked(on: bool, at: Option<i64>) -> UltraState {
        UltraState {
            on: Some(on),
            at,
            marker: true,
            ..Default::default()
        }
    }

    fn confirmed(on: bool, at: Option<i64>) -> UltraState {
        UltraState {
            on: Some(on),
            at,
            ..Default::default()
        }
    }

    fn unknown(why: UltraUnknown, at: Option<i64>) -> UltraState {
        UltraState {
            at,
            unknown: Some(why),
            ..Default::default()
        }
    }

    const V: &str = "2.1.284";

    #[test]
    fn ultracode_holds_as_of_the_last_regular_prompt() {
        // Switched on: the prompt gets a full `enter`, dated to the prompt.
        let on = prompt(1, V) + &other_attachment(1) + &ultra(1, "full", "u1") + &reply(2);
        let fi = scan_str(&on);
        assert_eq!(fi.agg.ultra.state, Some(marked(true, secs(1))));
        assert_eq!(fi.agg.ultra.seen, UltraSeen::Enter);
        // Later prompts without a marker (fewer than ~10 messages since the
        // `enter`) confirm it — also after a restart with the switch turned
        // back on, where Claude Code sees the `enter` of its history.
        let still = on.clone() + &prompt(60, V) + &other_attachment(60) + &reply(61);
        assert_eq!(
            scan_str(&still).agg.ultra.state,
            Some(confirmed(true, secs(60)))
        );
        // A sparse reminder says the same.
        let sparse = still.clone() + &prompt(90, V) + &ultra(90, "sparse", "u2") + &reply(91);
        assert_eq!(
            scan_str(&sparse).agg.ultra.state,
            Some(marked(true, secs(90)))
        );
        // Switched off (or a restart left it off): the next prompt gets an
        // `exit`; later prompts confirm off.
        let off = still.clone() + &prompt(120, V) + &ultra(120, "exit", "u3") + &reply(121);
        assert_eq!(
            scan_str(&off).agg.ultra.state,
            Some(marked(false, secs(120)))
        );
        let off = off + &prompt(150, V) + &reply(151);
        assert_eq!(
            scan_str(&off).agg.ultra.state,
            Some(confirmed(false, secs(150)))
        );
        // Never switched on: every prompt confirms off.
        let never = prompt(1, V) + &reply(2) + &prompt(3, V) + &reply(4);
        assert_eq!(
            scan_str(&never).agg.ultra.state,
            Some(confirmed(false, secs(3)))
        );
        // A prompt whose attachments are not all written yet says nothing
        // until something follows them.
        let trailing = still.clone() + &prompt(200, V);
        let fi = scan_str(&trailing);
        assert_eq!(fi.agg.ultra.state, Some(confirmed(true, secs(60))));
        assert!(fi.agg.ultra.pending.is_some());
        // …and a checkpoint keeps it pending.
        let back: Aggregates =
            serde_json::from_str(&serde_json::to_string(&fi.agg).unwrap()).unwrap();
        assert_eq!(back.ultra, fi.agg.ultra);
        // No prompt at all: nothing known.
        assert_eq!(scan_str(&reply(1)).agg.ultra.state, None);
    }

    #[test]
    fn only_regular_prompts_make_claude_code_note_ultracode() {
        let on = prompt(1, V) + &ultra(1, "full", "u1") + &reply(2);
        let not_prompts = [
            // A background task's notification (also Mother Claude's
            // wake-ups): delivered without the per-prompt attachments.
            r#"{"type":"user","timestamp":"T","version":"2.1.284","origin":{"kind":"task-notification","producer":"session-task"},"promptSource":"system","queueSkipAttachments":true,"message":{"role":"user","content":"<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>\n</task-notification>"}}"#,
            // A `/loop` or cron fire.
            r#"{"type":"user","timestamp":"T","version":"2.1.284","isMeta":true,"origin":{"kind":"human"},"message":{"role":"user","content":"check the build"}}"#,
            // A compaction's summary.
            r#"{"type":"user","timestamp":"T","version":"2.1.284","isCompactSummary":true,"message":{"role":"user","content":"This session is being continued"}}"#,
            // A local slash command and its output.
            r#"{"type":"user","timestamp":"T","version":"2.1.284","origin":{"kind":"human"},"message":{"role":"user","content":"<command-name>/context</command-name>"}}"#,
            r#"{"type":"user","timestamp":"T","version":"2.1.284","message":{"role":"user","content":"<local-command-stdout>ok</local-command-stdout>"}}"#,
            // An interruption.
            r#"{"type":"user","timestamp":"T","version":"2.1.284","origin":{"kind":"human"},"message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            // A tool result.
            r#"{"type":"user","timestamp":"T","version":"2.1.284","origin":{"kind":"human"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
            // A subagent's prompt, and its markers.
            r#"{"type":"user","isSidechain":true,"timestamp":"T","version":"2.1.284","origin":{"kind":"human"},"message":{"role":"user","content":"explore"}}"#,
            r#"{"type":"attachment","isSidechain":true,"timestamp":"T","attachment":{"type":"ultra_effort_exit"}}"#,
        ];
        for (i, line) in not_prompts.iter().enumerate() {
            let t = 10 + 2 * i as u32;
            let text = on.clone()
                + &line.replace("\"T\"", &format!("\"{}\"", ts(t)))
                + "\n"
                + &reply(t + 1);
            let a = scan_str(&text).agg;
            assert_eq!(a.ultra.state, Some(marked(true, secs(1))), "{line}");
            assert_eq!(a.ultra.seen, UltraSeen::Enter, "{line}");
        }
    }

    #[test]
    fn compactions_hide_or_keep_the_marker() {
        let on = prompt(1, V) + &ultra(1, "full", "u1") + &reply(2);
        // An automatic compaction hides it: on, the next prompt writes a
        // full `enter` again; off, it writes nothing, which now means off.
        let auto = on.clone() + &boundary(3, "auto", "other");
        assert_eq!(scan_str(&auto).agg.ultra.seen, UltraSeen::None);
        let again = auto.clone() + &prompt(4, V) + &ultra(4, "full", "u2") + &reply(5);
        assert_eq!(
            scan_str(&again).agg.ultra.state,
            Some(marked(true, secs(4)))
        );
        let silent = auto + &prompt(4, V) + &reply(5);
        assert_eq!(
            scan_str(&silent).agg.ultra.state,
            Some(confirmed(false, secs(4)))
        );
        // Unless it kept the marker.
        let kept = on.clone() + &boundary(3, "auto", "u1") + &prompt(4, V) + &reply(5);
        assert_eq!(
            scan_str(&kept).agg.ultra.state,
            Some(confirmed(true, secs(4)))
        );
        // A manual `/compact` may keep it or not: a silent prompt can't tell,
        // the next marker can.
        let manual = on.clone() + &boundary(3, "manual", "other") + &prompt(4, V) + &reply(5);
        let a = scan_str(&manual).agg;
        assert_eq!(a.ultra.seen, UltraSeen::Compacted);
        assert_eq!(
            a.ultra.state,
            Some(unknown(UltraUnknown::Compacted, secs(4)))
        );
        let told = manual + &prompt(6, V) + &ultra(6, "sparse", "u3") + &reply(7);
        assert_eq!(scan_str(&told).agg.ultra.state, Some(marked(true, secs(6))));
        // After an `exit` both ways end the same: off.
        let off = prompt(1, V) + &ultra(1, "exit", "u1") + &reply(2);
        let manual_off = off + &boundary(3, "manual", "x") + &prompt(4, V) + &reply(5);
        assert_eq!(
            scan_str(&manual_off).agg.ultra.state,
            Some(confirmed(false, secs(4)))
        );
        // A compaction right after a prompt settles that prompt first, with
        // what Claude Code saw before it.
        let settled = on + &prompt(8, V) + &boundary(9, "auto", "x");
        assert_eq!(
            scan_str(&settled).agg.ultra.state,
            Some(confirmed(true, secs(8)))
        );
    }

    #[test]
    fn versions_before_ultracode_markers_say_nothing_by_silence() {
        let old = prompt(1, "2.1.240") + &reply(2);
        assert_eq!(
            scan_str(&old).agg.ultra.state,
            Some(unknown(UltraUnknown::Unsupported, secs(1)))
        );
        // A marker always tells.
        let marked_old = prompt(1, "2.1.245") + &ultra(1, "full", "u1") + &reply(2);
        assert_eq!(
            scan_str(&marked_old).agg.ultra.state,
            Some(marked(true, secs(1)))
        );
        assert!(notes_ultracode("2.1.246") && notes_ultracode("2.2.0"));
        assert!(!notes_ultracode("2.1.245") && !notes_ultracode("next"));
        assert_eq!(version_triple("2.1.246"), Some((2, 1, 246)));
        assert_eq!(version_triple("2.1.284-rc.1"), Some((2, 1, 284)));
        assert_eq!(version_triple("2.1"), None);
        assert_eq!(version_triple("x"), None);
    }

    /// A filler line of `len` bytes (at least ~300): an assistant message
    /// whose text quotes a marker and a boundary (escaped, as JSON text is),
    /// which are not ones.
    fn filler(sec: u32, len: usize) -> String {
        let quoted = r#"see \"type\":\"ultra_effort_exit\" and \"subtype\":\"compact_boundary\" "#;
        let line = |pad: usize| {
            format!(
                r#"{{"type":"assistant","timestamp":"{}","message":{{"id":"f{sec}","model":"claude-opus-5-5","content":[{{"type":"text","text":"{quoted}{}"}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#,
                ts(sec),
                "x".repeat(pad)
            ) + "\n"
        };
        line(len.saturating_sub(line(0).len()))
    }

    fn write(dir: &Path, text: &str) -> PathBuf {
        let path = dir.join("s.jsonl");
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Index `text` from a tail window of `window` bytes, seeded.
    fn window_scan(dir: &Path, text: &str, window: u64, base: Option<&FileIndex>) -> FileIndex {
        let path = write(dir, text);
        let mut fi = FileIndex::new(path, FileKind::Main, "s");
        fi.seek_window(window).unwrap();
        let start = fi.offset;
        fi.scan(None).unwrap();
        assert!(!fi.is_complete());
        fi.seed_ultra(start, base);
        fi
    }

    #[test]
    fn tail_windows_are_seeded_with_the_marker_before_them() {
        let dir = tempfile::tempdir().unwrap();
        let head = prompt(1, V) + &ultra(1, "full", "u1") + &reply(2);
        let fill: String = (0..40).map(|i| filler(3 + i, 2_000)).collect();
        let tail = prompt(100, V) + &other_attachment(100) + &reply(101);
        let text = head.clone() + &fill + &tail;
        // Unseeded, the window can't tell…
        let path = write(dir.path(), &text);
        let mut fi = FileIndex::new(path, FileKind::Main, "s");
        fi.seek_window(1_000).unwrap();
        fi.scan(None).unwrap();
        assert!(fi.agg.ultra.needs_seed());
        assert_eq!(
            fi.agg.ultra.state.as_ref().map(|s| (s.on, s.unknown)),
            Some((None, Some(UltraUnknown::Reading)))
        );
        // …seeded, the `enter` far before it confirms on as of the prompt in
        // the window, as the full pass says. The quoted markers in between
        // are text, not markers.
        let fi = window_scan(dir.path(), &text, 1_000, None);
        assert_eq!(fi.agg.ultra.state, Some(confirmed(true, secs(100))));
        assert_eq!(fi.agg.ultra.seen, UltraSeen::Enter);
        assert_eq!(fi.agg.ultra.marker_uuid.as_deref(), Some("u1"));
        assert_eq!(scan_str(&text).agg.ultra, fi.agg.ultra);
        // An automatic compaction in between (before the window too) hid it:
        // off.
        let hidden = head.clone() + &fill + &boundary(90, "auto", "x") + &filler(95, 2_000) + &tail;
        let fi = window_scan(dir.path(), &hidden, 1_000, None);
        assert_eq!(fi.agg.ultra.state, Some(confirmed(false, secs(100))));
        assert_eq!(scan_str(&hidden).agg.ultra, fi.agg.ultra);
        // One that kept it: on.
        let kept = head.clone() + &fill + &boundary(90, "auto", "u1") + &filler(95, 2_000) + &tail;
        let fi = window_scan(dir.path(), &kept, 1_000, None);
        assert_eq!(fi.agg.ultra.state, Some(confirmed(true, secs(100))));
        // No marker anywhere before: off.
        let never = prompt(1, V) + &reply(2) + &fill + &tail;
        let fi = window_scan(dir.path(), &never, 1_000, None);
        assert_eq!(fi.agg.ultra.state, Some(confirmed(false, secs(100))));
        // No prompt in the window: the state as of the marker before it…
        let quiet = head.clone() + &fill;
        let fi = window_scan(dir.path(), &quiet, 1_000, None);
        assert_eq!(fi.agg.ultra.state, Some(marked(true, secs(1))));
        // …or as of the last prompt before the window, which confirmed it (a
        // cron fire after it is no prompt).
        let cron = format!(
            r#"{{"type":"user","timestamp":"{}","version":"{V}","isMeta":true,"origin":{{"kind":"human"}},"message":{{"role":"user","content":"check"}}}}"#,
            ts(52)
        ) + "\n";
        let quiet = head.clone() + &filler(3, 2_000) + &prompt(50, V) + &reply(51) + &cron + &fill;
        let fi = window_scan(dir.path(), &quiet, 1_000, None);
        assert_eq!(fi.agg.ultra.state, Some(confirmed(true, secs(50))));
        assert_eq!(scan_str(&quiet).agg.ultra, fi.agg.ultra);
        // The window decides alone: nothing is read before it.
        let decided = head + &fill + &prompt(100, V) + &ultra(100, "exit", "u9") + &reply(101);
        let fi = window_scan(dir.path(), &decided, 1_000, None);
        assert!(!fi.agg.ultra.needs_seed());
        assert_eq!(fi.agg.ultra.state, Some(marked(false, secs(100))));
    }

    #[test]
    fn the_backward_scan_is_bounded_and_finds_markers_across_reads() {
        let dir = tempfile::tempdir().unwrap();
        let head = prompt(1, V) + &reply(2);
        let mark = ultra(3, "full", "u1");
        let needle_in_line = mark.find(MARKER_NEEDLE).unwrap();
        // The needle straddles the start of the first 1 MB read back from
        // `end`: found by the next read's overlap.
        let fill_len = needle_in_line + SEED_CHUNK as usize + 5 - mark.len();
        let text = head.clone() + &mark + &filler(4, fill_len);
        assert_eq!(text.len(), head.len() + mark.len() + fill_len);
        let end = text.len() as u64;
        let path = write(dir.path(), &text);
        let hit = (head.len() + needle_in_line) as u64;
        assert!(hit < end - SEED_CHUNK && hit + MARKER_NEEDLE.len() as u64 > end - SEED_CHUNK);
        let back = scan_back_ultra(&path, end, 0, ULTRA_SEED_BUDGET).unwrap();
        assert_eq!(
            back.stop,
            BackStop::Marker(true, secs(3), Some("u1".into()))
        );
        // Out of budget: unknown, never "off".
        let back = scan_back_ultra(&path, end, 0, 64 * 1024).unwrap();
        assert_eq!(back.stop, BackStop::Budget);
        assert_eq!(back.replay(None).seen, UltraSeen::Unknown);
        // A floor (a checkpoint's end) with nothing after it: the
        // checkpoint's replay, its pending prompt settled.
        let back =
            scan_back_ultra(&path, end, head.len() as u64 + mark.len() as u64, u64::MAX).unwrap();
        assert_eq!(back.stop, BackStop::Floor);
        let mut base = UltraReplay {
            seen: UltraSeen::Enter,
            ..Default::default()
        };
        base.prompt(secs(50), Some(V));
        let r = back.replay(Some(&base));
        assert_eq!(r.state, Some(confirmed(true, secs(50))));
    }

    #[test]
    fn tail_windows_seed_from_a_checkpoint_far_behind() {
        let dir = tempfile::tempdir().unwrap();
        let first = prompt(1, V) + &ultra(1, "full", "u1") + &reply(2);
        let path = write(dir.path(), &first);
        let mut base = FileIndex::new(path, FileKind::Main, "s");
        base.scan(None).unwrap();
        assert!(base.is_complete());
        // Grown since (no marker in the gap), then a prompt in the window.
        let fill: String = (0..20).map(|i| filler(3 + i, 2_000)).collect();
        let text = first + &fill + &prompt(100, V) + &reply(101);
        let fi = window_scan(dir.path(), &text, 1_000, Some(&base));
        assert_eq!(fi.agg.ultra.state, Some(confirmed(true, secs(100))));
    }

    #[test]
    fn plan_limit_rejections_keep_their_detail() {
        let reject = |text: &str, quota: &str| {
            format!(
                r#"{{"type":"assistant","timestamp":"2026-09-25T10:00:10.000Z","isApiErrorMessage":true,"error":"rate_limit","apiErrorStatus":429,"quotaLimits":{quota},"message":{{"id":"e1","model":"<synthetic>","content":[{{"type":"text","text":"{text}"}}]}}}}"#
            ) + "\n"
        };
        let fi = scan_str(&reject(
            "",
            r#"{"status":"rejected","resetsAt":1790010000,"rateLimitType":"five_hour","overageStatus":"rejected","upgradePaths":["upgrade_plan"]}"#,
        ));
        let e = fi.agg.last_error.as_ref().unwrap();
        assert_eq!(e.kind, "quota");
        assert_eq!(e.text, "Plan usage limit reached (5-hour window).");
        let q = fi.agg.last_quota.as_ref().unwrap();
        assert_eq!(q.rate_limit_type.as_deref(), Some("five_hour"));
        assert_eq!(q.resets_at, Some(1_790_010_000_000));
        assert_eq!(q.status.as_deref(), Some("rejected"));
        assert_eq!(q.ts, at(10));
        // The CLI's own text wins; a non-object quotaLimits still counts.
        let fi = scan_str(&reject("You've hit your limit · resets 3pm", "true"));
        let e = fi.agg.last_error.as_ref().unwrap();
        assert_eq!(e.kind, "quota");
        assert_eq!(e.text, "You've hit your limit · resets 3pm");
        let q = fi.agg.last_quota.as_ref().unwrap();
        assert_eq!(q.rate_limit_type, None);
        assert_eq!(q.ts, at(10));
        // No quotaLimits (or null): an ordinary API error.
        let fi = scan_str(&reject("Overloaded", "null"));
        assert_eq!(fi.agg.last_error.as_ref().unwrap().kind, "rate_limit");
        assert!(fi.agg.last_quota.is_none());
        let weekly = scan_str(&reject("", r#"{"rateLimitType":"seven_day_opus"}"#));
        assert_eq!(
            weekly.agg.last_error.as_ref().unwrap().text,
            "Plan usage limit reached (weekly window)."
        );
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
    fn contentless_dequeues_take_prompts_before_notifications() {
        let q = |op: &str, content: Option<&str>| {
            let c = content.map_or("null".to_string(), |c| serde_json::to_string(c).unwrap());
            format!(r#"{{"type":"queue-operation","operation":"{op}","content":{c}}}"#) + "\n"
        };
        let note = "<task-notification>\n<task-id>zz</task-id>\n</task-notification>";
        // A stale notification at the head, then a content-less prompt
        // (+T +N -N), then a text prompt taken by a content-less dequeue
        // (+H -N), as the CLI writes them.
        let lines = q("enqueue", Some(note))
            + &q("enqueue", None)
            + &q("dequeue", None)
            + &q("enqueue", Some("do this next"))
            + &q("dequeue", None);
        let fi = scan_str(&lines);
        let queue = &fi.agg.queue;
        assert_eq!(
            queue.iter().filter(|x| !x.notification).count(),
            0,
            "{queue:?}"
        );
        assert_eq!(queue.len(), 1);
        assert!(queue[0].notification);
        // With only notifications left, a dequeue takes the oldest.
        let fi = scan_str(&(q("enqueue", Some(note)) + &q("dequeue", None)));
        assert!(fi.agg.queue.is_empty());
    }

    #[test]
    fn a_full_queue_drops_stale_notifications_first() {
        let mut lines =
            String::from(r#"{"type":"queue-operation","operation":"enqueue","content":"keep me"}"#)
                + "\n";
        for i in 0..QUEUE_CAP {
            lines += &format!(
                r#"{{"type":"queue-operation","operation":"enqueue","content":"<task-notification><task-id>t{i}</task-id></task-notification>"}}"#
            );
            lines += "\n";
        }
        let fi = scan_str(&lines);
        assert_eq!(fi.agg.queue.len(), QUEUE_CAP);
        assert_eq!(fi.agg.queue[0].text, "keep me");
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
