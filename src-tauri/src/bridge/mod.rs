//! The Claude HTTP bridge.
//!
//! A programmatic HTTP surface over Claude on this machine — conversations,
//! turns, live event streams, steering, interruption, tool-approval callbacks,
//! image and document input, and a direct Messages API path — modelled on the
//! Codex HTTP bridge so a client written for that service can be repointed here
//! with only a base URL change.
//!
//! ## Two backends, one surface
//!
//! * **Agent SDK** (`/threads`, `/turns`, `/run`, `/requests`) — a real Claude
//!   Code conversation with file access, bash, MCP servers and human-in-the-loop
//!   tool approval, driven through the long-lived Node host in [`host`].
//! * **Messages API** (`/messages`, `/files`) — `api.anthropic.com` directly,
//!   for the model-level features the Agent SDK does not expose: the Files API,
//!   programmatic tool calling, server tools, and coordinate-safe vision. Needs
//!   an API key, so it is off unless one is configured.
//!
//! ## Where it listens
//!
//! Mounted at `/v1` on the main server (so it inherits TLS on LAN binds), and —
//! unless disabled — additionally on its own loopback-only port, where the same
//! routes are served at both `/` and `/v1`. That port is the ergonomic one: it
//! is what `http://127.0.0.1:5612/threads` talks to.

pub mod assets;
pub mod control;
pub mod error;
pub mod events;
pub mod host;
pub mod inputs;
pub mod messages;
pub mod openapi;
pub mod ops;
pub mod routes;

use std::collections::HashMap;

use axum::extract::DefaultBodyLimit;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::Router;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::state::AppState;
use host::RuntimeHost;
use messages::MessagesClient;
use ops::OperationRegistry;

/// Default loopback port for the dedicated bridge listener.
pub const DEFAULT_BRIDGE_PORT: u16 = 5612;

/// Largest accepted request body. Matches the Messages API's own 32 MB cap, so
/// an inline image that Anthropic would accept is never rejected by us first.
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// How long `POST /threads/{id}/run` waits for a turn before handing back a
/// 504 and leaving the operation running for polling or replay.
pub const RUN_TIMEOUT_SECS: u64 = 1800;

/// Bridge configuration, all from the environment.
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Serve the bridge at all. `MOTHER_CLAUDE_BRIDGE=0` turns it off.
    pub enabled: bool,
    /// Dedicated loopback port, or `None` to only serve `/v1` on the main port.
    pub port: Option<u16>,
    /// Who may call the dedicated loopback port without a token
    /// (`MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN`, see [`TokenPolicy`]).
    ///
    /// **Clients on this machine, by default**, so `curl
    /// http://127.0.0.1:5612/capabilities` works with no setup — the point of
    /// a local bridge is that local tools can simply call it. The `/v1` mount
    /// on the main, LAN-reachable server always requires a token regardless
    /// of this setting.
    pub token: TokenPolicy,
    /// Anthropic API key for the direct Messages API path, if one is available.
    pub api_key: Option<String>,
    /// Browser origins allowed to call the bridge. **Empty means any origin**,
    /// so a page you are developing can call the bridge without ceremony;
    /// naming even one origin switches to strict allowlisting.
    pub allowed_origins: Vec<String>,
}

impl BridgeConfig {
    /// Whether a browser `Origin` may call the bridge, given the `Host` it
    /// addressed.
    ///
    /// With no allowlist configured this is open: any page may call the bridge,
    /// which is what makes "fetch from the site I am building" work without
    /// setup. Naming any origin in `MOTHER_CLAUDE_BRIDGE_ORIGINS` turns it into
    /// a strict allowlist — plus same-origin, so the bundled reference and
    /// console keep working on whichever listener and scheme served them.
    ///
    /// Neither header can be forged by a page: the browser sets `Origin`, and
    /// `Host` describes the server being addressed.
    pub fn origin_allowed(&self, origin: &str, host: Option<&str>) -> bool {
        if self.allowed_origins.is_empty() {
            return true;
        }
        if self.allowed_origins.iter().any(|o| o == origin) {
            return true;
        }
        let Some(host) = host else { return false };
        let origin_host = origin
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(origin);
        origin_host == host
    }
}

/// Who may call the dedicated loopback port without a token
/// (`MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN`). Whatever it says, the `/v1` mount on
/// the main, LAN-reachable server always requires one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenPolicy {
    /// `1`, or any value not listed below: every API route needs a token.
    Required,
    /// The default — unset, `0`, `false`, `off` or `no`: no token for a client
    /// on this machine. A request through a proxy or tunnel needs one, and
    /// while a detected tunnel points at the port, every request does.
    Local,
    /// `never`, or the API screen's "Open to tunnels" switch: no token for
    /// anyone who reaches the port, through a tunnel too — an explicit opt-in
    /// that publishes the API to whoever finds the tunnel's URL. Dangerous actions stay with callers provably on this
    /// machine; while a tunnel points at the port no tokenless request is (a
    /// raw TCP tunnel adds no headers and lets its caller name a loopback
    /// `Host`), so then only a token holder has them.
    Never,
}

