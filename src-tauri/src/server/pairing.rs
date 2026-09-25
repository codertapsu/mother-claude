//! Pairing codes, the exchange and desktop approval (design §2.6, §10 B1).
//!
//! * **Codes**: 12 characters of Crockford base32 (60 bits of OS randomness),
//!   single use, 5-minute TTL, at most 5 active (a 6th evicts the oldest). The
//!   QR/link carries the code in the URL **fragment**
//!   (`{baseUrl}/#/pair?c={code}`), so it never reaches a server or tunnel log.
//!   Targets: `lan` and `internet` (desktop only) and `self` (a paired device
//!   adding a second token for its own iOS Home Screen app).
//! * **Exchange** (`POST /api/pair/exchange`, public): `201 {deviceId, token,
//!   name}` ONLY when the effective ingress is the LAN (no proxy headers, a
//!   private peer) and the code's target is `lan`. Everything else — any
//!   tunnel or proxied request, any `internet` code — becomes a pending pair:
//!   `202 {pendingId, pollSecret, fingerprint, expiresAt}`. A `lan` code
//!   presented through a tunnel is refused. `self` codes add a token to their
//!   own device and are answered `201` directly (they grant nothing the device
//!   does not already have).
//! * **Approval is typed**: the phone shows 4 digits, the desktop types them
//!   into `POST /api/devices/pending/{id}/approve {fingerprint}`; three wrong
//!   attempts deny the request. Neither `GET /api/devices` nor the
//!   `pairRequest` frame ever carries the fingerprint or the poll secret.
//! * A code presented again within its TTL marks the pending pair created from
//!   it **contested**: approval is refused, the desktop is told, and both phones
//!   learn the code was used by another device.
//! * Pending pairs expire after 10 minutes; at most 5 wait at once.
//! * Rate limit: failed exchanges and polls count per tunnel-aware client ip
//!   (`AuthLimiter::record_pair_failure`, 10/min). No global bucket.
//!
//! Nothing here logs a code, a poll secret or a token. The HTTP handlers live
//! in `server/routes/pairing.rs`. AppState field: `state.pairing`.

use std::path::Path;
use std::sync::Mutex;

use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::auth;
use super::context::{ClientContext, Ingress, Principal};
use super::devices::{self, audit::AuditEntry, sanitize_label, DeviceInfo};
use super::error::{ApiError, ApiResult};
use crate::claude::model::NoticeLevel;
use crate::state::AppState;

/// Characters in a pairing code.
pub const CODE_LEN: usize = 12;
/// Code lifetime.
pub const CODE_TTL_MS: i64 = 5 * 60 * 1000;
/// Codes alive at once.
pub const MAX_ACTIVE_CODES: usize = 5;
/// Pending-pair lifetime.
pub const PENDING_TTL_MS: i64 = 10 * 60 * 1000;
/// Pending pairs waiting at once.
pub const MAX_PENDING: usize = 5;
/// Wrong fingerprints typed on the desktop before a request is denied.
pub const MAX_FINGERPRINT_ATTEMPTS: u32 = 3;
/// How long an approved/denied/expired pair stays pollable.
const DECIDED_KEEP_MS: i64 = 10 * 60 * 1000;
const POLL_SECRET_PREFIX: &str = "mcps_";

/// Crockford base32 alphabet (no I, L, O, U).
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Who a code is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeTarget {
    Lan,
    Internet,
    #[serde(rename = "self")]
    SelfDevice,
}

impl CodeTarget {
    fn as_str(self) -> &'static str {
        match self {
            CodeTarget::Lan => "lan",
            CodeTarget::Internet => "internet",
            CodeTarget::SelfDevice => "self",
        }
    }
}

#[derive(Debug, Clone)]
struct CodeUse {
    pending_id: Option<String>,
    device_id: Option<String>,
}

#[derive(Debug, Clone)]
struct Code {
    code: String,
    target: CodeTarget,
    /// The device a `self` code belongs to.
    device_id: Option<String>,
    expires_at: i64,
    used: Option<CodeUse>,
}

/// Where a pending pair stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PendingStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Debug, Clone)]
struct PendingPair {
    id: String,
    poll_secret_sha: String,
    fingerprint: String,
    device_name: String,
    platform: String,
    user_agent: Option<String>,
    client_ip: String,
    ingress: Ingress,
    provider: Option<String>,
    code_target: CodeTarget,
    created_at: i64,
    expires_at: i64,
    status: PendingStatus,
    contested: bool,
    fingerprint_attempts: u32,
    decided_at: Option<i64>,
    device_id: Option<String>,
    /// The new device's token, until the phone picks it up (exactly once).
    token: Option<String>,
}

