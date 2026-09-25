//! Credentials, principals and the per-route guards (design §2.5).
//!
//! | credential | prefix | accepted | principal |
//! |---|---|---|---|
//! | desktop token (new every launch, in memory, `server_info` invoke only) | none | only when `ctx.local` | `Desktop` |
//! | local-script token (persisted `token` file, 0600) | none | only when `ctx.local` | `LocalScript` |
//! | device token (pairing exchange) | `mcd_` | any ingress | `Device` |
//! | hook token (new every launch; `hook.curlrc`, 0600, for the plugin) | `mch_` | only when `ctx.local` | `Hook` |
//! | sidecar token (per run, child env) | `mcs_` | only when `ctx.local` | `Sidecar` |
//!
//! `LocalScript` reaches data and control routes but never admin routes
//! (pairing, devices, remote access, plugin install) and never dangerous ones.
//!
//! Presentation: `Authorization: Bearer <t>`. `?token=` is accepted **only**
//! when the request is genuinely local (backcompat for local scripts and the
//! bridge's SSE); WebSockets authenticate with their first frame (see
//! `server::ws`). There is no cookie path.
//!
//! [`authenticate`] resolves the credential into `ctx.principal` (called by the
//! context middleware); handlers then pick a guard extractor:
//! [`Authed`] (desktop or device), [`DesktopOnly`], [`HookOnly`],
//! [`SidecarOnly`], [`DangerousOk`] (authed + may do dangerous actions).
//! Rejections use the `{"error":{"code","message"}}` envelope.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Request, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::context::{ClientContext, Principal};
use super::error::ApiError;
use crate::claude::model::Me;
use crate::state::AppState;

/// Prefix of paired-device tokens.
pub const DEVICE_PREFIX: &str = "mcd_";
/// Prefix of the hook plugin's token.
pub const HOOK_PREFIX: &str = "mch_";
/// Prefix of the Path A sidecar's per-run token.
pub const SIDECAR_PREFIX: &str = "mcs_";

/// Credentials carried in [`crate::state::Inner`]. `Debug` never prints a
/// token.
#[derive(Clone)]
pub struct Auth {
    /// Desktop token: a fresh random value every launch, kept in memory and
    /// handed to the webview through the `server_info` invoke only; accepted
    /// only from genuinely local clients.
    pub token: String,
    /// The persisted on-disk token (`<app data>/token`, 0600) kept for local
    /// scripts. Accepted only from genuinely local clients, as
    /// `Principal::LocalScript`.
    pub script_token: String,
    /// `mch_…` token of the foreign-control hook plugin: new every launch,
    /// written to `hook.curlrc` (0600) where the plugin's scripts read it.
    pub hook_token: String,
    /// `mcs_…` token for the Path A sidecar, new every run.
    pub sidecar_token: String,
    /// `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS`: LAN clients may perform
    /// dangerous actions without a per-device grant. Default off.
    pub allow_remote_dangerous: bool,
    /// Persisted-state directory (certs, tokens, devices…). `None` for
    /// ephemeral/test auth, which must never touch the disk.
    pub config_dir: Option<PathBuf>,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("token", &fingerprint(&self.token))
            .field("script_token", &fingerprint(&self.script_token))
            .field("hook_token", &fingerprint(&self.hook_token))
            .field("sidecar_token", &fingerprint(&self.sidecar_token))
            .field("allow_remote_dangerous", &self.allow_remote_dangerous)
            .field("config_dir", &self.config_dir)
            .finish()
    }
}

impl Auth {
    /// Random, in-memory only (tests — no disk writes).
    pub fn ephemeral() -> Self {
        Self {
            token: new_master_token(),
            script_token: new_master_token(),
            hook_token: random_token(HOOK_PREFIX),
            sidecar_token: random_token(SIDECAR_PREFIX),
            allow_remote_dangerous: false,
            config_dir: None,
        }
    }

