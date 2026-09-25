//! Web Push notifications (design §6).
//!
//! Owner: **B5 (push)** — STUB. What B5 implements here:
//!
//! * VAPID P-256 keypair persisted 0600 in the app data dir (crate
//!   `web-push-native` is already a dependency; send with our own reqwest);
//! * subscriptions per device in `push.json` (removed on device revoke and on
//!   404/410 from the push service);
//! * per-device prefs `{permission, question, plan, finished, failed}`;
//! * the triggers below, coalesced per session (10 s), skipping devices with a
//!   foreground WebSocket (`state.presence.is_foreground(device_id)`);
//! * payload `{title, body, sessionId, url:"/#/session/<id>"}`; VAPID `sub` =
//!   the public URL if https else `mailto:mother-claude@users.noreply.github.com`.
//!
//! The trigger functions are called by other subsystems and MUST stay cheap
//! and non-blocking (spawn any network work). Handlers live in
//! `server/routes/push.rs` (also B5). AppState field: `state.push`.

use std::path::Path;

use crate::claude::model::PendingItem;
use crate::state::AppState;

/// VAPID keys, subscriptions and prefs. STUB.
#[derive(Debug, Default)]
pub struct PushService {}

impl PushService {
    /// Tolerant constructor; `None` ⇒ in-memory only (tests).
    pub fn new(_data_dir: Option<&Path>) -> Self {
        Self::default()
    }
}

/// Start background work (e.g. the coalescing flusher). Called once from
/// `server::serve`. STUB.
pub fn start(_state: &AppState) {}

/// A new answerable pending item was raised (called by `pending::raise`).
/// STUB.
pub fn on_pending_added(_state: &AppState, _item: &PendingItem) {}

/// An owned/hooked session finished a turn after `worked_ms` of work (called by
/// the owned control loop, B3, and the hook dispatcher, B4). STUB.
pub fn on_turn_finished(_state: &AppState, _session_id: &str, _worked_ms: u64) {}

/// A session failed or hit an API error (called by B3/B4/B2). STUB.
pub fn on_session_failed(_state: &AppState, _session_id: &str, _text: &str) {}

/// Forget a revoked device's subscriptions (called by B1 on revoke). STUB.
pub fn drop_device(_state: &AppState, _device_id: &str) {}
