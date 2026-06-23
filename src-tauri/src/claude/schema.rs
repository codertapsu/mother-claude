//! Tolerant serde models for the undocumented `~/.claude` data formats.
//!
//! These describe research-preview internals that drift between Claude Code
//! versions, so **every** field is optional and unknown keys are captured in an
//! `extra` map rather than causing a hard parse failure. Parsing helpers log and
//! skip malformed input instead of panicking.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One entry from `claude agents --json`.
///
/// Observed on 2.1.185: `{pid, cwd, kind, startedAt, sessionId}`. `state` and
/// `waitingFor` are modeled here in case a future version adds them, but they are
/// currently absent — session state is derived elsewhere (see the registry).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AgentEntry {
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default, rename = "startedAt")]
    pub started_at: Option<i64>,
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default, rename = "waitingFor")]
    pub waiting_for: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `~/.claude/daemon/roster.json`. `workers` is a map keyed by worker id.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Roster {
    #[serde(default)]
    pub proto: Option<u32>,
    #[serde(default, rename = "supervisorPid")]
    pub supervisor_pid: Option<u32>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub workers: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `~/.claude/jobs/<id>/state.json`. Shape is undocumented and may be absent
/// entirely; kept maximally tolerant.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct StateJson {
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, rename = "waitingFor")]
    pub waiting_for: Option<Value>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A single line from a `projects/<encoded-cwd>/<session-id>.jsonl` transcript.
///
/// `tool_use` / `tool_result` are **not** top-level types — they appear as blocks
/// inside `message.content` (see [`ContentBlock`]).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TranscriptEvent {
    #[serde(rename = "type", default)]
    pub event_type: String,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default, rename = "parentUuid")]
    pub parent_uuid: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default, rename = "gitBranch")]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default, rename = "isSidechain")]
    pub is_sidechain: Option<bool>,
    #[serde(default)]
    pub message: Option<Message>,
    /// Top-level `content` (e.g. on `system` events it is a plain string).
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The `message` object on `user` / `assistant` events.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Message {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub content: Option<MessageContent>,
    #[serde(default, rename = "stop_reason")]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `message.content` is either a plain string or an array of typed blocks.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

impl MessageContent {
    /// Best-effort flattening to displayable text (text + thinking blocks).
    pub fn to_text(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| b.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Names of any `tool_use` blocks in this message.
    pub fn tool_uses(&self) -> Vec<String> {
        match self {
            MessageContent::Text(_) => Vec::new(),
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter(|b| b.block_type == "tool_use")
                .filter_map(|b| b.name.clone())
                .collect(),
        }
    }
}

/// A typed block inside `message.content`: `text`, `thinking`, `tool_use`,
/// `tool_result`, etc.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ContentBlock {
    #[serde(rename = "type", default)]
    pub block_type: String,
    #[serde(default)]
    pub text: Option<String>,
    /// `tool_use`: tool name.
    #[serde(default)]
    pub name: Option<String>,
    /// `tool_use`: block id.
    #[serde(default)]
    pub id: Option<String>,
    /// `tool_result`: the id of the originating `tool_use`.
    #[serde(default, rename = "tool_use_id")]
    pub tool_use_id: Option<String>,
    /// `tool_use`: tool input arguments.
    #[serde(default)]
    pub input: Option<Value>,
    /// `tool_result`: result payload (string or array).
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default, rename = "is_error")]
    pub is_error: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `message.usage` token accounting.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Usage {
    /// Sum of all input + output token counts (cache-read excluded from "new"
    /// tokens but included here as a coarse total of tokens processed).
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.unwrap_or(0)
            + self.output_tokens.unwrap_or(0)
            + self.cache_creation_input_tokens.unwrap_or(0)
            + self.cache_read_input_tokens.unwrap_or(0)
    }
}

