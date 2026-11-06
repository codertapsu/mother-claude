//! Mother Claude's MCP server: Streamable HTTP at `/mcp` on the loopback
//! listener (docs/MCP.md, plan §5.4).
//!
//! * **Where**: the loopback listener only. Every other listener answers
//!   `404` ([`route`]); a request that came through a tunnel or proxy, or
//!   whose `Host` is not loopback, gets `404` too; an `Origin` that is not
//!   this machine's (`http(s)://localhost|127.x|[::1]`) gets `403` (the
//!   transport's DNS-rebinding rule).
//! * **Who**: [`Principal::Mcp`] only — the per-launch `mcm_…` token the
//!   plugin's headers helper reads from `<app data>/mcp.token` (0600). That
//!   principal reaches nothing else, and is never dangerous: a tool's
//!   dangerous effect is an approval through `crate::pending`.
//! * **Which session**: `X-MC-Pid` (the CLI's pid) mapped through the
//!   pidfiles, or an owned session's `X-MC-Session` ([`super::identity`]).
//! * **Protocol**: JSON-RPC 2.0, single messages and batches, answered as
//!   `application/json` (no SSE: `GET` is `405`). `initialize` issues an
//!   `Mcp-Session-Id`, accepted but never required later (a session that
//!   outlived an app restart keeps working). Notifications get `202`.
//!   Unknown methods (the CLI's `server/discover` probe among them) are
//!   -32601, an unknown tool -32602; a tool's own failure is a normal
//!   result with `isError: true`.
//! * **Off**: with the setting off every request gets `503` and the plugin
//!   entry is left out ([`super::plugin`]).
//!
//! The registry is built once ([`super::registry`]) and kept here
//! (`state.mcp`). The last [`RECENT_CALLS`] calls are kept in memory for the
//! dashboard (time, session, tool, ok — never arguments).

use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, MethodRouter};
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::jsonrpc::{self, Body, Message};
use super::{Caller, Registry, ToolCtx, ToolError, ToolOutput};
use crate::server::auth::{random_token, MCP_PREFIX};
use crate::server::context::{ClientContext, Ingress, Principal};
use crate::server::devices::audit::AuditEntry;
use crate::server::error::ApiError;
use crate::state::AppState;

/// Largest request body (a tool's arguments can carry a file or an image).
pub const MCP_BODY_MAX: usize = 16 * 1024 * 1024;
/// The longest one tool call may run here (tools have their own, shorter
/// limits; this one only catches a handler that never ends).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Calls kept for the dashboard.
pub const RECENT_CALLS: usize = 50;
/// MCP sessions remembered (each `initialize` opens one).
const MAX_CLIENTS: usize = 512;
/// The setting's file in the app data dir.
pub const SETTINGS_FILE: &str = "mcp-settings.json";
/// The transport's session header.
pub const SESSION_HEADER: &str = "mcp-session-id";
/// The transport's version header (sent after `initialize`).
pub const VERSION_HEADER: &str = "mcp-protocol-version";

/// The persisted setting (`<app data>/mcp-settings.json`, 0600).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    /// `None` (never set) is on.
    enabled: Option<bool>,
}

/// One call, as the dashboard shows it: never its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallRecord {
    /// When it ended (ms since the epoch).
    pub at: i64,
    /// The calling session, when it could be told.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The calling session is one Mother Claude launched.
    pub owned: bool,
    pub tool: String,
    /// `false`: an `isError` result (a refusal, a timeout, a failure).
    pub ok: bool,
    pub duration_ms: u64,
}

/// An MCP session (`Mcp-Session-Id`): who initialized it, with what.
#[derive(Debug, Clone)]
struct Client {
    last_seen: i64,
    version: &'static str,
}

/// The MCP server's state (`state.mcp`).
pub struct McpState {
    registry: RwLock<Arc<Registry>>,
    token: String,
    settings_path: Option<PathBuf>,
    enabled: AtomicBool,
    clients: Mutex<HashMap<String, Client>>,
    recent: Mutex<VecDeque<CallRecord>>,
    calls: AtomicU64,
    /// In-flight calls by `<client>|<request id>`, for
    /// `notifications/cancelled`.
    inflight: Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>,
}

