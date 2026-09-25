//! Who is asking, and from where.
//!
//! Every request gets a [`ClientContext`], built once by [`middleware`] from
//! three facts:
//!
//! * **Which listener accepted it** ([`Ingress`]). Each listener's router is
//!   built with an `Extension(Ingress::X)`, so this cannot be forged by the
//!   client.
//! * **The TCP peer**, from axum's `ConnectInfo`.
//! * **The headers**: `Host`, `Origin` and any proxy/tunnel fingerprint.
//!
//! A tunnel (ngrok, cloudflared, Tailscale Funnel) connects from 127.0.0.1, so
//! the peer address alone says nothing about where a request came from. Two
//! predicates are derived from these facts (design §10, B1):
//!
//! * [`is_machine_local`] (`ClientContext::on_machine`): a loopback listener, a
//!   loopback peer, exactly one `Host` naming loopback, and no proxy headers.
//!   `Origin` is ignored. The bridge port's tokenless mode and its any-origin
//!   CORS key on this, so other websites, Postman or Swagger on this machine
//!   keep calling `127.0.0.1:5612` without a token.
//! * [`is_local`] (`ClientContext::local`): machine-local AND no `Origin` or a
//!   local one. It decides only whether the desktop token, the local-script
//!   token and `?token=` are accepted at all, and the 5612 dangerous rule.
//!
//! **Authorization is by principal, not by location.** `local` never grants a
//! `Device` or `Anonymous` principal anything on 6725: a raw TCP forward
//! (`ssh -R`, `ngrok tcp`) can forge `Host: localhost` and add no proxy
//! headers. See [`ClientContext::may_dangerous`].
//!
//! [`effective_ingress`] folds the header evidence back into an ingress for
//! route policy: a request on a loopback listener that carries any proxy
//! header, or one on the LAN listener that carries proxy headers or comes from
//! a non-private address, is treated as [`Ingress::Tunnel`] (reduced router,
//! tunnel pairing rules, the device's internet grant).
//!
//! This module is pure (no I/O) apart from [`middleware`], which asks
//! [`crate::server::auth`] to resolve the presented credential into a
//! [`Principal`] and applies the failed-auth rate limit.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde::Serialize;

use crate::state::AppState;

/// The listener a request arrived on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ingress {
    /// `127.0.0.1:<port>` plain HTTP (desktop webview, local scripts, hooks,
    /// sidecar — and anything tunnelled to 6725).
    Loopback,
    /// The TLS listeners on the machine's private LAN addresses (phones at
    /// home).
    Lan,
    /// `127.0.0.1:<tunnelPort>`, bound only while Internet access is on. Never
    /// local, whatever the headers say.
    Tunnel,
    /// The Claude HTTP bridge's dedicated loopback port (default 5612).
    BridgeLoopback,
}

impl Ingress {
    /// `loopback` | `lan` | `tunnel` | `bridge-loopback`.
    pub fn as_str(self) -> &'static str {
        match self {
            Ingress::Loopback => "loopback",
            Ingress::Lan => "lan",
            Ingress::Tunnel => "tunnel",
            Ingress::BridgeLoopback => "bridge-loopback",
        }
    }
}

/// The authenticated identity behind a request. Filled by the auth middleware.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Principal {
    /// No (valid) credential.
    #[default]
    Anonymous,
    /// The desktop webview's per-launch token (handed out only through the
    /// `server_info` invoke), presented by a genuinely local client.
    Desktop,
    /// A paired phone or browser. `allow_dangerous` is the grant that applies
    /// to *this request's* effective ingress: the LAN grant on the LAN, the
    /// internet grant anywhere else (resolved by `DeviceStore::authenticate`).
    Device { id: String, allow_dangerous: bool },
    /// The persisted on-disk token (`<app data>/token`) kept for local scripts:
    /// accepted only when `local`; data and control routes, never admin routes
    /// (pairing, devices, remote access, plugin install); dangerous actions
    /// only on the bridge port, as for a tokenless local caller there.
    LocalScript,
    /// The foreign-control hook plugin (`mch_…`), local only.
    Hook,
    /// The Path A sidecar (`mcs_…`), local only.
    Sidecar,
}

impl Principal {
    /// Desktop, a paired device or a local script: the principals the dashboard
    /// API serves.
    pub fn is_user(&self) -> bool {
        matches!(
            self,
            Principal::Desktop | Principal::Device { .. } | Principal::LocalScript
        )
    }

    pub fn device_id(&self) -> Option<&str> {
        match self {
            Principal::Device { id, .. } => Some(id),
            _ => None,
        }
    }

    /// Short label for logs and `/api/me`.
    pub fn label(&self) -> &'static str {
        match self {
            Principal::Anonymous => "anonymous",
            Principal::Desktop => "desktop",
            Principal::Device { .. } => "device",
            Principal::LocalScript => "local-script",
            Principal::Hook => "hook",
            Principal::Sidecar => "sidecar",
        }
    }
}

