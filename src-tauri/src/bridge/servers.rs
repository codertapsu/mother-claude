//! API servers (docs/ACCOUNTS.md §13.5).
//!
//! The **main** server (`main`) is the one there always was: the dedicated
//! loopback port (`MOTHER_CLAUDE_BRIDGE_PORT`, default 5612) and the `/v1`
//! mount on the main app port, its defaults chosen at each start and never
//! saved. Others are added on the API screen and saved in
//! `<app data dir>/api-servers.json` (0600) as `{id, name, port, defaults}`.
//! Each has its own loopback port, defaults (`accountOnly` among them), the
//! conversation unaddressed messages join, token policy and started gate.
//! None listens until it is started; whether a start opens it to tunnels is
//! chosen at that start and never saved.
//!
//! Every server is also reachable through the main ones — `/servers/<id>/`
//! and `/servers/<id>/v1/` on the main API port, `/servers/<id>/v1/` on the
//! main app port (a token always) — with the target's rules: a [`Mount`]
//! names the server and the port a request arrived on, and the request's
//! [`MountPolicy`] is resolved from it.
//!
//! A server with `accountOnly` runs only as its account: [`MountPolicy::only_account`]
//! and the helpers below hide every conversation, operation and runtime
//! request of another account (`404`, as if it did not exist) and refuse a
//! request that names another account (`403 account_not_served`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, RwLock};

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use serde_json::{json, Map, Value};

use super::control::{BridgeDefaults, Listener, DEFAULTS_FIELDS};
use super::error::{BridgeError, BridgeResult};
use super::TokenPolicy;
use crate::state::AppState;

/// The id of the server there always was.
pub const MAIN_ID: &str = "main";
/// Its name.
pub const MAIN_NAME: &str = "Main";
/// The added servers, in the app data dir (0600).
pub const FILE_NAME: &str = "api-servers.json";
/// A server's name, in characters.
pub const NAME_MAX: usize = 40;
/// A server's id: a slug of its name.
pub const ID_MAX: usize = 32;
/// Where an added server's port is looked for when none is asked for.
pub const FIRST_PORT: u16 = 5613;
/// The lowest port an added server may ask for.
pub const MIN_PORT: u16 = 1024;

fn get<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set<T>(lock: &RwLock<T>, value: T) {
    *lock.write().unwrap_or_else(|e| e.into_inner()) = value;
}

/// One API server: what is saved about it and its run.
pub struct ApiServer {
    pub id: String,
    name: RwLock<String>,
    /// The loopback port it listens on: `None` for a main server without a
    /// dedicated port (`MOTHER_CLAUDE_BRIDGE_PORT=0`); `0` asks the system
    /// for one at each start (tests and embedders, never the API screen).
    port: RwLock<Option<u16>>,
    /// Whether it serves (a mirror of `listener`, for sync readers).
    running: AtomicBool,
    /// The port bound while it runs (`0`: none).
    bound: AtomicU16,
    /// The bound listener; `None`: not serving, and the routes that drive
    /// Claude answer `503 bridge_not_started`.
    pub(crate) listener: tokio::sync::Mutex<Option<Listener>>,
    /// What a start without defaults uses: an added server's saved
    /// defaults, the main server's last start's (never written anywhere).
    saved: RwLock<BridgeDefaults>,
    /// The defaults in force: the last start's, with the conversation
    /// unaddressed messages join kept current.
    defaults: RwLock<BridgeDefaults>,
    /// The conversation an unaddressed message joins.
    active_thread: RwLock<Option<String>>,
    /// The token policy of its current (or last) run.
    token: RwLock<TokenPolicy>,
    /// Saved fields this version does not know, written back unchanged: the
    /// entry's own, and its defaults'.
    extra: RwLock<Map<String, Value>>,
    defaults_extra: RwLock<Map<String, Value>>,
}

impl std::fmt::Debug for ApiServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiServer")
            .field("id", &self.id)
            .field("port", &self.port())
            .field("running", &self.is_running())
            .finish_non_exhaustive()
    }
}

impl ApiServer {
    pub(crate) fn new(
        id: &str,
        name: &str,
        port: Option<u16>,
        defaults: BridgeDefaults,
        token: TokenPolicy,
    ) -> Self {
        Self {
            id: id.to_string(),
            name: RwLock::new(name.to_string()),
            port: RwLock::new(port),
            running: AtomicBool::new(false),
            bound: AtomicU16::new(0),
            listener: tokio::sync::Mutex::new(None),
            saved: RwLock::new(defaults.clone()),
            defaults: RwLock::new(defaults.clone()),
            active_thread: RwLock::new(defaults.default_thread),
            token: RwLock::new(token),
            extra: RwLock::new(Map::new()),
            defaults_extra: RwLock::new(Map::new()),
        }
    }

