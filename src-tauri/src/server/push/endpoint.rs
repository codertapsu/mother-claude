//! Push endpoint validation: the SSRF defence for Web Push.
//!
//! A device (possibly one on the internet) tells us where to POST its
//! notifications. Left unchecked, that is a blind SSRF from the Mac: point it
//! at `http://127.0.0.1:5612` (which then looks genuinely local), a router, a
//! NAS or a metadata IP, and use `/push/test` as a port scanner. So an
//! endpoint must be, both when it is stored and again before every send:
//!
//! * `https`, no userinfo, no explicit port other than 443, no fragment;
//! * a DNS name (never an IP literal, in any spelling the URL parser accepts);
//! * on the browser push services' allowlist ([`ALLOWED_HOSTS`]);
//! * resolving only to public addresses — enforced at send time by
//!   [`PublicOnlyResolver`] (reqwest's DNS hook, so there is no gap between the
//!   check and the connect), and at subscribe time by [`check_resolution`].
//!
//! The sending client also has redirects disabled, a 10 s timeout and no
//! proxy (a proxy would resolve the name itself and bypass the filter).

use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;

/// Longest endpoint URL we store (WNS URLs carry a long token; FCM and Apple
/// are ~200 chars).
pub const MAX_ENDPOINT_LEN: usize = 2048;

/// The browser push services. `*.` entries match any subdomain (not the bare
/// suffix itself).
pub const ALLOWED_HOSTS: &[&str] = &[
    // Safari / WebKit (web.push.apple.com, …)
    "*.push.apple.com",
    // Chrome, Edge on Android, Samsung Internet, Opera, Brave, …
    "fcm.googleapis.com",
    // Firefox
    "updates.push.services.mozilla.com",
    // Edge on Windows (WNS)
    "*.notify.windows.com",
];

/// Why an endpoint was refused. The message is safe to show the device (it
/// never echoes the URL, which is a capability).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointError {
    TooLong,
    Unparseable,
    NotHttps,
    Credentials,
    Fragment,
    IpLiteral,
    Port,
    BadHost,
    NotAllowed,
    PrivateAddress,
}

impl EndpointError {
    pub fn message(&self) -> &'static str {
        match self {
            EndpointError::TooLong => "The push endpoint URL is too long.",
            EndpointError::Unparseable => "The push endpoint is not a valid URL.",
            EndpointError::NotHttps => "The push endpoint must use https.",
            EndpointError::Credentials => "The push endpoint must not contain credentials.",
            EndpointError::Fragment => "The push endpoint must not contain a fragment.",
            EndpointError::IpLiteral => "The push endpoint must be a host name, not an IP address.",
            EndpointError::Port => "The push endpoint must use the default https port.",
            EndpointError::BadHost => "The push endpoint host name is not valid.",
            EndpointError::NotAllowed => {
                "The push endpoint is not a known browser push service \
                 (Apple, Google FCM, Mozilla or Windows)."
            }
            EndpointError::PrivateAddress => {
                "The push endpoint resolves to a private or local address."
            }
        }
    }
}

impl std::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Parse and check an endpoint (everything except DNS). Returns the parsed
/// URL; its serialization is the canonical form we store.
pub fn validate_endpoint(raw: &str) -> Result<Url, EndpointError> {
    let raw = raw.trim();
    if raw.len() > MAX_ENDPOINT_LEN {
        return Err(EndpointError::TooLong);
    }
    let url = Url::parse(raw).map_err(|_| EndpointError::Unparseable)?;
    if url.scheme() != "https" {
        return Err(EndpointError::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(EndpointError::Credentials);
    }
    if url.fragment().is_some() {
        return Err(EndpointError::Fragment);
    }
    // `port()` is None for the scheme default (443).
    if url.port().is_some() {
        return Err(EndpointError::Port);
    }
    // The URL parser already normalised every IPv4 spelling (`127.1`,
    // `0x7f000001`, `2130706433`) to dotted quads and brackets IPv6.
    let host = url.host_str().ok_or(EndpointError::BadHost)?.to_string();
    if host.starts_with('[') || host.parse::<Ipv4Addr>().is_ok() {
        return Err(EndpointError::IpLiteral);
    }
    if !valid_dns_name(&host) {
        return Err(EndpointError::BadHost);
    }
    if !host_allowed(&host) {
        return Err(EndpointError::NotAllowed);
    }
    Ok(url)
}

/// The host of an endpoint for logs (the full URL is a capability: never log
/// it).
pub fn log_host(endpoint: &str) -> String {
    Url::parse(endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "?".into())
}

/// A lowercase LDH DNS name: dot-separated labels of 1–63 `[a-z0-9-]` chars,
/// not starting or ending with `-`, at most 253 chars, no trailing dot, and
/// not all-numeric in the last label (which would be an IPv4 spelling).
pub fn valid_dns_name(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    if !labels.iter().all(|l| label_ok(l)) {
        return false;
    }
    labels
        .last()
        .is_some_and(|tld| !tld.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `host` (already lowercase) is one of the push services.
pub fn host_allowed(host: &str) -> bool {
    ALLOWED_HOSTS
        .iter()
        .any(|pattern| match pattern.strip_prefix("*.") {
            Some(suffix) => host
                .strip_suffix(suffix)
                .and_then(|head| head.strip_suffix('.'))
                .is_some_and(|head| !head.is_empty()),
            None => host == *pattern,
        })
}

/// Whether an address is publicly routable (not loopback, private, link-local,
/// CGNAT, multicast, documentation, reserved, …). IPv4-mapped / NAT64 / 6to4
/// IPv6 addresses are judged by the IPv4 address they embed.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0                                   // 0.0.0.0/8 "this network"
        || (o[0] == 100 && (o[1] & 0xc0) == 64)        // 100.64.0.0/10 CGNAT / Tailscale
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)     // 192.0.0.0/24 IETF protocol
        || (o[0] == 198 && (o[1] & 0xfe) == 18)        // 198.18.0.0/15 benchmarking
        || o[0] >= 240) // 240.0.0.0/4 reserved (+ broadcast)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    if ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
    {
        return false;
    }
    // fec0::/10 deprecated site-local.
    if (s[0] & 0xffc0) == 0xfec0 {
        return false;
    }
    // 2001:db8::/32 documentation; 2001::/32 Teredo (tunnels to anything).
    if s[0] == 0x2001 && (s[1] == 0x0db8 || s[1] == 0) {
        return false;
    }
    // 64:ff9b::/96 NAT64: judge the embedded IPv4.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6].iter().all(|x| *x == 0) {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_public_v4(v4);
    }
    // 2002::/16 6to4: judge the embedded IPv4.
    if s[0] == 0x2002 {
        let v4 = Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8);
        return is_public_v4(v4);
    }
    // ::/96 IPv4-compatible (deprecated) — never a real push service.
    if s[..6].iter().all(|x| *x == 0) {
        return false;
    }
    true
}

