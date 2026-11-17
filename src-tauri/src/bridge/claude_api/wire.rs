//! The Claude API's wire format (platform.claude.com/docs/en/api): its error
//! envelope, and a Messages request read field by field into what Claude Code
//! can run — refused in the API's own words when it is malformed, and when it
//! asks for something only the Anthropic API itself can do.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Map, Value};

use crate::bridge::error::BridgeError;

/// A request id written the way the API writes them.
pub fn request_id() -> String {
    let raw = uuid::Uuid::new_v4().simple().to_string();
    format!("req_mc{}", &raw[..22])
}

/// An error in the Claude API's envelope:
/// `{"type":"error","error":{"type","message"},"request_id"}`.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    /// `invalid_request_error`, `authentication_error`, …
    pub kind: &'static str,
    pub message: String,
    /// Mother Claude's own code for it (`bridge_not_started`,
    /// `api_key_required`, …), beside the API's type: a client of the API
    /// ignores it, one that knows Mother Claude can tell the cases apart.
    pub code: Option<String>,
}

impl ApiError {
    pub fn new(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            kind,
            message: message.into(),
            code: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request_error", message)
    }
    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "authentication_error", message)
    }
    pub fn permission(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "permission_error", message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found_error", message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict_error", message)
    }
    pub fn too_large(message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "request_too_large", message)
    }
    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limit_error", message)
    }
    pub fn api(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "api_error", message)
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "api_error", message)
    }
    pub fn overloaded(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::from_u16(529).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
            "overloaded_error",
            message,
        )
    }
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(StatusCode::GATEWAY_TIMEOUT, "timeout_error", message)
    }

    /// The `error` object alone (a stream's `error` event).
    pub fn object(&self) -> Value {
        let mut o = json!({ "type": self.kind, "message": self.message });
        if let Some(code) = &self.code {
            o["code"] = json!(code);
        }
        o
    }

    pub fn body(&self, request_id: &str) -> Value {
        json!({ "type": "error", "error": self.object(), "request_id": request_id })
    }

    /// The response, with the request's id in the body and the header.
    pub fn respond(&self, request_id: &str) -> Response {
        let mut resp = (self.status, axum::Json(self.body(request_id))).into_response();
        if let Ok(v) = HeaderValue::from_str(request_id) {
            resp.headers_mut().insert("request-id", v);
        }
        resp
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        self.respond(&request_id())
    }
}

/// The bridge's errors in the API's words: the status decides the type, the
/// message stays (it says what to do).
impl From<BridgeError> for ApiError {
    fn from(e: BridgeError) -> Self {
        let kind = match e.status.as_u16() {
            400 | 405 | 415 | 422 => "invalid_request_error",
            401 => "authentication_error",
            403 => "permission_error",
            404 => "not_found_error",
            409 => "conflict_error",
            410 => "not_found_error",
            413 => "request_too_large",
            429 => "rate_limit_error",
            504 => "timeout_error",
            529 => "overloaded_error",
            _ => "api_error",
        };
        let status = match e.status.as_u16() {
            // The API answers a malformed body with 400, never 415 or 422.
            415 | 422 => StatusCode::BAD_REQUEST,
            410 => StatusCode::NOT_FOUND,
            _ => e.status,
        };
        Self::new(status, kind, e.message).with_code(e.code)
    }
}

/// Who said what, in a request's `messages`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// One message: its role and its content as blocks (a string is one text
/// block).
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Value>,
}

/// A tool the client defines, and runs.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

/// A Messages request, as Claude Code can run it.
#[derive(Debug, Clone, PartialEq)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u64,
    pub messages: Vec<Message>,
    /// The system prompt, as one text (blocks joined).
    pub system: String,
    pub stream: bool,
    pub stop_sequences: Vec<String>,
    /// `metadata.user_id`: who the client says the request is for.
    pub user_id: Option<String>,
    /// `thinking`, as the API writes it (`{type, budget_tokens?, display?}`).
    pub thinking: Option<Value>,
    pub tools: Vec<ClientTool>,
    /// `tool_choice.type`: `auto`, `any`, `tool`, `none`.
    pub tool_choice: Option<String>,
    /// `tool_choice.name`, when it names one tool.
    pub tool_name: Option<String>,
    /// `output_config.effort`.
    pub effort: Option<String>,
    /// `output_config.format` (`{type:"json_schema", schema}`).
    pub output_format: Option<Value>,
    /// Mother Claude's own: the conversation to continue (`conversation_id`).
    pub conversation: Option<String>,
    /// Fields Claude Code has no way to honour (sampling, tiers): named in a
    /// response header, never an error.
    pub ignored: Vec<&'static str>,
}

