//! WebSocket v2 hub (design §4.6).
//!
//! One socket per client, all fed from the single broadcast bus
//! ([`crate::state::Inner::bus`]). Frames are JSON objects tagged with `t`.
//!
//! Client → server: `auth{token}`, `sub{session, agent?, since?}`,
//! `unsub{session, agent?}`, `subFleet{}`, `unsubFleet{}`, `subInbox{}`,
//! `unsubInbox{}`, `ping{}`, `pong{}`, `visible{v}`.
//!
//! Server → client: `hello{v:2, me, serverTime}` after auth, then
//! `sessions{full:true, sessions, seq}` and `planUsage{usage}`, then
//! `sessions{full:false, upsert, remove, seq}` diffs (content-hash per
//! session, `active` scope only), `planUsage` whenever the plan usage clients
//! see changes, `transcript{session, agent?, events, tail}` /
//! `transcriptReset{session, agent?}` for that exact subscription, `stream` /
//! `agents` for subscribers of the session, `pending{item}` for session and
//! inbox subscribers, `pendingResolved{id, session, outcome, by?, at}` for
//! everyone, `fleet` for fleet subscribers, `notice`, desktop-only `devices` /
//! `pairRequest`, `resync` (then a fresh full `sessions` and `planUsage`),
//! `ping` every 25 s, `pong`.
//!
//! `sub{since}` replays the file after the cursor, capped at 2 MB / 300
//! events; beyond that the client gets `transcriptReset` and refetches the
//! last page. The hub counts subscriptions so the monitor only reads the
//! transcripts someone is watching.
//!
//! * Auth: the upgrade needs no token in the URL. A local client may still use
//!   `?token=` (resolved by the context middleware), anyone may send
//!   `Authorization: Bearer`; otherwise the first frame must be
//!   `{"t":"auth","token":…}` within 5 s. A token that is rejected closes
//!   the socket with 4401; no auth frame in time (or a first frame that is
//!   not one) closes it with 4408, so a client that was merely slow (a phone
//!   frozen right after the upgrade) retries instead of forgetting its token.
//! * Origin: the upgrade is refused unless `Origin` is absent, local, or the
//!   same host as `Host`.
//! * Backpressure: each client has a bounded queue (256 frames). On overflow
//!   (or bus lag) the queue is dropped and `resync` + a full `sessions` frame
//!   are sent instead.
//! * Pre-auth pools: a socket that has not authenticated yet waits in a pool
//!   per scope (this machine, the LAN, tunnels); a full pool closes its
//!   oldest waiting socket with 4408, so the client retries.
//! * Liveness: a client that sends nothing for 60 s is closed (4408).
//! * At most 8 sockets per device (the oldest is closed with 4409).
//! * `visible{v}` frames feed [`Presence`] (push skips foreground devices).
//!
//! Owner: foundation. Other agents publish with the `publish_*` helpers and
//! never touch sockets directly.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{watch, Notify};

use super::context::{origin_is_local, ClientContext};
use super::error::ApiError;
use crate::claude::model::{
    AgentInfo, FleetSession, FleetSummary, NoticeLevel, PendingItem, PendingPreview,
    PendingResolved, PlanUsage, SessionStatus, SessionSummary, StreamFrame, UiEvent, WorkflowRun,
};
use crate::state::AppState;

pub const PROTOCOL_VERSION: u32 = 2;
/// Frames buffered per client before it is told to resync.
pub const QUEUE_CAP: usize = 256;
pub const PING_EVERY: Duration = Duration::from_secs(25);
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(60);
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_SOCKETS_PER_DEVICE: usize = 8;

/// Close codes.
pub const CLOSE_UNAUTHORIZED: u16 = 4401;
pub const CLOSE_TIMEOUT: u16 = 4408;
pub const CLOSE_REPLACED: u16 = 4409;

/// Every event on the bus. Serialized as the wire frame (`t` tag).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServerEvent {
    /// Full snapshot (`full: true`, `sessions`) or diff (`upsert`, `remove`).
    Sessions {
        full: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        sessions: Option<Vec<SessionSummary>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        upsert: Option<Vec<SessionSummary>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        remove: Option<Vec<String>>,
        seq: u64,
    },
    /// Transcript appends for one session, or one of its agents (that exact
    /// subscription only).
    Transcript {
        session: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        events: Vec<UiEvent>,
        tail: String,
    },
    /// The subscriber's cursor is too far behind for a bounded replay:
    /// refetch the last page.
    TranscriptReset {
        session: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
    },
    /// A full pending item appeared (session and inbox subscribers).
    Pending { item: PendingItem },
    /// Owned live deltas (subscribers only).
    Stream {
        session: String,
        #[serde(flatten)]
        frame: StreamFrame,
    },
    /// A session's agents changed (subscribers only).
    Agents {
        session: String,
        agents: Vec<AgentInfo>,
        workflows: Vec<WorkflowRun>,
    },
    /// Running agents across sessions (fleet subscribers only).
    Fleet {
        sessions: Vec<FleetSession>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<FleetSummary>,
    },
    /// Human-readable notice (everyone).
    Notice {
        level: NoticeLevel,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session: Option<String>,
    },
    /// A notice only the desktop sees (pairings, a device's dangerous
    /// actions, remote-access changes): the same `notice` frame on the wire.
    #[serde(rename = "notice")]
    DesktopNotice {
        level: NoticeLevel,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session: Option<String>,
    },
    /// Devices or pending pairings changed — refetch (desktop only).
    Devices,
    /// A pairing waits for approval (desktop only).
    PairRequest { pending: Value },
    /// Everyone: refetch and resubscribe.
    Resync,
    /// A pending item was answered or withdrawn (everyone). Produced by the
    /// pending store (B3); `{t:"pendingResolved", id, session, outcome, by?,
    /// at, text?}`.
    PendingResolved(crate::claude::model::PendingResolved),
    /// The claude.ai plan's usage changed (everyone; also sent right after
    /// the full `sessions` frame): `{t:"planUsage", usage}`.
    PlanUsage { usage: PlanUsage },
    /// The accounts changed (everyone; sent on connect once there is more
    /// than the Default): `{t:"accounts", accounts}` (docs/ACCOUNTS.md §4.4).
    Accounts {
        accounts: crate::claude::accounts::view::AccountsView,
    },
    /// Another account's plan usage changed (everyone; sent on connect for
    /// every account but the Default): `{t:"accountUsage", accountId, usage}`.
    /// The Default's stays `planUsage`, which older clients know.
    AccountUsage {
        account_id: String,
        usage: PlanUsage,
    },
}

