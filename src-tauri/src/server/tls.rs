//! TLS for the LAN listeners: a local CA and a leaf certificate it signs.
//!
//! * **Which addresses.** [`lan_candidates`] lists the machine's interface
//!   addresses and keeps only private IPv4 ones: RFC 1918 and 169.254/16, plus
//!   Tailscale's 100.64/10 when opted in. Public IPv4 and every IPv6 address
//!   are never bound; the ones skipped are reported with a reason.
//! * **The local CA** (`certs/ca.pem`, `certs/ca-key.pem`, 0600) is generated
//!   once. It is name-constrained to private address ranges and `localhost`
//!   (Tailscale's 100.64/10 only once a CGNAT address is actually served), and
//!   limited to TLS server authentication (EKU `serverAuth`, which Apple
//!   enforces down the chain), so even a leaked CA key cannot mint a
//!   certificate for a public site, an e-mail address or code signing. A
//!   phone that trusts it (Settings › Profile on iOS, a user CA on Android)
//!   gets a secure context on the LAN: wss, service workers and push work.
//!   Removing the profile on the phone revokes that trust. Only the public
//!   CA certificate is ever served (`/api/pair/ca.crt`,
//!   `/api/pair/ca.mobileconfig`). `certs/ca.json` records what the CA
//!   covers; an older or narrower CA is replaced (the phone then reinstalls
//!   the profile).
//! * **The leaf** (`certs/cert.pem`, `certs/key.pem`) is re-issued whenever its
//!   SANs no longer cover the current addresses, it nears expiry (397-day
//!   validity, Apple's limit), or the CA changed. `certs/leaf.json` records
//!   what it covers.
//!
//! The `ring` rustls provider is used (no aws-lc C toolchain).

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::Path;

use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, SanType,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::remote::{LanAddr, SkippedAddr};

/// Leaf validity (Apple rejects server certificates valid for longer than
/// 398 days).
const LEAF_DAYS: i64 = 397;
/// Re-issue the leaf when fewer days than this remain.
const LEAF_RENEW_DAYS: i64 = 30;
/// CA validity.
const CA_YEARS: i32 = 10;
/// Bumped whenever the CA's constraints change, so an older CA is replaced.
const CA_VERSION: u32 = 2;

/// A PEM cert/key pair plus fingerprints and the public CA.
#[derive(Debug, Clone)]
pub struct CertBundle {
    pub cert_pem: String,
    pub key_pem: String,
    /// SHA-256 of the leaf (what a browser shows).
    pub fingerprint: String,
    pub ca_pem: String,
    pub ca_der: Vec<u8>,
    pub ca_fingerprint: String,
    /// The leaf was (re)issued by this call.
    pub regenerated: bool,
}

/// Private (bindable) IPv4 check: RFC 1918, link-local, and 100.64/10 when
/// `allow_cgnat`.
pub fn is_bindable(ip: IpAddr, allow_cgnat: bool) -> Result<(), &'static str> {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                Err("loopback (served separately)")
            } else if v4.is_private() || v4.is_link_local() {
                Ok(())
            } else if is_cgnat(v4) {
                if allow_cgnat {
                    Ok(())
                } else {
                    Err("Tailscale/CGNAT address (enable \"Also listen on Tailscale\")")
                }
            } else if v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast() {
                Err("not a unicast address")
            } else {
                Err("public address (never bound)")
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                Err("loopback (served separately)")
            } else {
                Err("IPv6 (not bound)")
            }
        }
    }
}

fn is_cgnat(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 100 && (64..128).contains(&o[1])
}