/// Records that a dangerous gate let this request through (read by the audit
/// layer to raise a desktop notice for devices). Shared between the clones of
/// one request's context.
#[derive(Clone, Debug, Default)]
pub struct DangerMarker(Arc<Mutex<Option<String>>>);

impl DangerMarker {
    pub fn set(&self, what: &str) {
        if let Ok(mut g) = self.0.lock() {
            if g.is_none() {
                *g = Some(what.chars().take(200).collect());
            }
        }
    }

    pub fn get(&self) -> Option<String> {
        self.0.lock().ok().and_then(|g| g.clone())
    }
}

/// Everything a handler needs to know about the caller.
#[derive(Clone, Debug)]
pub struct ClientContext {
    /// The listener that accepted the connection.
    pub ingress: Ingress,
    /// The ingress for route policy and grants ([`effective_ingress`]).
    pub effective_ingress: Ingress,
    /// Any proxy/tunnel header is present.
    pub proxied: bool,
    /// Raw TCP peer (`0.0.0.0:0` when the listener did not record one).
    pub peer: SocketAddr,
    /// Tunnel-aware client address: for rate limits, device last-ip and
    /// audit — never for trust decisions.
    pub client_ip: IpAddr,
    /// Machine-local and a local or absent `Origin` ([`is_local`]).
    pub local: bool,
    /// Machine-local, `Origin` ignored ([`is_machine_local`]).
    pub on_machine: bool,
    pub principal: Principal,
    /// `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS`, captured when the context was
    /// built so [`ClientContext::may_dangerous`] needs no state. Applies to
    /// devices on the (effective) LAN only, never to tunnels.
    pub allow_remote_dangerous: bool,
    /// A credential was presented and rejected (already counted by the
    /// failed-auth limiter).
    pub credential_rejected: bool,
    /// Set by the dangerous guards when they let this request through.
    pub danger: DangerMarker,
}

impl ClientContext {
    /// Build a context with an anonymous principal.
    pub fn new(
        ingress: Ingress,
        peer: SocketAddr,
        headers: &HeaderMap,
        allow_remote_dangerous: bool,
    ) -> Self {
        Self {
            ingress,
            effective_ingress: effective_ingress(ingress, peer, headers),
            proxied: has_proxy_headers(headers),
            peer,
            client_ip: client_ip(ingress, peer, headers),
            local: is_local(ingress, peer, headers),
            on_machine: is_machine_local(ingress, peer, headers),
            principal: Principal::Anonymous,
            allow_remote_dangerous,
            credential_rejected: false,
            danger: DangerMarker::default(),
        }
    }

    /// A context that is never local and never authenticated. Used when a
    /// router was built without the context layer (fail closed).
    pub fn unknown() -> Self {
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
        Self {
            ingress: Ingress::Lan,
            effective_ingress: Ingress::Tunnel,
            proxied: false,
            peer,
            client_ip: peer.ip(),
            local: false,
            on_machine: false,
            principal: Principal::Anonymous,
            allow_remote_dangerous: false,
            credential_rejected: false,
            danger: DangerMarker::default(),
        }
    }

    /// `machine_local` under the name used by the design (§10).
    pub fn machine_local(&self) -> bool {
        self.on_machine
    }

    /// Whether this caller may perform a dangerous action (bypassPermissions,
    /// standing permission grants, rm, PTY/tmux keys, transcript purge, …).
    /// Decided by **principal**, never by network position alone:
    ///
    /// * `Desktop` → yes (it can only authenticate from a local client);
    /// * a `Device` → only with the grant for this request's effective ingress
    ///   (`allowDangerous` on the LAN, `allowDangerousInternet` elsewhere —
    ///   already resolved into `allow_dangerous`), or the
    ///   `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS` override on the effective LAN;
    /// * `Anonymous` → only the tokenless bridge port with a genuinely local
    ///   caller (curl, Postman, a localhost page) — never another website;
    /// * `LocalScript` → the same as an anonymous caller: on the bridge port,
    ///   with a genuinely local caller (sending the `token` file must never
    ///   take away what the same request has without it, as it does with
    ///   `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=1`); never on the dashboard port;
    /// * `Hook`, `Sidecar` → never.
    pub fn may_dangerous(&self) -> bool {
        match &self.principal {
            Principal::Desktop => true,
            Principal::Device {
                allow_dangerous, ..
            } => {
                *allow_dangerous
                    || (self.allow_remote_dangerous && self.effective_ingress == Ingress::Lan)
            }
            Principal::Anonymous | Principal::LocalScript => {
                self.ingress == Ingress::BridgeLoopback && self.local
            }
            Principal::Hook | Principal::Sidecar => false,
        }
    }