// ─────────────────────────────── DNS ────────────────────────────────────────

/// A boxed, sendable future (object-safe trait methods).
pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Name resolution, injectable so tests never touch the network.
pub trait HostLookup: Send + Sync {
    fn lookup(&self, host: String) -> BoxFut<io::Result<Vec<IpAddr>>>;
}

/// The system resolver (`getaddrinfo` via tokio).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemLookup;

impl HostLookup for SystemLookup {
    fn lookup(&self, host: String) -> BoxFut<io::Result<Vec<IpAddr>>> {
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 443)).await?;
            Ok(addrs.map(|a| a.ip()).collect())
        })
    }
}

/// reqwest DNS hook: resolve with `lookup`, keep only public addresses, fail
/// when none is left. Used for every push delivery, so a name that is (or
/// becomes, via rebinding) private can never be connected to.
pub struct PublicOnlyResolver {
    pub lookup: Arc<dyn HostLookup>,
}

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let lookup = self.lookup.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let ips = lookup.lookup(host).await?;
            let public: Vec<SocketAddr> = ips
                .into_iter()
                .filter(|ip| is_public_ip(*ip))
                .map(|ip| SocketAddr::new(ip, 0))
                .collect();
            if public.is_empty() {
                return Err(Box::new(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "push endpoint resolves only to non-public addresses",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: reqwest::dns::Addrs = Box::new(public.into_iter());
            Ok(addrs)
        })
    }
}