/// Every interface address as `(interface, ip)`.
#[cfg(unix)]
pub fn interface_addrs() -> Vec<(String, IpAddr)> {
    let mut out = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `ifap` with a linked list we free below with
    // freeifaddrs; every pointer is checked for null before it is read, and
    // sockaddr casts follow the family the kernel reported.
    unsafe {
        if libc::getifaddrs(&mut ifap) != 0 {
            return fallback_addrs();
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_addr.is_null() {
                let name = if ifa.ifa_name.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(ifa.ifa_name)
                        .to_string_lossy()
                        .into_owned()
                };
                match i32::from((*ifa.ifa_addr).sa_family) {
                    libc::AF_INET => {
                        let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                        let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                        out.push((name, IpAddr::V4(ip)));
                    }
                    libc::AF_INET6 => {
                        let sin6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                        let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
                        out.push((name, IpAddr::V6(ip)));
                    }
                    _ => {}
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    out
}

#[cfg(not(unix))]
pub fn interface_addrs() -> Vec<(String, IpAddr)> {
    fallback_addrs()
}

/// The UDP-connect trick: the address of the default route (no packet is
/// sent).
fn primary_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip())
}

fn fallback_addrs() -> Vec<(String, IpAddr)> {
    primary_ip()
        .map(|ip| vec![("default".to_string(), ip)])
        .unwrap_or_default()
}

/// Bindable LAN addresses (primary route first) and the skipped ones.
pub fn lan_candidates(port: u16, allow_cgnat: bool) -> (Vec<LanAddr>, Vec<SkippedAddr>) {
    let primary = primary_ip();
    let mut addrs = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (iface, ip) in interface_addrs() {
        if !seen.insert(ip) {
            continue;
        }
        match is_bindable(ip, allow_cgnat) {
            Ok(()) => addrs.push(LanAddr {
                ip: ip.to_string(),
                interface: iface,
                url: format!("https://{ip}:{port}"),
            }),
            Err(reason) => {
                // Loopback and link-local IPv6 noise is not worth reporting.
                let noisy = ip.is_loopback()
                    || matches!(ip, IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) == 0xfe80);
                if !noisy {
                    skipped.push(SkippedAddr {
                        ip: ip.to_string(),
                        interface: iface,
                        reason: reason.to_string(),
                    });
                }
            }
        }
    }
    addrs.sort_by_key(|a| {
        (
            primary.map(|p| p.to_string()) != Some(a.ip.clone()),
            a.interface.clone(),
            a.ip.clone(),
        )
    });
    (addrs, skipped)
}

/// This machine's bindable LAN IPv4 addresses (primary route first).
pub fn local_ips() -> Vec<IpAddr> {
    lan_candidates(0, false)
        .0
        .into_iter()
        .filter_map(|a| a.ip.parse().ok())
        .collect()
}

/// `AA:BB:…` SHA-256 of a DER blob.
pub fn sha256_fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn ymd(dt: chrono::DateTime<chrono::Utc>) -> (i32, u8, u8) {
    use chrono::Datelike;
    (dt.year(), dt.month() as u8, dt.day() as u8)
}

/// The CA's parameters, rebuilt deterministically from its key (the CN
/// carries a short hash of the public key), so the issuer can be recreated
/// from the persisted key without parsing the certificate. `cgnat`: also
/// permit Tailscale's 100.64/10.
fn ca_params(key: &KeyPair, cgnat: bool) -> CertificateParams {
    let tag = &hex(&Sha256::digest(key.public_key_raw()))[..8];
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, format!("Mother Claude Local CA {tag}"));
    dn.push(DnType::OrganizationName, "Mother Claude");
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    // Name constraints cover only IP and DNS names (rcgen has no URI subtree,
    // and an empty rfc822 exclusion is ambiguous), so the EKU is what keeps
    // the CA from vouching for mail, code or clients.
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let private =
        |a: [u8; 4], prefix: u8| GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix(a, prefix));
    let mut permitted = vec![
        private([10, 0, 0, 0], 8),
        private([172, 16, 0, 0], 12),
        private([192, 168, 0, 0], 16),
        private([169, 254, 0, 0], 16),
        private([127, 0, 0, 0], 8),
        GeneralSubtree::DnsName("localhost".into()),
    ];
    if cgnat {
        permitted.push(private([100, 64, 0, 0], 10));
    }
    p.name_constraints = Some(NameConstraints {
        permitted_subtrees: permitted,
        excluded_subtrees: Vec::new(),
    });
    p
}

/// What the persisted CA was minted with (`certs/ca.json`).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CaMeta {
    version: u32,
    cgnat: bool,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct Ca {
    key: KeyPair,
    pem: String,
    der: Vec<u8>,
    cgnat: bool,
}

