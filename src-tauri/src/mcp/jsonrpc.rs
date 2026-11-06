//! JSON-RPC 2.0 framing for the MCP endpoint (pure, no I/O): one HTTP body
//! into messages, MCP protocol versions, and the shapes of answers.
//!
//! A body is one message or a batch (an array). A message is a **request**
//! (`method` and a string or number `id`: answered), a **notification**
//! (`method`, no `id`: never answered), a **response** from the client (we
//! send no requests, so it is ignored), or invalid (answered with -32600).

use serde_json::{json, Map, Value};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// The MCP protocol versions this server speaks, newest first. Claude Code
/// 2.1.319 asks for `2025-11-25` (after a `server/discover` probe of the
/// 2026 draft, which gets -32601 here and falls back to `initialize`).
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The newest supported version: the answer to a client that asks for one
/// this server does not know.
pub const LATEST_VERSION: &str = PROTOCOL_VERSIONS[0];

/// The version assumed for a request that names none (the transport spec's
/// default for a server that cannot tell).
pub const DEFAULT_VERSION: &str = "2025-03-26";

/// One JSON-RPC message.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Answered with a result or an error carrying `id`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// Never answered.
    Notification { method: String, params: Value },
    /// The client answering a request of ours: ignored (we send none).
    Response,
    /// Answered with -32600 (`id` when one could be read, else `null`).
    Invalid { id: Value, reason: String },
}

/// A POST body.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Single(Message),
    Batch(Vec<Message>),
}

/// Why a body could not be read at all (answered with one error, id `null`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyError {
    pub code: i64,
    pub message: String,
}

/// Parse a POST body: JSON that is an object (one message) or a non-empty
/// array of them (a batch).
pub fn parse(bytes: &[u8]) -> Result<Body, BodyError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| BodyError {
        code: PARSE_ERROR,
        message: format!("Parse error: {e}"),
    })?;
    match value {
        Value::Array(items) if items.is_empty() => Err(BodyError {
            code: INVALID_REQUEST,
            message: "Invalid Request: an empty batch".into(),
        }),
        Value::Array(items) => Ok(Body::Batch(items.into_iter().map(classify).collect())),
        other => Ok(Body::Single(classify(other))),
    }
}

/// A usable request id: a string or a number (MCP never allows `null`).
fn valid_id(id: &Value) -> bool {
    matches!(id, Value::String(_) | Value::Number(_))
}

/// What one JSON value is, as a JSON-RPC message.
pub fn classify(value: Value) -> Message {
    let Value::Object(mut obj) = value else {
        return Message::Invalid {
            id: Value::Null,
            reason: "Invalid Request: not a JSON-RPC object".into(),
        };
    };
    let id = obj.remove("id");
    let readable_id = id.clone().filter(valid_id).unwrap_or(Value::Null);
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Message::Invalid {
            id: readable_id,
            reason: "Invalid Request: \"jsonrpc\" must be \"2.0\"".into(),
        };
    }
    match obj.remove("method") {
        Some(Value::String(method)) => {
            let params = obj.remove("params").unwrap_or(Value::Null);
            match id {
                None => Message::Notification { method, params },
                Some(id) if valid_id(&id) => Message::Request { id, method, params },
                Some(_) => Message::Invalid {
                    id: Value::Null,
                    reason: "Invalid Request: \"id\" must be a string or a number".into(),
                },
            }
        }
        Some(_) => Message::Invalid {
            id: readable_id,
            reason: "Invalid Request: \"method\" must be a string".into(),
        },
        None if id.is_some() && (obj.contains_key("result") || obj.contains_key("error")) => {
            Message::Response
        }
        None => Message::Invalid {
            id: readable_id,
            reason: "Invalid Request: no \"method\"".into(),
        },
    }
}

/// A success answer.
pub fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// An error answer.
pub fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

/// The version to answer `initialize` with: the client's when supported,
/// else the newest this server speaks (the client then decides).
pub fn negotiate(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|r| PROTOCOL_VERSIONS.iter().find(|v| **v == r))
        .copied()
        .unwrap_or(LATEST_VERSION)
}

/// A supported version named exactly, else `None`.
pub fn known_version(v: &str) -> Option<&'static str> {
    PROTOCOL_VERSIONS.iter().find(|k| **k == v).copied()
}

/// How new a version is: 0 = 2024-11-05, 1 = 2025-03-26 (tool annotations),
/// 2 = 2025-06-18 and later (`title`, `structuredContent`).
fn rank(version: &str) -> u8 {
    match version {
        "2024-11-05" => 0,
        "2025-03-26" => 1,
        _ => 2,
    }
}

/// A tool definition as a client of `version` knows it: no `title` before
/// 2025-06-18, no `annotations` before 2025-03-26.
pub fn adapt_tool(mut tool: Value, version: &str) -> Value {
    if let Value::Object(obj) = &mut tool {
        let r = rank(version);
        if r < 2 {
            obj.remove("title");
            obj.remove("outputSchema");
        }
        if r < 1 {
            obj.remove("annotations");
        }
    }
    tool
}

/// A tool result as a client of `version` knows it: no `structuredContent`
/// before 2025-06-18 (the text block carries the same JSON).
pub fn adapt_result(mut result: Value, version: &str) -> Value {
    if rank(version) < 2 {
        if let Value::Object(obj) = &mut result {
            obj.remove("structuredContent");
        }
    }
    result
}