impl PendingPair {
    fn is_live(&self, now: i64) -> bool {
        self.status == PendingStatus::Pending && now < self.expires_at
    }

    /// What the desktop sees: never the fingerprint or the poll secret.
    fn view(&self) -> Value {
        json!({
            "id": self.id,
            "deviceName": self.device_name,
            "platform": self.platform,
            "userAgent": self.user_agent,
            "clientIp": self.client_ip,
            "ingress": self.ingress.as_str(),
            "provider": self.provider,
            "codeTarget": self.code_target.as_str(),
            "createdAt": self.created_at,
            "expiresAt": self.expires_at,
            "status": self.status,
            "contested": self.contested,
            "fingerprintAttemptsLeft": MAX_FINGERPRINT_ATTEMPTS.saturating_sub(self.fingerprint_attempts),
        })
    }
}

#[derive(Debug, Default)]
struct Inner {
    codes: Vec<Code>,
    pending: Vec<PendingPair>,
}

impl Inner {
    fn prune(&mut self, now: i64) -> bool {
        let mut changed = false;
        // Used codes are kept until their TTL so a replay can be detected.
        let before = self.codes.len();
        self.codes.retain(|c| now < c.expires_at);
        changed |= before != self.codes.len();
        for p in &mut self.pending {
            if p.status == PendingStatus::Pending && now >= p.expires_at {
                p.status = PendingStatus::Expired;
                p.decided_at = Some(now);
                p.token = None;
                changed = true;
            }
        }
        let before = self.pending.len();
        self.pending.retain(|p| {
            p.status == PendingStatus::Pending
                || now < p.decided_at.unwrap_or(p.expires_at) + DECIDED_KEEP_MS
        });
        changed |= before != self.pending.len();
        changed
    }
}

/// Active codes and pending approvals (in memory only).
#[derive(Debug, Default)]
pub struct PairingState {
    inner: Mutex<Inner>,
}

impl PairingState {
    /// Tolerant constructor; nothing is persisted (codes and pending pairs do
    /// not survive a restart by design).
    pub fn new(_data_dir: Option<&Path>) -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Drop expired codes and expire stale pending pairs. `true` when a
    /// pending pair changed (desktop clients should refetch).
    pub fn sweep(&self) -> bool {
        self.lock().prune(now_ms())
    }

    /// Pending pairs as the desktop sees them (live ones first).
    pub fn pending_views(&self) -> Vec<Value> {
        let mut g = self.lock();
        g.prune(now_ms());
        let mut out: Vec<&PendingPair> = g.pending.iter().collect();
        out.sort_by_key(|p| (p.status != PendingStatus::Pending, p.created_at));
        out.into_iter().map(PendingPair::view).collect()
    }

    /// Number of live pending pairs.
    pub fn pending_count(&self) -> usize {
        let now = now_ms();
        self.lock()
            .pending
            .iter()
            .filter(|p| p.is_live(now))
            .count()
    }
}

/// A fresh random code.
pub fn new_code() -> String {
    auth::random_bytes(CODE_LEN)
        .into_iter()
        .map(|b| CROCKFORD[(b & 31) as usize] as char)
        .collect()
}

/// Normalise what a person typed or scanned: case-insensitive, spaces and
/// dashes ignored, Crockford's O→0 and I/L→1. `None` when it cannot be a code.
pub fn normalize_code(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_LEN);
    for c in raw.chars() {
        let c = match c.to_ascii_uppercase() {
            ' ' | '-' | '_' | '\t' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !CROCKFORD.contains(&(c as u8)) || !c.is_ascii() {
            return None;
        }
        out.push(c);
        if out.len() > CODE_LEN {
            return None;
        }
    }
    (out.len() == CODE_LEN).then_some(out)
}

/// `XXXX-XXXX-XXXX` for display.
pub fn display_code(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("-")
}

fn new_fingerprint() -> String {
    let b = auth::random_bytes(4);
    let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) % 10_000;
    format!("{n:04}")
}