/// Who receives an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience<'a> {
    All,
    Desktop,
    /// Anyone subscribed to the session (main or any agent transcript).
    Session(&'a str),
    /// That exact `(session, agent?)` subscription.
    SessionKey(&'a str, Option<&'a str>),
    /// Session subscribers plus inbox subscribers.
    SessionOrInbox(&'a str),
    Fleet,
}

impl ServerEvent {
    pub fn audience(&self) -> Audience<'_> {
        match self {
            ServerEvent::Sessions { .. }
            | ServerEvent::Notice { .. }
            | ServerEvent::Resync
            | ServerEvent::PendingResolved(_)
            | ServerEvent::PlanUsage { .. }
            | ServerEvent::Accounts { .. }
            | ServerEvent::AccountUsage { .. } => Audience::All,
            ServerEvent::Transcript { session, agent, .. }
            | ServerEvent::TranscriptReset { session, agent } => {
                Audience::SessionKey(session, agent.as_deref())
            }
            ServerEvent::Stream { session, .. } | ServerEvent::Agents { session, .. } => {
                Audience::Session(session)
            }
            ServerEvent::Pending { item } => Audience::SessionOrInbox(&item.session_id),
            ServerEvent::Fleet { .. } => Audience::Fleet,
            ServerEvent::Devices
            | ServerEvent::PairRequest { .. }
            | ServerEvent::DesktopNotice { .. } => Audience::Desktop,
        }
    }
}

// ───────────────────────────────── snapshot ───────────────────────────────────

/// The session list every client sees, plus the diff bookkeeping.
#[derive(Debug, Default)]
pub struct Snapshot {
    /// All known sessions (any scope), most recent first.
    pub sessions: Vec<SessionSummary>,
    /// Sequence number of the last broadcast `sessions` frame.
    pub seq: u64,
    /// Content hash per in-scope session id, as last broadcast.
    hashes: HashMap<String, u64>,
    /// Pending item ids already sent as `pending{item}` frames.
    announced: HashSet<String>,
}

impl Snapshot {
    pub fn find(&self, id: &str) -> Option<&SessionSummary> {
        self.sessions.iter().find(|s| s.id == id)
    }

    /// Sessions in the `active` scope, in list order.
    pub fn active(&self, now_ms: i64) -> Vec<SessionSummary> {
        self.sessions
            .iter()
            .filter(|s| s.in_active_scope(now_ms))
            .cloned()
            .collect()
    }
}

fn content_hash(s: &SessionSummary) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match serde_json::to_vec(s) {
        Ok(bytes) => bytes.hash(&mut h),
        Err(_) => s.id.hash(&mut h),
    }
    h.finish()
}

