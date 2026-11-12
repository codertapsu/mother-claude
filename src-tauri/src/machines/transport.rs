//! The peer transport (docs/PLAN-MACHINES.md §5.3, D7): one iroh endpoint
//! per machine, addressed by the machine key — direct on the local network,
//! hole-punched or relayed across networks, end-to-end encrypted.
//!
//! * **Started only when Machines is in use** (a pairing, or a paired
//!   machine): a user who never pairs makes no new network traffic.
//! * **The API rides on it**: every bidirectional stream carries one
//!   HTTP/1.1 connection into the peer router (`routes::peer_app`), with the
//!   QUIC-authenticated caller put on the request ([`PeerCaller`]) — the
//!   context middleware turns it into `Principal::Peer`; no token is read.
//! * **Unknown keys are refused at the door**, except the one key that is
//!   pairing with this machine right now (it reaches `/peer/pair/*` only).
//! * Connections serve both ways: whichever side dialled, either may open
//!   streams on it, so one connection per pair of machines suffices.
//! * Each machine remembers its peers' last direct addresses and tries them
//!   first (mDNS is filtered on some networks).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Weak;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::ConnectInfo;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use iroh::address_lookup::{DnsAddressLookup, PkarrPublisher, PkarrResolver, UserData};
use iroh::endpoint::{presets, Connection};
use iroh::{
    Endpoint, EndpointAddr, PublicKey, RelayMap, RelayMode as IrohRelayMode, RelayUrl, SecretKey,
};
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use serde::Serialize;
use serde_json::Value;
use tower::ServiceExt;

use super::protocol::{MachineInfo, ALPN_PAIR, ALPN_PEER};
use super::store::{RelayMode, RelaySetting};
use crate::server::context::PeerCaller;
use crate::state::{AppState, Inner};

/// How a connection to another machine runs right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
    /// Direct, on the local network (a private address).
    Lan,
    /// Direct, across the internet (a public address: hole-punched or open).
    Internet,
    /// Through the relay.
    Relay,
}

/// The running endpoint.
pub struct Net {
    pub endpoint: Endpoint,
    pub mdns: Option<MdnsAddressLookup>,
    pub relay: RelaySetting,
    tasks: Vec<tokio::task::AbortHandle>,
}

impl Drop for Net {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// How long a request to another machine may take by default.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// The largest JSON answer read from another machine.
const MAX_ANSWER: usize = 16 * 1024 * 1024;

// ─── bulk streams yield (plan §5.3) ──────────────────────────────────────────────

/// The QUIC send priority of a stream that carries a big body — a file's
/// chunk (moves, copies, code sync, runs, tasks, team roles), an artifact or
/// a result's file, the readiness test's transfer — below the API's and the
/// dashboard's frames' (0, every stream's default): a connection sends what
/// those have buffered first, so a dashboard on a slow relay stays
/// responsive while a move fills the link.
pub const BULK_PRIORITY: i32 = -1;
// Below the default every stream has.
const _: () = assert!(BULK_PRIORITY < 0);

/// Whether a body of `content_type` is bulk: raw bytes
/// (`application/octet-stream`), what chunked transfers send and what their
/// downloads answer. JSON, the WebSocket and everything else stay at the
/// default priority.
pub fn is_bulk_type(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|t| {
        t.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/octet-stream")
    })
}

/// A request's send priority from its headers: [`BULK_PRIORITY`] for a body
/// of raw bytes (a chunk going up), else the default.
pub fn priority_for(headers: &[(&str, String)]) -> i32 {
    let bulk = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && is_bulk_type(Some(v)));
    if bulk {
        BULK_PRIORITY
    } else {
        0
    }
}

/// What has a QUIC send priority (a send stream; a fake in tests).
pub(crate) trait SetPriority {
    /// Sets it; false when the stream is closed already.
    fn set_send_priority(&self, priority: i32) -> bool;
}

impl SetPriority for iroh::endpoint::SendStream {
    fn set_send_priority(&self, priority: i32) -> bool {
        self.set_priority(priority).is_ok()
    }
}

/// A served stream's send half whose priority follows what the answer
/// carries: the router marks `want` bulk when it answers raw bytes, and the
/// next write applies it (counted in `lowered`, for the tests).
pub(crate) struct Prioritized<S> {
    inner: S,
    want: std::sync::Arc<std::sync::atomic::AtomicI32>,
    applied: i32,
    lowered: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl<S: SetPriority> Prioritized<S> {
    pub(crate) fn new(
        inner: S,
        want: std::sync::Arc<std::sync::atomic::AtomicI32>,
        lowered: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            inner,
            want,
            applied: 0,
            lowered,
        }
    }

