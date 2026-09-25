//! Transcript normalisation into [`UiEvent`]s (design §3.6, §4.4).
//!
//! Owner: **B2 (data layer)**. The foundation ships a *naive* mapping so the
//! REST transcript, WS `transcript` frames and `sub{since}` replay work today:
//!
//! * [`map_event`]: one raw record → zero or more UiEvents (prompts, text,
//!   thinking, tool calls with a short summary, tool results truncated to
//!   4 KB, image refs, compact boundaries, TodoWrite / AskUserQuestion /
//!   ExitPlanMode as their own kinds). Sidechain records are skipped.
//! * [`read_page`]: a forward streaming scan with a bounded ring buffer
//!   (never `read_to_string`); B2 replaces it with backwards chunked reading.
//! * [`read_after`]: everything after a byte cursor (for `sub{since}`).
//! * [`Timeline`]: Mother Claude's own events (e.g. hook-relayed messages, B4)
//!   merged into a session's view.
//!
//! Cursors are byte offsets rendered as decimal strings (opaque to clients).
//! B2 may change the internals and add fields; keep these signatures (or
//! update every caller in the same commit).

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

use serde_json::Value;

use super::model::{
    Question, Todo, TodoStatus, TranscriptPage, UiEvent, UiImage, UiKind, UiPlan, UiQuestion,
    UiResult, UiRole, UiTool,
};
use super::schema::{parse_transcript_line, ContentBlock, MessageContent, TranscriptEvent};

/// Tool-result text kept per event.
pub const RESULT_TEXT_MAX: usize = 4 * 1024;
/// Largest page a client may ask for.
pub const PAGE_MAX: usize = 300;
/// Lines longer than this are skipped by the naive reader.
const LINE_MAX: usize = 8 * 1024 * 1024;

fn parse_ts(ts: Option<&str>) -> Option<i64> {
    ts.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| d.timestamp_millis())
}

/// Truncate to at most `max` bytes on a char boundary. Returns (text, cut?).
pub fn truncate_bytes(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}…", &s[..end]), true)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// One-line description of a tool call (design §3.5).
pub fn tool_summary(name: &str, input: Option<&Value>) -> String {
    let get = |k: &str| {
        input
            .and_then(|i| i.get(k))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let s = match name {
        "Bash" => get("description").or_else(|| get("command")),
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            get("file_path").or_else(|| get("notebook_path"))
        }
        "Grep" | "Glob" => get("pattern"),
        "Agent" | "Task" => get("description"),
        "Workflow" => get("name"),
        "WebFetch" => get("url"),
        "WebSearch" => get("query"),
        "TodoWrite" => Some("Update the todo list".into()),
        _ => None,
    };
    truncate_chars(&s.unwrap_or_else(|| name.to_string()), 120)
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

fn result_text(block: &ContentBlock) -> String {
    match &block.content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => block.text.clone().unwrap_or_default(),
    }
}