    pub fn is_main(&self) -> bool {
        self.id == MAIN_ID
    }

    pub fn name(&self) -> String {
        get(&self.name)
    }

    pub(crate) fn set_name(&self, name: String) {
        set(&self.name, name);
    }

    /// The port it is configured to listen on.
    pub fn port(&self) -> Option<u16> {
        get(&self.port)
    }

    pub(crate) fn set_port(&self, port: Option<u16>) {
        set(&self.port, port);
    }

    /// The port it is bound to now (`None`: stopped, or the main server
    /// serving only the `/v1` mount).
    pub fn bound_port(&self) -> Option<u16> {
        Some(self.bound.load(Ordering::SeqCst)).filter(|p| *p != 0)
    }

    /// The port it listens on, or would: the bound one while it runs.
    pub fn current_port(&self) -> Option<u16> {
        self.bound_port()
            .or_else(|| self.port().filter(|p| *p != 0))
    }

    /// Every port a tunnel could point at to reach it: the configured one
    /// and the bound one.
    pub fn ports(&self) -> Vec<u16> {
        let mut out: Vec<u16> = self.port().into_iter().filter(|p| *p != 0).collect();
        if let Some(p) = self.bound_port() {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    /// Whether it serves. Everything that drives Claude through it checks this.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Called with `listener` locked, whenever it changes.
    pub(crate) fn note_listener(&self, running: bool, bound: Option<u16>) {
        self.bound.store(bound.unwrap_or(0), Ordering::SeqCst);
        self.running.store(running, Ordering::SeqCst);
    }

    /// The defaults in force.
    pub fn defaults(&self) -> BridgeDefaults {
        get(&self.defaults)
    }

    pub fn update_defaults<F: FnOnce(&mut BridgeDefaults)>(&self, f: F) {
        f(&mut self.defaults.write().unwrap_or_else(|e| e.into_inner()));
    }

    /// What a start without defaults uses.
    pub fn saved(&self) -> BridgeDefaults {
        get(&self.saved)
    }

    pub fn update_saved<F: FnOnce(&mut BridgeDefaults)>(&self, f: F) {
        f(&mut self.saved.write().unwrap_or_else(|e| e.into_inner()));
    }

    /// Take `defaults` as the ones in force and the ones a start without
    /// defaults uses; the conversation unaddressed messages join is theirs.
    pub(crate) fn apply_defaults(&self, defaults: BridgeDefaults) {
        set(&self.active_thread, defaults.default_thread.clone());
        set(&self.saved, defaults.clone());
        set(&self.defaults, defaults);
    }

    /// The conversation an unaddressed message joins.
    pub fn active_thread(&self) -> Option<String> {
        get(&self.active_thread)
    }

    /// Change the conversation unaddressed messages join (for this run: an
    /// added server's saved defaults keep what was chosen at its start).
    pub fn set_active_thread(&self, thread_id: Option<String>) {
        set(&self.active_thread, thread_id.clone());
        self.update_defaults(|d| d.default_thread = thread_id);
    }

    /// Stop pointing at a conversation that is gone.
    pub(crate) fn forget_active(&self, thread_id: &str) {
        let mut active = self
            .active_thread
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if active.as_deref() == Some(thread_id) {
            *active = None;
        }
    }

    /// The token policy of its current (or last) run.
    pub fn token_policy(&self) -> TokenPolicy {
        get(&self.token)
    }

    pub(crate) fn set_token_policy(&self, token: TokenPolicy) {
        set(&self.token, token);
    }

    /// Where it is reached through the main ones: `""` for the main server,
    /// `/servers/<id>` for the others (the main API port serves it there and
    /// under `/v1`; the main app port only under `/v1`).
    pub fn prefix(&self) -> String {
        if self.is_main() {
            String::new()
        } else {
            format!("/servers/{}", self.id)
        }
    }

    /// The one account it runs as, when its defaults say `accountOnly`.
    pub fn only_account(&self) -> Option<String> {
        self.defaults().only_account().map(str::to_string)
    }

    /// The entry `api-servers.json` keeps for it.
    fn to_saved(&self) -> Value {
        let mut defaults = serde_json::to_value(self.saved()).unwrap_or_else(|_| json!({}));
        if let Some(obj) = defaults.as_object_mut() {
            for (k, v) in get(&self.defaults_extra) {
                obj.entry(k).or_insert(v);
            }
        }
        let mut entry = get(&self.extra);
        entry.insert("id".into(), json!(self.id));
        entry.insert("name".into(), json!(self.name()));
        entry.insert("port".into(), json!(self.port().unwrap_or(0)));
        entry.insert("defaults".into(), defaults);
        Value::Object(entry)
    }
}

// ─────────────────────────────── validation ───────────────────────────────

/// A server id: 1–32 of `[a-z0-9-]`.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The id a name suggests: lower case, every run of other characters one
/// `-`, at most 32 characters; `server` when nothing is left.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().map(|c| c.to_ascii_lowercase()) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(ID_MAX);
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "server".to_string()
    } else {
        out
    }
}

