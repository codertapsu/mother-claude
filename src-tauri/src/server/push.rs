//! Web Push notifications (design §6 + review amendments).
//!
//! Owner: **B5 (push)**.
//!
//! * **Keys** — one VAPID P-256 keypair, persisted 0600 as `vapid.json` in
//!   the app data dir ([`vapid`]).
//! * **Subscriptions** — per paired device in `push.json` (0600, [`store`]):
//!   at most 5 per device, removed when the device is revoked
//!   ([`drop_device`] / [`on_device_revoked`]), when it turns out unknown or
//!   revoked at send time, and when the push service answers 404/410.
//! * **SSRF defence** ([`endpoint`]) — an endpoint must be https, a DNS name
//!   (no IP literal) on the browser push services' allowlist, and resolve only
//!   to public addresses; checked on subscribe and again before every send.
//!   The sending client has redirects disabled, a 10 s timeout and no proxy.
//! * **Triggers** — called by other subsystems (cheap, non-blocking: they only
//!   enqueue): [`on_pending_added`] (every channel via `pending::raise`),
//!   [`on_turn_finished`], [`on_session_failed`], [`on_claude_notification`],
//!   [`on_notification_hook`]. The same facts are also derived from the event
//!   bus (`sessions` diffs, `transcript` appends), so a subsystem that forgets
//!   to call a trigger still produces the push; a [`engine::Suppressor`]
//!   makes the two paths yield one notification.
//! * **Coalescing** — per session, 10 s ([`engine::Coalescer`]): the first
//!   event goes out at once, the rest of the window becomes one follow-up. The
//!   `Topic` header (per session) lets the push service replace undelivered
//!   ones too.
//! * **Who gets it** — devices whose prefs / follow / mute / one-shot select
//!   the event ([`engine::wants`]), that are still paired, and that are NOT
//!   looking at the app right now (`state.presence.is_foreground`).
//! * **Payload** ([`payload`]) — title = session display name, body = event
//!   kind + tool name; details only for devices that opted in. No URL: the
//!   service worker builds it from `sessionId`.
//! * **VAPID `sub`** — `MOTHER_CLAUDE_VAPID_SUBJECT` if set, else the public
//!   https URL ([`set_public_url`], fed by remote access), else
//!   `mailto:mother-claude@users.noreply.github.com`.
//!
//! Handlers live in `server/routes/push.rs`. AppState field: `state.push`.

mod endpoint;
mod engine;
mod payload;
mod store;
mod vapid;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::claude::model::{PendingItem, SessionSummary};
use crate::server::context::{ClientContext, Ingress};
use crate::server::error::{ApiError, ApiResult};
use crate::state::{AppState, Inner, ServerEvent};

pub use endpoint::{validate_endpoint, EndpointError, ALLOWED_HOSTS};
pub use engine::{PushEvent, PushKind};
pub use store::{Prefs, PrefsPatch, PrefsView, SubscribeOutcome, Subscription};

use endpoint::{HostLookup, PublicOnlyResolver, SystemLookup};
use engine::{Batch, Coalescer, SessionTracker, Suppressor};
use payload::Notification;
use store::PushFile;
use vapid::VapidKeys;

/// Default VAPID contact (Apple rejects `localhost` and `.invalid`).
pub const DEFAULT_CONTACT: &str = "mailto:mother-claude@users.noreply.github.com";
/// Optional override for the VAPID `sub` (`mailto:` or `https:`).
pub const SUBJECT_ENV: &str = "MOTHER_CLAUDE_VAPID_SUBJECT";
/// Per-request timeout for the push service.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Subscribe-time DNS check budget.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);
/// Minimum spacing of `/push/test` per device (no scanning through it).
const TEST_INTERVAL: Duration = Duration::from_secs(5);
/// Consecutive 400/401/403/413 answers before a subscription is dropped.
const MAX_REJECTIONS: u32 = 5;
/// Trigger queue; triggers drop (logged) rather than block when it is full.
const QUEUE: usize = 1024;

/// iOS guidance shown by the Settings page when not running as a Home Screen
/// app.
const IOS_HOME_SCREEN: &str = "On iPhone and iPad, notifications work only in the Home Screen \
app: tap Share, then Add to Home Screen, open Mother Claude from the Home Screen, pair it, and \
enable notifications there.";

