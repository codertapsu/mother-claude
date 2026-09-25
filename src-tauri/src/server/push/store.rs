//! `push.json` (0600, app data dir): per-device subscriptions and notification
//! preferences. Pure data + operations; persistence and locking live in the
//! parent module.
//!
//! ```json
//! { "version": 1, "vapidPublicKey": "BN…",
//!   "devices": { "<deviceId>": {
//!       "prefs": {"permission":true,"question":true,"plan":true,"finished":true,
//!                 "failed":true,"claude":true,"idle":false,"showDetails":false},
//!       "sessions": {"<sessionId>": "follow"|"mute"},
//!       "turnFinished": {"<sessionId>": <expires ms>},
//!       "subscriptions": [{"endpoint","p256dh","auth","createdAt","updatedAt"}] } } }
//! ```
//!
//! Every field is tolerant (`default`); unknown session modes are ignored.

use std::collections::BTreeMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// File name inside the app data dir.
pub const FILE: &str = "push.json";
pub const VERSION: u32 = 1;
/// Subscriptions kept per device (the oldest is replaced beyond this).
pub const MAX_SUBSCRIPTIONS_PER_DEVICE: usize = 5;
/// Per-session follow/mute entries per device.
pub const MAX_SESSION_MODES: usize = 500;
/// Pending "notify me when this turn finishes" one-shots per device.
pub const MAX_TURN_ONCE: usize = 50;
/// A one-shot that never fired expires after a day.
pub const TURN_ONCE_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// Which events a device wants pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// A permission prompt waits (or a session waits at the Mac).
    pub permission: bool,
    /// Claude asked a question (AskUserQuestion, MCP elicitation).
    pub question: bool,
    /// A plan waits for review.
    pub plan: bool,
    /// A turn finished after ≥ 30 s of work.
    pub finished: bool,
    /// A session failed / stopped on an error.
    pub failed: bool,
    /// Claude decided to notify (PushNotification tool, Notification hooks).
    pub claude: bool,
    /// "Claude is waiting for your input" (idle_prompt), off by default.
    pub idle: bool,
    /// Include tool input / question / plan / message text in the
    /// notification. Off by default: notification text shows on lock screens
    /// and passes through Apple/Google/Mozilla.
    pub show_details: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            permission: true,
            question: true,
            plan: true,
            finished: true,
            failed: true,
            claude: true,
            idle: false,
            show_details: false,
        }
    }
}

/// Per-session override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    /// Every event of this session, whatever the per-event prefs say.
    Follow,
    /// Nothing from this session (except an explicit one-shot).
    Mute,
}

impl SessionMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "follow" => Some(SessionMode::Follow),
            "mute" => Some(SessionMode::Mute),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SessionMode::Follow => "follow",
            SessionMode::Mute => "mute",
        }
    }
}

/// One browser push subscription (`PushSubscription.toJSON()`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Subscription {
    /// Canonical https URL on an allowlisted push service. A capability:
    /// never log it (log `endpoint::log_host`).
    pub endpoint: String,
    /// base64url (no padding) of the 65-byte P-256 point.
    pub p256dh: String,
    /// base64url (no padding) of the 16-byte auth secret.
    pub auth: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DevicePush {
    pub prefs: Prefs,
    /// sessionId → "follow" | "mute".
    pub sessions: BTreeMap<String, String>,
    /// sessionId → expiry (epoch ms) of a one-shot "notify when this turn
    /// finishes".
    pub turn_finished: BTreeMap<String, i64>,
    pub subscriptions: Vec<Subscription>,
}

impl DevicePush {
    pub fn mode(&self, session_id: &str) -> Option<SessionMode> {
        self.sessions
            .get(session_id)
            .and_then(|m| SessionMode::parse(m))
    }

    pub fn turn_once_active(&self, session_id: &str, now: i64) -> bool {
        self.turn_finished
            .get(session_id)
            .is_some_and(|exp| *exp > now)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PushFile {
    pub version: u32,
    /// The VAPID public key the stored subscriptions were made against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vapid_public_key: Option<String>,
    pub devices: BTreeMap<String, DevicePush>,
}

/// What `subscribe` did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubscribeOutcome {
    /// False when the endpoint was already this device's (keys refreshed).
    pub added: bool,
    /// Subscriptions replaced because the device hit the cap.
    pub evicted: usize,
    /// The endpoint moved here from another device (same browser, re-paired).
    pub moved_from: Option<String>,
    /// The device's subscription count afterwards.
    pub count: usize,
}

/// `PUT /api/push/prefs` body. Every field optional; maps are merged.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PrefsPatch {
    pub permission: Option<bool>,
    pub question: Option<bool>,
    pub plan: Option<bool>,
    pub finished: Option<bool>,
    pub failed: Option<bool>,
    pub claude: Option<bool>,
    pub idle: Option<bool>,
    pub show_details: Option<bool>,
    /// sessionId → "follow" | "mute" | "default" (or null = default).
    pub sessions: Option<BTreeMap<String, Option<String>>>,
    /// sessionId → true (arm "notify me when this turn finishes") | false.
    pub turn_finished: Option<BTreeMap<String, bool>>,
}