/// `base`, or `base-2`, `base-3`… — the first one `taken` does not know and
/// that is not `main`.
pub fn unique_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if base != MAIN_ID && !taken(base) {
        return base.to_string();
    }
    for n in 2u32.. {
        let suffix = format!("-{n}");
        let keep = ID_MAX.saturating_sub(suffix.len()).min(base.len());
        let candidate = format!("{}{suffix}", base[..keep].trim_end_matches('-'));
        if candidate != MAIN_ID && !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!("an id is always found")
}

/// Trim and check a name: 1–40 characters, no control characters.
pub fn clean_name(name: &str) -> BridgeResult<String> {
    let name = name.trim();
    let n = name.chars().count();
    if n == 0 || n > NAME_MAX || name.chars().any(char::is_control) {
        return Err(BridgeError::invalid_field(
            "name",
            format!("A server's name is 1–{NAME_MAX} characters, with no control characters."),
        ));
    }
    Ok(name.to_string())
}

/// `409 port_unavailable`, naming the port.
fn port_taken(port: u16, why: String) -> BridgeError {
    BridgeError::conflict("port_unavailable", why)
        .with("field", "port")
        .with("port", port)
}

/// Whether `port` may be the port of the added server `id` (`None`: a new
/// one): 1024–65535 (or `0`: any free port at each start, which only
/// callers in this crate's tests and embedders ask for), and not the port of
/// another server, of the main API port, of the app or of the tunnel
/// listener.
pub fn check_port(state: &AppState, id: Option<&str>, port: u16) -> BridgeResult<()> {
    if port == 0 {
        return Ok(());
    }
    if port < MIN_PORT {
        return Err(BridgeError::invalid_field(
            "port",
            format!("A server's port is between {MIN_PORT} and 65535."),
        ));
    }
    if port == state.config.port {
        return Err(port_taken(
            port,
            format!("Port {port} is the Mother Claude app's own port."),
        ));
    }
    if port == state.remote.tunnel_port() {
        return Err(port_taken(
            port,
            format!("Port {port} is the Internet-access (tunnel) port."),
        ));
    }
    for server in state.bridge.servers.all() {
        if Some(server.id.as_str()) == id {
            continue;
        }
        if server.port() == Some(port) || server.bound_port() == Some(port) {
            let what = if server.is_main() {
                format!("Port {port} is the main API port.")
            } else {
                format!("Port {port} is the API server \"{}\"'s.", server.name())
            };
            let err = port_taken(port, what);
            return Err(if server.is_main() {
                err
            } else {
                err.with("server", server.id.clone())
            });
        }
    }
    Ok(())
}

/// The first port from 5613 no server has and nothing holds right now.
pub fn first_free_port(state: &AppState) -> BridgeResult<u16> {
    (FIRST_PORT..=u16::MAX)
        .filter(|p| check_port(state, None, *p).is_ok())
        .find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
        .ok_or_else(|| {
            BridgeError::conflict(
                "port_unavailable",
                "No port from 5613 up is free; name one.",
            )
            .with("field", "port")
        })
}

/// `404 server_not_found`.
pub fn server_not_found(id: &str) -> BridgeError {
    BridgeError::not_found("server_not_found", format!("No API server {id:?}."))
        .with("server", id.to_string())
}

// ─────────────────────────────── persistence ──────────────────────────────