enum Msg {
    Event(PushEvent),
    Forget(String),
}

/// Delivery health of one endpoint (memory only).
#[derive(Debug, Default, Clone, Copy)]
struct Health {
    rejections: u32,
    failures: u32,
    retry_after: Option<Instant>,
}

/// How pushes leave the machine.
struct Transport {
    client: reqwest::Client,
    lookup: Arc<dyn HostLookup>,
    /// Tests only: accept `http://127.0.0.1` mock endpoints.
    allow_insecure: bool,
}

impl Transport {
    fn production() -> Result<Self, String> {
        let lookup: Arc<dyn HostLookup> = Arc::new(SystemLookup);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(SEND_TIMEOUT)
            .connect_timeout(Duration::from_secs(5))
            .https_only(true)
            // A proxy would resolve the name itself and bypass the filter.
            .no_proxy()
            .dns_resolver(Arc::new(PublicOnlyResolver {
                lookup: lookup.clone(),
            }))
            .user_agent(concat!("mother-claude/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            lookup,
            allow_insecure: false,
        })
    }
}

/// VAPID keys, subscriptions, prefs and the trigger queue.
pub struct PushService {
    path: Option<PathBuf>,
    keys: Arc<VapidKeys>,
    data: Mutex<PushFile>,
    /// Serialises writes of `push.json` (the latest state always wins).
    write_lock: Mutex<()>,
    health: Mutex<HashMap<String, Health>>,
    tx: mpsc::Sender<Msg>,
    rx: Mutex<Option<mpsc::Receiver<Msg>>>,
    transport: Mutex<Option<Arc<Transport>>>,
    public_url: RwLock<Option<String>>,
    last_test: Mutex<HashMap<String, Instant>>,
    window_ms: std::sync::atomic::AtomicI64,
    #[cfg(test)]
    trust_devices: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for PushService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushService")
            .field("path", &self.path)
            .field("public_key", &self.keys.public_key())
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

impl PushService {
    /// Load (or create) the VAPID key and `push.json` from `data_dir`.
    /// Tolerant: missing or corrupt files mean empty. `None` ⇒ in-memory only.
    pub fn new(data_dir: Option<&Path>) -> Self {
        let keys = VapidKeys::load_or_create(data_dir);
        let path = data_dir.map(|d| d.join(store::FILE));
        let mut data: PushFile = path
            .as_deref()
            .and_then(crate::persist::read_json)
            .unwrap_or_default();
        let changed = data.sanitize(keys.public_key(), now_ms(), |s| {
            validate_endpoint(&s.endpoint).is_ok()
                && store::canonical_keys(&s.p256dh, &s.auth).is_ok()
        });
        let (tx, rx) = mpsc::channel(QUEUE);
        let svc = Self {
            path,
            keys: Arc::new(keys),
            data: Mutex::new(data),
            write_lock: Mutex::new(()),
            health: Mutex::new(HashMap::new()),
            tx,
            rx: Mutex::new(Some(rx)),
            transport: Mutex::new(None),
            public_url: RwLock::new(None),
            last_test: Mutex::new(HashMap::new()),
            window_ms: std::sync::atomic::AtomicI64::new(engine::WINDOW_MS),
            #[cfg(test)]
            trust_devices: std::sync::atomic::AtomicBool::new(false),
        };
        if changed {
            svc.persist();
        }
        svc
    }

    /// The `applicationServerKey` (base64url, uncompressed P-256 point).
    pub fn public_key(&self) -> &str {
        self.keys.public_key()
    }

    /// Whether any device has a subscription.
    pub fn has_subscriptions(&self) -> bool {
        lock(&self.data).has_subscriptions()
    }

    /// Number of subscriptions a device has.
    pub fn subscription_count(&self, device_id: &str) -> usize {
        lock(&self.data)
            .devices
            .get(device_id)
            .map_or(0, |d| d.subscriptions.len())
    }

    /// The device's prefs (defaults for an unknown device).
    pub fn prefs(&self, device_id: &str, available: bool) -> PrefsView {
        lock(&self.data).view(device_id, now_ms(), available)
    }