    /// Note that a dangerous gate let this request through (audit + a desktop
    /// notice when the caller is a device). Call it where you check
    /// [`Self::may_dangerous`] for an action that actually happens.
    pub fn note_dangerous(&self, what: &str) {
        self.danger.set(what);
    }

    pub fn is_desktop(&self) -> bool {
        self.principal == Principal::Desktop
    }

    pub fn device_id(&self) -> Option<&str> {
        self.principal.device_id()
    }

    /// The request reached us through a tunnel or proxy (route policy).
    pub fn is_tunneled(&self) -> bool {
        self.effective_ingress == Ingress::Tunnel
    }
}

/// Handlers may take `ClientContext` directly. A router built without the
/// context layer yields [`ClientContext::unknown`] rather than an error, so a
/// wiring mistake fails closed instead of open.
impl<S: Send + Sync> FromRequestParts<S> for ClientContext {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<ClientContext>()
            .cloned()
            .unwrap_or_else(ClientContext::unknown))
    }
}

/// Exact header names whose presence means a proxy or tunnel is in the path.
pub const PROXY_HEADERS: [&str; 11] = [
    "forwarded",
    "via",
    "x-real-ip",
    "true-client-ip",
    "cdn-loop",
    "x-client-ip",
    "x-cluster-client-ip",
    "fastly-client-ip",
    "fly-client-ip",
    "x-original-forwarded-for",
    "x-envoy-external-address",
];

/// Header-name prefixes that mean a proxy or tunnel is in the path
/// (`x-forwarded-for/-host/-proto/-port…`, Cloudflare `cf-*`, Tailscale
/// `tailscale-*`, ngrok `ngrok-*` except [`NGROK_SKIP_WARNING`]).
pub const PROXY_HEADER_PREFIXES: [&str; 4] = ["x-forwarded-", "cf-", "tailscale-", "ngrok-"];

/// The one `ngrok-` header our own SPA sends; it says nothing about a proxy.
pub const NGROK_SKIP_WARNING: &str = "ngrok-skip-browser-warning";

fn is_proxy_header(name: &str) -> bool {
    if name == NGROK_SKIP_WARNING {
        return false;
    }
    PROXY_HEADERS.contains(&name) || PROXY_HEADER_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Whether any proxy/tunnel fingerprint is present.
pub fn has_proxy_headers(headers: &HeaderMap) -> bool {
    headers.keys().any(|name| is_proxy_header(name.as_str()))
}

/// Split an authority into `(host, port_ok)`. The host keeps no brackets;
/// `port_ok` is false for a malformed port. `None` for garbage.
fn split_authority(authority: &str) -> Option<&str> {
    let a = authority.trim();
    if a.is_empty() {
        return None;
    }
    if let Some(rest) = a.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        return port_ok(after).then_some(host);
    }
    match a.split_once(':') {
        Some((host, port)) => (!port.contains(':') && port_ok_digits(port)).then_some(host),
        None => Some(a),
    }
}

fn port_ok(after: &str) -> bool {
    after.is_empty() || after.strip_prefix(':').is_some_and(port_ok_digits)
}

fn port_ok_digits(p: &str) -> bool {
    !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit())
}

