//! Paired devices and their tokens (design §2.5, §2.6, §10 B1).
//!
//! * `devices.json` in the app data dir (0600, atomic write via
//!   `crate::persist::write_json_private`):
//!   `{devices:[{id,name,platform,userAgent,tokenSha256,extraTokenSha256,
//!   createdAt,lastSeenAt,lastIp,ingress,pairedVia,allowDangerous,
//!   allowDangerousInternet,revoked,revokedAt,revokeReason}], pending:[]}`.
//!   Only SHA-256 hashes of `mcd_…` tokens are stored; lookups compare the
//!   hex digests in constant time. Pending pairings are in memory only
//!   (`server::pairing`).
//! * [`DeviceStore::authenticate`] resolves a token to an unrevoked, unexpired
//!   device and the grant for the request's effective ingress: `allowDangerous`
//!   on the LAN, `allowDangerousInternet` everywhere else. It updates lastSeen /
//!   lastIp / ingress in memory and writes them to disk at most once a minute.
//! * Devices idle for more than `idleDays` (default 30) expire.
//! * [`revoke`] closes the device's WebSockets, drops its push subscriptions
//!   and tells desktop clients to refetch.
//! * An append-only audit log (`audit.jsonl`, [`audit`]) records every
//!   non-desktop state change; each device keeps its recent actions in memory
//!   for the Remote page.
//!
//! AppState field: `state.devices` ([`DeviceStore::load`] from
//! `Auth::config_dir`; `None` = in-memory only, for tests).

pub mod audit;

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::auth::{self, DEVICE_PREFIX};
use super::context::Ingress;
use crate::claude::model::NoticeLevel;
use crate::state::AppState;

/// Default idle expiry.
pub const DEFAULT_IDLE_DAYS: u32 = 30;
/// Longest accepted device name.
pub const MAX_NAME_CHARS: usize = 64;
/// Recent actions kept per device (in memory).
const RECENT_PER_DEVICE: usize = 20;
/// lastSeen/lastIp are flushed to disk at most this often.
const FLUSH_EVERY: Duration = Duration::from_secs(60);
const DAY_MS: i64 = 86_400_000;

/// What authentication learns about a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuth {
    pub id: String,
    pub name: String,
    /// The grant for the ingress this request arrived on.
    pub allow_dangerous: bool,
}

/// One recent action by a device (the Remote page's activity list).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecentAction {
    pub at: i64,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dangerous: bool,
}

/// A paired device as the desktop's device list shows it (never the token or
/// its hash).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ip: Option<String>,
    /// Effective ingress of the last request (`lan` | `tunnel` | …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<String>,
    /// Effective ingress the device was paired through.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paired_via: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Dangerous actions over the LAN.
    pub allow_dangerous: bool,
    /// Dangerous actions over a tunnel / proxy / anything but the LAN.
    pub allow_dangerous_internet: bool,
    pub revoked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<i64>,
    /// `revoked` | `unpaired` | `expired`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoke_reason: Option<String>,
    /// When an idle device will expire (unrevoked devices only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Tokens held (the Home Screen app adds a second one).
    pub token_count: usize,
    /// Open WebSockets right now (filled by the route).
    pub sockets: usize,
    /// Most recent first.
    pub recent: Vec<RecentAction>,
}

/// On-disk record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct DeviceRecord {
    id: String,
    name: String,
    platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<String>,
    token_sha256: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    extra_token_sha256: Vec<String>,
    created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seen_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ingress: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paired_via: Option<String>,
    allow_dangerous: bool,
    allow_dangerous_internet: bool,
    revoked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoke_reason: Option<String>,
    /// Unknown fields survive a round trip.
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