    /// Apply a prefs patch; 422 on invalid input (nothing applied).
    pub fn update_prefs(&self, device_id: &str, patch: PrefsPatch) -> ApiResult<PrefsView> {
        let now = now_ms();
        lock(&self.data)
            .apply_patch(device_id, patch, now)
            .map_err(|m| ApiError::unprocessable("invalid_push_prefs", m))?;
        self.persist();
        Ok(self.prefs(device_id, true))
    }

    /// Remove one of the device's subscriptions.
    pub fn unsubscribe(&self, device_id: &str, endpoint: &str) -> bool {
        let canonical = validate_endpoint(endpoint)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| endpoint.trim().to_string());
        let removed = lock(&self.data).unsubscribe(device_id, &canonical);
        if removed {
            lock(&self.health).remove(&canonical);
            self.persist();
            tracing::info!(
                device = device_id,
                host = %endpoint::log_host(&canonical),
                "push subscription removed"
            );
        }
        removed
    }

    /// Forget everything about a device (revoked or unknown).
    pub fn forget_device(&self, device_id: &str) {
        let removed = {
            let mut data = lock(&self.data);
            let endpoints: Vec<String> = data
                .devices
                .get(device_id)
                .map(|d| d.subscriptions.iter().map(|s| s.endpoint.clone()).collect())
                .unwrap_or_default();
            let removed = data.drop_device(device_id);
            let mut health = lock(&self.health);
            for e in endpoints {
                health.remove(&e);
            }
            removed
        };
        if removed {
            self.persist();
            tracing::info!(device = device_id, "push subscriptions dropped for device");
        }
    }

    /// The public URL (https origin) used as the VAPID contact, if known.
    pub fn public_url(&self) -> Option<String> {
        self.public_url
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The VAPID `sub` claim.
    pub fn contact(&self) -> String {
        if let Some(s) = std::env::var(SUBJECT_ENV)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| valid_subject(s))
        {
            return s;
        }
        self.public_url()
            .unwrap_or_else(|| DEFAULT_CONTACT.to_string())
    }

    fn enqueue(&self, msg: Msg) {
        if let Err(e) = self.tx.try_send(msg) {
            tracing::debug!(error = %e, "push trigger dropped (queue full or stopped)");
        }
    }

    fn transport(&self) -> Option<Arc<Transport>> {
        let mut t = lock(&self.transport);
        if t.is_none() {
            match Transport::production() {
                Ok(tr) => *t = Some(Arc::new(tr)),
                Err(e) => {
                    tracing::error!(error = %e, "could not build the push HTTP client");
                    return None;
                }
            }
        }
        t.clone()
    }

    fn persist(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let _w = lock(&self.write_lock);
        let snapshot = lock(&self.data).clone();
        if let Err(e) = crate::persist::write_json_private(path, &snapshot) {
            tracing::warn!(path = %path.display(), error = %e, "could not write push.json");
        }
    }

    fn wanted_by_any(&self, ev: &PushEvent, now: i64) -> bool {
        lock(&self.data)
            .devices
            .values()
            .any(|d| !d.subscriptions.is_empty() && engine::wants(d, ev, now))
    }

    /// Per-device selection for one batch; consumes fired one-shots.
    fn targets(&self, events: &[PushEvent], now: i64) -> Vec<Target> {
        let mut consumed = false;
        let mut out = Vec::new();
        {
            let mut data = lock(&self.data);
            let health = lock(&self.health);
            let instant = Instant::now();
            for (id, dev) in data.devices.iter_mut() {
                if dev.subscriptions.is_empty() {
                    continue;
                }
                let picked: Vec<PushEvent> = events
                    .iter()
                    .filter(|e| engine::wants(dev, e, now))
                    .cloned()
                    .collect();
                if picked.is_empty() {
                    continue;
                }
                for e in picked.iter().filter(|e| e.kind == PushKind::Finished) {
                    consumed |= dev.turn_finished.remove(&e.session_id).is_some();
                }
                let subs: Vec<Subscription> = dev
                    .subscriptions
                    .iter()
                    .filter(|s| {
                        health
                            .get(&s.endpoint)
                            .and_then(|h| h.retry_after)
                            .is_none_or(|t| t <= instant)
                    })
                    .cloned()
                    .collect();
                out.push(Target {
                    device: id.clone(),
                    show_details: dev.prefs.show_details,
                    events: picked,
                    subs,
                });
            }
        }
        if consumed {
            self.persist();
        }
        out
    }