    fn apply(&mut self) {
        use std::sync::atomic::Ordering;
        let want = self.want.load(Ordering::Relaxed);
        if want != self.applied {
            if self.inner.set_send_priority(want) && want < 0 {
                self.lowered.fetch_add(1, Ordering::Relaxed);
            }
            self.applied = want;
        }
    }
}

impl<S: SetPriority + tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for Prioritized<S> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        this.apply();
        std::pin::Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        this.apply();
        std::pin::Pin::new(&mut this.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

// ─── the endpoint ──────────────────────────────────────────────────────────────

/// The endpoint, starting it if needed. (Boxed as a `Send` future: the
/// presence loop it starts may come back here, and the compiler cannot prove
/// such a cycle `Send` on its own.)
pub fn ensure_started(
    state: &AppState,
) -> futures_util::future::BoxFuture<'_, Result<Endpoint, String>> {
    Box::pin(start_inner(state))
}

async fn start_inner(state: &AppState) -> Result<Endpoint, String> {
    let mut net = state.machines.net.lock().await;
    if state
        .machines
        .held_down
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err("this computer is offline".into());
    }
    if let Some(n) = net.as_ref() {
        return Ok(n.endpoint.clone());
    }
    let ident = super::identity(state)?;
    let relay = state.machines.settings().relay;
    let test_lookup = state.machines.test_lookup();
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(ident.key.clone())
        .alpns(vec![ALPN_PEER.to_vec()]);
    builder = builder.relay_mode(relay_mode(&relay)?);
    if let Some(tls) = super::relay_pin::tls_config(&relay, state.machines.relay_pin.clone())? {
        builder = builder.ca_tls_config(tls);
    }
    let mut mdns = None;
    if let Some(memory) = &test_lookup {
        builder = builder.address_lookup(memory.clone());
    } else {
        if relay.mode == RelayMode::Public {
            builder = builder
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(PkarrResolver::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns());
        }
        match MdnsAddressLookup::builder().build(ident.key.public()) {
            Ok(m) => {
                builder = builder.address_lookup(m.clone());
                mdns = Some(m);
            }
            // No multicast here (or not allowed, e.g. macOS's Local Network
            // permission): the setup's network check says so.
            Err(e) => tracing::warn!(error = %e, "mDNS unavailable"),
        }
    }
    let endpoint = builder
        .bind()
        .await
        .map_err(|e| format!("The network could not be opened for Machines: {e}"))?;
    if let Some(memory) = &test_lookup {
        memory.add_endpoint_info(endpoint.addr());
    }
    let weak = std::sync::Arc::downgrade(state);
    let mut tasks = vec![tokio::spawn(accept_loop(weak.clone(), endpoint.clone())).abort_handle()];
    if let Some(m) = &mdns {
        tasks.push(tokio::spawn(watch_nearby(weak.clone(), m.clone())).abort_handle());
    }
    tasks.push(tokio::spawn(super::presence_loop(weak.clone())).abort_handle());
    tasks.push(tokio::spawn(super::federation::run(weak)).abort_handle());
    tracing::info!(id = %endpoint.id(), relay = relay.mode.as_str(), "Machines transport started");
    *net = Some(Net {
        endpoint: endpoint.clone(),
        mdns,
        relay,
        tasks,
    });
    drop(net);
    super::publish(state, "transport");
    Ok(endpoint)
}

/// Stops the endpoint (Machines no longer in use, or a setting changed).
pub async fn stop(state: &AppState) {
    let net = state.machines.net.lock().await.take();
    state.machines.forget_connections();
    if let Some(n) = net {
        n.endpoint.close().await;
        drop(n);
        super::publish(state, "transport");
    }
}

/// Tests: this machine as if switched off. [`stop`] alone does not do it:
/// the next request this machine makes (its link to a peer, a hello) starts
/// the endpoint again, as a running app is online. Held down, nothing here
/// starts it until [`come_back`].
#[doc(hidden)]
pub async fn go_offline(state: &AppState) {
    state
        .machines
        .held_down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    stop(state).await;
}

/// Tests: the machine [`go_offline`] held down may start its endpoint again.
#[doc(hidden)]
pub fn come_back(state: &AppState) {
    state
        .machines
        .held_down
        .store(false, std::sync::atomic::Ordering::SeqCst);
}

