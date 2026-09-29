//! Shared application state and the single broadcast bus.
//!
//! One [`tokio::sync::broadcast`] sender fans every update out to the desktop
//! webview *and* all LAN/tunnel WebSocket clients — there is no separate path
//! for desktop vs mobile. The WebSocket hub (`server::ws`) filters it per
//! client (subscriptions, desktop-only frames).
//!
//! **One field per subsystem, owned by that subsystem's file.** Agents extend
//! their own types (add fields inside `DeviceStore`, `HookHub`, …) instead of
//! adding fields here; see `interfaces.md` for who owns what.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};

use crate::bridge::{BridgeConfig, BridgeState};
use crate::claude::activity::ActivityTracker;
use crate::claude::agents::AgentTracker;
use crate::claude::control::ControlRegistry;
use crate::claude::hooks::PluginManager;
use crate::claude::index::TranscriptIndex;
use crate::claude::live::LiveRoster;
use crate::claude::model::{NoticeLevel, SessionSummary};
use crate::claude::tmux::TmuxState;
use crate::claude::uievents::Timeline;
use crate::claude::usage::UsageStore;
use crate::claude::{AuthStatus, ClaudeHome, Session};
use crate::pending::PendingStore;
use crate::server::auth::Auth;
use crate::server::devices::DeviceStore;
use crate::server::hooks::HookHub;
use crate::server::pairing::PairingState;
use crate::server::push::PushService;
use crate::server::ratelimit::AuthLimiter;
use crate::server::remote::RemoteState;
use crate::server::ws::{Hub, Presence, Snapshot};

pub use crate::server::ws::ServerEvent;

/// Cloneable shared state handle.
pub type AppState = Arc<Inner>;

/// Server bind configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl ServerConfig {
    /// Read from `MOTHER_CLAUDE_HOST` / `MOTHER_CLAUDE_PORT`, defaulting to
    /// `0.0.0.0:6725`.
    pub fn from_env() -> Self {
        let host = std::env::var("MOTHER_CLAUDE_HOST")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "0.0.0.0".to_string());
        let port = std::env::var("MOTHER_CLAUDE_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(6725);
        Self { host, port }
    }

    pub fn bind_addr(&self) -> Option<SocketAddr> {
        format!("{}:{}", self.host, self.port).parse().ok()
    }

    /// True when bound to something other than loopback — TLS + auth become
    /// mandatory.
    pub fn is_non_loopback(&self) -> bool {
        !(self.host == "127.0.0.1" || self.host == "localhost" || self.host == "::1")
    }
}