    fn record(&self, endpoint: &str, outcome: &Outcome) {
        let host = endpoint::log_host(endpoint);
        let remove = {
            let mut health = lock(&self.health);
            match outcome {
                Outcome::Delivered => {
                    health.remove(endpoint);
                    false
                }
                Outcome::Gone => {
                    tracing::info!(%host, "push subscription expired (404/410); removing it");
                    true
                }
                Outcome::Invalid(why) => {
                    tracing::warn!(%host, reason = %why, "unusable push subscription; removing it");
                    true
                }
                Outcome::Rejected(code) => {
                    let h = health.entry(endpoint.to_string()).or_default();
                    h.rejections += 1;
                    tracing::warn!(%host, status = code, rejections = h.rejections, "push service rejected a notification");
                    h.rejections >= MAX_REJECTIONS
                }
                Outcome::RateLimited(secs) => {
                    let h = health.entry(endpoint.to_string()).or_default();
                    h.retry_after =
                        Some(Instant::now() + Duration::from_secs((*secs).clamp(1, 3600)));
                    tracing::info!(%host, retry_after = secs, "push service rate limited us");
                    false
                }
                Outcome::Failed(why) => {
                    let h = health.entry(endpoint.to_string()).or_default();
                    h.failures += 1;
                    tracing::debug!(%host, error = %why, failures = h.failures, "push delivery failed");
                    false
                }
            }
        };
        if remove {
            lock(&self.health).remove(endpoint);
            if lock(&self.data).remove_endpoint(endpoint).is_some() {
                self.persist();
            }
        }
    }

    /// Send one rendered notification to some of a device's subscriptions.
    /// Returns whether at least one was accepted.
    async fn send_to(&self, subs: &[Subscription], n: &Notification, topic: Option<&str>) -> bool {
        if subs.is_empty() {
            return false;
        }
        let Some(transport) = self.transport() else {
            return false;
        };
        let contact = self.contact();
        let body = payload::payload_json(n, now_ms()).to_string().into_bytes();
        let sends = subs.iter().map(|s| {
            send_one(
                transport.clone(),
                self.keys.clone(),
                contact.clone(),
                s.clone(),
                body.clone(),
                n.ttl_secs,
                n.urgency,
                topic.map(str::to_string),
            )
        });
        let results = futures_util::future::join_all(sends).await;
        let mut any = false;
        for (sub, outcome) in subs.iter().zip(results) {
            any |= matches!(outcome, Outcome::Delivered);
            self.record(&sub.endpoint, &outcome);
        }
        any
    }
}

struct Target {
    device: String,
    show_details: bool,
    events: Vec<PushEvent>,
    subs: Vec<Subscription>,
}

/// What the push service said.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Delivered,
    /// 404 / 410: the subscription is gone.
    Gone,
    /// We could not even build a request (bad keys, endpoint no longer valid).
    Invalid(String),
    /// 400 / 401 / 403 / 413 / 3xx.
    Rejected(u16),
    /// 429 with Retry-After seconds.
    RateLimited(u64),
    /// Network error or 5xx.
    Failed(String),
}

#[allow(clippy::too_many_arguments)]
async fn send_one(
    transport: Arc<Transport>,
    keys: Arc<VapidKeys>,
    contact: String,
    sub: Subscription,
    body: Vec<u8>,
    ttl_secs: u64,
    urgency: payload::Urgency,
    topic: Option<String>,
) -> Outcome {
    if !transport.allow_insecure {
        if let Err(e) = validate_endpoint(&sub.endpoint) {
            return Outcome::Invalid(e.message().to_string());
        }
    }
    let req = match payload::build_request(
        &keys,
        &contact,
        &sub,
        body,
        ttl_secs,
        urgency,
        topic.as_deref(),
    ) {
        Ok(r) => r,
        Err(e) => return Outcome::Invalid(e.0),
    };
    let req = match reqwest::Request::try_from(req) {
        Ok(r) => r,
        Err(e) => return Outcome::Invalid(e.without_url().to_string()),
    };
    match transport.client.execute(req).await {
        Ok(resp) => {
            let status = resp.status();
            let code = status.as_u16();
            if status.is_success() {
                Outcome::Delivered
            } else if code == 404 || code == 410 {
                Outcome::Gone
            } else if code == 429 {
                let secs = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .unwrap_or(60);
                Outcome::RateLimited(secs)
            } else if status.is_redirection() || matches!(code, 400 | 401 | 403 | 413) {
                Outcome::Rejected(code)
            } else {
                Outcome::Failed(format!("HTTP {code}"))
            }
        }
        Err(e) => Outcome::Failed(e.without_url().to_string()),
    }
}