    /// New desktop and hook tokens for this launch; load the persisted
    /// local-script token from the app data dir, generating and persisting it
    /// (0600) on first run. The hook token reaches the foreign-control plugin
    /// only through `hook.curlrc`, which `claude::hooks::on_start` rewrites at
    /// every launch (the scripts re-read it on every call).
    pub fn load_or_create() -> Self {
        let config_dir = app_config_dir();
        let script_token = config_dir
            .as_deref()
            .map(|d| load_or_create_secret(&d.join("token"), new_master_token))
            .unwrap_or_else(new_master_token);
        // Never the same value as the on-disk token: the desktop credential
        // exists only in this process's memory.
        let mut token = new_master_token();
        while token == script_token {
            token = new_master_token();
        }
        let hook_token = random_token(HOOK_PREFIX);
        if let Some(dir) = config_dir.as_deref() {
            // The hook token used to be persisted; that file is stale now.
            let _ = std::fs::remove_file(dir.join("hook-token"));
        }
        Self {
            token,
            script_token,
            hook_token,
            sidecar_token: random_token(SIDECAR_PREFIX),
            allow_remote_dangerous: std::env::var("MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            config_dir,
        }
    }
}

/// Read a secret file, or generate one with `make`, persist it 0600 and return
/// it. A write failure still returns a usable (in-memory) secret.
fn load_or_create_secret(path: &Path, make: impl Fn() -> String) -> String {
    if let Ok(s) = std::fs::read_to_string(path) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    let t = make();
    if let Err(e) = crate::persist::write_private(path, t.as_bytes()) {
        tracing::warn!(path = %path.display(), error = %e, "could not persist a secret");
    }
    t
}

/// The legacy master-token format: 64 hex chars.
fn new_master_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// `n` bytes from the OS CSPRNG. Falls back to UUIDv4 randomness (also OS
/// randomness) if `getrandom` fails, which it does not on supported platforms.
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if getrandom::fill(&mut buf).is_err() {
        tracing::warn!("getrandom failed; falling back to uuid randomness");
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            out.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        }
        out.truncate(n);
        return out;
    }
    buf
}

/// `<prefix><base64url(32 random bytes)>` — e.g. `mcd_…`, `mch_…`, `mcs_…`.
pub fn random_token(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(32))
    )
}