/// Exactly `localhost`, a `127.0.0.0/8` literal, or `::1` (bracketed in an
/// authority). No trailing dots, no `0.0.0.0`, no IPv4-mapped forms.
fn host_name_is_loopback(host: &str, bracketed: bool) -> bool {
    if bracketed {
        return host
            .parse::<Ipv6Addr>()
            .is_ok_and(|ip| ip == Ipv6Addr::LOCALHOST);
    }
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

/// `Host` names loopback: exactly `localhost`, `127.0.0.0/8` or `[::1]`, with
/// an optional numeric port. A missing `Host` (HTTP/1.0) is **not** local.
pub fn host_is_loopback(host: Option<&str>) -> bool {
    let Some(h) = host else { return false };
    let bracketed = h.trim().starts_with('[');
    split_authority(h).is_some_and(|name| host_name_is_loopback(name, bracketed))
}

/// A browser `Origin` that belongs to this machine: `http(s)://localhost[:p]`,
/// `http(s)://127.x.x.x[:p]`, `http(s)://[::1][:p]`, `tauri://localhost`,
/// `http(s)://tauri.localhost`. `null` and anything else are not local.
pub fn origin_is_local(origin: &str) -> bool {
    let origin = origin.trim();
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    // An origin has no path, userinfo, query or fragment.
    if authority.is_empty() || authority.contains(['/', '@', '?', '#', ' ']) {
        return false;
    }
    let bracketed = authority.starts_with('[');
    let Some(host) = split_authority(authority) else {
        return false;
    };
    match scheme.as_str() {
        "tauri" => host.eq_ignore_ascii_case("localhost") && authority == host,
        "http" | "https" => {
            (!bracketed && host.eq_ignore_ascii_case("tauri.localhost"))
                || host_name_is_loopback(host, bracketed)
        }
        _ => false,
    }
}

/// Exactly one `Host` header, naming loopback.
fn single_loopback_host(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(header::HOST).iter();
    match (values.next(), values.next()) {
        (Some(v), None) => host_is_loopback(v.to_str().ok()),
        _ => false,
    }
}

/// `machine_local`: loopback listener, canonical loopback peer, exactly one
/// loopback `Host`, no proxy headers. `Origin` is deliberately not considered.
pub fn is_machine_local(ingress: Ingress, peer: SocketAddr, headers: &HeaderMap) -> bool {
    matches!(ingress, Ingress::Loopback | Ingress::BridgeLoopback)
        && peer.ip().to_canonical().is_loopback()
        && single_loopback_host(headers)
        && !has_proxy_headers(headers)
}

/// Former name of [`is_machine_local`].
pub fn is_on_machine(ingress: Ingress, peer: SocketAddr, headers: &HeaderMap) -> bool {
    is_machine_local(ingress, peer, headers)
}

/// `local`: [`is_machine_local`] and no `Origin`, or exactly one local one.
pub fn is_local(ingress: Ingress, peer: SocketAddr, headers: &HeaderMap) -> bool {
    if !is_machine_local(ingress, peer, headers) {
        return false;
    }
    let mut origins = headers.get_all(header::ORIGIN).iter();
    match (origins.next(), origins.next()) {
        (None, _) => true,
        (Some(o), None) => o.to_str().is_ok_and(origin_is_local),
        _ => false,
    }
}

/// A private address: RFC 1918, IPv4 link-local, IPv6 ULA or link-local.
/// (Tailscale's 100.64/10 is deliberately not included.)
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            let seg = v6.segments()[0];
            (seg & 0xfe00) == 0xfc00 || (seg & 0xffc0) == 0xfe80
        }
    }
}

/// The ingress used for route policy, pairing rules and device grants:
///
/// * the tunnel listener → `Tunnel`;
/// * any proxy header on any listener → `Tunnel`;
/// * the LAN listener with a non-private peer (port forward, public bind) →
///   `Tunnel`;
/// * otherwise the listener's own ingress.
pub fn effective_ingress(ingress: Ingress, peer: SocketAddr, headers: &HeaderMap) -> Ingress {
    if ingress == Ingress::Tunnel || has_proxy_headers(headers) {
        return Ingress::Tunnel;
    }
    if ingress == Ingress::Lan && !is_private_ip(peer.ip()) {
        return Ingress::Tunnel;
    }
    ingress
}

/// All values of a possibly repeated, comma-separated header, in order.
fn header_values(headers: &HeaderMap, name: &str) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The last value of a comma-separated forwarding header (`X-Forwarded-For`,
/// `X-Forwarded-Proto`): the one the nearest proxy appended. Earlier values can
/// be forged by the client.
pub fn last_forwarded(headers: &HeaderMap, name: &str) -> Option<String> {
    header_values(headers, name).pop()
}

/// Parse an address from a forwarding header, tolerating `ip:port`, quotes
/// and bracketed IPv6.
fn parse_forwarded_ip(value: &str) -> Option<IpAddr> {
    let v = value.trim().trim_matches('"');
    v.parse::<IpAddr>()
        .ok()
        .or_else(|| v.parse::<SocketAddr>().ok().map(|s| s.ip()))
        .or_else(|| {
            let host = v.strip_prefix('[').and_then(|r| r.split(']').next());
            host.and_then(|h| h.parse::<IpAddr>().ok())
        })
        .or_else(|| {
            v.rsplit_once(':')
                .and_then(|(h, _)| h.parse::<Ipv4Addr>().ok())
                .map(IpAddr::V4)
        })
}

/// The `for=` of the last element of an RFC 7239 `Forwarded` header.
fn forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    let last = header_values(headers, "forwarded").pop()?;
    last.split(';').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case("for")
            .then(|| parse_forwarded_ip(v))
            .flatten()
    })
}

/// Tunnel-aware client address. When the peer is loopback and ANY proxy
/// header is present: `CF-Connecting-IP`, else the LAST `X-Forwarded-For`
/// value, else `Forwarded: for=`, else the peer. Otherwise the peer address
/// (a LAN peer's own headers are never trusted).
pub fn client_ip(_ingress: Ingress, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    let peer_ip = peer.ip().to_canonical();
    if !peer_ip.is_loopback() || !has_proxy_headers(headers) {
        return peer_ip;
    }
    header_values(headers, "cf-connecting-ip")
        .last()
        .and_then(|v| parse_forwarded_ip(v))
        .or_else(|| last_forwarded(headers, "x-forwarded-for").and_then(|v| parse_forwarded_ip(&v)))
        .or_else(|| forwarded_for(headers))
        .map(|ip| ip.to_canonical())
        .unwrap_or(peer_ip)
}