/// `mailto:x@y.z` or an https URL.
fn valid_subject(s: &str) -> bool {
    if let Some(addr) = s.strip_prefix("mailto:") {
        let mut parts = addr.split('@');
        let (Some(user), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
            return false;
        };
        return !user.is_empty()
            && domain.contains('.')
            && !domain.ends_with(".invalid")
            && domain != "localhost"
            && !s.chars().any(|c| c.is_whitespace() || c.is_control());
    }
    https_origin(s).is_some()
}

/// The `https://host[:port]` origin of `url`, when it is an https URL on a
/// DNS name (not localhost, not an IP literal).
fn https_origin(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url.trim()).ok()?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    let host = u.host_str()?;
    if host.starts_with('[')
        || host.parse::<std::net::Ipv4Addr>().is_ok()
        || !endpoint::valid_dns_name(host)
        || host.ends_with(".localhost")
    {
        return None;
    }
    Some(match u.port() {
        Some(p) => format!("https://{host}:{p}"),
        None => format!("https://{host}"),
    })
}

/// A device may receive pushes only while it is paired and not revoked.
fn device_active(state: &AppState, device_id: &str) -> bool {
    #[cfg(test)]
    if state
        .push
        .trust_devices
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return true;
    }
    state.devices.get(device_id).is_some_and(|d| !d.revoked)
}

fn display_name(s: &SessionSummary) -> String {
    if !s.name.trim().is_empty() {
        s.name.clone()
    } else if !s.project_name.trim().is_empty() {
        s.project_name.clone()
    } else {
        "Claude session".into()
    }
}

// ─────────────────────────────── routes' logic ───────────────────────────────

/// `POST /api/push/subscribe` for `device_id`.
pub async fn subscribe(
    state: &AppState,
    device_id: &str,
    endpoint_raw: &str,
    p256dh: &str,
    auth: &str,
    application_server_key: Option<&str>,
) -> ApiResult<SubscribeOutcome> {
    let url = validate_endpoint(endpoint_raw)
        .map_err(|e| ApiError::unprocessable("invalid_push_endpoint", e.message()))?;
    let (p256dh, auth) = store::canonical_keys(p256dh, auth)
        .map_err(|m| ApiError::unprocessable("invalid_push_keys", m))?;
    if let Some(key) = application_server_key
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        let theirs = store::decode_b64(key);
        let ours = store::decode_b64(state.push.public_key());
        if theirs.is_none() || theirs != ours {
            return Err(ApiError::conflict(
                "vapid_key_mismatch",
                "This subscription was made with an old server key. Unsubscribe in the browser \
                 and enable notifications again.",
            )
            .with("publicKey", state.push.public_key().to_string()));
        }
    }
    let host = url.host_str().unwrap_or_default().to_string();
    let lookup = match state.push.transport() {
        Some(t) => t.lookup.clone(),
        None => Arc::new(SystemLookup) as Arc<dyn HostLookup>,
    };
    endpoint::check_resolution(lookup.as_ref(), &host, RESOLVE_TIMEOUT)
        .await
        .map_err(|e| ApiError::unprocessable("invalid_push_endpoint", e.message()))?;
    let sub = Subscription {
        endpoint: url.to_string(),
        p256dh,
        auth,
        ..Default::default()
    };
    let out = lock(&state.push.data).subscribe(device_id, sub, now_ms());
    lock(&state.push.health).remove(url.as_str());
    state.push.persist();
    tracing::info!(
        device = device_id,
        %host,
        added = out.added,
        evicted = out.evicted,
        moved = out.moved_from.is_some(),
        "push subscription stored"
    );
    Ok(out)
}