/// Hex SHA-256 of a token (device tokens are stored only as this).
pub fn sha256_hex(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A short, non-reversible fingerprint safe for logs (`sha256:1a2b3c4d`).
pub fn fingerprint(token: &str) -> String {
    format!("sha256:{}", &sha256_hex(token)[..8])
}

/// `~/Library/Application Support/dev.motherclaude…` (or the platform equivalent).
pub fn app_config_dir() -> Option<PathBuf> {
    crate::persist::app_data_dir()
}

/// The dangerous-action gate: block a dangerous action unless the caller is
/// local or remote-dangerous is allowed. Prefer
/// [`ClientContext::may_dangerous`], which also honours per-device grants; this
/// stays for callers that only have the two booleans.
pub fn dangerous_blocked(dangerous: bool, local: bool, allow_remote: bool) -> bool {
    dangerous && !local && !allow_remote
}

/// Constant-time equality (length leaks, content does not).
pub fn token_matches(expected: &str, presented: &str) -> bool {
    let a = expected.as_bytes();
    let b = presented.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The credential a request presents: `Authorization: Bearer`, or `?token=`
/// when `allow_query` (only genuinely local requests). No cookies.
pub fn presented_token(headers: &HeaderMap, uri: &Uri, allow_query: bool) -> Option<String> {
    if let Some(value) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(s) = value.to_str() {
            let s = s.trim();
            if s.len() > 7 && s[..7].eq_ignore_ascii_case("bearer ") {
                let t = s[7..].trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    if allow_query {
        if let Some(query) = uri.query() {
            for pair in query.split('&') {
                if let Some(v) = pair.strip_prefix("token=") {
                    let t = urldecode(v);
                    if !t.is_empty() {
                        return Some(t);
                    }
                }
            }
        }
    }
    None
}

/// Resolve a presented token into a principal for `ctx`, or `None` when it is
/// not valid *for this caller* (e.g. the desktop token from a tunnel).
///
/// Device tokens are resolved against the request's **effective** ingress, so
/// the device's LAN grant applies only on the LAN and its internet grant
/// everywhere else.
pub fn principal_for_token(
    state: &AppState,
    ctx: &ClientContext,
    token: &str,
) -> Option<Principal> {
    if token.starts_with(DEVICE_PREFIX) {
        return state
            .devices
            .authenticate(token, ctx.client_ip, ctx.effective_ingress)
            .map(|d| Principal::Device {
                id: d.id,
                allow_dangerous: d.allow_dangerous,
            });
    }
    if !ctx.local {
        return None;
    }
    if token.starts_with(HOOK_PREFIX) {
        return token_matches(&state.auth.hook_token, token).then_some(Principal::Hook);
    }
    if token.starts_with(SIDECAR_PREFIX) {
        return token_matches(&state.auth.sidecar_token, token).then_some(Principal::Sidecar);
    }
    if token_matches(&state.auth.token, token) {
        return Some(Principal::Desktop);
    }
    token_matches(&state.auth.script_token, token).then_some(Principal::LocalScript)
}

/// Count one rejected credential against the caller's failed-auth budget,
/// unless the caller is machine-local (never limited). `Err(429)` while the
/// caller is over budget.
fn reject_credential(
    state: &AppState,
    ctx: &mut ClientContext,
    token: &str,
) -> Result<(), ApiError> {
    tracing::debug!(
        ip = %ctx.client_ip,
        ingress = ctx.ingress.as_str(),
        credential = %fingerprint(token),
        "rejected credential"
    );
    if ctx.on_machine {
        return Ok(());
    }
    ctx.credential_rejected = true;
    if let Some(retry) = state.limits.blocked(ctx.client_ip) {
        return Err(ApiError::too_many_requests(
            retry,
            "Too many failed authentication attempts. Try again later.",
        ));
    }
    state.limits.record_failure(ctx.client_ip);
    Ok(())
}

/// Fill `ctx.principal` from the request's credential. Called once per request
/// by the context middleware.
///
/// A presented-but-invalid credential counts against the client ip's
/// failed-auth budget (20/min, then an exponential block); over budget,
/// further invalid credentials get `429` with `Retry-After`. Valid credentials
/// and machine-local callers are never rate limited.
pub fn authenticate(
    state: &AppState,
    ctx: &mut ClientContext,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<(), ApiError> {
    let Some(token) = presented_token(headers, uri, ctx.local) else {
        return Ok(());
    };
    if let Some(p) = principal_for_token(state, ctx, &token) {
        ctx.principal = p;
        return Ok(());
    }
    reject_credential(state, ctx, &token)
}

/// Authenticate a bare token for an already-built context (WebSocket
/// first-frame auth). Applies the same rate limit as [`authenticate`].
pub fn authenticate_token(
    state: &AppState,
    ctx: &mut ClientContext,
    token: &str,
) -> Result<(), ApiError> {
    if let Some(p) = principal_for_token(state, ctx, token) {
        ctx.principal = p;
        return Ok(());
    }
    reject_credential(state, ctx, token)?;
    Err(ApiError::unauthorized("Invalid or revoked token."))
}

fn unauthorized(ctx: &ClientContext) -> ApiError {
    if ctx.is_tunneled() {
        ApiError::unauthorized(
            "This request came through a proxy or tunnel: pair this device (scan the QR code \
             on the desktop) and send its token as `Authorization: Bearer <token>`.",
        )
    } else {
        ApiError::unauthorized(
            "Authentication required: send `Authorization: Bearer <token>`. Pair this device \
             from the desktop to get one.",
        )
    }
}

fn ctx_of(parts: &Parts) -> ClientContext {
    parts
        .extensions
        .get::<ClientContext>()
        .cloned()
        .unwrap_or_else(ClientContext::unknown)
}

/// Desktop (local desktop token), a paired device, or a local script.
#[derive(Debug, Clone)]
pub struct Authed(pub ClientContext);

impl<S: Send + Sync> FromRequestParts<S> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = ctx_of(parts);
        match ctx.principal {
            Principal::Desktop | Principal::Device { .. } | Principal::LocalScript => {
                Ok(Authed(ctx))
            }
            Principal::Anonymous => Err(unauthorized(&ctx)),
            Principal::Hook | Principal::Sidecar => Err(ApiError::forbidden(
                "wrong_credential",
                "This credential is only valid for its own endpoint.",
            )),
        }
    }
}

/// The local desktop only (the per-launch desktop token from a genuinely local
/// client). Admin routes: pairing codes and approvals, devices, remote access,
/// plugin install.
#[derive(Debug, Clone)]
pub struct DesktopOnly(pub ClientContext);

impl<S: Send + Sync> FromRequestParts<S> for DesktopOnly {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = ctx_of(parts);
        match ctx.principal {
            Principal::Desktop => Ok(DesktopOnly(ctx)),
            Principal::Anonymous => Err(unauthorized(&ctx)),
            _ => Err(ApiError::forbidden(
                "desktop_only",
                "This action is only available on the Mother Claude desktop app.",
            )),
        }
    }
}

/// The hook plugin (`mch_…`, local only).
#[derive(Debug, Clone)]
pub struct HookOnly(pub ClientContext);

impl<S: Send + Sync> FromRequestParts<S> for HookOnly {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = ctx_of(parts);
        match ctx.principal {
            Principal::Hook => Ok(HookOnly(ctx)),
            Principal::Anonymous => Err(ApiError::unauthorized(
                "Hook endpoints need the hook token from a local client.",
            )),
            _ => Err(ApiError::forbidden(
                "hook_only",
                "Only the Mother Claude hook plugin may call this endpoint.",
            )),
        }
    }
}

/// The Path A sidecar (`mcs_…`, local only).
#[derive(Debug, Clone)]
pub struct SidecarOnly(pub ClientContext);

impl<S: Send + Sync> FromRequestParts<S> for SidecarOnly {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = ctx_of(parts);
        match ctx.principal {
            Principal::Sidecar => Ok(SidecarOnly(ctx)),
            Principal::Anonymous => Err(ApiError::unauthorized(
                "This endpoint needs the sidecar token from a local client.",
            )),
            _ => Err(ApiError::forbidden(
                "sidecar_only",
                "Only Mother Claude's own sidecar may call this endpoint.",
            )),
        }
    }
}

