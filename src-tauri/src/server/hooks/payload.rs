//! The hook payload as the plugin posts it, parsed tolerantly, plus the id
//! validators and the canonical tool-input key used for correlation.
//!
//! `tool_response` (PostToolUse can carry a whole file, a 35 MB transcript
//! line, images) is skipped with [`IgnoredAny`] so it is never built into a
//! `Value` tree. `transcript_path`, `cwd` and `agent_transcript_path` are
//! deliberately not read: any hook-token holder controls them, and nothing
//! here ever opens a file named by a hook.

use std::hash::{Hash, Hasher};

use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_json::Value;

/// An id in a hook payload failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidId;

/// One hook event (`POST /hooks/event` body, also the `/hooks/wait` body).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct HookPayload {
    pub hook_event_name: String,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<Value>,
    /// Never materialised.
    pub tool_response: IgnoredAny,
    pub prompt_id: Option<String>,
    /// UserPromptSubmit only.
    pub prompt: Option<String>,
    pub permission_mode: Option<String>,
    pub permission_suggestions: Option<Value>,
    pub notification_type: Option<String>,
    /// SessionStart `startup|resume|clear|compact|fork`, SessionEnd reason.
    pub source: Option<String>,
    pub reason: Option<String>,
    pub mcp_server: Option<String>,
}

impl HookPayload {
    /// Parse a body; `None` for anything that is not a JSON object.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        // serde would also accept a JSON array for a struct.
        if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
            return None;
        }
        match serde_json::from_slice::<HookPayload>(bytes) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::debug!(error = %e, "unparseable hook payload");
                None
            }
        }
    }

    /// The agent id when it is well-formed; a malformed one is an error
    /// (the event is ignored).
    pub fn agent(&self) -> Result<Option<&str>, InvalidId> {
        match self.agent_id.as_deref() {
            None | Some("") => Ok(None),
            Some(a) if is_safe_id(a) => Ok(Some(a)),
            Some(_) => Err(InvalidId),
        }
    }

    /// The tool use id when well-formed (a malformed one is dropped).
    pub fn tool_use(&self) -> Option<&str> {
        self.tool_use_id.as_deref().filter(|t| is_safe_id(t))
    }
}

/// Canonical lowercase UUID (`8-4-4-4-12` hex).
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, &c)| match i {
            8 | 13 | 18 | 23 => c == b'-',
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(&c),
        })
}

/// `^[A-Za-z0-9_-]{1,80}$` (agent ids, tool use ids, message ids).
pub fn is_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// Serialize `v` with object keys sorted, so equal inputs compare equal
/// whatever order the CLI emitted them in.
pub fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Correlation key of a tool input.
pub fn input_key(v: &Value) -> u64 {
    let mut s = String::new();
    canonical(v, &mut s);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn payloads_parse_tolerantly_and_skip_tool_response() {
        let body = serde_json::to_vec(&json!({
            "hook_event_name": "PostToolUse",
            "session_id": "0a1b2c3d-0000-4000-8000-000000000000",
            "tool_use_id": "toolu_01",
            "tool_name": "Read",
            "tool_input": { "file_path": "/x" },
            "tool_response": { "file": { "content": "x".repeat(1000) }, "nested": [[[1]]] },
            "transcript_path": "/etc/passwd",
            "novel": { "field": true }
        }))
        .unwrap();
        let p = HookPayload::parse(&body).unwrap();
        assert_eq!(p.hook_event_name, "PostToolUse");
        assert_eq!(p.tool_use(), Some("toolu_01"));
        assert_eq!(p.tool_input.unwrap()["file_path"], "/x");
        assert!(HookPayload::parse(b"not json").is_none());
        assert!(HookPayload::parse(b"[]").is_none());
        let empty = HookPayload::parse(b"{}").unwrap();
        assert!(empty.session_id.is_empty());
    }

    #[test]
    fn ids_are_validated() {
        assert!(is_uuid("0a1b2c3d-0000-4000-8000-000000000000"));
        for bad in [
            "",
            "0A1B2C3D-0000-4000-8000-000000000000",
            "0a1b2c3d-0000-4000-8000-00000000000",
            "0a1b2c3d_0000-4000-8000-000000000000",
            "../../../../etc/passwd/aaaaaaaaaaaaaaa",
            "*",
        ] {
            assert!(!is_uuid(bad), "{bad:?}");
        }
        assert!(is_safe_id("a740b850a4b08f003"));
        assert!(is_safe_id("toolu_01DP1KMn-x"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("../x"));
        assert!(!is_safe_id("a b"));
        assert!(!is_safe_id(&"a".repeat(81)));
        let p = HookPayload {
            agent_id: Some("..%2F".into()),
            tool_use_id: Some("x/y".into()),
            ..Default::default()
        };
        assert!(p.agent().is_err());
        assert!(p.tool_use().is_none());
    }

    #[test]
    fn input_keys_ignore_key_order() {
        let a =
            json!({ "command": "ls", "description": "list", "nested": { "b": 1, "a": [1, 2] } });
        let b: Value = serde_json::from_str(
            r#"{"nested":{"a":[1,2],"b":1},"description":"list","command":"ls"}"#,
        )
        .unwrap();
        assert_eq!(input_key(&a), input_key(&b));
        assert_ne!(input_key(&a), input_key(&json!({ "command": "ls" })));
        let mut s = String::new();
        canonical(&json!({ "b": "x\"y", "a": null }), &mut s);
        assert_eq!(s, r#"{"a":null,"b":"x\"y"}"#);
    }
}