impl TokenPolicy {
    /// Read a `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN` value. Anything not
    /// recognised requires the token, so a typo can never open the port.
    pub fn parse(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self::Local;
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "never" => Self::Never,
            "0" | "false" | "off" | "no" => Self::Local,
            _ => Self::Required,
        }
    }

    /// Every API route needs a token.
    pub fn requires_token(self) -> bool {
        self == Self::Required
    }

    /// This policy with the port closed to tokenless tunnels again: what the
    /// API screen's "open to tunnels" switch means when it is off.
    pub fn closed(self) -> Self {
        match self {
            Self::Never => Self::Local,
            other => other,
        }
    }
}

fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "off" || v == "no")
        }
        Err(_) => default,
    }
}

impl BridgeConfig {
    pub fn from_env() -> Self {
        let port = match std::env::var("MOTHER_CLAUDE_BRIDGE_PORT") {
            Ok(v) => match v.trim().parse::<u16>() {
                // An explicit 0 means "no dedicated port".
                Ok(0) => None,
                Ok(p) => Some(p),
                Err(_) => Some(DEFAULT_BRIDGE_PORT),
            },
            Err(_) => Some(DEFAULT_BRIDGE_PORT),
        };
        Self {
            enabled: env_flag("MOTHER_CLAUDE_BRIDGE", true),
            port,
            token: TokenPolicy::parse(
                std::env::var("MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN")
                    .ok()
                    .as_deref(),
            ),
            api_key: std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty()),
            allowed_origins: std::env::var("MOTHER_CLAUDE_BRIDGE_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .map(|o| o.trim().trim_end_matches('/').to_string())
                .filter(|o| !o.is_empty())
                .collect(),
        }
    }

    #[cfg(test)]
    pub fn for_test() -> Self {
        Self {
            enabled: true,
            port: None,
            token: TokenPolicy::Required,
            api_key: None,
            allowed_origins: Vec::new(),
        }
    }
}

/// A conversation the bridge knows about.
///
/// The Agent SDK has no notion of an "empty conversation": no transcript exists
/// until the first turn runs, so — exactly as the Codex bridge does — a created
/// thread lives here in memory until then, and does not survive a restart.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ThreadRecord {
    pub thread_id: String,
    pub cwd: String,
    pub created_at: i64,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: String,
    pub title: Option<String>,
    /// False until the first turn has run and a transcript exists on disk.
    pub persisted: bool,
    /// The turn currently running, if any.
    pub active_turn: Option<String>,
    /// The Claude account it runs under (docs/ACCOUNTS.md §5.5).
    pub account: String,
}

/// Everything the bridge owns, hung off [`crate::state::Inner`].
#[derive(Debug)]
pub struct BridgeState {
    pub config: BridgeConfig,
    pub ops: OperationRegistry,
    pub host: RuntimeHost,
    pub messages: MessagesClient,
    threads: RwLock<HashMap<String, ThreadRecord>>,
    /// The bound listener, when the bridge has been started. `None` means the
    /// bridge is not serving — the routes that drive Claude answer 503.
    pub listener: tokio::sync::Mutex<Option<control::Listener>>,
    /// What the person chose when they started it.
    pub defaults: RwLock<control::BridgeDefaults>,
    /// The conversation an unaddressed message joins.
    pub active_thread: RwLock<Option<String>>,
    /// The token policy the listener runs with — the configured one until a
    /// start from the API screen chooses otherwise ([`Self::token_policy`]).
    token: std::sync::RwLock<TokenPolicy>,
    /// Each live conversation's account, for sync lookups (session rows).
    accounts: std::sync::Mutex<HashMap<String, String>>,
}