/// Map one raw transcript record into UI events. `fallback_id` names the
/// record when it has no `uuid` (e.g. `"@<byte offset>"`).
pub fn map_event(session_id: &str, ev: &TranscriptEvent, fallback_id: &str) -> Vec<UiEvent> {
    if ev.is_sidechain == Some(true) {
        return Vec::new();
    }
    let base_id = ev.uuid.clone().unwrap_or_else(|| fallback_id.to_string());
    let ts = parse_ts(ev.timestamp.as_deref());
    let mut out = Vec::new();
    let mk = |idx: usize, role: UiRole, kind: UiKind| UiEvent {
        id: format!("{base_id}:{idx}"),
        ts,
        role,
        kind,
        ..Default::default()
    };

    match ev.event_type.as_str() {
        "user" => {
            let Some(content) = ev.message.as_ref().and_then(|m| m.content.as_ref()) else {
                return out;
            };
            let is_meta = ev.extra.get("isMeta").and_then(Value::as_bool) == Some(true);
            match content {
                MessageContent::Text(t) => {
                    if !t.trim().is_empty() && !is_meta {
                        let mut e = mk(0, UiRole::User, UiKind::Prompt);
                        e.text = Some(t.clone());
                        out.push(e);
                    }
                }
                MessageContent::Blocks(blocks) => {
                    for (i, b) in blocks.iter().enumerate() {
                        match b.block_type.as_str() {
                            "text" if !is_meta => {
                                if let Some(t) = b.text.as_ref().filter(|t| !t.trim().is_empty()) {
                                    let mut e = mk(i, UiRole::User, UiKind::Prompt);
                                    e.text = Some(t.clone());
                                    out.push(e);
                                }
                            }
                            "tool_result" => {
                                let (text, cut) = truncate_bytes(&result_text(b), RESULT_TEXT_MAX);
                                let mut e = mk(i, UiRole::User, UiKind::Result);
                                e.result = Some(UiResult {
                                    tool_use_id: b.tool_use_id.clone().unwrap_or_default(),
                                    is_error: b.is_error.unwrap_or(false),
                                    text: Some(text),
                                    patch: None,
                                    truncated: cut.then_some(true),
                                });
                                out.push(e);
                            }
                            "image" => {
                                let media_type = b
                                    .extra
                                    .get("source")
                                    .and_then(|s| s.get("media_type"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("image/png")
                                    .to_string();
                                let mut e = mk(i, UiRole::User, UiKind::Image);
                                e.image = Some(UiImage {
                                    media_type,
                                    reference: format!(
                                        "/api/sessions/{session_id}/media/{base_id}/{i}"
                                    ),
                                });
                                out.push(e);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        "assistant" => {
            let Some(MessageContent::Blocks(blocks)) =
                ev.message.as_ref().and_then(|m| m.content.as_ref())
            else {
                if let Some(MessageContent::Text(t)) =
                    ev.message.as_ref().and_then(|m| m.content.as_ref())
                {
                    let mut e = mk(0, UiRole::Assistant, UiKind::Text);
                    e.text = Some(t.clone());
                    out.push(e);
                }
                return out;
            };
            for (i, b) in blocks.iter().enumerate() {
                match b.block_type.as_str() {
                    "text" => {
                        if let Some(t) = b.text.as_ref().filter(|t| !t.trim().is_empty()) {
                            let mut e = mk(i, UiRole::Assistant, UiKind::Text);
                            e.text = Some(t.clone());
                            out.push(e);
                        }
                    }
                    "thinking" => {
                        let t = b
                            .extra
                            .get("thinking")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| b.text.clone())
                            .unwrap_or_default();
                        if !t.trim().is_empty() {
                            let mut e = mk(i, UiRole::Assistant, UiKind::Thinking);
                            e.text = Some(t);
                            out.push(e);
                        }
                    }
                    "tool_use" => {
                        let name = b.name.clone().unwrap_or_default();
                        let input = b.input.as_ref();
                        let kind = match name.as_str() {
                            "TodoWrite" => UiKind::Todos,
                            "AskUserQuestion" => UiKind::Question,
                            "ExitPlanMode" => UiKind::Plan,
                            _ => UiKind::Tool,
                        };
                        let mut e = mk(i, UiRole::Assistant, kind);
                        e.tool = Some(UiTool {
                            id: b.id.clone().unwrap_or_default(),
                            name: name.clone(),
                            summary: tool_summary(&name, input),
                            input: input.cloned(),
                        });
                        match kind {
                            UiKind::Todos => e.todos = Some(todos_from(input)),
                            UiKind::Question => {
                                let questions: Vec<Question> = input
                                    .map(crate::pending::questions_from_input)
                                    .unwrap_or_default();
                                e.question = Some(UiQuestion {
                                    questions,
                                    answers: None,
                                });
                            }
                            UiKind::Plan => {
                                e.plan = Some(UiPlan {
                                    text: input
                                        .and_then(|i| i.get("plan"))
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string(),
                                })
                            }
                            _ => {}
                        }
                        out.push(e);
                    }
                    _ => {}
                }
            }
        }
        "system" => {
            let subtype = ev.subtype.as_deref().unwrap_or_default();
            let text = ev
                .content
                .as_ref()
                .and_then(Value::as_str)
                .map(str::to_string);
            let kind = match subtype {
                "compact_boundary" => Some(UiKind::Compact),
                "api_error" => Some(UiKind::Error),
                _ if text.is_some() => Some(UiKind::Notice),
                _ => None,
            };
            if let Some(kind) = kind {
                let mut e = mk(0, UiRole::System, kind);
                e.text = text;
                out.push(e);
            }
        }
        _ => {}
    }
    out
}

/// Parse lines from a reader, yielding `(start offset, event)`. Oversized and
/// unparseable lines are skipped; a trailing partial line is not consumed.
/// Returns the offset just past the last complete line.
fn scan<R: BufRead>(
    mut reader: R,
    start: u64,
    mut each: impl FnMut(u64, TranscriptEvent),
) -> std::io::Result<u64> {
    let mut offset = start;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            // EOF, or a partial line still being written.
            return Ok(offset);
        }
        let line_start = offset;
        offset += n as u64;
        if n > LINE_MAX {
            continue;
        }
        if let Some(ev) = parse_transcript_line(&String::from_utf8_lossy(&buf)) {
            each(line_start, ev);
        }
    }
}

/// A page of UI events ending before byte `before` (or at EOF), newest last,
/// at most `limit` (≤ [`PAGE_MAX`]). Missing file ⇒ empty page.
pub fn read_page(
    session_id: &str,
    path: &Path,
    before: Option<u64>,
    limit: usize,
) -> std::io::Result<TranscriptPage> {
    let limit = limit.clamp(1, PAGE_MAX);
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
    let end = before.unwrap_or(len).min(len);
    // Keep whole records (so a page boundary never splits one) holding at most
    // `limit` events — more only when a single record alone exceeds it.
    let mut ring: VecDeque<(u64, Vec<UiEvent>)> = VecDeque::new();
    let mut count = 0usize;
    let reader = BufReader::new(file);
    let tail = scan(reader, 0, |off, ev| {
        if off >= end {
            return;
        }
        let events = map_event(session_id, &ev, &format!("@{off}"));
        if events.is_empty() {
            return;
        }
        count += events.len();
        ring.push_back((off, events));
        while count > limit && ring.len() > 1 {
            if let Some((_, dropped)) = ring.pop_front() {
                count -= dropped.len();
            }
        }
    })?;
    let first_off = ring.front().map(|(off, _)| *off);
    let events: Vec<UiEvent> = ring.into_iter().flat_map(|(_, evs)| evs).collect();
    Ok(TranscriptPage {
        events,
        before: first_off.filter(|o| *o > 0).map(|o| o.to_string()),
        tail: tail.to_string(),
    })
}

/// Everything after byte `since`, plus the new tail cursor. A cursor past EOF
/// (file truncated/rotated) restarts from 0.
pub fn read_after(
    session_id: &str,
    path: &Path,
    since: u64,
) -> std::io::Result<(Vec<UiEvent>, u64)> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), since)),
        Err(e) => return Err(e),
    };
    let len = file.metadata()?.len();
    let start = if since > len { 0 } else { since };
    file.seek(SeekFrom::Start(start))?;
    let mut events = Vec::new();
    let tail = scan(BufReader::new(file), start, |off, ev| {
        events.extend(map_event(session_id, &ev, &format!("@{off}")));
    })?;
    Ok((events, tail))
}