/// Load the CA, or mint one: when there is none, when it predates the
/// current constraints ([`CA_VERSION`]), or when `cgnat` is needed and it
/// does not cover 100.64/10.
fn load_or_create_ca(dir: &Path, cgnat: bool) -> Result<Ca> {
    let cert_path = dir.join("ca.pem");
    let key_path = dir.join("ca-key.pem");
    let meta_path = dir.join("ca.json");
    let meta: CaMeta = crate::persist::read_json(&meta_path).unwrap_or_default();
    let current = meta.version == CA_VERSION && (meta.cgnat || !cgnat);
    if let (true, Ok(pem), Ok(key_pem)) = (
        current,
        std::fs::read_to_string(&cert_path),
        std::fs::read_to_string(&key_path),
    ) {
        if let (Ok(key), Some(der)) = (KeyPair::from_pem(&key_pem), pem_to_der(&pem)) {
            return Ok(Ca {
                key,
                pem,
                der,
                cgnat: meta.cgnat,
            });
        }
        tracing::warn!("the local CA files are unreadable; generating a new CA");
    } else if cert_path.exists() {
        tracing::info!(
            cgnat,
            "the local CA predates its current constraints; generating a new one (reinstall \
             the profile on phones)"
        );
    }
    let key = KeyPair::generate().context("generate CA key")?;
    let mut params = ca_params(&key, cgnat);
    let now = chrono::Utc::now();
    let (y, m, d) = ymd(now - chrono::Duration::days(1));
    params.not_before = rcgen::date_time_ymd(y, m, d);
    params.not_after = rcgen::date_time_ymd(y + CA_YEARS, m, d.min(28));
    let cert = params.self_signed(&key).context("self-sign CA")?;
    let pem = cert.pem();
    let der = cert.der().to_vec();
    crate::persist::write_private(&key_path, key.serialize_pem().as_bytes())?;
    crate::persist::write_private(&cert_path, pem.as_bytes())?;
    crate::persist::write_json_private(
        &meta_path,
        &CaMeta {
            version: CA_VERSION,
            cgnat,
        },
    )?;
    Ok(Ca {
        key,
        pem,
        der,
        cgnat,
    })
}

/// Decode the first PEM block (base64 body between the armour lines).
fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .ok()
        .filter(|d| !d.is_empty())
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LeafMeta {
    ips: Vec<String>,
    /// Epoch ms.
    not_after: i64,
    ca_fingerprint: String,
}

/// Load (or issue) the leaf for `ips`, signed by the local CA; re-issued
/// when its SANs no longer cover `ips`, it nears expiry, or the CA changed.
pub fn ensure_cert(dir: &Path, ips: &[IpAddr]) -> Result<CertBundle> {
    std::fs::create_dir_all(dir).context("create cert dir")?;
    let cgnat = ips
        .iter()
        .any(|ip| matches!(ip, IpAddr::V4(v4) if is_cgnat(*v4)));
    let ca = load_or_create_ca(dir, cgnat)?;
    let ca_fingerprint = sha256_fingerprint(&ca.der);
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    let meta_path = dir.join("leaf.json");
    let now_ms = chrono::Utc::now().timestamp_millis();

    let meta: Option<LeafMeta> = crate::persist::read_json(&meta_path);
    let covered = meta.as_ref().is_some_and(|m| {
        m.ca_fingerprint == ca_fingerprint
            && m.not_after - now_ms > LEAF_RENEW_DAYS * 86_400_000
            && ips.iter().all(|ip| m.ips.contains(&ip.to_string()))
    });
    if covered {
        if let (Ok(cert_pem), Ok(key_pem)) = (
            std::fs::read_to_string(&cert_path),
            std::fs::read_to_string(&key_path),
        ) {
            if let Some(der) = pem_to_der(&cert_pem) {
                return Ok(CertBundle {
                    fingerprint: sha256_fingerprint(&der),
                    cert_pem,
                    key_pem,
                    ca_pem: ca.pem,
                    ca_der: ca.der,
                    ca_fingerprint,
                    regenerated: false,
                });
            }
        }
    }

    // Issue a new leaf covering localhost, loopback and every LAN address.
    let mut sans = vec![
        SanType::DnsName("localhost".try_into().context("localhost SAN")?),
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    ];
    let mut ip_list: Vec<String> = Vec::new();
    for ip in ips {
        sans.push(SanType::IpAddress(*ip));
        ip_list.push(ip.to_string());
    }
    let key = KeyPair::generate().context("generate leaf key")?;
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Mother Claude");
    dn.push(DnType::OrganizationName, "Mother Claude");
    params.distinguished_name = dn;
    params.subject_alt_names = sans;
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    let now = chrono::Utc::now();
    let (y, m, d) = ymd(now - chrono::Duration::days(1));
    params.not_before = rcgen::date_time_ymd(y, m, d);
    let end = now + chrono::Duration::days(LEAF_DAYS - 1);
    let (y, m, d) = ymd(end);
    params.not_after = rcgen::date_time_ymd(y, m, d);
    let issuer = Issuer::new(ca_params(&ca.key, ca.cgnat), &ca.key);
    let cert = params.signed_by(&key, &issuer).context("sign leaf")?;
    // Serve the chain: leaf then CA.
    let cert_pem = format!("{}{}", cert.pem(), ca.pem);
    let key_pem = key.serialize_pem();
    let fingerprint = sha256_fingerprint(cert.der());
    crate::persist::write_private(&key_path, key_pem.as_bytes())?;
    crate::persist::write_private(&cert_path, cert_pem.as_bytes())?;
    let _ = crate::persist::write_private(&dir.join("fingerprint.txt"), fingerprint.as_bytes());
    crate::persist::write_json_private(
        &meta_path,
        &LeafMeta {
            ips: ip_list,
            not_after: end.timestamp_millis(),
            ca_fingerprint: ca_fingerprint.clone(),
        },
    )?;
    Ok(CertBundle {
        cert_pem,
        key_pem,
        fingerprint,
        ca_pem: ca.pem,
        ca_der: ca.der,
        ca_fingerprint,
        regenerated: true,
    })
}