/// The running endpoint, if any (no start).
pub async fn current(state: &AppState) -> Option<Endpoint> {
    state
        .machines
        .net
        .lock()
        .await
        .as_ref()
        .map(|n| n.endpoint.clone())
}

fn relay_mode(relay: &RelaySetting) -> Result<IrohRelayMode, String> {
    Ok(match relay.mode {
        RelayMode::Public => IrohRelayMode::Default,
        RelayMode::None => IrohRelayMode::Disabled,
        RelayMode::Custom => {
            let url = relay
                .url
                .as_deref()
                .ok_or("Your relay's address is missing.")?;
            let url = RelayUrl::from_str(url)
                .map_err(|e| format!("Your relay's address is not valid: {e}"))?;
            IrohRelayMode::Custom(RelayMap::from_iter([url]))
        }
    })
}

/// A short-lived pairing rendezvous with a key derived from the code
/// (`pairing.rs`): reachable like the main endpoint, listed as *Nearby* on
/// the local network.
pub async fn bind_rendezvous(
    state: &AppState,
    key: SecretKey,
    me: &MachineInfo,
) -> Result<Endpoint, String> {
    let relay = state.machines.settings().relay;
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(key.clone())
        .alpns(vec![ALPN_PAIR.to_vec()])
        .relay_mode(relay_mode(&relay)?);
    if let Some(tls) = super::relay_pin::tls_config(&relay, state.machines.relay_pin.clone())? {
        builder = builder.ca_tls_config(tls);
    }
    let note: String = format!("{}{}:{}", super::pairing::NEARBY_TAG, me.os, me.name)
        .chars()
        .take(200)
        .collect();
    if let Ok(data) = UserData::try_from(note) {
        builder = builder.user_data_for_address_lookup(data);
    }
    let test_lookup = state.machines.test_lookup();
    if let Some(memory) = &test_lookup {
        builder = builder.address_lookup(memory.clone());
    } else {
        if relay.mode == RelayMode::Public {
            builder = builder.address_lookup(PkarrPublisher::n0_dns());
        }
        if let Ok(m) = MdnsAddressLookup::builder().build(key.public()) {
            builder = builder.address_lookup(m);
        }
    }
    let endpoint = builder
        .bind()
        .await
        .map_err(|e| format!("The pairing could not be offered on the network: {e}"))?;
    if let Some(memory) = &test_lookup {
        memory.add_endpoint_info(endpoint.addr());
    }
    if relay.mode != RelayMode::None {
        // Publish through the relay before the code is shown; on a network
        // without the internet the local one still works.
        let _ = tokio::time::timeout(Duration::from_secs(8), endpoint.online()).await;
    }
    Ok(endpoint)
}

// ─── serving ───────────────────────────────────────────────────────────────────

async fn accept_loop(state: Weak<Inner>, endpoint: Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        let state = state.clone();
        tokio::spawn(async move {
            let Ok(conn) = incoming.await else { return };
            let Some(state) = state.upgrade() else { return };
            let key = conn.remote_id().to_string();
            if conn.alpn() != ALPN_PEER || !admitted(&state, &key) {
                conn.close(1u32.into(), b"not paired");
                return;
            }
            serve(state, conn).await;
        });
    }
}

/// A paired key, or the key pairing with this machine right now.
fn admitted(state: &AppState, key: &str) -> bool {
    state.machines.store_read(|f| f.peer_by_key(key).is_some())
        || state.machines.pairing_admits(key)
}