/// Validate a base URL the desktop supplied: `http(s)://host[:port]`, no
/// path, query, fragment or userinfo. Returns it without a trailing slash.
pub fn validate_base_url(raw: &str, https_only: bool) -> ApiResult<String> {
    let url = raw.trim().trim_end_matches('/');
    let (scheme, rest) = url.split_once("://").ok_or_else(|| {
        ApiError::unprocessable("invalid_url", "Use a full URL like https://example.com.")
    })?;
    let scheme_ok = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => !https_only,
        _ => false,
    };
    if !scheme_ok {
        return Err(ApiError::unprocessable(
            "invalid_url",
            if https_only {
                "The public URL must start with https://."
            } else {
                "The URL must start with http:// or https://."
            },
        ));
    }
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', ' ', '\\']) || rest.len() > 253 {
        return Err(ApiError::unprocessable(
            "invalid_url",
            "Give only the scheme, host and optional port (no path).",
        ));
    }
    if rest.chars().any(|c| c.is_control()) {
        return Err(ApiError::unprocessable(
            "invalid_url",
            "Invalid characters in the URL.",
        ));
    }
    Ok(format!("{}://{rest}", scheme.to_ascii_lowercase()))
}

/// `POST /api/pair/codes` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeRequest {
    #[serde(default = "default_target")]
    pub target: CodeTarget,
    #[serde(default)]
    pub base_url: Option<String>,
}

fn default_target() -> CodeTarget {
    CodeTarget::Lan
}

/// Create a pairing code. `lan` and `internet` codes are desktop-only; a
/// device may create a `self` code for its own record.
pub fn create_code(state: &AppState, ctx: &ClientContext, req: CodeRequest) -> ApiResult<Value> {
    let (base_url, device_id) = match req.target {
        CodeTarget::SelfDevice => {
            let Some(id) = ctx.device_id() else {
                return Err(ApiError::forbidden(
                    "device_only",
                    "Only a paired device can create a code for itself.",
                ));
            };
            (None, Some(id.to_string()))
        }
        CodeTarget::Lan | CodeTarget::Internet => {
            auth::require_desktop(ctx, "Creating a pairing code")?;
            let base = match (&req.base_url, req.target) {
                (Some(b), CodeTarget::Internet) => validate_base_url(b, true)?,
                (Some(b), _) => validate_base_url(b, false)?,
                (None, CodeTarget::Lan) => state.remote.lan_base_url().ok_or_else(|| {
                    ApiError::conflict(
                        "lan_unavailable",
                        "Mother Claude is not listening on your local network (it is bound \
                         to 127.0.0.1, or no private network address was found).",
                    )
                })?,
                (None, _) => state.remote.public_url().ok_or_else(|| {
                    ApiError::conflict(
                        "no_public_url",
                        "No public URL yet: start a tunnel (see Remote access) or enter the \
                         public URL by hand.",
                    )
                })?,
            };
            (Some(base), None)
        }
    };

    let now = now_ms();
    let code = new_code();
    {
        let mut g = state.pairing.lock();
        g.prune(now);
        // At most MAX_ACTIVE_CODES unused codes: evict the oldest.
        loop {
            let unused: Vec<usize> = g
                .codes
                .iter()
                .enumerate()
                .filter(|(_, c)| c.used.is_none())
                .map(|(i, _)| i)
                .collect();
            if unused.len() < MAX_ACTIVE_CODES {
                break;
            }
            g.codes.remove(unused[0]);
        }
        g.codes.push(Code {
            code: code.clone(),
            target: req.target,
            device_id,
            expires_at: now + CODE_TTL_MS,
            used: None,
        });
    }
    let url = match &base_url {
        Some(b) => format!("{b}/#/pair?c={code}"),
        None => format!("/#/pair?c={code}"),
    };
    if !ctx.is_desktop() {
        state
            .devices
            .audit
            .record(AuditEntry::from_ctx(ctx, "pair.code.self"));
    }
    Ok(json!({
        "code": code,
        "display": display_code(&code),
        "url": url,
        "svg": auth::qr_svg(&url),
        "expiresAt": now + CODE_TTL_MS,
        "ttlSec": CODE_TTL_MS / 1000,
        "target": req.target,
        "baseUrl": base_url,
    }))
}

/// `POST /api/pair/exchange` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeRequest {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub platform: String,
}

