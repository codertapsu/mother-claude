//! Transcript normalisation into [`UiEvent`]s (design §3.6, §4.4, amended
//! §10 B2).
//!
//! * **Typed, bounded parsing**: lines are parsed into UI-specific records
//!   whose heavy parts are skipped or clipped (image `data`, `originalFile`,
//!   tool-result bodies beyond 4 KB, Bash output beyond a 4 KB tail, patches
//!   beyond 200 lines). Nothing is ever read with `read_to_string`.
//! * **Ids**: `UiEvent.id = "<record uuid>:<block index>"` (records without a
//!   uuid use `@<byte offset>`), `messageId` = `message.id`. Stable across
//!   REST pages, WS appends and restarts.
//! * **Noise filter**: metadata records (titles, modes, bridge-session,
//!   file-history, queue-operation, cost-state, …), attachments (except the
//!   hook context that carries a relayed message) and meta user records are
//!   dropped.
//! * **Images** become refs `/api/sessions/<id>/media/<lineOffset>/<uuid>/
//!   <blockPath>` (`blockPath` = `3` or `3.0` for an image inside a
//!   tool_result); [`read_media`] seeks to the line, verifies the uuid and
//!   returns the decoded bytes only for PNG/JPEG/GIF/WebP by magic bytes.
//! * **Paging**: [`read_page`] reads BACKWARDS from EOF (or `before`) in
//!   chunks, scans at most 32 MB per request and returns `before` whenever
//!   there is more. [`read_range`] reads forward between two cursors with the
//!   2 MB / 300-event cap used for WS replay and REST `after=`.
//! * **Relayed messages** (Mother Claude's hook relay, marker `[[MC-RELAY`)
//!   are recognised in task-notification prompts, `hook_additional_context`
//!   attachments and `stop_hook_summary.hookAdditionalContext`.
//! * [`Timeline`] keeps Mother Claude's own events (e.g. `decision` rows);
//!   they are merged into pages by timestamp.
//!
//! Cursors are byte offsets rendered as decimal strings (opaque to clients).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::Value;

use super::index::agg::{parse_task_notification, parse_ts, ErrField};
use super::index::loose::{self, parse_line, CapStr, LBool, LStr, Obj, Seq, LU64};
use super::model::{
    Todo, TodoStatus, TranscriptPage, UiEvent, UiImage, UiKind, UiPatch, UiPlan, UiQuestion,
    UiResult, UiRole, UiTask, UiTool,
};
use super::schema::TranscriptEvent;

/// Tool-result text kept per event.
pub const RESULT_TEXT_MAX: usize = 4 * 1024;
/// Largest page a client may ask for.
pub const PAGE_MAX: usize = 300;
/// Bytes a backward page may scan before it returns with a `before` cursor.
pub const PAGE_SCAN_MAX: u64 = 32 * 1024 * 1024;
/// Replay / `after=` caps: events and serialized bytes.
pub const REPLAY_MAX_EVENTS: usize = 300;
pub const REPLAY_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Raw bytes a forward read may cover before it gives up with `reset`.
pub const RANGE_SCAN_MAX: u64 = 32 * 1024 * 1024;
/// Lines longer than this are skipped.
pub const LINE_MAX: usize = super::index::agg::LINE_CAP;
/// Patch lines kept per Edit/Write result.
const PATCH_LINES_MAX: usize = 200;
/// Tool inputs larger than this (JSON bytes) have their strings clipped.
const INPUT_MAX: usize = 32 * 1024;
/// Decoded media larger than this is refused.
pub const MEDIA_MAX: usize = 20 * 1024 * 1024;
/// The relay marker Mother Claude's hook relay prefixes messages with.
pub const RELAY_MARKER: &str = "[[MC-RELAY";

/// Truncate to at most `max` bytes on a char boundary. Returns (text, cut?).
pub fn truncate_bytes(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    (format!("{}…", loose::clip(s, max)), true)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// A path relative to `cwd` when it lies under it.
fn rel<'a>(path: &'a str, cwd: Option<&str>) -> &'a str {
    match cwd {
        Some(c) if !c.is_empty() => path
            .strip_prefix(c)
            .map(|r| r.trim_start_matches('/'))
            .filter(|r| !r.is_empty())
            .unwrap_or(path),
        _ => path,
    }
}

/// One-line description of a tool call (design §3.5).
pub fn tool_summary(name: &str, input: Option<&Value>) -> String {
    tool_summary_in(name, input, None)
}