/// The shared application state.
pub struct Inner {
    pub home: ClaudeHome,
    pub config: ServerConfig,
    /// Credentials (master / hook / sidecar tokens). Foundation.
    pub auth: Auth,
    /// Owned-session children. B3 (`claude/control.rs`, `claude/owned.rs`).
    pub control: ControlRegistry,
    /// Experimental PTY-attach registry. B4.
    #[cfg(feature = "experimental")]
    pub pty: crate::claude::experimental::PtyRegistry,
    /// The single broadcast bus (events are `Arc`ed: one allocation, many
    /// receivers). Publish through `server::ws::publish_*`. Foundation.
    pub bus: broadcast::Sender<Arc<ServerEvent>>,
    /// TLS certificate fingerprint, set once the LAN listeners bind. B1.
    pub fingerprint: RwLock<Option<String>>,
    /// Session ids Mother Claude spawned/continued (and bridge threads). B3.
    pub owned: RwLock<HashSet<String>>,
    /// The session list every client sees + diff bookkeeping. Write only via
    /// `server::ws::{publish_sessions, upsert_session, patch_session}`.
    pub snapshot: RwLock<Snapshot>,
    /// Legacy registry rows from the last refresh. B2 (replaced by the new
    /// data layer).
    pub registry: RwLock<Vec<Session>>,
    /// Pending human decisions. Foundation (`pending.rs`).
    pub pending: PendingStore,
    /// Connected WebSocket clients. Foundation (`server/ws.rs`).
    pub hub: Hub,
    /// Per-device foreground state from `visible` frames. Foundation; read by
    /// B5.
    pub presence: Presence,
    /// The Claude HTTP bridge.
    pub bridge: BridgeState,
    /// Claude's own Anthropic sign-in, checked once at startup.
    pub claude_auth: RwLock<AuthStatus>,
    /// Failed-auth rate limiting. B1 (`server/ratelimit.rs`).
    pub limits: AuthLimiter,
    /// Paired devices. B1 (`server/devices.rs`).
    pub devices: DeviceStore,
    /// Pairing codes and approvals. B1 (`server/pairing.rs`).
    pub pairing: PairingState,
    /// Internet access / tunnels. B1 (`server/remote.rs`).
    pub remote: RemoteState,
    /// `~/.claude/sessions/*.json` roster. B2 (`claude/live.rs`).
    pub live: LiveRoster,
    /// Incremental transcript index. B2 (`claude/index.rs`).
    pub index: TranscriptIndex,
    /// Subagents / workflows / background tasks. B2 (`claude/agents.rs`).
    pub agents: AgentTracker,
    /// "What is it doing now" hints. B2 (`claude/activity.rs`).
    pub activity: ActivityTracker,
    /// Mother Claude's own timeline events (relayed messages). B2
    /// (`claude/uievents.rs`); B4 pushes into it.
    pub timeline: Timeline,
    /// Hook dispatcher state. B4 (`server/hooks.rs`).
    pub hooks: HookHub,
    /// Hooks plugin install state. B4 (`claude/hooks.rs`).
    pub plugin: PluginManager,
    /// tmux sockets/panes. B4 (`claude/tmux.rs`).
    pub tmux: TmuxState,
    /// Web Push. B5 (`server/push.rs`).
    pub push: PushService,
    /// The claude.ai plan's usage limits and their refresh scheduler
    /// (`claude/usage.rs`).
    pub usage: UsageStore,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("config", &self.config)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

impl Inner {
    pub fn new(home: ClaudeHome, config: ServerConfig, auth: Auth) -> AppState {
        Self::with_bridge_config(home, config, auth, BridgeConfig::from_env())
    }

    /// Build the state with an explicit bridge configuration.
    ///
    /// Tests use this to get a bridge that never binds a real port
    /// (`port: None`), which the environment-driven constructor cannot express
    /// without mutating process-global state that parallel tests would race on.
    /// With `Auth::ephemeral()` (no `config_dir`) nothing touches the disk.
    pub fn with_bridge_config(
        home: ClaudeHome,
        config: ServerConfig,
        auth: Auth,
        bridge: BridgeConfig,
    ) -> AppState {
        let (bus, _rx) = broadcast::channel(1024);
        let data_dir = auth.config_dir.clone();
        let data_dir = data_dir.as_deref();
        let live = LiveRoster::new();
        if let Some(dir) = data_dir {
            // The plan-usage helper's `claude -p` is never a session.
            live.ignore_cwd(&crate::claude::usage::helper_dir(dir));
        }
        Arc::new(Inner {
            home,
            config,
            control: ControlRegistry::new(),
            #[cfg(feature = "experimental")]
            pty: crate::claude::experimental::PtyRegistry::new(),
            bus,
            fingerprint: RwLock::new(None),
            owned: RwLock::new(HashSet::new()),
            snapshot: RwLock::new(Snapshot::default()),
            registry: RwLock::new(Vec::new()),
            pending: PendingStore::new(),
            hub: Hub::new(),
            presence: Presence::new(),
            bridge: BridgeState::new(bridge),
            claude_auth: RwLock::new(AuthStatus::default()),
            limits: AuthLimiter::new(),
            devices: DeviceStore::load(data_dir),
            pairing: PairingState::new(data_dir),
            remote: RemoteState::new(data_dir),
            live,
            index: TranscriptIndex::new(data_dir),
            agents: AgentTracker::new(),
            activity: ActivityTracker::new(),
            timeline: Timeline::new(),
            hooks: HookHub::load(data_dir),
            plugin: PluginManager::new(),
            tmux: TmuxState::new(),
            push: PushService::new(data_dir),
            usage: UsageStore::new(data_dir),
            auth,
        })
    }

