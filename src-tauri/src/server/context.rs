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
//! the peer address alone says nothing about where a request came from. The
//! strict [`is_local`] predicate therefore also requires a loopback `Host`, no
//! forwarding headers and a local (or absent) `Origin`. Only a request that
//! passes it may use the desktop master token, `?token=`, or a dangerous action
//! without a per-device grant.
//!
//! This module is pure (no I/O, no state) apart from [`middleware`], which asks
//! [`crate::server::auth`] to resolve the presented credential into a
//! [`Principal`].

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap};
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
    /// The TLS listeners on the machine's LAN addresses (phones at home).
    Lan,
    /// `127.0.0.1:<tunnelPort>`, bound only while Internet access is on. Never
    /// local, whatever the headers say.
    Tunnel,
    /// The Claude HTTP bridge's dedicated loopback port (default 5612).
    BridgeLoopback,
}

/// The authenticated identity behind a request. Filled by the auth middleware.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Principal {
    /// No (valid) credential.
    #[default]
    Anonymous,
    /// The desktop master token, presented by a genuinely local client.
    Desktop,
    /// A paired phone or browser.
    Device { id: String, allow_dangerous: bool },
    /// The foreign-control hook plugin (`mch_…`), local only.
    Hook,
    /// The Path A sidecar (`mcs_…`), local only.
    Sidecar,
}

impl Principal {
    /// Desktop or a paired device: the principals the dashboard API serves.
    pub fn is_user(&self) -> bool {
        matches!(self, Principal::Desktop | Principal::Device { .. })
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
            Principal::Hook => "hook",
            Principal::Sidecar => "sidecar",
        }
    }
}

/// Everything a handler needs to know about the caller.
#[derive(Clone, Debug)]
pub struct ClientContext {
    pub ingress: Ingress,
    /// Raw TCP peer (`0.0.0.0:0` when the listener did not record one).
    pub peer: SocketAddr,
    /// Tunnel-aware client address (§2.4): for rate limits, device last-ip and
    /// audit — never for trust decisions.
    pub client_ip: IpAddr,
    /// The strict local predicate (§2.2), `Origin` included.
    pub local: bool,
    /// Like `local` but ignoring `Origin`: an unproxied connection from this
    /// machine to a loopback listener. This is what the bridge's tokenless mode
    /// keys on — a page on another website calling `127.0.0.1:5612` stays
    /// tokenless for non-dangerous routes (the user asked for exactly that),
    /// while dangerous actions still need the full `local`.
    pub on_machine: bool,
    pub principal: Principal,
    /// `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS`, captured when the context was
    /// built so [`ClientContext::may_dangerous`] needs no state.
    pub allow_remote_dangerous: bool,
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
            peer,
            client_ip: client_ip(ingress, peer, headers),
            local: is_local(ingress, peer, headers),
            on_machine: is_on_machine(ingress, peer, headers),
            principal: Principal::Anonymous,
            allow_remote_dangerous,
        }
    }

    /// A context that is never local and never authenticated. Used when a
    /// router was built without the context layer (fail closed).
    pub fn unknown() -> Self {
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
        Self {
            ingress: Ingress::Lan,
            peer,
            client_ip: peer.ip(),
            local: false,
            on_machine: false,
            principal: Principal::Anonymous,
            allow_remote_dangerous: false,
        }
    }

    /// Whether this caller may perform a dangerous action (bypassPermissions,
    /// standing permission grants, rm, PTY injection, transcript purge, …).
    ///
    /// * genuinely local → yes;
    /// * a non-local request on the bridge's tokenless port → never;
    /// * a device the desktop granted `allowDangerous` → yes;
    /// * the `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS` override → LAN only (it
    ///   predates tunnels and must not silently extend to the internet).
    pub fn may_dangerous(&self) -> bool {
        if self.local {
            return true;
        }
        if self.ingress == Ingress::BridgeLoopback {
            return false;
        }
        if let Principal::Device {
            allow_dangerous: true,
            ..
        } = self.principal
        {
            return true;
        }
        self.allow_remote_dangerous && self.ingress == Ingress::Lan
    }

    pub fn is_desktop(&self) -> bool {
        self.principal == Principal::Desktop
    }

    pub fn device_id(&self) -> Option<&str> {
        self.principal.device_id()
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

/// Headers whose presence means a proxy or tunnel is in the path.
pub const PROXY_HEADERS: [&str; 12] = [
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-real-ip",
    "cf-connecting-ip",
    "cf-ray",
    "cf-visitor",
    "true-client-ip",
    "tailscale-funnel-request",
    "tailscale-user-login",
    "via",
];

/// The one `ngrok-` header our own SPA sends; it says nothing about a proxy.
pub const NGROK_SKIP_WARNING: &str = "ngrok-skip-browser-warning";

/// Whether any proxy/tunnel fingerprint is present: one of
/// [`PROXY_HEADERS`], or any `ngrok-*` header except
/// [`NGROK_SKIP_WARNING`].
pub fn has_proxy_headers(headers: &HeaderMap) -> bool {
    headers.keys().any(|name| {
        let n = name.as_str();
        PROXY_HEADERS.contains(&n) || (n.starts_with("ngrok-") && n != NGROK_SKIP_WARNING)
    })
}

/// Whether a provider-specific header (Cloudflare, Tailscale, ngrok) is present
/// — the signal that forwarding headers were written by a tunnel we know how to
/// read, not by an arbitrary client.
fn has_provider_headers(headers: &HeaderMap) -> bool {
    headers.keys().any(|name| {
        let n = name.as_str();
        n == "cf-connecting-ip"
            || n == "cf-ray"
            || n.starts_with("tailscale-")
            || (n.starts_with("ngrok-") && n != NGROK_SKIP_WARNING)
    })
}

/// Split `host[:port]`, handling bracketed IPv6 (`[::1]:6725`).
fn strip_port(host: &str) -> &str {
    let host = host.trim();
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or_default();
    }
    // A bare IPv6 literal has several colons and no port we could strip safely.
    if host.matches(':').count() > 1 {
        return host;
    }
    host.split(':').next().unwrap_or_default()
}