/// Parse a cursor string (`"12345"`).
pub fn parse_cursor(s: &str) -> Option<u64> {
    s.trim().parse().ok()
}

/// Mother Claude's own per-session events (hook-relayed messages, …), kept in
/// memory and merged into the session's view. Capped per session.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: &str = concat!(
        r#"{"type":"user","uuid":"u1","timestamp":"2026-06-23T08:00:00.000Z","message":{"role":"user","content":"hello there"}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Hi"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la","description":"List files"}}]}}"#,
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

    #[test]
    fn maps_the_basic_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, LINES).unwrap();
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
        assert!(page.events[0].ts.is_some());
        assert_eq!(page.tail, LINES.len().to_string());
        assert!(page.before.is_none());
    }

    #[test]
    fn pages_backwards_and_reads_after() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, LINES).unwrap();
        let last = read_page("s", &path, None, 1).unwrap();
        assert_eq!(last.events.len(), 1);
        assert_eq!(last.events[0].kind, UiKind::Compact);
        let before = parse_cursor(last.before.as_deref().unwrap()).unwrap();
        let prev = read_page("s", &path, Some(before), 1).unwrap();
        assert_eq!(prev.events[0].kind, UiKind::Todos);

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
    fn partial_trailing_line_is_not_consumed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let first = LINES.lines().next().unwrap();
        std::fs::write(&path, format!("{first}\n{{\"type\":\"user\"")).unwrap();
        let (events, tail) = read_after("s", &path, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(tail, first.len() as u64 + 1);
    }

    #[test]
    fn truncation_and_summaries() {
        let (t, cut) = truncate_bytes("héllo", 2);
        assert!(cut);
        assert_eq!(t, "h…");
        assert_eq!(
            tool_summary("Read", Some(&serde_json::json!({ "file_path": "/a/b.rs" }))),
            "/a/b.rs"
        );
        assert_eq!(tool_summary("Mystery", None), "Mystery");
    }

    #[test]
    fn timeline_is_capped() {
        let t = Timeline::new();
        for i in 0..250 {
            t.push(
                "s",
                UiEvent {
                    id: i.to_string(),
                    ..Default::default()
                },
            );
        }
        let l = t.list("s");
        assert_eq!(l.len(), 200);
        assert_eq!(l[0].id, "50");
        assert!(t.list("other").is_empty());
    }
}