/// `POST /api/push/test` for `device_id`: send a test notification to every
/// subscription of the device. Returns only whether one was accepted (no
/// status codes: `/push/test` must not become a probe).
pub async fn send_test(state: &AppState, device_id: &str) -> ApiResult<bool> {
    {
        let mut last = lock(&state.push.last_test);
        let now = Instant::now();
        if let Some(t) = last.get(device_id) {
            let since = now.duration_since(*t);
            if since < TEST_INTERVAL {
                let wait = (TEST_INTERVAL - since).as_secs().max(1);
                return Err(ApiError::too_many_requests(
                    wait,
                    "Wait a few seconds before sending another test notification.",
                ));
            }
        }
        last.insert(device_id.to_string(), now);
        if last.len() > 1024 {
            last.retain(|_, t| now.duration_since(*t) < TEST_INTERVAL);
        }
    }
    let subs: Vec<Subscription> = lock(&state.push.data)
        .devices
        .get(device_id)
        .map(|d| d.subscriptions.clone())
        .unwrap_or_default();
    if subs.is_empty() {
        return Err(ApiError::conflict(
            "push_not_subscribed",
            "This device has no push subscription yet. Enable notifications first.",
        ));
    }
    let n = payload::render(
        None,
        "",
        &[PushEvent::new("", PushKind::Test, "test")],
        false,
    );
    Ok(state.push.send_to(&subs, &n, None).await)
}

/// Hints for the Settings page: can this origin do Web Push at all?
pub fn secure_context_hint(state: &AppState, ctx: &ClientContext) -> Value {
    let (code, text) = match ctx.ingress {
        _ if ctx.is_desktop() => (
            "desktop",
            "This Mac shows alerts in the app. Push notifications are for paired phones and \
             browsers.",
        ),
        Ingress::Tunnel => (
            "ok",
            "This address has a trusted certificate, so notifications can work here.",
        ),
        Ingress::Lan => (
            "lan_certificate",
            "Notifications need a trusted HTTPS address. On this local-network address they \
             work only if this device trusts Mother Claude's certificate; otherwise open the \
             internet address and enable notifications there.",
        ),
        Ingress::Loopback | Ingress::BridgeLoopback => (
            "ok",
            "Notifications can work when this page is served over trusted HTTPS.",
        ),
    };
    json!({
        "code": code,
        "text": text,
        "publicUrl": state.push.public_url(),
        "iosHomeScreen": IOS_HOME_SCREEN,
        "requiresUserGesture": true,
    })
}

// ─────────────────────────────── triggers ────────────────────────────────────

/// A new pending item was raised (called by `pending::raise` for every
/// channel).
pub fn on_pending_added(state: &AppState, item: &PendingItem) {
    if !state.push.has_subscriptions() {
        return;
    }
    state
        .push
        .enqueue(Msg::Event(PushEvent::from_pending(item)));
}

/// An owned/hooked session finished a turn after `worked_ms` of work (B3's
/// `result`, B4's Stop hook). Short turns only notify devices that armed
/// "notify me when this turn finishes".
pub fn on_turn_finished(state: &AppState, session_id: &str, worked_ms: u64) {
    if !state.push.has_subscriptions() {
        return;
    }
    let mut e = PushEvent::new(session_id, PushKind::Finished, "finished");
    e.worked_ms = Some(worked_ms);
    state.push.enqueue(Msg::Event(e));
}

/// A session failed or stopped on an API error. `text` is shown only to
/// devices with details on.
pub fn on_session_failed(state: &AppState, session_id: &str, text: &str) {
    if !state.push.has_subscriptions() {
        return;
    }
    let mut e = PushEvent::new(session_id, PushKind::Failed, "failed");
    e.detail = Some(text.to_string()).filter(|t| !t.trim().is_empty());
    state.push.enqueue(Msg::Event(e));
}

/// Claude called the `PushNotification` tool (owned stream or PreToolUse hook;
/// pass the tool-use id as `key` so the transcript copy is deduplicated) or a
/// hook reported a Claude-initiated notification. `message` is shown only to
/// devices with details on.
pub fn on_claude_notification(
    state: &AppState,
    session_id: &str,
    message: Option<&str>,
    key: Option<&str>,
) {
    if !state.push.has_subscriptions() {
        return;
    }
    let key = match key.filter(|k| !k.is_empty()) {
        Some(k) => format!("tool:{k}"),
        None => format!("claude:{}", uuid::Uuid::new_v4().simple()),
    };
    let mut e = PushEvent::new(session_id, PushKind::Claude, key);
    e.detail = message.map(str::to_string).filter(|m| !m.trim().is_empty());
    state.push.enqueue(Msg::Event(e));
}