/// Whether a hostname (no port) names this machine's loopback.
fn hostname_is_loopback(name: &str) -> bool {
    let name = name.trim_end_matches('.');
    if name.eq_ignore_ascii_case("localhost") {
        return true;
    }
    name.parse::<IpAddr>()
        .map(|ip| ip.to_canonical().is_loopback())
        .unwrap_or(false)
}

/// `Host` names loopback: `localhost`, `127.0.0.0/8` or `[::1]`, any port. A
/// missing `Host` (HTTP/1.0) is **not** local.
pub fn host_is_loopback(host: Option<&str>) -> bool {
    match host {
        Some(h) if !h.trim().is_empty() => hostname_is_loopback(strip_port(h)),
        _ => false,
    }
}

/// A browser `Origin` that belongs to this machine: `http(s)://localhost[:p]`,
/// `http(s)://127.x.x.x[:p]`, `http(s)://[::1][:p]`, `tauri://localhost`,
/// `http(s)://tauri.localhost`.
pub fn origin_is_local(origin: &str) -> bool {
    let origin = origin.trim().trim_end_matches('/');
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split('/').next().unwrap_or_default();
    // Refuse userinfo tricks (`http://localhost@evil.example`).
    if authority.contains('@') {
        return false;
    }
    let host = strip_port(authority);
    match scheme.as_str() {
        "tauri" => host.eq_ignore_ascii_case("localhost"),
        "http" | "https" => {
            host.eq_ignore_ascii_case("tauri.localhost") || hostname_is_loopback(host)
        }
        _ => false,
    }
}

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Loopback listener, loopback peer, loopback `Host`, no proxy headers.
/// `Origin` is deliberately not considered (see [`ClientContext::on_machine`]).
pub fn is_on_machine(ingress: Ingress, peer: SocketAddr, headers: &HeaderMap) -> bool {
    matches!(ingress, Ingress::Loopback | Ingress::BridgeLoopback)
        && peer.ip().to_canonical().is_loopback()
        && host_is_loopback(header_str(headers, header::HOST))
        && !has_proxy_headers(headers)
}