impl std::fmt::Debug for McpState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpState")
            .field("tools", &self.registry().len())
            .field("enabled", &self.enabled())
            .field("token", &crate::server::auth::fingerprint(&self.token))
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

impl McpState {
    /// Every tool ([`super::registry`]), a new token, and the setting from
    /// the app data dir (on when never set; in memory without a data dir).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let settings_path = data_dir.map(|d| d.join(SETTINGS_FILE));
        let enabled = settings_path
            .as_deref()
            .and_then(crate::persist::read_json::<Settings>)
            .and_then(|s| s.enabled)
            .unwrap_or(true);
        Self {
            registry: RwLock::new(Arc::new(super::registry())),
            token: random_token(MCP_PREFIX),
            settings_path,
            enabled: AtomicBool::new(enabled),
            clients: Mutex::new(HashMap::new()),
            recent: Mutex::new(VecDeque::new()),
            calls: AtomicU64::new(0),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// The tools this server offers.
    pub fn registry(&self) -> Arc<Registry> {
        self.registry
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Serve another set of tools (tests: a slow or failing tool).
    #[doc(hidden)]
    pub fn replace_registry(&self, registry: Registry) {
        *self.registry.write().unwrap_or_else(|p| p.into_inner()) = Arc::new(registry);
    }

    /// The `mcm_…` credential of this launch. Written only to `mcp.token`
    /// (0600); never logged, framed, returned or audited.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Whether `/mcp` serves (the setting).
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Turn the server on or off, persisted first (0600). Applying it to the
    /// plugin files is [`super::plugin::apply`]'s job.
    pub fn set_enabled(&self, on: bool) -> std::io::Result<()> {
        if let Some(path) = &self.settings_path {
            crate::persist::write_json_private(path, &Settings { enabled: Some(on) })?;
        }
        self.enabled.store(on, Ordering::Relaxed);
        Ok(())
    }

    /// The last calls, newest first.
    pub fn recent_calls(&self) -> Vec<CallRecord> {
        lock(&self.recent).iter().rev().cloned().collect()
    }

    /// Calls since the app started.
    pub fn calls_total(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }

    /// MCP sessions that sent anything within the last `within_ms`.
    pub fn clients_seen(&self, within_ms: i64) -> usize {
        let since = now_ms() - within_ms;
        lock(&self.clients)
            .values()
            .filter(|c| c.last_seen >= since)
            .count()
    }

    fn record(&self, call: CallRecord) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut recent = lock(&self.recent);
        recent.push_back(call);
        while recent.len() > RECENT_CALLS {
            recent.pop_front();
        }
    }

    fn open_client(&self, id: &str, version: &'static str) {
        let mut clients = lock(&self.clients);
        if clients.len() >= MAX_CLIENTS {
            if let Some(oldest) = clients
                .iter()
                .min_by_key(|(_, c)| c.last_seen)
                .map(|(k, _)| k.clone())
            {
                clients.remove(&oldest);
            }
        }
        clients.insert(
            id.to_string(),
            Client {
                last_seen: now_ms(),
                version,
            },
        );
    }

    /// The version a known MCP session negotiated (and mark it seen).
    fn client_version(&self, id: &str) -> Option<&'static str> {
        let mut clients = lock(&self.clients);
        let c = clients.get_mut(id)?;
        c.last_seen = now_ms();
        Some(c.version)
    }

    fn close_client(&self, id: &str) {
        lock(&self.clients).remove(id);
    }
}

// ─────────────────────────────── HTTP ───────────────────────────────

/// The `/mcp` route of a listener: the MCP server on the loopback listener,
/// `404` on every other one (LAN, tunnel).
pub fn route(ingress: Ingress) -> MethodRouter<AppState> {
    if ingress == Ingress::Loopback {
        axum::routing::post(post_mcp)
            .get(get_mcp)
            .delete(delete_mcp)
            .layer(DefaultBodyLimit::max(MCP_BODY_MAX))
    } else {
        any(not_served_here)
    }
}