/// Parse saved defaults: the fields this version knows (a value it cannot
/// read drops the defaults, with a warning), the others kept aside.
fn read_defaults(id: &str, value: Option<&Value>) -> (BridgeDefaults, Map<String, Value>) {
    let Some(obj) = value.and_then(Value::as_object) else {
        return (BridgeDefaults::default(), Map::new());
    };
    let (known, extra): (Map<String, Value>, Map<String, Value>) = obj
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .partition(|(k, _)| DEFAULTS_FIELDS.contains(&k.as_str()));
    let defaults = match serde_json::from_value::<BridgeDefaults>(Value::Object(known)) {
        Ok(d) => d.normalized(),
        Err(e) => {
            tracing::warn!(server = %id, error = %e, "ignoring an API server's unreadable defaults");
            BridgeDefaults::default()
        }
    };
    (defaults, extra)
}

/// Read `api-servers.json`: every entry with a valid id (not `main`), a
/// valid name and a port, each id, name and port once. Anything else is
/// skipped with a warning — a damaged file never stops the app.
pub(crate) fn load(path: &Path) -> Vec<ApiServer> {
    let Some(file) = crate::persist::read_json::<Value>(path) else {
        return Vec::new();
    };
    let entries = file
        .get("servers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<ApiServer> = Vec::new();
    for entry in entries {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let id = obj.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .map(clean_name)
            .and_then(Result::ok);
        let port = obj
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .filter(|p| *p == 0 || *p >= MIN_PORT);
        let (Some(name), Some(port)) = (name, port) else {
            tracing::warn!(server = %id, "skipping an API server without a usable name or port");
            continue;
        };
        let duplicate = out.iter().any(|s| {
            s.id == id
                || s.name().eq_ignore_ascii_case(&name)
                || (port != 0 && s.port() == Some(port))
        });
        if !is_valid_id(id) || id == MAIN_ID || duplicate {
            tracing::warn!(server = %id, "skipping an API server with an unusable or repeated id, name or port");
            continue;
        }
        let (defaults, defaults_extra) = read_defaults(id, obj.get("defaults"));
        let server = ApiServer::new(id, &name, Some(port), defaults, TokenPolicy::Required);
        let extra: Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| !["id", "name", "port", "defaults"].contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        set(&server.extra, extra);
        set(&server.defaults_extra, defaults_extra);
        out.push(server);
    }
    out
}

/// Write the added servers (never the main one) to `path`.
pub(crate) fn save(path: &Path, servers: &[Arc<ApiServer>]) -> std::io::Result<()> {
    let entries: Vec<Value> = servers
        .iter()
        .filter(|s| !s.is_main())
        .map(|s| s.to_saved())
        .collect();
    crate::persist::write_json_private(path, &json!({ "version": 1, "servers": entries }))
}

/// The registry of servers, main first. Lives in [`super::BridgeState`].
#[derive(Debug)]
pub struct Servers {
    list: RwLock<Vec<Arc<ApiServer>>>,
    path: Option<PathBuf>,
    /// Serializes changes to the servers — adding, changing, removing,
    /// starting and stopping — so a port or a name is checked and taken in
    /// one step.
    pub(crate) admin: tokio::sync::Mutex<()>,
}

impl Servers {
    /// The main server, plus the saved ones from `data_dir` (none without one).
    pub(crate) fn load(main: ApiServer, data_dir: Option<&Path>, token: TokenPolicy) -> Self {
        let path = data_dir.map(|d| d.join(FILE_NAME));
        let mut list = vec![Arc::new(main)];
        if let Some(path) = &path {
            for server in load(path) {
                // An added server starts closed to tunnels unless its start
                // says otherwise ("open to tunnels" is never saved).
                server.set_token_policy(token.closed());
                list.push(Arc::new(server));
            }
        }
        Self {
            list: RwLock::new(list),
            path,
            admin: tokio::sync::Mutex::new(()),
        }
    }

    pub fn all(&self) -> Vec<Arc<ApiServer>> {
        get(&self.list)
    }

    pub fn main(&self) -> Arc<ApiServer> {
        self.list.read().unwrap_or_else(|e| e.into_inner())[0].clone()
    }

    pub fn get(&self, id: &str) -> Option<Arc<ApiServer>> {
        self.list
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    /// Whether a server besides the main one exists.
    pub fn has_added(&self) -> bool {
        self.list.read().unwrap_or_else(|e| e.into_inner()).len() > 1
    }

    /// Write the added servers; a failed save is the caller's error.
    pub(crate) fn save(&self) -> BridgeResult<()> {
        self.save_list(&self.all())
    }

    fn save_list(&self, list: &[Arc<ApiServer>]) -> BridgeResult<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        save(path, list).map_err(|e| {
            BridgeError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "servers_not_saved",
                format!("Could not save the API servers: {e}"),
            )
        })
    }

    /// Add a server, saved before it is listed.
    pub(crate) fn insert(&self, server: ApiServer) -> BridgeResult<Arc<ApiServer>> {
        let server = Arc::new(server);
        let mut next = self.all();
        next.push(server.clone());
        self.save_list(&next)?;
        set(&self.list, next);
        Ok(server)
    }

    /// Remove an added server, saved before it is gone.
    pub(crate) fn remove(&self, id: &str) -> BridgeResult<()> {
        let next: Vec<Arc<ApiServer>> = self
            .all()
            .into_iter()
            .filter(|s| s.id != id || s.is_main())
            .collect();
        self.save_list(&next)?;
        set(&self.list, next);
        Ok(())
    }
}