/// Authed AND allowed to perform dangerous actions (`L` routes: the desktop, or
/// a device the desktop granted dangerous actions for this ingress). Marks the
/// request so the audit layer can notify the desktop when a device used it.
#[derive(Debug, Clone)]
pub struct DangerousOk(pub ClientContext);

impl<S: Send + Sync> FromRequestParts<S> for DangerousOk {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Authed(ctx) = Authed::from_request_parts(parts, state).await?;
        if ctx.may_dangerous() {
            ctx.note_dangerous(parts.uri.path());
            Ok(DangerousOk(ctx))
        } else {
            Err(ApiError::dangerous_blocked(DANGEROUS_BLOCKED_TEXT))
        }
    }
}

/// The refusal text for dangerous actions.
pub const DANGEROUS_BLOCKED_TEXT: &str = "This action is restricted to the Mother Claude \
     desktop app, or to a device you granted dangerous actions in Remote > Devices.";

/// Require the dangerous right for a condition decided inside the handler (e.g.
/// only when the body asks for bypassPermissions). Marks the request when it
/// passes (audit + desktop notice for devices).
pub fn require_dangerous(ctx: &ClientContext, what: &str) -> Result<(), ApiError> {
    if ctx.may_dangerous() {
        ctx.note_dangerous(what);
        Ok(())
    } else {
        Err(ApiError::dangerous_blocked(format!(
            "{what} is restricted to the Mother Claude desktop app, or to a device you \
             granted dangerous actions in Remote > Devices."
        )))
    }
}

/// Require the desktop for a condition decided inside the handler.
pub fn require_desktop(ctx: &ClientContext, what: &str) -> Result<(), ApiError> {
    if ctx.is_desktop() {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "desktop_only",
            format!("{what} is only available on the Mother Claude desktop app."),
        ))
    }
}

/// Defence in depth for the protected `/api` router: whatever guard a handler
/// forgets, an anonymous caller never reaches it, a hook credential never
/// reaches `/api`, and the sidecar credential reaches only its one route.
/// (A local script passes here; `DesktopOnly` keeps it off admin routes.)
pub async fn require_api_principal(req: Request<Body>, next: Next) -> Response {
    let ctx = req
        .extensions()
        .get::<ClientContext>()
        .cloned()
        .unwrap_or_else(ClientContext::unknown);
    match ctx.principal {
        Principal::Desktop | Principal::Device { .. } | Principal::LocalScript => {
            next.run(req).await
        }
        Principal::Sidecar if req.uri().path().ends_with("/permission-request") => {
            next.run(req).await
        }
        Principal::Anonymous => unauthorized(&ctx).into_response(),
        _ => ApiError::forbidden(
            "wrong_credential",
            "This credential is only valid for its own endpoint.",
        )
        .into_response(),
    }
}

/// `GET /api/me` payload for a caller.
pub fn me(state: &AppState, ctx: &ClientContext) -> Me {
    let device = ctx.device_id().and_then(|id| state.devices.get(id));
    Me {
        principal: ctx.principal.label().to_string(),
        device_id: ctx.device_id().map(str::to_string),
        name: device.as_ref().map(|d| d.name.clone()),
        ingress: ctx.ingress.as_str().to_string(),
        local: ctx.local,
        allow_dangerous: ctx.may_dangerous(),
        effective_ingress: Some(ctx.effective_ingress.as_str().to_string()),
        platform: device
            .as_ref()
            .map(|d| d.platform.clone())
            .filter(|p| !p.is_empty()),
        paired_at: device.as_ref().map(|d| d.created_at),
        allow_dangerous_lan: device.as_ref().map(|d| d.allow_dangerous),
        allow_dangerous_internet: device.as_ref().map(|d| d.allow_dangerous_internet),
        client_ip: (!ctx.local).then(|| ctx.client_ip.to_string()),
    }
}

