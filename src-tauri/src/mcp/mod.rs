//! Mother Claude's MCP server (docs/PLAN-BROWSER-AND-TEAMS.md §5.4): the tools
//! Claude Code sessions call — the browser (`browser_*`), the task board and
//! teams (`task_*`, `team_*`), the user's other computers (`machine_*`,
//! docs/PLAN-MACHINES.md Phase 4) and Mother Claude itself (`mc_*`).
//!
//! This file is the **contract** the areas share. A tool is a [`ToolDef`]
//! (what `tools/list` shows) and a handler (what `tools/call` runs). Each area
//! registers its tools from its own `register(&mut Registry)` function, added
//! to [`registry`] in one line. The Streamable HTTP endpoint (`/mcp`), the
//! caller's identity and authorization belong to `mcp::server`.
//!
//! A handler's failures that the model should see (a refusal, a timeout, a bad
//! argument) are [`ToolError`]s, which become an `isError: true` result — never
//! a JSON-RPC protocol error. Protocol errors are for malformed requests and
//! unknown tools only.
//!
//! The server around the registry (docs/MCP.md): [`server`] (the `/mcp`
//! endpoint and `state.mcp`), [`jsonrpc`] (framing, versions), [`identity`]
//! (the calling session), [`plugin`] (the plugin's `.mcp.json`, its headers
//! helper and the token file), [`api`] (the dashboard's panel).

pub mod api;
pub mod identity;
pub mod jsonrpc;
pub mod plugin;
pub mod server;
mod session_tool;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::state::AppState;

/// A boxed, sendable future (what a handler returns).
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Who is calling a tool, as far as the server could tell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    /// The calling Claude Code session, when it could be told (the CLI's pid
    /// matched to its pidfile, or the owned session the request names).
    pub session_id: Option<String>,
    /// The calling process (the Claude Code CLI), when known.
    pub pid: Option<u32>,
    /// That session's working directory (its project), when known.
    pub cwd: Option<String>,
    /// Mother Claude launched the session (an owned session).
    pub owned: bool,
}

/// What a handler gets besides its arguments.
#[derive(Clone)]
pub struct ToolCtx {
    pub state: AppState,
    pub caller: Caller,
}

/// The behaviour hints of a tool (MCP `ToolAnnotations`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

/// One tool as `tools/list` shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
    /// `browser_navigate`, `task_create`, `mc_ping`: lower case, `_`, at most
    /// 64 characters (the CLI prefixes it with `mcp__plugin_…__`).
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub description: String,
    /// A JSON Schema object for the arguments.
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
}

/// One content block of a tool result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    Text {
        text: String,
    },
    Image {
        /// Base64, no data-URL prefix.
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

/// What a tool call returns (MCP `CallToolResult`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutput {
    pub content: Vec<Content>,
    /// The whole result as JSON, or nothing. Never a part of it beside a
    /// text that holds the rest: when it is there, a client may give the
    /// model only this (Claude Code 2.1.319 does), so a text result — a page,
    /// a snapshot — uses [`ToolOutput::text`] alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

impl ToolOutput {
    /// A plain-text result.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text { text: text.into() }],
            structured_content: None,
            is_error: false,
        }
    }

    /// A JSON result: the value as compact text (what the model reads) and as
    /// `structuredContent` (what a client can parse).
    pub fn json(value: Value) -> Self {
        Self {
            content: vec![Content::Text {
                text: value.to_string(),
            }],
            structured_content: Some(value),
            is_error: false,
        }
    }

    /// An `isError` result the model reads as the reason.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::text(message)
        }
    }
}

/// A call that could not do what it was asked: shown to the model as an
/// `isError` result `"<message> (<code>)"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    /// Machine-readable: `invalid_arguments`, `not_found`, `denied`, `timeout`,
    /// `unavailable`, ….
    pub code: String,
    pub message: String,
}