    /// Broadcast an event; ignores the "no receivers" error.
    pub fn broadcast(&self, event: ServerEvent) {
        let _ = self.bus.send(Arc::new(event));
    }

    /// Broadcast an info notice (shorthand for `ws::publish_notice`).
    pub fn notice(&self, text: impl Into<String>) {
        let _ = self.bus.send(Arc::new(ServerEvent::Notice {
            level: NoticeLevel::Info,
            text: text.into(),
            session: None,
        }));
    }

    pub async fn is_owned(&self, id: &str) -> bool {
        self.owned.read().await.contains(id)
    }

    /// One session from the current snapshot.
    pub async fn find_session(&self, id: &str) -> Option<SessionSummary> {
        self.snapshot.read().await.find(id).cloned()
    }

    /// Every session in the current snapshot (any scope), most recent first.
    pub async fn sessions(&self) -> Vec<SessionSummary> {
        self.snapshot.read().await.sessions.clone()
    }

    /// The transcript file of a session: the index's answer (located by id),
    /// else derived from the session's cwd.
    pub async fn transcript_path(&self, id: &str) -> Option<PathBuf> {
        if let Some(p) = self.index.transcript_path(id) {
            return Some(p);
        }
        let cwd = match self.find_session(id).await {
            Some(s) => s.cwd,
            None => self
                .registry
                .read()
                .await
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.cwd.clone())
                .unwrap_or_default(),
        };
        (!cwd.is_empty()).then(|| self.home.transcript_path(&cwd, id))
    }

    /// Mark a session owned and reflect it in the snapshot immediately (flip
    /// an existing row, or insert a synthetic one for a brand-new id), so
    /// spawn/continue show up without waiting for the next refresh.
    pub async fn mark_owned(self: &Arc<Self>, id: &str, cwd: &str, started_at: i64) {
        self.owned.write().await.insert(id.to_string());
        // Sync mirror for `claude::owned::is_owned` (hook short-circuit).
        self.control.note_owned(id, true);
        // A bridge conversation (its record is inserted first) gets the
        // bridge's narrower caps.
        let bridge_thread = self.bridge.thread(id).await.is_some();
        let control = crate::claude::caps::compute(&crate::claude::caps::CapsInput {
            owned: !bridge_thread,
            bridge_thread,
            live: true,
            ..Default::default()
        });
        let caps = control.clone();
        let found = crate::server::ws::patch_session(self, id, move |s| {
            s.owned = true;
            s.live = true;
            s.surface = crate::claude::model::Surface::MotherClaude;
            s.control = caps;
        })
        .await;
        if !found {
            let mut summary = crate::claude::summaries::synthetic_owned(id, cwd, started_at);
            summary.control = control;
            crate::server::ws::upsert_session(self, summary).await;
        }
    }

    /// Forget a session we no longer control, so `owned` does not grow for the
    /// life of the process and a dead conversation stops claiming it can be
    /// driven. Its pending items are withdrawn.
    pub async fn unmark_owned(self: &Arc<Self>, id: &str) {
        self.owned.write().await.remove(id);
        self.control.note_owned(id, false);
        crate::pending::withdraw_session(self, id, "session no longer owned").await;
        crate::server::ws::patch_session(self, id, |s| {
            s.owned = false;
            s.live = false;
            s.status = crate::claude::model::SessionStatus::Ended;
            s.activity = crate::claude::summaries::activity_for(s.status, &s.pending);
            s.control = crate::claude::caps::compute(&crate::claude::caps::CapsInput::default());
        })
        .await;
    }

    /// Re-check Claude's sign-in and cache the answer.
    pub async fn refresh_claude_auth(&self) -> AuthStatus {
        let status = crate::claude::auth_status().await;
        *self.claude_auth.write().await = status.clone();
        status
    }
}
