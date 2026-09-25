//! WebSocket v2 hub (design §4.6).
//!
//! One socket per client, all fed from the single broadcast bus
//! ([`crate::state::Inner::bus`]). Frames are JSON objects tagged with `t`.
//!
//! Client → server: `auth{token}`, `sub{session, since?}`, `unsub{session}`,
//! `subFleet{}`, `unsubFleet{}`, `ping{}`, `pong{}`, `visible{v}`.
//!
//! Server → client: `hello{v:2, me, serverTime}` after auth, then
//! `sessions{full:true, sessions, seq}`, then `sessions{full:false, upsert,
//! remove, seq}` diffs (content-hash per session, `active` scope only),
//! `transcript` / `stream` / `agents` only for subscribed sessions, `fleet` for
//! fleet subscribers, `notice`, desktop-only `devices` / `pairRequest`,
//! `resync` (then a fresh full `sessions`), `ping` every 25 s, `pong`.
//!
//! * Auth: the upgrade needs no token in the URL. A local client may still use
//!   `?token=` (resolved by the context middleware), anyone may send
//!   `Authorization: Bearer`; otherwise the first frame must be
//!   `{"t":"auth","token":…}` within 5 s, or the socket closes with 4401.
//! * Origin: the upgrade is refused unless `Origin` is absent, local, or the
//!   same host as `Host`.
//! * Backpressure: each client has a bounded queue (256 frames). On overflow
//!   (or bus lag) the queue is dropped and `resync` + a full `sessions` frame
//!   are sent instead.
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
    AgentInfo, FleetSession, NoticeLevel, SessionStatus, SessionSummary, StreamFrame, UiEvent,
    WorkflowRun,
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
    /// Transcript appends for one session (subscribers only).
    Transcript {
        session: String,
        events: Vec<UiEvent>,
        tail: String,
    },
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
    Fleet { sessions: Vec<FleetSession> },
    /// Human-readable notice (everyone).
    Notice {
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
}

/// Who receives an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience<'a> {
    All,
    Desktop,
    Session(&'a str),
    Fleet,
}

