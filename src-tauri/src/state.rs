//! Shared application state and the single broadcast bus.
//!
//! One [`tokio::sync::broadcast`] sender fans every update out to the desktop
//! webview *and* all LAN/WebSocket clients — there is no separate path for
//! desktop vs mobile. The [`AppState`] handle is cloned into axum handlers and
//! the background monitor.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{broadcast, oneshot, RwLock};

use crate::claude::control::ControlRegistry;
use crate::claude::{
    ClaudeHome, PendingInput, Session, SessionState, Surface, TranscriptEvent, UsageSummary,
};
use crate::server::auth::Auth;

/// How a pending prompt was resolved by a human.
#[derive(Debug)]
pub enum Resolution {
    Allow,
    Deny,
    Answer(String),
}

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
    /// mandatory (enforced in the auth commit).
    pub fn is_non_loopback(&self) -> bool {
        !(self.host == "127.0.0.1" || self.host == "localhost" || self.host == "::1")
    }
}

/// Events broadcast to every connected client. Adjacently tagged so the Angular
/// client can switch on `kind` and read `data`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum ServerEvent {
    /// Full refreshed session list.
    Sessions(Vec<Session>),
    /// New transcript lines for one session (live deltas; history is via REST).
    #[serde(rename_all = "camelCase")]
    Transcript {
        session_id: String,
        events: Vec<TranscriptEvent>,
    },
    /// A raw hook event forwarded from a (possibly foreign) session.
    Hook(serde_json::Value),
    /// A pending input changed (set or cleared) for a session.
    #[serde(rename_all = "camelCase")]
    Pending {
        session_id: String,
        pending: Option<PendingInput>,
    },
    /// A human-readable notice.
    Notice(String),
}

/// The shared application state.
pub struct Inner {
    pub home: ClaudeHome,
    pub config: ServerConfig,
    pub auth: Auth,
    pub control: ControlRegistry,
    /// Experimental PTY-attach registry (Stage 3, feature-gated).
    #[cfg(feature = "experimental")]
    pub pty: crate::claude::experimental::PtyRegistry,
    pub bus: broadcast::Sender<ServerEvent>,
    /// TLS certificate fingerprint, set once the server binds (None for http).
    pub fingerprint: RwLock<Option<String>>,
    /// Session ids spawned by Mother Claude (full control / injection allowed).
    pub owned: RwLock<HashSet<String>>,
    /// Live pending prompts keyed by session id.
    pub pending: RwLock<HashMap<String, PendingInput>>,
    /// Oneshot resolvers keyed by request id; a blocked permission/question
    /// request awaits its sender. std Mutex — held only for insert/remove.
    pub resolvers: std::sync::Mutex<HashMap<String, oneshot::Sender<Resolution>>>,
    /// Last computed session list (served by REST without recomputation).
    pub sessions: RwLock<Vec<Session>>,
}

impl Inner {
    pub fn new(home: ClaudeHome, config: ServerConfig, auth: Auth) -> AppState {
        let (bus, _rx) = broadcast::channel(1024);
        Arc::new(Inner {
            home,
            config,
            auth,
            control: ControlRegistry::new(),
            #[cfg(feature = "experimental")]
            pty: crate::claude::experimental::PtyRegistry::new(),
            bus,
            fingerprint: RwLock::new(None),
            owned: RwLock::new(HashSet::new()),
            pending: RwLock::new(HashMap::new()),
            resolvers: std::sync::Mutex::new(HashMap::new()),
            sessions: RwLock::new(Vec::new()),
        })
    }

    /// Broadcast an event; ignores the "no receivers" error.
    pub fn broadcast(&self, event: ServerEvent) {
        let _ = self.bus.send(event);
    }

    pub async fn is_owned(&self, id: &str) -> bool {
        self.owned.read().await.contains(id)
    }

    /// Look up a session from the last computed snapshot.
    pub async fn find_session(&self, id: &str) -> Option<Session> {
        self.sessions
            .read()
            .await
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    /// Set or clear a session's pending prompt, updating the cached snapshot and
    /// broadcasting both a `pending` and a fresh `sessions` event so the UI
    /// reacts immediately (without waiting for the next monitor sweep).
    pub async fn set_pending(&self, id: &str, pending: Option<PendingInput>) {
        {
            let mut map = self.pending.write().await;
            match &pending {
                Some(p) => {
                    map.insert(id.to_string(), p.clone());
                }
                None => {
                    map.remove(id);
                }
            }
        }
        let snapshot = {
            let mut sessions = self.sessions.write().await;
            if let Some(s) = sessions.iter_mut().find(|s| s.id == id) {
                s.pending = pending.clone();
                if pending.is_some() {
                    s.state = SessionState::NeedsInput;
                }
            }
            sessions.clone()
        };
        self.broadcast(ServerEvent::Pending {
            session_id: id.to_string(),
            pending,
        });
        self.broadcast(ServerEvent::Sessions(snapshot));
    }

    /// Mark a session owned and reflect it in the cached snapshot immediately
    /// (flip an existing row, or insert a synthetic one for a brand-new id), then
    /// broadcast — so spawn/continue show up without waiting for the next sweep.
    pub async fn mark_owned(&self, id: &str, cwd: &str, started_at: i64) {
        self.owned.write().await.insert(id.to_string());
        let snapshot = {
            let mut sessions = self.sessions.write().await;
            match sessions.iter_mut().find(|s| s.id == id) {
                Some(s) => {
                    s.owned = true;
                    s.can_inject = true;
                    s.running = true;
                }
                None => {
                    let project_name = std::path::Path::new(cwd)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(cwd)
                        .to_string();
                    sessions.push(Session {
                        id: id.to_string(),
                        cwd: cwd.to_string(),
                        project_name,
                        surface: Surface::Unknown,
                        owned: true,
                        state: SessionState::Working,
                        model: None,
                        title: None,
                        started_at: Some(started_at),
                        last_activity: Some(started_at),
                        pid: None,
                        kind: None,
                        git_branch: None,
                        running: true,
                        message_count: 0,
                        usage: UsageSummary::default(),
                        pending: None,
                        can_inject: true,
                    });
                }
            }
            sessions.clone()
        };
        self.broadcast(ServerEvent::Sessions(snapshot));
    }

    pub fn register_resolver(&self, request_id: String, tx: oneshot::Sender<Resolution>) {
        if let Ok(mut r) = self.resolvers.lock() {
            r.insert(request_id, tx);
        }
    }

    pub fn take_resolver(&self, request_id: &str) -> Option<oneshot::Sender<Resolution>> {
        self.resolvers
            .lock()
            .ok()
            .and_then(|mut r| r.remove(request_id))
    }
}