async fn not_served_here() -> Response {
    not_here().into_response()
}

fn not_here() -> ApiError {
    ApiError::not_found(
        "not_found",
        "Mother Claude's MCP server only answers Claude Code on this machine, at \
         http://127.0.0.1:<port>/mcp.",
    )
}

/// The transport's DNS-rebinding rule: no `Origin`, or exactly one naming
/// this machine over http(s) — `localhost`, `127.x.x.x` or `[::1]`.
pub fn origin_ok(headers: &HeaderMap) -> bool {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    match (origins.next(), origins.next()) {
        (None, _) => true,
        (Some(o), None) => o.to_str().is_ok_and(loopback_origin),
        _ => false,
    }
}

fn loopback_origin(origin: &str) -> bool {
    let lower = origin.trim().to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && !lower.contains("tauri.localhost")
        && crate::server::context::origin_is_local(origin)
}

/// Whether this request may reach the MCP server at all; the refusal
/// otherwise. In order: the listener and the network path (404), the
/// `Origin` (403), the setting (503), the credential (401 / 403).
fn admit(state: &AppState, ctx: &ClientContext, headers: &HeaderMap) -> Result<(), ApiError> {
    if ctx.ingress != Ingress::Loopback || !ctx.on_machine {
        return Err(not_here());
    }
    if !origin_ok(headers) {
        return Err(ApiError::forbidden(
            "forbidden_origin",
            "Mother Claude's MCP server does not answer web pages: send no Origin, or a \
             loopback one.",
        ));
    }
    if !state.mcp.enabled() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "mcp_disabled",
            "Mother Claude's tools for Claude Code are turned off (Services > Claude Code \
             tools in the app).",
        ));
    }
    match ctx.principal {
        Principal::Mcp => Ok(()),
        Principal::Anonymous => Err(ApiError::unauthorized(
            "Mother Claude's MCP server needs its token, which the Mother Claude plugin's \
             headers helper sends.",
        )),
        _ => Err(ApiError::forbidden(
            "wrong_credential",
            "Only Claude Code sessions, through the Mother Claude plugin, call /mcp.",
        )),
    }
}

fn no_store(resp: &mut Response) {
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

fn json_response(status: StatusCode, body: &Value, session: Option<&str>) -> Response {
    let mut resp = (status, axum::Json(body)).into_response();
    no_store(&mut resp);
    if let Some(v) = session.and_then(|s| HeaderValue::from_str(s).ok()) {
        resp.headers_mut().insert(SESSION_HEADER, v);
    }
    resp
}

fn accepted() -> Response {
    let mut resp = StatusCode::ACCEPTED.into_response();
    no_store(&mut resp);
    resp
}

/// `POST /mcp`: one JSON-RPC message or a batch.
async fn post_mcp(
    State(state): State<AppState>,
    ctx: ClientContext,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    if let Err(refused) = admit(&state, &ctx, &headers) {
        return refused.into_response();
    }
    let body = match body {
        Ok(b) => b,
        Err(e) => {
            let status = e.status();
            let message = if status == StatusCode::PAYLOAD_TOO_LARGE {
                format!("The request is larger than {MCP_BODY_MAX} bytes.")
            } else {
                "The request body could not be read.".to_string()
            };
            return json_response(
                status,
                &jsonrpc::error(Value::Null, jsonrpc::INVALID_REQUEST, message),
                None,
            );
        }
    };
    let parsed = match jsonrpc::parse(&body) {
        Ok(p) => p,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &jsonrpc::error(Value::Null, e.code, e.message),
                None,
            )
        }
    };
    drop(body);
    let mut exchange = Exchange::new(state, ctx, &headers);
    let reply = match parsed {
        Body::Single(m) => exchange.handle(m).await,
        Body::Batch(messages) => {
            let mut out = Vec::new();
            for m in messages {
                if let Some(v) = exchange.handle(m).await {
                    out.push(v);
                }
            }
            (!out.is_empty()).then_some(Value::Array(out))
        }
    };
    match reply {
        Some(v) => json_response(StatusCode::OK, &v, exchange.new_client.as_deref()),
        None => accepted(),
    }
}