/// The largest number of stop sequences taken.
pub const MAX_STOP_SEQUENCES: usize = 32;

/// Fields the API takes that Claude Code cannot honour: accepted, ignored,
/// and named in the `x-mother-claude-ignored` header.
const IGNORED: [&str; 9] = [
    "temperature",
    "top_p",
    "top_k",
    "service_tier",
    "speed",
    "inference_geo",
    "cache_control",
    "diagnostics",
    "context_management",
];

/// Fields only the Anthropic API itself serves.
const REFUSED: [&str; 2] = ["container", "mcp_servers"];

/// Tool types that run on Anthropic's servers: Claude Code does not run them
/// for an API client.
fn server_tool(kind: &str) -> bool {
    [
        "web_search_",
        "web_fetch_",
        "code_execution_",
        "bash_",
        "text_editor_",
        "computer_",
        "browser_",
        "memory_",
        "tool_search_",
    ]
    .iter()
    .any(|p| kind.starts_with(p))
}

fn field<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    obj.get(key).filter(|v| !v.is_null())
}

fn required(key: &str) -> ApiError {
    ApiError::invalid(format!("{key}: Field required"))
}

/// A content value as blocks: a string is one text block.
pub fn blocks(content: &Value, at: &str) -> Result<Vec<Value>, ApiError> {
    match content {
        Value::String(s) => Ok(vec![json!({ "type": "text", "text": s })]),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let Some(t) = item.get("type").and_then(Value::as_str) else {
                    return Err(ApiError::invalid(format!("{at}.{i}.type: Field required")));
                };
                if t == "text" && !item.get("text").is_some_and(Value::is_string) {
                    return Err(ApiError::invalid(format!("{at}.{i}.text: Field required")));
                }
            }
            Ok(items.clone())
        }
        _ => Err(ApiError::invalid(format!(
            "{at}: Input should be a valid string or a valid list"
        ))),
    }
}

/// The system prompt's text: a string, or text blocks joined.
fn system_text(v: &Value) -> Result<String, ApiError> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for (i, item) in items.iter().enumerate() {
                match (
                    item.get("type").and_then(Value::as_str),
                    item.get("text").and_then(Value::as_str),
                ) {
                    (Some("text"), Some(t)) => parts.push(t.to_string()),
                    _ => {
                        return Err(ApiError::invalid(format!(
                            "system.{i}: Input should be a text block"
                        )))
                    }
                }
            }
            Ok(parts.join("\n\n"))
        }
        _ => Err(ApiError::invalid(
            "system: Input should be a valid string or a list of text blocks",
        )),
    }
}

fn number_in(obj: &Map<String, Value>, key: &str, lo: f64, hi: f64) -> Result<(), ApiError> {
    if let Some(v) = field(obj, key) {
        let Some(n) = v.as_f64() else {
            return Err(ApiError::invalid(format!(
                "{key}: Input should be a valid number"
            )));
        };
        if n < lo || n > hi {
            return Err(ApiError::invalid(format!(
                "{key}: Input should be between {lo} and {hi}"
            )));
        }
    }
    Ok(())
}

