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
    /// Require the API token on the dedicated loopback port.
    ///
    /// **Off by default**, so `curl http://127.0.0.1:5612/capabilities` works
    /// with no setup — the point of a local bridge is that local tools can
    /// simply call it. Set `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=1` to require a
    /// bearer token here too. The `/v1` mount on the main, LAN-reachable
    /// server always requires one regardless of this setting.
    pub require_token: bool,
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
            require_token: env_flag("MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN", false),
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
            require_token: true,
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
}

impl BridgeState {
    pub fn new(config: BridgeConfig) -> Self {
        let messages = MessagesClient::new(config.api_key.clone());
        Self {
            config,
            ops: OperationRegistry::default(),
            host: RuntimeHost::new(),
            messages,
            threads: RwLock::new(HashMap::new()),
            listener: tokio::sync::Mutex::new(None),
            defaults: RwLock::new(control::BridgeDefaults::default()),
            active_thread: RwLock::new(None),
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
/// `require_token` gates only this instance: the `/v1` mount on the main server
/// always passes `true`, because that listener is reachable from the LAN.
/// Documentation, the OpenAPI document, the JS client and the example page are
/// public either way — they are static assets and reveal no token.
pub fn router(state: AppState, require_token: bool) -> Router<AppState> {
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

    let api = meta.merge(api);

    let api = if require_token {
        api.route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_bridge_token,
        ))
    } else {
        api
    };

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
        .layer(axum::Extension(MountPolicy { require_token }))
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
    pub require_token: bool,
}

/// Token check that answers with the bridge's error envelope.
///
/// `auth::require_token` returns a bare 401 with no body, which is right for the
/// dashboard but breaks the bridge's promise that every failure is
/// `{"error":{"code","message"}}`.
async fn require_bridge_token(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if crate::server::auth::request_is_authorized(&state, &req) {
        return next.run(req).await;
    }
    error::BridgeError::unauthorized(
        "This endpoint requires the Mother Claude API token. Send it as          `Authorization: Bearer <token>`, `?token=` or an `mc_token` cookie.",
    )
    .into_response()
}

/// Reject cross-origin browser requests from pages we do not trust.
///
/// CORS alone is not enough. A "simple" cross-origin POST is *sent* even when
/// the response cannot be read, so a page on the open web could fire commands
/// at a tokenless loopback bridge and never need to see the answer. Checking
/// `Origin` server-side is what actually stops that; the JSON content-type
/// requirement is the second lock on the same door.
async fn guard_origin(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
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
pub fn listener_router(state: AppState, require_token: bool) -> Router {
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

    Router::new()
        .merge(router(state.clone(), require_token))
        .nest("/v1", router(state.clone(), require_token))
        .layer(cors)
        .with_state(state)
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
    let home = state.home.clone();
    let id = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        crate::claude::scan_transcripts(&home)
            .into_iter()
            .find(|t| t.id == id)
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
pub async fn ensure_thread_live(state: &AppState, thread_id: &str) -> error::BridgeResult<()> {
    if state.bridge.thread(thread_id).await.is_some() {
        return Ok(());
    }
    let stored = stored_thread(state, thread_id)
        .await
        .ok_or_else(|| error::BridgeError::thread_not_found(thread_id))?;

    let defaults = state.bridge.defaults().await;
    // Resume where the conversation actually lived. Reopening it somewhere else
    // would silently change which files it can see.
    let cwd = stored
        .cwd
        .clone()
        .or_else(|| defaults.cwd.clone())
        .unwrap_or_else(routes::threads::default_cwd);

    let mut options = serde_json::json!({
        "cwd": cwd,
        "resume": thread_id,
        "permissionMode": defaults.permission_mode.clone().unwrap_or_else(|| "default".into()),
        "includePartialMessages": true,
    });
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
    };
    state.bridge.insert_thread(record.clone()).await;
    state.mark_owned(thread_id, &cwd, record.created_at).await;
    Ok(())
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
        assert!(!BridgeConfig::from_env().require_token);
        std::env::set_var("MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN", "1");
        assert!(BridgeConfig::from_env().require_token);
        std::env::remove_var("MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN");
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