/// Serves the streams the other side opens on `conn`, until it closes.
pub(super) async fn serve(state: AppState, conn: Connection) {
    let key = conn.remote_id().to_string();
    state.machines.remember_connection(&key, &conn);
    let weak = std::sync::Arc::downgrade(&state);
    drop(state);
    loop {
        let Ok((send, recv)) = conn.accept_bi().await else {
            break;
        };
        let Some(state) = weak.upgrade() else { break };
        let conn = conn.clone();
        let key = key.clone();
        tokio::spawn(async move {
            if !admitted(&state, &key) {
                return;
            }
            let machine = state
                .machines
                .store_read(|f| f.peer_by_key(&key).map(|p| p.id.clone()));
            let path = path_of(&conn);
            let caller = PeerCaller {
                key,
                machine,
                lan: path == PathKind::Lan,
            };
            let remote = ConnectInfo(remote_addr(&conn));
            let app = state.machines.peer_app(&state);
            let lowered = state.machines.bulk_streams.clone();
            drop(state);
            // An answer of raw bytes (a file's chunk, the readiness test's
            // transfer) goes at the bulk priority; everything else at the
            // default (plan §5.3).
            let want = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
            let mark = want.clone();
            let svc = app
                .map_request(move |mut req: axum::http::Request<hyper::body::Incoming>| {
                    req.extensions_mut().insert(caller.clone());
                    req.extensions_mut().insert(remote);
                    req.extensions_mut().insert(path);
                    req
                })
                .map_response(move |resp: axum::response::Response| {
                    let bulk = is_bulk_type(
                        resp.headers()
                            .get(axum::http::header::CONTENT_TYPE)
                            .and_then(|v| v.to_str().ok()),
                    );
                    mark.store(
                        if bulk { BULK_PRIORITY } else { 0 },
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    resp
                });
            let send = Prioritized::new(send, want, lowered);
            let io = TokioIo::new(tokio::io::join(recv, send));
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, TowerToHyperService::new(svc))
                .with_upgrades()
                .await;
        });
    }
}

/// The selected path's kind.
pub fn path_of(conn: &Connection) -> PathKind {
    let list = conn.paths();
    for p in list.iter() {
        if p.is_selected() {
            if p.is_relay() {
                return PathKind::Relay;
            }
            if let Some(ip) = ip_of(p.remote_addr()) {
                // A private address, or this very computer.
                return if crate::server::context::is_private_ip(ip) || ip.is_loopback() {
                    PathKind::Lan
                } else {
                    PathKind::Internet
                };
            }
        }
    }
    PathKind::Relay
}

/// The selected path's round trip, in ms.
pub fn rtt_of(conn: &Connection) -> Option<f64> {
    let list = conn.paths();
    for p in list.iter() {
        if p.is_selected() {
            return Some((p.rtt().as_secs_f64() * 10_000.0).round() / 10.0);
        }
    }
    None
}

/// The other side's direct addresses on this connection.
pub fn direct_addrs(conn: &Connection) -> Vec<SocketAddr> {
    let list = conn.paths();
    list.iter()
        .filter_map(|p| match p.remote_addr() {
            iroh::TransportAddr::Ip(a) => Some(*a),
            _ => None,
        })
        .collect()
}

fn ip_of(addr: &iroh::TransportAddr) -> Option<IpAddr> {
    match addr {
        iroh::TransportAddr::Ip(a) => Some(a.ip()),
        _ => None,
    }
}

fn remote_addr(conn: &Connection) -> SocketAddr {
    let list = conn.paths();
    for p in list.iter() {
        if p.is_selected() {
            if let iroh::TransportAddr::Ip(a) = p.remote_addr() {
                return *a;
            }
        }
    }
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
}

// ─── nearby pairings ─────────────────────────────────────────────────────────────

async fn watch_nearby(state: Weak<Inner>, mdns: MdnsAddressLookup) {
    use futures_util::StreamExt;
    let mut events = mdns.subscribe().await;
    while let Some(event) = events.next().await {
        let Some(state) = state.upgrade() else { break };
        match event {
            DiscoveryEvent::Discovered { endpoint_info, .. } => {
                let Some(note) = endpoint_info
                    .user_data()
                    .map(|d| d.to_string())
                    .and_then(|d| {
                        d.strip_prefix(super::pairing::NEARBY_TAG)
                            .map(str::to_string)
                    })
                else {
                    continue;
                };
                let (os, name) = note.split_once(':').unwrap_or(("other", note.as_str()));
                state.machines.note_nearby(super::Nearby {
                    id: endpoint_info.endpoint_id.to_string(),
                    name: name.to_string(),
                    os: os.to_string(),
                    addr: endpoint_info.into_endpoint_addr(),
                    seen: Instant::now(),
                });
            }
            DiscoveryEvent::Expired { endpoint_id } => {
                state.machines.forget_nearby(&endpoint_id.to_string());
            }
            _ => {}
        }
    }
}

// ─── asking another machine ───────────────────────────────────────────────────

/// A connection to `addr` for `alpn`.
pub async fn dial(state: &AppState, addr: EndpointAddr, alpn: &[u8]) -> Result<Connection, String> {
    let endpoint = ensure_started(state).await?;
    let conn = endpoint
        .connect(addr, alpn)
        .await
        .map_err(|e| format!("could not connect: {e}"))?;
    if alpn == ALPN_PEER {
        let key = conn.remote_id().to_string();
        state.machines.remember_connection(&key, &conn);
        // The other side may open streams on it too.
        tokio::spawn(serve(state.clone(), conn.clone()));
    }
    Ok(conn)
}