/// A `Notification` hook arrived (B4). `permission_prompt` is covered by the
/// pending card and `auth_success` is noise; `idle_prompt` feeds the `idle`
/// pref; elicitation dialogs are a wait at the Mac; everything else is a
/// Claude notification (`claude` pref).
pub fn on_notification_hook(
    state: &AppState,
    session_id: &str,
    notification_type: Option<&str>,
    message: Option<&str>,
) {
    if !state.push.has_subscriptions() {
        return;
    }
    let ty = notification_type.unwrap_or_default();
    let e = match ty {
        "permission_prompt" | "auth_success" => return,
        "idle_prompt" => PushEvent::new(session_id, PushKind::Idle, "idle"),
        t if t.starts_with("elicitation") => {
            let mut e = PushEvent::new(session_id, PushKind::Waiting, "waiting");
            e.label = Some("input needed".into());
            e
        }
        _ => {
            let mut e = PushEvent::new(
                session_id,
                PushKind::Claude,
                format!("claude:{}", uuid::Uuid::new_v4().simple()),
            );
            e.detail = message.map(str::to_string).filter(|m| !m.trim().is_empty());
            e
        }
    };
    state.push.enqueue(Msg::Event(e));
}

/// Forget a revoked device's subscriptions (B1 calls this on revoke).
pub fn drop_device(state: &AppState, device_id: &str) {
    state.push.forget_device(device_id);
}

/// Alias of [`drop_device`].
pub fn on_device_revoked(state: &AppState, device_id: &str) {
    drop_device(state, device_id);
}

/// The public https URL (tunnel / manual) changed; used as the VAPID contact.
/// Anything that is not an https URL on a DNS name clears it.
pub fn set_public_url(state: &AppState, url: Option<&str>) {
    let origin = url.and_then(https_origin);
    *state
        .push
        .public_url
        .write()
        .unwrap_or_else(|p| p.into_inner()) = origin;
}

// ─────────────────────────────── background ──────────────────────────────────

/// Start the trigger engine and the bus observer. Called once from
/// `server::serve`; later calls are no-ops.
pub fn start(state: &AppState) {
    if tokio::runtime::Handle::try_current().is_err() {
        tracing::warn!("push::start called outside a Tokio runtime; push stays off");
        return;
    }
    let Some(rx) = lock(&state.push.rx).take() else {
        return;
    };
    let window = state
        .push
        .window_ms
        .load(std::sync::atomic::Ordering::Relaxed);
    let weak = Arc::downgrade(state);
    tokio::spawn(engine_loop(weak.clone(), rx, window));
    tokio::spawn(watch_bus(weak, state.bus.subscribe()));
}

/// Consume trigger messages: suppress duplicates, coalesce per session, and
/// spawn deliveries. Holds only a `Weak` so the state can drop.
async fn engine_loop(weak: Weak<Inner>, mut rx: mpsc::Receiver<Msg>, window_ms: i64) {
    let t0 = Instant::now();
    let clock = move || t0.elapsed().as_millis() as i64;
    let mut suppressor = Suppressor::default();
    let mut coalescer = Coalescer::new(window_ms);
    loop {
        let next = match coalescer.next_deadline() {
            None => match rx.recv().await {
                Some(m) => Some(m),
                None => break,
            },
            Some(deadline) => {
                let wait = Duration::from_millis((deadline - clock()).max(0) as u64);
                tokio::select! {
                    m = rx.recv() => match m {
                        Some(m) => Some(m),
                        None => break,
                    },
                    _ = tokio::time::sleep(wait) => None,
                }
            }
        };
        let Some(state) = weak.upgrade() else {
            break;
        };
        let now = clock();
        for batch in coalescer.due(now) {
            tokio::spawn(deliver(state.clone(), batch));
        }
        match next {
            Some(Msg::Forget(session)) => suppressor.forget(&session),
            Some(Msg::Event(ev)) => {
                tracing::debug!(
                    session = %ev.session_id,
                    kind = ?ev.kind,
                    derived = ev.derived,
                    "push trigger"
                );
                let has_answerable = ev.kind == PushKind::Waiting
                    && state
                        .pending
                        .list(&ev.session_id)
                        .iter()
                        .any(|p| p.answerable);
                if !suppressor.admit(now, &ev, has_answerable) {
                    continue;
                }
                if !state.push.wanted_by_any(&ev, now_ms()) {
                    continue;
                }
                if let Some(batch) = coalescer.offer(now, ev) {
                    tokio::spawn(deliver(state.clone(), batch));
                }
            }
            None => {}
        }
    }
    tracing::debug!("push engine stopped");
}