/// A best-effort label for where a request came through (for the approval
/// card): the tunnel provider from its headers, else the ingress.
pub fn provider_hint(ctx: &ClientContext, headers: &HeaderMap) -> String {
    let has = |prefix: &str| headers.keys().any(|k| k.as_str().starts_with(prefix));
    if has("cf-") {
        "cloudflare".into()
    } else if has("tailscale-") {
        "tailscale".into()
    } else if headers.keys().any(|k| {
        k.as_str().starts_with("ngrok-") && k.as_str() != super::context::NGROK_SKIP_WARNING
    }) {
        "ngrok".into()
    } else {
        match ctx.effective_ingress {
            Ingress::Lan => "lan".into(),
            Ingress::Tunnel if ctx.ingress == Ingress::Tunnel => "tunnel".into(),
            Ingress::Tunnel => "proxy".into(),
            other => other.as_str().into(),
        }
    }
}

fn pair_failure(state: &AppState, ctx: &ClientContext, err: ApiError) -> ApiError {
    state.limits.record_pair_failure(ctx.client_ip);
    err
}

fn invalid_code() -> ApiError {
    ApiError::not_found(
        "invalid_code",
        "This pairing code is invalid or has expired. Ask for a new one on your Mac.",
    )
}

fn check_pair_limit(state: &AppState, ctx: &ClientContext) -> ApiResult<()> {
    match state.limits.pair_blocked(ctx.client_ip) {
        Some(retry) => Err(ApiError::too_many_requests(
            retry,
            "Too many pairing attempts from this address. Try again later.",
        )),
        None => Ok(()),
    }
}