/// Paths that attempt authentication: a blocked client gets `429` here rather
/// than the SPA's static files, which stay reachable.
fn is_auth_path(path: &str) -> bool {
    path.starts_with("/api/")
        || path == "/api"
        || path == "/ws"
        || path.starts_with("/hooks/")
        || path.starts_with("/v1")
}

/// Build the [`ClientContext`] for a request, resolve its credential into a
/// [`Principal`] and store it (and the [`AppState`]) in the request
/// extensions.
///
/// Idempotent: a request that already carries a context (the `/v1` bridge mount
/// nested inside the main router) keeps it. Never rejects a request itself —
/// the per-route guards in [`crate::server::auth`] do — except the failed-auth
/// rate limit: a client over budget that presents no valid credential gets
/// `429` on API paths. Failures counted: a rejected credential, and any `401`
/// answered to an anonymous, non-machine-local caller. A valid credential or
/// a machine-local request is never limited.
pub async fn middleware(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    if req.extensions().get::<ClientContext>().is_some() {
        return next.run(req).await;
    }
    // Fail closed: a router that forgot its ingress is treated as LAN.
    let ingress = req
        .extensions()
        .get::<Ingress>()
        .copied()
        .unwrap_or(Ingress::Lan);
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0)
        .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
    let mut ctx = ClientContext::new(
        ingress,
        peer,
        req.headers(),
        state.auth.allow_remote_dangerous,
    );
    if let Err(rejection) =
        crate::server::auth::authenticate(&state, &mut ctx, req.headers(), req.uri())
    {
        return rejection.into_response();
    }
    let limited = ctx.principal == Principal::Anonymous && !ctx.on_machine;
    if limited && is_auth_path(req.uri().path()) {
        if let Some(retry) = state.limits.blocked(ctx.client_ip) {
            return crate::server::error::ApiError::too_many_requests(
                retry,
                "Too many failed authentication attempts. Try again later.",
            )
            .into_response();
        }
    }
    let counted = ctx.credential_rejected;
    let client_ip = ctx.client_ip;
    req.extensions_mut().insert(ctx);
    req.extensions_mut().insert(state.clone());
    let response = next.run(req).await;
    if limited && !counted && response.status() == StatusCode::UNAUTHORIZED {
        state.limits.record_failure(client_ip);
    }
    response
}

