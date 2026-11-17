//! From Claude Code's stream to the Claude API's.
//!
//! Claude Code (with partial messages on) passes on the API's own stream
//! events (`stream_event`: `message_start`, `content_block_*`,
//! `message_delta`, `message_stop`), so a client gets the model's real
//! message id, usage and blocks. What changes on the way: a client tool's
//! name loses the `mcp__client__` prefix the model saw, stop sequences (which
//! Claude Code does not take) are honoured here — the text is cut where one
//! begins and the message ends `stop_sequence` — and the same events build
//! the whole message for a request that does not stream.

use serde_json::{json, Map, Value};

/// What the model sees in front of a client tool's name.
pub const CLIENT_TOOL_PREFIX: &str = "mcp__client__";

/// One event of the Claude API's stream: `event: <name>` + `data: <json>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: String,
    pub data: Value,
}

impl Event {
    fn of(data: Value) -> Self {
        Self {
            name: data
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("message")
                .to_string(),
            data,
        }
    }
}

/// A client tool call in the message, as the client sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// The message being built from its events.
#[derive(Debug, Default, Clone)]
struct Accumulator {
    message: Map<String, Value>,
    blocks: Vec<Value>,
    /// Each tool_use block's input as streamed (`input_json_delta`).
    json: std::collections::HashMap<usize, String>,
}

impl Accumulator {
    fn start(&mut self, message: &Value) {
        self.message = message.as_object().cloned().unwrap_or_default();
        self.message.insert("content".into(), json!([]));
        self.blocks.clear();
        self.json.clear();
    }

    fn block_start(&mut self, index: usize, block: &Value) {
        while self.blocks.len() <= index {
            self.blocks.push(Value::Null);
        }
        self.blocks[index] = block.clone();
    }

    fn delta(&mut self, index: usize, delta: &Value) {
        let Some(block) = self.blocks.get_mut(index) else {
            return;
        };
        let append = |block: &mut Value, key: &str, more: &str| {
            let now = block.get(key).and_then(Value::as_str).unwrap_or_default();
            block[key] = json!(format!("{now}{more}"));
        };
        match delta.get("type").and_then(Value::as_str).unwrap_or("") {
            "text_delta" => append(
                block,
                "text",
                delta.get("text").and_then(Value::as_str).unwrap_or(""),
            ),
            "thinking_delta" => append(
                block,
                "thinking",
                delta.get("thinking").and_then(Value::as_str).unwrap_or(""),
            ),
            "signature_delta" => append(
                block,
                "signature",
                delta.get("signature").and_then(Value::as_str).unwrap_or(""),
            ),
            "input_json_delta" => {
                self.json.entry(index).or_default().push_str(
                    delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
            }
            "citations_delta" => {
                if let Some(c) = delta.get("citation") {
                    match block.get_mut("citations").and_then(Value::as_array_mut) {
                        Some(list) => list.push(c.clone()),
                        None => block["citations"] = json!([c]),
                    }
                }
            }
            _ => {}
        }
    }

    fn block_stop(&mut self, index: usize) {
        if let Some(raw) = self.json.remove(&index) {
            if let Some(block) = self.blocks.get_mut(index) {
                block["input"] = if raw.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
                };
            }
        }
    }

    fn message_delta(&mut self, delta: &Value, usage: Option<&Value>) {
        if let Some(d) = delta.as_object() {
            for (k, v) in d {
                self.message.insert(k.clone(), v.clone());
            }
        }
        if let Some(u) = usage.and_then(Value::as_object) {
            let mut now = self
                .message
                .get("usage")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for (k, v) in u {
                if !v.is_null() {
                    now.insert(k.clone(), v.clone());
                }
            }
            self.message.insert("usage".into(), Value::Object(now));
        }
    }

    fn message(&self) -> Value {
        let mut m = self.message.clone();
        m.insert(
            "content".into(),
            Value::Array(
                self.blocks
                    .iter()
                    .filter(|b| !b.is_null())
                    .cloned()
                    .collect(),
            ),
        );
        Value::Object(m)
    }
}