/// Redeem a code (public, rate limited).
pub fn exchange(
    state: &AppState,
    ctx: &ClientContext,
    headers: &HeaderMap,
    req: ExchangeRequest,
) -> ApiResult<(StatusCode, Value)> {
    check_pair_limit(state, ctx)?;
    let Some(code) = normalize_code(&req.code) else {
        return Err(pair_failure(state, ctx, invalid_code()));
    };
    let now = now_ms();
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|u| sanitize_label(u, 200));
    let name = sanitize_label(&req.device_name, devices::MAX_NAME_CHARS);
    let platform = sanitize_label(&req.platform, 32);
    let provider = provider_hint(ctx, headers);
    let lan_auto = ctx.effective_ingress == Ingress::Lan
        && ctx.ingress == Ingress::Lan
        && !ctx.proxied
        && super::context::is_private_ip(ctx.peer.ip());

    enum Plan {
        SelfToken(String),
        Auto,
        Pending(Box<PendingPair>),
    }

    // Decide under the lock; do device-store work after it.
    let plan = {
        let mut g = state.pairing.lock();
        g.prune(now);
        let Some(idx) = g
            .codes
            .iter()
            .position(|c| c.code.len() == code.len() && auth::token_matches(&c.code, &code))
        else {
            drop(g);
            return Err(pair_failure(state, ctx, invalid_code()));
        };
        if let Some(used) = g.codes[idx].used.clone() {
            // Replay within the TTL: somebody else holds this code too.
            let mut contested_view = None;
            if let Some(pid) = &used.pending_id {
                if let Some(p) = g.pending.iter_mut().find(|p| &p.id == pid) {
                    if p.status == PendingStatus::Pending {
                        p.contested = true;
                        contested_view = Some(p.view());
                    }
                }
            }
            drop(g);
            let text = if contested_view.is_some() {
                "A pairing code was used by two devices. That request can no longer be \
                 approved; create a new code."
            } else {
                "A pairing code that was already used was presented again by another device. \
                 Check Remote > Devices."
            };
            crate::server::ws::publish_notice(state, NoticeLevel::Warning, text, None);
            crate::server::ws::publish_devices(state);
            state.devices.audit.record(AuditEntry {
                detail: Some(json!({
                    "pendingId": used.pending_id,
                    "deviceId": used.device_id,
                    "provider": provider,
                })),
                ..AuditEntry::from_ctx(ctx, "pair.contested")
            });
            return Err(pair_failure(
                state,
                ctx,
                ApiError::conflict(
                    "code_already_used",
                    "This code was already used by another device. Ask for a new code on \
                     your Mac.",
                ),
            ));
        }
        let target = g.codes[idx].target;
        match target {
            CodeTarget::Lan if !lan_auto => {
                drop(g);
                return Err(pair_failure(
                    state,
                    ctx,
                    ApiError::forbidden(
                        "wrong_network",
                        "This code is for your home network and cannot be used through a \
                         tunnel or proxy. On your Mac, create an Internet code instead.",
                    ),
                ));
            }
            CodeTarget::SelfDevice => {
                let Some(dev) = g.codes[idx].device_id.clone() else {
                    drop(g);
                    return Err(pair_failure(state, ctx, invalid_code()));
                };
                g.codes[idx].used = Some(CodeUse {
                    pending_id: None,
                    device_id: Some(dev.clone()),
                });
                Plan::SelfToken(dev)
            }
            CodeTarget::Lan => {
                g.codes[idx].used = Some(CodeUse {
                    pending_id: None,
                    device_id: None,
                });
                Plan::Auto
            }
            CodeTarget::Internet => {
                let live = g.pending.iter().filter(|p| p.is_live(now)).count();
                if live >= MAX_PENDING {
                    drop(g);
                    return Err(ApiError::too_many_requests(
                        60,
                        "Too many pairing requests are waiting for approval. Try again in a \
                         minute.",
                    ));
                }
                let poll_secret = auth::random_token(POLL_SECRET_PREFIX);
                let p = PendingPair {
                    id: format!("pp_{}", uuid::Uuid::new_v4().simple()),
                    poll_secret_sha: auth::sha256_hex(&poll_secret),
                    fingerprint: new_fingerprint(),
                    device_name: if name.is_empty() {
                        "Unnamed device".into()
                    } else {
                        name.clone()
                    },
                    platform: platform.clone(),
                    user_agent: user_agent.clone().filter(|u| !u.is_empty()),
                    client_ip: ctx.client_ip.to_string(),
                    ingress: ctx.effective_ingress,
                    provider: Some(provider.clone()),
                    code_target: target,
                    created_at: now,
                    expires_at: now + PENDING_TTL_MS,
                    status: PendingStatus::Pending,
                    contested: false,
                    fingerprint_attempts: 0,
                    decided_at: None,
                    device_id: None,
                    token: Some(poll_secret),
                };
                g.codes[idx].used = Some(CodeUse {
                    pending_id: Some(p.id.clone()),
                    device_id: None,
                });
                g.pending.push(p.clone());
                // The poll secret travels back once, in this response only.
                if let Some(stored) = g.pending.iter_mut().find(|x| x.id == p.id) {
                    stored.token = None;
                }
                Plan::Pending(Box::new(p))
            }
        }
    };

    match plan {
        Plan::SelfToken(dev) => {
            let token = state.devices.add_token(&dev).ok_or_else(invalid_code)?;
            let info = state.devices.get(&dev).unwrap_or_default();
            state.limits.clear_pair_failures(ctx.client_ip);
            crate::server::ws::publish_devices(state);
            state.devices.audit.record(AuditEntry {
                device_id: Some(dev.clone()),
                device_name: Some(info.name.clone()),
                ..AuditEntry::from_ctx(ctx, "pair.self")
            });
            Ok((
                StatusCode::CREATED,
                json!({ "deviceId": dev, "token": token, "name": info.name }),
            ))
        }
        Plan::Auto => {
            let (info, token) = state.devices.issue(
                if name.is_empty() {
                    "Unnamed device"
                } else {
                    &name
                },
                &platform,
                user_agent.as_deref(),
                ctx.effective_ingress,
                ctx.client_ip,
            );
            {
                let mut g = state.pairing.lock();
                if let Some(c) = g.codes.iter_mut().find(|c| c.code == code) {
                    c.used = Some(CodeUse {
                        pending_id: None,
                        device_id: Some(info.id.clone()),
                    });
                }
            }
            state.limits.clear_pair_failures(ctx.client_ip);
            announce_new_device(state, &info, ctx);
            state.devices.audit.record(AuditEntry {
                device_id: Some(info.id.clone()),
                device_name: Some(info.name.clone()),
                detail: Some(json!({ "approval": "automatic", "provider": provider })),
                ..AuditEntry::from_ctx(ctx, "pair.exchange")
            });
            Ok((
                StatusCode::CREATED,
                json!({ "deviceId": info.id, "token": token, "name": info.name }),
            ))
        }
        Plan::Pending(p) => {
            let poll_secret = p.token.clone().unwrap_or_default();
            let view = p.view();
            // The desktop's approval dialog comes from the desktop-only
            // `pairRequest` frame; other clients learn nothing about it.
            crate::server::ws::publish_pair_request(state, view);
            crate::server::ws::publish_devices(state);
            state.devices.audit.record(AuditEntry {
                device_name: Some(p.device_name.clone()),
                detail: Some(json!({ "pendingId": p.id, "provider": provider })),
                ..AuditEntry::from_ctx(ctx, "pair.pending")
            });
            Ok((
                StatusCode::ACCEPTED,
                json!({
                    "pendingId": p.id,
                    "pollSecret": poll_secret,
                    "fingerprint": p.fingerprint,
                    "expiresAt": p.expires_at,
                }),
            ))
        }
    }
}