impl BridgeState {
    pub fn new(config: BridgeConfig) -> Self {
        let messages = MessagesClient::new(config.api_key.clone());
        let token = std::sync::RwLock::new(config.token);
        Self {
            config,
            ops: OperationRegistry::default(),
            host: RuntimeHost::new(),
            messages,
            threads: RwLock::new(HashMap::new()),
            listener: tokio::sync::Mutex::new(None),
            defaults: RwLock::new(control::BridgeDefaults::default()),
            active_thread: RwLock::new(None),
            token,
            accounts: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// The account a live conversation runs under (sync).
    pub fn account_of(&self, id: &str) -> Option<String> {
        self.accounts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// The token policy of the listener's current (or last) run: the
    /// configured one (`MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN`) until the API
    /// screen starts it with the "open to tunnels" switch set either way.
    pub fn token_policy(&self) -> TokenPolicy {
        self.token.read().map(|t| *t).unwrap_or(self.config.token)
    }

    pub(crate) fn set_token_policy(&self, token: TokenPolicy) {
        if let Ok(mut t) = self.token.write() {
            *t = token;
        }
    }

    /// Whether the bridge is serving. Everything that drives Claude checks this.
    pub async fn is_running(&self) -> bool {
        self.listener.lock().await.is_some()
    }

    pub async fn defaults(&self) -> control::BridgeDefaults {
        self.defaults.read().await.clone()
    }

    pub async fn update_defaults<F: FnOnce(&mut control::BridgeDefaults)>(&self, f: F) {
        let mut guard = self.defaults.write().await;
        f(&mut guard);
    }

    /// Remember the conversation unaddressed messages should join.
    pub async fn set_active_thread(&self, thread_id: Option<String>) {
        *self.active_thread.write().await = thread_id.clone();
        self.update_defaults(|d| d.default_thread = thread_id).await;
    }

    pub async fn insert_thread(&self, record: ThreadRecord) {
        self.accounts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(record.thread_id.clone(), record.account.clone());
        self.threads
            .write()
            .await
            .insert(record.thread_id.clone(), record);
    }

    pub async fn thread(&self, id: &str) -> Option<ThreadRecord> {
        self.threads.read().await.get(id).cloned()
    }

    pub async fn threads(&self) -> Vec<ThreadRecord> {
        let mut out: Vec<ThreadRecord> = self.threads.read().await.values().cloned().collect();
        out.sort_by_key(|t| std::cmp::Reverse(t.created_at));
        out
    }

    /// Record that a turn started (or finished, with `None`).
    pub async fn set_active_turn(&self, id: &str, turn: Option<String>) {
        if let Some(record) = self.threads.write().await.get_mut(id) {
            if turn.is_some() {
                record.persisted = true;
            }
            record.active_turn = turn;
        }
    }

    pub async fn update_thread<F: FnOnce(&mut ThreadRecord)>(&self, id: &str, f: F) {
        if let Some(record) = self.threads.write().await.get_mut(id) {
            f(record);
        }
    }

    /// Drop a conversation the host has closed. If it was the one unaddressed
    /// messages joined, stop pointing at it — the next message should open a
    /// fresh conversation rather than fail against a dead id.
    pub async fn forget_thread(&self, id: &str) {
        self.threads.write().await.remove(id);
        self.accounts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        let mut active = self.active_thread.write().await;
        if active.as_deref() == Some(id) {
            *active = None;
        }
    }

    /// Bridge conversations as owned-session metadata for the registry.
    ///
    /// Without this a conversation created through the API disappears from the
    /// dashboard on the next monitor sweep — the synthetic row `mark_owned`
    /// inserted is replaced by a rebuild that has never heard of it, because it
    /// has no transcript until its first turn.
    pub async fn live_sessions(&self) -> Vec<crate::claude::OwnedSessionMeta> {
        self.threads
            .read()
            .await
            .values()
            .map(|t| crate::claude::OwnedSessionMeta {
                id: t.thread_id.clone(),
                cwd: t.cwd.clone(),
                started_at: t.created_at,
            })
            .collect()
    }
}

/// End a bridge conversation from outside the bridge's own routes — the
/// dashboard's Stop button, which would otherwise report success while the
/// conversation carried on (there is no `ControlRegistry` handle for it).
pub async fn close_thread(state: &AppState, thread_id: &str) -> bool {
    if state.bridge.thread(thread_id).await.is_none() {
        return false;
    }
    let result = state
        .bridge
        .host
        .call_if_running(
            state,
            "session.close",
            serde_json::json!({ "sessionId": thread_id }),
            std::time::Duration::from_secs(30),
        )
        .await;
    if let Err(e) = result {
        tracing::warn!(error = %e, thread_id, "could not close a bridge conversation");
    }
    state.bridge.forget_thread(thread_id).await;
    state.unmark_owned(thread_id).await;
    true
}

/// Send a dashboard instruction to a bridge conversation by starting a turn.
///
/// Bridge conversations are marked owned so they appear in the dashboard, but
/// they are driven through the host rather than a `ControlRegistry` stdin pipe,
/// so the dashboard's normal inject path has nothing to write to.
pub async fn send_message(
    state: &AppState,
    thread_id: &str,
    text: &str,
) -> error::BridgeResult<()> {
    routes::turns::begin_turn(
        state,
        thread_id,
        &routes::turns::TurnBody {
            input: serde_json::Value::String(text.to_string()),
            priority: None,
        },
    )
    .await
    .map(|_| ())
}

/// Build the bridge router, root-mounted.
///
/// `token` gates only this instance: the `/v1` mount on the main server always
/// passes [`TokenPolicy::Required`], because that listener is reachable from
/// the LAN. Documentation, the OpenAPI document, the JS client and the example
/// page are public either way — they are static assets and reveal no token.
pub fn router(state: AppState, token: TokenPolicy) -> Router<AppState> {
    // Discovery stays available whether or not the bridge has been started —
    // "is it running?" has to be answerable when the answer is no.
    let meta = Router::new()
        .route("/health", get(routes::meta::health))
        .route("/capabilities", get(routes::meta::capabilities))
        .route("/metadata", get(routes::meta::metadata))
        .route("/models", get(routes::meta::models))
        .route("/auth", get(routes::meta::auth))
        .route("/auth/login", post(routes::meta::auth_login));

    // Everything that can spend tokens or touch the filesystem is gated on the
    // bridge having been started from the app.
    let api = Router::new()
        .route(
            "/threads",
            get(routes::threads::list).post(routes::threads::create),
        )
        .route(
            "/threads/{thread_id}",
            get(routes::threads::read).delete(routes::threads::close),
        )
        .route(
            "/threads/{thread_id}/messages",
            get(routes::threads::messages),
        )
        .route(
            "/threads/{thread_id}/context",
            get(routes::threads::context),
        )
        .route("/threads/{thread_id}/name", post(routes::threads::rename))
        .route("/threads/{thread_id}/fork", post(routes::threads::fork))
        .route(
            "/threads/{thread_id}/model",
            post(routes::threads::set_model),
        )
        .route(
            "/threads/{thread_id}/permission-mode",
            post(routes::threads::set_permission_mode),
        )
        .route("/threads/{thread_id}/turns", post(routes::turns::start))
        .route("/threads/{thread_id}/run", post(routes::turns::run))
        .route(
            "/threads/{thread_id}/turns/{turn_id}/steer",
            post(routes::turns::steer),
        )
        .route(
            "/threads/{thread_id}/turns/{turn_id}/interrupt",
            post(routes::turns::interrupt),
        )
        .route("/chat", post(routes::turns::chat))
        .route("/operations/{operation_id}", get(routes::turns::operation))
        .route(
            "/operations/{operation_id}/events",
            get(routes::turns::operation_events),
        )
        .route("/events", get(routes::turns::global_events))
        .route("/requests", get(routes::requests::list))
        .route(
            "/requests/{request_id}/respond",
            post(routes::requests::respond),
        )
        .route("/messages", post(routes::messages::create))
        .route(
            "/files",
            get(routes::messages::list_files).post(routes::messages::upload_file),
        )
        .route("/files/{file_id}", delete(routes::messages::delete_file))
        .route(
            "/files/{file_id}/content",
            get(routes::messages::file_content),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_started,
        ));

    // Authentication runs on every mount: the token is always required on
    // `/v1` of the main server, and on the loopback port the tokenless mode
    // applies only to requests genuinely from this machine (design §2.3) —
    // unless the operator opted into `TokenPolicy::Never`.
    let api = meta
        .merge(api)
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_bridge_auth,
        ));