fn sort_sessions(sessions: &mut [SessionSummary]) {
    sessions.sort_by(|a, b| {
        b.last_activity_at
            .cmp(&a.last_activity_at)
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Diff the snapshot's in-scope sessions against its stored hashes, update
/// them and broadcast one `sessions` diff if anything changed. Caller holds
/// the write lock, so seq order on the bus matches snapshot order.
fn diff_and_broadcast(state: &AppState, snap: &mut Snapshot, only: Option<&str>) {
    let now = crate::claude::registry::now_ms();
    let mut upsert = Vec::new();
    let mut remove = Vec::new();
    match only {
        Some(id) => {
            let current = snap
                .sessions
                .iter()
                .find(|s| s.id == id && s.in_active_scope(now));
            match current {
                Some(s) => {
                    let h = content_hash(s);
                    if snap.hashes.get(id) != Some(&h) {
                        upsert.push(s.clone());
                        snap.hashes.insert(id.to_string(), h);
                    }
                }
                None => {
                    if snap.hashes.remove(id).is_some() {
                        remove.push(id.to_string());
                    }
                }
            }
        }
        None => {
            let mut next = HashMap::new();
            for s in snap.sessions.iter().filter(|s| s.in_active_scope(now)) {
                let h = content_hash(s);
                if snap.hashes.get(&s.id) != Some(&h) {
                    upsert.push(s.clone());
                }
                next.insert(s.id.clone(), h);
            }
            remove = snap
                .hashes
                .keys()
                .filter(|id| !next.contains_key(*id))
                .cloned()
                .collect();
            remove.sort();
            snap.hashes = next;
        }
    }
    if upsert.is_empty() && remove.is_empty() {
        return;
    }
    snap.seq += 1;
    state.broadcast(ServerEvent::Sessions {
        full: false,
        sessions: None,
        upsert: Some(upsert),
        remove: Some(remove),
        seq: snap.seq,
    });
}

/// Replace the whole session list (the data layer's refresh) and broadcast the
/// diff — only sessions whose content changed are sent.
pub async fn publish_sessions(state: &AppState, mut sessions: Vec<SessionSummary>) {
    sort_sessions(&mut sessions);
    let mut snap = state.snapshot.write().await;
    snap.sessions = sessions;
    diff_and_broadcast(state, &mut snap, None);
}

/// Insert or replace one session and broadcast it if it changed.
pub async fn upsert_session(state: &AppState, summary: SessionSummary) {
    let mut snap = state.snapshot.write().await;
    let id = summary.id.clone();
    match snap.sessions.iter_mut().find(|s| s.id == id) {
        Some(existing) => *existing = summary,
        None => snap.sessions.push(summary),
    }
    sort_sessions(&mut snap.sessions);
    diff_and_broadcast(state, &mut snap, Some(&id));
}

/// Mutate one session in place and broadcast it if it changed. Returns false
/// when the session is not in the snapshot.
pub async fn patch_session<F>(state: &AppState, id: &str, f: F) -> bool
where
    F: FnOnce(&mut SessionSummary),
{
    let mut snap = state.snapshot.write().await;
    let Some(s) = snap.sessions.iter_mut().find(|s| s.id == id) else {
        return false;
    };
    f(s);
    diff_and_broadcast(state, &mut snap, Some(id));
    true
}

/// Refresh one session's `pending` previews (store items + kept observed
/// items) and the status/activity it implies, then broadcast. New store items
/// also go out in full as `pending{item}` frames (session + inbox
/// subscribers).
pub async fn sync_pending(state: &AppState, session_id: &str) {
    let live = state.pending.list(session_id);
    patch_session(state, session_id, |s| {
        let observed: Vec<PendingPreview> = s.pending.drain(..).filter(|p| !p.answerable).collect();
        s.pending = live.iter().map(PendingPreview::from).collect();
        s.pending.extend(observed);
        if s.pending.iter().any(|p| p.answerable) {
            s.status = SessionStatus::Waiting;
        } else if s.status == SessionStatus::Waiting && s.pending.is_empty() {
            s.status = if s.live {
                SessionStatus::Working
            } else {
                SessionStatus::Ended
            };
        }
        s.activity = crate::claude::summaries::activity_for(s.status, &s.pending);
    })
    .await;
    // Announce each store item once, whether or not the session is in the
    // snapshot yet (a refresh may already have listed its preview).
    let fresh: Vec<PendingItem> = {
        let all: HashSet<String> = state.pending.all().into_iter().map(|p| p.id).collect();
        let mut snap = state.snapshot.write().await;
        snap.announced.retain(|id| all.contains(id));
        live.into_iter()
            .filter(|p| snap.announced.insert(p.id.clone()))
            .collect()
    };
    for item in fresh {
        publish_pending(state, item);
    }
}

/// Transcript appends for subscribers of `session` (main transcript).
pub fn publish_transcript(state: &AppState, session: &str, events: Vec<UiEvent>, tail: String) {
    publish_agent_transcript(state, session, None, events, tail);
}

/// Transcript appends for subscribers of `session` / `agent`.
pub fn publish_agent_transcript(
    state: &AppState,
    session: &str,
    agent: Option<&str>,
    events: Vec<UiEvent>,
    tail: String,
) {
    if events.is_empty() {
        return;
    }
    state.broadcast(ServerEvent::Transcript {
        session: session.to_string(),
        agent: agent.map(str::to_string),
        events,
        tail,
    });
}

/// Tell subscribers of `session` / `agent` to refetch the last page.
pub fn publish_transcript_reset(state: &AppState, session: &str, agent: Option<&str>) {
    state.broadcast(ServerEvent::TranscriptReset {
        session: session.to_string(),
        agent: agent.map(str::to_string),
    });
}

/// A full pending item, for subscribers of its session and inbox
/// subscribers.
pub fn publish_pending(state: &AppState, item: PendingItem) {
    state.broadcast(ServerEvent::Pending { item });
}

/// An owned session's live delta, for subscribers of `session`.
pub fn publish_stream(state: &AppState, session: &str, frame: StreamFrame) {
    state.broadcast(ServerEvent::Stream {
        session: session.to_string(),
        frame,
    });
}

/// A session's agents changed, for subscribers of `session`.
pub fn publish_agents(
    state: &AppState,
    session: &str,
    agents: Vec<AgentInfo>,
    workflows: Vec<WorkflowRun>,
) {
    state.broadcast(ServerEvent::Agents {
        session: session.to_string(),
        agents,
        workflows,
    });
}

/// Running agents across sessions, for fleet subscribers.
pub fn publish_fleet(state: &AppState, sessions: Vec<FleetSession>) {
    state.broadcast(ServerEvent::Fleet {
        sessions,
        summary: Some(state.agents.fleet_summary()),
    });
}

/// A notice for everyone.
pub fn publish_notice(
    state: &AppState,
    level: NoticeLevel,
    text: impl Into<String>,
    session: Option<&str>,
) {
    state.broadcast(ServerEvent::Notice {
        level,
        text: text.into(),
        session: session.map(str::to_string),
    });
}

/// A notice for desktop clients only (security events that name devices,
/// addresses or remote-access settings).
pub fn publish_desktop_notice(state: &AppState, level: NoticeLevel, text: impl Into<String>) {
    state.broadcast(ServerEvent::DesktopNotice {
        level,
        text: text.into(),
        session: None,
    });
}

/// Devices / pending pairings changed (desktop clients refetch).
pub fn publish_devices(state: &AppState) {
    state.broadcast(ServerEvent::Devices);
}

/// A pairing waits for approval (desktop clients).
pub fn publish_pair_request(state: &AppState, pending: Value) {
    state.broadcast(ServerEvent::PairRequest { pending });
}

/// A pending item left the store (answered here or elsewhere, withdrawn,
/// expired), for everyone.
pub fn publish_pending_resolved(state: &AppState, resolved: PendingResolved) {
    state.broadcast(ServerEvent::PendingResolved(resolved));
}

/// Tell every client to refetch and resubscribe.
pub fn publish_resync(state: &AppState) {
    state.broadcast(ServerEvent::Resync);
}

/// The plan usage changed, for everyone (`claude::usage` publishes only
/// when what clients see changed).
pub fn publish_plan_usage(state: &AppState, usage: PlanUsage) {
    state.broadcast(ServerEvent::PlanUsage { usage });
}

/// The accounts changed, for everyone.
pub fn publish_accounts(state: &AppState, accounts: crate::claude::accounts::view::AccountsView) {
    state.broadcast(ServerEvent::Accounts { accounts });
}

/// Another account's plan usage changed, for everyone.
pub fn publish_account_usage(state: &AppState, account: &str, usage: PlanUsage) {
    state.broadcast(ServerEvent::AccountUsage {
        account_id: account.to_string(),
        usage,
    });
}

/// Every other account's usage, as the frames a connecting client gets.
fn account_usage_frames(state: &AppState) -> Vec<ServerEvent> {
    crate::claude::usage::others(state)
        .into_iter()
        .map(|(account_id, usage)| ServerEvent::AccountUsage { account_id, usage })
        .collect()
}

/// The `accounts` frame a client gets after its `planUsage` frame, once
/// there is more than the Default account.
fn accounts_frame(state: &AppState) -> Option<ServerEvent> {
    crate::claude::accounts::view::connect_frame(state)
        .map(|accounts| ServerEvent::Accounts { accounts })
}

/// The `planUsage` frame a client gets after its full `sessions` frame.
fn plan_usage_frame(state: &AppState) -> ServerEvent {
    ServerEvent::PlanUsage {
        usage: crate::claude::usage::current(state),
    }
}

/// Close every socket of a revoked device (4401). Returns how many.
pub fn close_device(state: &AppState, device_id: &str) -> usize {
    state
        .hub
        .close_device(device_id, CLOSE_UNAUTHORIZED, "device revoked")
}

// ────────────────────────────────── hub ───────────────────────────────────────

type CloseSignal = Option<(u16, &'static str)>;

struct ClientEntry {
    device_id: Option<String>,
    desktop: bool,
    opened: Instant,
    close: watch::Sender<CloseSignal>,
}

/// A subscription key: a session's main transcript (`None`) or one agent.
pub type SubKey = (String, Option<String>);

/// Registry of connected sockets (for revocation, caps and stats) and of
/// what they subscribe to (so the monitor reads only watched transcripts).
#[derive(Default)]
pub struct Hub {
    next_id: AtomicU64,
    clients: Mutex<HashMap<u64, ClientEntry>>,
    sub_counts: Mutex<HashMap<SubKey, usize>>,
    fleet_subs: AtomicU64,
    inbox_subs: AtomicU64,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub")
            .field("clients", &self.client_count())
            .finish()
    }
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, ClientEntry>> {
        self.clients.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Register a socket; closes the device's oldest sockets beyond the cap.
    fn register(
        &self,
        device_id: Option<String>,
        desktop: bool,
    ) -> (u64, watch::Receiver<CloseSignal>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = watch::channel(None);
        let mut clients = self.lock();
        if let Some(dev) = &device_id {
            let mut same: Vec<(u64, Instant)> = clients
                .iter()
                .filter(|(_, c)| c.device_id.as_deref() == Some(dev))
                .map(|(id, c)| (*id, c.opened))
                .collect();
            same.sort_by_key(|(_, opened)| *opened);
            while same.len() >= MAX_SOCKETS_PER_DEVICE {
                let (old, _) = same.remove(0);
                if let Some(c) = clients.remove(&old) {
                    let _ = c.close.send(Some((CLOSE_REPLACED, "too many sockets")));
                }
            }
        }
        clients.insert(
            id,
            ClientEntry {
                device_id,
                desktop,
                opened: Instant::now(),
                close: tx,
            },
        );
        (id, rx)
    }

    fn unregister(&self, id: u64) {
        self.lock().remove(&id);
    }

    /// Close every socket of `device_id` with `code`. Returns how many.
    pub fn close_device(&self, device_id: &str, code: u16, reason: &'static str) -> usize {
        let mut clients = self.lock();
        let ids: Vec<u64> = clients
            .iter()
            .filter(|(_, c)| c.device_id.as_deref() == Some(device_id))
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            if let Some(c) = clients.remove(id) {
                let _ = c.close.send(Some((code, reason)));
            }
        }
        ids.len()
    }

    pub fn client_count(&self) -> usize {
        self.lock().len()
    }

    pub fn desktop_count(&self) -> usize {
        self.lock().values().filter(|c| c.desktop).count()
    }

    pub fn device_sockets(&self, device_id: &str) -> usize {
        self.lock()
            .values()
            .filter(|c| c.device_id.as_deref() == Some(device_id))
            .count()
    }

    fn counts(&self) -> std::sync::MutexGuard<'_, HashMap<SubKey, usize>> {
        self.sub_counts.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn add_sub(&self, key: &SubKey) {
        *self.counts().entry(key.clone()).or_default() += 1;
    }

    fn remove_sub(&self, key: &SubKey) {
        let mut g = self.counts();
        if let Some(n) = g.get_mut(key) {
            *n -= 1;
            if *n == 0 {
                g.remove(key);
            }
        }
    }

    /// Every `(session, agent?)` someone is subscribed to.
    pub fn subscriptions(&self) -> Vec<SubKey> {
        self.counts().keys().cloned().collect()
    }

    /// Whether anyone watches `(session, agent?)`.
    pub fn is_subscribed(&self, session: &str, agent: Option<&str>) -> bool {
        self.counts()
            .contains_key(&(session.to_string(), agent.map(str::to_string)))
    }

    /// Whether any client subscribed to the fleet.
    pub fn fleet_subscribed(&self) -> bool {
        self.fleet_subs.load(Ordering::Relaxed) > 0
    }

    /// Whether any client subscribed to the inbox.
    pub fn inbox_subscribed(&self) -> bool {
        self.inbox_subs.load(Ordering::Relaxed) > 0
    }
}

// ──────────────────────────────── presence ────────────────────────────────────

/// Per-device foreground state reported by `visible{v}` frames (B5 reads it
/// to skip push for devices that are looking at the app).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevicePresence {
    /// Socket id → visible.
    pub sockets: HashMap<u64, bool>,
    /// Epoch ms of the last frame from any of the device's sockets.
    pub last_seen: i64,
}

impl DevicePresence {
    pub fn visible(&self) -> bool {
        self.sockets.values().any(|v| *v)
    }
}

#[derive(Debug, Default)]
pub struct Presence {
    devices: Mutex<HashMap<String, DevicePresence>>,
}

impl Presence {
    pub fn new() -> Self {
        Self::default()
    }

    fn with<R>(&self, f: impl FnOnce(&mut HashMap<String, DevicePresence>) -> R) -> R {
        let mut g = self.devices.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut g)
    }

    /// A socket opened. New sockets count as visible until told otherwise (a
    /// page that just connected is on screen).
    pub fn connect(&self, device_id: &str, socket: u64) {
        let now = crate::claude::registry::now_ms();
        self.with(|m| {
            let d = m.entry(device_id.to_string()).or_default();
            d.sockets.insert(socket, true);
            d.last_seen = now;
        });
    }

    pub fn disconnect(&self, device_id: &str, socket: u64) {
        self.with(|m| {
            if let Some(d) = m.get_mut(device_id) {
                d.sockets.remove(&socket);
            }
        });
    }

    pub fn set_visible(&self, device_id: &str, socket: u64, visible: bool) {
        let now = crate::claude::registry::now_ms();
        self.with(|m| {
            let d = m.entry(device_id.to_string()).or_default();
            d.sockets.insert(socket, visible);
            d.last_seen = now;
        });
    }

    pub fn touch(&self, device_id: &str) {
        let now = crate::claude::registry::now_ms();
        self.with(|m| {
            if let Some(d) = m.get_mut(device_id) {
                d.last_seen = now;
            }
        });
    }

    pub fn get(&self, device_id: &str) -> Option<DevicePresence> {
        self.with(|m| m.get(device_id).cloned())
    }

    /// A socket of this device is open, visible, and heard from within 60 s.
    pub fn is_foreground(&self, device_id: &str) -> bool {
        let now = crate::claude::registry::now_ms();
        self.get(device_id)
            .is_some_and(|d| d.visible() && now - d.last_seen < CLIENT_TIMEOUT.as_millis() as i64)
    }
}

// ─────────────────────────────── per client ───────────────────────────────────

enum Out {
    Text(String),
    /// The queue overflowed: send `resync`, then a fresh full snapshot.
    Resync,
}

/// Bounded per-client queue; overflow drops everything queued and leaves one
/// resync marker.
struct OutQueue {
    q: Mutex<VecDeque<Out>>,
    notify: Notify,
}

impl OutQueue {
    fn new() -> Self {
        Self {
            q: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
        }
    }

    fn push(&self, out: Out) {
        {
            let mut q = self.q.lock().unwrap_or_else(|p| p.into_inner());
            if q.len() >= QUEUE_CAP {
                q.clear();
                q.push_back(Out::Resync);
            } else {
                q.push_back(out);
            }
        }
        self.notify.notify_one();
    }

    fn push_event(&self, ev: &ServerEvent) {
        match serde_json::to_string(ev) {
            Ok(text) => self.push(Out::Text(text)),
            Err(e) => tracing::warn!(error = %e, "could not serialize a ws frame"),
        }
    }

    async fn pop(&self) -> Out {
        loop {
            if let Some(o) = self.q.lock().unwrap_or_else(|p| p.into_inner()).pop_front() {
                return o;
            }
            self.notify.notified().await;
        }
    }
}

#[derive(Debug, Default)]
struct Subs {
    sessions: HashSet<SubKey>,
    fleet: bool,
    inbox: bool,
}

impl Subs {
    fn has_session(&self, session: &str) -> bool {
        self.sessions.iter().any(|(s, _)| s == session)
    }
}

/// Frames a client may send.
#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum ClientFrame {
    Auth {
        #[serde(default)]
        token: String,
    },
    Sub {
        session: String,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        since: Option<String>,
    },
    Unsub {
        session: String,
        #[serde(default)]
        agent: Option<String>,
    },
    SubFleet,
    UnsubFleet,
    SubInbox,
    UnsubInbox,
    Ping,
    Pong,
    Visible {
        #[serde(default)]
        v: bool,
    },
}

