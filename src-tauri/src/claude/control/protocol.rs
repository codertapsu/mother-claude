//! The stream-json control protocol spoken with an owned `claude -p` child
//! (design §5.1, verified against CLI 2.1.282 and the Agent SDK typings).
//!
//! Pure functions only: building the lines we write, and turning the lines the
//! CLI writes into dashboard types. The I/O lives in `control.rs`.
//!
//! Envelopes (nested, exactly as the CLI and the SDK use them):
//!
//! ```text
//! → {"type":"control_request","request_id":ID,"request":{"subtype":…,…}}
//! ← {"type":"control_response","response":{"subtype":"success","request_id":ID,"response":{…}}}
//! ← {"type":"control_response","response":{"subtype":"error","request_id":ID,"error":"…","error_code"?:"…"}}
//! ← {"type":"control_request","request_id":ID,"request":{"subtype":"can_use_tool",…}}   (CLI → host)
//! ← {"type":"control_cancel_request","request_id":ID}
//! → {"type":"user","message":{"role":"user","content":[…]},"parent_tool_use_id":null,"uuid":…}
//! ```

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::claude::model::{
    Activity, ActivityKind, CommandInfo, PendingItem, PendingKind, PendingSource, StreamFrame,
    StreamKind,
};
use crate::pending;

/// A CLI `error` control response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub message: String,
    pub code: Option<String>,
}

/// `get_settings.applied` — "runtime-resolved values … what will actually be
/// sent to the API" (verified against CLI 2.1.284:
/// `{model, effort: level|null, advisor, ultracode, ultracodeRequested,
/// ultracodeAvailable}`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppliedSettings {
    pub model: Option<String>,
    /// `None`: the model has no effort setting (Haiku).
    pub effort: Option<String>,
    /// In effect (requested and available).
    pub ultracode: bool,
    pub ultracode_requested: bool,
    /// The model and workspace can run it.
    pub ultracode_available: bool,
}

/// Parse a `get_settings` response (`None` without an `applied` object).
pub fn applied_settings(resp: &Value) -> Option<AppliedSettings> {
    let a = resp.get("applied")?.as_object()?;
    let text = |k: &str| {
        a.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .map(str::to_string)
    };
    let flag = |k: &str| a.get(k).and_then(Value::as_bool).unwrap_or(false);
    Some(AppliedSettings {
        model: text("model"),
        effort: text("effort").filter(|e| e.len() <= 20),
        ultracode: flag("ultracode"),
        ultracode_requested: flag("ultracodeRequested"),
        ultracode_available: flag("ultracodeAvailable"),
    })
}

/// Whether the model an `initialize` catalog entry describes is `model`
/// (by `resolvedModel` or `value`, ignoring case and a `[1m]` suffix).
fn catalog_matches(entry: &Value, model: &str) -> bool {
    let norm = |s: &str| s.trim().to_ascii_lowercase().replace("[1m]", "");
    let want = norm(model);
    ["resolvedModel", "value"]
        .iter()
        .filter_map(|k| entry.get(*k).and_then(Value::as_str))
        .any(|v| norm(v) == want)
}

/// `initialize.models[].supportsAdaptiveThinking` for `model` (an entry
/// without the key does not support it); `None` when the catalog does not
/// list the model.
pub fn catalog_adaptive_thinking(models: &[Value], model: &str) -> Option<bool> {
    models.iter().find(|m| catalog_matches(m, model)).map(|m| {
        m.get("supportsAdaptiveThinking")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}

/// `{"type":"control_request","request_id":ID,"request":REQUEST}`.
pub fn control_request(request_id: &str, request: Value) -> Value {
    json!({ "type": "control_request", "request_id": request_id, "request": request })
}

/// A success reply to a CLI → host control request.
pub fn control_success(request_id: &str, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request_id, "response": response },
    })
}

/// An error reply to a CLI → host control request.
pub fn control_error(request_id: &str, error: &str) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "error", "request_id": request_id, "error": error },
    })
}