/// `params` as an object: absent or `null` is `{}`; anything else that is
/// not an object is `None` (invalid params).
pub fn params_object(params: &Value) -> Option<Map<String, Value>> {
    match params {
        Value::Null => Some(Map::new()),
        Value::Object(m) => Some(m.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_messages_are_classified() {
        let req = parse(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
        assert_eq!(
            req,
            Body::Single(Message::Request {
                id: json!(1),
                method: "ping".into(),
                params: Value::Null
            })
        );
        let note = parse(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
        assert!(matches!(
            note,
            Body::Single(Message::Notification { ref method, .. }) if method == "notifications/initialized"
        ));
        let resp = parse(br#"{"jsonrpc":"2.0","id":"s1","result":{}}"#).unwrap();
        assert_eq!(resp, Body::Single(Message::Response));
        let string_id = parse(br#"{"jsonrpc":"2.0","id":"server-discover-probe-1","method":"server/discover","params":{}}"#).unwrap();
        assert!(matches!(
            string_id,
            Body::Single(Message::Request { ref id, .. }) if id == "server-discover-probe-1"
        ));
    }

    #[test]
    fn invalid_messages_keep_their_id_when_they_can() {
        for (body, id) in [
            (r#"{"id":3,"method":"ping"}"#, json!(3)),
            (r#"{"jsonrpc":"1.0","id":"a","method":"ping"}"#, json!("a")),
            (r#"{"jsonrpc":"2.0","id":4,"method":7}"#, json!(4)),
            (r#"{"jsonrpc":"2.0","id":5}"#, json!(5)),
            (
                r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
                Value::Null,
            ),
            (
                r#"{"jsonrpc":"2.0","id":{"x":1},"method":"ping"}"#,
                Value::Null,
            ),
            (r#""just a string""#, Value::Null),
            (r#"42"#, Value::Null),
        ] {
            match parse(body.as_bytes()).unwrap() {
                Body::Single(Message::Invalid { id: got, .. }) => assert_eq!(got, id, "{body}"),
                other => panic!("{body}: {other:?}"),
            }
        }
    }

    #[test]
    fn bodies_that_cannot_be_read() {
        assert_eq!(parse(b"{not json").unwrap_err().code, PARSE_ERROR);
        assert_eq!(parse(b"").unwrap_err().code, PARSE_ERROR);
        assert_eq!(parse(b"[]").unwrap_err().code, INVALID_REQUEST);
    }

    #[test]
    fn batches_mix_kinds() {
        let body = br#"[
            {"jsonrpc":"2.0","id":1,"method":"ping"},
            {"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}},
            7
        ]"#;
        let Body::Batch(items) = parse(body).unwrap() else {
            panic!("a batch")
        };
        assert_eq!(items.len(), 4);
        assert!(matches!(items[0], Message::Request { .. }));
        assert!(matches!(items[1], Message::Notification { .. }));
        assert!(matches!(items[2], Message::Request { .. }));
        assert!(matches!(items[3], Message::Invalid { .. }));
    }

    #[test]
    fn answers_have_the_jsonrpc_shape() {
        assert_eq!(
            result(json!(1), json!({})),
            json!({"jsonrpc": "2.0", "id": 1, "result": {}})
        );
        assert_eq!(
            error(json!("x"), METHOD_NOT_FOUND, "no"),
            json!({"jsonrpc": "2.0", "id": "x", "error": {"code": -32601, "message": "no"}})
        );
    }

    #[test]
    fn versions_are_negotiated() {
        assert_eq!(negotiate(Some("2025-06-18")), "2025-06-18");
        assert_eq!(negotiate(Some("2025-03-26")), "2025-03-26");
        assert_eq!(negotiate(Some("2024-11-05")), "2024-11-05");
        assert_eq!(negotiate(Some("2025-11-25")), "2025-11-25");
        assert_eq!(negotiate(Some("2026-07-28")), LATEST_VERSION);
        assert_eq!(negotiate(Some("garbage")), LATEST_VERSION);
        assert_eq!(negotiate(None), LATEST_VERSION);
        assert_eq!(known_version("2025-03-26"), Some("2025-03-26"));
        assert_eq!(known_version("2026-07-28"), None);
    }

    #[test]
    fn old_clients_get_what_they_know() {
        let tool = json!({
            "name": "t", "title": "T", "description": "d",
            "inputSchema": {"type": "object"},
            "annotations": {"readOnlyHint": true}
        });
        let new = adapt_tool(tool.clone(), "2025-06-18");
        assert_eq!(new, tool);
        let mid = adapt_tool(tool.clone(), "2025-03-26");
        assert!(mid.get("title").is_none() && mid.get("annotations").is_some());
        let old = adapt_tool(tool, "2024-11-05");
        assert!(old.get("title").is_none() && old.get("annotations").is_none());
        let res = json!({"content": [], "structuredContent": {"a": 1}});
        assert!(adapt_result(res.clone(), "2025-11-25")
            .get("structuredContent")
            .is_some());
        assert!(adapt_result(res, "2025-03-26")
            .get("structuredContent")
            .is_none());
    }

    #[test]
    fn params_must_be_objects() {
        assert_eq!(params_object(&Value::Null), Some(Map::new()));
        assert!(params_object(&json!({"a": 1})).is_some());
        assert_eq!(params_object(&json!([1])), None);
        assert_eq!(params_object(&json!("x")), None);
    }
}