/// `GET /api/push/prefs` answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefsView {
    #[serde(flatten)]
    pub prefs: Prefs,
    pub sessions: BTreeMap<String, String>,
    pub turn_finished: Vec<String>,
    /// This device's stored subscriptions.
    pub subscriptions: usize,
    /// Whether this caller can subscribe at all (paired devices only).
    pub available: bool,
}

/// Session ids used as map keys: `^[A-Za-z0-9_-]{1,80}$` (same shape as the
/// data layer's id validator; never used to build a path here).
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

const LENIENT: GeneralPurposeConfig =
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent);

/// Decode base64url or standard base64, padded or not (browsers send
/// base64url; hand-rolled clients often use `btoa`).
pub fn decode_b64(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.is_empty() || s.len() > 256 {
        return None;
    }
    let url = GeneralPurpose::new(&base64::alphabet::URL_SAFE, LENIENT);
    let std = GeneralPurpose::new(&base64::alphabet::STANDARD, LENIENT);
    url.decode(s).ok().or_else(|| std.decode(s).ok())
}

/// Validate the subscription keys and return their canonical base64url
/// forms. `p256dh` must be a valid uncompressed P-256 point, `auth` exactly
/// 16 bytes.
pub fn canonical_keys(p256dh: &str, auth: &str) -> Result<(String, String), &'static str> {
    let point = decode_b64(p256dh).ok_or("keys.p256dh is not base64.")?;
    if point.len() != 65 || point[0] != 0x04 {
        return Err("keys.p256dh must be an uncompressed P-256 public key (65 bytes).");
    }
    if web_push_native::p256::PublicKey::from_sec1_bytes(&point).is_err() {
        return Err("keys.p256dh is not a valid P-256 public key.");
    }
    let secret = decode_b64(auth).ok_or("keys.auth is not base64.")?;
    if secret.len() != 16 {
        return Err("keys.auth must be 16 bytes.");
    }
    Ok((
        URL_SAFE_NO_PAD.encode(point),
        URL_SAFE_NO_PAD.encode(secret),
    ))
}

impl PushFile {
    /// Clean a freshly loaded file: drop subscriptions made with another
    /// VAPID key or failing `valid`, expired one-shots, unknown session modes,
    /// and devices left empty. Returns whether anything changed.
    pub fn sanitize(
        &mut self,
        vapid_public_key: &str,
        now: i64,
        valid: impl Fn(&Subscription) -> bool,
    ) -> bool {
        let before = self.clone();
        self.version = VERSION;
        let key_changed = self
            .vapid_public_key
            .as_deref()
            .is_some_and(|k| k != vapid_public_key);
        if key_changed {
            tracing::warn!(
                "the VAPID key changed; dropping push subscriptions made with the old key"
            );
        }
        self.vapid_public_key = Some(vapid_public_key.to_string());
        for dev in self.devices.values_mut() {
            if key_changed {
                dev.subscriptions.clear();
            }
            let n = dev.subscriptions.len();
            dev.subscriptions.retain(|s| valid(s));
            if dev.subscriptions.len() != n {
                tracing::warn!(
                    dropped = n - dev.subscriptions.len(),
                    "dropped invalid push subscriptions"
                );
            }
            dev.turn_finished.retain(|_, exp| *exp > now);
            dev.sessions
                .retain(|id, m| valid_session_id(id) && SessionMode::parse(m).is_some());
        }
        self.devices.retain(|id, d| {
            !id.is_empty()
                && (!d.subscriptions.is_empty()
                    || d.prefs != Prefs::default()
                    || !d.sessions.is_empty()
                    || !d.turn_finished.is_empty())
        });
        *self != before
    }