/// `GET /mcp`: no server-to-client stream here.
async fn get_mcp(
    State(state): State<AppState>,
    ctx: ClientContext,
    headers: HeaderMap,
) -> Response {
    if let Err(refused) = admit(&state, &ctx, &headers) {
        return refused.into_response();
    }
    let mut resp = ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "This MCP server answers POST requests with JSON; it opens no event stream.",
    )
    .into_response();
    resp.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST, DELETE"));
    resp
}

/// `DELETE /mcp`: the client ends its MCP session.
async fn delete_mcp(
    State(state): State<AppState>,
    ctx: ClientContext,
    headers: HeaderMap,
) -> Response {
    if let Err(refused) = admit(&state, &ctx, &headers) {
        return refused.into_response();
    }
    if let Some(id) = header_str(&headers, SESSION_HEADER) {
        state.mcp.close_client(id);
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    no_store(&mut resp);
    resp
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

// ───────────────────────────── one request ─────────────────────────────

/// One HTTP request's worth of messages.
struct Exchange {
    state: AppState,
    ctx: ClientContext,
    headers: HeaderMap,
    /// The `Mcp-Session-Id` the client sent (any value is accepted).
    client: Option<String>,
    /// The protocol version the answers are shaped for.
    version: &'static str,
    /// The `Mcp-Session-Id` an `initialize` in this request opened.
    new_client: Option<String>,
    /// Who is calling, once a tool call needed to know.
    caller: Option<Caller>,
}

impl Exchange {
    fn new(state: AppState, ctx: ClientContext, headers: &HeaderMap) -> Self {
        let client = header_str(headers, SESSION_HEADER)
            .filter(|s| s.len() <= 128 && s.bytes().all(|b| (0x21..=0x7e).contains(&b)))
            .map(str::to_string);
        // The header names the version once negotiated; else what the MCP
        // session agreed; else the transport's default.
        let version = header_str(headers, VERSION_HEADER)
            .and_then(jsonrpc::known_version)
            .or_else(|| client.as_deref().and_then(|c| state.mcp.client_version(c)))
            .unwrap_or(jsonrpc::DEFAULT_VERSION);
        Self {
            state,
            ctx,
            headers: headers.clone(),
            client,
            version,
            new_client: None,
            caller: None,
        }
    }

    async fn handle(&mut self, message: Message) -> Option<Value> {
        match message {
            Message::Request { id, method, params } => {
                Some(self.request(id, &method, &params).await)
            }
            Message::Notification { method, params } => {
                if method == "notifications/cancelled" {
                    self.cancel(&params);
                }
                None
            }
            Message::Response => None,
            Message::Invalid { id, reason } => {
                Some(jsonrpc::error(id, jsonrpc::INVALID_REQUEST, reason))
            }
        }
    }

    async fn request(&mut self, id: Value, method: &str, params: &Value) -> Value {
        match method {
            "initialize" => self.initialize(id, params),
            "ping" => jsonrpc::result(id, json!({})),
            "tools/list" => self.tools_list(id, params),
            "tools/call" => self.tools_call(id, params).await,
            _ => jsonrpc::error(
                id,
                jsonrpc::METHOD_NOT_FOUND,
                format!("Method not found: {}", clip(method, 100)),
            ),
        }
    }

    fn initialize(&mut self, id: Value, params: &Value) -> Value {
        let Some(p) = jsonrpc::params_object(params) else {
            return jsonrpc::error(
                id,
                jsonrpc::INVALID_PARAMS,
                "initialize takes an object of params",
            );
        };
        let version = jsonrpc::negotiate(p.get("protocolVersion").and_then(Value::as_str));
        let client = uuid::Uuid::new_v4().simple().to_string();
        self.state.mcp.open_client(&client, version);
        self.version = version;
        self.new_client = Some(client);
        let mut server_info = json!({
            "name": "mother-claude",
            "title": "Mother Claude",
            "version": env!("CARGO_PKG_VERSION"),
        });
        if version < "2025-06-18" {
            if let Some(o) = server_info.as_object_mut() {
                o.remove("title");
            }
        }
        jsonrpc::result(
            id,
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": server_info,
            }),
        )
    }

    fn tools_list(&self, id: Value, params: &Value) -> Value {
        let Some(p) = jsonrpc::params_object(params) else {
            return jsonrpc::error(id, jsonrpc::INVALID_PARAMS, "tools/list takes an object");
        };
        if p.get("cursor").is_some_and(|c| !c.is_null()) {
            return jsonrpc::error(
                id,
                jsonrpc::INVALID_PARAMS,
                "Invalid cursor: every tool is listed in one page",
            );
        }
        let registry = self.state.mcp.registry();
        let tools: Vec<Value> = registry
            .list()
            .filter_map(|d| serde_json::to_value(d).ok())
            .map(|t| jsonrpc::adapt_tool(t, self.version))
            .collect();
        jsonrpc::result(id, json!({ "tools": tools }))
    }

    async fn caller(&mut self) -> Caller {
        if let Some(c) = &self.caller {
            return c.clone();
        }
        let c = super::identity::resolve(&self.state, &self.headers).await;
        self.caller = Some(c.clone());
        c
    }

    /// The in-flight key of a request id: the MCP session (else the
    /// calling pid) and the id.
    fn inflight_key(&self, id: &Value) -> String {
        let scope = self.client.clone().unwrap_or_else(|| {
            header_str(&self.headers, "x-mc-pid")
                .map(|p| format!("pid:{}", clip(p, 16)))
                .unwrap_or_else(|| "-".into())
        });
        format!("{scope}|{id}")
    }

    async fn tools_call(&mut self, id: Value, params: &Value) -> Value {
        let Some(p) = jsonrpc::params_object(params) else {
            return jsonrpc::error(id, jsonrpc::INVALID_PARAMS, "tools/call takes an object");
        };
        let Some(name) = p.get("name").and_then(Value::as_str).map(str::to_string) else {
            return jsonrpc::error(
                id,
                jsonrpc::INVALID_PARAMS,
                "tools/call needs the tool's \"name\"",
            );
        };
        let args = match p.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(v @ Value::Object(_)) => v.clone(),
            Some(_) => {
                return jsonrpc::error(
                    id,
                    jsonrpc::INVALID_PARAMS,
                    "\"arguments\" must be an object",
                )
            }
        };
        let registry = self.state.mcp.registry();
        let Some(tool) = registry.get(&name) else {
            return jsonrpc::error(
                id,
                jsonrpc::INVALID_PARAMS,
                format!("Unknown tool: {}", clip(&name, 100)),
            );
        };
        let read_only = tool
            .def
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false);
        let caller = self.caller().await;
        let started = Instant::now();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let _inflight = Inflight::register(&self.state, self.inflight_key(&id), cancel_tx);
        let ctx = ToolCtx {
            state: self.state.clone(),
            caller: caller.clone(),
        };
        let output = run_tool(&registry, &name, ctx, args, cancel_rx, CALL_TIMEOUT).await;
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let ok = !output.is_error;
        self.state.mcp.record(CallRecord {
            at: now_ms(),
            session: caller.session_id.clone(),
            owned: caller.owned,
            tool: name.clone(),
            ok,
            duration_ms,
        });
        if !read_only {
            // A call that may change something is an audit line: the tool,
            // the session and the outcome — never the arguments.
            self.state.devices.audit.record(AuditEntry {
                method: Some("tools/call".into()),
                path: Some("/mcp".into()),
                detail: Some(json!({
                    "tool": name,
                    "session": caller.session_id,
                    "owned": caller.owned,
                    "ok": ok,
                    "ms": duration_ms,
                })),
                ..AuditEntry::from_ctx(&self.ctx, "mcp.call")
            });
        }
        match serde_json::to_value(&output) {
            Ok(v) => jsonrpc::result(id, jsonrpc::adapt_result(v, self.version)),
            Err(e) => jsonrpc::error(
                id,
                jsonrpc::INTERNAL_ERROR,
                format!("The tool's result could not be encoded: {e}"),
            ),
        }
    }

    /// `notifications/cancelled`: stop the named call of this client.
    fn cancel(&self, params: &Value) {
        let Some(request) = params.get("requestId").filter(|v| !v.is_null()) else {
            return;
        };
        let key = self.inflight_key(request);
        if let Some(tx) = lock(&self.state.mcp.inflight).remove(&key) {
            let _ = tx.send(());
        }
    }
}