/// Subscribe-time DNS check: refuse a name that resolves to ANY non-public
/// address (a hostile resolver or hosts-file entry). A lookup that fails or
/// times out is accepted — the send-time resolver is authoritative, and the
/// allowlisted names are public services.
pub async fn check_resolution(
    lookup: &dyn HostLookup,
    host: &str,
    timeout: Duration,
) -> Result<(), EndpointError> {
    match tokio::time::timeout(timeout, lookup.lookup(host.to_string())).await {
        Ok(Ok(ips)) => {
            if ips.iter().any(|ip| !is_public_ip(*ip)) {
                Err(EndpointError::PrivateAddress)
            } else {
                Ok(())
            }
        }
        Ok(Err(e)) => {
            tracing::debug!(host, error = %e, "push endpoint lookup failed; checked again at send time");
            Ok(())
        }
        Err(_) => {
            tracing::debug!(
                host,
                "push endpoint lookup timed out; checked again at send time"
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_table() {
        let ok = [
            "https://fcm.googleapis.com/fcm/send/abc:def",
            "https://FCM.googleapis.com/wp/xyz",
            "https://web.push.apple.com/QGx9-abc",
            "https://api.push.apple.com/3/device/x",
            "https://updates.push.services.mozilla.com/wpush/v2/gAAAA",
            "https://wns2-par02p.notify.windows.com/w/?token=BQYAAAB",
            "https://fcm.googleapis.com:443/fcm/send/abc",
        ];
        for e in ok {
            assert!(validate_endpoint(e).is_ok(), "{e}");
        }
        let bad: &[(&str, EndpointError)] = &[
            (
                "http://fcm.googleapis.com/fcm/send/abc",
                EndpointError::NotHttps,
            ),
            ("ftp://fcm.googleapis.com/x", EndpointError::NotHttps),
            (
                "https://user:pw@fcm.googleapis.com/x",
                EndpointError::Credentials,
            ),
            (
                "https://user@fcm.googleapis.com/x",
                EndpointError::Credentials,
            ),
            ("https://fcm.googleapis.com/x#frag", EndpointError::Fragment),
            ("https://fcm.googleapis.com:8443/x", EndpointError::Port),
            ("https://127.0.0.1/x", EndpointError::IpLiteral),
            ("https://127.1/x", EndpointError::IpLiteral),
            ("https://0x7f000001/x", EndpointError::IpLiteral),
            ("https://2130706433/x", EndpointError::IpLiteral),
            ("https://[::1]/x", EndpointError::IpLiteral),
            ("https://[::ffff:10.0.0.1]/x", EndpointError::IpLiteral),
            (
                "https://169.254.169.254/latest/meta-data",
                EndpointError::IpLiteral,
            ),
            ("https://localhost/x", EndpointError::BadHost),
            ("https://fcm.googleapis.com./x", EndpointError::BadHost),
            (
                "https://fcm.googleapis.com.evil.com/x",
                EndpointError::NotAllowed,
            ),
            (
                "https://evilfcm.googleapis.com/x",
                EndpointError::NotAllowed,
            ),
            ("https://push.apple.com/x", EndpointError::NotAllowed),
            (
                "https://push.apple.com.evil.org/x",
                EndpointError::NotAllowed,
            ),
            (
                "https://notify.windows.com.attacker.net/x",
                EndpointError::NotAllowed,
            ),
            ("https://example.com/x", EndpointError::NotAllowed),
            ("not a url", EndpointError::Unparseable),
        ];
        for (e, want) in bad {
            assert_eq!(validate_endpoint(e).err().as_ref(), Some(want), "{e}");
        }
        let long = format!(
            "https://fcm.googleapis.com/{}",
            "a".repeat(MAX_ENDPOINT_LEN)
        );
        assert_eq!(validate_endpoint(&long).err(), Some(EndpointError::TooLong));
        // Percent-encoded dots cannot smuggle a different host past the check.
        assert!(validate_endpoint("https://fcm.googleapis.com%2Eevil.com/x").is_err());
        // Canonical form is what we store.
        assert_eq!(
            validate_endpoint("https://FCM.googleapis.com:443/a")
                .unwrap()
                .as_str(),
            "https://fcm.googleapis.com/a"
        );
    }

    #[test]
    fn public_addresses() {
        let public = [
            "142.250.74.10",
            "17.188.128.10",
            "2a00:1450:4001:80b::200a",
            "2002:8efa:4a0a::1",
        ];
        for ip in public {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        let private = [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "192.0.0.8",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "2001:0::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
            "64:ff9b::a00:1",
            "2002:c0a8:0101::1",
            "::127.0.0.1",
        ];
        for ip in private {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    struct Fixed(Vec<IpAddr>);
    impl HostLookup for Fixed {
        fn lookup(&self, _host: String) -> BoxFut<io::Result<Vec<IpAddr>>> {
            let ips = self.0.clone();
            Box::pin(async move { Ok(ips) })
        }
    }
    struct Failing;
    impl HostLookup for Failing {
        fn lookup(&self, _host: String) -> BoxFut<io::Result<Vec<IpAddr>>> {
            Box::pin(async { Err(io::Error::other("offline")) })
        }
    }

    #[tokio::test]
    async fn resolution_checks() {
        let t = Duration::from_secs(1);
        let public = Fixed(vec!["142.250.74.10".parse().unwrap()]);
        assert_eq!(
            check_resolution(&public, "fcm.googleapis.com", t).await,
            Ok(())
        );
        let mixed = Fixed(vec![
            "142.250.74.10".parse().unwrap(),
            "10.0.0.5".parse().unwrap(),
        ]);
        assert_eq!(
            check_resolution(&mixed, "fcm.googleapis.com", t).await,
            Err(EndpointError::PrivateAddress)
        );
        // Offline: accepted now, enforced at send time.
        assert_eq!(
            check_resolution(&Failing, "fcm.googleapis.com", t).await,
            Ok(())
        );

        // The send-time resolver drops private answers and fails when none
        // are left.
        use reqwest::dns::Resolve as _;
        let r = PublicOnlyResolver {
            lookup: Arc::new(mixed),
        };
        let addrs: Vec<SocketAddr> = r
            .resolve("fcm.googleapis.com".parse().unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].ip(), "142.250.74.10".parse::<IpAddr>().unwrap());
        let r = PublicOnlyResolver {
            lookup: Arc::new(Fixed(vec!["127.0.0.1".parse().unwrap()])),
        };
        assert!(r
            .resolve("fcm.googleapis.com".parse().unwrap())
            .await
            .is_err());
    }

    #[test]
    fn log_host_never_leaks_the_path() {
        assert_eq!(
            log_host("https://fcm.googleapis.com/fcm/send/secret-token"),
            "fcm.googleapis.com"
        );
        assert_eq!(log_host("garbage"), "?");
    }
}