// ───────────────────────────── mounts and scope ─────────────────────────────

/// How one entry point reaches the bridge's routes: which server, under
/// which token policy, arriving on which port.
#[derive(Debug, Clone)]
pub struct Mount {
    pub server: String,
    /// The mount's own token policy — the main app port's mounts always
    /// `Required` — or `None`: the server's own run's.
    pub token: Option<TokenPolicy>,
    /// The API port the request arrived on, whose tunnels decide the
    /// tokenless mode; `None` on the main app port (a token always), and on
    /// routers built without a port (tests), where a tunnel to any API port
    /// counts.
    pub port: Option<u16>,
}

impl Mount {
    pub fn new(server: &str, token: Option<TokenPolicy>, port: Option<u16>) -> Self {
        Self {
            server: server.to_string(),
            token,
            port,
        }
    }

    /// What a router built without a mount gets: the main server, a token
    /// always (fail closed).
    pub fn fail_closed() -> Self {
        Self::new(MAIN_ID, Some(TokenPolicy::Required), None)
    }
}

/// A request's server and the rules it gets, resolved from its [`Mount`]
/// by the bridge's router. `/capabilities` and the OpenAPI document describe
/// *this* mount — the loopback port and the LAN mount have different
/// authentication policies, and advertising the wrong one sends clients to
/// debug a token they do not need (or omit one they do).
#[derive(Debug, Clone)]
pub struct MountPolicy {
    pub server: Arc<ApiServer>,
    pub token: TokenPolicy,
    /// The API port the request arrived on (see [`Mount::port`]).
    pub port: Option<u16>,
    /// A detected tunnel points at that port (or, without one, at any API
    /// port).
    pub exposed: bool,
    /// The server's defaults when the request arrived.
    pub defaults: BridgeDefaults,
}

impl MountPolicy {
    /// Every API route on this mount needs a token.
    pub fn require_token(&self) -> bool {
        self.token.requires_token()
    }

    pub fn server_id(&self) -> &str {
        &self.server.id
    }

    /// The one account this server runs as, when it is `accountOnly`.
    pub fn only_account(&self) -> Option<&str> {
        self.defaults.only_account()
    }

    /// `403 account_not_served` when a request names an account this
    /// server does not run as.
    pub fn check_asked(&self, asked: Option<&str>) -> BridgeResult<()> {
        let asked = asked.map(str::trim).filter(|a| !a.is_empty());
        match (self.only_account(), asked) {
            (Some(only), Some(asked)) if only != asked => Err(BridgeError::forbidden(
                "account_not_served",
                format!(
                    "This API server runs only as its own account; it does not serve {asked:?}."
                ),
            )
            .with("account", asked.to_string())
            .with("server", self.server.id.clone())),
            _ => Ok(()),
        }
    }

    /// Whether this server shows the conversation `thread_id` — always,
    /// unless it is `accountOnly` and the conversation is another account's.
    pub fn serves(&self, state: &AppState, thread_id: &str) -> bool {
        match self.only_account() {
            None => true,
            Some(only) => thread_account(state, thread_id) == only,
        }
    }

    /// `404 thread_not_found` for a conversation this server does not show.
    pub fn require_thread(&self, state: &AppState, thread_id: &str) -> BridgeResult<()> {
        if self.serves(state, thread_id) {
            Ok(())
        } else {
            Err(BridgeError::thread_not_found(thread_id))
        }
    }
}

/// The account a conversation runs (or ran) under, a token no account holds
/// counting as the Default's ([`crate::claude::summaries::acting_account`]).
pub fn thread_account(state: &AppState, thread_id: &str) -> String {
    crate::claude::summaries::acting_account(state, thread_id)
}