    let public = Router::new()
        .route("/openapi.json", get(routes::meta::openapi))
        // A bare `/docs` must redirect rather than render: the page's assets are
        // relative, so serving it at a path with no trailing slash resolves
        // every one of them a level too high and the reference comes up blank.
        .route("/docs", get(assets::redirect_to_slash))
        .route("/docs/", get(assets::docs_index))
        .route("/docs/{*path}", get(assets::docs_asset))
        .route("/client.mjs", get(assets::client_module))
        .route("/example", get(assets::redirect_to_slash))
        .route("/example/", get(assets::example_index))
        .route("/example/{*path}", get(assets::example_asset));

    api.merge(public)
        // Unknown paths and wrong methods get the same envelope as everything
        // else, so a client never has to parse two error formats.
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(axum::Extension(MountPolicy { token }))
        .layer(axum::middleware::from_fn_with_state(state, guard_origin))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

/// Refuse work until the bridge has been started from the app.
///
/// Opening Mother Claude does not publish this API; starting it is a deliberate
/// act, because the port can drive Claude with the user's account. Discovery
/// routes stay open so a client can tell "not started" from "not there".
async fn require_started(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if state.bridge.is_running().await {
        return next.run(req).await;
    }
    error::BridgeError::new(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "bridge_not_started",
        "The Claude bridge has not been started. Open Mother Claude, choose the \
         defaults you want on the API screen, and press Start.",
    )
    .into_response()
}

/// Which listener a request arrived on. `/capabilities` and the OpenAPI
/// document must describe *this* mount — the loopback port and the LAN mount
/// have different authentication policies, and advertising the wrong one sends
/// clients to debug a token they do not need (or omit one they do).
#[derive(Debug, Clone, Copy)]
pub struct MountPolicy {
    pub token: TokenPolicy,
}

impl MountPolicy {
    /// Every API route on this mount needs a token.
    pub fn require_token(&self) -> bool {
        self.token.requires_token()
    }
}

/// Authentication with the bridge's error envelope (design §2.3, §10 B1).
///
/// * A desktop, local-script or paired-device credential always passes.
/// * On a tokenless mount (the dedicated loopback port with
///   `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN` unset) a **machine-local** request —
///   loopback peer, exactly one loopback `Host`, no proxy/tunnel headers —
///   passes without a token, whatever its `Origin` (local tools, Postman,
///   Swagger and pages on other websites keep working; dangerous actions still
///   need a local or absent `Origin`).
/// * While tunnel detection sees a tunnel whose upstream is this port, the
///   tokenless mode is off (`bridge_exposed_requires_token`): a raw TCP tunnel
///   is invisible to header checks.
/// * With [`TokenPolicy::Never`] every request passes without a token, from
///   anywhere — but while a tunnel points at the port, a tokenless request
///   counts as remote (not `local`, not `on_machine`), because nothing then
///   tells this machine apart from a raw TCP tunnel. Dangerous actions, which
///   an anonymous caller only has when `local`, stay with token holders.
/// * Anything else is `401`; a tunnelled request to the tokenless port gets
///   `tunneled_request_requires_token`.
async fn require_bridge_auth(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(policy): axum::Extension<MountPolicy>,
    ctx: crate::server::context::ClientContext,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if ctx.principal.is_user() {
        if let Some(id) = ctx.device_id() {
            return run_until_revoked(state, id.to_string(), req, next).await;
        }
        return next.run(req).await;
    }
    if policy.token == TokenPolicy::Never {
        if state.remote.bridge_exposed() && (ctx.local || ctx.on_machine) {
            let mut remote = ctx;
            remote.local = false;
            remote.on_machine = false;
            req.extensions_mut().insert(remote);
        }
        return next.run(req).await;
    }
    if !policy.require_token() {
        if state.remote.bridge_exposed() {
            return error::BridgeError::new(
                axum::http::StatusCode::UNAUTHORIZED,
                "bridge_exposed_requires_token",
                "A tunnel points at this port, so tokenless access is switched off until it \
                 stops. Stop the tunnel, or send a Mother Claude token as `Authorization: \
                 Bearer <token>`.",
            )
            .into_response();
        }
        // A development server or reverse proxy on this machine (all its
        // X-Forwarded-For hops loopback, nothing a tunnel adds) is a local
        // tool too.
        if ctx.on_machine
            || crate::server::context::is_local_dev_proxy(ctx.ingress, ctx.peer, req.headers())
        {
            return next.run(req).await;
        }
        return error::BridgeError::new(
            axum::http::StatusCode::UNAUTHORIZED,
            "tunneled_request_requires_token",
            "This request came through a proxy or tunnel; the tokenless mode is only for \
             clients on this machine (a development proxy here is fine when every \
             X-Forwarded-For hop is loopback). From elsewhere, use a paired device's token.",
        )
        .into_response();
    }
    error::BridgeError::unauthorized(
        "This endpoint requires a Mother Claude token. Send it as \
         `Authorization: Bearer <token>` (a paired device's token, or the desktop token \
         from this machine).",
    )
    .into_response()
}

fn device_revoked() -> error::BridgeError {
    error::BridgeError::new(
        axum::http::StatusCode::UNAUTHORIZED,
        "device_revoked",
        "This device was unpaired on the Mother Claude desktop app.",
    )
}

/// Serve a paired device's request, but never past the device's revocation:
/// a long-running handler (a run, a chat) answers `401 device_revoked`, and an
/// event stream ends with a terminal `bridge/error` frame. (Revoking already
/// closes the device's WebSockets; this covers `/v1`.)
async fn run_until_revoked(
    state: AppState,
    id: String,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use futures_util::StreamExt as _;

    let response = {
        let state = state.clone();
        let id = id.clone();
        tokio::select! {
            r = next.run(req) => r,
            _ = async move { state.devices.until_revoked(&id).await } => {
                return device_revoked().into_response();
            }
        }
    };
    let streaming = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| c.starts_with("text/event-stream"));
    if !streaming {
        return response;
    }
    let (parts, body) = response.into_parts();
    let revoked: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        Box::pin(async move { state.devices.until_revoked(&id).await });
    let frame = format!(
        "data: {}\n\n",
        serde_json::json!({
            "method": "bridge/error",
            "params": { "status": "failed", "error": device_revoked().body()["error"] },
        })
    );
    let stream = futures_util::stream::unfold(
        (body.into_data_stream(), Some(revoked)),
        move |(mut body, revoked)| {
            let frame = frame.clone();
            async move {
                let mut revoked = revoked?;
                tokio::select! {
                    item = body.next() => item.map(|i| (i, (body, Some(revoked)))),
                    _ = &mut revoked => Some((Ok(axum::body::Bytes::from(frame)), (body, None))),
                }
            }
        },
    );
    axum::response::Response::from_parts(parts, axum::body::Body::from_stream(stream))
}