/// `Origin` absent, local, or naming the same host as `Host`.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    if origin_is_local(origin) {
        return true;
    }
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let origin_authority = origin
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
        .unwrap_or_default();
    !host.is_empty() && origin_authority.eq_ignore_ascii_case(host)
}

/// `GET /ws`.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    ctx: ClientContext,
    headers: HeaderMap,
) -> Response {
    if !origin_allowed(&headers) {
        return ApiError::forbidden(
            "origin_not_allowed",
            "WebSocket connections must come from this app's own origin.",
        )
        .into_response();
    }
    // Pre-auth limits (B1): a socket that has not authenticated yet holds a
    // slot until its first frame is checked. Pools are per scope (this
    // machine, the LAN, tunnels) and a full one closes its oldest waiting
    // socket rather than refusing the newcomer, so nobody on the internet can
    // lock the desktop or a paired phone out of live updates.
    let preauth = (!ctx.principal.is_user()).then(|| state.limits.preauth(ctx.rate_key));
    // Client frames are small; never buffer megabytes for a stranger.
    ws.max_message_size(MAX_CLIENT_FRAME)
        .max_frame_size(MAX_CLIENT_FRAME)
        .on_upgrade(move |socket| async move {
            run_client(socket, state, ctx, preauth).await;
        })
}