/// An open connection to the machine with `key`: a live one either side
/// made, or a new one (its last addresses first, then the lookups).
pub async fn connection_to(state: &AppState, key: &str) -> Result<Connection, String> {
    if let Some(conn) = state.machines.live_connection(key) {
        return Ok(conn);
    }
    let public = PublicKey::from_str(key).map_err(|_| "a malformed machine key".to_string())?;
    let mut addr = EndpointAddr::new(public);
    let (addrs, relay_url) = state.machines.store_read(|f| {
        f.peer_by_key(key)
            .map(|p| (p.addrs.clone(), p.relay_url.clone()))
            .unwrap_or_default()
    });
    for a in addrs.iter().filter_map(|a| a.parse::<SocketAddr>().ok()) {
        addr = addr.with_ip_addr(a);
    }
    let settings = state.machines.settings();
    let relay = if settings.relay.mode == RelayMode::Custom {
        settings.relay.url
    } else {
        relay_url
    };
    if let Some(url) = relay.as_deref().and_then(|u| RelayUrl::from_str(u).ok()) {
        addr = addr.with_relay_url(url);
    }
    dial(state, addr, ALPN_PEER).await
}

/// One request on a connection: an HTTP/1.1 exchange on a new stream.
pub async fn request_on(
    conn: &Connection,
    method: &str,
    path: &str,
    body: Option<Value>,
    timeout: Duration,
) -> Result<(u16, Value), String> {
    tokio::time::timeout(timeout, request_inner(conn, method, path, body))
        .await
        .map_err(|_| "the other computer did not answer in time".to_string())?
}

async fn request_inner(
    conn: &Connection,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<(u16, Value), String> {
    let (send, recv) = conn
        .open_bi()
        .await
        .map_err(|e| format!("could not open a stream: {e}"))?;
    let io = TokioIo::new(tokio::io::join(recv, send));
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| format!("could not start a request: {e}"))?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let bytes = match &body {
        Some(v) => Bytes::from(serde_json::to_vec(v).map_err(|e| e.to_string())?),
        None => Bytes::new(),
    };
    let mut req = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "peer");
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let req = req.body(Full::new(bytes)).map_err(|e| e.to_string())?;
    let resp = sender
        .send_request(req)
        .await
        .map_err(|e| format!("the request failed: {e}"))?;
    let status = resp.status().as_u16();
    let collected = http_body_util::Limited::new(resp.into_body(), MAX_ANSWER)
        .collect()
        .await
        .map_err(|e| format!("the answer could not be read: {e}"))?
        .to_bytes();
    let value = if collected.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&collected).unwrap_or(Value::Null)
    };
    Ok((status, value))
}

/// One request to the machine with `key`.
pub async fn request_key(
    state: &AppState,
    key: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    timeout: Duration,
) -> Result<(u16, Value), String> {
    let conn = tokio::time::timeout(timeout, connection_to(state, key))
        .await
        .map_err(|_| "the other computer could not be reached in time".to_string())??;
    match request_on(&conn, method, path, body.clone(), timeout).await {
        Ok(r) => Ok(r),
        Err(e) => {
            // A stale connection: forget it and try a fresh one once.
            if conn.close_reason().is_some() {
                state.machines.drop_connection(key);
                let conn = connection_to(state, key).await?;
                return request_on(&conn, method, path, body, timeout).await;
            }
            Err(e)
        }
    }
}

/// A request with raw bytes both ways (the dashboard's proxy): the status,
/// the answer's content type and its body.
pub async fn request_raw(
    state: &AppState,
    machine: &str,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Bytes,
    timeout: Duration,
) -> Result<(u16, Option<String>, Bytes), String> {
    let key = state
        .machines
        .store_read(|f| f.peer(machine).map(|p| p.key.clone()))
        .ok_or_else(|| "not a paired computer".to_string())?;
    let conn = tokio::time::timeout(timeout, connection_to(state, &key))
        .await
        .map_err(|_| "it could not be reached in time".to_string())??;
    let exchange = async {
        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| format!("could not open a stream: {e}"))?;
        // A chunk going up yields to everything else on the connection
        // (plan §5.3).
        let priority = priority_for(headers);
        if priority != 0 && send.set_send_priority(priority) {
            state
                .machines
                .bulk_streams
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let io = TokioIo::new(tokio::io::join(recv, send));
        let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| format!("could not start a request: {e}"))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let mut req = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("host", "peer");
        for (k, v) in headers {
            req = req.header(*k, v.as_str());
        }
        let req = req.body(Full::new(body)).map_err(|e| e.to_string())?;
        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| format!("the request failed: {e}"))?;
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = http_body_util::Limited::new(resp.into_body(), 64 * 1024 * 1024)
            .collect()
            .await
            .map_err(|e| format!("the answer could not be read: {e}"))?
            .to_bytes();
        Ok::<_, String>((status, content_type, bytes))
    };
    tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| "it did not answer in time".to_string())?
}