impl ToolError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_arguments", message)
    }

    /// The result the model sees.
    pub fn into_output(self) -> ToolOutput {
        ToolOutput::error(format!("{} ({})", self.message, self.code))
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ToolError {}

/// A tool's handler.
pub type Handler =
    Arc<dyn Fn(ToolCtx, Value) -> BoxFuture<Result<ToolOutput, ToolError>> + Send + Sync>;

/// A registered tool.
#[derive(Clone)]
pub struct Tool {
    pub def: ToolDef,
    pub handler: Handler,
    /// How long one call may run here; `None`: the server's own limit
    /// (`server::CALL_TIMEOUT`). A tool that waits for a person or a long
    /// job says so (`machine_run` waits for an approval, then a build).
    pub limit: Option<std::time::Duration>,
}

/// The tools the server offers, in registration order.
#[derive(Clone, Default)]
pub struct Registry {
    tools: Vec<Tool>,
}

impl Registry {
    /// Add a tool. A second tool with the same name replaces the first (a
    /// programming error, logged).
    pub fn add<F, Fut>(&mut self, def: ToolDef, handler: F)
    where
        F: Fn(ToolCtx, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ToolOutput, ToolError>> + Send + 'static,
    {
        self.add_with_limit(def, None, handler);
    }

    /// [`Registry::add`] for a tool whose call may run longer than the
    /// server's own limit (`limit`).
    pub fn add_with_limit<F, Fut>(
        &mut self,
        def: ToolDef,
        limit: Option<std::time::Duration>,
        handler: F,
    ) where
        F: Fn(ToolCtx, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ToolOutput, ToolError>> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |ctx, args| Box::pin(handler(ctx, args)));
        let tool = Tool {
            def,
            handler,
            limit,
        };
        if let Some(old) = self.tools.iter_mut().find(|t| t.def.name == tool.def.name) {
            tracing::error!(tool = %tool.def.name, "two MCP tools with one name; the later one wins");
            *old = tool;
            return;
        }
        self.tools.push(tool);
    }

    pub fn list(&self) -> impl Iterator<Item = &ToolDef> {
        self.tools.iter().map(|t| &t.def)
    }

    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.def.name == name)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Run a tool: its result, or its [`ToolError`] as an `isError` result.
    /// `None`: no such tool (a protocol error for the caller to report).
    pub async fn call(&self, name: &str, ctx: ToolCtx, args: Value) -> Option<ToolOutput> {
        let tool = self.get(name)?;
        let fut = (tool.handler)(ctx, args);
        Some(fut.await.unwrap_or_else(ToolError::into_output))
    }
}

/// Every tool Mother Claude serves. Each area adds its `register` call here,
/// one line, in this order: core, browser, teams, machines.
pub fn registry() -> Registry {
    let mut r = Registry::default();
    core_tools(&mut r);
    crate::extension::mcp::register(&mut r);
    crate::orchestration::mcp::register(&mut r);
    crate::machines::work::mcp::register(&mut r);
    r
}

/// `mc_ping`: whether Mother Claude answers, and who it thinks is calling;
/// `mc_session`: the calling session as Mother Claude sees it.
fn core_tools(r: &mut Registry) {
    r.add(
        ToolDef {
            name: "mc_ping".into(),
            title: Some("Ping Mother Claude".into()),
            description: "Check that the Mother Claude desktop app is reachable. Returns its \
                          version and which Claude Code session it sees calling."
                .into(),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(true),
                idempotent_hint: Some(true),
                ..Default::default()
            }),
        },
        |ctx: ToolCtx, _args: Value| async move {
            Ok(ToolOutput::json(serde_json::json!({
                "app": "Mother Claude",
                "version": env!("CARGO_PKG_VERSION"),
                "session": ctx.caller.session_id,
                "owned": ctx.caller.owned,
            })))
        },
    );
    session_tool::register(r);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_serialize_as_mcp_results() {
        let ok = serde_json::to_value(ToolOutput::json(serde_json::json!({"a": 1}))).unwrap();
        assert_eq!(ok["content"][0]["type"], "text");
        assert_eq!(ok["content"][0]["text"], r#"{"a":1}"#);
        assert_eq!(ok["structuredContent"]["a"], 1);
        assert!(ok.get("isError").is_none());
        let err = serde_json::to_value(ToolError::new("denied", "Not on this site").into_output())
            .unwrap();
        assert_eq!(err["isError"], true);
        assert_eq!(err["content"][0]["text"], "Not on this site (denied)");
        let img = serde_json::to_value(Content::Image {
            data: "AAAA".into(),
            mime_type: "image/png".into(),
        })
        .unwrap();
        assert_eq!(
            img,
            serde_json::json!({"type": "image", "data": "AAAA", "mimeType": "image/png"})
        );
    }

    #[test]
    fn the_registry_lists_and_finds_its_tools_once() {
        let mut r = registry();
        assert!(r.get("mc_ping").is_some());
        let n = r.len();
        r.add(
            ToolDef {
                name: "mc_ping".into(),
                title: None,
                description: "again".into(),
                input_schema: serde_json::json!({"type": "object"}),
                annotations: None,
            },
            |_ctx: ToolCtx, _args: Value| async move { Ok(ToolOutput::text("pong")) },
        );
        assert_eq!(r.len(), n, "same name: replaced, not added");
        assert_eq!(
            r.get("mc_ping").map(|t| t.def.description.as_str()),
            Some("again")
        );
        assert!(r.list().all(|d| d.name.len() <= 64));
    }
}