/// Largest client frame accepted (client frames are tiny JSON objects).
pub const MAX_CLIENT_FRAME: usize = 64 * 1024;
/// Largest first (`auth`) frame accepted before authentication.
pub const MAX_AUTH_FRAME: usize = 4 * 1024;

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

/// Wait for the first `auth` frame (control frames are skipped). `None` when
/// the socket ends or the first frame is not an auth frame; an auth frame
/// with an empty token yields `Some("")` (a credential that is rejected).
async fn first_auth_token(socket: &mut WebSocket) -> Option<String> {
    loop {
        match socket.recv().await? {
            Ok(Message::Text(text)) => {
                if text.len() > MAX_AUTH_FRAME {
                    return None;
                }
                return match serde_json::from_str::<ClientFrame>(&text) {
                    Ok(ClientFrame::Auth { token }) => Some(token),
                    _ => None,
                };
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            _ => return None,
        }
    }
}

fn full_frame(snap: &Snapshot) -> ServerEvent {
    ServerEvent::Sessions {
        full: true,
        sessions: Some(snap.active(crate::claude::registry::now_ms())),
        upsert: None,
        remove: None,
        seq: snap.seq,
    }
}

async fn run_client(
    mut socket: WebSocket,
    state: AppState,
    mut ctx: ClientContext,
    preauth: Option<super::ratelimit::PreAuthSlot>,
) {
    // 1. Authenticate (header / local ?token= already resolved, else first frame).
    if !ctx.principal.is_user() {
        let wait = tokio::time::timeout(AUTH_TIMEOUT, first_auth_token(&mut socket));
        let token = match &preauth {
            Some(slot) => tokio::select! {
                t = wait => t.ok().flatten(),
                _ = slot.evicted() => {
                    close(&mut socket, CLOSE_TIMEOUT, "too many unauthenticated sockets").await;
                    return;
                }
            },
            None => wait.await.ok().flatten(),
        };
        let Some(token) = token else {
            // Nothing was rejected: the client was too slow (or spoke first
            // with something else). 4401 would make it drop its token.
            close(&mut socket, CLOSE_TIMEOUT, "auth timeout").await;
            return;
        };
        let ok =
            !token.is_empty() && super::auth::authenticate_token(&state, &mut ctx, &token).is_ok();
        if !ok || !ctx.principal.is_user() {
            close(&mut socket, CLOSE_UNAUTHORIZED, "unauthorized").await;
            return;
        }
    }
    drop(preauth);
    let device_id = ctx.device_id().map(str::to_string);
    let desktop = ctx.is_desktop();
    let (client_id, mut close_rx) = state.hub.register(device_id.clone(), desktop);
    if let Some(dev) = &device_id {
        state.presence.connect(dev, client_id);
    }

    // 2. Subscribe BEFORE reading the snapshot so nothing falls in between;
    //    diffs already contained in the snapshot are skipped by seq.
    let mut rx = state.bus.subscribe();
    let out = Arc::new(OutQueue::new());
    let subs = Arc::new(Mutex::new(Subs::default()));
    let hello = serde_json::json!({
        "t": "hello",
        "v": PROTOCOL_VERSION,
        "me": super::auth::me(&state, &ctx),
        "serverTime": crate::claude::registry::now_ms(),
    });
    out.push(Out::Text(hello.to_string()));
    let base_seq = {
        let snap = state.snapshot.read().await;
        out.push_event(&full_frame(&snap));
        snap.seq
    };
    out.push_event(&plan_usage_frame(&state));
    if let Some(frame) = accounts_frame(&state) {
        out.push_event(&frame);
    }
    for frame in account_usage_frames(&state) {
        out.push_event(&frame);
    }
    // The first client may make stale plan usage worth a read.
    crate::claude::usage::on_client_connected(&state, state.hub.client_count());

    // 3. Pump: bus → filter → this client's queue.
    let pump = {
        let (out, subs) = (out.clone(), subs.clone());
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let wanted = match ev.audience() {
                            Audience::All => match &*ev {
                                ServerEvent::Sessions { seq, .. } => *seq > base_seq,
                                _ => true,
                            },
                            Audience::Desktop => desktop,
                            Audience::Session(s) => {
                                subs.lock().map(|g| g.has_session(s)).unwrap_or(false)
                            }
                            Audience::SessionKey(s, a) => subs
                                .lock()
                                .map(|g| {
                                    g.sessions.contains(&(s.to_string(), a.map(str::to_string)))
                                })
                                .unwrap_or(false),
                            Audience::SessionOrInbox(s) => subs
                                .lock()
                                .map(|g| g.inbox || g.has_session(s))
                                .unwrap_or(false),
                            Audience::Fleet => subs.lock().map(|g| g.fleet).unwrap_or(false),
                        };
                        if wanted {
                            out.push_event(&ev);
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "ws client lagged; resync");
                        out.push(Out::Resync);
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        })
    };

    // 4. Connection loop.
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await; // the first tick is immediate
    let mut last_rx = Instant::now();
    loop {
        tokio::select! {
            frame = out.pop() => {
                let sent = match frame {
                    Out::Text(text) => socket.send(Message::Text(text.into())).await,
                    Out::Resync => {
                        let resync = serde_json::to_string(&ServerEvent::Resync).unwrap_or_default();
                        let full = {
                            let snap = state.snapshot.read().await;
                            serde_json::to_string(&full_frame(&snap)).unwrap_or_default()
                        };
                        // A dropped backlog may have held a planUsage or
                        // accounts frame.
                        let usage =
                            serde_json::to_string(&plan_usage_frame(&state)).unwrap_or_default();
                        let accounts = accounts_frame(&state)
                            .and_then(|f| serde_json::to_string(&f).ok());
                        let others: Vec<String> = account_usage_frames(&state)
                            .iter()
                            .filter_map(|f| serde_json::to_string(f).ok())
                            .collect();
                        let mut sent = socket.send(Message::Text(resync.into())).await;
                        for text in [Some(full), Some(usage), accounts]
                            .into_iter()
                            .flatten()
                            .chain(others)
                        {
                            if sent.is_ok() {
                                sent = socket.send(Message::Text(text.into())).await;
                            }
                        }
                        sent
                    }
                };
                if sent.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                let Some(Ok(msg)) = incoming else { break };
                last_rx = Instant::now();
                if let Some(dev) = &device_id {
                    state.presence.touch(dev);
                }
                match msg {
                    Message::Text(text) => {
                        handle_frame(&state, &out, &subs, client_id, device_id.as_deref(), &text);
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            _ = ping.tick() => {
                if last_rx.elapsed() > CLIENT_TIMEOUT {
                    close(&mut socket, CLOSE_TIMEOUT, "no traffic").await;
                    break;
                }
                if socket.send(Message::Text(r#"{"t":"ping"}"#.into())).await.is_err() {
                    break;
                }
            }
            changed = close_rx.changed() => {
                if changed.is_err() {
                    // Sender dropped without a signal: we were unregistered.
                    break;
                }
                let signal = *close_rx.borrow();
                if let Some((code, reason)) = signal {
                    close(&mut socket, code, reason).await;
                    break;
                }
            }
        }
    }

    pump.abort();
    // Release this client's subscriptions.
    if let Ok(g) = subs.lock() {
        for key in &g.sessions {
            state.hub.remove_sub(key);
        }
        if g.fleet {
            state.hub.fleet_subs.fetch_sub(1, Ordering::Relaxed);
        }
        if g.inbox {
            state.hub.inbox_subs.fetch_sub(1, Ordering::Relaxed);
        }
    }
    state.hub.unregister(client_id);
    if let Some(dev) = &device_id {
        state.presence.disconnect(dev, client_id);
    }
}

fn handle_frame(
    state: &AppState,
    out: &Arc<OutQueue>,
    subs: &Arc<Mutex<Subs>>,
    client_id: u64,
    device_id: Option<&str>,
    text: &str,
) {
    let frame = match serde_json::from_str::<ClientFrame>(text) {
        Ok(f) => f,
        Err(_) => return, // tolerant: unknown frames are ignored
    };
    match frame {
        ClientFrame::Auth { .. } | ClientFrame::Pong => {}
        ClientFrame::Ping => out.push(Out::Text(r#"{"t":"pong"}"#.into())),
        ClientFrame::Sub {
            session,
            agent,
            since,
        } => {
            // Ids name files: only well-formed ones are accepted.
            if session.is_empty()
                || session.len() > 80
                || agent
                    .as_deref()
                    .is_some_and(|a| !crate::claude::ids::is_agent_id(a))
            {
                return;
            }
            let key: SubKey = (session.clone(), agent.clone());
            if let Ok(mut g) = subs.lock() {
                if g.sessions.len() < MAX_SUBS_PER_CLIENT && g.sessions.insert(key.clone()) {
                    state.hub.add_sub(&key);
                }
            }
            if let Some(since) = since
                .as_deref()
                .and_then(crate::claude::uievents::parse_cursor)
            {
                let (state, out) = (state.clone(), out.clone());
                tokio::spawn(async move {
                    replay(&state, &out, &session, agent.as_deref(), since).await
                });
            }
        }
        ClientFrame::Unsub { session, agent } => {
            let key: SubKey = (session, agent);
            if let Ok(mut g) = subs.lock() {
                if g.sessions.remove(&key) {
                    state.hub.remove_sub(&key);
                }
            }
        }
        ClientFrame::SubFleet => {
            if let Ok(mut g) = subs.lock() {
                if !g.fleet {
                    g.fleet = true;
                    state.hub.fleet_subs.fetch_add(1, Ordering::Relaxed);
                }
            }
            out.push_event(&ServerEvent::Fleet {
                sessions: state.agents.fleet(),
                summary: Some(state.agents.fleet_summary()),
            });
        }
        ClientFrame::UnsubFleet => {
            if let Ok(mut g) = subs.lock() {
                if g.fleet {
                    g.fleet = false;
                    state.hub.fleet_subs.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
        ClientFrame::SubInbox => {
            if let Ok(mut g) = subs.lock() {
                if !g.inbox {
                    g.inbox = true;
                    state.hub.inbox_subs.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        ClientFrame::UnsubInbox => {
            if let Ok(mut g) = subs.lock() {
                if g.inbox {
                    g.inbox = false;
                    state.hub.inbox_subs.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
        ClientFrame::Visible { v } => {
            if let Some(dev) = device_id {
                state.presence.set_visible(dev, client_id, v);
            }
        }
    }
}

/// Subscriptions one client may hold.
const MAX_SUBS_PER_CLIENT: usize = 64;

/// `sub{since}`: send everything written after the client's cursor, capped
/// (2 MB / 300 events); beyond the cap send `transcriptReset`.
async fn replay(state: &AppState, out: &OutQueue, session: &str, agent: Option<&str>, since: u64) {
    // Paths come from the index (discovered on disk), never from the ids.
    let path = match agent {
        Some(a) => state.index.subagent_path(session, a),
        None if crate::claude::ids::is_session_id(session) => state.transcript_path(session).await,
        None => None,
    };
    let Some(path) = path else {
        return;
    };
    let sid = session.to_string();
    let agent_owned = agent.map(str::to_string);
    let read = tokio::task::spawn_blocking(move || {
        crate::claude::uievents::read_range(&sid, &path, since, None, agent_owned.as_deref())
    })
    .await;
    match read {
        Ok(Ok(r)) if r.reset => out.push_event(&ServerEvent::TranscriptReset {
            session: session.to_string(),
            agent: agent.map(str::to_string),
        }),
        Ok(Ok(r)) => out.push_event(&ServerEvent::Transcript {
            session: session.to_string(),
            agent: agent.map(str::to_string),
            events: r.events,
            tail: r.tail.to_string(),
        }),
        Ok(Err(e)) => tracing::debug!(error = %e, session, "transcript replay failed"),
        Err(e) => tracing::debug!(error = %e, session, "transcript replay task failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_use_the_contract_shape() {
        let diff = ServerEvent::Sessions {
            full: false,
            sessions: None,
            upsert: Some(vec![]),
            remove: Some(vec!["x".into()]),
            seq: 3,
        };
        let v = serde_json::to_value(&diff).unwrap();
        assert_eq!(
            v,
            json!({ "t": "sessions", "full": false, "upsert": [], "remove": ["x"], "seq": 3 })
        );

        let stream = ServerEvent::Stream {
            session: "s".into(),
            frame: StreamFrame {
                kind: crate::claude::model::StreamKind::Text,
                delta: Some("hi".into()),
                ..Default::default()
            },
        };
        assert_eq!(
            serde_json::to_value(&stream).unwrap(),
            json!({ "t": "stream", "session": "s", "kind": "text", "delta": "hi" })
        );
        assert_eq!(
            serde_json::to_value(&ServerEvent::Resync).unwrap(),
            json!({ "t": "resync" })
        );
        assert_eq!(
            serde_json::to_value(&ServerEvent::Devices).unwrap(),
            json!({ "t": "devices" })
        );
        assert_eq!(
            serde_json::to_value(&ServerEvent::PairRequest {
                pending: json!({"id": 1})
            })
            .unwrap(),
            json!({ "t": "pairRequest", "pending": { "id": 1 } })
        );
        let notice = ServerEvent::Notice {
            level: NoticeLevel::Warning,
            text: "x".into(),
            session: None,
        };
        assert_eq!(
            serde_json::to_value(&notice).unwrap(),
            json!({ "t": "notice", "level": "warning", "text": "x" })
        );
        // The desktop-only notice is the same frame, for the desktop only.
        let desk = ServerEvent::DesktopNotice {
            level: NoticeLevel::Warning,
            text: "x".into(),
            session: None,
        };
        assert_eq!(
            serde_json::to_value(&desk).unwrap(),
            json!({ "t": "notice", "level": "warning", "text": "x" })
        );
        assert_eq!(desk.audience(), Audience::Desktop);
        assert_eq!(notice.audience(), Audience::All);
        let t = ServerEvent::Transcript {
            session: "s".into(),
            agent: Some("a1".into()),
            events: vec![],
            tail: "5".into(),
        };
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            json!({ "t": "transcript", "session": "s", "agent": "a1", "events": [], "tail": "5" })
        );
        assert_eq!(
            serde_json::to_value(&ServerEvent::TranscriptReset {
                session: "s".into(),
                agent: None
            })
            .unwrap(),
            json!({ "t": "transcriptReset", "session": "s" })
        );
        let r = ServerEvent::PendingResolved(PendingResolved {
            id: "p1".into(),
            session: "s".into(),
            outcome: crate::claude::model::PendingOutcome::Allowed,
            by: None,
            at: 7,
            text: None,
        });
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({ "t": "pendingResolved", "id": "p1", "session": "s", "outcome": "allowed", "at": 7 })
        );
        let p = ServerEvent::Pending {
            item: PendingItem {
                id: "p2".into(),
                session_id: "s".into(),
                ..Default::default()
            },
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["t"], "pending");
        assert_eq!(v["item"]["id"], "p2");
        assert_eq!(p.audience(), Audience::SessionOrInbox("s"));

        let u = ServerEvent::PlanUsage {
            usage: PlanUsage {
                available: true,
                windows: vec![crate::claude::model::PlanWindow {
                    key: "five_hour".into(),
                    label: "Current session".into(),
                    pct: 42.0,
                    resets_at: Some(1_790_000_000_000),
                    severity: Some(crate::claude::model::PlanSeverity::Warning),
                }],
                extra_usage: Some(crate::claude::model::ExtraUsage {
                    enabled: true,
                    used_minor: 1250,
                    limit_minor: Some(5000),
                    currency: Some("USD".into()),
                    decimals: 2,
                    utilization: Some(25.0),
                }),
                fetched_at: Some(5),
                source: Some(crate::claude::model::PlanUsageSource::Live),
                stream_at: Some(7),
                refreshing: false,
                next_refresh_at: Some(9),
                interval_secs: 300,
                error: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&u).unwrap(),
            json!({ "t": "planUsage", "usage": {
                "available": true,
                "windows": [{ "key": "five_hour", "label": "Current session", "pct": 42.0,
                              "resetsAt": 1_790_000_000_000_i64, "severity": "warning" }],
                "extraUsage": { "enabled": true, "usedMinor": 1250, "limitMinor": 5000,
                                "currency": "USD", "decimals": 2, "utilization": 25.0 },
                "fetchedAt": 5, "source": "live", "streamAt": 7, "refreshing": false,
                "nextRefreshAt": 9, "intervalSecs": 300 } })
        );
        assert_eq!(u.audience(), Audience::All);
        // Unknowns stay out of the frame.
        let v = serde_json::to_value(ServerEvent::PlanUsage {
            usage: PlanUsage::default(),
        })
        .unwrap();
        assert_eq!(
            v,
            json!({ "t": "planUsage", "usage": { "available": false, "windows": [],
                     "refreshing": false, "intervalSecs": 0 } })
        );
    }

    #[test]
    fn hub_counts_subscriptions() {
        let hub = Hub::new();
        let k: SubKey = ("s".into(), None);
        hub.add_sub(&k);
        hub.add_sub(&k);
        assert!(hub.is_subscribed("s", None));
        assert!(!hub.is_subscribed("s", Some("a")));
        hub.remove_sub(&k);
        assert_eq!(hub.subscriptions(), vec![k.clone()]);
        hub.remove_sub(&k);
        assert!(hub.subscriptions().is_empty());
        assert!(!hub.fleet_subscribed() && !hub.inbox_subscribed());
    }

    #[test]
    fn client_frames_parse() {
        let f: ClientFrame =
            serde_json::from_str(r#"{"t":"sub","session":"a","since":"12"}"#).unwrap();
        assert!(
            matches!(f, ClientFrame::Sub { ref session, agent: None, since: Some(ref s) } if session == "a" && s == "12")
        );
        let f: ClientFrame =
            serde_json::from_str(r#"{"t":"sub","session":"a","agent":"a1"}"#).unwrap();
        assert!(matches!(f, ClientFrame::Sub { agent: Some(ref a), .. } if a == "a1"));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"t":"subInbox"}"#).unwrap(),
            ClientFrame::SubInbox
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"t":"subFleet"}"#).unwrap(),
            ClientFrame::SubFleet
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(r#"{"t":"visible","v":true}"#).unwrap(),
            ClientFrame::Visible { v: true }
        ));
        assert!(serde_json::from_str::<ClientFrame>(r#"{"t":"nope"}"#).is_err());
    }

    #[test]
    fn queue_overflow_collapses_to_one_resync() {
        let q = OutQueue::new();
        for i in 0..QUEUE_CAP + 10 {
            q.push(Out::Text(i.to_string()));
        }
        let g = q.q.lock().unwrap();
        // Overflow dropped the backlog; what follows the marker is newer.
        assert!(matches!(g.front(), Some(Out::Resync)));
        assert!(g.len() <= 10);
    }

    #[test]
    fn origin_rule() {
        let mut h = HeaderMap::new();
        assert!(origin_allowed(&h));
        h.insert("host", "abc.ngrok-free.app".parse().unwrap());
        h.insert("origin", "https://abc.ngrok-free.app".parse().unwrap());
        assert!(origin_allowed(&h));
        h.insert("origin", "https://evil.example".parse().unwrap());
        assert!(!origin_allowed(&h));
        h.insert("origin", "tauri://localhost".parse().unwrap());
        assert!(origin_allowed(&h));
        h.insert("host", "192.168.1.5:6725".parse().unwrap());
        h.insert("origin", "https://192.168.1.5:6725".parse().unwrap());
        assert!(origin_allowed(&h));
    }

    #[test]
    fn hub_caps_sockets_per_device_and_closes_revoked() {
        let hub = Hub::new();
        let mut rxs = Vec::new();
        for _ in 0..MAX_SOCKETS_PER_DEVICE {
            rxs.push(hub.register(Some("d1".into()), false));
        }
        assert_eq!(hub.device_sockets("d1"), MAX_SOCKETS_PER_DEVICE);
        let _extra = hub.register(Some("d1".into()), false);
        assert_eq!(hub.device_sockets("d1"), MAX_SOCKETS_PER_DEVICE);
        assert_eq!(
            *rxs[0].1.borrow(),
            Some((CLOSE_REPLACED, "too many sockets"))
        );
        let _desk = hub.register(None, true);
        assert_eq!(hub.desktop_count(), 1);
        assert_eq!(
            hub.close_device("d1", CLOSE_UNAUTHORIZED, "device revoked"),
            MAX_SOCKETS_PER_DEVICE
        );
        assert_eq!(
            *rxs[1].1.borrow(),
            Some((CLOSE_UNAUTHORIZED, "device revoked"))
        );
        assert_eq!(hub.client_count(), 1);
    }

    #[test]
    fn presence_tracks_visibility() {
        let p = Presence::new();
        assert!(!p.is_foreground("d"));
        p.connect("d", 1);
        assert!(p.is_foreground("d"));
        p.set_visible("d", 1, false);
        assert!(!p.is_foreground("d"));
        p.connect("d", 2);
        assert!(p.is_foreground("d"));
        p.disconnect("d", 2);
        assert!(!p.is_foreground("d"));
    }
}