/// Run one tool: its output, or an `isError` one when it panicked (debug
/// builds; release builds abort on a panic, so a tool must never panic),
/// ran past `limit` ([`CALL_TIMEOUT`]) or was cancelled.
async fn run_tool(
    registry: &Registry,
    name: &str,
    ctx: ToolCtx,
    args: Value,
    cancel: tokio::sync::oneshot::Receiver<()>,
    limit: Duration,
) -> ToolOutput {
    let run = AssertUnwindSafe(registry.call(name, ctx, args)).catch_unwind();
    tokio::select! {
        r = tokio::time::timeout(limit, run) => match r {
            Ok(Ok(Some(out))) => out,
            Ok(Ok(None)) => ToolError::new("not_found", format!("No tool {name}.")).into_output(),
            Ok(Err(_)) => {
                tracing::error!(tool = %name, "an MCP tool panicked");
                ToolError::new("internal", "The tool failed unexpectedly.").into_output()
            }
            Err(_) => ToolError::new(
                "timeout",
                format!("The tool did not finish within {} s.", limit.as_secs()),
            )
            .into_output(),
        },
        Ok(()) = cancel => ToolError::new("cancelled", "The call was cancelled.").into_output(),
    }
}

/// An in-flight call's cancellation slot, freed however the call ends
/// (answered, cancelled, or dropped with its connection).
struct Inflight {
    state: AppState,
    key: String,
}