fn announce_new_device(state: &AppState, info: &DeviceInfo, ctx: &ClientContext) {
    crate::server::ws::publish_devices(state);
    crate::server::ws::publish_notice(
        state,
        NoticeLevel::Info,
        format!(
            "New device paired: \"{}\" from {} ({}).",
            info.name,
            ctx.client_ip,
            ctx.effective_ingress.as_str()
        ),
        None,
    );
}

/// `GET /api/pair/pending/{id}` with `X-Pair-Secret` (public).
pub fn poll(
    state: &AppState,
    ctx: &ClientContext,
    id: &str,
    secret: Option<&str>,
) -> ApiResult<Value> {
    check_pair_limit(state, ctx)?;
    let not_found = || {
        ApiError::not_found(
            "not_found",
            "No such pairing request (it may have expired).",
        )
    };
    let secret_sha = secret.map(auth::sha256_hex).unwrap_or_default();
    let now = now_ms();
    let mut g = state.pairing.lock();
    g.prune(now);
    let Some(p) = g.pending.iter_mut().find(|p| {
        p.id == id && !secret_sha.is_empty() && auth::token_matches(&p.poll_secret_sha, &secret_sha)
    }) else {
        drop(g);
        return Err(pair_failure(state, ctx, not_found()));
    };
    if p.contested {
        return Ok(json!({
            "status": "contested",
            "message": "This code was already used by another device. Ask for a new code on your Mac.",
        }));
    }
    Ok(match p.status {
        PendingStatus::Approved => match p.token.take() {
            Some(token) => {
                let out = json!({
                    "status": "approved",
                    "token": token,
                    "deviceId": p.device_id,
                    "name": p.device_name,
                });
                drop(g);
                state.limits.clear_pair_failures(ctx.client_ip);
                out
            }
            None => json!({ "status": "approved", "deviceId": p.device_id }),
        },
        PendingStatus::Pending => json!({ "status": "pending", "expiresAt": p.expires_at }),
        PendingStatus::Denied => json!({ "status": "denied" }),
        PendingStatus::Expired => json!({ "status": "expired" }),
    })
}

