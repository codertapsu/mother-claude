//! Internet access through a tunnel (design §2.8).
//!
//! Owner: **B1 (security/remote)** — STUB. What B1 implements here:
//!
//! * `remote.json` in the app data dir (default OFF): `internetEnabled`,
//!   `manualUrl`, `requireApproval`.
//! * the tunnel listener `127.0.0.1:<tunnelPort>` (default 6726, env
//!   `MOTHER_CLAUDE_TUNNEL_PORT`), bound only while Internet access is on,
//!   serving `crate::server::router_for(state, Ingress::Tunnel)` (already
//!   excludes `/hooks/*` and `/v1`); require last `X-Forwarded-Proto == https`
//!   (else 400 with guidance) — e.g. in `server::security::apply`.
//! * tunnel detection loop (ngrok `127.0.0.1:4040/api/endpoints`, cloudflared
//!   `/quicktunnel` on 20241..20245, `tailscale funnel status --json`), the
//!   `GET/PUT /api/remote` payloads and warnings.
//!
//! [`start`] is called once from `server::serve` after the loopback listener
//! is ready; spawn background tasks from there. Handlers live in
//! `server/routes/remote.rs` (also B1). AppState field: `state.remote`.

use std::path::Path;

use crate::state::AppState;

/// Persisted settings + detection results. STUB.
#[derive(Debug, Default)]
pub struct RemoteState {}

impl RemoteState {
    /// Tolerant constructor; `None` ⇒ in-memory only.
    pub fn new(_data_dir: Option<&Path>) -> Self {
        Self::default()
    }
}

/// Start remote-access background work (tunnel listener if enabled, detection
/// loop). Called once from `server::serve`. STUB: nothing to start.
pub fn start(_state: &AppState) {}