/// An iOS configuration profile that installs the CA (the user still has to
/// enable full trust in Settings › General › About › Certificate Trust
/// Settings).
pub fn mobileconfig(ca_der: &[u8]) -> String {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(ca_der);
    let digest = Sha256::digest(ca_der);
    let uuid = |salt: u8| {
        let mut b = [0u8; 16];
        b.copy_from_slice(&digest[..16]);
        b[0] ^= salt;
        uuid::Builder::from_random_bytes(b)
            .into_uuid()
            .hyphenated()
            .to_string()
            .to_uppercase()
    };
    let (outer, inner) = (uuid(0), uuid(1));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>PayloadContent</key>
  <array>
    <dict>
      <key>PayloadCertificateFileName</key>
      <string>mother-claude-ca.cer</string>
      <key>PayloadContent</key>
      <data>{b64}</data>
      <key>PayloadDescription</key>
      <string>Lets this device trust Mother Claude on your local network (private addresses only).</string>
      <key>PayloadDisplayName</key>
      <string>Mother Claude Local CA</string>
      <key>PayloadIdentifier</key>
      <string>dev.motherclaude.ca.{inner}</string>
      <key>PayloadType</key>
      <string>com.apple.security.root</string>
      <key>PayloadUUID</key>
      <string>{inner}</string>
      <key>PayloadVersion</key>
      <integer>1</integer>
    </dict>
  </array>
  <key>PayloadDisplayName</key>
  <string>Mother Claude (this Mac)</string>
  <key>PayloadIdentifier</key>
  <string>dev.motherclaude.profile.{outer}</string>
  <key>PayloadRemovalDisallowed</key>
  <false/>
  <key>PayloadType</key>
  <string>Configuration</string>
  <key>PayloadUUID</key>
  <string>{outer}</string>
  <key>PayloadVersion</key>
  <integer>1</integer>
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn only_private_ipv4_is_bindable() {
        for ok in [
            "192.168.1.5",
            "10.0.0.2",
            "172.16.4.4",
            "172.31.255.1",
            "169.254.1.1",
        ] {
            assert!(is_bindable(ip(ok), false).is_ok(), "{ok}");
        }
        for bad in [
            "8.8.8.8",
            "203.0.113.4",
            "172.32.0.1",
            "127.0.0.1",
            "0.0.0.0",
            "::1",
            "2001:db8::1",
            "fd00::1",
        ] {
            assert!(is_bindable(ip(bad), false).is_err(), "{bad}");
        }
        assert!(is_bindable(ip("100.101.2.3"), false).is_err());
        assert!(is_bindable(ip("100.101.2.3"), true).is_ok());
        assert!(is_bindable(ip("100.128.0.1"), true).is_err());
    }

    #[test]
    fn candidates_never_include_public_addresses() {
        let (addrs, _) = lan_candidates(6725, false);
        for a in addrs {
            let parsed: IpAddr = a.ip.parse().unwrap();
            assert!(is_bindable(parsed, false).is_ok(), "{a:?}");
            assert!(a.url.starts_with("https://") && a.url.ends_with(":6725"));
        }
    }

    #[test]
    fn issues_ca_signed_leaf_and_reissues_when_sans_are_stale() {
        let dir = tempfile::tempdir().unwrap();
        let a = ensure_cert(dir.path(), &[ip("192.168.1.5")]).unwrap();
        assert!(a.regenerated);
        assert!(a.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(a.key_pem.contains("PRIVATE KEY"));
        assert!(a.fingerprint.contains(':'));
        assert_ne!(a.fingerprint, a.ca_fingerprint);
        assert_eq!(sha256_fingerprint(&a.ca_der), a.ca_fingerprint);
        // The served chain is leaf + CA.
        assert_eq!(a.cert_pem.matches("BEGIN CERTIFICATE").count(), 2);

        // Same addresses: reused.
        let b = ensure_cert(dir.path(), &[ip("192.168.1.5")]).unwrap();
        assert!(!b.regenerated);
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.cert_pem, b.cert_pem);
        // A subset is still covered.
        assert!(!ensure_cert(dir.path(), &[]).unwrap().regenerated);

        // A new address: a new leaf from the same CA.
        let c = ensure_cert(dir.path(), &[ip("192.168.1.5"), ip("10.0.0.7")]).unwrap();
        assert!(c.regenerated);
        assert_ne!(c.fingerprint, a.fingerprint);
        assert_eq!(c.ca_fingerprint, a.ca_fingerprint);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for f in ["ca-key.pem", "key.pem"] {
                let mode = std::fs::metadata(dir.path().join(f))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600, "{f}");
            }
        }
    }

    /// `needle` occurs in `hay`.
    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn the_ca_is_limited_to_server_auth_and_cgnat_only_when_served() {
        // id-kp-serverAuth, 1.3.6.1.5.5.7.3.1.
        const SERVER_AUTH: [u8; 10] = [0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];
        // iPAddress 100.64.0.0/255.192.0.0 in the name constraints.
        const CGNAT: [u8; 8] = [100, 64, 0, 0, 0xff, 0xc0, 0, 0];
        let dir = tempfile::tempdir().unwrap();
        let a = ensure_cert(dir.path(), &[ip("192.168.1.5")]).unwrap();
        assert!(contains(&a.ca_der, &SERVER_AUTH));
        assert!(
            !contains(&a.ca_der, &CGNAT),
            "no Tailscale range unless needed"
        );
        // Serving a Tailscale address widens the CA once (a new CA).
        let b = ensure_cert(dir.path(), &[ip("192.168.1.5"), ip("100.101.2.3")]).unwrap();
        assert!(contains(&b.ca_der, &CGNAT));
        assert_ne!(a.ca_fingerprint, b.ca_fingerprint);
        // …and is kept afterwards, with or without it.
        let c = ensure_cert(dir.path(), &[ip("192.168.1.5")]).unwrap();
        assert_eq!(c.ca_fingerprint, b.ca_fingerprint);
        // A CA from before these constraints (no ca.json) is replaced.
        std::fs::remove_file(dir.path().join("ca.json")).unwrap();
        let d = ensure_cert(dir.path(), &[ip("192.168.1.5")]).unwrap();
        assert_ne!(d.ca_fingerprint, c.ca_fingerprint);
        assert!(d.regenerated, "a new CA means a new leaf");
        let e = ensure_cert(dir.path(), &[ip("192.168.1.5")]).unwrap();
        assert_eq!(e.ca_fingerprint, d.ca_fingerprint);
    }

    #[test]
    fn the_leaf_is_accepted_by_rustls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();
        let b = ensure_cert(dir.path(), &[ip("192.168.1.5")]).unwrap();
        let certs: Vec<_> = b
            .cert_pem
            .split("-----END CERTIFICATE-----")
            .filter(|s| s.contains("BEGIN"))
            .filter_map(|s| pem_to_der(&format!("{s}-----END CERTIFICATE-----")))
            .map(rustls::pki_types::CertificateDer::from)
            .collect();
        assert_eq!(certs.len(), 2);
        let key = pem_to_der(&b.key_pem).unwrap();
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(key.into());
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs.clone(), key)
            .expect("rustls accepts the chain and key");
        // A client that trusts only the local CA accepts the leaf for the LAN
        // address (the CA's EKU and name constraints allow it) and nothing else.
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certs[1].clone()).unwrap();
        let verifier = rustls::client::WebPkiServerVerifier::builder(std::sync::Arc::new(roots))
            .build()
            .unwrap();
        let verify = |name: &str| {
            verifier.verify_server_cert(
                &certs[0],
                &[],
                &rustls::pki_types::ServerName::try_from(name.to_string()).unwrap(),
                &[],
                rustls::pki_types::UnixTime::now(),
            )
        };
        use rustls::client::danger::ServerCertVerifier as _;
        assert!(verify("192.168.1.5").is_ok());
        assert!(verify("localhost").is_ok());
        assert!(verify("192.168.1.6").is_err());
        assert!(verify("example.com").is_err());
    }

    #[test]
    fn mobileconfig_embeds_the_ca() {
        let dir = tempfile::tempdir().unwrap();
        let b = ensure_cert(dir.path(), &[]).unwrap();
        let profile = mobileconfig(&b.ca_der);
        assert!(profile.contains("com.apple.security.root"));
        use base64::Engine as _;
        assert!(profile.contains(&base64::engine::general_purpose::STANDARD.encode(&b.ca_der)));
        // Deterministic ids: re-downloading replaces the profile.
        assert_eq!(profile, mobileconfig(&b.ca_der));
        assert!(!profile.contains("PRIVATE KEY"));
    }
}