/// Parse a `control_response` line: `(request_id, Ok(response) | Err(error))`.
pub fn parse_control_response(v: &Value) -> Option<(String, Result<Value, CliError>)> {
    let r = v.get("response")?;
    let id = r.get("request_id")?.as_str()?.to_string();
    let outcome = match r.get("subtype").and_then(Value::as_str) {
        Some("success") => Ok(r.get("response").cloned().unwrap_or(Value::Null)),
        _ => Err(CliError {
            message: r
                .get("error")
                .map(|e| match e {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "The session refused the request.".into()),
            code: r
                .get("error_code")
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
    };
    Some((id, outcome))
}

/// A stream-json user message. `content` blocks; `uuid` lets the CLI echo
/// which of our messages a turn answers (and lets us cancel a queued one).
pub fn user_message(content: Vec<Value>, uuid: Option<&str>) -> Value {
    let mut v = json!({
        "type": "user",
        "message": { "role": "user", "content": content },
        "parent_tool_use_id": null,
    });
    if let Some(u) = uuid {
        v["uuid"] = json!(u);
    }
    v
}

/// A text content block.
pub fn text_block(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

/// A base64 image content block.
pub fn image_block(media_type: &str, data: &str) -> Value {
    json!({
        "type": "image",
        "source": { "type": "base64", "media_type": media_type, "data": data },
    })
}

/// Serialize a text-only user message line (no uuid).
pub fn user_message_json(text: &str) -> String {
    user_message(vec![text_block(text)], None).to_string()
}

/// The `uuid`s of our messages a frame says it answers
/// (`user_message_uuids`, else `user_message_uuid`).
pub fn answered_uuids(v: &Value) -> Vec<String> {
    if let Some(list) = v.get("user_message_uuids").and_then(Value::as_array) {
        return list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }
    v.get("user_message_uuid")
        .and_then(Value::as_str)
        .map(|u| vec![u.to_string()])
        .unwrap_or_default()
}

/// The slash commands of an `initialize` response (or `commands_changed`),
/// `/`-prefixed like the static list.
pub fn commands_from(v: &Value) -> Vec<CommandInfo> {
    v.get("commands")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|c| {
                    let name = c.get("name")?.as_str()?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    Some(CommandInfo {
                        name: if name.starts_with('/') {
                            name.to_string()
                        } else {
                            format!("/{name}")
                        },
                        description: c
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        argument_hint: c
                            .get("argumentHint")
                            .and_then(Value::as_str)
                            .filter(|h| !h.is_empty())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// The pending item for a `can_use_tool` request (permission, native
/// AskUserQuestion, or ExitPlanMode plan approval).
pub fn item_from_can_use_tool(session_id: &str, req: &Value) -> PendingItem {
    let text = |k: &str| req.get(k).and_then(Value::as_str).map(str::to_string);
    let tool = text("tool_name").unwrap_or_else(|| "a tool".into());
    let input = req.get("input").cloned().unwrap_or_else(|| json!({}));
    let display = text("display_name").unwrap_or_else(|| tool.clone());
    let (kind, title, questions, plan) = match tool.as_str() {
        "AskUserQuestion" => {
            let qs = pending::questions_from_input(&input);
            let title = qs
                .first()
                .map(|q| truncate(&q.question, 240))
                .unwrap_or_else(|| "Claude has a question".into());
            (PendingKind::Question, title, Some(qs), None)
        }
        "ExitPlanMode" => {
            let plan = input
                .get("plan")
                .and_then(Value::as_str)
                .map(str::to_string);
            (
                PendingKind::Plan,
                "Claude has a plan ready for review".to_string(),
                None,
                plan,
            )
        }
        _ => (
            PendingKind::Permission,
            text("title").unwrap_or_else(|| format!("Claude wants to use {display}")),
            None,
            None,
        ),
    };
    let detail = match kind {
        PendingKind::Permission => {
            let summary = crate::claude::uievents::tool_summary(&tool, Some(&input));
            let mut d = text("description")
                .filter(|d| !d.trim().is_empty())
                .unwrap_or(summary);
            if let Some(p) = text("blocked_path") {
                d = format!("{d} (outside the allowed paths: {p})");
            }
            Some(truncate(&d, 600))
        }
        _ => text("description").map(|d| truncate(&d, 600)),
    };
    PendingItem {
        session_id: session_id.to_string(),
        kind,
        source: PendingSource::Owned,
        tool_name: Some(tool),
        tool_use_id: text("tool_use_id"),
        agent_id: text("agent_id"),
        title,
        detail,
        input: Some(input),
        questions,
        plan,
        suggestions: pending::suggestions_from_raw(req.get("permission_suggestions")),
        ..Default::default()
    }
}

/// The pending item for an MCP `elicitation` request.
pub fn item_from_elicitation(session_id: &str, req: &Value) -> PendingItem {
    let text = |k: &str| req.get(k).and_then(Value::as_str).map(str::to_string);
    let server = text("mcp_server_name").unwrap_or_else(|| "An MCP server".into());
    let mut detail = text("message").unwrap_or_default();
    if let Some(url) = text("url") {
        detail = format!("{detail}\n{url}").trim().to_string();
    }
    PendingItem {
        session_id: session_id.to_string(),
        kind: PendingKind::Elicitation,
        source: PendingSource::Owned,
        title: text("title").unwrap_or_else(|| {
            format!(
                "{} asks for input",
                text("display_name").unwrap_or(server.clone())
            )
        }),
        detail: (!detail.is_empty()).then(|| truncate(&detail, 1000)),
        input: Some(json!({
            "mcpServerName": server,
            "mode": text("mode"),
            "url": text("url"),
            "elicitationId": text("elicitation_id"),
            "requestedSchema": req.get("requested_schema").cloned().unwrap_or(Value::Null),
        })),
        ..Default::default()
    }
}

/// What one `stream_event` means for the dashboard.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct StreamOut {
    /// Frames to publish (text/thinking deltas are coalesced by the caller).
    pub frames: Vec<StreamFrame>,
    /// A new "what is it doing" hint.
    pub activity: Option<Activity>,
}

#[derive(Debug, Clone, PartialEq)]
struct Block {
    kind: String,
    tool: Option<(String, String)>,
}

/// Tracks the current API message and its content blocks across
/// `stream_event`s so every delta can carry `messageId` + `index`.
#[derive(Debug, Default)]
pub struct StreamTracker {
    message_id: Option<String>,
    blocks: HashMap<u32, Block>,
}

impl StreamTracker {
    fn frame(&self, kind: StreamKind, index: Option<u32>) -> StreamFrame {
        StreamFrame {
            kind,
            message_id: self.message_id.clone(),
            index,
            ..Default::default()
        }
    }

    /// Interpret one `stream_event.event`.
    pub fn on_event(&mut self, event: &Value) -> StreamOut {
        let mut out = StreamOut::default();
        let index = event
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|i| u32::try_from(i).ok());
        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "message_start" => {
                self.message_id = event
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                self.blocks.clear();
            }
            "content_block_start" => {
                let block = event.get("content_block").cloned().unwrap_or(Value::Null);
                let kind = block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let tool = match kind.as_str() {
                    "tool_use" | "server_tool_use" | "mcp_tool_use" => Some((
                        block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_string(),
                    )),
                    _ => None,
                };
                match (&tool, kind.as_str()) {
                    (Some((id, name)), _) => {
                        let mut f = self.frame(StreamKind::Tool, index);
                        f.tool = Some(json!({ "id": id, "name": name }));
                        f.status = Some("start".into());
                        out.frames.push(f);
                        let mut a = Activity::new(ActivityKind::Tool, format!("Running {name}"));
                        a.tool = Some(name.clone());
                        out.activity = Some(a);
                    }
                    (None, "thinking" | "redacted_thinking") => {
                        out.activity = Some(Activity::new(ActivityKind::Thinking, "Thinking"));
                    }
                    (None, "text") => {
                        out.activity = Some(Activity::new(ActivityKind::Thinking, "Responding"));
                    }
                    _ => {}
                }
                if let Some(i) = index {
                    self.blocks.insert(i, Block { kind, tool });
                }
            }
            "content_block_delta" => {
                let delta = event.get("delta").cloned().unwrap_or(Value::Null);
                let (kind, text) = match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => (StreamKind::Text, delta.get("text")),
                    Some("thinking_delta") => (StreamKind::Thinking, delta.get("thinking")),
                    _ => return out,
                };
                if let Some(t) = text.and_then(Value::as_str).filter(|t| !t.is_empty()) {
                    let mut f = self.frame(kind, index);
                    f.delta = Some(t.to_string());
                    out.frames.push(f);
                }
            }
            "content_block_stop" => {
                if let Some(Block {
                    tool: Some((id, name)),
                    ..
                }) = index.and_then(|i| self.blocks.get(&i))
                {
                    let mut f = self.frame(StreamKind::Tool, index);
                    f.tool = Some(json!({ "id": id, "name": name }));
                    f.status = Some("stop".into());
                    out.frames.push(f);
                }
            }
            "message_stop" => {
                let mut f = self.frame(StreamKind::Status, None);
                f.status = Some("message_stop".into());
                out.frames.push(f);
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_are_nested() {
        let r = control_request("mc_1", json!({ "subtype": "interrupt" }));
        assert_eq!(r["type"], "control_request");
        assert_eq!(r["request"]["subtype"], "interrupt");
        let ok = control_success("r1", json!({ "behavior": "allow" }));
        assert_eq!(ok["response"]["subtype"], "success");
        assert_eq!(ok["response"]["request_id"], "r1");
        assert_eq!(ok["response"]["response"]["behavior"], "allow");
        let err = control_error("r2", "unsupported");
        assert_eq!(err["response"]["subtype"], "error");
        assert_eq!(err["response"]["error"], "unsupported");

        let (id, out) = parse_control_response(&json!({
            "type": "control_response",
            "response": { "subtype": "error", "request_id": "x", "error": "no",
                          "error_code": "auto_mode_model" }
        }))
        .unwrap();
        assert_eq!(id, "x");
        let e = out.unwrap_err();
        assert_eq!(e.message, "no");
        assert_eq!(e.code.as_deref(), Some("auto_mode_model"));
        let (_, ok) = parse_control_response(&json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": "y" }
        }))
        .unwrap();
        assert_eq!(ok.unwrap(), Value::Null);
    }

    #[test]
    fn user_messages() {
        let v = user_message(
            vec![text_block("hi"), image_block("image/png", "AAAA")],
            Some("u-1"),
        );
        assert_eq!(v["type"], "user");
        assert_eq!(v["uuid"], "u-1");
        assert!(v["parent_tool_use_id"].is_null());
        assert_eq!(
            v["message"]["content"][1]["source"]["media_type"],
            "image/png"
        );
        let line: Value = serde_json::from_str(&user_message_json("x")).unwrap();
        assert_eq!(line["message"]["content"][0]["text"], "x");
        assert!(line.get("uuid").is_none());
        assert_eq!(
            answered_uuids(&json!({ "user_message_uuids": ["a", "b"], "user_message_uuid": "b" })),
            vec!["a", "b"]
        );
        assert_eq!(
            answered_uuids(&json!({ "user_message_uuid": "c" })),
            vec!["c"]
        );
        assert!(answered_uuids(&json!({})).is_empty());
    }

    #[test]
    fn can_use_tool_items() {
        let p = item_from_can_use_tool(
            "s",
            &json!({
                "subtype": "can_use_tool", "tool_name": "Bash", "display_name": "Bash",
                "input": { "command": "touch x1", "description": "Create file x1" },
                "description": "Create file x1",
                "permission_suggestions": [
                    { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "touch x1 *" }],
                      "behavior": "allow", "destination": "localSettings" },
                    { "type": "setMode", "mode": "acceptEdits", "destination": "session" }
                ],
                "tool_use_id": "toolu_1", "agent_id": "a1"
            }),
        );
        assert_eq!(p.kind, PendingKind::Permission);
        assert_eq!(p.title, "Claude wants to use Bash");
        assert_eq!(p.detail.as_deref(), Some("Create file x1"));
        assert_eq!(p.tool_use_id.as_deref(), Some("toolu_1"));
        assert_eq!(p.agent_id.as_deref(), Some("a1"));
        // session copy + the 2 CLI suggestions
        assert_eq!(p.suggestions.len(), 3);

        let q = item_from_can_use_tool(
            "s",
            &json!({ "tool_name": "AskUserQuestion", "input": { "questions": [
                { "question": "Favourite colour?", "header": "Colour", "multiSelect": false,
                  "options": [{ "label": "Blue" }, { "label": "Red" }] }
            ]}}),
        );
        assert_eq!(q.kind, PendingKind::Question);
        assert_eq!(q.title, "Favourite colour?");
        assert_eq!(q.questions.as_ref().unwrap()[0].options.len(), 2);

        let plan = item_from_can_use_tool(
            "s",
            &json!({ "tool_name": "ExitPlanMode", "input": { "plan": "1. a\n2. b", "planFilePath": "/p" } }),
        );
        assert_eq!(plan.kind, PendingKind::Plan);
        assert_eq!(plan.plan.as_deref(), Some("1. a\n2. b"));

        let e = item_from_elicitation(
            "s",
            &json!({ "mcp_server_name": "gh", "message": "Pick a repo",
                     "requested_schema": { "type": "object" } }),
        );
        assert_eq!(e.kind, PendingKind::Elicitation);
        assert_eq!(e.title, "gh asks for input");
        assert_eq!(e.input.unwrap()["requestedSchema"]["type"], "object");
    }

    #[test]
    fn get_settings_applied_is_parsed_tolerantly() {
        // The shape CLI 2.1.284 answers (haiku: no effort, no ultracode).
        let a = applied_settings(&json!({
            "effective": {}, "sources": [],
            "applied": { "model": "claude-haiku-4-5-20251001", "effort": null, "advisor": null,
                         "ultracode": false, "ultracodeRequested": true,
                         "ultracodeAvailable": false }
        }))
        .unwrap();
        assert_eq!(a.model.as_deref(), Some("claude-haiku-4-5-20251001"));
        assert_eq!(a.effort, None);
        assert!(!a.ultracode && a.ultracode_requested && !a.ultracode_available);
        let a = applied_settings(&json!({ "applied": {
            "model": "claude-opus-5-5", "effort": "max", "ultracode": true,
            "ultracodeRequested": true, "ultracodeAvailable": true, "novel": 1 } }))
        .unwrap();
        assert_eq!(a.effort.as_deref(), Some("max"));
        assert!(a.ultracode && a.ultracode_available);
        // Drifted types degrade per field; no `applied` is no answer.
        let a =
            applied_settings(&json!({ "applied": { "effort": 5, "ultracode": "yes" } })).unwrap();
        assert_eq!(a, AppliedSettings::default());
        assert!(applied_settings(&json!({})).is_none());
        assert!(applied_settings(&Value::Null).is_none());
    }

    #[test]
    fn adaptive_thinking_from_the_initialize_catalog() {
        let models = vec![
            json!({ "value": "opus", "resolvedModel": "claude-opus-5-5",
                    "supportsAdaptiveThinking": true }),
            json!({ "value": "haiku", "resolvedModel": "claude-haiku-4-5-20251001" }),
        ];
        assert_eq!(
            catalog_adaptive_thinking(&models, "claude-opus-5-5[1m]"),
            Some(true)
        );
        assert_eq!(catalog_adaptive_thinking(&models, "opus"), Some(true));
        assert_eq!(catalog_adaptive_thinking(&models, "haiku"), Some(false));
        assert_eq!(catalog_adaptive_thinking(&models, "claude-x"), None);
    }

    #[test]
    fn commands_are_slash_prefixed() {
        let c = commands_from(&json!({ "commands": [
            { "name": "compact", "description": "Compact", "argumentHint": "[x]" },
            { "name": "/review", "description": "", "argumentHint": "" },
            { "name": "" }
        ]}));
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].name, "/compact");
        assert_eq!(c[0].argument_hint.as_deref(), Some("[x]"));
        assert_eq!(c[1].name, "/review");
        assert!(c[1].argument_hint.is_none());
    }

    #[test]
    fn stream_tracking() {
        let mut t = StreamTracker::default();
        t.on_event(&json!({ "type": "message_start", "message": { "id": "msg_1" } }));
        let o = t.on_event(&json!({ "type": "content_block_start", "index": 0,
            "content_block": { "type": "text", "text": "" } }));
        assert_eq!(o.activity.unwrap().label, "Responding");
        let o = t.on_event(&json!({ "type": "content_block_delta", "index": 0,
            "delta": { "type": "text_delta", "text": "Hel" } }));
        assert_eq!(o.frames[0].kind, StreamKind::Text);
        assert_eq!(o.frames[0].delta.as_deref(), Some("Hel"));
        assert_eq!(o.frames[0].message_id.as_deref(), Some("msg_1"));
        assert_eq!(o.frames[0].index, Some(0));
        let o = t.on_event(&json!({ "type": "content_block_start", "index": 1,
            "content_block": { "type": "tool_use", "id": "toolu_9", "name": "Bash", "input": {} } }));
        assert_eq!(o.frames[0].kind, StreamKind::Tool);
        assert_eq!(o.frames[0].status.as_deref(), Some("start"));
        assert_eq!(o.activity.unwrap().tool.as_deref(), Some("Bash"));
        // Tool input deltas are not forwarded.
        let o = t.on_event(&json!({ "type": "content_block_delta", "index": 1,
            "delta": { "type": "input_json_delta", "partial_json": "{\"c" } }));
        assert!(o.frames.is_empty());
        let o = t.on_event(&json!({ "type": "content_block_stop", "index": 1 }));
        assert_eq!(o.frames[0].status.as_deref(), Some("stop"));
        assert_eq!(o.frames[0].tool.as_ref().unwrap()["id"], "toolu_9");
        let o = t.on_event(&json!({ "type": "message_stop" }));
        assert_eq!(o.frames[0].kind, StreamKind::Status);
        let o = t.on_event(&json!({ "type": "content_block_delta", "index": 0,
            "delta": { "type": "thinking_delta", "thinking": "hmm" } }));
        assert_eq!(o.frames[0].kind, StreamKind::Thinking);
    }
}
