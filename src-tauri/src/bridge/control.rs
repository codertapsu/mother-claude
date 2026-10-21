//! Starting, stopping and configuring the API servers from the dashboard.
//!
//! An API server does not listen until you ask it to. That is deliberate:
//! starting one publishes an unauthenticated port that can drive Claude with
//! your account, and the moment that happens should be a thing you did, not a
//! side effect of opening the app.
//!
//! What you choose at start time becomes the *defaults* for every conversation
//! the server creates — model, effort, working directory, permission mode,
//! account — and every one of them can still be overridden per request (only
//! the account cannot, on a server that runs as one account only). You can
//! also nominate a conversation for new messages to join, so a client that
//! only ever sends `{"message": "…"}` keeps one continuous thread instead of a
//! new one each time.
//!
//! The **main** server is the one there always was (`/api/bridge/start`,
//! `/stop`, `/active-thread`); its defaults are never saved. Servers added on
//! the API screen are saved with theirs in `api-servers.json`
//! (docs/ACCOUNTS.md §13.5) and driven through `/api/bridge/servers/…`.
//!
//! These handlers live on the **dashboard** API (`/api/bridge/…`), not on the
//! bridge itself — you have to be able to start something that is not running.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::error::{BridgeError, BridgeResult};
use super::routes::threads::{EFFORT_LEVELS, PERMISSION_MODES, SETTING_SOURCES};
use super::servers::{self, ApiServer, MAIN_ID};
use super::TokenPolicy;
use crate::server::auth::Authed;
use crate::server::context::ClientContext;
use crate::state::AppState;

/// Defaults applied to every conversation an API server creates.
///
/// Every field is optional and every one is overridable per request; `None`
/// means "whatever the user's own Claude settings say", which is the same rule
/// the dashboard's launch form follows.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeDefaults {
    /// Working directory for new conversations.
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// "on" | "off".
    pub thinking: Option<String>,
    pub permission_mode: Option<String>,
    /// Which of `user` / `project` / `local` settings to load. `[]` isolates.
    pub setting_sources: Option<Vec<String>>,
    pub system_prompt_append: Option<String>,
    /// The conversation new messages join when the caller does not name one.
    ///
    /// `None` means the first message creates a conversation and everything
    /// after it joins that one, which is what a client sending only
    /// `{"message": "…"}` almost always wants.
    pub default_thread: Option<String>,
    /// The Claude account conversations run under (docs/ACCOUNTS.md §5.5);
    /// `None`: the project's, else the accounts' bridge default. Left out
    /// when unset, so `GET /api/bridge` reads as before accounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Run **only** as `account` (docs/ACCOUNTS.md §13.5): a request naming
    /// another is refused, and another account's conversations, operations
    /// and runtime requests do not exist through this server. Left out when
    /// false, so `GET /api/bridge` reads as before.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub account_only: bool,
}

/// The fields of [`BridgeDefaults`] as they are written (camelCase): what a
/// saved server's defaults are read by.
pub const DEFAULTS_FIELDS: [&str; 10] = [
    "cwd",
    "model",
    "effort",
    "thinking",
    "permissionMode",
    "settingSources",
    "systemPromptAppend",
    "defaultThread",
    "account",
    "accountOnly",
];

fn one_of(field: &str, value: &Option<String>, allowed: &[&str]) -> BridgeResult<()> {
    if let Some(v) = value {
        if !allowed.contains(&v.as_str()) {
            return Err(BridgeError::invalid_field(
                field,
                format!(
                    "Unknown {field} {v:?}; expected one of {}.",
                    allowed.join(", ")
                ),
            ));
        }
    }
    Ok(())
}

impl BridgeDefaults {
    /// Reject anything the runtime would reject later, while the person is
    /// still looking at the form.
    pub fn validate(&self) -> BridgeResult<()> {
        one_of("effort", &self.effort, &EFFORT_LEVELS)?;
        one_of("permissionMode", &self.permission_mode, &PERMISSION_MODES)?;
        one_of("thinking", &self.thinking, &["on", "off"])?;
        if let Some(sources) = &self.setting_sources {
            for source in sources {
                if !SETTING_SOURCES.contains(&source.as_str()) {
                    return Err(BridgeError::invalid_field(
                        "settingSources",
                        format!(
                            "Unknown setting source {source:?}; expected one of {}.",
                            SETTING_SOURCES.join(", ")
                        ),
                    ));
                }
            }
        }
        if let Some(cwd) = &self.cwd {
            if !cwd.trim().is_empty() && !std::path::Path::new(cwd).is_dir() {
                return Err(BridgeError::invalid_field(
                    "cwd",
                    format!("{cwd} is not a directory on this machine."),
                ));
            }
        }
        if self.account_only && self.account.is_none() {
            return Err(BridgeError::invalid_field(
                "accountOnly",
                "Running as one account only needs that account: choose one.",
            ));
        }
        Ok(())
    }