/// Reject cross-origin browser requests from pages we do not trust.
///
/// CORS alone is not enough. A "simple" cross-origin POST is *sent* even when
/// the response cannot be read, so a page on the open web could fire commands
/// at a tokenless loopback bridge and never need to see the answer. Checking
/// `Origin` server-side is what actually stops that; the JSON content-type
/// requirement is the second lock on the same door.
///
/// A genuinely local request (`ClientContext::local`: this machine, no proxy,
/// and a local or absent `Origin`) always passes — its origin is the desktop
/// webview or a page served from localhost.
async fn guard_origin(
    axum::extract::State(state): axum::extract::State<AppState>,
    ctx: crate::server::context::ClientContext,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if ctx.local {
        return next.run(req).await;
    }
    let origin = req
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    match origin {
        // No Origin header: curl, a native client, or a same-origin navigation.
        None => next.run(req).await,
        Some(origin) if state.bridge.config.origin_allowed(&origin, host.as_deref()) => {
            next.run(req).await
        }
        Some(origin) => error::BridgeError::forbidden(
            "origin_not_allowed",
            format!(
                "Origin {origin} is not allowed. Set MOTHER_CLAUDE_BRIDGE_ORIGINS                  to a comma-separated allowlist to permit it."
            ),
        )
        .with("origin", origin)
        .into_response(),
    }
}

async fn not_found(uri: axum::http::Uri) -> axum::response::Response {
    error::BridgeError::not_found("not_found", format!("No such endpoint: {}", uri.path()))
        .into_response()
}

async fn method_not_allowed(
    method: axum::http::Method,
    uri: axum::http::Uri,
) -> axum::response::Response {
    error::BridgeError::new(
        axum::http::StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        format!("{method} is not supported on {}.", uri.path()),
    )
    .into_response()
}

/// The app served on the dedicated loopback port: the same routes at `/`
/// (so `http://127.0.0.1:5612/threads` works) and under `/v1` (so one client
/// base URL works against either listener).
///
/// Loopback only, always: the bridge can drive a coding agent with filesystem
/// write access, and its tokenless mode must never be reachable from the
/// network. LAN clients use the `/v1` mount on the main (TLS + token) server.
pub fn listener_router(state: AppState, token: TokenPolicy) -> Router {
    let config = state.bridge.config.clone();
    // The same routes at `/` (Codex-compatible) and under `/v1` (so one client
    // base URL works against either listener).
    // CORS mirrors `origin_allowed`: open unless an allowlist is configured, so
    // a browser app can call the bridge out of the box and a deployment that
    // wants it locked down gets exactly that by naming its origins.
    let mut cors = tower_http::cors::CorsLayer::new()
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
        .expose_headers([axum::http::HeaderName::from_static("x-operation-id")]);
    if config.allowed_origins.is_empty() {
        cors = cors.allow_origin(tower_http::cors::Any);
    } else {
        for origin in &config.allowed_origins {
            match origin.parse::<axum::http::HeaderValue>() {
                Ok(value) => cors = cors.allow_origin(value),
                Err(_) => tracing::warn!(%origin, "ignoring an unparsable bridge origin"),
            }
        }
    }

    let app = Router::new()
        .merge(router(state.clone(), token))
        .nest("/v1", router(state.clone(), token))
        .layer(cors)
        .with_state(state.clone());
    crate::server::context::layer(app, state, crate::server::context::Ingress::BridgeLoopback)
}

