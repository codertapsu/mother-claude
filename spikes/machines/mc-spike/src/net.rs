//! Endpoints, keys, addresses and the relay settings of D10.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use iroh::{
    address_lookup::{AddrFilter, DnsAddressLookup, PkarrPublisher, PkarrResolver},
    endpoint::{presets, Connection},
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl, SecretKey,
};
use iroh_tickets::{endpoint::EndpointTicket, Ticket};
use serde_json::{json, Value};

use crate::args::Args;

/// The ALPN of the spike's protocol (the product would use `mother-claude/peer/1`).
pub const ALPN: &[u8] = b"mother-claude/spike/0";

#[derive(Clone, Debug)]
pub enum Relay {
    /// n0's public relays, the library's default.
    Public,
    /// Your own relay(s), e.g. `iroh-relay` on the NAS.
    Own(Vec<RelayUrl>),
    /// No relay: direct connections only.
    None,
}

#[derive(Clone, Debug)]
pub struct NetOpts {
    pub relay: Relay,
    /// Publish and resolve through n0's DNS (pkarr) — dial by id alone.
    pub n0: bool,
    /// Find endpoints on the LAN by mDNS.
    pub mdns: bool,
    /// No IP sockets: every packet goes through the relay (measuring it).
    pub relay_only: bool,
    /// Trust a test relay's self-signed certificate.
    #[cfg_attr(not(feature = "local-relay"), allow(dead_code))]
    pub insecure_relay_tls: bool,
}

impl NetOpts {
    pub fn plain(relay: Relay) -> Self {
        Self {
            relay,
            n0: false,
            mdns: false,
            relay_only: false,
            insecure_relay_tls: false,
        }
    }

    pub fn from_args(a: &Args) -> Result<Self> {
        let relay = match a.get("relay").unwrap_or("public") {
            "public" => Relay::Public,
            "none" => Relay::None,
            urls => Relay::Own(
                urls.split(',')
                    .map(|u| RelayUrl::from_str(u.trim()).with_context(|| format!("relay url {u}")))
                    .collect::<Result<_>>()?,
            ),
        };
        // With the public relays, n0's DNS finds a peer's current home relay; with
        // your own relay the peer is dialled as id + that relay, so no third party
        // is asked anything; with none, only the LAN (mDNS) is left.
        let default_lookup = match relay {
            Relay::Public => "n0,mdns",
            Relay::Own(_) | Relay::None => "mdns",
        };
        let lookup = a.get("lookup").unwrap_or(default_lookup);
        Ok(Self {
            n0: lookup.split(',').any(|l| l == "n0"),
            mdns: lookup.split(',').any(|l| l == "mdns"),
            relay_only: a.has("relay-only"),
            insecure_relay_tls: false,
            relay,
        })
    }

    pub fn describe(&self) -> Value {
        json!({
            "relay": match &self.relay {
                Relay::Public => "public".to_string(),
                Relay::Own(u) => u.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(","),
                Relay::None => "none".to_string(),
            },
            "n0Lookup": self.n0,
            "mdns": self.mdns,
            "relayOnly": self.relay_only,
        })
    }
}

pub async fn bind(key: SecretKey, o: &NetOpts, accept: bool) -> Result<Endpoint> {
    let mut b = Endpoint::builder(presets::Minimal).secret_key(key);
    if accept {
        b = b.alpns(vec![ALPN.to_vec()]);
    }
    b = b.relay_mode(match &o.relay {
        Relay::Public => RelayMode::Default,
        Relay::Own(urls) => RelayMode::Custom(RelayMap::from_iter(urls.clone())),
        Relay::None => RelayMode::Disabled,
    });
    if o.n0 {
        b = b
            .address_lookup(PkarrPublisher::n0_dns())
            .address_lookup(PkarrResolver::n0_dns())
            .address_lookup(DnsAddressLookup::n0_dns());
    }
    if o.mdns {
        b = b.address_lookup(iroh_mdns_address_lookup::MdnsAddressLookup::builder());
    }
    if o.relay_only {
        b = b
            .clear_ip_transports()
            .addr_filter(AddrFilter::relay_only());
    }
    #[cfg(feature = "local-relay")]
    if o.insecure_relay_tls {
        b = b.ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify());
    }
    b.bind().await.map_err(|e| anyhow!("bind: {e}"))
}

/// Waits for a home relay when one is configured (bounded).
pub async fn wait_online(ep: &Endpoint, o: &NetOpts, secs: u64) -> Result<()> {
    if matches!(o.relay, Relay::None) {
        return Ok(());
    }
    tokio::time::timeout(Duration::from_secs(secs), ep.online())
        .await
        .map_err(|_| anyhow!("no home relay after {secs}s"))
}

pub fn ticket(ep: &Endpoint) -> String {
    EndpointTicket::new(ep.addr()).encode_string()
}

/// A ticket (id + addresses) or a bare id. A bare id is completed with your own
/// relay when that is the setting; otherwise the address lookups find it.
pub fn target_addr(s: &str, o: &NetOpts) -> Result<EndpointAddr> {
    if let Ok(t) = EndpointTicket::from_str(s) {
        return Ok(t.endpoint_addr().clone());
    }
    let id = EndpointId::from_str(s).map_err(|e| anyhow!("not a ticket or an endpoint id: {e}"))?;
    let mut addr = EndpointAddr::new(id);
    if let Relay::Own(urls) = &o.relay {
        if let Some(u) = urls.first() {
            addr = addr.with_relay_url(u.clone());
        }
    }
    Ok(addr)
}

pub fn paths(conn: &Connection) -> Value {
    let list = conn.paths();
    Value::Array(
        list.iter()
            .map(|p| {
                json!({
                    "selected": p.is_selected(),
                    "kind": if p.is_relay() { "relay" } else if p.is_ip() { "direct" } else { "other" },
                    "addr": format!("{:?}", p.remote_addr()),
                    "rttMs": ms(p.rtt()),
                })
            })
            .collect(),
    )
}

pub fn selected_kind(conn: &Connection) -> &'static str {
    let list = conn.paths();
    for p in list.iter() {
        if p.is_selected() {
            return if p.is_relay() { "relay" } else { "direct" };
        }
    }
    "none"
}

pub fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 100_000.0).round() / 100.0
}

// ---- keys ------------------------------------------------------------------

pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home().join("Library/Application Support")
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/share"))
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn spike_dir() -> PathBuf {
    data_dir().join("mc-spike")
}

pub fn key_path(a: &Args, default_name: &str) -> PathBuf {
    match a.get("key") {
        Some(p) => PathBuf::from(p),
        None => spike_dir().join(format!("{}.key", a.get("name").unwrap_or(default_name))),
    }
}

pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    match fs::read_to_string(path) {
        Ok(s) => {
            let b = hex_decode(s.trim())?;
            let arr: [u8; 32] = b
                .try_into()
                .map_err(|_| anyhow!("{} does not hold a 32-byte key", path.display()))?;
            Ok(SecretKey::from_bytes(&arr))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            let mut o = fs::OpenOptions::new();
            o.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            let mut f = o
                .open(path)
                .with_context(|| format!("creating {}", path.display()))?;
            f.write_all(hex_encode(&key.to_bytes()).as_bytes())?;
            Ok(key)
        }
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn cmd_id(a: &Args) -> Result<()> {
    let path = key_path(a, "spike");
    let key = load_or_create_key(&path)?;
    println!(
        "{}",
        json!({ "id": key.public().to_string(), "keyFile": path })
    );
    Ok(())
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(anyhow!("odd hex length"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| anyhow!("bad hex: {e}")))
        .collect()
}