/// `POST /api/devices/pending/{id}/approve {fingerprint}` (desktop).
pub fn approve(
    state: &AppState,
    ctx: &ClientContext,
    id: &str,
    fingerprint: &str,
) -> ApiResult<Value> {
    auth::require_desktop(ctx, "Approving a pairing")?;
    let now = now_ms();
    let typed: String = fingerprint.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut g = state.pairing.lock();
    g.prune(now);
    let Some(p) = g.pending.iter_mut().find(|p| p.id == id) else {
        return Err(ApiError::not_found(
            "not_found",
            "No such pairing request (it may have expired).",
        ));
    };
    match p.status {
        PendingStatus::Pending => {}
        PendingStatus::Expired => {
            return Err(ApiError::new(
                StatusCode::GONE,
                "expired",
                "This pairing request expired. Ask for a new code.",
            ))
        }
        _ => {
            return Err(ApiError::conflict(
                "already_decided",
                "This pairing request was already answered.",
            ))
        }
    }
    if p.contested {
        return Err(ApiError::conflict(
            "contested",
            "This code was used by two devices, so this request cannot be approved. Create \
             a new code.",
        ));
    }
    if typed.len() != 4 || !auth::token_matches(&p.fingerprint, &typed) {
        p.fingerprint_attempts += 1;
        let left = MAX_FINGERPRINT_ATTEMPTS.saturating_sub(p.fingerprint_attempts);
        let view = if left == 0 {
            p.status = PendingStatus::Denied;
            p.decided_at = Some(now);
            Some(p.view())
        } else {
            None
        };
        drop(g);
        crate::server::ws::publish_devices(state);
        if view.is_some() {
            state.devices.audit.record(AuditEntry {
                detail: Some(json!({ "pendingId": id, "reason": "fingerprint" })),
                ..AuditEntry::from_ctx(ctx, "pair.deny")
            });
        }
        return Err(ApiError::unprocessable(
            "fingerprint_mismatch",
            if left == 0 {
                "Those digits do not match the phone. The request was denied.".to_string()
            } else {
                format!("Those digits do not match the ones on the phone. {left} attempt(s) left.")
            },
        )
        .with("attemptsLeft", left));
    }
    // Issue while still holding the pairing lock, so a double-clicked Approve
    // can never mint two devices (lock order: pairing, then devices).
    let (info, token) = state.devices.issue(
        &p.device_name,
        &p.platform,
        p.user_agent.as_deref(),
        p.ingress,
        p.client_ip
            .parse()
            .unwrap_or(std::net::IpAddr::from([0, 0, 0, 0])),
    );
    p.status = PendingStatus::Approved;
    p.decided_at = Some(now);
    p.device_id = Some(info.id.clone());
    p.token = Some(token);
    let pending = p.clone();
    drop(g);
    crate::server::ws::publish_devices(state);
    crate::server::ws::publish_notice(
        state,
        NoticeLevel::Info,
        format!(
            "New device paired: \"{}\" from {} ({}).",
            info.name,
            pending.client_ip,
            pending.ingress.as_str()
        ),
        None,
    );
    state.devices.audit.record(AuditEntry {
        device_id: Some(info.id.clone()),
        device_name: Some(info.name.clone()),
        detail: Some(json!({
            "pendingId": id,
            "clientIp": pending.client_ip,
            "ingress": pending.ingress.as_str(),
            "provider": pending.provider,
        })),
        ..AuditEntry::from_ctx(ctx, "pair.approve")
    });
    Ok(json!({ "ok": true, "device": info }))
}

/// `POST /api/devices/pending/{id}/deny` (desktop).
pub fn deny(state: &AppState, ctx: &ClientContext, id: &str) -> ApiResult<Value> {
    auth::require_desktop(ctx, "Denying a pairing")?;
    let now = now_ms();
    {
        let mut g = state.pairing.lock();
        g.prune(now);
        let Some(p) = g.pending.iter_mut().find(|p| p.id == id) else {
            return Err(ApiError::not_found(
                "not_found",
                "No such pairing request (it may have expired).",
            ));
        };
        if p.status != PendingStatus::Pending {
            return Err(ApiError::conflict(
                "already_decided",
                "This pairing request was already answered.",
            ));
        }
        p.status = PendingStatus::Denied;
        p.decided_at = Some(now);
        p.token = None;
    }
    crate::server::ws::publish_devices(state);
    state.devices.audit.record(AuditEntry {
        detail: Some(json!({ "pendingId": id })),
        ..AuditEntry::from_ctx(ctx, "pair.deny")
    });
    Ok(json!({ "ok": true }))
}

/// Periodic housekeeping: expire pending pairs (tell the desktop).
pub fn sweep(state: &AppState) {
    if state.pairing.sweep() {
        crate::server::ws::publish_devices(state);
    }
}