/// Find a stored conversation by id, reading `~/.claude` through the adapter.
///
/// Deliberately not a runtime call: "is there such a conversation?" must be
/// answerable without starting a Node process, so that naming a nonexistent id
/// is a 404 about the id rather than a 502 about a runtime that was only
/// spawned to say no.
async fn stored_thread(
    state: &AppState,
    thread_id: &str,
) -> Option<crate::claude::TranscriptSummary> {
    let state = state.clone();
    let id = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        crate::claude::registry::stored_transcript_in(&state.index, &state.accounts.homes(), &id)
    })
    .await
    .ok()
    .flatten()
}

/// Whether a conversation exists at all — live in the host, or on disk.
///
/// Used before nominating one as the default, so a stale pick fails on the
/// button that made it rather than on the first message sent afterwards.
pub async fn thread_exists(state: &AppState, thread_id: &str) -> bool {
    state.bridge.thread(thread_id).await.is_some()
        || stored_thread(state, thread_id).await.is_some()
}

/// Make sure a conversation is live in the host, resuming it from disk if it
/// is not.
///
/// This is what lets a client name any past conversation — or nominate one as
/// the default before the bridge has ever run it — and simply keep talking.
/// `ctx` is the caller: resuming into a default permission mode that removes
/// the human from the loop (bypassPermissions / dontAsk) is refused for a
/// caller that may not perform dangerous actions (design G1).
pub async fn ensure_thread_live(
    state: &AppState,
    ctx: &crate::server::context::ClientContext,
    thread_id: &str,
) -> error::BridgeResult<()> {
    if state.bridge.thread(thread_id).await.is_some() {
        return Ok(());
    }
    let stored = stored_thread(state, thread_id)
        .await
        .ok_or_else(|| error::BridgeError::thread_not_found(thread_id))?;
    // Never resume a session another process still runs (or Mother Claude
    // already drives): two writers on one transcript, and marking it owned
    // would silence its hooks. Silently forking would break a caller that
    // keeps using this id, so say how to fork.
    if routes::threads::takeover(state, thread_id).await? == routes::threads::Takeover::Fork {
        return Err(error::BridgeError::conflict(
            "thread_live_elsewhere",
            "That conversation is running in another window. Fork it with POST /threads \
             {\"resume\": \"<id>\", \"fork\": true} and talk to the fork.",
        )
        .with("thread_id", thread_id.to_string()));
    }

    let defaults = state.bridge.defaults().await;
    if crate::claude::owned::is_dangerous_mode(defaults.permission_mode.as_deref())
        && !ctx.may_dangerous()
    {
        return Err(error::BridgeError::forbidden(
            "dangerous_blocked",
            "The bridge's default permission mode skips approvals, so resuming a \
             conversation with it is restricted to local clients.",
        ));
    }
    // Resume where the conversation actually lived. Reopening it somewhere else
    // would silently change which files it can see.
    let cwd = stored
        .cwd
        .clone()
        .or_else(|| defaults.cwd.clone())
        .unwrap_or_else(routes::threads::default_cwd);

    // It continues under the account whose directory holds it (one that ran
    // with a token no account holds, under the bridge's).
    let account = crate::claude::summaries::session_account(state, thread_id);
    let account = if state.accounts.is_usable(&account) {
        account
    } else {
        state.accounts.account_for_bridge(&cwd)
    };
    let env = state
        .accounts
        .env(&account)
        .map_err(routes::threads::account_error)?;
    let mut options = serde_json::json!({
        "cwd": cwd,
        "resume": thread_id,
        "permissionMode": defaults.permission_mode.clone().unwrap_or_else(|| "default".into()),
        "includePartialMessages": true,
    });
    if !env.is_inherit() {
        options["env"] = serde_json::json!(env.overrides());
    }
    for (key, value) in [
        ("model", defaults.model.clone()),
        ("effort", defaults.effort.clone()),
        ("thinking", defaults.thinking.clone()),
        ("systemPromptAppend", defaults.system_prompt_append.clone()),
    ] {
        if let Some(v) = value {
            options[key] = serde_json::json!(v);
        }
    }
    if let Some(sources) = &defaults.setting_sources {
        options["settingSources"] = serde_json::json!(sources);
    }

    state
        .bridge
        .host
        .call(
            state,
            "session.create",
            serde_json::json!({ "sessionId": thread_id, "options": options }),
            host::CREATE_TIMEOUT,
        )
        .await?;

    let record = ThreadRecord {
        thread_id: thread_id.to_string(),
        cwd: cwd.clone(),
        created_at: crate::claude::registry::now_ms(),
        model: defaults.model.clone(),
        effort: defaults.effort.clone(),
        permission_mode: defaults
            .permission_mode
            .clone()
            .unwrap_or_else(|| "default".into()),
        title: stored.title.clone(),
        persisted: true,
        active_turn: None,
        account,
    };
    state.bridge.insert_thread(record.clone()).await;
    state.mark_owned(thread_id, &cwd, record.created_at).await;
    Ok(())
}

