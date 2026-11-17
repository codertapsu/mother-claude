//! Messages as a conversation keeps them, and as Claude Code is given them.
//!
//! **Fingerprints.** A client of the Claude API sends the whole history with
//! each request. Each message gets a fingerprint that stays the same however
//! a client re-serialises what it got back (SDK objects add `citations:
//! null`, `caller`, cache markers; many drop earlier thinking), so a request
//! whose history is a conversation's continues that conversation.
//!
//! **A history the server never saw** (a conversation begun elsewhere) is
//! written into the first message, so the model still has it: Claude Code
//! takes user messages only.

use serde_json::{json, Map, Value};

use super::wire::{ApiError, Message, Role};
use crate::claude::secrets::sha256_hex;

/// A source of an image or document, by what it is (large data by its
/// hash).
fn canonical_source(src: &Value) -> Value {
    let kind = src.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "base64" => json!({
            "type": "base64",
            "media_type": src.get("media_type"),
            "sha256": src.get("data").and_then(Value::as_str).map(|d| sha256_hex(d.as_bytes())),
        }),
        "url" => json!({ "type": "url", "url": src.get("url") }),
        "file" => json!({ "type": "file", "file_id": src.get("file_id") }),
        "text" => json!({ "type": "text", "data": src.get("data") }),
        "content" => json!({ "type": "content", "content": canonical_content(src.get("content")) }),
        _ => src.clone(),
    }
}

/// A content value (string or blocks) as canonical blocks.
fn canonical_content(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => json!([{ "type": "text", "text": s }]),
        Some(Value::Array(items)) => {
            Value::Array(items.iter().filter_map(canonical_block).collect())
        }
        _ => json!([]),
    }
}

/// One block reduced to what it means. `None`: it does not count (thinking,
/// which clients may drop from earlier turns).
pub fn canonical_block(b: &Value) -> Option<Value> {
    let kind = b.get("type").and_then(Value::as_str).unwrap_or("");
    Some(match kind {
        "text" => json!({ "type": "text", "text": b.get("text") }),
        "image" | "document" => {
            json!({ "type": kind, "source": canonical_source(b.get("source").unwrap_or(&Value::Null)) })
        }
        "tool_use" => json!({
            "type": "tool_use",
            "id": b.get("id"),
            "name": b.get("name"),
            "input": b.get("input").cloned().unwrap_or_else(|| json!({})),
        }),
        "tool_result" => json!({
            "type": "tool_result",
            "tool_use_id": b.get("tool_use_id"),
            "is_error": b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            "content": canonical_content(b.get("content")),
        }),
        "thinking" | "redacted_thinking" => return None,
        _ => {
            let mut o: Map<String, Value> = b.as_object().cloned().unwrap_or_default();
            o.remove("cache_control");
            o.retain(|_, v| !v.is_null());
            Value::Object(o)
        }
    })
}

/// A message's fingerprint.
pub fn fingerprint(m: &Message) -> String {
    let blocks: Vec<Value> = m.content.iter().filter_map(canonical_block).collect();
    let canonical = json!({ "role": m.role.as_str(), "content": blocks });
    sha256_hex(canonical.to_string().as_bytes())
}

pub fn fingerprints(ms: &[Message]) -> Vec<String> {
    ms.iter().map(fingerprint).collect()
}

/// A fingerprint of anything (a system prompt, a tool list).
pub fn digest(v: &Value) -> String {
    sha256_hex(v.to_string().as_bytes())
}

/// A tool result the client sent: which call, and its answer for the
/// model (MCP content).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: Vec<Value>,
    pub is_error: bool,
}

/// A user message's tool results, and the rest of it.
pub fn split_tool_results(blocks: &[Value]) -> (Vec<ToolResult>, Vec<Value>) {
    let mut results = Vec::new();
    let mut rest = Vec::new();
    for b in blocks {
        if b.get("type").and_then(Value::as_str) == Some("tool_result") {
            let content = match b.get("content") {
                Some(Value::String(s)) => vec![json!({ "type": "text", "text": s })],
                Some(Value::Array(items)) => items.iter().map(strip_cache_marks).collect(),
                _ => Vec::new(),
            };
            results.push(ToolResult {
                tool_use_id: b
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                content,
                is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            });
        } else {
            rest.push(b.clone());
        }
    }
    (results, rest)
}

/// A block without the client's cache breakpoints: Claude Code places its
/// own, and the API allows four.
fn strip_cache_marks(b: &Value) -> Value {
    let mut b = b.clone();
    if let Some(o) = b.as_object_mut() {
        o.remove("cache_control");
    }
    b
}

/// Refuses what Claude Code cannot be given: a Files API reference (this
/// server keeps no files of the client's).
fn check_block(b: &Value, at: &str) -> Result<(), ApiError> {
    if b.pointer("/source/type").and_then(Value::as_str) == Some("file") {
        return Err(ApiError::invalid(format!(
            "{at}.source: file_id sources need the Files API, which Claude Code does not serve \
             here (Mother Claude); send the bytes (base64) or a URL"
        )));
    }
    Ok(())
}

/// A user message's blocks as Claude Code takes them.
pub fn for_claude_code(blocks: &[Value], at: &str) -> Result<Vec<Value>, ApiError> {
    let mut out = Vec::with_capacity(blocks.len());
    for (i, b) in blocks.iter().enumerate() {
        check_block(b, &format!("{at}.{i}"))?;
        out.push(strip_cache_marks(b));
    }
    Ok(out)
}