/// Parse a single transcript line. Returns `None` (and logs at debug) for blank
/// lines or lines that don't deserialize — never panics.
pub fn parse_transcript_line(line: &str) -> Option<TranscriptEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    match serde_json::from_str::<TranscriptEvent>(trimmed) {
        Ok(ev) => Some(ev),
        Err(e) => {
            tracing::debug!(error = %e, "skipping unparseable transcript line");
            None
        }
    }
}

/// Parse a whole transcript blob into events, skipping malformed lines.
pub fn parse_transcript(blob: &str) -> Vec<TranscriptEvent> {
    blob.lines().filter_map(parse_transcript_line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_entry_without_state_field() {
        // The real 2.1.185 shape — no `state`, no `waitingFor`.
        let json = r#"{"pid":4011,"cwd":"/a/b","kind":"interactive","startedAt":1782182119394,"sessionId":"ee712ae5"}"#;
        let e: AgentEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.pid, Some(4011));
        assert_eq!(e.session_id.as_deref(), Some("ee712ae5"));
        assert_eq!(e.kind.as_deref(), Some("interactive"));
        assert!(e.state.is_none());
    }

    #[test]
    fn agent_entry_tolerates_unknown_fields() {
        let json = r#"{"sessionId":"x","brandNewField":42,"nested":{"a":1}}"#;
        let e: AgentEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.session_id.as_deref(), Some("x"));
        assert!(e.extra.contains_key("brandNewField"));
        assert!(e.extra.contains_key("nested"));
    }

    #[test]
    fn parses_roster_with_map_workers() {
        let json = r#"{"proto":1,"supervisorPid":37911,"updatedAt":1782202069189,"workers":{}}"#;
        let r: Roster = serde_json::from_str(json).unwrap();
        assert_eq!(r.proto, Some(1));
        assert_eq!(r.supervisor_pid, Some(37911));
        assert!(r.workers.is_empty());
    }

    #[test]
    fn parses_assistant_event_with_usage_and_blocks() {
        let json = r#"{"type":"assistant","uuid":"u1","parentUuid":"p0","timestamp":"2026-06-23T08:59:08.494Z","sessionId":"s1","cwd":"/x","gitBranch":"main","message":{"role":"assistant","model":"claude-opus","stop_reason":"end_turn","content":[{"type":"text","text":"hello"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"},"caller":"x"}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100}}}"#;
        let ev = parse_transcript_line(json).unwrap();
        assert_eq!(ev.event_type, "assistant");
        let msg = ev.message.unwrap();
        assert_eq!(msg.model.as_deref(), Some("claude-opus"));
        let content = msg.content.unwrap();
        assert_eq!(content.to_text(), "hello");
        assert_eq!(content.tool_uses(), vec!["Bash".to_string()]);
        let usage = msg.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.total_tokens(), 115);
    }

    #[test]
    fn parses_system_event_with_string_content() {
        let json =
            r#"{"type":"system","subtype":"informational","content":"Auto mode lets Claude..."}"#;
        let ev = parse_transcript_line(json).unwrap();
        assert_eq!(ev.event_type, "system");
        assert_eq!(ev.subtype.as_deref(), Some("informational"));
        assert!(ev.content.is_some());
    }

    #[test]
    fn parses_tool_result_block() {
        let json = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok","is_error":false}]}}"#;
        let ev = parse_transcript_line(json).unwrap();
        let MessageContent::Blocks(blocks) = ev.message.unwrap().content.unwrap() else {
            panic!("expected blocks");
        };
        assert_eq!(blocks[0].block_type, "tool_result");
        assert_eq!(blocks[0].tool_use_id.as_deref(), Some("t1"));
        assert_eq!(blocks[0].is_error, Some(false));
    }

    #[test]
    fn skips_blank_and_garbage_lines() {
        assert!(parse_transcript_line("").is_none());
        assert!(parse_transcript_line("   ").is_none());
        assert!(parse_transcript_line("{not json").is_none());
        let blob = "\n{\"type\":\"user\"}\ngarbage\n{\"type\":\"assistant\"}\n";
        let events = parse_transcript(blob);
        assert_eq!(events.len(), 2);
    }
}