/// The permission mode an approval result switches its session to: the
/// last `setMode` update (destination `session` or `cliArg`) of an allow.
pub fn mode_set_by(result: &Value) -> Option<String> {
    if result.get("behavior").and_then(Value::as_str) != Some("allow") {
        return None;
    }
    result
        .get("updatedPermissions")
        .and_then(Value::as_array)?
        .iter()
        .filter(|u| u.get("type").and_then(Value::as_str) == Some("setMode"))
        .filter(|u| {
            matches!(
                u.get("destination").and_then(Value::as_str),
                None | Some("session") | Some("cliArg")
            )
        })
        .filter_map(|u| u.get("mode").and_then(Value::as_str))
        .map(str::to_string)
        .next_back()
}

/// A conversation now runs in `mode` (the host reported it, or an approval
/// switched it): keep the bridge record, the dashboard's mode note and the
/// summary in step, so the "no unapproved execution" gates see it.
pub async fn note_thread_mode(state: &AppState, thread_id: &str, mode: &str) {
    if mode.is_empty() || mode.len() > 40 {
        return;
    }
    let m = mode.to_string();
    state
        .bridge
        .update_thread(thread_id, move |r| r.permission_mode = m)
        .await;
    state.activity.note_permission_mode(thread_id, mode);
    let m = mode.to_string();
    crate::server::ws::patch_session(state, thread_id, move |s| s.permission_mode = Some(m)).await;
}