/// Reads Claude Code's messages for one assistant message of the API.
#[derive(Debug, Default)]
pub struct Translator {
    stop_sequences: Vec<String>,
    /// Text held back per block: it may be the start of a stop sequence.
    held: std::collections::HashMap<usize, String>,
    /// The stop sequence that ended the message.
    stopped: Option<String>,
    acc: Accumulator,
    /// Blocks whose `content_block_stop` went out.
    closed: std::collections::HashSet<usize>,
    started: bool,
    ended: bool,
    /// Text sent so far, for an estimate of the output tokens of a message
    /// cut at a stop sequence.
    chars_out: usize,
}

impl Translator {
    pub fn new(stop_sequences: Vec<String>) -> Self {
        Self {
            stop_sequences,
            ..Self::default()
        }
    }

    /// A `message_start` came.
    pub fn started(&self) -> bool {
        self.started
    }

    /// `message_stop` was sent: the client's message is whole.
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// The stop sequence that ended the message, when one did (the turn is
    /// still running then, and should be interrupted).
    pub fn stopped_at(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// The message as built so far.
    pub fn message(&self) -> Value {
        self.acc.message()
    }

    pub fn stop_reason(&self) -> Option<String> {
        self.acc
            .message
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    /// The client tool calls the message holds.
    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.acc
            .message()
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                    .map(|b| ToolCall {
                        id: b
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        name: b
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        input: b.get("input").cloned().unwrap_or_else(|| json!({})),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One of Claude Code's messages; the events of the API's stream it
    /// makes (none for anything that is not the main conversation's stream).
    pub fn push(&mut self, sdk: &Value) -> Vec<Event> {
        if self.ended {
            return Vec::new();
        }
        if sdk.get("type").and_then(Value::as_str) != Some("stream_event")
            || !sdk.get("parent_tool_use_id").is_none_or(Value::is_null)
        {
            return Vec::new();
        }
        let Some(event) = sdk.get("event") else {
            return Vec::new();
        };
        self.event(event)
    }

    fn event(&mut self, event: &Value) -> Vec<Event> {
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "message_start" => {
                // A new message before the last one ended: the API call was
                // retried; what the client builds starts over too.
                self.started = true;
                let message = event.get("message").cloned().unwrap_or_else(|| json!({}));
                self.acc.start(&message);
                self.held.clear();
                self.closed.clear();
                vec![Event::of(event.clone())]
            }
            "content_block_start" => {
                let mut event = event.clone();
                if let Some(name) = event
                    .pointer("/content_block/name")
                    .and_then(Value::as_str)
                    .and_then(|n| n.strip_prefix(CLIENT_TOOL_PREFIX))
                    .map(str::to_string)
                {
                    event["content_block"]["name"] = json!(name);
                }
                self.acc.block_start(index, &event["content_block"]);
                vec![Event::of(event)]
            }
            "content_block_delta" => {
                let delta = event.get("delta").cloned().unwrap_or_else(|| json!({}));
                if delta.get("type").and_then(Value::as_str) == Some("text_delta")
                    && !self.stop_sequences.is_empty()
                {
                    let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
                    return self.text(index, text);
                }
                if delta.get("type").and_then(Value::as_str) == Some("text_delta") {
                    self.chars_out += delta
                        .get("text")
                        .and_then(Value::as_str)
                        .map_or(0, str::len);
                }
                self.acc.delta(index, &delta);
                vec![Event::of(event.clone())]
            }
            "content_block_stop" => {
                let mut out = self.flush(index);
                self.acc.block_stop(index);
                self.closed.insert(index);
                out.push(Event::of(event.clone()));
                out
            }
            "message_delta" => {
                self.acc.message_delta(
                    event.get("delta").unwrap_or(&Value::Null),
                    event.get("usage"),
                );
                vec![Event::of(event.clone())]
            }
            "message_stop" => {
                self.ended = true;
                vec![Event::of(event.clone())]
            }
            // Pings are the server's own.
            "ping" => Vec::new(),
            _ => vec![Event::of(event.clone())],
        }
    }

    fn text_delta(&mut self, index: usize, text: &str) -> Event {
        let delta = json!({ "type": "text_delta", "text": text });
        self.acc.delta(index, &delta);
        self.chars_out += text.len();
        Event::of(json!({ "type": "content_block_delta", "index": index, "delta": delta }))
    }

    /// A text delta with stop sequences in force: the text up to a stop
    /// sequence goes out and the message ends there; text that might begin
    /// one waits for the next delta.
    fn text(&mut self, index: usize, more: &str) -> Vec<Event> {
        let mut text = self.held.remove(&index).unwrap_or_default();
        text.push_str(more);
        let hit = self
            .stop_sequences
            .iter()
            .filter_map(|s| text.find(s.as_str()).map(|at| (at, s.clone())))
            .min_by_key(|(at, _)| *at);
        if let Some((at, seq)) = hit {
            let mut out = Vec::new();
            if at > 0 {
                out.push(self.text_delta(index, &text[..at]));
            }
            self.stopped = Some(seq.clone());
            self.acc.block_stop(index);
            self.closed.insert(index);
            out.push(Event::of(
                json!({ "type": "content_block_stop", "index": index }),
            ));
            let tokens = (self.chars_out / 4).max(1);
            let delta = json!({ "stop_reason": "stop_sequence", "stop_sequence": seq });
            let usage = json!({ "output_tokens": tokens });
            self.acc.message_delta(&delta, Some(&usage));
            out.push(Event::of(
                json!({ "type": "message_delta", "delta": delta, "usage": usage }),
            ));
            out.push(Event::of(json!({ "type": "message_stop" })));
            self.ended = true;
            return out;
        }
        // Hold back the longest end of the text that begins a stop sequence.
        let keep = self
            .stop_sequences
            .iter()
            .map(|s| {
                (1..s.len().min(text.len() + 1))
                    .rev()
                    .find(|&n| {
                        text.is_char_boundary(text.len() - n)
                            && s.starts_with(&text[text.len() - n..])
                    })
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0);
        let send = text.len() - keep;
        if keep > 0 {
            self.held.insert(index, text[send..].to_string());
        }
        if send == 0 {
            return Vec::new();
        }
        vec![self.text_delta(index, &text[..send])]
    }

    /// The turn ended without the message's own end: its open blocks
    /// closed, and the message with them (`end_turn`).
    pub fn finish(&mut self) -> Vec<Event> {
        if !self.started || self.ended {
            return Vec::new();
        }
        let mut out = Vec::new();
        let open: Vec<usize> = self
            .acc
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.is_null())
            .map(|(i, _)| i)
            .filter(|i| !self.closed.contains(i))
            .collect();
        for index in open {
            out.extend(self.event(&json!({ "type": "content_block_stop", "index": index })));
        }
        if self.stop_reason().is_none() {
            out.extend(self.event(&json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn", "stop_sequence": null },
                "usage": { "output_tokens": (self.chars_out / 4).max(1) },
            })));
        }
        out.extend(self.event(&json!({ "type": "message_stop" })));
        out
    }

    /// Text held back for a block, sent at its end.
    fn flush(&mut self, index: usize) -> Vec<Event> {
        match self.held.remove(&index) {
            Some(t) if !t.is_empty() => vec![self.text_delta(index, &t)],
            _ => Vec::new(),
        }
    }
}

/// The API's keep-alive.
pub const PING_FRAME: &str = "event: ping\ndata: {\"type\": \"ping\"}\n\n";

/// An SSE frame of the API's stream.
pub fn sse_frame(event: &Event) -> String {
    format!("event: {}\ndata: {}\n\n", event.name, event.data)
}

#[cfg(test)]
mod tests;