    /// Store `sub` for `device` (dedupe by endpoint across all devices; cap per
    /// device, replacing the least recently updated).
    pub fn subscribe(&mut self, device: &str, mut sub: Subscription, now: i64) -> SubscribeOutcome {
        let mut out = SubscribeOutcome {
            added: true,
            ..Default::default()
        };
        let mut created_at = now;
        for (id, dev) in self.devices.iter_mut() {
            if let Some(pos) = dev
                .subscriptions
                .iter()
                .position(|s| s.endpoint == sub.endpoint)
            {
                let old = dev.subscriptions.remove(pos);
                if id == device {
                    created_at = old.created_at;
                    out.added = false;
                } else {
                    out.moved_from = Some(id.clone());
                }
            }
        }
        let dev = self.devices.entry(device.to_string()).or_default();
        sub.created_at = created_at;
        sub.updated_at = now;
        dev.subscriptions.push(sub);
        while dev.subscriptions.len() > MAX_SUBSCRIPTIONS_PER_DEVICE {
            let oldest = dev
                .subscriptions
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.updated_at)
                .map(|(i, _)| i)
                .unwrap_or(0);
            dev.subscriptions.remove(oldest);
            out.evicted += 1;
        }
        out.count = dev.subscriptions.len();
        self.prune_empty();
        out
    }

    /// Remove one of `device`'s subscriptions. Only the owner can.
    pub fn unsubscribe(&mut self, device: &str, endpoint: &str) -> bool {
        let removed = self.devices.get_mut(device).is_some_and(|d| {
            let n = d.subscriptions.len();
            d.subscriptions.retain(|s| s.endpoint != endpoint);
            d.subscriptions.len() != n
        });
        self.prune_empty();
        removed
    }

    /// Forget an endpoint wherever it is (the push service said 404/410).
    /// Returns the device that held it.
    pub fn remove_endpoint(&mut self, endpoint: &str) -> Option<String> {
        let mut owner = None;
        for (id, d) in self.devices.iter_mut() {
            let n = d.subscriptions.len();
            d.subscriptions.retain(|s| s.endpoint != endpoint);
            if d.subscriptions.len() != n {
                owner = Some(id.clone());
            }
        }
        self.prune_empty();
        owner
    }

    /// Forget everything about a device (revoked / unknown).
    pub fn drop_device(&mut self, device: &str) -> bool {
        self.devices.remove(device).is_some()
    }

    /// Apply a prefs patch for `device`. All-or-nothing.
    pub fn apply_patch(&mut self, device: &str, patch: PrefsPatch, now: i64) -> Result<(), String> {
        let mut dev = self.devices.get(device).cloned().unwrap_or_default();
        let p = &mut dev.prefs;
        for (slot, v) in [
            (&mut p.permission, patch.permission),
            (&mut p.question, patch.question),
            (&mut p.plan, patch.plan),
            (&mut p.finished, patch.finished),
            (&mut p.failed, patch.failed),
            (&mut p.claude, patch.claude),
            (&mut p.idle, patch.idle),
            (&mut p.show_details, patch.show_details),
        ] {
            if let Some(v) = v {
                *slot = v;
            }
        }
        for (id, mode) in patch.sessions.unwrap_or_default() {
            if !valid_session_id(&id) {
                return Err(format!("Invalid session id in sessions: {id:.80}"));
            }
            match mode.as_deref() {
                None | Some("default") => {
                    dev.sessions.remove(&id);
                }
                Some(m) => match SessionMode::parse(m) {
                    Some(mode) => {
                        dev.sessions.insert(id, mode.as_str().to_string());
                    }
                    None => {
                        return Err(format!(
                            "Unknown session mode {m:.20}: use follow, mute or default."
                        ))
                    }
                },
            }
        }
        if dev.sessions.len() > MAX_SESSION_MODES {
            return Err(format!(
                "At most {MAX_SESSION_MODES} sessions can be followed or muted."
            ));
        }
        dev.turn_finished.retain(|_, exp| *exp > now);
        for (id, on) in patch.turn_finished.unwrap_or_default() {
            if !valid_session_id(&id) {
                return Err(format!("Invalid session id in turnFinished: {id:.80}"));
            }
            if on {
                dev.turn_finished.insert(id, now + TURN_ONCE_TTL_MS);
            } else {
                dev.turn_finished.remove(&id);
            }
        }
        if dev.turn_finished.len() > MAX_TURN_ONCE {
            return Err(format!(
                "At most {MAX_TURN_ONCE} turn-finished alerts can be armed at once."
            ));
        }
        self.devices.insert(device.to_string(), dev);
        self.prune_empty();
        Ok(())
    }

    pub fn view(&self, device: &str, now: i64, available: bool) -> PrefsView {
        let dev = self.devices.get(device).cloned().unwrap_or_default();
        PrefsView {
            prefs: dev.prefs,
            sessions: dev.sessions,
            turn_finished: dev
                .turn_finished
                .iter()
                .filter(|(_, exp)| **exp > now)
                .map(|(id, _)| id.clone())
                .collect(),
            subscriptions: dev.subscriptions.len(),
            available,
        }
    }

    /// Whether any device has a subscription (cheap early exit for triggers).
    pub fn has_subscriptions(&self) -> bool {
        self.devices.values().any(|d| !d.subscriptions.is_empty())
    }

    fn prune_empty(&mut self) {
        self.devices.retain(|_, d| {
            !d.subscriptions.is_empty()
                || d.prefs != Prefs::default()
                || !d.sessions.is_empty()
                || !d.turn_finished.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use web_push_native::jwt_simple::algorithms::ES256KeyPair;
    use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint as _;

    fn keys() -> (String, String) {
        let kp = ES256KeyPair::generate();
        let sk = web_push_native::p256::SecretKey::from_slice(&kp.to_bytes()).unwrap();
        let point = sk.public_key().to_encoded_point(false);
        (
            URL_SAFE_NO_PAD.encode(point.as_bytes()),
            URL_SAFE_NO_PAD.encode([7u8; 16]),
        )
    }

    fn sub(endpoint: &str) -> Subscription {
        let (p, a) = keys();
        Subscription {
            endpoint: endpoint.into(),
            p256dh: p,
            auth: a,
            ..Default::default()
        }
    }

    #[test]
    fn key_validation() {
        let (p, a) = keys();
        assert!(canonical_keys(&p, &a).is_ok());
        // Standard alphabet with padding is accepted and canonicalised.
        let raw = URL_SAFE_NO_PAD.decode(&p).unwrap();
        let (cp, _) = canonical_keys(&STANDARD.encode(&raw), &STANDARD.encode([7u8; 16])).unwrap();
        assert_eq!(cp, p);
        assert!(canonical_keys("nope!", &a).is_err());
        assert!(canonical_keys(&p, &URL_SAFE_NO_PAD.encode([1u8; 15])).is_err());
        // 65 bytes but not on the curve.
        let mut bad = vec![4u8];
        bad.extend([1u8; 64]);
        assert!(canonical_keys(&URL_SAFE_NO_PAD.encode(&bad), &a).is_err());
        // Compressed points are not what browsers send.
        assert!(canonical_keys(&URL_SAFE_NO_PAD.encode(&raw[..33]), &a).is_err());
    }

    #[test]
    fn subscription_lifecycle() {
        let mut f = PushFile::default();
        let out = f.subscribe("d1", sub("https://fcm.googleapis.com/a"), 10);
        assert!(out.added);
        assert_eq!(out.count, 1);
        // Re-subscribing the same endpoint refreshes, keeps createdAt.
        let out = f.subscribe("d1", sub("https://fcm.googleapis.com/a"), 20);
        assert!(!out.added);
        assert_eq!(out.count, 1);
        let s = &f.devices["d1"].subscriptions[0];
        assert_eq!((s.created_at, s.updated_at), (10, 20));
        // Cap: the least recently updated goes.
        for i in 0..6 {
            f.subscribe(
                "d1",
                sub(&format!("https://fcm.googleapis.com/n{i}")),
                30 + i,
            );
        }
        let d1 = &f.devices["d1"];
        assert_eq!(d1.subscriptions.len(), MAX_SUBSCRIPTIONS_PER_DEVICE);
        assert!(!d1.subscriptions.iter().any(|s| s.endpoint.ends_with("/a")));
        assert!(!d1.subscriptions.iter().any(|s| s.endpoint.ends_with("/n0")));
        // The same browser re-paired as another device: the endpoint moves.
        let out = f.subscribe("d2", sub("https://fcm.googleapis.com/n5"), 50);
        assert_eq!(out.moved_from.as_deref(), Some("d1"));
        assert_eq!(f.devices["d1"].subscriptions.len(), 4);
        // Only the owner can unsubscribe.
        assert!(!f.unsubscribe("d1", "https://fcm.googleapis.com/n5"));
        assert!(f.unsubscribe("d2", "https://fcm.googleapis.com/n5"));
        assert!(!f.devices.contains_key("d2"), "empty device pruned");
        // 404/410 removal finds the owner.
        assert_eq!(
            f.remove_endpoint("https://fcm.googleapis.com/n4")
                .as_deref(),
            Some("d1")
        );
        assert_eq!(f.remove_endpoint("https://fcm.googleapis.com/zz"), None);
        // Revocation forgets the device.
        assert!(f.drop_device("d1"));
        assert!(!f.has_subscriptions());
    }

    #[test]
    fn prefs_patch_and_view() {
        let mut f = PushFile::default();
        let now = 1_000;
        let v = f.view("d1", now, true);
        assert_eq!(v.prefs, Prefs::default());
        assert!(!v.prefs.idle && !v.prefs.show_details);
        let patch: PrefsPatch = serde_json::from_value(serde_json::json!({
            "finished": false, "idle": true, "showDetails": true,
            "sessions": {"s-1": "follow", "s-2": "mute", "s-3": "default"},
            "turnFinished": {"s-1": true},
            "subscriptions": 9, "available": false
        }))
        .unwrap();
        f.apply_patch("d1", patch, now).unwrap();
        let v = f.view("d1", now, true);
        assert!(!v.prefs.finished && v.prefs.idle && v.prefs.show_details);
        assert_eq!(v.sessions.get("s-1").map(String::as_str), Some("follow"));
        assert_eq!(v.sessions.get("s-2").map(String::as_str), Some("mute"));
        assert_eq!(v.turn_finished, vec!["s-1".to_string()]);
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["showDetails"], true);
        assert_eq!(json["turnFinished"][0], "s-1");
        // null / "default" clears; bad input is rejected without partial writes.
        let clear: PrefsPatch =
            serde_json::from_value(serde_json::json!({"sessions": {"s-2": null}})).unwrap();
        f.apply_patch("d1", clear, now).unwrap();
        assert!(!f.view("d1", now, true).sessions.contains_key("s-2"));
        for bad in [
            serde_json::json!({"sessions": {"../x": "follow"}}),
            serde_json::json!({"sessions": {"s-9": "loud"}}),
            serde_json::json!({"turnFinished": {"a b": true}, "plan": false}),
        ] {
            let p: PrefsPatch = serde_json::from_value(bad).unwrap();
            assert!(f.apply_patch("d1", p, now).is_err());
        }
        assert!(
            f.view("d1", now, true).prefs.plan,
            "rejected patch applied nothing"
        );
        // One-shots expire after a day; disarming removes them.
        assert!(f.devices["d1"].turn_once_active("s-1", now));
        assert!(!f.devices["d1"].turn_once_active("s-1", now + TURN_ONCE_TTL_MS + 1));
        let disarm: PrefsPatch =
            serde_json::from_value(serde_json::json!({"turnFinished": {"s-1": false}})).unwrap();
        f.apply_patch("d1", disarm, now).unwrap();
        assert!(f.view("d1", now, true).turn_finished.is_empty());
    }

    #[test]
    fn sanitize_drops_stale_state() {
        let mut f = PushFile {
            vapid_public_key: Some("OLD".into()),
            ..Default::default()
        };
        f.subscribe("d1", sub("https://fcm.googleapis.com/a"), 1);
        f.devices
            .get_mut("d1")
            .unwrap()
            .sessions
            .insert("s".into(), "weird".into());
        assert!(f.sanitize("NEW", 5, |_| true));
        assert!(f.devices.is_empty(), "key changed: subscriptions dropped");
        assert_eq!(f.vapid_public_key.as_deref(), Some("NEW"));

        let mut f = PushFile::default();
        f.subscribe("d1", sub("https://fcm.googleapis.com/a"), 1);
        f.subscribe("d1", sub("http://127.0.0.1/evil"), 2);
        f.sanitize("K", 5, |s| s.endpoint.starts_with("https://"));
        assert_eq!(f.devices["d1"].subscriptions.len(), 1);
        // Tolerant parse of a partial / odd file.
        let parsed: PushFile = serde_json::from_str(
            r#"{"devices":{"d9":{"subscriptions":[{"endpoint":"https://fcm.googleapis.com/q"}],"extra":1}}}"#,
        )
        .unwrap();
        assert_eq!(parsed.devices["d9"].subscriptions.len(), 1);
    }

    #[test]
    fn session_ids() {
        assert!(valid_session_id("0b6a8f1e-1c2d-4e5f-8a9b-0c1d2e3f4a5b"));
        assert!(valid_session_id("thread_abc-1"));
        for bad in ["", "a/b", "..", "a b", "x*", &"a".repeat(81)] {
            assert!(!valid_session_id(bad), "{bad}");
        }
    }
}