/// Start the bridge at boot when `MOTHER_CLAUDE_BRIDGE_AUTOSTART=1`.
///
/// Off by default: the app should not publish this port just because it opened.
/// The flag exists for headless and CI runs, where there is no one to press the
/// button.
pub async fn autostart(state: AppState) {
    if !state.bridge.config.enabled {
        tracing::info!("Claude HTTP bridge disabled (MOTHER_CLAUDE_BRIDGE=0)");
        return;
    }
    if !env_flag("MOTHER_CLAUDE_BRIDGE_AUTOSTART", false) {
        return;
    }
    match control::start(&state, control::BridgeDefaults::default()).await {
        Ok(_) => {
            if let Some(port) = state.bridge.config.port {
                println!("  Bridge: http://127.0.0.1:{port}  (autostarted)");
            }
        }
        Err(e) => tracing::error!(error = %e, "could not autostart the Claude bridge"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Env-var parsing is shared by every knob, so pin its edges.
    #[test]
    fn flags_default_on_and_accept_the_usual_off_spellings() {
        let name = "MOTHER_CLAUDE_TEST_FLAG";
        std::env::remove_var(name);
        assert!(env_flag(name, true));
        assert!(!env_flag(name, false));
        for off in ["0", "false", "OFF", "no"] {
            std::env::set_var(name, off);
            assert!(!env_flag(name, true), "{off} should read as off");
        }
        std::env::set_var(name, "1");
        assert!(env_flag(name, false));
        std::env::remove_var(name);
    }

    #[tokio::test]
    async fn thread_records_round_trip_and_sort_newest_first() {
        let bridge = BridgeState::new(BridgeConfig::for_test());
        for (id, created) in [("old", 1_i64), ("new", 2)] {
            bridge
                .insert_thread(ThreadRecord {
                    thread_id: id.into(),
                    cwd: "/tmp".into(),
                    created_at: created,
                    model: None,
                    effort: None,
                    permission_mode: "default".into(),
                    title: None,
                    persisted: false,
                    active_turn: None,
                    account: crate::claude::accounts::DEFAULT_ID.into(),
                })
                .await;
        }
        let listed = bridge.threads().await;
        assert_eq!(listed[0].thread_id, "new");
        assert_eq!(listed[1].thread_id, "old");

        // Starting a turn marks the conversation persisted.
        bridge.set_active_turn("new", Some("turn-1".into())).await;
        let record = bridge.thread("new").await.unwrap();
        assert_eq!(record.active_turn.as_deref(), Some("turn-1"));
        assert!(record.persisted);

        bridge.set_active_turn("new", None).await;
        assert!(bridge.thread("new").await.unwrap().active_turn.is_none());

        bridge.forget_thread("new").await;
        assert!(bridge.thread("new").await.is_none());
    }

    #[test]
    fn origins_are_open_until_an_allowlist_is_configured() {
        let mut config = BridgeConfig::for_test();
        assert!(config.allowed_origins.is_empty());

        // Open by default: a page being developed anywhere can call the bridge.
        assert!(config.origin_allowed("http://localhost:4200", Some("127.0.0.1:5612")));
        assert!(config.origin_allowed("https://anything.example", Some("127.0.0.1:5612")));
        assert!(config.origin_allowed("http://127.0.0.1:5612", None));

        // Naming one origin switches to strict allowlisting.
        config.allowed_origins = vec!["http://localhost:4200".into()];
        assert!(config.origin_allowed("http://localhost:4200", Some("127.0.0.1:5612")));
        assert!(!config.origin_allowed("https://anything.example", Some("127.0.0.1:5612")));
        // Same-origin survives it, so the bundled console keeps working.
        assert!(config.origin_allowed("http://127.0.0.1:5612", Some("127.0.0.1:5612")));
        assert!(config.origin_allowed("https://192.168.1.9:6725", Some("192.168.1.9:6725")));
        assert!(!config.origin_allowed("https://anything.example", None));
    }

    #[test]
    fn the_loopback_port_is_tokenless_unless_asked_otherwise() {
        std::env::remove_var("MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN");
        assert_eq!(BridgeConfig::from_env().token, TokenPolicy::Local);
        std::env::set_var("MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN", "1");
        assert_eq!(BridgeConfig::from_env().token, TokenPolicy::Required);
        std::env::set_var("MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN", "never");
        assert_eq!(BridgeConfig::from_env().token, TokenPolicy::Never);
        std::env::remove_var("MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN");
    }

    /// Only `never` opens the port to tunnels; the switch-off words keep the
    /// local default; anything else — a typo, an empty value — requires the
    /// token rather than guessing in the open direction.
    #[test]
    fn the_token_policy_reads_every_spelling_and_fails_closed() {
        assert_eq!(TokenPolicy::parse(None), TokenPolicy::Local);
        for local in ["0", "false", "off", "no", " OFF ", "No"] {
            assert_eq!(
                TokenPolicy::parse(Some(local)),
                TokenPolicy::Local,
                "{local:?}"
            );
        }
        for never in ["never", "NEVER", " Never "] {
            assert_eq!(
                TokenPolicy::parse(Some(never)),
                TokenPolicy::Never,
                "{never:?}"
            );
        }
        for required in ["1", "true", "on", "yes", "", "nevr", "open"] {
            assert_eq!(
                TokenPolicy::parse(Some(required)),
                TokenPolicy::Required,
                "{required:?}"
            );
        }
        assert!(TokenPolicy::Required.requires_token());
        assert!(!TokenPolicy::Local.requires_token() && !TokenPolicy::Never.requires_token());
    }

    /// `never`'s one safety rule, through the real context layer on a real
    /// loopback socket: no token from anywhere, but while a tunnel points at
    /// the port no tokenless request counts as local — not even one that looks
    /// local, because a raw TCP tunnel looks exactly like that — so none of
    /// them has the dangerous right.
    #[tokio::test]
    async fn the_never_policy_drops_the_token_but_not_the_proof_of_being_local() {
        let home = tempfile::tempdir().unwrap();
        let state = crate::state::Inner::with_bridge_config(
            crate::claude::ClaudeHome::with_base(home.path()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
            BridgeConfig {
                token: TokenPolicy::Never,
                ..BridgeConfig::for_test()
            },
        );
        async fn probe(ctx: crate::server::context::ClientContext) -> axum::Json<Value> {
            axum::Json(serde_json::json!({
                "local": ctx.local,
                "dangerous": ctx.may_dangerous(),
            }))
        }
        let app = Router::new()
            .route("/probe", get(probe))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_bridge_auth,
            ))
            .layer(axum::Extension(MountPolicy {
                token: TokenPolicy::Never,
            }))
            .with_state(state.clone());
        let app = crate::server::context::layer(
            app,
            state.clone(),
            crate::server::context::Ingress::BridgeLoopback,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/probe", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let http = reqwest::Client::new();
        let ask = |forwarded_for: Option<&'static str>| {
            let mut req = http.get(&base);
            if let Some(ip) = forwarded_for {
                req = req.header("x-forwarded-for", ip);
            }
            async move {
                let resp = req.send().await.unwrap();
                assert!(resp.status().is_success(), "got {}", resp.status());
                resp.json::<Value>().await.unwrap()
            }
        };

        // Nothing points at the port: a caller here is local and keeps the
        // dangerous right, exactly as with the default policy.
        assert_eq!(
            ask(None).await,
            serde_json::json!({ "local": true, "dangerous": true })
        );
        // Through a tunnel: no token needed, and not local.
        assert_eq!(
            ask(Some("203.0.113.7")).await,
            serde_json::json!({ "local": false, "dangerous": false })
        );
        // A tunnel points at the port: the same local-looking request is
        // remote now, and only a token holder could do something dangerous.
        state.remote.set_detected(
            vec![crate::server::remote::DetectedTunnel {
                provider: "ngrok".into(),
                url: "https://me.ngrok.app".into(),
                upstream_port: Some(5612),
                kind: "tcp".into(),
                target: "bridge".into(),
                ..Default::default()
            }],
            Some(5612),
        );
        assert!(state.remote.bridge_exposed());
        assert_eq!(
            ask(None).await,
            serde_json::json!({ "local": false, "dangerous": false })
        );
    }

    #[test]
    fn an_explicit_zero_port_means_no_dedicated_listener() {
        std::env::set_var("MOTHER_CLAUDE_BRIDGE_PORT", "0");
        assert!(BridgeConfig::from_env().port.is_none());
        std::env::set_var("MOTHER_CLAUDE_BRIDGE_PORT", "7777");
        assert_eq!(BridgeConfig::from_env().port, Some(7777));
        std::env::remove_var("MOTHER_CLAUDE_BRIDGE_PORT");
        assert_eq!(BridgeConfig::from_env().port, Some(DEFAULT_BRIDGE_PORT));
    }
}