/// Whether `principal` may create a code for `target` (route-level check,
/// before any body parsing side effects).
pub fn may_create(principal: &Principal, target: CodeTarget) -> bool {
    match target {
        CodeTarget::SelfDevice => matches!(principal, Principal::Device { .. }),
        _ => *principal == Principal::Desktop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_crockford_and_normalise() {
        let c = new_code();
        assert_eq!(c.len(), CODE_LEN);
        assert!(c.bytes().all(|b| CROCKFORD.contains(&b)));
        assert_ne!(new_code(), c);
        assert_eq!(
            normalize_code(&c.to_lowercase()).as_deref(),
            Some(c.as_str())
        );
        assert_eq!(
            normalize_code(&display_code(&c)).as_deref(),
            Some(c.as_str())
        );
        // O→0, I/L→1.
        assert_eq!(
            normalize_code("oooo-iiii-llll").as_deref(),
            Some("000011111111")
        );
        assert_eq!(
            normalize_code("ABCD EFGH JKMN").as_deref(),
            Some("ABCDEFGHJKMN")
        );
        assert!(normalize_code("ABCDEFGHJKMNP").is_none(), "too long");
        assert!(normalize_code("ABCDEFGHJKM").is_none(), "too short");
        assert!(
            normalize_code("ABCDEFGHJKMU").is_none(),
            "U is not Crockford"
        );
        assert!(normalize_code("ABCDEFGHJKM!").is_none());
        assert!(normalize_code("ÀBCDEFGHJKMN").is_none());
        assert_eq!(display_code("ABCDEFGHJKMN"), "ABCD-EFGH-JKMN");
    }

    #[test]
    fn fingerprints_are_four_digits() {
        for _ in 0..50 {
            let f = new_fingerprint();
            assert_eq!(f.len(), 4);
            assert!(f.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn base_urls_are_strict() {
        assert_eq!(
            validate_base_url("https://abc.ngrok-free.app/", true).unwrap(),
            "https://abc.ngrok-free.app"
        );
        assert_eq!(
            validate_base_url("HTTPS://x.ts.net:8443", true).unwrap(),
            "https://x.ts.net:8443"
        );
        assert!(validate_base_url("http://x.example", true).is_err());
        assert!(validate_base_url("http://192.168.1.2:6725", false).is_ok());
        assert!(validate_base_url("https://x.example/path", true).is_err());
        assert!(validate_base_url("https://x.example?q", true).is_err());
        assert!(validate_base_url("https://user@x.example", true).is_err());
        assert!(validate_base_url("javascript://x", false).is_err());
        assert!(validate_base_url("x.example", true).is_err());
    }

    #[test]
    fn prune_expires_pending_and_codes() {
        let mut inner = Inner::default();
        inner.codes.push(Code {
            code: "A".into(),
            target: CodeTarget::Lan,
            device_id: None,
            expires_at: 10,
            used: None,
        });
        inner.pending.push(PendingPair {
            id: "p".into(),
            poll_secret_sha: String::new(),
            fingerprint: "1234".into(),
            device_name: "x".into(),
            platform: String::new(),
            user_agent: None,
            client_ip: "1.2.3.4".into(),
            ingress: Ingress::Tunnel,
            provider: None,
            code_target: CodeTarget::Internet,
            created_at: 0,
            expires_at: 10,
            status: PendingStatus::Pending,
            contested: false,
            fingerprint_attempts: 0,
            decided_at: None,
            device_id: None,
            token: Some("t".into()),
        });
        assert!(inner.prune(11));
        assert!(inner.codes.is_empty());
        assert_eq!(inner.pending[0].status, PendingStatus::Expired);
        assert!(inner.pending[0].token.is_none());
        // Kept pollable for a while, then dropped.
        assert!(!inner.prune(11 + DECIDED_KEEP_MS - 1));
        assert!(inner.prune(11 + DECIDED_KEEP_MS + 1));
        assert!(inner.pending.is_empty());
    }

    #[test]
    fn the_desktop_view_never_carries_secrets() {
        let p = PendingPair {
            id: "p".into(),
            poll_secret_sha: "SECRETSHA".into(),
            fingerprint: "9876".into(),
            device_name: "x".into(),
            platform: String::new(),
            user_agent: None,
            client_ip: "1.2.3.4".into(),
            ingress: Ingress::Tunnel,
            provider: Some("ngrok".into()),
            code_target: CodeTarget::Internet,
            created_at: 0,
            expires_at: 10,
            status: PendingStatus::Pending,
            contested: false,
            fingerprint_attempts: 0,
            decided_at: None,
            device_id: None,
            token: Some("mcd_token".into()),
        };
        let text = p.view().to_string();
        assert!(!text.contains("9876"));
        assert!(!text.contains("SECRETSHA"));
        assert!(!text.contains("mcd_token"));
        assert!(text.contains("\"clientIp\":\"1.2.3.4\""));
    }

    #[test]
    fn who_may_create_which_code() {
        assert!(may_create(&Principal::Desktop, CodeTarget::Lan));
        assert!(may_create(&Principal::Desktop, CodeTarget::Internet));
        assert!(!may_create(&Principal::Desktop, CodeTarget::SelfDevice));
        let dev = Principal::Device {
            id: "d".into(),
            allow_dangerous: true,
        };
        assert!(may_create(&dev, CodeTarget::SelfDevice));
        assert!(!may_create(&dev, CodeTarget::Lan));
        assert!(!may_create(&Principal::LocalScript, CodeTarget::Lan));
    }
}