/// Wrap a router with the ingress marker and the context middleware, in the
/// right order (the `Extension` must be the OUTER layer so the middleware sees
/// it). Every listener's router goes through this exactly once.
pub fn layer(router: Router, state: AppState, ingress: Ingress) -> Router {
    router
        .layer(axum::middleware::from_fn_with_state(state, middleware))
        .layer(axum::Extension(ingress))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    fn peer(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn host_table() {
        let cases: &[(Option<&str>, bool)] = &[
            (Some("127.0.0.1:6725"), true),
            (Some("127.0.0.1"), true),
            (Some("127.5.6.7:80"), true),
            (Some("localhost:6725"), true),
            (Some("LOCALHOST"), true),
            (Some("[::1]:6725"), true),
            (Some("[::1]"), true),
            // Strict: exactly localhost / 127/8 / [::1].
            (Some("localhost."), false),
            (Some("::1"), false),
            (Some("[::ffff:127.0.0.1]:6725"), false),
            (Some("0.0.0.0"), false),
            (Some("0.0.0.0:6725"), false),
            (Some("[::]"), false),
            (Some("[::]:5612"), false),
            (Some("localhost:abc"), false),
            (Some("localhost:"), false),
            (Some("localhost:1:2"), false),
            (Some("abc.ngrok-free.app"), false),
            (Some("192.168.1.5:6725"), false),
            (Some("localhost.evil.com"), false),
            (Some("localhost.evil.example"), false),
            (Some("127.0.0.1.nip.io"), false),
            (Some(""), false),
            (None, false),
        ];
        for (host, want) in cases {
            assert_eq!(host_is_loopback(*host), *want, "host {host:?}");
        }
    }

    #[test]
    fn origin_table() {
        let cases: &[(&str, bool)] = &[
            ("http://localhost:4200", true),
            ("https://localhost", true),
            ("http://127.0.0.1:6725", true),
            ("http://127.9.9.9", true),
            ("http://[::1]:5612", true),
            ("tauri://localhost", true),
            ("http://tauri.localhost", true),
            ("https://tauri.localhost", true),
            ("https://abc.ngrok-free.app", false),
            ("https://evil.example", false),
            ("http://localhost@evil.example", false),
            ("http://localhost.evil.example", false),
            ("http://localhost.evil.com", false),
            ("http://localhost/path", false),
            ("tauri://evil", false),
            ("tauri://localhost:1", false),
            ("file://", false),
            ("null", false),
            ("", false),
        ];
        for (origin, want) in cases {
            assert_eq!(origin_is_local(origin), *want, "origin {origin:?}");
        }
    }

    #[test]
    fn local_predicate_table() {
        use Ingress::*;
        let lh = ("host", "127.0.0.1:6725");
        #[allow(clippy::type_complexity)]
        let cases: Vec<(Ingress, &str, Vec<(&str, &str)>, bool, bool)> = vec![
            // (ingress, peer, headers, local, machine_local)
            (Loopback, "127.0.0.1:5000", vec![lh], true, true),
            (
                BridgeLoopback,
                "127.0.0.1:5000",
                vec![("host", "localhost:5612")],
                true,
                true,
            ),
            // IPv4-mapped IPv6 loopback peer counts.
            (Loopback, "[::ffff:127.0.0.1]:5000", vec![lh], true, true),
            (
                Loopback,
                "[::1]:5000",
                vec![("host", "[::1]:6725")],
                true,
                true,
            ),
            // Desktop webview origin.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("origin", "tauri://localhost")],
                true,
                true,
            ),
            // ngrok http 6725: loopback peer, public Host, XFF.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![
                    ("host", "abc.ngrok-free.app"),
                    ("x-forwarded-for", "203.0.113.9"),
                    ("x-forwarded-proto", "https"),
                ],
                false,
                false,
            ),
            // Host rewritten to loopback but XFF still present.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("x-forwarded-for", "203.0.113.9")],
                false,
                false,
            ),
            // Any x-forwarded-* header, by prefix.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("x-forwarded-port", "443")],
                false,
                false,
            ),
            // Tailscale headers, by prefix.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("tailscale-funnel-request", "?1")],
                false,
                false,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("tailscale-user-name", "x")],
                false,
                false,
            ),
            // Cloudflare, by prefix.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("cf-connecting-ip", "198.51.100.4")],
                false,
                false,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("cf-warp-tag-id", "x")],
                false,
                false,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("cdn-loop", "cloudflare")],
                false,
                false,
            ),
            // Any ngrok-* header (but not the SPA's skip-warning header).
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("ngrok-trace-id", "x")],
                false,
                false,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("ngrok-skip-browser-warning", "1")],
                true,
                true,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("via", "1.1 proxy")],
                false,
                false,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("forwarded", "for=1.2.3.4")],
                false,
                false,
            ),
            // A web page on another origin: on this machine, but not local.
            (
                BridgeLoopback,
                "127.0.0.1:5000",
                vec![
                    ("host", "127.0.0.1:5612"),
                    ("origin", "https://evil.example"),
                ],
                false,
                true,
            ),
            (
                BridgeLoopback,
                "127.0.0.1:5000",
                vec![
                    ("host", "127.0.0.1:5612"),
                    ("origin", "http://localhost.evil.com"),
                ],
                false,
                true,
            ),
            // `Origin: null` (sandboxed frames, file://) is never local.
            (
                BridgeLoopback,
                "127.0.0.1:5000",
                vec![("host", "127.0.0.1:5612"), ("origin", "null")],
                false,
                true,
            ),
            // DNS rebinding: evil hostname resolving to 127.0.0.1.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![("host", "evil.example:6725")],
                false,
                false,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![("host", "localhost.evil.com")],
                false,
                false,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![("host", "127.0.0.1.nip.io:6725")],
                false,
                false,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![("host", "0.0.0.0:6725")],
                false,
                false,
            ),
            (
                Loopback,
                "127.0.0.1:5000",
                vec![("host", "[::]:6725")],
                false,
                false,
            ),
            // A duplicate Host header is never local.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("host", "127.0.0.1:6725")],
                false,
                false,
            ),
            // HTTP/1.0 without Host.
            (Loopback, "127.0.0.1:5000", vec![], false, false),
            // LAN listener, even from the machine's own address.
            (
                Lan,
                "192.168.1.5:5000",
                vec![("host", "192.168.1.5:6725")],
                false,
                false,
            ),
            (Lan, "127.0.0.1:5000", vec![lh], false, false),
            // Tunnel listener is never local.
            (
                Tunnel,
                "127.0.0.1:5000",
                vec![("host", "127.0.0.1:6726")],
                false,
                false,
            ),
            // Non-loopback peer on the loopback listener (should not happen).
            (Loopback, "10.0.0.2:5000", vec![lh], false, false),
        ];
        for (ingress, p, hs, local, machine_local) in cases {
            let h = headers(&hs);
            assert_eq!(
                is_local(ingress, peer(p), &h),
                local,
                "local {ingress:?} {p} {hs:?}"
            );
            assert_eq!(
                is_machine_local(ingress, peer(p), &h),
                machine_local,
                "machine_local {ingress:?} {p} {hs:?}"
            );
        }
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn effective_ingress_table() {
        use Ingress::*;
        let cases: Vec<(Ingress, &str, Vec<(&str, &str)>, Ingress)> = vec![
            (
                Loopback,
                "127.0.0.1:1",
                vec![("host", "127.0.0.1:6725")],
                Loopback,
            ),
            // ngrok http 6725 → Tunnel for route policy.
            (
                Loopback,
                "127.0.0.1:1",
                vec![("host", "a.ngrok.app"), ("x-forwarded-for", "1.2.3.4")],
                Tunnel,
            ),
            (BridgeLoopback, "127.0.0.1:1", vec![("cf-ray", "x")], Tunnel),
            (Tunnel, "127.0.0.1:1", vec![], Tunnel),
            (
                Lan,
                "192.168.1.9:1",
                vec![("host", "192.168.1.5:6725")],
                Lan,
            ),
            (Lan, "10.1.2.3:1", vec![], Lan),
            (Lan, "169.254.3.4:1", vec![], Lan),
            (Lan, "[fd00::5]:1", vec![], Lan),
            (Lan, "[fe80::1]:1", vec![], Lan),
            // A port-forwarded internet peer on the LAN listener.
            (Lan, "203.0.113.9:1", vec![], Tunnel),
            // Tailscale CGNAT peers are not "private" for pairing.
            (Lan, "100.101.1.2:1", vec![], Tunnel),
            // A LAN peer that went through a proxy.
            (Lan, "192.168.1.9:1", vec![("via", "1.1 x")], Tunnel),
        ];
        for (ingress, p, hs, want) in cases {
            assert_eq!(
                effective_ingress(ingress, peer(p), &headers(&hs)),
                want,
                "{ingress:?} {p} {hs:?}"
            );
        }
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn client_ip_table() {
        use Ingress::*;
        let cases: Vec<(Ingress, &str, Vec<(&str, &str)>, &str)> = vec![
            // No proxy: the peer.
            (Loopback, "127.0.0.1:1", vec![], "127.0.0.1"),
            // A LAN peer's own headers are never trusted.
            (
                Lan,
                "192.168.1.9:1",
                vec![("x-forwarded-for", "1.2.3.4")],
                "192.168.1.9",
            ),
            // Any proxy header on a loopback peer: the last XFF value (ngrok
            // appends, so an internet client never shares the desktop's bucket).
            (
                Loopback,
                "127.0.0.1:1",
                vec![("x-forwarded-for", "1.2.3.4")],
                "1.2.3.4",
            ),
            (
                Tunnel,
                "127.0.0.1:1",
                vec![("x-forwarded-for", "6.6.6.6, 203.0.113.9")],
                "203.0.113.9",
            ),
            (
                Tunnel,
                "127.0.0.1:1",
                vec![
                    ("x-forwarded-for", "6.6.6.6"),
                    ("x-forwarded-for", "203.0.113.7"),
                ],
                "203.0.113.7",
            ),
            // Cloudflare wins.
            (
                Loopback,
                "127.0.0.1:1",
                vec![
                    ("cf-connecting-ip", "198.51.100.4"),
                    ("x-forwarded-for", "9.9.9.9"),
                ],
                "198.51.100.4",
            ),
            (
                Loopback,
                "127.0.0.1:1",
                vec![("ngrok-trace-id", "t"), ("x-forwarded-for", "203.0.113.9")],
                "203.0.113.9",
            ),
            (
                Loopback,
                "127.0.0.1:1",
                vec![
                    ("tailscale-funnel-request", "?1"),
                    ("x-forwarded-for", "100.64.0.9"),
                ],
                "100.64.0.9",
            ),
            // RFC 7239 Forwarded, last element.
            (
                Loopback,
                "127.0.0.1:1",
                vec![(
                    "forwarded",
                    "for=1.1.1.1, for=\"[2001:db8::7]:4711\";proto=https",
                )],
                "2001:db8::7",
            ),
            (
                Loopback,
                "127.0.0.1:1",
                vec![("forwarded", "for=192.0.2.60:8080;proto=http")],
                "192.0.2.60",
            ),
            // A proxy header without any address: the peer.
            (Loopback, "127.0.0.1:1", vec![("via", "1.1 x")], "127.0.0.1"),
            // Garbage falls back to the peer.
            (
                Tunnel,
                "127.0.0.1:1",
                vec![("x-forwarded-for", "not-an-ip")],
                "127.0.0.1",
            ),
            // The tunnel listener with no proxy headers at all: the peer.
            (Tunnel, "127.0.0.1:1", vec![], "127.0.0.1"),
            // Mapped peer is canonicalised.
            (Loopback, "[::ffff:127.0.0.1]:1", vec![], "127.0.0.1"),
            // IPv6 client through a tunnel.
            (
                Tunnel,
                "127.0.0.1:1",
                vec![("x-forwarded-for", "2001:db8::1")],
                "2001:db8::1",
            ),
        ];
        for (ingress, p, hs, want) in cases {
            let got = client_ip(ingress, peer(p), &headers(&hs));
            assert_eq!(
                got,
                want.parse::<IpAddr>().unwrap(),
                "{ingress:?} {p} {hs:?}"
            );
        }
    }

    fn device(allow_dangerous: bool) -> Principal {
        Principal::Device {
            id: "d1".into(),
            allow_dangerous,
        }
    }

    #[test]
    fn dangerous_policy_is_by_principal() {
        // The desktop principal (only ever authenticated locally).
        let h = headers(&[("host", "127.0.0.1:6725")]);
        let mut ctx = ClientContext::new(Ingress::Loopback, peer("127.0.0.1:1"), &h, false);
        assert!(ctx.local);
        // Being local grants nothing by itself.
        assert!(!ctx.may_dangerous());
        ctx.principal = Principal::Desktop;
        assert!(ctx.may_dangerous());
        ctx.principal = Principal::LocalScript;
        assert!(!ctx.may_dangerous());
        ctx.principal = Principal::Hook;
        assert!(!ctx.may_dangerous());

        // A device token over a raw TCP forward (`Host: localhost`, loopback
        // peer, no proxy headers) is machine-local — and still gets nothing
        // without its grant.
        ctx.principal = device(false);
        assert!(ctx.on_machine && ctx.local);
        assert!(!ctx.may_dangerous());
        ctx.principal = device(true);
        assert!(ctx.may_dangerous());

        // A paired phone on the LAN: only with a grant.
        let h = headers(&[("host", "192.168.1.5:6725")]);
        ctx = ClientContext::new(Ingress::Lan, peer("192.168.1.9:1"), &h, false);
        ctx.principal = device(false);
        assert!(!ctx.may_dangerous());
        ctx.principal = device(true);
        assert!(ctx.may_dangerous());

        // The env override: devices on the effective LAN only.
        let mut lan = ClientContext::new(Ingress::Lan, peer("192.168.1.9:1"), &h, true);
        assert!(!lan.may_dangerous(), "anonymous never");
        lan.principal = device(false);
        assert!(lan.may_dangerous());
        let mut tun = ClientContext::new(Ingress::Tunnel, peer("127.0.0.1:1"), &h, true);
        tun.principal = device(false);
        assert!(!tun.may_dangerous());
        let mut forwarded = ClientContext::new(Ingress::Lan, peer("203.0.113.9:1"), &h, true);
        forwarded.principal = device(false);
        assert_eq!(forwarded.effective_ingress, Ingress::Tunnel);
        assert!(!forwarded.may_dangerous());

        // The tokenless bridge port: a genuinely local anonymous caller only.
        let h = headers(&[("host", "127.0.0.1:5612")]);
        let mut bridge =
            ClientContext::new(Ingress::BridgeLoopback, peer("127.0.0.1:1"), &h, false);
        assert!(bridge.may_dangerous());
        // Presenting the script token there keeps the same rights.
        bridge.principal = Principal::LocalScript;
        assert!(bridge.may_dangerous());
        bridge.principal = Principal::Hook;
        assert!(!bridge.may_dangerous());
        let h = headers(&[
            ("host", "127.0.0.1:5612"),
            ("origin", "https://example.com"),
        ]);
        let page = ClientContext::new(Ingress::BridgeLoopback, peer("127.0.0.1:1"), &h, false);
        assert!(page.on_machine && !page.local && !page.may_dangerous());
        // Anonymous on 6725 never, even locally.
        let h = headers(&[("host", "127.0.0.1:6725")]);
        let anon = ClientContext::new(Ingress::Loopback, peer("127.0.0.1:1"), &h, true);
        assert!(!anon.may_dangerous());

        assert!(!ClientContext::unknown().may_dangerous());
        assert!(!ClientContext::unknown().local);
    }

    #[test]
    fn danger_marker_is_shared_between_clones() {
        let ctx = ClientContext::unknown();
        let clone = ctx.clone();
        clone.note_dangerous("rm");
        assert_eq!(ctx.danger.get().as_deref(), Some("rm"));
        clone.note_dangerous("second");
        assert_eq!(ctx.danger.get().as_deref(), Some("rm"));
    }

    #[test]
    fn proxy_header_prefixes() {
        for name in [
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-forwarded-port",
            "x-forwarded-prefix",
            "cf-ray",
            "cf-ipcountry",
            "tailscale-user-login",
            "ngrok-trace-id",
            "forwarded",
            "via",
            "x-real-ip",
            "true-client-ip",
            "cdn-loop",
        ] {
            assert!(
                has_proxy_headers(&headers(&[(name, "x")])),
                "{name} should be a proxy header"
            );
        }
        for name in [
            "ngrok-skip-browser-warning",
            "x-forwarded",
            "accept",
            "cookie",
        ] {
            assert!(
                !has_proxy_headers(&headers(&[(name, "x")])),
                "{name} is not a proxy header"
            );
        }
    }
}