/// One request to the paired machine `machine` (by id).
pub async fn request(
    state: &AppState,
    machine: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    timeout: Duration,
) -> Result<(u16, Value), String> {
    let key = state
        .machines
        .store_read(|f| f.peer(machine).map(|p| p.key.clone()))
        .ok_or_else(|| "not a paired computer".to_string())?;
    request_key(state, &key, method, path, body, timeout).await
}

/// Downloads `mib` MiB from the machine (the readiness test's transfer).
pub async fn bulk_from(
    state: &AppState,
    machine: &str,
    mib: u64,
) -> Result<(u64, Duration), String> {
    let key = state
        .machines
        .store_read(|f| f.peer(machine).map(|p| p.key.clone()))
        .ok_or_else(|| "not a paired computer".to_string())?;
    let conn = connection_to(state, &key).await?;
    let started = Instant::now();
    let (send, recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    let io = TokioIo::new(tokio::io::join(recv, send));
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let req = axum::http::Request::builder()
        .method("GET")
        .uri(format!("/peer/bulk?mib={mib}"))
        .header("host", "peer")
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())?;
    let resp = sender.send_request(req).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("the other computer answered {}", resp.status()));
    }
    let mut body = resp.into_body();
    let mut n = 0u64;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| e.to_string())?;
        if let Some(data) = frame.data_ref() {
            n += data.len() as u64;
        }
    }
    Ok((n, started.elapsed()))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use tokio::io::AsyncWriteExt;

    use super::*;

    /// A writer that records the priorities it is given.
    #[derive(Clone, Default)]
    struct Fake {
        set: Arc<Mutex<Vec<i32>>>,
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl SetPriority for Fake {
        fn set_send_priority(&self, priority: i32) -> bool {
            self.set.lock().unwrap().push(priority);
            true
        }
    }

    impl tokio::io::AsyncWrite for Fake {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.written.lock().unwrap().extend_from_slice(buf);
            std::task::Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn raw_bytes_are_bulk_and_nothing_else_is() {
        assert!(is_bulk_type(Some("application/octet-stream")));
        assert!(is_bulk_type(Some("Application/Octet-Stream; charset=x")));
        for other in [
            None,
            Some("application/json"),
            Some("text/plain"),
            Some("application/octet-streamy"),
        ] {
            assert!(!is_bulk_type(other), "{other:?}");
        }
        let chunk = [("content-type", "application/octet-stream".to_string())];
        assert_eq!(priority_for(&chunk), BULK_PRIORITY);
        let json = [("content-type", "application/json".to_string())];
        assert_eq!(priority_for(&json), 0);
        assert_eq!(priority_for(&[]), 0);
    }

    #[tokio::test]
    async fn a_served_stream_yields_only_once_its_answer_is_bulk() {
        // An API answer: the default priority, never touched.
        let fake = Fake::default();
        let want = Arc::new(AtomicI32::new(0));
        let lowered = Arc::new(AtomicU64::new(0));
        let mut s = Prioritized::new(fake.clone(), want.clone(), lowered.clone());
        s.write_all(b"HTTP/1.1 200 OK\r\n\r\n{}").await.unwrap();
        assert!(fake.set.lock().unwrap().is_empty());
        assert_eq!(lowered.load(Ordering::Relaxed), 0);

        // Raw bytes: lowered at the next write, once.
        want.store(BULK_PRIORITY, Ordering::Relaxed);
        s.write_all(b"chunk one").await.unwrap();
        s.write_all(b"chunk two").await.unwrap();
        assert_eq!(*fake.set.lock().unwrap(), vec![BULK_PRIORITY]);
        assert_eq!(lowered.load(Ordering::Relaxed), 1);
        assert!(fake
            .written
            .lock()
            .unwrap()
            .ends_with(b"chunk onechunk two"));
    }
}