/// [`tool_summary`] with file paths shown relative to `cwd`.
pub fn tool_summary_in(name: &str, input: Option<&Value>, cwd: Option<&str>) -> String {
    let get = |k: &str| {
        input
            .and_then(|i| i.get(k))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    let s = match name {
        "Bash" => get("description").or_else(|| get("command")),
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "NotebookRead" => {
            get("file_path")
                .or_else(|| get("notebook_path"))
                .map(|p| rel(&p, cwd).to_string())
        }
        "Grep" | "Glob" => get("pattern"),
        "Agent" | "Task" => get("description"),
        "Workflow" => get("name").or_else(|| get("workflow")),
        "WebFetch" => get("url"),
        "WebSearch" => get("query"),
        "Skill" => get("skill").or_else(|| get("name")),
        "Monitor" => get("description").or_else(|| get("command")),
        "TaskOutput" | "TaskStop" | "KillShell" => get("task_id").or_else(|| get("shell_id")),
        "TodoWrite" => input
            .and_then(|i| i.get("todos"))
            .and_then(Value::as_array)
            .and_then(|todos| {
                todos
                    .iter()
                    .find(|t| t.get("status").and_then(Value::as_str) == Some("in_progress"))
                    .and_then(|t| t.get("activeForm").or_else(|| t.get("content")))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .or_else(|| Some("Update the todo list".into())),
        "AskUserQuestion" => input
            .and_then(|i| i.get("questions"))
            .and_then(Value::as_array)
            .and_then(|q| q.first())
            .and_then(|q| q.get("question"))
            .and_then(Value::as_str)
            .map(str::to_string),
        "ExitPlanMode" => Some("Plan ready for review".into()),
        _ if name.starts_with("mcp__") => {
            let mut parts = name.splitn(3, "__").skip(1);
            match (parts.next(), parts.next()) {
                (Some(server), Some(tool)) => Some(format!("{server} · {tool}")),
                _ => None,
            }
        }
        _ => None,
    };
    let one_line = s
        .unwrap_or_else(|| name.to_string())
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    truncate_chars(&one_line, 120)
}

fn todos_from(input: Option<&Value>) -> Vec<Todo> {
    input
        .and_then(|i| i.get("todos"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    Some(Todo {
                        content: t.get("content")?.as_str()?.to_string(),
                        status: match t.get("status").and_then(Value::as_str) {
                            Some("completed") => TodoStatus::Completed,
                            Some("in_progress") => TodoStatus::InProgress,
                            _ => TodoStatus::Pending,
                        },
                        active_form: t
                            .get("activeForm")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Clip every string leaf of a large value so tool inputs stay small.
fn clip_value(v: Value) -> Value {
    let size = serde_json::to_vec(&v)
        .map(|b| b.len())
        .unwrap_or(usize::MAX);
    if size <= INPUT_MAX {
        return v;
    }
    fn walk(v: Value, depth: usize) -> Value {
        match v {
            Value::String(s) if s.len() > 4096 => {
                Value::String(format!("{}… ({} bytes)", loose::clip(&s, 4096), s.len()))
            }
            Value::Array(a) if depth < 6 => Value::Array(
                a.into_iter()
                    .take(200)
                    .map(|x| walk(x, depth + 1))
                    .collect(),
            ),
            Value::Object(o) if depth < 6 => Value::Object(
                o.into_iter()
                    .map(|(k, x)| (k, walk(x, depth + 1)))
                    .collect(),
            ),
            other => other,
        }
    }
    walk(v, 0)
}

// ───────────────────────────── UI record shapes ──────────────────────────────

/// A string keeping only its LAST `N` bytes (Bash output tails).
#[derive(Debug, Clone, Default)]
struct TailStr<const N: usize> {
    text: Option<String>,
    cut: bool,
}

impl<'de, const N: usize> Deserialize<'de> for TailStr<N> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let c = CapStr::<{ usize::MAX / 2 }>::deserialize(d);
        // CapStr with a huge cap keeps the whole string; clip the tail here.
        let c = c?;
        Ok(match c.text {
            Some(s) if s.len() > N => {
                let mut start = s.len() - N;
                while start < s.len() && !s.is_char_boundary(start) {
                    start += 1;
                }
                TailStr {
                    text: Some(s[start..].to_string()),
                    cut: true,
                }
            }
            other => TailStr {
                text: other,
                cut: false,
            },
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct URec {
    #[serde(rename = "type")]
    ty: LStr,
    subtype: LStr,
    uuid: LStr,
    timestamp: LStr,
    #[serde(rename = "isSidechain")]
    is_sidechain: LBool,
    #[serde(rename = "isMeta")]
    is_meta: LBool,
    #[serde(rename = "isCompactSummary")]
    is_compact_summary: LBool,
    #[serde(rename = "isApiErrorMessage")]
    is_api_error: LBool,
    #[serde(rename = "agentId")]
    agent_id: LStr,
    cwd: LStr,
    message: Obj<UMsg>,
    #[serde(rename = "toolUseResult")]
    tur: Obj<UTur>,
    attachment: Obj<UAtt>,
    origin: Obj<UOrigin>,
    content: CapStr<8192>,
    operation: LStr,
    error: ErrField,
    #[serde(rename = "retryAttempt")]
    retry_attempt: LU64,
    #[serde(rename = "maxRetries")]
    max_retries: LU64,
    #[serde(rename = "hookAdditionalContext")]
    hook_additional_context: Seq<CapStr<16384>>,
    #[serde(rename = "compactMetadata")]
    compact: Obj<UCompact>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UCompact {
    trigger: LStr,
    #[serde(rename = "preTokens")]
    pre_tokens: LU64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UOrigin {
    kind: LStr,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UMsg {
    id: LStr,
    content: UContent,
}

#[derive(Debug, Default)]
enum UContent {
    #[default]
    None,
    Text(CapStr<65536>),
    Blocks(Vec<UBlock>),
}

impl<'de> Deserialize<'de> for UContent {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, SeqAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = UContent;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("content")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<UContent, E> {
                let kept = loose::clip(v, 65536);
                Ok(UContent::Text(CapStr {
                    text: Some(kept.to_string()),
                    cut: kept.len() < v.len(),
                    len: v.len(),
                }))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<UContent, A::Error> {
                let mut out = Vec::new();
                while let Some(b) = seq.next_element::<UBlock>()? {
                    if out.len() < 512 {
                        out.push(b);
                    }
                }
                Ok(UContent::Blocks(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<UContent, A::Error> {
                while map
                    .next_entry::<de::IgnoredAny, de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(UContent::None)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<UContent, E> {
                Ok(UContent::None)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<UContent, E> {
                Ok(UContent::None)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<UContent, E> {
                Ok(UContent::None)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<UContent, E> {
                Ok(UContent::None)
            }
            fn visit_unit<E: de::Error>(self) -> Result<UContent, E> {
                Ok(UContent::None)
            }
            fn visit_none<E: de::Error>(self) -> Result<UContent, E> {
                Ok(UContent::None)
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UBlock {
    #[serde(rename = "type")]
    ty: LStr,
    text: CapStr<65536>,
    thinking: CapStr<65536>,
    id: LStr,
    name: LStr,
    input: Option<Value>,
    tool_use_id: LStr,
    is_error: LBool,
    /// tool_result body: a string, or nested blocks (text / image).
    content: UContent,
    source: Obj<USource>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct USource {
    #[serde(rename = "type")]
    ty: LStr,
    media_type: LStr,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UTur {
    stdout: TailStr<4096>,
    stderr: TailStr<4096>,
    interrupted: LBool,
    #[serde(rename = "filePath")]
    file_path: CapStr<4096>,
    #[serde(rename = "structuredPatch")]
    structured_patch: Seq<UHunk>,
    answers: Option<Value>,
    questions: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UHunk {
    #[serde(rename = "oldStart")]
    old_start: LU64,
    #[serde(rename = "oldLines")]
    old_lines: LU64,
    #[serde(rename = "newStart")]
    new_start: LU64,
    #[serde(rename = "newLines")]
    new_lines: LU64,
    lines: Seq<CapStr<1024>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UAtt {
    #[serde(rename = "type")]
    ty: LStr,
    content: Option<Value>,
}

/// Text of an attachment `content` (string or array of strings/blocks).
fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(value_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(o) => o
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn strip_relay_marker(text: &str) -> String {
    // `[[MC-RELAY <marker>]] message…` → `message…`
    match text.find(RELAY_MARKER) {
        Some(i) => {
            let after = &text[i..];
            match after.find("]]") {
                Some(j) => after[j + 2..].trim().to_string(),
                None => text.to_string(),
            }
        }
        None => text.to_string(),
    }
}

fn answers_map(v: &Value) -> Option<BTreeMap<String, String>> {
    let o = v.as_object()?;
    Some(
        o.iter()
            .map(|(k, v)| {
                let s = match v {
                    Value::String(s) => s.clone(),
                    Value::Array(a) => a
                        .iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                        .join(", "),
                    other => other.to_string(),
                };
                (k.clone(), s)
            })
            .collect(),
    )
}

/// Parse `<command-name>/x</command-name><command-args>y</command-args>`.
fn slash_command(text: &str) -> Option<String> {
    let name = super::index::agg::xml_tag(text, "command-name")?;
    let args = super::index::agg::xml_tag(text, "command-args").unwrap_or("");
    let name = if name.starts_with('/') {
        name.to_string()
    } else {
        format!("/{name}")
    };
    Some(if args.is_empty() {
        name
    } else {
        format!("{name} {args}")
    })
}

/// Options for mapping one record.
#[derive(Debug, Clone, Copy, Default)]
pub struct MapOpts<'a> {
    /// Reading a subagent file (`?agent=`): sidechain records are the content.
    pub agent: Option<&'a str>,
}

fn media_ref(session_id: &str, offset: u64, uuid: &str, path: &str, agent: Option<&str>) -> String {
    let base = format!("/api/sessions/{session_id}/media/{offset}/{uuid}/{path}");
    match agent {
        Some(a) => format!("{base}?agent={a}"),
        None => base,
    }
}

fn map_rec(session_id: &str, rec: URec, offset: u64, opts: MapOpts) -> Vec<UiEvent> {
    if rec.is_sidechain.yes() && opts.agent.is_none() {
        return Vec::new();
    }
    let uuid = rec.uuid.get().map(str::to_string);
    let base_id = uuid.clone().unwrap_or_else(|| format!("@{offset}"));
    let uuid_seg = uuid.clone().unwrap_or_else(|| "_".into());
    let ts = parse_ts(rec.timestamp.get());
    let cwd = rec.cwd.get().map(str::to_string);
    let agent_id = opts
        .agent
        .map(str::to_string)
        .or_else(|| rec.agent_id.get().map(str::to_string))
        .filter(|_| opts.agent.is_some());
    let message_id = rec
        .message
        .get()
        .and_then(|m| m.id.get().map(str::to_string));
    let mk = |idx: &str, role: UiRole, kind: UiKind| UiEvent {
        id: format!("{base_id}:{idx}"),
        message_id: message_id.clone(),
        ts,
        role,
        kind,
        agent_id: agent_id.clone(),
        ..Default::default()
    };
    let mut out = Vec::new();

    match rec.ty.get().unwrap_or("") {
        "user" => {
            if rec.is_meta.yes() || rec.is_compact_summary.yes() {
                return out;
            }
            let origin = rec
                .origin
                .get()
                .and_then(|o| o.kind.get())
                .map(str::to_string);
            let Some(msg) = rec.message.0 else {
                return out;
            };
            let tur = rec.tur.0;
            let blocks: Vec<UBlock> = match msg.content {
                UContent::Text(t) => vec![UBlock {
                    ty: LStr(Some("text".into())),
                    text: t,
                    ..Default::default()
                }],
                UContent::Blocks(b) => b,
                UContent::None => Vec::new(),
            };
            for (i, b) in blocks.into_iter().enumerate() {
                let idx = i.to_string();
                match b.ty.get().unwrap_or("") {
                    "text" => {
                        let Some(text) = b.text.text.filter(|t| !t.trim().is_empty()) else {
                            continue;
                        };
                        out.extend(user_text(&mk, &idx, &text, origin.as_deref()));
                    }
                    "tool_result" => {
                        let tool_use_id = b.tool_use_id.get().unwrap_or("").to_string();
                        let is_error = b.is_error.yes();
                        let mut texts: Vec<String> = Vec::new();
                        let mut images: Vec<(String, String)> = Vec::new();
                        match b.content {
                            UContent::Text(t) => {
                                if let Some(t) = t.text {
                                    texts.push(t);
                                }
                            }
                            UContent::Blocks(parts) => {
                                for (j, p) in parts.into_iter().enumerate() {
                                    match p.ty.get() {
                                        Some("text") => {
                                            if let Some(t) = p.text.text {
                                                texts.push(t);
                                            }
                                        }
                                        Some("image") => {
                                            let mt = p
                                                .source
                                                .get()
                                                .and_then(|s| s.media_type.get())
                                                .unwrap_or("image/png")
                                                .to_string();
                                            images.push((format!("{i}.{j}"), mt));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            UContent::None => {}
                        }
                        let mut text = texts.join("\n");
                        let mut patch = None;
                        let mut truncated = false;
                        if let Some(t) = &tur {
                            // Bash: stdout/stderr tails.
                            let out_tail = t.stdout.text.clone().unwrap_or_default();
                            let err_tail = t.stderr.text.clone().unwrap_or_default();
                            if !out_tail.is_empty() || !err_tail.is_empty() {
                                text = match (out_tail.is_empty(), err_tail.is_empty()) {
                                    (false, true) => out_tail,
                                    (true, false) => err_tail,
                                    _ => format!("{out_tail}\n{err_tail}"),
                                };
                                truncated |= t.stdout.cut || t.stderr.cut;
                            }
                            // Edit/Write: the structured patch (≤ 200 lines).
                            if let Some(path) = t.file_path.text.clone() {
                                if !t.structured_patch.0.is_empty() {
                                    let mut budget = PATCH_LINES_MAX;
                                    let mut hunks = Vec::new();
                                    for h in &t.structured_patch.0 {
                                        if budget == 0 {
                                            truncated = true;
                                            break;
                                        }
                                        let lines: Vec<String> = h
                                            .lines
                                            .0
                                            .iter()
                                            .take(budget)
                                            .filter_map(|l| l.text.clone())
                                            .collect();
                                        if lines.len() < h.lines.0.len() {
                                            truncated = true;
                                        }
                                        budget -= lines.len();
                                        hunks.push(serde_json::json!({
                                            "oldStart": h.old_start.n(),
                                            "oldLines": h.old_lines.n(),
                                            "newStart": h.new_start.n(),
                                            "newLines": h.new_lines.n(),
                                            "lines": lines,
                                        }));
                                    }
                                    patch = Some(UiPatch {
                                        path: rel(&path, cwd.as_deref()).to_string(),
                                        hunks,
                                    });
                                }
                            }
                        }
                        let (text, cut) = truncate_bytes(&text, RESULT_TEXT_MAX);
                        let mut e = mk(&idx, UiRole::User, UiKind::Result);
                        e.result = Some(UiResult {
                            tool_use_id: tool_use_id.clone(),
                            is_error,
                            text: Some(text).filter(|t| !t.is_empty()),
                            patch,
                            truncated: (cut || truncated).then_some(true),
                        });
                        // AskUserQuestion answers ride on the result.
                        if let Some(t) = &tur {
                            if let Some(answers) = t.answers.as_ref().and_then(answers_map) {
                                e.kind = UiKind::Question;
                                e.question = Some(UiQuestion {
                                    questions: t
                                        .questions
                                        .as_ref()
                                        .map(|q| {
                                            crate::pending::questions_from_input(
                                                &serde_json::json!({ "questions": q }),
                                            )
                                        })
                                        .unwrap_or_default(),
                                    answers: Some(answers),
                                });
                            }
                        }
                        out.push(e);
                        for (path, mt) in images {
                            let mut img = mk(&path, UiRole::User, UiKind::Image);
                            img.image = Some(UiImage {
                                media_type: mt,
                                reference: media_ref(
                                    session_id, offset, &uuid_seg, &path, opts.agent,
                                ),
                            });
                            out.push(img);
                        }
                    }
                    "image" => {
                        let mt = b
                            .source
                            .get()
                            .and_then(|s| s.media_type.get())
                            .unwrap_or("image/png")
                            .to_string();
                        let mut e = mk(&idx, UiRole::User, UiKind::Image);
                        e.image = Some(UiImage {
                            media_type: mt,
                            reference: media_ref(session_id, offset, &uuid_seg, &idx, opts.agent),
                        });
                        out.push(e);
                    }
                    _ => {}
                }
            }
        }
        "assistant" => {
            let Some(msg) = rec.message.0 else {
                return out;
            };
            if rec.is_api_error.yes() {
                let text = match &msg.content {
                    UContent::Blocks(b) => b.iter().find_map(|b| b.text.text.clone()),
                    UContent::Text(t) => t.text.clone(),
                    UContent::None => None,
                };
                let mut e = mk("0", UiRole::System, UiKind::Error);
                e.text = text.or(rec.error.0);
                out.push(e);
                return out;
            }
            let blocks = match msg.content {
                UContent::Blocks(b) => b,
                UContent::Text(t) => vec![UBlock {
                    ty: LStr(Some("text".into())),
                    text: t,
                    ..Default::default()
                }],
                UContent::None => Vec::new(),
            };
            for (i, b) in blocks.into_iter().enumerate() {
                let idx = i.to_string();
                match b.ty.get().unwrap_or("") {
                    "text" => {
                        if let Some(t) = b.text.text.filter(|t| !t.trim().is_empty()) {
                            let mut e = mk(&idx, UiRole::Assistant, UiKind::Text);
                            e.text = Some(t);
                            out.push(e);
                        }
                    }
                    "thinking" => {
                        let t = b.thinking.text.or(b.text.text).unwrap_or_default();
                        if !t.trim().is_empty() {
                            let mut e = mk(&idx, UiRole::Assistant, UiKind::Thinking);
                            e.text = Some(t);
                            out.push(e);
                        }
                    }
                    "tool_use" | "server_tool_use" => {
                        let name = b.name.get().unwrap_or_default().to_string();
                        let input = b.input.map(clip_value);
                        let kind = match name.as_str() {
                            "TodoWrite" => UiKind::Todos,
                            "AskUserQuestion" => UiKind::Question,
                            "ExitPlanMode" => UiKind::Plan,
                            _ => UiKind::Tool,
                        };
                        let mut e = mk(&idx, UiRole::Assistant, kind);
                        match kind {
                            UiKind::Todos => e.todos = Some(todos_from(input.as_ref())),
                            UiKind::Question => {
                                e.question = Some(UiQuestion {
                                    questions: input
                                        .as_ref()
                                        .map(crate::pending::questions_from_input)
                                        .unwrap_or_default(),
                                    answers: None,
                                });
                            }
                            UiKind::Plan => {
                                e.plan = Some(UiPlan {
                                    text: input
                                        .as_ref()
                                        .and_then(|i| i.get("plan"))
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string(),
                                })
                            }
                            _ => {}
                        }
                        e.tool = Some(UiTool {
                            id: b.id.get().unwrap_or_default().to_string(),
                            summary: tool_summary_in(&name, input.as_ref(), cwd.as_deref()),
                            name,
                            input,
                        });
                        out.push(e);
                    }
                    _ => {}
                }
            }
        }
        "system" => match rec.subtype.get().unwrap_or("") {
            "compact_boundary" => {
                let mut e = mk("0", UiRole::System, UiKind::Compact);
                let detail = rec.compact.get().map(|c| {
                    let trig = c.trigger.get().unwrap_or("auto");
                    match c.pre_tokens.0 {
                        Some(n) => format!("Conversation compacted ({trig}, {n} tokens before)"),
                        None => format!("Conversation compacted ({trig})"),
                    }
                });
                e.text = detail
                    .or(rec.content.text)
                    .or(Some("Conversation compacted".into()));
                out.push(e);
            }
            "api_error" => {
                let mut e = mk("0", UiRole::System, UiKind::Error);
                let mut text = rec.error.0.unwrap_or_else(|| "API error".into());
                if let (Some(n), Some(max)) = (rec.retry_attempt.0, rec.max_retries.0) {
                    text = format!("{text} (retry {n} of {max})");
                }
                e.text = Some(text);
                out.push(e);
            }
            "local_command" => {
                if let Some(t) = rec.content.text.as_deref() {
                    let cleaned = super::index::agg::xml_tag(t, "local-command-stdout")
                        .map(str::to_string)
                        .unwrap_or_else(|| t.to_string());
                    if !cleaned.trim().is_empty() {
                        let mut e = mk("0", UiRole::System, UiKind::Notice);
                        e.text = Some(truncate_bytes(&cleaned, RESULT_TEXT_MAX).0);
                        out.push(e);
                    }
                }
            }
            "stop_hook_summary" => {
                for (i, c) in rec.hook_additional_context.0.iter().enumerate() {
                    if let Some(t) = c.get().filter(|t| t.contains(RELAY_MARKER)) {
                        let mut e = mk(&i.to_string(), UiRole::User, UiKind::Relayed);
                        e.text = Some(strip_relay_marker(t));
                        out.push(e);
                    }
                }
            }
            _ => {}
        },
        "queue-operation" => {
            if rec.operation.is("enqueue") {
                if let Some(text) = rec.content.text.as_deref() {
                    if !text.contains(RELAY_MARKER) {
                        if let Some(n) = parse_task_notification(text) {
                            let mut e = mk("0", UiRole::System, UiKind::Task);
                            e.text = n.summary.clone();
                            e.task = Some(UiTask {
                                task_id: n.task_id,
                                status: n.status,
                                summary: n.summary,
                            });
                            out.push(e);
                        }
                    }
                }
            }
        }
        "attachment" => {
            if let Some(att) = rec.attachment.0 {
                if att.ty.is("hook_additional_context") {
                    let text = att.content.as_ref().map(value_text).unwrap_or_default();
                    if text.contains(RELAY_MARKER) {
                        let mut e = mk("0", UiRole::User, UiKind::Relayed);
                        e.text = Some(strip_relay_marker(&text));
                        out.push(e);
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn user_text(
    mk: &dyn Fn(&str, UiRole, UiKind) -> UiEvent,
    idx: &str,
    text: &str,
    origin: Option<&str>,
) -> Vec<UiEvent> {
    let mut out = Vec::new();
    if text.contains(RELAY_MARKER) {
        let mut e = mk(idx, UiRole::User, UiKind::Relayed);
        e.text = Some(strip_relay_marker(text));
        out.push(e);
        return out;
    }
    if origin == Some("task-notification") || text.contains("<task-notification>") {
        // Rendered from its `queue-operation` enqueue (every notification has
        // exactly one; many are absorbed mid-turn and have no user record).
        return out;
    }
    if text.starts_with("<local-command-stdout>") || text.starts_with("<local-command-stderr>") {
        let cleaned = super::index::agg::xml_tag(text, "local-command-stdout")
            .or_else(|| super::index::agg::xml_tag(text, "local-command-stderr"))
            .unwrap_or("");
        if !cleaned.trim().is_empty() {
            let mut e = mk(idx, UiRole::System, UiKind::Notice);
            e.text = Some(truncate_bytes(cleaned, RESULT_TEXT_MAX).0);
            out.push(e);
        }
        return out;
    }
    if let Some(cmd) = text
        .contains("<command-name>")
        .then(|| slash_command(text))
        .flatten()
    {
        let mut e = mk(idx, UiRole::User, UiKind::Prompt);
        e.text = Some(cmd);
        out.push(e);
        return out;
    }
    if text.starts_with("[Request interrupted by user") {
        let mut e = mk(idx, UiRole::System, UiKind::Notice);
        e.text = Some("Interrupted".into());
        out.push(e);
        return out;
    }
    if text.starts_with("<system-reminder>") || text.starts_with("Caveat: ") {
        return out;
    }
    let mut e = mk(idx, UiRole::User, UiKind::Prompt);
    e.text = Some(text.to_string());
    out.push(e);
    out
}

/// Map one raw line (at byte `offset`) into UI events.
pub fn map_line(session_id: &str, line: &[u8], offset: u64, opts: MapOpts) -> Vec<UiEvent> {
    match parse_line::<URec>(line) {
        Some(rec) => map_rec(session_id, rec, offset, opts),
        None => Vec::new(),
    }
}

/// Map one already-parsed record (compat entry point for callers holding a
/// [`TranscriptEvent`]). `fallback_id` names the record when it has no
/// `uuid` (e.g. `"@<byte offset>"`).
pub fn map_event(session_id: &str, ev: &TranscriptEvent, fallback_id: &str) -> Vec<UiEvent> {
    let offset = fallback_id
        .trim_start_matches('@')
        .parse::<u64>()
        .unwrap_or(0);
    let Ok(bytes) = serde_json::to_vec(ev) else {
        return Vec::new();
    };
    map_line(session_id, &bytes, offset, MapOpts::default())
}

// ─────────────────────────────── paging ──────────────────────────────────────

/// Reads lines backwards from a byte position.
struct BackReader {
    file: File,
    /// `win` holds the file bytes `[win_start, win_start + win.len())`; the
    /// window always ends at the current line end.
    win_start: u64,
    win: Vec<u8>,
    scanned: u64,
}

const BACK_CHUNK: u64 = 256 * 1024;

impl BackReader {
    /// Position at `end` (a line start; trailing partial lines are excluded
    /// when `end` is EOF).
    fn new(mut file: File, end: u64, len: u64) -> std::io::Result<Self> {
        let mut end = end.min(len);
        if end == len && len > 0 {
            // Exclude a trailing partial line.
            let mut last = [0u8; 1];
            file.seek(SeekFrom::Start(len - 1))?;
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                let mut probe = Self {
                    file,
                    win_start: len,
                    win: Vec::new(),
                    scanned: 0,
                };
                let start = probe.skip_back_to_newline(len)?;
                end = start;
                file = probe.file;
            }
        }
        Ok(Self {
            file,
            win_start: end,
            win: Vec::new(),
            scanned: 0,
        })
    }

    fn end(&self) -> u64 {
        self.win_start + self.win.len() as u64
    }

    /// Scan backwards from `from` (exclusive) to just past the previous `\n`
    /// (or 0) without keeping bytes.
    fn skip_back_to_newline(&mut self, from: u64) -> std::io::Result<u64> {
        // `from` is never inside a terminator: the byte at `from - 1`, if it
        // is a newline, ends the previous line, so the answer is `from`.
        let mut pos = from;
        let mut buf = vec![0u8; BACK_CHUNK as usize];
        while pos > 0 {
            let n = BACK_CHUNK.min(pos);
            let start = pos - n;
            self.file.seek(SeekFrom::Start(start))?;
            self.file.read_exact(&mut buf[..n as usize])?;
            self.scanned += n;
            if let Some(i) = buf[..n as usize].iter().rposition(|b| *b == b'\n') {
                return Ok(start + i as u64 + 1);
            }
            pos = start;
        }
        Ok(0)
    }

    /// The line ending at the current position: `(start, bytes)`; bytes are
    /// `None` for a line longer than [`LINE_MAX`]. `None` at the file start.
    fn prev_line(&mut self) -> std::io::Result<Option<(u64, Option<Vec<u8>>)>> {
        let end = self.end();
        if end == 0 {
            return Ok(None);
        }
        // Search excluding the line's own terminator (the last byte).
        let mut search_to = self.win.len().saturating_sub(1);
        loop {
            if let Some(i) = self.win[..search_to].iter().rposition(|b| *b == b'\n') {
                let line = self.win.split_off(i + 1);
                return Ok(Some((self.win_start + i as u64 + 1, Some(line))));
            }
            if self.win_start == 0 {
                let line = std::mem::take(&mut self.win);
                return Ok(Some((0, Some(line))));
            }
            if self.win.len() > LINE_MAX {
                let start = self.skip_back_to_newline(self.win_start)?;
                self.win.clear();
                self.win_start = start;
                return Ok(Some((start, None)));
            }
            let grow = (self.win.len() as u64).max(BACK_CHUNK).min(self.win_start);
            let new_start = self.win_start - grow;
            let mut chunk = vec![0u8; grow as usize];
            self.file.seek(SeekFrom::Start(new_start))?;
            self.file.read_exact(&mut chunk)?;
            self.scanned += grow;
            chunk.extend_from_slice(&self.win);
            self.win = chunk;
            self.win_start = new_start;
            search_to = grow as usize;
            // The first read includes the terminator of this line: exclude it.
            if end == self.end() && search_to == self.win.len() {
                search_to = search_to.saturating_sub(1);
            }
        }
    }
}

/// Paging options.
#[derive(Debug, Clone, Copy, Default)]
pub struct PageOpts<'a> {
    pub before: Option<u64>,
    pub limit: usize,
    pub agent: Option<&'a str>,
}

/// A page of UI events ending before byte `before` (or at EOF), newest last,
/// at most `limit` (≤ [`PAGE_MAX`]) events — more only when one record alone
/// exceeds it. Missing file ⇒ empty page.
pub fn read_page(
    session_id: &str,
    path: &Path,
    before: Option<u64>,
    limit: usize,
) -> std::io::Result<TranscriptPage> {
    read_page_opts(
        session_id,
        path,
        PageOpts {
            before,
            limit,
            agent: None,
        },
    )
}

/// [`read_page`] with an agent transcript option.
pub fn read_page_opts(
    session_id: &str,
    path: &Path,
    opts: PageOpts,
) -> std::io::Result<TranscriptPage> {
    let limit = opts.limit.clamp(1, PAGE_MAX);
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TranscriptPage {
                tail: "0".into(),
                ..Default::default()
            })
        }
        Err(e) => return Err(e),
    };
    let len = file.metadata()?.len();
    let mut back = BackReader::new(file, opts.before.unwrap_or(len), len)?;
    // The tail cursor: the end of the last complete line.
    let tail = if opts.before.is_none() {
        back.end()
    } else {
        tail_of(path, len)?
    };
    let map_opts = MapOpts { agent: opts.agent };
    let mut records: Vec<Vec<UiEvent>> = Vec::new();
    let mut count = 0usize;
    while count < limit && back.scanned < PAGE_SCAN_MAX {
        let Some((start, line)) = back.prev_line()? else {
            break;
        };
        if let Some(line) = line {
            let events = map_line(session_id, &line, start, map_opts);
            if !events.is_empty() {
                count += events.len();
                records.push(events);
            }
        }
    }
    let first = back.end();
    records.reverse();
    Ok(TranscriptPage {
        events: records.into_iter().flatten().collect(),
        before: (first > 0).then(|| first.to_string()),
        tail: tail.to_string(),
        reset: None,
    })
}

/// End of the last complete line of a file of length `len`.
fn tail_of(path: &Path, len: u64) -> std::io::Result<u64> {
    if len == 0 {
        return Ok(0);
    }
    let back = BackReader::new(File::open(path)?, len, len)?;
    Ok(back.end())
}

/// Result of a bounded forward read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RangeRead {
    pub events: Vec<UiEvent>,
    /// The cursor after the last complete line read.
    pub tail: u64,
    /// The range was too large for one bounded read: the client should
    /// refetch the last page.
    pub reset: bool,
}

/// Events in `[from, to)` (or to EOF), capped at [`REPLAY_MAX_EVENTS`] events
/// / [`REPLAY_MAX_BYTES`] of JSON / [`RANGE_SCAN_MAX`] raw bytes; beyond
/// that `reset` is set and no events are returned. A cursor past EOF (file
/// truncated/replaced) restarts from 0.
pub fn read_range(
    session_id: &str,
    path: &Path,
    from: u64,
    to: Option<u64>,
    agent: Option<&str>,
) -> std::io::Result<RangeRead> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RangeRead {
                tail: from,
                ..Default::default()
            })
        }
        Err(e) => return Err(e),
    };
    let len = file.metadata()?.len();
    let start = if from > len { 0 } else { from };
    let end = to.unwrap_or(len).min(len);
    if end.saturating_sub(start) > RANGE_SCAN_MAX {
        return Ok(RangeRead {
            events: Vec::new(),
            tail: tail_of(path, len)?,
            reset: true,
        });
    }
    file.seek(SeekFrom::Start(start))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut offset = start;
    let mut events = Vec::new();
    let mut bytes = 0usize;
    let mut buf = Vec::new();
    let opts = MapOpts { agent };
    while offset < end {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            break; // EOF / partial line
        }
        let line_start = offset;
        offset += n as u64;
        if n > LINE_MAX {
            continue;
        }
        for e in map_line(session_id, &buf, line_start, opts) {
            bytes += serde_json::to_vec(&e).map(|b| b.len()).unwrap_or(0);
            events.push(e);
        }
        if events.len() > REPLAY_MAX_EVENTS || bytes > REPLAY_MAX_BYTES {
            return Ok(RangeRead {
                events: Vec::new(),
                tail: tail_of(path, len)?,
                reset: true,
            });
        }
    }
    Ok(RangeRead {
        events,
        tail: offset,
        reset: false,
    })
}

/// Everything after byte `since`, plus the new tail cursor (uncapped for
/// small files; callers that must bound replay use [`read_range`]). A cursor
/// past EOF restarts from 0.
pub fn read_after(
    session_id: &str,
    path: &Path,
    since: u64,
) -> std::io::Result<(Vec<UiEvent>, u64)> {
    let r = read_range(session_id, path, since, None, None)?;
    Ok((r.events, r.tail))
}

/// Parse a cursor string (`"12345"`).
pub fn parse_cursor(s: &str) -> Option<u64> {
    s.trim().parse().ok()
}

// ──────────────────────────────── media ──────────────────────────────────────

/// Why a media request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// No line / wrong uuid / no image at that block path.
    NotFound,
    /// Not a PNG/JPEG/GIF/WebP by magic bytes.
    Unsupported,
    TooLarge,
    Io(String),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MRec {
    uuid: LStr,
    message: Obj<MMsg>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MMsg {
    content: MContent,
}

#[derive(Debug, Default)]
struct MContent(Vec<MBlock>);

impl<'de> Deserialize<'de> for MContent {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(MContent(Seq::<MBlock>::deserialize(d)?.0))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MBlock {
    #[serde(rename = "type")]
    ty: LStr,
    source: Obj<MSource>,
    content: MContent,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MSource {
    #[serde(rename = "type")]
    ty: LStr,
    data: LStr,
}

/// Detect an allowed image type from magic bytes.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Read the image at `block_path` of the record at byte `offset`, verifying
/// the record uuid (`_` = a record without uuid). Returns `(mime, bytes)`
/// with the mime derived from the magic bytes.
pub fn read_media(
    path: &Path,
    offset: u64,
    uuid: &str,
    block_path: &[usize],
) -> Result<(&'static str, Vec<u8>), MediaError> {
    use base64::Engine;
    let mut file = File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => MediaError::NotFound,
        _ => MediaError::Io(e.to_string()),
    })?;
    let len = file
        .metadata()
        .map_err(|e| MediaError::Io(e.to_string()))?
        .len();
    if offset >= len {
        return Err(MediaError::NotFound);
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| MediaError::Io(e.to_string()))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file).take(LINE_MAX as u64 + 1);
    let mut line = Vec::new();
    reader
        .read_until(b'\n', &mut line)
        .map_err(|e| MediaError::Io(e.to_string()))?;
    if line.len() > LINE_MAX {
        return Err(MediaError::TooLarge);
    }
    let rec: MRec = parse_line(&line).ok_or(MediaError::NotFound)?;
    let matches = match rec.uuid.get() {
        Some(u) => u == uuid,
        None => uuid == "_",
    };
    if !matches {
        return Err(MediaError::NotFound);
    }
    let mut blocks = &rec.message.get().ok_or(MediaError::NotFound)?.content.0;
    let mut found: Option<&MBlock> = None;
    for (depth, idx) in block_path.iter().enumerate() {
        let b = blocks.get(*idx).ok_or(MediaError::NotFound)?;
        if depth + 1 == block_path.len() {
            found = Some(b);
        } else {
            blocks = &b.content.0;
        }
    }
    let block = found.ok_or(MediaError::NotFound)?;
    if !block.ty.is("image") {
        return Err(MediaError::NotFound);
    }
    let source = block.source.get().ok_or(MediaError::NotFound)?;
    if source.ty.get().is_some_and(|t| t != "base64") {
        return Err(MediaError::NotFound);
    }
    let data = source.data.get().ok_or(MediaError::NotFound)?;
    if data.len() > MEDIA_MAX / 3 * 4 + 8 {
        return Err(MediaError::TooLarge);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|_| MediaError::Unsupported)?;
    if bytes.len() > MEDIA_MAX {
        return Err(MediaError::TooLarge);
    }
    let mime = sniff_image(&bytes).ok_or(MediaError::Unsupported)?;
    Ok((mime, bytes))
}

// ─────────────────────────────── timeline ────────────────────────────────────

/// Mother Claude's own per-session events (decision rows, …), kept in memory
/// and merged into the session's view by timestamp. Capped per session.
#[derive(Debug, Default)]
pub struct Timeline {
    events: Mutex<HashMap<String, VecDeque<UiEvent>>>,
}

impl Timeline {
    const CAP: usize = 200;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, session_id: &str, event: UiEvent) {
        if let Ok(mut m) = self.events.lock() {
            let q = m.entry(session_id.to_string()).or_default();
            q.push_back(event);
            while q.len() > Self::CAP {
                q.pop_front();
            }
        }
    }

    pub fn list(&self, session_id: &str) -> Vec<UiEvent> {
        self.events
            .lock()
            .ok()
            .and_then(|m| m.get(session_id).map(|q| q.iter().cloned().collect()))
            .unwrap_or_default()
    }

    /// Merge this session's timeline events into a page (by timestamp):
    /// events inside the page's time range, plus — for the newest page —
    /// anything after it.
    pub fn merge_into(&self, session_id: &str, page: &mut TranscriptPage, newest: bool) {
        let extra = self.list(session_id);
        if extra.is_empty() {
            return;
        }
        let lo = page.events.iter().filter_map(|e| e.ts).min();
        let hi = page.events.iter().filter_map(|e| e.ts).max();
        let keep: Vec<UiEvent> = extra
            .into_iter()
            .filter(|e| match (e.ts, lo, hi) {
                (Some(t), Some(lo), Some(hi)) => t >= lo && (t <= hi || newest),
                (_, None, _) => newest,
                _ => false,
            })
            .filter(|e| !page.events.iter().any(|p| p.id == e.id))
            .collect();
        if keep.is_empty() {
            return;
        }
        page.events.extend(keep);
        page.events.sort_by_key(|e| e.ts.unwrap_or(i64::MAX));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: &str = concat!(
        r#"{"type":"user","uuid":"u1","timestamp":"2026-06-23T08:00:00.000Z","message":{"role":"user","content":"hello there"}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a1","message":{"id":"msg_1","role":"assistant","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Hi"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la","description":"List files"}}]}}"#,
        "\n",
        r#"{"type":"user","uuid":"u2","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"a\nb","is_error":false}]}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a2","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"sub"}]}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a3","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"TodoWrite","input":{"todos":[{"content":"x","status":"in_progress","activeForm":"Doing x"}]}}]}}"#,
        "\n",
        r#"{"type":"system","uuid":"s1","subtype":"compact_boundary","content":"Conversation compacted"}"#,
        "\n",
        r#"{"type":"last-prompt","uuid":"n1"}"#,
        "\n",
    );

    fn write(dir: &tempfile::TempDir, text: &str) -> std::path::PathBuf {
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn maps_the_basic_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, LINES);
        let page = read_page("s", &path, None, 300).unwrap();
        let kinds: Vec<UiKind> = page.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                UiKind::Prompt,
                UiKind::Thinking,
                UiKind::Text,
                UiKind::Tool,
                UiKind::Result,
                UiKind::Todos,
                UiKind::Compact
            ]
        );
        assert_eq!(page.events[3].tool.as_ref().unwrap().summary, "List files");
        assert_eq!(page.events[4].result.as_ref().unwrap().tool_use_id, "t1");
        assert_eq!(
            page.events[5].todos.as_ref().unwrap()[0].status,
            TodoStatus::InProgress
        );
        assert_eq!(page.events[0].id, "u1:0");
        assert_eq!(page.events[2].id, "a1:1");
        assert_eq!(page.events[2].message_id.as_deref(), Some("msg_1"));
        assert!(page.events[0].ts.is_some());
        assert_eq!(page.tail, LINES.len().to_string());
        assert!(page.before.is_none());
    }

    #[test]
    fn pages_backwards_and_reads_after() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, LINES);
        let last = read_page("s", &path, None, 1).unwrap();
        assert_eq!(last.events.len(), 1);
        assert_eq!(last.events[0].kind, UiKind::Compact);
        let before = parse_cursor(last.before.as_deref().unwrap()).unwrap();
        let prev = read_page("s", &path, Some(before), 1).unwrap();
        assert_eq!(prev.events[0].kind, UiKind::Todos);
        // Walk all the way back: every event exactly once, then no cursor.
        let mut all = Vec::new();
        let mut cursor = None;
        loop {
            let p = read_page("s", &path, cursor, 2).unwrap();
            let mut evs = p.events.clone();
            evs.extend(all);
            all = evs;
            match p.before {
                Some(b) => cursor = parse_cursor(&b),
                None => break,
            }
        }
        assert_eq!(all.len(), 7);

        // Everything after the first line.
        let first_len = LINES.find('\n').unwrap() as u64 + 1;
        let (events, tail) = read_after("s", &path, first_len).unwrap();
        assert_eq!(events[0].kind, UiKind::Thinking);
        assert_eq!(tail, LINES.len() as u64);
        // Cursor past EOF restarts from 0.
        let (events, _) = read_after("s", &path, 10_000_000).unwrap();
        assert_eq!(events[0].kind, UiKind::Prompt);
        // Missing file.
        let (events, tail) = read_after("s", &dir.path().join("nope"), 5).unwrap();
        assert!(events.is_empty());
        assert_eq!(tail, 5);
    }

    #[test]
    fn partial_trailing_line_is_not_consumed_either_way() {
        let dir = tempfile::tempdir().unwrap();
        let first = LINES.lines().next().unwrap();
        let path = write(&dir, &format!("{first}\n{{\"type\":\"user\""));
        let (events, tail) = read_after("s", &path, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(tail, first.len() as u64 + 1);
        let page = read_page("s", &path, None, 10).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.tail, (first.len() + 1).to_string());
    }

    #[test]
    fn noise_is_filtered() {
        let lines = concat!(
            r#"{"type":"queue-operation","operation":"enqueue","content":"x"}"#,
            "\n",
            r#"{"type":"attachment","uuid":"x1","attachment":{"type":"hook_non_blocking_error","stderr":"ECONNREFUSED"}}"#,
            "\n",
            r#"{"type":"attachment","uuid":"x2","attachment":{"type":"skill_listing","content":"lots"}}"#,
            "\n",
            r#"{"type":"ai-title","aiTitle":"t"}"#,
            "\n",
            r#"{"type":"custom-title","customTitle":"t"}"#,
            "\n",
            r#"{"type":"mode","mode":"normal"}"#,
            "\n",
            r#"{"type":"permission-mode","permissionMode":"auto"}"#,
            "\n",
            r#"{"type":"bridge-session","bridgeSessionId":"cse","ownerAccountUuid":"SECRET"}"#,
            "\n",
            r#"{"type":"file-history-snapshot","snapshot":{}}"#,
            "\n",
            r#"{"type":"cost-state","totalCostUSD":1}"#,
            "\n",
            r#"{"type":"user","uuid":"m1","isMeta":true,"message":{"content":"<system-reminder>x</system-reminder>"}}"#,
            "\n",
            r#"{"type":"system","uuid":"s2","subtype":"stop_hook_summary","hookAdditionalContext":[]}"#,
            "\n",
            r#"{"type":"system","uuid":"s3","subtype":"turn_duration","durationMs":5}"#,
            "\n",
            r#"{"type":"user","uuid":"p1","message":{"content":"real prompt"}}"#,
            "\n",
        );
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, lines);
        let page = read_page("s", &path, None, 100).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].text.as_deref(), Some("real prompt"));
        assert!(!serde_json::to_string(&page).unwrap().contains("SECRET"));
    }

    #[test]
    fn images_become_refs_and_media_is_extracted() {
        use base64::Engine;
        let png = [
            0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13,
        ];
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        let svg = base64::engine::general_purpose::STANDARD.encode(b"<svg onload=alert(1)>");
        let first = r#"{"type":"user","uuid":"u0","message":{"content":"look"}}"#;
        let img_line = format!(
            r#"{{"type":"user","uuid":"img-1","message":{{"content":[{{"type":"text","text":"see"}},{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{b64}"}}}},{{"type":"tool_result","tool_use_id":"t9","content":[{{"type":"text","text":"shot"}},{{"type":"image","source":{{"type":"base64","media_type":"image/svg+xml","data":"{svg}"}}}}]}}]}}}}"#
        );
        let text = format!("{first}\n{img_line}\n");
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, &text);
        let page = read_page("sid", &path, None, 100).unwrap();
        let off = first.len() + 1;
        let imgs: Vec<&UiEvent> = page
            .events
            .iter()
            .filter(|e| e.kind == UiKind::Image)
            .collect();
        assert_eq!(imgs.len(), 2);
        assert_eq!(
            imgs[0].image.as_ref().unwrap().reference,
            format!("/api/sessions/sid/media/{off}/img-1/1")
        );
        assert_eq!(
            imgs[1].image.as_ref().unwrap().reference,
            format!("/api/sessions/sid/media/{off}/img-1/2.1")
        );
        // Never inline base64.
        assert!(!serde_json::to_string(&page).unwrap().contains(&b64));

        let (mime, bytes) = read_media(&path, off as u64, "img-1", &[1]).unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, png);
        // SVG by magic bytes is refused whatever the recorded type says.
        assert_eq!(
            read_media(&path, off as u64, "img-1", &[2, 1]),
            Err(MediaError::Unsupported)
        );
        // Wrong uuid / wrong offset / not an image.
        assert_eq!(
            read_media(&path, off as u64, "other", &[1]),
            Err(MediaError::NotFound)
        );
        assert_eq!(
            read_media(&path, 0, "img-1", &[1]),
            Err(MediaError::NotFound)
        );
        assert_eq!(
            read_media(&path, off as u64, "img-1", &[0]),
            Err(MediaError::NotFound)
        );
        assert_eq!(
            read_media(&path, off as u64, "img-1", &[9]),
            Err(MediaError::NotFound)
        );
    }

    #[test]
    fn results_patches_answers_tasks_and_relays() {
        let lines = concat!(
            r#"{"type":"user","uuid":"r1","cwd":"/p","message":{"content":[{"type":"tool_result","tool_use_id":"e1","content":"ok"}]},"toolUseResult":{"filePath":"/p/src/a.rs","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":2,"lines":["-a","+b","+c"]}],"originalFile":"HUGE"}}"#,
            "\n",
            r#"{"type":"user","uuid":"r2","message":{"content":[{"type":"tool_result","tool_use_id":"b1","content":"x"}]},"toolUseResult":{"stdout":"out","stderr":"","interrupted":false}}"#,
            "\n",
            r#"{"type":"user","uuid":"r3","message":{"content":[{"type":"tool_result","tool_use_id":"q1","content":"answered"}]},"toolUseResult":{"questions":[{"question":"DB?","options":[{"label":"pg"},{"label":"sqlite"}],"multiSelect":false}],"answers":{"DB?":"pg"}}}"#,
            "\n",
            r#"{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<task-id>b77</task-id>\n<status>completed</status>\n<summary>Build done</summary>\n</task-notification>"}"#,
            "\n",
            r#"{"type":"user","uuid":"r4","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>\n<task-id>b77</task-id>\n<status>completed</status>\n<summary>Build done</summary>\n</task-notification>"}}"#,
            "\n",
            r#"{"type":"user","uuid":"r5","origin":{"kind":"task-notification"},"message":{"content":"[[MC-RELAY abc123]] please also run the tests"}}"#,
            "\n",
            r#"{"type":"attachment","uuid":"r6","attachment":{"type":"hook_additional_context","content":["[[MC-RELAY abc123]] and lint"],"hookName":"PostToolUse:Bash"}}"#,
            "\n",
            r#"{"type":"system","uuid":"r7","subtype":"stop_hook_summary","hookAdditionalContext":["[[MC-RELAY abc123]] one more"]}"#,
            "\n",
            r#"{"type":"user","uuid":"r8","message":{"content":"<command-name>/effort</command-name>\n<command-args>low</command-args>"}}"#,
            "\n",
            r#"{"type":"system","uuid":"r9","subtype":"api_error","error":{"formatted":"529 Overloaded"},"retryAttempt":2,"maxRetries":10}"#,
            "\n",
            r#"{"type":"assistant","uuid":"r10","message":{"content":[{"type":"tool_use","id":"p1","name":"ExitPlanMode","input":{"plan":"1. do it"}}]}}"#,
            "\n",
        );
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, lines);
        let page = read_page("s", &path, None, 100).unwrap();
        let e = |id: &str| page.events.iter().find(|e| e.id.starts_with(id)).unwrap();
        let patch = e("r1").result.as_ref().unwrap().patch.as_ref().unwrap();
        assert_eq!(patch.path, "src/a.rs");
        assert_eq!(patch.hunks.len(), 1);
        assert_eq!(
            e("r2").result.as_ref().unwrap().text.as_deref(),
            Some("out")
        );
        let q = e("r3");
        assert_eq!(q.kind, UiKind::Question);
        assert_eq!(
            q.question.as_ref().unwrap().answers.as_ref().unwrap()["DB?"],
            "pg"
        );
        assert_eq!(q.question.as_ref().unwrap().questions.len(), 1);
        // One Task event per notification (from its enqueue record), none
        // from the user record that repeats it.
        let tasks: Vec<&UiEvent> = page
            .events
            .iter()
            .filter(|e| e.kind == UiKind::Task)
            .collect();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task.as_ref().unwrap().task_id, "b77");
        assert!(page.events.iter().all(|e| !e.id.starts_with("r4")));
        assert_eq!(e("r5").kind, UiKind::Relayed);
        assert_eq!(e("r5").text.as_deref(), Some("please also run the tests"));
        assert_eq!(e("r6").kind, UiKind::Relayed);
        assert_eq!(e("r7").kind, UiKind::Relayed);
        assert_eq!(e("r8").text.as_deref(), Some("/effort low"));
        assert_eq!(
            e("r9").text.as_deref(),
            Some("529 Overloaded (retry 2 of 10)")
        );
        assert_eq!(e("r10").plan.as_ref().unwrap().text, "1. do it");
    }

    #[test]
    fn bounded_ranges_reset_instead_of_flooding() {
        let mut text = String::new();
        for i in 0..(REPLAY_MAX_EVENTS + 20) {
            text += &format!(r#"{{"type":"user","uuid":"u{i}","message":{{"content":"p{i}"}}}}"#);
            text += "\n";
        }
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, &text);
        let r = read_range("s", &path, 0, None, None).unwrap();
        assert!(r.reset);
        assert!(r.events.is_empty());
        assert_eq!(r.tail, text.len() as u64);
        let small = read_range("s", &path, text.len() as u64 - 50, None, None).unwrap();
        assert!(!small.reset);
    }

    #[test]
    fn agent_transcripts_keep_sidechain_records() {
        let line = r#"{"type":"assistant","uuid":"x","isSidechain":true,"agentId":"a1","message":{"content":[{"type":"text","text":"sub work"}]}}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, &format!("{line}\n"));
        assert!(read_page("s", &path, None, 10).unwrap().events.is_empty());
        let page = read_page_opts(
            "s",
            &path,
            PageOpts {
                before: None,
                limit: 10,
                agent: Some("a1"),
            },
        )
        .unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].agent_id.as_deref(), Some("a1"));
    }

    #[test]
    fn backward_scan_handles_huge_lines_and_budget() {
        // A line bigger than a back chunk in the middle.
        let big = format!(
            r#"{{"type":"user","uuid":"big","message":{{"content":"{}"}}}}"#,
            "x".repeat(700 * 1024)
        );
        let text = format!(
            "{}\n{big}\n{}\n",
            r#"{"type":"user","uuid":"a","message":{"content":"first"}}"#,
            r#"{"type":"user","uuid":"b","message":{"content":"last"}}"#
        );
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, &text);
        let page = read_page("s", &path, None, 300).unwrap();
        let ids: Vec<&str> = page.events.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["a:0", "big:0", "b:0"]);
        assert!(page.events[1].text.as_ref().unwrap().len() <= 65536);
    }

    #[test]
    fn map_event_compat_and_summaries() {
        let ev: TranscriptEvent = serde_json::from_str(
            r#"{"type":"assistant","uuid":"z","message":{"content":[{"type":"text","text":"yo"}]}}"#,
        )
        .unwrap();
        let out = map_event("s", &ev, "@5");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "z:0");
        let (t, cut) = truncate_bytes("héllo", 2);
        assert!(cut);
        assert_eq!(t, "h…");
        assert_eq!(
            tool_summary_in(
                "Read",
                Some(&serde_json::json!({ "file_path": "/a/b/c.rs" })),
                Some("/a")
            ),
            "b/c.rs"
        );
        assert_eq!(tool_summary("Mystery", None), "Mystery");
        assert_eq!(
            tool_summary("mcp__figma__get_node", None),
            "figma · get_node"
        );
        assert_eq!(
            tool_summary(
                "TodoWrite",
                Some(
                    &serde_json::json!({"todos":[{"content":"a","status":"in_progress","activeForm":"Doing a"}]})
                )
            ),
            "Doing a"
        );
    }

    #[test]
    fn timeline_is_capped_and_merged() {
        let t = Timeline::new();
        for i in 0..250 {
            t.push(
                "s",
                UiEvent {
                    id: i.to_string(),
                    ts: Some(i),
                    ..Default::default()
                },
            );
        }
        let l = t.list("s");
        assert_eq!(l.len(), 200);
        assert_eq!(l[0].id, "50");
        assert!(t.list("other").is_empty());
        let mut page = TranscriptPage {
            events: vec![UiEvent {
                id: "x".into(),
                ts: Some(100),
                ..Default::default()
            }],
            ..Default::default()
        };
        t.merge_into("s", &mut page, true);
        assert_eq!(page.events.first().unwrap().id, "x");
        assert!(page.events.len() > 100);
    }
}