impl<S: Send + Sync> FromRequestParts<S> for MountPolicy {
    type Rejection = BridgeError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<MountPolicy>()
            .cloned()
            .ok_or_else(|| BridgeError::internal("This route was mounted without its API server."))
    }
}

/// The server object `/metadata` and `/capabilities` carry, or `None` while
/// only the Default account and only the main server exist (goal 7).
pub fn server_view(state: &AppState, mount: &MountPolicy) -> Option<Value> {
    if state.accounts.is_default_only() && !state.bridge.servers.has_added() {
        return None;
    }
    let account = mount
        .defaults
        .account
        .as_deref()
        .and_then(|id| state.accounts.get(id))
        .map(|a| json!({ "id": a.id, "name": a.name }));
    let mut view = json!({
        "id": mount.server.id,
        "name": mount.server.name(),
        "accountOnly": mount.only_account().is_some(),
    });
    if let Some(account) = account {
        view["account"] = account;
    }
    Some(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_slugs_of_names_made_unique() {
        assert_eq!(slug("Work"), "work");
        assert_eq!(slug("  My  API (personal) "), "my-api-personal");
        assert_eq!(slug("Ünïcode only"), "n-code-only");
        assert_eq!(slug("日本"), "server");
        assert_eq!(slug("---"), "server");
        assert_eq!(slug(&"a".repeat(50)).len(), ID_MAX);
        assert!(is_valid_id(&slug("Anything at all, 2026!")));

        let taken = ["work", "work-2"];
        assert_eq!(unique_id("work", |id| taken.contains(&id)), "work-3");
        assert_eq!(unique_id("main", |_| false), "main-2");
        assert_eq!(unique_id("solo", |_| false), "solo");
        let long = "a".repeat(ID_MAX);
        let id = unique_id(&long, |id| id == long);
        assert_eq!(id.len(), ID_MAX);
        assert!(id.ends_with("-2"), "{id}");

        assert!(is_valid_id("a-1"));
        for bad in ["", "Main!", "a/b", "UPPER", &"x".repeat(ID_MAX + 1)] {
            assert!(!is_valid_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(clean_name("  Work ").unwrap(), "Work");
        for bad in ["", "   ", "a\u{7}b", &"x".repeat(NAME_MAX + 1)] {
            let err = clean_name(bad).unwrap_err();
            assert_eq!(err.code, "invalid_field");
            assert_eq!(err.body()["error"]["field"], "name");
        }
        assert!(clean_name(&"x".repeat(NAME_MAX)).is_ok());
    }

    /// A damaged file never stops the app: what can be read is kept, what
    /// this version does not know is written back unchanged.
    #[test]
    fn the_file_is_read_tolerantly_and_round_trips_what_it_does_not_know() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(
            &path,
            json!({ "version": 1, "servers": [
                { "id": "work", "name": "Work", "port": 5613, "later": true,
                  "defaults": { "model": "haiku", "account": "acc_0123456789ab",
                                "accountOnly": true, "futureKnob": 3 } },
                { "id": "main", "name": "Main again", "port": 5614 },
                { "id": "Bad Id", "name": "Bad", "port": 5615 },
                { "id": "low", "name": "Low", "port": 80 },
                { "id": "twin", "name": "work", "port": 5616 },
                { "id": "same-port", "name": "Same port", "port": 5613 },
                { "id": "odd", "name": "Odd", "port": 5617, "defaults": { "model": 5 } },
                "not an object"
            ]})
            .to_string(),
        )
        .unwrap();
        let loaded = load(&path);
        let ids: Vec<&str> = loaded.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["work", "odd"]);
        let work = &loaded[0];
        assert_eq!(work.name(), "Work");
        assert_eq!(work.port(), Some(5613));
        let d = work.saved();
        assert_eq!(d.model.as_deref(), Some("haiku"));
        assert_eq!(d.only_account(), Some("acc_0123456789ab"));
        assert_eq!(loaded[1].saved(), BridgeDefaults::default());

        let servers: Vec<Arc<ApiServer>> = loaded.into_iter().map(Arc::new).collect();
        save(&path, &servers).unwrap();
        let again: Value = crate::persist::read_json(&path).unwrap();
        assert_eq!(again["servers"][0]["later"], true);
        assert_eq!(again["servers"][0]["defaults"]["futureKnob"], 3);
        assert_eq!(again["servers"][0]["defaults"]["accountOnly"], true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        std::fs::write(&path, "{ not json").unwrap();
        assert!(load(&path).is_empty());
        assert!(load(&dir.path().join("missing.json")).is_empty());
    }
}