/// Derive events from the bus (`sessions` diffs, `transcript` appends).
async fn watch_bus(weak: Weak<Inner>, mut rx: tokio::sync::broadcast::Receiver<Arc<ServerEvent>>) {
    let mut tracker = SessionTracker::default();
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(RecvError::Lagged(n)) => {
                tracing::debug!(skipped = n, "push bus observer lagged");
                continue;
            }
            Err(RecvError::Closed) => break,
        };
        let (events, gone) = tracker.observe(&ev, now_ms());
        if events.is_empty() && gone.is_empty() {
            continue;
        }
        let Some(state) = weak.upgrade() else {
            break;
        };
        for g in gone {
            state.push.enqueue(Msg::Forget(g));
        }
        if !events.is_empty() && state.push.has_subscriptions() {
            for e in events {
                state.push.enqueue(Msg::Event(e));
            }
        }
    }
}

/// Render and send one batch to every device that wants it.
async fn deliver(state: AppState, batch: Batch) {
    // Drop cards answered or withdrawn in the meantime (trailing batches).
    let events: Vec<PushEvent> = batch
        .events
        .into_iter()
        .filter(|e| {
            e.pending_id
                .as_deref()
                .is_none_or(|id| state.pending.get(id).is_some())
        })
        .collect();
    if events.is_empty() {
        return;
    }
    let name = state
        .find_session(&batch.session_id)
        .await
        .map(|s| display_name(&s));
    let topic = payload::topic_for(&batch.session_id);
    for target in state.push.targets(&events, now_ms()) {
        if !device_active(&state, &target.device) {
            tracing::info!(device = %target.device, "dropping push subscriptions of an unknown or revoked device");
            state.push.forget_device(&target.device);
            continue;
        }
        if state.presence.is_foreground(&target.device) {
            tracing::debug!(device = %target.device, "device is looking at the app; no push");
            continue;
        }
        let n = payload::render(
            name.as_deref(),
            &batch.session_id,
            &target.events,
            target.show_details,
        );
        state.push.send_to(&target.subs, &n, Some(&topic)).await;
    }
}

// ─────────────────────────────── test seams ──────────────────────────────────

/// A browser-side key pair and a matching subscription (tests).
#[cfg(test)]
pub(crate) fn payload_test_subscription() -> (web_push_native::p256::SecretKey, Subscription) {
    payload::tests::user_agent()
}

#[cfg(test)]
struct FixedDns(std::net::IpAddr);

#[cfg(test)]
impl HostLookup for FixedDns {
    fn lookup(&self, _host: String) -> endpoint::BoxFut<std::io::Result<Vec<std::net::IpAddr>>> {
        let ip = self.0;
        Box::pin(async move { Ok(vec![ip]) })
    }
}

#[cfg(test)]
impl PushService {
    fn install_test_transport(&self, ip: &str, allow_insecure: bool) {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(SEND_TIMEOUT)
            .no_proxy()
            .build()
            .unwrap();
        *lock(&self.transport) = Some(Arc::new(Transport {
            client,
            lookup: Arc::new(FixedDns(ip.parse().unwrap())),
            allow_insecure,
        }));
    }

    /// Send over plain HTTP to local mock endpoints, resolve every name to a
    /// public address, and treat every device as paired.
    fn use_test_transport(&self) {
        self.install_test_transport("142.250.74.10", true);
        self.trust_devices
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn set_window_ms(&self, ms: i64) {
        self.window_ms
            .store(ms, std::sync::atomic::Ordering::Relaxed);
    }

    /// Store a subscription without endpoint validation (mock endpoints).
    fn insert_unchecked(&self, device: &str, sub: Subscription) {
        lock(&self.data).subscribe(device, sub, now_ms());
    }

    fn device_endpoints(&self, device: &str) -> Vec<String> {
        lock(&self.data)
            .devices
            .get(device)
            .map(|d| d.subscriptions.iter().map(|s| s.endpoint.clone()).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