impl DeviceRecord {
    fn hashes(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.token_sha256).chain(self.extra_token_sha256.iter())
    }

    fn matches(&self, hash: &str) -> bool {
        // Constant time per comparison; every hash is checked.
        let mut hit = false;
        for h in self.hashes() {
            hit |= !h.is_empty() && auth::token_matches(h, hash);
        }
        hit
    }

    fn idle_since(&self) -> i64 {
        self.last_seen_at
            .unwrap_or(self.created_at)
            .max(self.created_at)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct DevicesFile {
    devices: Vec<DeviceRecord>,
    /// Pending pairings live in memory; kept for the documented shape.
    pending: Vec<Value>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

#[derive(Debug)]
struct Inner {
    file: DevicesFile,
    dirty: bool,
    last_flush: Instant,
    idle_days: u32,
    recent: HashMap<String, VecDeque<RecentAction>>,
}

/// The device registry.
#[derive(Debug)]
pub struct DeviceStore {
    /// `<app data dir>/devices.json`, or `None` for an in-memory store.
    pub path: Option<PathBuf>,
    inner: Mutex<Inner>,
    /// `<app data dir>/audit.jsonl` (in memory only without a data dir).
    pub audit: audit::AuditLog,
    /// Bumped whenever a device is revoked or forgotten, so long-running
    /// responses opened with its token can end ([`DeviceStore::until_revoked`]).
    revocations: tokio::sync::watch::Sender<u64>,
}

impl Default for DeviceStore {
    fn default() -> Self {
        Self::load(None)
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Strip control characters, collapse whitespace, cap the length.
pub fn sanitize_label(raw: &str, max_chars: usize) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max_chars).collect()
}

fn device_name(raw: &str) -> String {
    let n = sanitize_label(raw, MAX_NAME_CHARS);
    if n.is_empty() {
        "Unnamed device".to_string()
    } else {
        n
    }
}

impl DeviceStore {
    /// Load `devices.json` from `data_dir` (tolerant: missing or corrupt ⇒
    /// empty). `None` ⇒ in-memory only.
    pub fn load(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join("devices.json"));
        let mut file: DevicesFile = path
            .as_deref()
            .and_then(crate::persist::read_json)
            .unwrap_or_default();
        // Tolerant: drop records without an id rather than failing the load.
        file.devices.retain(|d| !d.id.is_empty());
        file.pending.clear();
        let idle_days = std::env::var("MOTHER_CLAUDE_DEVICE_IDLE_DAYS")
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|d| *d > 0)
            .unwrap_or(DEFAULT_IDLE_DAYS);
        Self {
            audit: audit::AuditLog::new(data_dir.map(|d| d.join("audit.jsonl"))),
            revocations: tokio::sync::watch::channel(0).0,
            path,
            inner: Mutex::new(Inner {
                file,
                dirty: false,
                last_flush: Instant::now(),
                idle_days,
                recent: HashMap::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn persist_locked(&self, g: &mut Inner) {
        g.dirty = false;
        g.last_flush = Instant::now();
        let Some(path) = &self.path else { return };
        if let Err(e) = crate::persist::write_json_private(path, &g.file) {
            tracing::warn!(path = %path.display(), error = %e, "could not save devices.json");
        }
    }

    /// Write pending lastSeen/lastIp changes now (shutdown, tests).
    pub fn flush(&self) {
        let mut g = self.lock();
        if g.dirty {
            self.persist_locked(&mut g);
        }
    }

    fn info(g: &Inner, d: &DeviceRecord) -> DeviceInfo {
        let expires_at = (!d.revoked).then(|| d.idle_since() + i64::from(g.idle_days) * DAY_MS);
        DeviceInfo {
            id: d.id.clone(),
            name: d.name.clone(),
            platform: d.platform.clone(),
            created_at: d.created_at,
            last_seen_at: d.last_seen_at,
            last_ip: d.last_ip.clone(),
            ingress: d.ingress.clone(),
            paired_via: d.paired_via.clone(),
            user_agent: d.user_agent.clone(),
            allow_dangerous: d.allow_dangerous,
            allow_dangerous_internet: d.allow_dangerous_internet,
            revoked: d.revoked,
            revoked_at: d.revoked_at,
            revoke_reason: d.revoke_reason.clone(),
            expires_at,
            token_count: if d.revoked {
                0
            } else {
                d.hashes().filter(|h| !h.is_empty()).count()
            },
            sockets: 0,
            recent: g
                .recent
                .get(&d.id)
                .map(|q| q.iter().rev().cloned().collect())
                .unwrap_or_default(),
        }
    }

    /// Resolve an `mcd_…` token presented from `client_ip` on the request's
    /// (effective) `ingress`. `None` for unknown, revoked or expired devices.
    pub fn authenticate(
        &self,
        token: &str,
        client_ip: IpAddr,
        ingress: Ingress,
    ) -> Option<DeviceAuth> {
        if !token.starts_with(DEVICE_PREFIX) {
            return None;
        }
        let hash = auth::sha256_hex(token);
        let now = now_ms();
        let mut g = self.lock();
        let idle_ms = i64::from(g.idle_days) * DAY_MS;
        let mut found = None;
        for (i, d) in g.file.devices.iter().enumerate() {
            if d.matches(&hash) && found.is_none() {
                found = Some(i);
            }
        }
        let i = found?;
        let d = &mut g.file.devices[i];
        if d.revoked {
            return None;
        }
        if now - d.idle_since() > idle_ms {
            return None;
        }
        let ip = client_ip.to_string();
        let ingress_label = ingress.as_str().to_string();
        d.last_seen_at = Some(now);
        d.last_ip = Some(ip);
        d.ingress = Some(ingress_label);
        let out = DeviceAuth {
            id: d.id.clone(),
            name: d.name.clone(),
            allow_dangerous: if ingress == Ingress::Lan {
                d.allow_dangerous
            } else {
                d.allow_dangerous_internet
            },
        };
        g.dirty = true;
        if g.last_flush.elapsed() >= FLUSH_EVERY {
            self.persist_locked(&mut g);
        }
        Some(out)
    }

    /// One device by id.
    pub fn get(&self, id: &str) -> Option<DeviceInfo> {
        let g = self.lock();
        g.file
            .devices
            .iter()
            .find(|d| d.id == id)
            .map(|d| Self::info(&g, d))
    }

    /// Every device (including revoked ones, flagged), newest first.
    pub fn list(&self) -> Vec<DeviceInfo> {
        let g = self.lock();
        let mut out: Vec<DeviceInfo> = g.file.devices.iter().map(|d| Self::info(&g, d)).collect();
        out.sort_by(|a, b| {
            a.revoked
                .cmp(&b.revoked)
                .then(b.created_at.cmp(&a.created_at))
        });
        out
    }

    /// Unrevoked devices.
    pub fn active_count(&self) -> usize {
        self.lock()
            .file
            .devices
            .iter()
            .filter(|d| !d.revoked)
            .count()
    }

    /// Create a device and return it with its token (shown exactly once).
    pub fn issue(
        &self,
        name: &str,
        platform: &str,
        user_agent: Option<&str>,
        paired_via: Ingress,
        client_ip: IpAddr,
    ) -> (DeviceInfo, String) {
        let token = auth::random_token(DEVICE_PREFIX);
        let now = now_ms();
        let record = DeviceRecord {
            id: format!("dev_{}", uuid::Uuid::new_v4().simple()),
            name: device_name(name),
            platform: sanitize_label(platform, 32),
            user_agent: user_agent
                .map(|u| sanitize_label(u, 200))
                .filter(|u| !u.is_empty()),
            token_sha256: auth::sha256_hex(&token),
            created_at: now,
            last_seen_at: Some(now),
            last_ip: Some(client_ip.to_string()),
            ingress: Some(paired_via.as_str().to_string()),
            paired_via: Some(paired_via.as_str().to_string()),
            ..Default::default()
        };
        let mut g = self.lock();
        g.file.devices.push(record.clone());
        self.persist_locked(&mut g);
        let info = Self::info(&g, &record);
        (info, token)
    }

    /// Give an existing, unrevoked device a second token (the iOS Home Screen
    /// app has its own storage). Revoking the device kills every token.
    pub fn add_token(&self, id: &str) -> Option<String> {
        let token = auth::random_token(DEVICE_PREFIX);
        let mut g = self.lock();
        let d = g
            .file
            .devices
            .iter_mut()
            .find(|d| d.id == id && !d.revoked)?;
        d.extra_token_sha256.push(auth::sha256_hex(&token));
        // Keep the list short: at most 4 extra tokens (oldest dropped).
        if d.extra_token_sha256.len() > 4 {
            d.extra_token_sha256.remove(0);
        }
        d.last_seen_at = Some(now_ms());
        self.persist_locked(&mut g);
        Some(token)
    }

    /// Rename an unrevoked device.
    pub fn rename(&self, id: &str, name: &str) -> Option<DeviceInfo> {
        let mut g = self.lock();
        let d = g
            .file
            .devices
            .iter_mut()
            .find(|d| d.id == id && !d.revoked)?;
        d.name = device_name(name);
        let d = d.clone();
        self.persist_locked(&mut g);
        Some(Self::info(&g, &d))
    }

    /// Change a device's dangerous grants (`None` leaves one as it is).
    pub fn set_grants(
        &self,
        id: &str,
        lan: Option<bool>,
        internet: Option<bool>,
    ) -> Option<DeviceInfo> {
        let mut g = self.lock();
        let d = g
            .file
            .devices
            .iter_mut()
            .find(|d| d.id == id && !d.revoked)?;
        if let Some(v) = lan {
            d.allow_dangerous = v;
        }
        if let Some(v) = internet {
            d.allow_dangerous_internet = v;
        }
        let d = d.clone();
        self.persist_locked(&mut g);
        Some(Self::info(&g, &d))
    }

    /// Mark a device revoked (its token hashes are erased). `false` when it
    /// does not exist or was already revoked. Prefer [`revoke`], which also
    /// closes its sockets and drops its push subscriptions.
    pub fn revoke_record(&self, id: &str, reason: &str) -> bool {
        let mut g = self.lock();
        let Some(d) = g.file.devices.iter_mut().find(|d| d.id == id && !d.revoked) else {
            return false;
        };
        d.revoked = true;
        d.revoked_at = Some(now_ms());
        d.revoke_reason = Some(reason.to_string());
        d.token_sha256.clear();
        d.extra_token_sha256.clear();
        d.allow_dangerous = false;
        d.allow_dangerous_internet = false;
        self.persist_locked(&mut g);
        drop(g);
        self.revocations.send_modify(|n| *n = n.wrapping_add(1));
        true
    }

    /// Whether `id` is a known, unrevoked device.
    pub fn is_active(&self, id: &str) -> bool {
        self.lock()
            .file
            .devices
            .iter()
            .any(|d| d.id == id && !d.revoked)
    }

    /// Completes once device `id` is revoked or forgotten (at once if it
    /// already is). Long-running responses opened with its token race this.
    pub async fn until_revoked(&self, id: &str) {
        let mut rx = self.revocations.subscribe();
        loop {
            if !self.is_active(id) {
                return;
            }
            if rx.changed().await.is_err() {
                // The store is going away; nothing will be revoked any more.
                std::future::pending::<()>().await;
            }
        }
    }

    /// Forget a revoked device entirely.
    pub fn forget(&self, id: &str) -> bool {
        let mut g = self.lock();
        let before = g.file.devices.len();
        g.file.devices.retain(|d| !(d.id == id && d.revoked));
        let removed = g.file.devices.len() != before;
        if removed {
            g.recent.remove(id);
            self.persist_locked(&mut g);
            drop(g);
            self.revocations.send_modify(|n| *n = n.wrapping_add(1));
        }
        removed
    }

    pub fn idle_days(&self) -> u32 {
        self.lock().idle_days
    }

    pub fn set_idle_days(&self, days: u32) {
        self.lock().idle_days = days.clamp(1, 3650);
    }

    /// Ids of unrevoked devices idle for longer than `idleDays`.
    pub fn expired_ids(&self) -> Vec<String> {
        let now = now_ms();
        let g = self.lock();
        let idle_ms = i64::from(g.idle_days) * DAY_MS;
        g.file
            .devices
            .iter()
            .filter(|d| !d.revoked && now - d.idle_since() > idle_ms)
            .map(|d| d.id.clone())
            .collect()
    }

    /// Remember one action for the device's activity list.
    pub fn note_action(&self, id: &str, action: RecentAction) {
        let mut g = self.lock();
        if !g.file.devices.iter().any(|d| d.id == id) {
            return;
        }
        let q = g.recent.entry(id.to_string()).or_default();
        q.push_back(action);
        while q.len() > RECENT_PER_DEVICE {
            q.pop_front();
        }
    }

    /// Test helper: a device and its token, without the pairing flow.
    #[cfg(test)]
    pub(crate) fn issue_for_test(&self, name: &str) -> (String, String) {
        let (info, token) = self.issue(
            name,
            "test",
            None,
            Ingress::Lan,
            IpAddr::from([192, 168, 1, 50]),
        );
        (info.id, token)
    }
}

/// Revoke a device: erase its tokens, close its WebSockets (4401), drop its
/// push subscriptions, tell desktop clients and record it. `false` when there
/// was nothing to revoke.
pub fn revoke(state: &AppState, id: &str, reason: &str) -> bool {
    let name = state.devices.get(id).map(|d| d.name);
    if !state.devices.revoke_record(id, reason) {
        return false;
    }
    let closed = crate::server::ws::close_device(state, id);
    crate::server::push::drop_device(state, id);
    crate::server::ws::publish_devices(state);
    state.devices.audit.record(audit::AuditEntry {
        action: format!("device.{reason}"),
        device_id: Some(id.to_string()),
        device_name: name,
        detail: Some(serde_json::json!({ "socketsClosed": closed })),
        ..audit::AuditEntry::now()
    });
    true
}

/// Expire idle devices (called periodically by `remote::start`).
pub fn sweep_expired(state: &AppState) {
    for id in state.devices.expired_ids() {
        let name = state.devices.get(&id).map(|d| d.name).unwrap_or_default();
        if revoke(state, &id, "expired") {
            crate::server::ws::publish_desktop_notice(
                state,
                NoticeLevel::Info,
                format!("\"{name}\" was unpaired after being idle too long."),
            );
        }
    }
    state.devices.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lan_ip() -> IpAddr {
        IpAddr::from([192, 168, 1, 9])
    }

    #[test]
    fn issue_authenticate_and_revoke() {
        let s = DeviceStore::load(None);
        let (info, token) = s.issue(
            "  My\u{7}  iPhone\n",
            "ios",
            Some("UA"),
            Ingress::Lan,
            lan_ip(),
        );
        assert_eq!(info.name, "My iPhone");
        assert!(token.starts_with(DEVICE_PREFIX));
        assert_eq!(info.token_count, 1);

        let a = s.authenticate(&token, lan_ip(), Ingress::Lan).unwrap();
        assert_eq!(a.id, info.id);
        assert!(!a.allow_dangerous);
        assert!(s.authenticate("mcd_nope", lan_ip(), Ingress::Lan).is_none());
        assert!(s
            .authenticate("not-a-device", lan_ip(), Ingress::Lan)
            .is_none());

        // Grants are per ingress.
        s.set_grants(&info.id, Some(true), None).unwrap();
        assert!(
            s.authenticate(&token, lan_ip(), Ingress::Lan)
                .unwrap()
                .allow_dangerous
        );
        assert!(
            !s.authenticate(&token, lan_ip(), Ingress::Tunnel)
                .unwrap()
                .allow_dangerous
        );
        assert!(
            !s.authenticate(&token, lan_ip(), Ingress::Loopback)
                .unwrap()
                .allow_dangerous
        );
        s.set_grants(&info.id, Some(false), Some(true)).unwrap();
        assert!(
            !s.authenticate(&token, lan_ip(), Ingress::Lan)
                .unwrap()
                .allow_dangerous
        );
        assert!(
            s.authenticate(&token, lan_ip(), Ingress::Tunnel)
                .unwrap()
                .allow_dangerous
        );

        let seen = s.get(&info.id).unwrap();
        assert_eq!(seen.last_ip.as_deref(), Some("192.168.1.9"));
        assert_eq!(seen.ingress.as_deref(), Some("tunnel"));

        // A second token for the same device; revoking kills both.
        let second = s.add_token(&info.id).unwrap();
        assert_eq!(
            s.authenticate(&second, lan_ip(), Ingress::Lan).unwrap().id,
            info.id
        );
        assert_eq!(s.get(&info.id).unwrap().token_count, 2);
        assert!(s.revoke_record(&info.id, "revoked"));
        assert!(!s.revoke_record(&info.id, "revoked"));
        assert!(s.authenticate(&token, lan_ip(), Ingress::Lan).is_none());
        assert!(s.authenticate(&second, lan_ip(), Ingress::Lan).is_none());
        let revoked = s.get(&info.id).unwrap();
        assert!(revoked.revoked && !revoked.allow_dangerous_internet);
        assert!(s.add_token(&info.id).is_none());
        assert!(s.forget(&info.id));
        assert!(s.get(&info.id).is_none());
    }

    #[test]
    fn idle_devices_expire() {
        let s = DeviceStore::load(None);
        let (info, token) = s.issue("old", "", None, Ingress::Lan, lan_ip());
        {
            let mut g = s.lock();
            let d = g.file.devices.iter_mut().find(|d| d.id == info.id).unwrap();
            d.created_at -= 40 * DAY_MS;
            d.last_seen_at = Some(d.created_at);
        }
        assert_eq!(s.expired_ids(), vec![info.id.clone()]);
        assert!(s.authenticate(&token, lan_ip(), Ingress::Lan).is_none());
        s.set_idle_days(60);
        assert!(s.expired_ids().is_empty());
        assert!(s.authenticate(&token, lan_ip(), Ingress::Lan).is_some());
    }

    #[test]
    fn persisted_privately_with_hashes_only() {
        let dir = tempfile::tempdir().unwrap();
        let s = DeviceStore::load(Some(dir.path()));
        let (info, token) = s.issue("Pixel", "android", None, Ingress::Tunnel, lan_ip());
        let text = std::fs::read_to_string(dir.path().join("devices.json")).unwrap();
        assert!(!text.contains(&token), "only hashes on disk");
        assert!(text.contains(&auth::sha256_hex(&token)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("devices.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        // Reload: same device, same token works; unknown fields survive.
        let mut v: Value = serde_json::from_str(&text).unwrap();
        v["devices"][0]["futureField"] = serde_json::json!(7);
        std::fs::write(dir.path().join("devices.json"), v.to_string()).unwrap();
        let again = DeviceStore::load(Some(dir.path()));
        assert_eq!(
            again
                .authenticate(&token, lan_ip(), Ingress::Lan)
                .unwrap()
                .id,
            info.id
        );
        again.rename(&info.id, "Renamed").unwrap();
        let text = std::fs::read_to_string(dir.path().join("devices.json")).unwrap();
        assert!(text.contains("futureField"));
        // Corrupt file: tolerant empty store.
        std::fs::write(dir.path().join("devices.json"), "{nope").unwrap();
        assert!(DeviceStore::load(Some(dir.path())).list().is_empty());
    }

    #[test]
    fn recent_actions_are_capped_and_newest_first() {
        let s = DeviceStore::load(None);
        let (info, _) = s.issue("p", "", None, Ingress::Lan, lan_ip());
        for i in 0..30 {
            s.note_action(
                &info.id,
                RecentAction {
                    at: i,
                    action: format!("a{i}"),
                    ..Default::default()
                },
            );
        }
        let recent = s.get(&info.id).unwrap().recent;
        assert_eq!(recent.len(), RECENT_PER_DEVICE);
        assert_eq!(recent[0].action, "a29");
        s.note_action("unknown", RecentAction::default());
    }

    #[test]
    fn labels_are_sanitized() {
        assert_eq!(sanitize_label("a\u{1b}[31mb", 64), "a [31mb");
        assert_eq!(sanitize_label(&"x".repeat(100), 64).len(), 64);
        assert_eq!(device_name("\n\t"), "Unnamed device");
    }
}