/// The strict local predicate (§2.2): [`is_on_machine`] plus a local or absent
/// `Origin`.
pub fn is_local(ingress: Ingress, peer: SocketAddr, headers: &HeaderMap) -> bool {
    is_on_machine(ingress, peer, headers)
        && header_str(headers, header::ORIGIN).is_none_or(origin_is_local)
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

/// Parse an address from a forwarding header, tolerating `ip:port` and
/// bracketed IPv6.
fn parse_forwarded_ip(value: &str) -> Option<IpAddr> {
    let v = value.trim().trim_matches('"');
    v.parse::<IpAddr>()
        .ok()
        .or_else(|| v.parse::<SocketAddr>().ok().map(|s| s.ip()))
        .or_else(|| strip_port(v).parse::<IpAddr>().ok())
}

/// Tunnel-aware client address (§2.4). Forwarding headers are trusted only when
/// the peer is loopback AND (the request came in on the tunnel listener OR a
/// provider header is present). Cloudflare → `CF-Connecting-IP`; ngrok and
/// Tailscale → the LAST `X-Forwarded-For` value. Otherwise the peer address.
pub fn client_ip(ingress: Ingress, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    let peer_ip = peer.ip().to_canonical();
    if !peer_ip.is_loopback() || !(ingress == Ingress::Tunnel || has_provider_headers(headers)) {
        return peer_ip;
    }
    if let Some(ip) = header_values(headers, "cf-connecting-ip")
        .last()
        .and_then(|v| parse_forwarded_ip(v))
    {
        return ip.to_canonical();
    }
    last_forwarded(headers, "x-forwarded-for")
        .and_then(|v| parse_forwarded_ip(&v))
        .map(|ip| ip.to_canonical())
        .unwrap_or(peer_ip)
}

/// Build the [`ClientContext`] for a request, resolve its credential into a
/// [`Principal`] and store it in the request extensions.
///
/// Idempotent: a request that already carries a context (the `/v1` bridge mount
/// nested inside the main router) keeps it. Never rejects a request itself —
/// the per-route guards in [`crate::server::auth`] do — except that a client
/// over the failed-auth rate limit gets `429`.
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
    req.extensions_mut().insert(ctx);
    next.run(req).await
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
            (Some("localhost."), true),
            (Some("[::1]:6725"), true),
            (Some("[::1]"), true),
            (Some("::1"), true),
            (Some("[::ffff:127.0.0.1]:6725"), true),
            (Some("abc.ngrok-free.app"), false),
            (Some("192.168.1.5:6725"), false),
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
            ("tauri://evil", false),
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
            // (ingress, peer, headers, local, on_machine)
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
            // Tailscale Funnel header.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("tailscale-funnel-request", "?1")],
                false,
                false,
            ),
            // Cloudflare.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![lh, ("cf-connecting-ip", "198.51.100.4")],
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
            // DNS rebinding: evil hostname resolving to 127.0.0.1.
            (
                Loopback,
                "127.0.0.1:5000",
                vec![("host", "evil.example:6725")],
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
        for (ingress, p, hs, local, on_machine) in cases {
            let h = headers(&hs);
            assert_eq!(
                is_local(ingress, peer(p), &h),
                local,
                "local {ingress:?} {p} {hs:?}"
            );
            assert_eq!(
                is_on_machine(ingress, peer(p), &h),
                on_machine,
                "on_machine {ingress:?} {p} {hs:?}"
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
            (
                Lan,
                "192.168.1.9:1",
                vec![("x-forwarded-for", "1.2.3.4")],
                "192.168.1.9",
            ),
            // Plain XFF on the loopback listener with no provider header: a
            // local client could have written it — not trusted.
            (
                Loopback,
                "127.0.0.1:1",
                vec![("x-forwarded-for", "1.2.3.4")],
                "127.0.0.1",
            ),
            // Tunnel listener: last XFF value (ngrok appends).
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
            // Cloudflare on the loopback listener.
            (
                Loopback,
                "127.0.0.1:1",
                vec![
                    ("cf-connecting-ip", "198.51.100.4"),
                    ("x-forwarded-for", "9.9.9.9"),
                ],
                "198.51.100.4",
            ),
            // ngrok provider header on 6725.
            (
                Loopback,
                "127.0.0.1:1",
                vec![("ngrok-trace-id", "t"), ("x-forwarded-for", "203.0.113.9")],
                "203.0.113.9",
            ),
            // Tailscale.
            (
                Loopback,
                "127.0.0.1:1",
                vec![
                    ("tailscale-funnel-request", "?1"),
                    ("x-forwarded-for", "100.64.0.9"),
                ],
                "100.64.0.9",
            ),
            // Garbage falls back to the peer.
            (
                Tunnel,
                "127.0.0.1:1",
                vec![("x-forwarded-for", "not-an-ip")],
                "127.0.0.1",
            ),
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

    #[test]
    fn dangerous_policy() {
        let h = headers(&[("host", "127.0.0.1:6725")]);
        let mut ctx = ClientContext::new(Ingress::Loopback, peer("127.0.0.1:1"), &h, false);
        assert!(ctx.local && ctx.may_dangerous());

        // A paired phone on the LAN: only with a per-device grant.
        let h = headers(&[("host", "192.168.1.5:6725")]);
        ctx = ClientContext::new(Ingress::Lan, peer("192.168.1.9:1"), &h, false);
        ctx.principal = Principal::Device {
            id: "d1".into(),
            allow_dangerous: false,
        };
        assert!(!ctx.may_dangerous());
        ctx.principal = Principal::Device {
            id: "d1".into(),
            allow_dangerous: true,
        };
        assert!(ctx.may_dangerous());

        // The env override applies to the LAN only.
        let mut lan = ClientContext::new(Ingress::Lan, peer("192.168.1.9:1"), &h, true);
        assert!(lan.may_dangerous());
        lan.ingress = Ingress::Tunnel;
        assert!(!lan.may_dangerous());

        // Non-local on the bridge port: never, grant or not.
        let h = headers(&[("host", "x.ngrok-free.app"), ("x-forwarded-for", "1.2.3.4")]);
        let mut bridge = ClientContext::new(Ingress::BridgeLoopback, peer("127.0.0.1:1"), &h, true);
        bridge.principal = Principal::Device {
            id: "d".into(),
            allow_dangerous: true,
        };
        assert!(!bridge.may_dangerous());

        assert!(!ClientContext::unknown().may_dangerous());
        assert!(!ClientContext::unknown().local);
    }
}