impl Inflight {
    fn register(state: &AppState, key: String, tx: tokio::sync::oneshot::Sender<()>) -> Self {
        lock(&state.mcp.inflight).insert(key.clone(), tx);
        Self {
            state: state.clone(),
            key,
        }
    }
}

impl Drop for Inflight {
    fn drop(&mut self) {
        lock(&self.state.mcp.inflight).remove(&self.key);
    }
}

/// At most `max` characters of `s` (for echoing request text back).
fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Server start: the token file, the plugin files and the owned sessions'
/// plugin directory, as the setting says ([`super::plugin::apply`]).
pub fn start(state: &AppState) {
    super::plugin::apply(state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_checked_like_the_transport_asks() {
        let h = |pairs: &[(&str, &str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.append(
                    axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                    HeaderValue::from_str(v).unwrap(),
                );
            }
            m
        };
        assert!(origin_ok(&h(&[])));
        for ok in [
            "http://localhost",
            "http://127.0.0.1:6274",
            "https://localhost:3000",
            "http://[::1]:8080",
        ] {
            assert!(origin_ok(&h(&[("origin", ok)])), "{ok}");
        }
        for bad in [
            "https://evil.example",
            "http://localhost.evil.example",
            "null",
            "tauri://localhost",
            "http://tauri.localhost",
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop",
            "",
        ] {
            assert!(!origin_ok(&h(&[("origin", bad)])), "{bad}");
        }
        assert!(!origin_ok(&h(&[
            ("origin", "http://localhost"),
            ("origin", "http://localhost")
        ])));
    }

    #[test]
    fn the_setting_persists_privately_and_defaults_to_on() {
        let dir = tempfile::tempdir().unwrap();
        let st = McpState::load(Some(dir.path()));
        assert!(st.enabled());
        assert!(st.token().starts_with(MCP_PREFIX));
        st.set_enabled(false).unwrap();
        assert!(!McpState::load(Some(dir.path())).enabled());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(SETTINGS_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        // A new token every launch.
        assert_ne!(McpState::load(None).token(), McpState::load(None).token());
        assert!(McpState::load(None).enabled());
    }

    #[test]
    fn recent_calls_are_bounded_and_newest_first() {
        let st = McpState::load(None);
        for i in 0..(RECENT_CALLS + 5) {
            st.record(CallRecord {
                at: i as i64,
                session: None,
                owned: false,
                tool: format!("t{i}"),
                ok: true,
                duration_ms: 1,
            });
        }
        let r = st.recent_calls();
        assert_eq!(r.len(), RECENT_CALLS);
        assert_eq!(r[0].tool, format!("t{}", RECENT_CALLS + 4));
        assert_eq!(st.calls_total(), (RECENT_CALLS + 5) as u64);
        let v = serde_json::to_value(&r[0]).unwrap();
        assert!(v.get("session").is_none());
        assert_eq!(v["durationMs"], 1);
    }

    fn tool(name: &str) -> super::super::ToolDef {
        super::super::ToolDef {
            name: name.into(),
            title: None,
            description: "test".into(),
            input_schema: json!({"type": "object"}),
            annotations: None,
        }
    }

    fn test_state() -> AppState {
        crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        )
    }

    #[tokio::test]
    async fn a_tool_that_hangs_panics_or_is_cancelled_still_answers() {
        let mut r = Registry::default();
        r.add(tool("slow"), |_c: ToolCtx, _a: Value| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(ToolOutput::text("late"))
        });
        r.add(tool("boom"), |_c: ToolCtx, _a: Value| async move {
            if std::hint::black_box(true) {
                panic!("a tool bug");
            }
            Ok(ToolOutput::text("never"))
        });
        r.add(tool("refuses"), |_c: ToolCtx, _a: Value| async move {
            Err(ToolError::new("denied", "Not here"))
        });
        let ctx = || ToolCtx {
            state: test_state(),
            caller: Caller::default(),
        };
        let text = |o: &ToolOutput| serde_json::to_value(o).unwrap()["content"][0]["text"].clone();

        let (_keep, rx) = tokio::sync::oneshot::channel();
        let out = run_tool(&r, "slow", ctx(), json!({}), rx, Duration::from_millis(50)).await;
        assert!(out.is_error);
        assert!(text(&out).as_str().unwrap().ends_with("(timeout)"));

        let (_keep, rx) = tokio::sync::oneshot::channel();
        let out = run_tool(&r, "boom", ctx(), json!({}), rx, CALL_TIMEOUT).await;
        assert!(out.is_error);
        assert!(text(&out).as_str().unwrap().ends_with("(internal)"));

        let (tx, rx) = tokio::sync::oneshot::channel();
        let call = tokio::spawn({
            let r = r.clone();
            async move { run_tool(&r, "slow", ctx(), json!({}), rx, CALL_TIMEOUT).await }
        });
        tx.send(()).unwrap();
        let out = call.await.unwrap();
        assert!(text(&out).as_str().unwrap().ends_with("(cancelled)"));

        // A dropped cancel slot is not a cancellation.
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        drop(tx);
        let out = run_tool(&r, "refuses", ctx(), json!({}), rx, CALL_TIMEOUT).await;
        assert_eq!(text(&out), "Not here (denied)");
    }

    #[test]
    fn clients_are_remembered_with_their_version() {
        let st = McpState::load(None);
        st.open_client("abc", "2025-03-26");
        assert_eq!(st.client_version("abc"), Some("2025-03-26"));
        assert_eq!(st.client_version("nope"), None);
        assert_eq!(st.clients_seen(60_000), 1);
        st.close_client("abc");
        assert_eq!(st.clients_seen(60_000), 0);
        for i in 0..(MAX_CLIENTS + 3) {
            st.open_client(&format!("c{i}"), "2025-06-18");
        }
        assert!(lock(&st.clients).len() <= MAX_CLIENTS);
    }
}