fn text(t: impl Into<String>) -> Value {
    json!({ "type": "text", "text": t.into() })
}

/// A history Claude Code never saw, written into a first message: the
/// conversation so far as labelled turns (images and documents where they
/// were; tool calls and their results described; thinking left out), then
/// the new message.
pub fn seeded_first_message(history: &[Message], last: &[Value]) -> Result<Vec<Value>, ApiError> {
    let mut out = vec![text(
        "<conversation_so_far>\nThe conversation so far, begun before this one; continue it as \
         the assistant.",
    )];
    for (i, m) in history.iter().enumerate() {
        let who = match m.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        out.push(text(format!("\n\n{who}:")));
        for (j, b) in m.content.iter().enumerate() {
            let at = format!("messages.{i}.content.{j}");
            match b.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" => out.push(text(
                    b.get("text").and_then(Value::as_str).unwrap_or_default(),
                )),
                "image" | "document" => {
                    check_block(b, &at)?;
                    out.push(strip_cache_marks(b));
                }
                "tool_use" => out.push(text(format!(
                    "[called the tool `{}` with {}]",
                    b.get("name").and_then(Value::as_str).unwrap_or("?"),
                    b.get("input").cloned().unwrap_or_else(|| json!({}))
                ))),
                "tool_result" => {
                    let (results, _) = split_tool_results(std::slice::from_ref(b));
                    for r in results {
                        out.push(text(format!(
                            "[{} of the tool call {}:]",
                            if r.is_error { "error" } else { "result" },
                            r.tool_use_id
                        )));
                        for c in r.content {
                            if c.get("type").and_then(Value::as_str) == Some("image") {
                                check_block(&c, &at)?;
                            }
                            out.push(c);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out.push(text("\n</conversation_so_far>\n\nThe user now says:\n"));
    out.extend(for_claude_code(last, "messages.last.content")?);
    Ok(merge_text(out))
}

/// A user message whose tool results answer calls Claude Code is no longer
/// waiting on (the turn that made them ended — a restart, a long wait):
/// the results written out, for the model to read, the rest as it is.
pub fn describe_tool_results(blocks: &[Value], at: &str) -> Result<Vec<Value>, ApiError> {
    let mut out = Vec::with_capacity(blocks.len());
    for (i, b) in blocks.iter().enumerate() {
        if b.get("type").and_then(Value::as_str) != Some("tool_result") {
            check_block(b, &format!("{at}.{i}"))?;
            out.push(strip_cache_marks(b));
            continue;
        }
        let (results, _) = split_tool_results(std::slice::from_ref(b));
        for r in results {
            out.push(text(format!(
                "[{} of the tool call {}:]\n",
                if r.is_error { "error" } else { "result" },
                r.tool_use_id
            )));
            for c in r.content {
                if c.get("type").and_then(Value::as_str) == Some("image") {
                    check_block(&c, &format!("{at}.{i}"))?;
                }
                out.push(c);
            }
        }
    }
    Ok(merge_text(out))
}

/// A message a conversation kept ([`for_record`]) as a message again: an
/// image or document whose bytes were left out becomes a note of it.
pub fn from_record(v: &Value) -> Option<Message> {
    let role = match v.get("role").and_then(Value::as_str)? {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => return None,
    };
    let content = v
        .get("content")
        .and_then(Value::as_array)?
        .iter()
        .map(|b| {
            let omitted = b
                .pointer("/source/data/omitted")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if omitted {
                let kind = b.get("type").and_then(Value::as_str).unwrap_or("file");
                text(format!("[{kind} not kept]"))
            } else {
                b.clone()
            }
        })
        .collect();
    Some(Message { role, content })
}

/// Adjacent text blocks as one.
fn merge_text(blocks: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(blocks.len());
    for b in blocks {
        let is_text = |v: &Value| {
            v.get("type").and_then(Value::as_str) == Some("text")
                && v.as_object().is_some_and(|o| o.len() == 2)
        };
        if is_text(&b) {
            if let Some(prev) = out.last_mut().filter(|p| is_text(p)) {
                let joined = format!(
                    "{}{}",
                    prev.get("text").and_then(Value::as_str).unwrap_or_default(),
                    b.get("text").and_then(Value::as_str).unwrap_or_default()
                );
                *prev = text(joined);
                continue;
            }
        }
        out.push(b);
    }
    out
}

/// What a stored conversation keeps of a message: everything but the bytes
/// of images and documents (their size and hash instead).
pub fn for_record(m: &Message) -> Value {
    let content: Vec<Value> = m
        .content
        .iter()
        .map(|b| {
            let mut b = b.clone();
            if let Some(src) = b.get_mut("source").and_then(Value::as_object_mut) {
                if let Some(data) = src.get("data").and_then(Value::as_str) {
                    if src.get("type").and_then(Value::as_str) == Some("base64") {
                        let omitted = json!({
                            "omitted": true,
                            "bytes": data.len() / 4 * 3,
                            "sha256": sha256_hex(data.as_bytes()),
                        });
                        src.insert("data".into(), omitted);
                    }
                }
            }
            b
        })
        .collect();
    json!({ "role": m.role.as_str(), "content": content })
}

#[cfg(test)]
mod tests;
