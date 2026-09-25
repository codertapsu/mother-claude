//! QR/code pairing (design §2.6).
//!
//! Owner: **B1 (security/remote)** — STUB. What B1 implements here:
//!
//! * pairing codes: 12 chars Crockford base32 (60 bits, `auth::random_bytes`),
//!   single use, 5-minute TTL, at most 5 active; invalidated after 5 failed
//!   attempts. URL = `{baseUrl}/#/pair?c={code}` (code in the FRAGMENT).
//! * exchange → `201 {deviceId, token, name}` or, for an internet code with
//!   `requireApprovalForInternet`, `202 {pendingId, pollSecret, fingerprint}`
//!   and a `pairRequest` WS frame to the desktop
//!   (`crate::server::ws::publish_pair_request`).
//! * pending polling with `X-Pair-Secret`; the token is returned exactly once.
//! * rate limits: 10 failures/min per client ip, 30 per 10 min globally (extend
//!   `server::ratelimit`).
//!
//! The HTTP handlers live in `server/routes/pairing.rs` (also B1).
//! AppState field: `state.pairing`.

use std::path::Path;

/// Active codes and pending approvals. STUB.
#[derive(Debug, Default)]
pub struct PairingState {}

impl PairingState {
    /// Tolerant constructor; `None` ⇒ in-memory only.
    pub fn new(_data_dir: Option<&Path>) -> Self {
        Self::default()
    }
}