/// Legacy `GET /api/pairing` payload. **Never contains a token** any more: the
/// QR encodes only the base URL; pairing codes (B1, `/api/pair/codes`) replace
/// it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub url: String,
    pub svg: String,
    pub fingerprint: String,
    pub addresses: Vec<String>,
    pub port: u16,
    pub tls: bool,
}

/// Build the legacy pairing info (base URL QR, TLS fingerprint, addresses).
pub fn build_pairing(state: &AppState, fingerprint: &str) -> Pairing {
    let tls = state.config.is_non_loopback();
    let scheme = if tls { "https" } else { "http" };
    let port = state.config.port;
    let ips: Vec<String> = super::tls::local_ips()
        .into_iter()
        .map(|ip| ip.to_string())
        .collect();
    let host = ips
        .first()
        .cloned()
        .unwrap_or_else(|| "localhost".to_string());
    let url = format!("{scheme}://{host}:{port}/#/pair");
    let svg = qr_svg(&url);
    Pairing {
        url,
        svg,
        fingerprint: fingerprint.to_string(),
        addresses: ips,
        port,
        tls,
    }
}

/// Render `data` as an inline SVG QR code (no raster image dependency).
pub fn qr_svg(data: &str) -> String {
    match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(220, 220)
            .quiet_zone(true)
            .build(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::context::Ingress;

    fn uri(s: &str) -> Uri {
        s.parse().unwrap()
    }

    #[test]
    fn token_compare() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc123", "abc124"));
        assert!(!token_matches("abc", "abcd"));
    }

    #[test]
    fn ephemeral_tokens_are_long_unique_and_prefixed() {
        let a = Auth::ephemeral();
        let b = Auth::ephemeral();
        assert_eq!(a.token.len(), 64);
        assert_ne!(a.token, b.token);
        assert_ne!(a.token, a.script_token);
        assert!(a.hook_token.starts_with(HOOK_PREFIX));
        assert!(a.sidecar_token.starts_with(SIDECAR_PREFIX));
        // 32 bytes base64url without padding = 43 chars.
        assert_eq!(a.hook_token.len(), HOOK_PREFIX.len() + 43);
        assert!(a.config_dir.is_none());
        // Debug never prints a token.
        let dbg = format!("{a:?}");
        assert!(!dbg.contains(&a.token));
        assert!(!dbg.contains(&a.script_token));
        assert!(!dbg.contains(&a.hook_token));
    }

    #[test]
    fn secrets_persist_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hook-token");
        let first = load_or_create_secret(&path, || random_token(HOOK_PREFIX));
        let second = load_or_create_secret(&path, || random_token(HOOK_PREFIX));
        assert_eq!(first, second);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn presentation_rules() {
        let mut h = HeaderMap::new();
        assert_eq!(
            presented_token(&h, &uri("/api/x?token=abc"), true).as_deref(),
            Some("abc")
        );
        // ?token= only for local callers.
        assert_eq!(presented_token(&h, &uri("/api/x?token=abc"), false), None);
        h.insert("authorization", "Bearer  tok ".parse().unwrap());
        assert_eq!(
            presented_token(&h, &uri("/api/x"), false).as_deref(),
            Some("tok")
        );
        h.insert("authorization", "bearer lower".parse().unwrap());
        assert_eq!(
            presented_token(&h, &uri("/"), false).as_deref(),
            Some("lower")
        );
        // Cookies are not a credential any more.
        let mut c = HeaderMap::new();
        c.insert("cookie", "mc_token=abc".parse().unwrap());
        assert_eq!(presented_token(&c, &uri("/"), true), None);
        assert_eq!(urldecode("ab%20cd"), "ab cd");
        assert_eq!(urldecode("trail%2"), "trail%2");
    }

    fn state() -> AppState {
        crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            Auth::ephemeral(),
        )
    }

    fn ctx(local: bool) -> ClientContext {
        let mut h = HeaderMap::new();
        if local {
            h.insert("host", "127.0.0.1:6725".parse().unwrap());
        } else {
            h.insert("host", "x.ngrok-free.app".parse().unwrap());
            h.insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
        }
        ClientContext::new(Ingress::Loopback, "127.0.0.1:1".parse().unwrap(), &h, false)
    }

    #[test]
    fn principals_by_credential_and_locality() {
        let st = state();
        let master = st.auth.token.clone();
        let script = st.auth.script_token.clone();
        let hook = st.auth.hook_token.clone();
        let sidecar = st.auth.sidecar_token.clone();
        assert_ne!(master, script);
        assert_eq!(
            principal_for_token(&st, &ctx(true), &master),
            Some(Principal::Desktop)
        );
        assert_eq!(principal_for_token(&st, &ctx(false), &master), None);
        assert_eq!(
            principal_for_token(&st, &ctx(true), &script),
            Some(Principal::LocalScript)
        );
        assert_eq!(principal_for_token(&st, &ctx(false), &script), None);
        assert_eq!(
            principal_for_token(&st, &ctx(true), &hook),
            Some(Principal::Hook)
        );
        assert_eq!(principal_for_token(&st, &ctx(false), &hook), None);
        assert_eq!(
            principal_for_token(&st, &ctx(true), &sidecar),
            Some(Principal::Sidecar)
        );
        assert_eq!(principal_for_token(&st, &ctx(false), &sidecar), None);
        assert_eq!(principal_for_token(&st, &ctx(true), "mcd_unknown"), None);
        assert_eq!(principal_for_token(&st, &ctx(true), "wrong"), None);
    }

    #[test]
    fn failed_auth_is_rate_limited() {
        let st = state();
        let mut h = HeaderMap::new();
        h.insert("host", "192.168.1.2:6725".parse().unwrap());
        h.insert("authorization", "Bearer mcd_bogus".parse().unwrap());
        let mut last = Ok(());
        for _ in 0..25 {
            let mut c =
                ClientContext::new(Ingress::Lan, "192.168.1.50:1".parse().unwrap(), &h, false);
            last = authenticate(&st, &mut c, &h, &uri("/api/sessions"));
            assert_eq!(c.principal, Principal::Anonymous);
        }
        let err = last.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
        // A different client is unaffected.
        let mut other =
            ClientContext::new(Ingress::Lan, "192.168.1.51:1".parse().unwrap(), &h, false);
        assert!(authenticate(&st, &mut other, &h, &uri("/api/sessions")).is_ok());
        assert!(other.credential_rejected);

        // A machine-local caller is never limited, however many bad tokens.
        let mut lh = HeaderMap::new();
        lh.insert("host", "127.0.0.1:6725".parse().unwrap());
        lh.insert("authorization", "Bearer wrong".parse().unwrap());
        for _ in 0..40 {
            let mut c = ClientContext::new(
                Ingress::Loopback,
                "127.0.0.1:1".parse().unwrap(),
                &lh,
                false,
            );
            assert!(authenticate(&st, &mut c, &lh, &uri("/api/sessions")).is_ok());
            assert!(!c.credential_rejected);
        }
        // A valid credential from a blocked address still works.
        let mut good = HeaderMap::new();
        good.insert("host", "192.168.1.2:6725".parse().unwrap());
        let mut c = ClientContext::new(
            Ingress::Lan,
            "192.168.1.50:1".parse().unwrap(),
            &good,
            false,
        );
        assert!(st.limits.blocked(c.client_ip).is_some());
        let (id, token) = st.devices.issue_for_test("phone");
        assert!(authenticate_token(&st, &mut c, &token).is_ok());
        assert_eq!(c.principal.device_id(), Some(id.as_str()));
    }

    #[test]
    fn dangerous_gate() {
        assert!(!dangerous_blocked(false, false, false));
        assert!(!dangerous_blocked(true, true, false));
        assert!(dangerous_blocked(true, false, false));
        assert!(!dangerous_blocked(true, false, true));
    }

    #[test]
    fn qr_svg_is_svg_and_pairing_has_no_token() {
        let svg = qr_svg("https://example.test/#/pair");
        assert!(svg.contains("<svg"));
        let st = state();
        let p = build_pairing(&st, "n/a");
        assert!(!p.url.contains(&st.auth.token));
        assert!(!serde_json::to_string(&p).unwrap().contains(&st.auth.token));
    }

    #[test]
    fn fingerprints_do_not_reveal_tokens() {
        let f = fingerprint("secret-token");
        assert!(f.starts_with("sha256:"));
        assert_eq!(f.len(), "sha256:".len() + 8);
        assert!(!f.contains("secret"));
        assert_eq!(sha256_hex("abc").len(), 64);
    }
}