    /// Blank strings from a web form mean "unset", not "empty".
    pub fn normalized(mut self) -> Self {
        let blank = |v: &mut Option<String>| {
            if v.as_deref().map(str::trim).is_some_and(str::is_empty) {
                *v = None;
            }
        };
        blank(&mut self.cwd);
        blank(&mut self.model);
        blank(&mut self.effort);
        blank(&mut self.thinking);
        blank(&mut self.permission_mode);
        blank(&mut self.system_prompt_append);
        blank(&mut self.default_thread);
        blank(&mut self.account);
        self
    }

    /// The one account these defaults run as, when they say `accountOnly`.
    pub fn only_account(&self) -> Option<&str> {
        self.account.as_deref().filter(|_| self.account_only)
    }
}

/// A bound, serving API server listener.
pub struct Listener {
    pub port: u16,
    pub started_at: i64,
    /// Dropped or fired to unbind the port.
    shutdown: Option<oneshot::Sender<()>>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("port", &self.port)
            .field("started_at", &self.started_at)
            .finish()
    }
}

impl Listener {
    fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

// ─────────────────────────────── running a server ──────────────────────────

/// Bind `port` on loopback. Right after a restart in place the listener just
/// told to stop may hold the port for a moment, so then it is tried again for
/// up to a second.
async fn bind(port: u16, retry: bool) -> std::io::Result<tokio::net::TcpListener> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut tries = if retry && port != 0 { 40 } else { 1 };
    loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(socket) => return Ok(socket),
            Err(e) if tries > 1 && e.kind() == std::io::ErrorKind::AddrInUse => {
                tries -= 1;
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Check `defaults` for a start of `server`: what the runtime would refuse,
/// an account that does not exist, a nominated conversation that does not
/// (or is another account's, on a server that runs as one), a port another
/// listener has.
async fn prepare(
    state: &AppState,
    server: &ApiServer,
    defaults: BridgeDefaults,
) -> BridgeResult<BridgeDefaults> {
    if !state.bridge.config.enabled {
        return Err(BridgeError::forbidden(
            "bridge_disabled",
            "The bridge is disabled for this process (MOTHER_CLAUDE_BRIDGE=0).",
        ));
    }
    let defaults = defaults.normalized();
    defaults.validate()?;
    if let Some(account) = &defaults.account {
        if state.accounts.get(account).is_none() {
            return Err(BridgeError::invalid_field(
                "account",
                format!("No account {account:?} — pick another, or leave it unset."),
            ));
        }
    }

    // A nominated conversation must actually exist, or the first message would
    // fail in a way that looks like the bridge is broken rather than like a
    // stale pick.
    if let Some(thread) = &defaults.default_thread {
        if !super::thread_exists(state, thread).await {
            return Err(BridgeError::invalid_field(
                "defaultThread",
                format!("No conversation {thread} — pick another, or leave it unset."),
            )
            .with("thread_id", thread.clone()));
        }
        if let Some(only) = defaults.only_account() {
            if servers::thread_account(state, thread) != only {
                return Err(BridgeError::invalid_field(
                    "defaultThread",
                    format!("Conversation {thread} runs under another account than this server's."),
                )
                .with("thread_id", thread.clone()));
            }
        }
    }
    if !server.is_main() {
        if let Some(port) = server.port() {
            servers::check_port(state, Some(&server.id), port)?;
        }
    }
    Ok(defaults)
}

/// Start `server` with `defaults` under `token`, replacing a run it has.
/// `persist`: an added server's saved defaults become these (written before
/// the port is bound, so a failed save starts nothing). Callers hold the
/// servers' admin lock.
pub(crate) async fn run(
    state: &AppState,
    server: &Arc<ApiServer>,
    defaults: BridgeDefaults,
    token: TokenPolicy,
    persist: bool,
) -> BridgeResult<()> {
    let defaults = prepare(state, server, defaults).await?;
    if persist && !server.is_main() {
        let before = server.saved();
        server.update_saved(|d| *d = defaults.clone());
        if let Err(e) = state.bridge.servers.save() {
            server.update_saved(|d| *d = before);
            return Err(e);
        }
    }

    let mut guard = server.listener.lock().await;
    let restarting = guard.as_ref().map(|l| l.port);
    if let Some(listener) = guard.as_mut() {
        // Restarting in place: keep the port, take the new defaults.
        listener.stop();
    }
    *guard = None;
    server.note_listener(false, None);
    server.apply_defaults(defaults);

    let Some(port) = server.port() else {
        // No dedicated port configured: the `/v1` mount is the whole surface,
        // and "started" is still meaningful because it ungates those routes.
        *guard = Some(Listener {
            port: 0,
            started_at: crate::claude::registry::now_ms(),
            shutdown: None,
        });
        server.set_token_policy(token);
        server.note_listener(true, None);
        drop(guard);
        announce(state, server, "started").await;
        return Ok(());
    };

    // Bind before reporting success, so "port already in use" is an error the
    // person sees on the button they pressed.
    let socket = bind(port, restarting.is_some_and(|p| p == port))
        .await
        .map_err(|e| {
            BridgeError::conflict(
                "port_unavailable",
                format!(
                    "Could not bind 127.0.0.1:{port}: {e}. Another process may already hold it."
                ),
            )
            .with("port", port)
        })?;

    // The port actually bound: the configured one, or the one the system
    // picked when a test asked for port 0.
    let port = socket.local_addr().map(|a| a.port()).unwrap_or(port);
    let open = token == TokenPolicy::Never;
    if open {
        tracing::warn!(
            port,
            server = %server.id,
            "the API port takes requests without a token from anyone who can reach it, \
             through a tunnel too"
        );
    }
    server.set_token_policy(token);
    let app = super::server_listener_router(state.clone(), &server.id, port);
    let (tx, rx) = oneshot::channel::<()>();
    let id = server.id.clone();
    tokio::spawn(async move {
        let served = axum::serve(
            socket,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = rx.await;
        })
        .await;
        if let Err(e) = served {
            tracing::error!(error = %e, server = %id, "API server listener stopped");
        } else {
            tracing::info!(server = %id, "API server listener closed");
        }
    });

    *guard = Some(Listener {
        port,
        started_at: crate::claude::registry::now_ms(),
        shutdown: Some(tx),
    });
    server.note_listener(true, Some(port));
    drop(guard);

    announce(
        state,
        server,
        if open {
            "started — open without a token, through tunnels too"
        } else {
            "started"
        },
    )
    .await;
    Ok(())
}

/// Stop `server`'s listener. Conversations already created stay alive —
/// stopping the door does not evict the people inside it.
pub(crate) async fn halt(state: &AppState, server: &ApiServer) {
    let mut guard = server.listener.lock().await;
    if let Some(listener) = guard.as_mut() {
        listener.stop();
    }
    *guard = None;
    server.note_listener(false, None);
    drop(guard);
    announce(state, server, "stopped").await;
}

async fn announce(state: &AppState, server: &ApiServer, what: &str) {
    let running = server.is_running();
    if server.is_main() {
        tracing::info!(running, "Claude bridge {what}");
        state.notice(format!("Claude bridge {what}"));
    } else {
        let name = server.name();
        tracing::info!(running, server = %server.id, "API server {name} {what}");
        state.notice(format!("API server \"{name}\" {what}"));
    }
}

// ─────────────────────────────── the main server ───────────────────────────

/// Start the main server with `defaults`, replacing any previous run, under
/// the configured token policy (`MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN`).
pub async fn start(state: &AppState, defaults: BridgeDefaults) -> BridgeResult<Value> {
    start_with(state, defaults, state.bridge.config.token).await
}

/// [`start`] under a token policy chosen at start time — the API screen's
/// "open to tunnels" switch. Callers decide who may choose which; this only
/// applies it.
pub async fn start_with(
    state: &AppState,
    defaults: BridgeDefaults,
    token: TokenPolicy,
) -> BridgeResult<Value> {
    let main = state.bridge.main();
    {
        let _admin = state.bridge.servers.admin.lock().await;
        run(state, &main, defaults, token, false).await?;
    }
    Ok(status(state).await)
}

/// Stop the main server's listener.
pub async fn stop(state: &AppState) -> BridgeResult<Value> {
    let main = state.bridge.main();
    {
        let _admin = state.bridge.servers.admin.lock().await;
        halt(state, &main).await;
    }
    Ok(status(state).await)
}

/// Whether a tunnel points at one of `server`'s ports.
pub fn exposed(state: &AppState, server: &ApiServer) -> bool {
    server
        .ports()
        .iter()
        .any(|port| state.remote.port_exposed(*port))
}

/// The public URL of a tunnel that points at one of `server`'s ports.
pub fn tunnel_url(state: &AppState, server: &ApiServer) -> Option<String> {
    let ports = server.ports();
    state
        .remote
        .detected()
        .into_iter()
        .find(|t| t.upstream_port.is_some_and(|p| ports.contains(&p)))
        .map(|t| t.url)
}

/// The conversations `server` made, for the dashboard. Projected to the
/// dashboard's camelCase convention. `ThreadRecord` is snake_case because the
/// bridge's own API mirrors the Codex one; this payload belongs to `/api/*`,
/// which is camelCase throughout.
async fn thread_views(state: &AppState, server: &str) -> Vec<Value> {
    state
        .bridge
        .threads()
        .await
        .into_iter()
        .filter(|t| t.server == server)
        .map(|t| {
            let mut v = json!({
                "threadId": t.thread_id,
                "cwd": t.cwd,
                "createdAt": t.created_at,
                "model": t.model,
                "effort": t.effort,
                "permissionMode": t.permission_mode,
                "title": t.title,
                "persisted": t.persisted,
                "activeTurn": t.active_turn,
            });
            // Its account's chip, once there is more than the Default.
            if let Some(account) = crate::claude::accounts::view::account_ref(state, &t.account) {
                v["account"] = json!(account);
            }
            v
        })
        .collect()
}

/// Everything the API screen needs in one payload: the main server as it
/// always was, plus `servers` (main first) once another one exists.
pub async fn status(state: &AppState) -> Value {
    let config = &state.bridge.config;
    let main = state.bridge.main();
    let token = main.token_policy();
    let listener = main.listener.lock().await;
    let running = listener.is_some();
    let port = listener.as_ref().map(|l| l.port).or(config.port);
    let started_at = listener.as_ref().map(|l| l.started_at);
    drop(listener);

    let url = port
        .filter(|p| *p != 0)
        .map(|p| format!("http://127.0.0.1:{p}"));
    let threads = thread_views(state, MAIN_ID).await;
    let auth = state.claude_auth.read().await.clone();

    let mut out = json!({
        "enabled": config.enabled,
        "running": running,
        "port": port,
        "url": url,
        "startedAt": started_at,
        "requireToken": token.requires_token(),
        // `required` | `local` | `never` — the running (or last) start's policy;
        // the API screen warns about `never`, loudest while a tunnel points at
        // the port. `configuredTokenPolicy` is what MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN
        // says, and so what the "open to tunnels" switch means when it is off.
        "tokenPolicy": token,
        "configuredTokenPolicy": config.token,
        "exposedByTunnel": exposed(state, &main),
        "tunnelUrl": tunnel_url(state, &main),
        "hostBuilt": super::host::host_entry().is_some(),
        "hostRunning": state.bridge.host.is_running().await,
        "messagesApi": state.bridge.messages.is_configured(),
        "claudeAuthenticated": auth.logged_in,
        "claudeAccount": auth.summary(),
        "activeOperations": state.bridge.ops.active_count(),
        "defaults": main.defaults(),
        "activeThread": main.active_thread(),
        "threads": threads,
        "docsUrl": url.as_ref().map(|u| format!("{u}/docs/")),
        "consoleUrl": url.as_ref().map(|u| format!("{u}/example/")),
    });
    if state.bridge.servers.has_added() {
        let mut list = Vec::new();
        for server in state.bridge.servers.all() {
            list.push(server_status(state, &server).await);
        }
        out["servers"] = json!(list);
    }
    out
}

/// One server's part of the API screen (docs/ACCOUNTS.md §13.5).
pub async fn server_status(state: &AppState, server: &ApiServer) -> Value {
    let listener = server.listener.lock().await;
    let running = listener.is_some();
    let port = listener.as_ref().map(|l| l.port).or(server.port());
    let started_at = listener.as_ref().map(|l| l.started_at);
    drop(listener);

    let url = port
        .filter(|p| *p != 0)
        .map(|p| format!("http://127.0.0.1:{p}"));
    let token = server.token_policy();
    let defaults = server.defaults();
    let missing = defaults
        .account
        .as_deref()
        .is_some_and(|a| state.accounts.get(a).is_none());
    let mut out = json!({
        "id": server.id,
        "name": server.name(),
        "running": running,
        "port": port,
        "url": url,
        "prefix": server.prefix(),
        "startedAt": started_at,
        "requireToken": token.requires_token(),
        "tokenPolicy": token,
        "exposedByTunnel": exposed(state, server),
        "tunnelUrl": tunnel_url(state, server),
        "defaults": defaults,
        "activeThread": server.active_thread(),
        "threads": thread_views(state, &server.id).await,
        "docsUrl": url.as_ref().map(|u| format!("{u}/docs/")),
        "consoleUrl": url.as_ref().map(|u| format!("{u}/example/")),
    });
    if missing {
        // Its account was removed: it cannot start until another is chosen.
        out["accountMissing"] = json!(true);
    }
    out
}

// ─────────────────────────────── added servers ─────────────────────────────

/// An added server by id: `404 server_not_found`, and `409
/// main_server_fixed` for the main one (its port comes from the
/// environment, and it is never removed).
fn added(state: &AppState, id: &str) -> BridgeResult<Arc<ApiServer>> {
    let server = state
        .bridge
        .server(id)
        .ok_or_else(|| servers::server_not_found(id))?;
    if server.is_main() {
        return Err(BridgeError::conflict(
            "main_server_fixed",
            "The main API server keeps its port (MOTHER_CLAUDE_BRIDGE_PORT) and cannot be \
             changed or removed here; start it with other defaults instead.",
        )
        .with("server", MAIN_ID));
    }
    Ok(server)
}

/// `409 server_name_taken` when another server has this name (in any case).
fn name_free(state: &AppState, name: &str, except: Option<&str>) -> BridgeResult<()> {
    let taken = state
        .bridge
        .servers
        .all()
        .into_iter()
        .any(|s| Some(s.id.as_str()) != except && s.name().eq_ignore_ascii_case(name));
    if taken {
        return Err(BridgeError::conflict(
            "server_name_taken",
            format!("An API server named {name:?} already exists."),
        )
        .with("field", "name"));
    }
    Ok(())
}

/// Defaults an added server may keep: what [`prepare`] checks but the port.
async fn checked_defaults(
    state: &AppState,
    defaults: BridgeDefaults,
) -> BridgeResult<BridgeDefaults> {
    let defaults = defaults.normalized();
    defaults.validate()?;
    if let Some(account) = &defaults.account {
        if state.accounts.get(account).is_none() {
            return Err(BridgeError::invalid_field(
                "account",
                format!("No account {account:?} — pick another, or leave it unset."),
            ));
        }
    }
    if let Some(thread) = &defaults.default_thread {
        let exists = super::thread_exists(state, thread).await;
        let foreign = defaults
            .only_account()
            .is_some_and(|only| servers::thread_account(state, thread) != only);
        if !exists || foreign {
            return Err(BridgeError::invalid_field(
                "defaultThread",
                format!(
                    "No conversation {thread} of this server's — pick another, or leave it unset."
                ),
            )
            .with("thread_id", thread.clone()));
        }
    }
    Ok(defaults)
}

/// Add an API server (not started). `port`: the first free from 5613 when
/// `None`; `Some(0)` — any free port at each start — is for tests and
/// embedders only (the dashboard route refuses it).
pub async fn add_server(
    state: &AppState,
    name: &str,
    port: Option<u16>,
    defaults: BridgeDefaults,
) -> BridgeResult<Arc<ApiServer>> {
    let name = servers::clean_name(name)?;
    let defaults = checked_defaults(state, defaults).await?;
    let _admin = state.bridge.servers.admin.lock().await;
    name_free(state, &name, None)?;
    let port = match port {
        Some(port) => {
            servers::check_port(state, None, port)?;
            port
        }
        None => servers::first_free_port(state)?,
    };
    let id = servers::unique_id(&servers::slug(&name), |id| {
        state.bridge.servers.get(id).is_some()
    });
    let server = ApiServer::new(
        &id,
        &name,
        Some(port),
        defaults,
        state.bridge.config.token.closed(),
    );
    let server = state.bridge.servers.insert(server)?;
    tracing::info!(server = %server.id, port, "API server added");
    Ok(server)
}

/// Change an added server's name, port or saved defaults; a running one is
/// restarted with them (under its run's token policy).
pub async fn update_server(
    state: &AppState,
    id: &str,
    name: Option<&str>,
    port: Option<u16>,
    defaults: Option<BridgeDefaults>,
) -> BridgeResult<Arc<ApiServer>> {
    let server = added(state, id)?;
    let name = name.map(servers::clean_name).transpose()?;
    let defaults = match defaults {
        Some(d) => Some(checked_defaults(state, d).await?),
        None => None,
    };
    let _admin = state.bridge.servers.admin.lock().await;
    if let Some(name) = &name {
        name_free(state, name, Some(id))?;
    }
    if let Some(port) = port {
        servers::check_port(state, Some(id), port)?;
    }
    let before = (server.name(), server.port(), server.saved());
    let moved = port.is_some_and(|p| Some(p) != server.port());
    if let Some(name) = name {
        server.set_name(name);
    }
    if let Some(port) = port {
        server.set_port(Some(port));
    }
    if let Some(defaults) = &defaults {
        server.update_saved(|d| *d = defaults.clone());
    }
    if let Err(e) = state.bridge.servers.save() {
        server.set_name(before.0);
        server.set_port(before.1);
        server.update_saved(|d| *d = before.2);
        return Err(e);
    }
    if server.is_running() && (moved || defaults.is_some()) {
        run(state, &server, server.saved(), server.token_policy(), false).await?;
    } else if defaults.is_some() {
        server.apply_defaults(server.saved());
    }
    Ok(server)
}

/// Remove an added server, stopped first. Its conversations go on, as the
/// main server's.
pub async fn remove_server(state: &AppState, id: &str) -> BridgeResult<()> {
    let server = added(state, id)?;
    let _admin = state.bridge.servers.admin.lock().await;
    if server.is_running() {
        halt(state, &server).await;
    }
    state.bridge.servers.remove(id)?;
    state.bridge.reassign_threads(id, MAIN_ID).await;
    tracing::info!(server = %id, "API server removed");
    Ok(())
}

/// Start a server with `defaults` — an added server keeps them as its saved
/// defaults — or, without, with the ones it has saved (`409
/// account_not_found` when their account was removed since).
pub async fn start_server(
    state: &AppState,
    id: &str,
    defaults: Option<BridgeDefaults>,
    token: TokenPolicy,
) -> BridgeResult<Arc<ApiServer>> {
    let server = state
        .bridge
        .server(id)
        .ok_or_else(|| servers::server_not_found(id))?;
    let _admin = state.bridge.servers.admin.lock().await;
    let persist = defaults.is_some();
    let defaults = match defaults {
        Some(defaults) => defaults,
        None => {
            let saved = server.saved();
            if let Some(account) = saved
                .account
                .as_deref()
                .filter(|a| state.accounts.get(a).is_none())
            {
                return Err(BridgeError::conflict(
                    "account_not_found",
                    format!(
                        "The account this API server runs under ({account}) was removed; \
                         choose another to start it."
                    ),
                )
                .with("field", "account")
                .with("account", account.to_string())
                .with("server", server.id.clone()));
            }
            saved
        }
    };
    run(state, &server, defaults, token, persist).await?;
    Ok(server)
}

/// Start a saved server at launch (`MOTHER_CLAUDE_BRIDGE_AUTOSTART=all`):
/// its saved defaults, closed to tunnels; a nominated conversation that is
/// gone meanwhile is dropped rather than keeping the server down.
pub(crate) async fn autostart_server(
    state: &AppState,
    server: &Arc<ApiServer>,
) -> BridgeResult<()> {
    if let Some(thread) = server.saved().default_thread {
        if !super::thread_exists(state, &thread).await {
            tracing::warn!(server = %server.id, thread_id = %thread,
                "the API server's nominated conversation is gone; starting without it");
            server.update_saved(|d| d.default_thread = None);
        }
    }
    start_server(state, &server.id, None, state.bridge.config.token.closed())
        .await
        .map(|_| ())
}

/// Stop a server.
pub async fn stop_server(state: &AppState, id: &str) -> BridgeResult<Arc<ApiServer>> {
    let server = state
        .bridge
        .server(id)
        .ok_or_else(|| servers::server_not_found(id))?;
    let _admin = state.bridge.servers.admin.lock().await;
    halt(state, &server).await;
    Ok(server)
}

/// Change which conversation `server`'s unaddressed messages join (`None`:
/// the next one starts a new conversation). One it does not show — gone, or
/// another account's on a server that runs as one — is `404`.
pub async fn set_active(
    state: &AppState,
    server: &ApiServer,
    thread_id: Option<String>,
) -> BridgeResult<()> {
    if let Some(thread) = &thread_id {
        let exists = super::thread_exists(state, thread).await;
        let foreign = server
            .only_account()
            .is_some_and(|only| servers::thread_account(state, thread) != only);
        if !exists || foreign {
            return Err(BridgeError::thread_not_found(thread));
        }
    }
    server.set_active_thread(thread_id);
    Ok(())
}

// --- dashboard handlers ----------------------------------------------------

/// `GET /api/bridge`
pub async fn get_status(Authed(_ctx): Authed, State(state): State<AppState>) -> Response {
    axum::Json(status(&state).await).into_response()
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartBody {
    #[serde(default)]
    pub defaults: BridgeDefaults,
    /// The API screen's "open to tunnels" switch. `true`: no token from anyone
    /// who can reach the port, tunnels included (`TokenPolicy::Never`), and
    /// only the desktop app may ask for it. `false`: the configured policy with
    /// that opening closed. Absent: the configured policy as it is.
    #[serde(default)]
    pub open_to_tunnels: Option<bool>,
}

/// The token policy a start of `server` asks for: the switch on — desktop
/// only, and only where there is a port to open — or off; absent, the
/// configured policy for the main server, and closed to tunnels for the
/// others ("open to tunnels" is chosen at each start, never saved).
fn start_token(
    state: &AppState,
    ctx: &ClientContext,
    server: &ApiServer,
    open: Option<bool>,
) -> BridgeResult<TokenPolicy> {
    let configured = state.bridge.config.token;
    Ok(match open {
        None if server.is_main() => configured,
        None | Some(false) => configured.closed(),
        Some(true) => {
            if !ctx.is_desktop() {
                return Err(BridgeError::forbidden(
                    "desktop_only",
                    "Opening the API port to callers without a token, through tunnels too, is \
                     only possible from the Mother Claude desktop app.",
                ));
            }
            if server.port().is_none() {
                return Err(BridgeError::invalid_field(
                    "openToTunnels",
                    "There is no dedicated API port (MOTHER_CLAUDE_BRIDGE_PORT=0), and the /v1 \
                     mount always needs a token.",
                ));
            }
            TokenPolicy::Never
        }
    })
}

/// The dangerous parts of a server's defaults keep their rules (design G1):
/// a permission mode that skips approvals is desktop-only — every
/// conversation the server then creates or resumes would inherit it — and a
/// default working directory the user never trusted would run its project
/// hooks and MCP servers unasked in every conversation (B1), so it needs the
/// dangerous right. `what` names the action in the refusal.
async fn check_defaults(
    state: &AppState,
    ctx: &ClientContext,
    defaults: &BridgeDefaults,
    what: &str,
) -> BridgeResult<()> {
    if crate::claude::owned::is_dangerous_mode(defaults.permission_mode.as_deref())
        && !ctx.is_desktop()
    {
        return Err(BridgeError::forbidden(
            "desktop_only",
            format!(
                "{what} with a permission mode that skips approvals is only possible from the \
                 Mother Claude desktop app."
            ),
        ));
    }
    if let Some(cwd) = defaults.cwd.clone().filter(|c| !c.trim().is_empty()) {
        // Trusted by the account its conversations run under (each keeps its
        // own trust).
        let account = defaults
            .account
            .clone()
            .filter(|a| !a.trim().is_empty())
            .unwrap_or_else(|| state.accounts.account_for_bridge(&cwd));
        let home = state
            .accounts
            .home(&account)
            .unwrap_or_else(|| state.home.clone());
        let trusted = tokio::task::spawn_blocking(move || {
            super::routes::threads::cwd_is_trusted(&home, &cwd)
        })
        .await
        .unwrap_or(false);
        if !trusted {
            if ctx.may_dangerous() {
                ctx.note_dangerous("bridge defaults with an untrusted folder");
            } else {
                return Err(BridgeError::forbidden(
                    "untrusted_cwd",
                    "That folder has never been opened in Claude Code on this Mac, so it can \
                     only be the bridge's default from the desktop app.",
                )
                .with("field", "cwd"));
            }
        }
    }
    Ok(())
}

/// `POST /api/bridge/start` — the main server. Defaults with a permission
/// mode that skips approvals are desktop-only (design G1). So is opening the
/// port to tokenless callers through tunnels: it publishes the API to whoever
/// finds the URL.
pub async fn post_start(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    super::routes::Body(body): super::routes::Body<StartBody>,
) -> BridgeResult<Response> {
    let main = state.bridge.main();
    let token = start_token(&state, &ctx, &main, body.open_to_tunnels)?;
    check_defaults(&state, &ctx, &body.defaults, "Starting the bridge").await?;
    if body.open_to_tunnels == Some(true) {
        ctx.note_dangerous("the API port opened without a token, tunnels included");
    }
    Ok(axum::Json(start_with(&state, body.defaults, token).await?).into_response())
}

/// `POST /api/bridge/stop`
pub async fn post_stop(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
) -> BridgeResult<Response> {
    Ok(axum::Json(stop(&state).await?).into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveThreadBody {
    /// `null` clears it, so the next message starts a fresh conversation.
    pub thread_id: Option<String>,
}

/// `POST /api/bridge/active-thread` — change which conversation the main
/// server's new messages join.
pub async fn post_active_thread(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    super::routes::Body(body): super::routes::Body<ActiveThreadBody>,
) -> BridgeResult<Response> {
    set_active(&state, &state.bridge.main(), body.thread_id).await?;
    Ok(axum::Json(status(&state).await).into_response())
}

/// A port a server may ask for through the dashboard: 1024–65535.
fn dashboard_port(port: Option<u16>) -> BridgeResult<()> {
    match port {
        Some(p) if p < servers::MIN_PORT => Err(BridgeError::invalid_field(
            "port",
            format!(
                "A server's port is between {} and 65535.",
                servers::MIN_PORT
            ),
        )),
        _ => Ok(()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddServerBody {
    pub name: String,
    /// Default: the first free port from 5613.
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub defaults: Option<BridgeDefaults>,
}

/// `POST /api/bridge/servers` → `201` its status (not started).
pub async fn post_server(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    super::routes::Body(body): super::routes::Body<AddServerBody>,
) -> BridgeResult<Response> {
    dashboard_port(body.port)?;
    let defaults = body.defaults.unwrap_or_default();
    check_defaults(&state, &ctx, &defaults, "Saving an API server's defaults").await?;
    let server = add_server(&state, &body.name, body.port, defaults).await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(server_status(&state, &server).await),
    )
        .into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchServerBody {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub defaults: Option<BridgeDefaults>,
}

/// `PATCH /api/bridge/servers/{id}` — a running server is restarted with the
/// change.
pub async fn patch_server(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    super::routes::Body(body): super::routes::Body<PatchServerBody>,
) -> BridgeResult<Response> {
    dashboard_port(body.port)?;
    if let Some(defaults) = &body.defaults {
        check_defaults(&state, &ctx, defaults, "Saving an API server's defaults").await?;
    }
    let server = update_server(&state, &id, body.name.as_deref(), body.port, body.defaults).await?;
    Ok(axum::Json(server_status(&state, &server).await).into_response())
}

/// `DELETE /api/bridge/servers/{id}` → `204`.
pub async fn delete_server(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> BridgeResult<Response> {
    remove_server(&state, &id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerStartBody {
    /// The defaults to start with (an added server saves them); absent: the
    /// ones it has.
    #[serde(default)]
    pub defaults: Option<BridgeDefaults>,
    /// As on `/api/bridge/start`; absent on an added server: closed to tunnels.
    #[serde(default)]
    pub open_to_tunnels: Option<bool>,
}

/// `POST /api/bridge/servers/{id}/start`
pub async fn post_server_start(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    super::routes::Body(body): super::routes::Body<ServerStartBody>,
) -> BridgeResult<Response> {
    let server = state
        .bridge
        .server(&id)
        .ok_or_else(|| servers::server_not_found(&id))?;
    let token = start_token(&state, &ctx, &server, body.open_to_tunnels)?;
    let check = body.defaults.clone().unwrap_or_else(|| server.saved());
    check_defaults(&state, &ctx, &check, "Starting an API server").await?;
    if body.open_to_tunnels == Some(true) {
        ctx.note_dangerous("an API port opened without a token, tunnels included");
    }
    let server = start_server(&state, &id, body.defaults, token).await?;
    Ok(axum::Json(server_status(&state, &server).await).into_response())
}

/// `POST /api/bridge/servers/{id}/stop`
pub async fn post_server_stop(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> BridgeResult<Response> {
    let server = stop_server(&state, &id).await?;
    Ok(axum::Json(server_status(&state, &server).await).into_response())
}

/// `POST /api/bridge/servers/{id}/active-thread`
pub async fn post_server_active_thread(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    super::routes::Body(body): super::routes::Body<ActiveThreadBody>,
) -> BridgeResult<Response> {
    let server = state
        .bridge
        .server(&id)
        .ok_or_else(|| servers::server_not_found(&id))?;
    set_active(&state, &server, body.thread_id).await?;
    Ok(axum::Json(server_status(&state, &server).await).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Goal 7: without an account the defaults read as before accounts, and
    /// a v0.4 body (no `account`) is still accepted.
    #[test]
    fn no_account_is_left_out() {
        let v = serde_json::to_value(BridgeDefaults::default()).unwrap();
        assert!(v.get("account").is_none(), "{v}");
        assert!(v.get("accountOnly").is_none(), "{v}");
        assert!(v.get("cwd").is_some(), "the others stay as they were");
        let parsed: BridgeDefaults =
            serde_json::from_value(serde_json::json!({ "cwd": null, "model": "opus" })).unwrap();
        assert!(parsed.account.is_none());
        assert!(!parsed.account_only);
        let with = BridgeDefaults {
            account: Some("acc_0123456789ab".into()),
            account_only: true,
            ..Default::default()
        };
        let v = serde_json::to_value(with).unwrap();
        assert_eq!(v["account"], "acc_0123456789ab");
        assert_eq!(v["accountOnly"], true);
    }

    /// The saved file is read by these names: every field, written.
    #[test]
    fn the_defaults_fields_are_the_written_ones() {
        let full = BridgeDefaults {
            cwd: Some("/".into()),
            model: Some("m".into()),
            effort: Some("low".into()),
            thinking: Some("on".into()),
            permission_mode: Some("plan".into()),
            setting_sources: Some(vec![]),
            system_prompt_append: Some("x".into()),
            default_thread: Some("t".into()),
            account: Some("a".into()),
            account_only: true,
        };
        let v = serde_json::to_value(full).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut fields = DEFAULTS_FIELDS.to_vec();
        fields.sort_unstable();
        assert_eq!(keys, fields);
    }

    #[test]
    fn blank_form_fields_mean_unset() {
        let defaults = BridgeDefaults {
            cwd: Some("   ".into()),
            model: Some(String::new()),
            effort: Some("high".into()),
            ..Default::default()
        }
        .normalized();
        assert!(defaults.cwd.is_none());
        assert!(defaults.model.is_none());
        assert_eq!(defaults.effort.as_deref(), Some("high"));
    }

    #[test]
    fn invalid_choices_are_named() {
        let bad = BridgeDefaults {
            effort: Some("turbo".into()),
            ..Default::default()
        };
        assert_eq!(
            bad.validate().unwrap_err().body()["error"]["field"],
            "effort"
        );

        let bad = BridgeDefaults {
            permission_mode: Some("whatever".into()),
            ..Default::default()
        };
        assert_eq!(
            bad.validate().unwrap_err().body()["error"]["field"],
            "permissionMode"
        );

        let bad = BridgeDefaults {
            setting_sources: Some(vec!["nope".into()]),
            ..Default::default()
        };
        assert_eq!(
            bad.validate().unwrap_err().body()["error"]["field"],
            "settingSources"
        );

        let bad = BridgeDefaults {
            cwd: Some("/definitely/not/here".into()),
            ..Default::default()
        };
        assert_eq!(bad.validate().unwrap_err().body()["error"]["field"], "cwd");

        // One account only, without an account, is no account at all.
        let bad = BridgeDefaults {
            account_only: true,
            ..Default::default()
        };
        assert_eq!(
            bad.validate().unwrap_err().body()["error"]["field"],
            "accountOnly"
        );
        assert!(bad.only_account().is_none());
        let only = BridgeDefaults {
            account: Some("acc_0123456789ab".into()),
            account_only: true,
            ..Default::default()
        };
        assert!(only.validate().is_ok());
        assert_eq!(only.only_account(), Some("acc_0123456789ab"));
        let default_for_new = BridgeDefaults {
            account_only: false,
            ..only
        };
        assert!(default_for_new.only_account().is_none());
    }

    #[test]
    fn a_plausible_configuration_validates() {
        let good = BridgeDefaults {
            cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
            model: Some("haiku".into()),
            effort: Some("low".into()),
            thinking: Some("off".into()),
            permission_mode: Some("acceptEdits".into()),
            setting_sources: Some(vec!["user".into(), "project".into()]),
            system_prompt_append: Some("Be terse.".into()),
            default_thread: None,
            account: None,
            account_only: false,
        };
        assert!(good.validate().is_ok());
    }

    #[test]
    fn the_start_body_rejects_typos() {
        assert!(
            serde_json::from_value::<StartBody>(json!({ "defaults": { "model": "haiku" } }))
                .is_ok()
        );
        assert!(serde_json::from_value::<StartBody>(json!({})).is_ok());
        let err = serde_json::from_value::<StartBody>(json!({ "defaults": { "modl": "haiku" } }))
            .unwrap_err();
        assert!(err.to_string().starts_with("unknown field"));
        for body in [
            json!({ "name": "Work", "prt": 5613 }),
            json!({ "name": "Work", "defaults": { "accountonly": true } }),
        ] {
            let err = serde_json::from_value::<AddServerBody>(body).unwrap_err();
            assert!(err.to_string().starts_with("unknown field"), "{err}");
        }
        assert!(
            serde_json::from_value::<ServerStartBody>(json!({ "openToTunnels": false })).is_ok()
        );
    }
}