/// A tool name the API accepts: `^[a-zA-Z0-9_-]{1,128}$`.
fn tool_name_ok(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl MessagesRequest {
    /// Reads a request body. `allow_without_max_tokens`: count_tokens takes
    /// none.
    pub fn parse(body: &Value, allow_without_max_tokens: bool) -> Result<Self, ApiError> {
        let Some(obj) = body.as_object() else {
            return Err(ApiError::invalid("The request body must be a JSON object."));
        };
        let model = match field(obj, "model") {
            Some(Value::String(m)) if !m.trim().is_empty() => m.trim().to_string(),
            Some(_) => return Err(ApiError::invalid("model: Input should be a valid string")),
            None => return Err(required("model")),
        };
        let max_tokens = match field(obj, "max_tokens") {
            Some(v) => v.as_u64().ok_or_else(|| {
                ApiError::invalid(
                    "max_tokens: Input should be a valid integer, greater than or equal to 0",
                )
            })?,
            None if allow_without_max_tokens => 0,
            None => return Err(required("max_tokens")),
        };
        let raw_messages = match field(obj, "messages") {
            Some(Value::Array(m)) => m,
            Some(_) => return Err(ApiError::invalid("messages: Input should be a valid list")),
            None => return Err(required("messages")),
        };
        let mut system = match field(obj, "system") {
            Some(v) => system_text(v)?,
            None => String::new(),
        };
        let mut messages = Vec::with_capacity(raw_messages.len());
        for (i, m) in raw_messages.iter().enumerate() {
            let at = format!("messages.{i}");
            let role = match m.get("role").and_then(Value::as_str) {
                Some("user") => Role::User,
                Some("assistant") => Role::Assistant,
                // A system message in the list joins the system prompt.
                Some("system") => {
                    let text =
                        system_text(m.get("content").unwrap_or(&Value::Null)).map_err(|_| {
                            ApiError::invalid(format!("{at}.content: Input should be text"))
                        })?;
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(&text);
                    continue;
                }
                Some(_) => {
                    return Err(ApiError::invalid(format!(
                        "{at}.role: Input should be 'user' or 'assistant'"
                    )))
                }
                None => return Err(ApiError::invalid(format!("{at}.role: Field required"))),
            };
            let content = blocks(
                m.get("content")
                    .ok_or_else(|| required(&format!("{at}.content")))?,
                &format!("{at}.content"),
            )?;
            // Consecutive messages of one role are one turn, as the API reads them.
            match messages.last_mut() {
                Some(Message {
                    role: r,
                    content: c,
                }) if *r == role => c.extend(content),
                _ => messages.push(Message { role, content }),
            }
        }
        if messages.is_empty() && !allow_without_max_tokens {
            return Err(ApiError::invalid(
                "messages: at least one message is required",
            ));
        }
        if messages.first().is_some_and(|m| m.role == Role::Assistant) {
            return Err(ApiError::invalid(
                "messages: the first message must use the \"user\" role",
            ));
        }

        let stream = match field(obj, "stream") {
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(ApiError::invalid("stream: Input should be a valid boolean")),
            None => false,
        };
        let stop_sequences = match field(obj, "stop_sequences") {
            Some(Value::Array(items)) => {
                let mut out = Vec::new();
                for (i, s) in items.iter().enumerate() {
                    match s.as_str() {
                        Some(s) if !s.is_empty() => out.push(s.to_string()),
                        _ => {
                            return Err(ApiError::invalid(format!(
                                "stop_sequences.{i}: Input should be a non-empty string"
                            )))
                        }
                    }
                }
                if out.len() > MAX_STOP_SEQUENCES {
                    return Err(ApiError::invalid(format!(
                        "stop_sequences: at most {MAX_STOP_SEQUENCES} here"
                    )));
                }
                out
            }
            Some(_) => {
                return Err(ApiError::invalid(
                    "stop_sequences: Input should be a valid list",
                ))
            }
            None => Vec::new(),
        };
        number_in(obj, "temperature", 0.0, 1.0)?;
        number_in(obj, "top_p", 0.0, 1.0)?;
        if let Some(v) = field(obj, "top_k") {
            if v.as_u64().is_none() {
                return Err(ApiError::invalid(
                    "top_k: Input should be a valid integer, greater than or equal to 0",
                ));
            }
        }
        let user_id = match field(obj, "metadata") {
            Some(Value::Object(m)) => match field(m, "user_id") {
                Some(Value::String(u)) if u.len() <= 512 => Some(u.clone()),
                Some(Value::String(_)) => {
                    return Err(ApiError::invalid(
                        "metadata.user_id: String should have at most 512 characters",
                    ))
                }
                Some(_) => {
                    return Err(ApiError::invalid(
                        "metadata.user_id: Input should be a valid string",
                    ))
                }
                None => None,
            },
            Some(_) => {
                return Err(ApiError::invalid(
                    "metadata: Input should be a valid dictionary",
                ))
            }
            None => None,
        };
        let conversation = field(obj, "conversation_id")
            .or_else(|| {
                field(obj, "metadata")
                    .and_then(Value::as_object)
                    .and_then(|m| field(m, "conversation_id"))
            })
            .map(|v| {
                v.as_str().map(str::to_string).ok_or_else(|| {
                    ApiError::invalid("conversation_id: Input should be a valid string")
                })
            })
            .transpose()?;

        let thinking = match field(obj, "thinking") {
            Some(t) => Some(Self::read_thinking(t, max_tokens)?),
            None => None,
        };
        let (tools, tool_choice, tool_name) = Self::read_tools(obj)?;
        let (effort, output_format) = match field(obj, "output_config") {
            Some(Value::Object(c)) => {
                let effort = match field(c, "effort") {
                    Some(Value::String(e))
                        if ["low", "medium", "high", "xhigh", "max"].contains(&e.as_str()) =>
                    {
                        Some(e.clone())
                    }
                    Some(_) => {
                        return Err(ApiError::invalid(
                            "output_config.effort: Input should be 'low', 'medium', 'high', 'xhigh' or 'max'",
                        ))
                    }
                    None => None,
                };
                let format = match field(c, "format") {
                    Some(f)
                        if f.get("type").and_then(Value::as_str) == Some("json_schema")
                            && f.get("schema").is_some_and(Value::is_object) =>
                    {
                        Some(f.clone())
                    }
                    Some(_) => {
                        return Err(ApiError::invalid(
                            "output_config.format: Input should be {\"type\":\"json_schema\",\"schema\":{…}}",
                        ))
                    }
                    None => None,
                };
                (effort, format)
            }
            Some(_) => {
                return Err(ApiError::invalid(
                    "output_config: Input should be a valid dictionary",
                ))
            }
            None => (None, None),
        };
        for key in REFUSED {
            if field(obj, key).is_some() {
                return Err(ApiError::invalid(format!(
                    "{key}: not available when Claude Code answers this API (Mother Claude); \
                     use the Anthropic API for it"
                )));
            }
        }
        let mut ignored: Vec<&'static str> = IGNORED
            .iter()
            .copied()
            .filter(|k| field(obj, k).is_some())
            .collect();
        // Claude Code cannot force a tool call or keep calls one at a time:
        // the model is asked to, and the header says so.
        if matches!(tool_choice.as_deref(), Some("any") | Some("tool")) {
            ignored.push("tool_choice");
        }
        if field(obj, "tool_choice")
            .and_then(|c| c.get("disable_parallel_tool_use"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            ignored.push("tool_choice.disable_parallel_tool_use");
        }

        Ok(Self {
            model,
            max_tokens,
            messages,
            system,
            stream,
            stop_sequences,
            user_id,
            thinking,
            tools,
            tool_choice,
            tool_name,
            effort,
            output_format,
            conversation,
            ignored,
        })
    }

    /// The thinking the conversation runs with: the request's, else what
    /// the Claude API does for its model ([`thinks_by_default`]).
    pub fn effective_thinking(&self) -> Value {
        self.thinking.clone().unwrap_or_else(|| {
            json!({
                "type": if thinks_by_default(&self.model) { "adaptive" } else { "disabled" }
            })
        })
    }

    fn read_thinking(t: &Value, max_tokens: u64) -> Result<Value, ApiError> {
        let kind = t.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            // `between_tools` is how the API turns thinking off where it
            // cannot be disabled.
            "disabled" | "between_tools" => Ok(json!({ "type": "disabled" })),
            "adaptive" => {
                let mut out = json!({ "type": "adaptive" });
                if let Some(d) = t.get("display").and_then(Value::as_str) {
                    out["display"] = json!(d);
                }
                Ok(out)
            }
            "enabled" => {
                let budget = t
                    .get("budget_tokens")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        ApiError::invalid("thinking.enabled.budget_tokens: Field required")
                    })?;
                if budget < 1024 {
                    return Err(ApiError::invalid(
                        "thinking.enabled.budget_tokens: Input should be greater than or equal to 1024",
                    ));
                }
                if max_tokens > 0 && budget >= max_tokens {
                    return Err(ApiError::invalid(
                        "max_tokens must be greater than thinking.budget_tokens",
                    ));
                }
                let mut out = json!({ "type": "enabled", "budget_tokens": budget });
                if let Some(d) = t.get("display").and_then(Value::as_str) {
                    out["display"] = json!(d);
                }
                Ok(out)
            }
            _ => Err(ApiError::invalid(
                "thinking.type: Input should be 'enabled', 'disabled' or 'adaptive'",
            )),
        }
    }

    #[allow(clippy::type_complexity)]
    fn read_tools(
        obj: &Map<String, Value>,
    ) -> Result<(Vec<ClientTool>, Option<String>, Option<String>), ApiError> {
        let mut tools = Vec::new();
        if let Some(v) = field(obj, "tools") {
            let Some(items) = v.as_array() else {
                return Err(ApiError::invalid("tools: Input should be a valid list"));
            };
            for (i, t) in items.iter().enumerate() {
                let kind = t.get("type").and_then(Value::as_str).unwrap_or("custom");
                if kind != "custom" {
                    if server_tool(kind) {
                        return Err(ApiError::invalid(format!(
                            "tools.{i}: {kind} runs on Anthropic's servers, which Claude Code does \
                             not do for an API client (Mother Claude); define your own tool, or use \
                             the Anthropic API"
                        )));
                    }
                    return Err(ApiError::invalid(format!(
                        "tools.{i}.type: unknown tool type {kind:?}"
                    )));
                }
                let name = t.get("name").and_then(Value::as_str).unwrap_or("");
                if !tool_name_ok(name) {
                    return Err(ApiError::invalid(format!(
                        "tools.{i}.name: String should match pattern '^[a-zA-Z0-9_-]{{1,128}}$'"
                    )));
                }
                let schema = t
                    .get("input_schema")
                    .filter(|s| s.is_object())
                    .ok_or_else(|| {
                        ApiError::invalid(format!("tools.{i}.input_schema: Field required"))
                    })?;
                if tools.iter().any(|x: &ClientTool| x.name == name) {
                    return Err(ApiError::invalid(format!(
                        "tools: Tool names must be unique ({name})"
                    )));
                }
                tools.push(ClientTool {
                    name: name.to_string(),
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    input_schema: schema.clone(),
                });
            }
        }
        let mut tool_name = None;
        let tool_choice = match field(obj, "tool_choice") {
            Some(c) => {
                let kind = c.get("type").and_then(Value::as_str).unwrap_or("");
                if !["auto", "any", "tool", "none"].contains(&kind) {
                    return Err(ApiError::invalid(
                        "tool_choice.type: Input should be 'auto', 'any', 'tool' or 'none'",
                    ));
                }
                if kind == "tool"
                    && !c
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|n| tools.iter().any(|t| t.name == n))
                {
                    return Err(ApiError::invalid(
                        "tool_choice.name: must name one of the request's tools",
                    ));
                }
                if kind == "tool" {
                    tool_name = c.get("name").and_then(Value::as_str).map(str::to_string);
                }
                Some(kind.to_string())
            }
            None => None,
        };
        Ok((tools, tool_choice, tool_name))
    }

    /// The text of the first user message, for a conversation's title.
    pub fn first_text(&self) -> Option<String> {
        self.messages
            .iter()
            .find(|m| m.role == Role::User)
            .and_then(|m| {
                m.content.iter().find_map(|b| {
                    (b.get("type").and_then(Value::as_str) == Some("text"))
                        .then(|| b.get("text").and_then(Value::as_str))
                        .flatten()
                })
            })
            .map(|t| t.chars().take(80).collect())
    }
}

/// The `x-mother-claude-ignored` header: the fields Claude Code could not
/// honour.
pub fn ignored_header(ignored: &[&'static str]) -> Option<(header::HeaderName, HeaderValue)> {
    if ignored.is_empty() {
        return None;
    }
    HeaderValue::from_str(&ignored.join(", ")).ok().map(|v| {
        (
            header::HeaderName::from_static("x-mother-claude-ignored"),
            v,
        )
    })
}

#[cfg(test)]
mod tests;

/// Whether the Claude API thinks when a request leaves `thinking` out: it
/// does, adaptively, on the Claude 5 models (and Claude Code's aliases name
/// those), and not on the models before them.
pub fn thinks_by_default(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    let Some(at) = m.find("claude-") else {
        return true;
    };
    let id = &m[at..];
    ![
        "claude-2",
        "claude-3",
        "claude-instant",
        "claude-opus-4",
        "claude-sonnet-4",
        "claude-haiku-4",
    ]
    .iter()
    .any(|old| id.starts_with(old))
}