impl ServerEvent {
    pub fn audience(&self) -> Audience<'_> {
        match self {
            ServerEvent::Sessions { .. } | ServerEvent::Notice { .. } | ServerEvent::Resync => {
                Audience::All
            }
            ServerEvent::Transcript { session, .. }
            | ServerEvent::Stream { session, .. }
            | ServerEvent::Agents { session, .. } => Audience::Session(session),
            ServerEvent::Fleet { .. } => Audience::Fleet,
            ServerEvent::Devices | ServerEvent::PairRequest { .. } => Audience::Desktop,
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

/// Refresh one session's `pending` (store items + kept observed items) and the
/// status/activity it implies, then broadcast.
pub async fn sync_pending(state: &AppState, session_id: &str) {
    let live = state.pending.list(session_id);
    patch_session(state, session_id, |s| {
        let observed: Vec<_> = s.pending.drain(..).filter(|p| !p.answerable).collect();
        s.pending = live;
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
}

/// Transcript appends for subscribers of `session`.
pub fn publish_transcript(state: &AppState, session: &str, events: Vec<UiEvent>, tail: String) {
    if events.is_empty() {
        return;
    }
    state.broadcast(ServerEvent::Transcript {
        session: session.to_string(),
        events,
        tail,
    });
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
    state.broadcast(ServerEvent::Fleet { sessions });
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

/// Devices / pending pairings changed (desktop clients refetch).
pub fn publish_devices(state: &AppState) {
    state.broadcast(ServerEvent::Devices);
}

/// A pairing waits for approval (desktop clients).
pub fn publish_pair_request(state: &AppState, pending: Value) {
    state.broadcast(ServerEvent::PairRequest { pending });
}

/// Tell every client to refetch and resubscribe.
pub fn publish_resync(state: &AppState) {
    state.broadcast(ServerEvent::Resync);
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

/// Registry of connected sockets (for revocation, caps and stats).
#[derive(Default)]
pub struct Hub {
    next_id: AtomicU64,
    clients: Mutex<HashMap<u64, ClientEntry>>,
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
    sessions: HashSet<String>,
    fleet: bool,
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
        since: Option<String>,
    },
    Unsub {
        session: String,
    },
    SubFleet,
    UnsubFleet,
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
    // slot (4 per client ip, 64 overall) until its first frame is checked.
    let preauth = if ctx.principal.is_user() {
        None
    } else {
        match state.limits.try_preauth(ctx.client_ip) {
            Some(slot) => Some(slot),
            None => {
                return ApiError::too_many_requests(
                    5,
                    "Too many unauthenticated WebSocket connections.",
                )
                .into_response()
            }
        }
    };
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

/// Wait for the first `auth` frame (control frames are skipped).
async fn first_auth_token(socket: &mut WebSocket) -> Option<String> {
    loop {
        match socket.recv().await? {
            Ok(Message::Text(text)) => {
                if text.len() > MAX_AUTH_FRAME {
                    return None;
                }
                return match serde_json::from_str::<ClientFrame>(&text) {
                    Ok(ClientFrame::Auth { token }) if !token.is_empty() => Some(token),
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
        let token = tokio::time::timeout(AUTH_TIMEOUT, first_auth_token(&mut socket))
            .await
            .ok()
            .flatten();
        let ok = token
            .map(|t| super::auth::authenticate_token(&state, &mut ctx, &t).is_ok())
            .unwrap_or(false);
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
                                subs.lock().map(|g| g.sessions.contains(s)).unwrap_or(false)
                            }
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
                        match socket.send(Message::Text(resync.into())).await {
                            Ok(()) => socket.send(Message::Text(full.into())).await,
                            Err(e) => Err(e),
                        }
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
        ClientFrame::Sub { session, since } => {
            if let Ok(mut g) = subs.lock() {
                g.sessions.insert(session.clone());
            }
            if let Some(since) = since
                .as_deref()
                .and_then(crate::claude::uievents::parse_cursor)
            {
                let (state, out) = (state.clone(), out.clone());
                tokio::spawn(async move { replay(&state, &out, &session, since).await });
            }
        }
        ClientFrame::Unsub { session } => {
            if let Ok(mut g) = subs.lock() {
                g.sessions.remove(&session);
            }
        }
        ClientFrame::SubFleet => {
            if let Ok(mut g) = subs.lock() {
                g.fleet = true;
            }
            out.push_event(&ServerEvent::Fleet {
                sessions: state.agents.fleet(),
            });
        }
        ClientFrame::UnsubFleet => {
            if let Ok(mut g) = subs.lock() {
                g.fleet = false;
            }
        }
        ClientFrame::Visible { v } => {
            if let Some(dev) = device_id {
                state.presence.set_visible(dev, client_id, v);
            }
        }
    }
}

/// `sub{since}`: send everything written after the client's cursor.
async fn replay(state: &AppState, out: &OutQueue, session: &str, since: u64) {
    let Some(path) = state.transcript_path(session).await else {
        return;
    };
    let sid = session.to_string();
    let read = tokio::task::spawn_blocking(move || {
        crate::claude::uievents::read_after(&sid, &path, since)
    })
    .await;
    match read {
        Ok(Ok((events, tail))) => out.push_event(&ServerEvent::Transcript {
            session: session.to_string(),
            events,
            tail: tail.to_string(),
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
    }

    #[test]
    fn client_frames_parse() {
        let f: ClientFrame =
            serde_json::from_str(r#"{"t":"sub","session":"a","since":"12"}"#).unwrap();
        assert!(
            matches!(f, ClientFrame::Sub { ref session, since: Some(ref s) } if session == "a" && s == "12")
        );
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
