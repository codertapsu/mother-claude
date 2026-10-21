//! Internet access through a tunnel, and the remote-access status (design
//! §2.8, §10 B1).
//!
//! * `remote.json` in the app data dir (0600): `internetEnabled` (default
//!   off), `manualUrl`, `bindTailscale` (LAN listeners on 100.64/10, opt-in),
//!   `deviceIdleDays`. Internet pairings always need desktop approval, so
//!   `requireApproval` is reported as `true` and cannot be switched off.
//! * The **tunnel listener** `127.0.0.1:<tunnelPort>` (default 6726, env
//!   `MOTHER_CLAUDE_TUNNEL_PORT`) is bound only while Internet access is on and
//!   unbound (every open connection, WebSockets included, cut) when it is
//!   switched off. It serves `server::router_for(state, Ingress::Tunnel)`: no
//!   `/hooks/*`, no `/v1`, a device token required, and the last
//!   `X-Forwarded-Proto` must be `https` (`server::security`).
//! * **Detection** (local reads only, never from request headers): ngrok's
//!   agent API (`127.0.0.1:4040/api/endpoints`, fallback `/api/tunnels`),
//!   cloudflared's metrics (`/quicktunnel` on 20241–20245) and
//!   `tailscale serve status --json` (Serve and Funnel). Every 10 s while
//!   Internet access is on, the Remote page is open or an API server is
//!   running; every minute otherwise. A tunnel whose upstream is an API
//!   server's port turns the tokenless mode of that port off, and only of that
//!   one ([`RemoteState::port_exposed`], docs/ACCOUNTS.md §13.5).
//! * `GET/PUT /api/remote` payloads ([`status`], [`update`]) with the tunnel
//!   commands and warnings.
//! * The LAN listener status (addresses, skipped interfaces, certificate
//!   fingerprints) is kept here too, written by `server::listeners`.
//!
//! [`start`] is called once from `server::serve`. Handlers live in
//! `server/routes/remote.rs`. AppState field: `state.remote`.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};

use super::context::Ingress;
use super::error::{ApiError, ApiResult};
use crate::claude::model::NoticeLevel;
use crate::state::AppState;

/// Default tunnel ingress port.
pub const DEFAULT_TUNNEL_PORT: u16 = 6726;
/// How often detection runs while it matters.
const DETECT_EVERY: Duration = Duration::from_secs(10);
/// The Remote page counts as open this long after its last `GET /api/remote`.
const PAGE_OPEN_FOR: Duration = Duration::from_secs(60);

/// Persisted settings (`remote.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteSettings {
    pub internet_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_url: Option<String>,
    /// Also bind the LAN listeners on Tailscale (100.64/10) addresses.
    pub bind_tailscale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_idle_days: Option<u32>,
    /// Unknown fields survive a round trip.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// One tunnel found by detection.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedTunnel {
    /// `ngrok` | `cloudflared` | `tailscale-funnel` | `tailscale-serve`.
    pub provider: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_port: Option<u16>,
    /// ngrok's local web inspector is recording this endpoint.
    pub inspect: bool,
    /// `http` | `tcp`.
    pub kind: String,
    /// `tunnel` (6726) | `main` (6725) | `bridge` (an API server's port: 5612,
    /// or another server's) | `other` | `unknown`.
    pub target: String,
    /// The API server a `bridge` tunnel points at, when it is not the main
    /// one (absent: the main server, so with only that one it reads as
    /// before).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

/// A bound LAN address.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LanAddr {
    pub ip: String,
    pub interface: String,
    pub url: String,
}

/// An interface address the LAN listeners deliberately did not bind.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedAddr {
    pub ip: String,
    pub interface: String,
    pub reason: String,
}

/// LAN listener status (written by `server::listeners`).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanStatus {
    /// TLS listeners are configured (the bind host is not loopback).
    pub enabled: bool,
    pub port: u16,
    pub addresses: Vec<LanAddr>,
    pub skipped: Vec<SkippedAddr>,
    /// SHA-256 of the leaf certificate the browser sees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// SHA-256 of the local CA a phone can trust.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_fingerprint: Option<String>,
    #[serde(skip)]
    pub ca_der: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Default)]
struct Detection {
    tunnels: Vec<DetectedTunnel>,
    checked_at: Option<i64>,
}

/// The running tunnel listener.
struct TunnelListener {
    port: u16,
    kill: Arc<KillSwitch>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl std::fmt::Debug for TunnelListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelListener")
            .field("port", &self.port)
            .finish()
    }
}

/// Persisted settings + detection results + listener handles.
#[derive(Debug)]
pub struct RemoteState {
    path: Option<PathBuf>,
    tunnel_port: AtomicU16,
    settings: Mutex<RemoteSettings>,
    detection: Mutex<Detection>,
    lan: Mutex<LanStatus>,
    page_seen: Mutex<Option<Instant>>,
    /// The API ports a detected tunnel points at, sorted.
    exposed: Mutex<Vec<u16>>,
    tunnel: tokio::sync::Mutex<Option<TunnelListener>>,
    tunnel_error: Mutex<Option<String>>,
    /// LAN listener runtime (owned by `server::listeners`).
    pub(crate) lan_rt: tokio::sync::Mutex<super::listeners::LanRuntime>,
}

impl Default for RemoteState {
    fn default() -> Self {
        Self::new(None)
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn tunnel_port_from_env() -> u16 {
    std::env::var("MOTHER_CLAUDE_TUNNEL_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|p| *p != 0)
        .unwrap_or(DEFAULT_TUNNEL_PORT)
}

impl RemoteState {
    /// Tolerant constructor; `None` ⇒ in-memory only.
    pub fn new(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join("remote.json"));
        let settings: RemoteSettings = path
            .as_deref()
            .and_then(crate::persist::read_json)
            .unwrap_or_default();
        Self {
            path,
            tunnel_port: AtomicU16::new(tunnel_port_from_env()),
            settings: Mutex::new(settings),
            detection: Mutex::new(Detection::default()),
            lan: Mutex::new(LanStatus::default()),
            page_seen: Mutex::new(None),
            exposed: Mutex::new(Vec::new()),
            tunnel: tokio::sync::Mutex::new(None),
            tunnel_error: Mutex::new(None),
            lan_rt: tokio::sync::Mutex::new(Default::default()),
        }
    }

    /// The configured tunnel port (the bound one may differ when it is `0`).
    pub fn tunnel_port(&self) -> u16 {
        self.tunnel_port.load(Ordering::Relaxed)
    }

    /// Change the tunnel port used by the next bind (`0` = any free port;
    /// tests and embedders). Takes effect when Internet access is next
    /// switched on.
    pub fn set_tunnel_port(&self, port: u16) {
        self.tunnel_port.store(port, Ordering::Relaxed);
    }

    /// The port the tunnel listener is bound to right now.
    pub async fn listening_port(&self) -> Option<u16> {
        self.tunnel.lock().await.as_ref().map(|t| t.port)
    }

    pub fn settings(&self) -> RemoteSettings {
        self.settings.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn save(&self, s: &RemoteSettings) {
        if let Ok(mut g) = self.settings.lock() {
            *g = s.clone();
        }
        let Some(path) = &self.path else { return };
        if let Err(e) = crate::persist::write_json_private(path, s) {
            tracing::warn!(path = %path.display(), error = %e, "could not save remote.json");
        }
    }

    /// Tunnels found by the last detection pass.
    pub fn detected(&self) -> Vec<DetectedTunnel> {
        self.detection
            .lock()
            .map(|d| d.tunnels.clone())
            .unwrap_or_default()
    }

    /// Replace detection results (the detection loop; tests). `ports` are
    /// the API servers' ports (an `Option` works: the main one's alone);
    /// returns whether the set of them a tunnel points at changed.
    pub fn set_detected(
        &self,
        tunnels: Vec<DetectedTunnel>,
        ports: impl IntoIterator<Item = u16>,
    ) -> bool {
        let mut exposed: Vec<u16> = ports
            .into_iter()
            .filter(|p| tunnels.iter().any(|t| t.upstream_port == Some(*p)))
            .collect();
        exposed.sort_unstable();
        exposed.dedup();
        if let Ok(mut d) = self.detection.lock() {
            d.tunnels = tunnels;
            d.checked_at = Some(now_ms());
        }
        let mut current = self.exposed.lock().unwrap_or_else(|e| e.into_inner());
        let changed = *current != exposed;
        *current = exposed;
        changed
    }

    /// The API ports a detected tunnel points at.
    pub fn exposed_ports(&self) -> Vec<u16> {
        self.exposed.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// A detected tunnel points at this API port: its tokenless mode is off
    /// until that tunnel stops. Other ports are not affected.
    pub fn port_exposed(&self, port: u16) -> bool {
        self.exposed
            .lock()
            .map(|g| g.contains(&port))
            .unwrap_or(true)
    }

    /// A detected tunnel points at an API port, any server's.
    pub fn bridge_exposed(&self) -> bool {
        self.exposed.lock().map(|g| !g.is_empty()).unwrap_or(true)
    }

    /// The public URL for Internet pairing: the manual one, else a detected
    /// https tunnel to the tunnel port, else one to the main port. Never
    /// learned from request headers.
    pub fn public_url(&self) -> Option<String> {
        if let Some(u) = self.settings().manual_url.filter(|u| !u.is_empty()) {
            return Some(u);
        }
        let tunnels = self.detected();
        let https = |t: &&DetectedTunnel| t.kind == "http" && t.url.starts_with("https://");
        tunnels
            .iter()
            .filter(https)
            .find(|t| t.target == "tunnel")
            .or_else(|| tunnels.iter().filter(https).find(|t| t.target == "main"))
            .map(|t| t.url.trim_end_matches('/').to_string())
    }

    /// `https://<first LAN address>:<port>`, when the LAN listeners run.
    pub fn lan_base_url(&self) -> Option<String> {
        let lan = self.lan();
        if !lan.enabled {
            return None;
        }
        lan.addresses.first().map(|a| a.url.clone())
    }

    pub fn lan(&self) -> LanStatus {
        self.lan.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn set_lan(&self, status: LanStatus) {
        if let Ok(mut g) = self.lan.lock() {
            *g = status;
        }
    }

    /// The Remote page asked for status (detection runs every 10 s for a
    /// minute after).
    pub fn mark_page_seen(&self) {
        if let Ok(mut g) = self.page_seen.lock() {
            *g = Some(Instant::now());
        }
    }

    fn page_open(&self) -> bool {
        self.page_seen
            .lock()
            .ok()
            .and_then(|g| *g)
            .is_some_and(|t| t.elapsed() < PAGE_OPEN_FOR)
    }

    /// Whether the tunnel listener is bound.
    pub async fn tunnel_listening(&self) -> bool {
        self.tunnel.lock().await.is_some()
    }

    /// `wss://…` origins a page may connect to besides `'self'` (for the CSP):
    /// the public URL and the LAN addresses. Never from request headers.
    pub fn ws_origins(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(u) = self.public_url() {
            if let Some(rest) = u.strip_prefix("https://") {
                out.push(format!("wss://{rest}"));
            }
        }
        for a in self.lan().addresses {
            if let Some(rest) = a.url.strip_prefix("https://") {
                out.push(format!("wss://{rest}"));
            }
        }
        out
    }
}

// ───────────────────────────── tunnel listener ────────────────────────────────

/// Cuts every connection accepted by one listener, upgraded WebSockets
/// included: each stream checks the flag on every poll and is woken when it
/// flips.
#[derive(Debug, Default)]
struct KillSwitch {
    killed: AtomicBool,
    next: AtomicU64,
    wakers: Mutex<HashMap<u64, Waker>>,
}

impl KillSwitch {
    fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        let wakers: Vec<Waker> = self
            .wakers
            .lock()
            .map(|mut w| w.drain().map(|(_, w)| w).collect())
            .unwrap_or_default();
        for w in wakers {
            w.wake();
        }
    }

    fn register(&self, slot: u64, waker: &Waker) {
        if let Ok(mut w) = self.wakers.lock() {
            match w.get(&slot) {
                Some(existing) if existing.will_wake(waker) => {}
                _ => {
                    w.insert(slot, waker.clone());
                }
            }
        }
    }

    fn forget(&self, id: u64) {
        if let Ok(mut w) = self.wakers.lock() {
            w.remove(&(id * 2));
            w.remove(&(id * 2 + 1));
        }
    }
}

/// A TCP stream that fails once its [`KillSwitch`] fires.
pub struct KillableStream {
    inner: TcpStream,
    switch: Arc<KillSwitch>,
    id: u64,
}

impl Drop for KillableStream {
    fn drop(&mut self) {
        self.switch.forget(self.id);
    }
}

fn killed_err() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "Internet access was turned off",
    )
}

impl AsyncRead for KillableStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.switch.killed.load(Ordering::SeqCst) {
            return Poll::Ready(Err(killed_err()));
        }
        this.switch.register(this.id * 2, cx.waker());
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for KillableStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.switch.killed.load(Ordering::SeqCst) {
            return Poll::Ready(Err(killed_err()));
        }
        this.switch.register(this.id * 2 + 1, cx.waker());
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.switch.killed.load(Ordering::SeqCst) {
            return Poll::Ready(Err(killed_err()));
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// `axum::serve` listener whose connections share one kill switch.
struct KillableListener {
    inner: TcpListener,
    switch: Arc<KillSwitch>,
}

impl axum::serve::Listener for KillableListener {
    type Io = KillableStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.inner.accept().await {
                Ok((stream, addr)) => {
                    let id = self.switch.next.fetch_add(1, Ordering::Relaxed);
                    return (
                        KillableStream {
                            inner: stream,
                            switch: self.switch.clone(),
                            id,
                        },
                        addr,
                    );
                }
                Err(e) => {
                    tracing::debug!(error = %e, "tunnel accept error");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

/// Serve `app` on an already-bound listener, returning the handle that stops
/// it and cuts its connections.
async fn serve_tunnel(state: &AppState, listener: TcpListener) -> io::Result<TunnelListener> {
    use axum::serve::ListenerExt;
    let port = listener.local_addr()?.port();
    let switch = Arc::new(KillSwitch::default());
    let l = KillableListener {
        inner: listener,
        switch: switch.clone(),
    }
    // `tap_io` gives the listener axum's `ConnectInfo<SocketAddr>` support.
    .tap_io(|_| {});
    let app = crate::server::router_for(state.clone(), Ingress::Tunnel);
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let served = axum::serve(l, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await;
        if let Err(e) = served {
            tracing::warn!(error = %e, "tunnel listener stopped");
        }
    });
    Ok(TunnelListener {
        port,
        kill: switch,
        shutdown: Some(tx),
    })
}

/// Bind or unbind the tunnel listener to match the settings.
pub async fn apply_tunnel(state: &AppState) {
    let want = state.remote.settings().internet_enabled;
    let mut guard = state.remote.tunnel.lock().await;
    match (want, guard.is_some()) {
        (true, false) => {
            let port = state.remote.tunnel_port();
            let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
            let bound = match TcpListener::bind(addr).await {
                Ok(l) => serve_tunnel(state, l).await,
                Err(e) => Err(e),
            };
            match bound {
                Ok(t) => {
                    tracing::info!(port = t.port, "tunnel ingress listening on 127.0.0.1");
                    *guard = Some(t);
                    set_tunnel_error(state, None);
                }
                Err(e) => {
                    tracing::warn!(port, error = %e, "could not bind the tunnel ingress");
                    set_tunnel_error(
                        state,
                        Some(format!("Could not listen on 127.0.0.1:{port}: {e}")),
                    );
                }
            }
        }
        (false, true) => {
            if let Some(mut t) = guard.take() {
                t.kill.kill();
                if let Some(tx) = t.shutdown.take() {
                    let _ = tx.send(());
                }
                tracing::info!(port = t.port, "tunnel ingress closed");
            }
            set_tunnel_error(state, None);
        }
        _ => {}
    }
}

fn set_tunnel_error(state: &AppState, e: Option<String>) {
    if let Ok(mut g) = state.remote.tunnel_error.lock() {
        *g = e;
    }
}

// ─────────────────────────────── detection ────────────────────────────────────

fn port_of(upstream: &str) -> Option<u16> {
    let s = upstream.trim();
    let s = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
    let s = s.split('/').next().unwrap_or(s);
    if let Ok(p) = s.parse::<u16>() {
        return Some(p);
    }
    s.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok())
}

/// What a tunnel upstream points at.
pub fn classify_target(
    upstream_port: Option<u16>,
    tunnel: u16,
    main: u16,
    bridge: Option<u16>,
) -> String {
    match upstream_port {
        None => "unknown",
        Some(p) if p == tunnel => "tunnel",
        Some(p) if p == main => "main",
        Some(p) if Some(p) == bridge => "bridge",
        Some(_) => "other",
    }
    .to_string()
}

/// What a tunnel whose upstream is `upstream_port` points at
/// ([`classify_target`]), every API server's port counting as `bridge`, and
/// which API server that is when it is not the main one.
pub fn target_of(state: &AppState, upstream_port: Option<u16>) -> (String, Option<String>) {
    let mut target = classify_target(
        upstream_port,
        state.remote.tunnel_port(),
        state.config.port,
        state.bridge.config.port,
    );
    let mut server = None;
    if target == "other" || target == "bridge" {
        if let Some(s) = upstream_port.and_then(|p| state.bridge.server_for_port(p)) {
            target = "bridge".to_string();
            server = (!s.is_main()).then(|| s.id.clone());
        }
    }
    (target, server)
}

/// Parse ngrok's `/api/endpoints` (agent ≥ 3.16) answer.
pub fn parse_ngrok_endpoints(v: &Value) -> Vec<(String, Option<String>, bool)> {
    v.get("endpoints")
        .and_then(Value::as_array)
        .map(|eps| {
            eps.iter()
                .filter_map(|e| {
                    let url = e.get("url").and_then(Value::as_str)?.to_string();
                    let upstream = e
                        .pointer("/upstream/url")
                        .or_else(|| e.get("upstream"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let inspect = e.get("inspect").and_then(Value::as_bool).unwrap_or(false);
                    Some((url, upstream, inspect))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse ngrok's legacy `/api/tunnels` answer.
pub fn parse_ngrok_tunnels(v: &Value) -> Vec<(String, Option<String>, bool)> {
    v.get("tunnels")
        .and_then(Value::as_array)
        .map(|ts| {
            ts.iter()
                .filter_map(|t| {
                    let url = t.get("public_url").and_then(Value::as_str)?.to_string();
                    let upstream = t
                        .pointer("/config/addr")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let inspect = t
                        .pointer("/config/inspect")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    Some((url, upstream, inspect))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `tailscale serve status --json` (an `ipn.ServeConfig`): HTTPS
/// handlers under `Web`, raw TCP forwards under `TCP`, Funnel under
/// `AllowFunnel`.
pub fn parse_tailscale(v: &Value) -> Vec<(String, String, Option<String>, String)> {
    let mut out = Vec::new();
    let funnel = |hostport: &str| {
        v.pointer("/AllowFunnel")
            .and_then(|a| a.get(hostport))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    if let Some(web) = v.get("Web").and_then(Value::as_object) {
        for (hostport, cfg) in web {
            let provider = if funnel(hostport) {
                "tailscale-funnel"
            } else {
                "tailscale-serve"
            };
            let (host, port) = hostport
                .rsplit_once(':')
                .unwrap_or((hostport.as_str(), "443"));
            let base = if port == "443" {
                format!("https://{host}")
            } else {
                format!("https://{host}:{port}")
            };
            if let Some(handlers) = cfg.get("Handlers").and_then(Value::as_object) {
                for (path, h) in handlers {
                    let upstream = h.get("Proxy").and_then(Value::as_str).map(str::to_string);
                    if upstream.is_none() {
                        continue;
                    }
                    let url = if path == "/" {
                        base.clone()
                    } else {
                        format!("{base}{path}")
                    };
                    out.push((provider.to_string(), url, upstream, "http".to_string()));
                }
            }
        }
    }
    if let Some(tcp) = v.get("TCP").and_then(Value::as_object) {
        for (port, cfg) in tcp {
            if let Some(fwd) = cfg.get("TCPForward").and_then(Value::as_str) {
                out.push((
                    "tailscale-serve".to_string(),
                    format!("tcp://:{port}"),
                    Some(fwd.to_string()),
                    "tcp".to_string(),
                ));
            }
        }
    }
    out
}

fn binary(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        dirs.push(PathBuf::from(extra));
    }
    dirs.into_iter()
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .or_else(|| {
            (name == "tailscale")
                .then(|| PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"))
                .filter(|p| p.is_file())
        })
}

async fn tailscale_status() -> Option<Value> {
    let bin = binary("tailscale")?;
    for args in [
        ["serve", "status", "--json"],
        ["funnel", "status", "--json"],
    ] {
        let mut cmd = tokio::process::Command::new(&bin);
        cmd.args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let out = tokio::time::timeout(Duration::from_secs(4), cmd.output()).await;
        if let Ok(Ok(out)) = out {
            if out.status.success() {
                if let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// One detection pass. Local reads only.
pub async fn detect(state: &AppState, with_tailscale: bool) -> Vec<DetectedTunnel> {
    let client = match reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(1500))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(error = %e, "no http client for tunnel detection");
            return Vec::new();
        }
    };
    let get = |url: String| {
        let client = client.clone();
        async move {
            let r = client.get(url).send().await.ok()?;
            if !r.status().is_success() {
                return None;
            }
            r.json::<Value>().await.ok()
        }
    };
    let mut found = Vec::new();
    let mut push = |provider: &str, url: String, upstream: Option<String>, inspect: bool| {
        let kind = if url.starts_with("tcp://") {
            "tcp"
        } else {
            "http"
        };
        let upstream_port = upstream.as_deref().and_then(port_of);
        let (target, server) = target_of(state, upstream_port);
        found.push(DetectedTunnel {
            provider: provider.to_string(),
            target,
            url,
            upstream,
            upstream_port,
            inspect,
            kind: kind.to_string(),
            server,
        });
    };

    // ngrok's agent API (4040, and 4041 for a second agent).
    for port in [4040u16, 4041] {
        let base = format!("http://127.0.0.1:{port}");
        let mut eps = match get(format!("{base}/api/endpoints")).await {
            Some(v) => parse_ngrok_endpoints(&v),
            None => Vec::new(),
        };
        if eps.is_empty() {
            if let Some(v) = get(format!("{base}/api/tunnels")).await {
                eps = parse_ngrok_tunnels(&v);
            }
        }
        for (url, upstream, inspect) in eps {
            push("ngrok", url, upstream, inspect);
        }
    }
    // cloudflared quick tunnels (the upstream cannot be read from there).
    for port in 20241u16..=20245 {
        if let Some(v) = get(format!("http://127.0.0.1:{port}/quicktunnel")).await {
            if let Some(host) = v
                .get("hostname")
                .and_then(Value::as_str)
                .filter(|h| !h.is_empty())
            {
                push("cloudflared", format!("https://{host}"), None, false);
            }
        }
    }
    if with_tailscale {
        if let Some(v) = tailscale_status().await {
            for (provider, url, upstream, _kind) in parse_tailscale(&v) {
                push(&provider, url, upstream, false);
            }
        }
    }
    found.sort_by(|a, b| (&a.provider, &a.url).cmp(&(&b.provider, &b.url)));
    found.dedup();
    found
}

async fn detect_and_store(state: &AppState, with_tailscale: bool) {
    let mut tunnels = detect(state, with_tailscale).await;
    if !with_tailscale {
        // Keep the last Tailscale answer between its (slower) refreshes.
        tunnels.extend(
            state
                .remote
                .detected()
                .into_iter()
                .filter(|t| t.provider.starts_with("tailscale")),
        );
    }
    let before = state.remote.exposed_ports();
    let changed = state
        .remote
        .set_detected(tunnels, state.bridge.server_ports());
    // The public https URL is the VAPID contact of our pushes.
    crate::server::push::set_public_url(state, state.remote.public_url().as_deref());
    if !changed {
        return;
    }
    let after = state.remote.exposed_ports();
    for server in state.bridge.servers.all() {
        let ports = server.ports();
        let was = ports.iter().any(|p| before.contains(p));
        let is = ports.iter().any(|p| after.contains(p));
        if was != is {
            notice_exposure(state, &server, is);
        }
    }
}

/// Tell the desktop that a tunnel now points at an API server's port, or no
/// longer does.
fn notice_exposure(state: &AppState, server: &crate::bridge::ApiServer, exposed: bool) {
    let open = server.token_policy() == crate::bridge::TokenPolicy::Never;
    if !server.is_main() {
        let name = server.name();
        let port = server.current_port().unwrap_or_default();
        let (level, text) = match (exposed, open) {
            (true, true) => (
                NoticeLevel::Error,
                format!(
                    "A tunnel points at the API server \"{name}\" (port {port}), which is open \
                     to tunnels: anyone who finds that URL can drive Claude on this Mac without \
                     a token. Dangerous actions stay off for them. Stop the tunnel unless you \
                     meant to publish it."
                ),
            ),
            (true, false) => (
                NoticeLevel::Error,
                format!(
                    "A tunnel points at the API server \"{name}\" (port {port}). Tokenless \
                     access on that port is switched off until the tunnel stops — stop it \
                     unless you meant to publish it."
                ),
            ),
            (false, _) => (
                NoticeLevel::Info,
                format!("No tunnel points at the API server \"{name}\" (port {port}) any more."),
            ),
        };
        tracing::warn!(server = %server.id, port, exposed, "a tunnel to an API server's port changed");
        crate::server::ws::publish_desktop_notice(state, level, text);
        return;
    }
    let bridge = server.current_port().unwrap_or_default();
    if exposed && open {
        tracing::warn!(
            port = bridge,
            "a tunnel points at the bridge port, and the API is open to tunnels without a \
                 token"
        );
        crate::server::ws::publish_desktop_notice(
            state,
            NoticeLevel::Error,
            format!(
                "A tunnel points at the Claude API port {bridge}, and the API is open to \
                     tunnels: anyone who finds that URL can drive Claude on this Mac without a \
                     token. Dangerous actions stay off for them. Stop the tunnel unless you \
                     meant to publish the API."
            ),
        );
    } else if exposed {
        tracing::warn!(
            port = bridge,
            "a tunnel points at the bridge port; tokenless mode is off"
        );
        crate::server::ws::publish_desktop_notice(
            state,
            NoticeLevel::Error,
            format!(
                "A tunnel points at the Claude API port {bridge}. Tokenless access on that \
                     port is switched off until the tunnel stops — stop it unless you meant to \
                     publish the API."
            ),
        );
    } else {
        crate::server::ws::publish_desktop_notice(
            state,
            NoticeLevel::Info,
            if open {
                format!("No tunnel points at the Claude API port {bridge} any more.")
            } else {
                format!(
                    "No tunnel points at port {bridge} any more; tokenless local access is \
                         back."
                )
            },
        );
    }
}

/// Start remote-access background work: the tunnel listener if Internet
/// access is on, tunnel detection, and housekeeping (idle devices, stale
/// pairing requests). Called once from `server::serve`.
pub fn start(state: &AppState) {
    if let Some(days) = state.remote.settings().device_idle_days {
        state.devices.set_idle_days(days);
    }
    let st = state.clone();
    tokio::spawn(async move {
        apply_tunnel(&st).await;
        let mut tick: u64 = 0;
        loop {
            let busy = st.remote.settings().internet_enabled
                || st.remote.page_open()
                || st.bridge.servers.all().iter().any(|s| s.is_running());
            if busy || tick.is_multiple_of(6) {
                detect_and_store(&st, tick.is_multiple_of(3)).await;
            }
            super::pairing::sweep(&st);
            if tick.is_multiple_of(6) {
                super::devices::sweep_expired(&st);
            }
            tick = tick.wrapping_add(1);
            tokio::time::sleep(DETECT_EVERY).await;
        }
    });
}

// ──────────────────────────── status / update ────────────────────────────────

/// The commands the Remote page offers to copy.
pub fn commands(tunnel_port: u16) -> Value {
    json!({
        "ngrok": format!("ngrok http 127.0.0.1:{tunnel_port} --inspect=false"),
        "cloudflared": format!("cloudflared tunnel --url http://127.0.0.1:{tunnel_port}"),
        "tailscale": format!("tailscale funnel --bg {tunnel_port}"),
        "tailscaleServe": format!("tailscale serve --bg {tunnel_port}"),
    })
}

/// An API server's port, as the Remote page's warnings name it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiPort {
    /// The server's id; `None` for the main one.
    pub server: Option<String>,
    pub name: String,
    pub port: Option<u16>,
    /// It runs open to tunnels (the API screen's switch, or
    /// `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN=never`): a tunnel to its port
    /// publishes it without a token, rather than switching tokenless mode off.
    pub open: bool,
}

/// Every API server's port, the main one first.
pub fn api_ports(state: &AppState) -> Vec<ApiPort> {
    state
        .bridge
        .servers
        .all()
        .into_iter()
        .map(|s| ApiPort {
            server: (!s.is_main()).then(|| s.id.clone()),
            name: s.name(),
            port: if s.is_main() {
                state.bridge.config.port
            } else {
                s.current_port()
            },
            open: s.token_policy() == crate::bridge::TokenPolicy::Never,
        })
        .collect()
}

/// Human-readable warnings for the Remote page.
pub fn warnings(
    settings: &RemoteSettings,
    detected: &[DetectedTunnel],
    tunnel_port: u16,
    main_port: u16,
    api: &[ApiPort],
    tunnel_error: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(e) = tunnel_error {
        out.push(e.to_string());
    }
    for t in detected {
        let to_api = (t.target == "bridge")
            .then(|| api.iter().find(|a| a.server == t.server))
            .flatten();
        match to_api {
            Some(a) if a.server.is_none() && a.open => out.push(format!(
                "A tunnel ({}) points at the Claude API port {}, and the API is open to \
                 tunnels: anyone who finds the URL can start conversations and approve Claude's \
                 ordinary tool calls on this Mac without a token. Dangerous actions stay off for \
                 them. Stop the tunnel, or turn off Open to tunnels on the API screen and \
                 restart the API.",
                t.url,
                a.port.unwrap_or_default()
            )),
            Some(a) if a.server.is_none() => out.push(format!(
                "A tunnel ({}) points at the Claude API port {}. Anyone who finds that URL could \
                 try to drive Claude. Tokenless access on port {} is off while it runs — stop \
                 that tunnel.",
                t.url,
                a.port.unwrap_or_default(),
                a.port.unwrap_or_default()
            )),
            Some(a) if a.open => out.push(format!(
                "A tunnel ({}) points at the API server \"{}\" (port {}), which is open to \
                 tunnels: anyone who finds the URL can start conversations and approve Claude's \
                 ordinary tool calls on this Mac without a token. Dangerous actions stay off for \
                 them. Stop the tunnel, or turn off Open to tunnels and restart that server.",
                t.url,
                a.name,
                a.port.unwrap_or_default()
            )),
            Some(a) => out.push(format!(
                "A tunnel ({}) points at the API server \"{}\" (port {}). Anyone who finds that \
                 URL could try to drive Claude. Tokenless access on port {} is off while it runs \
                 — stop that tunnel.",
                t.url,
                a.name,
                a.port.unwrap_or_default(),
                a.port.unwrap_or_default()
            )),
            None => {}
        }
        if t.inspect && t.provider == "ngrok" {
            out.push(format!(
                "ngrok's local inspector (http://127.0.0.1:4040) is recording every request to \
                 {}, headers included, for anything on this Mac to read. Restart ngrok with \
                 --inspect=false.",
                t.url
            ));
        }
        if t.provider == "cloudflared" {
            out.push(format!(
                "{} is a cloudflared quick tunnel: its URL changes every time cloudflared \
                 restarts, so paired phones and push notifications stop working. Use a named \
                 tunnel, ngrok's free static domain or Tailscale for a stable URL.",
                t.url
            ));
        }
        if t.kind == "tcp" && t.target == "main" {
            out.push(format!(
                "A raw TCP tunnel ({}) points at port {main_port}. TCP tunnels hide that requests \
                 come from the internet. Use an HTTP tunnel to port {tunnel_port} instead.",
                t.url
            ));
        } else if t.kind == "tcp" && t.target == "tunnel" {
            out.push(format!(
                "A raw TCP tunnel ({}) points at port {tunnel_port}; it cannot work because the \
                 tunnel port only accepts HTTPS-terminated traffic. Use an HTTP tunnel.",
                t.url
            ));
        } else if t.kind == "http" && t.target == "main" {
            out.push(format!(
                "A tunnel ({}) points at port {main_port}. It works — those requests are treated \
                 as remote — but point it at the tunnel port {tunnel_port} instead: it serves \
                 fewer routes and requires HTTPS.",
                t.url
            ));
        }
        if t.target == "tunnel" && !settings.internet_enabled {
            out.push(format!(
                "A tunnel ({}) points at port {tunnel_port}, but Internet access is off, so it \
                 has nothing to reach.",
                t.url
            ));
        }
        if t.provider == "tailscale-funnel" {
            out.push(format!(
                "{} is public (Tailscale Funnel) and its name is published in certificate logs. \
                 For access from your own devices only, use Tailscale Serve: tailscale serve --bg \
                 {tunnel_port}.",
                t.url
            ));
        }
    }
    let reaches_us = detected
        .iter()
        .any(|t| t.kind == "http" && (t.target == "tunnel" || t.target == "unknown"));
    if settings.internet_enabled
        && !reaches_us
        && settings.manual_url.as_deref().unwrap_or("").is_empty()
    {
        out.push(format!(
            "Internet access is on, but no tunnel to port {tunnel_port} was found. Start one of \
             the commands below, or enter your public URL by hand."
        ));
    }
    out.dedup();
    out
}

/// `GET /api/remote` payload.
pub async fn status(state: &AppState) -> Value {
    state.remote.mark_page_seen();
    let settings = state.remote.settings();
    let detected = state.remote.detected();
    let listening = state.remote.listening_port().await;
    let tunnel_port = listening.unwrap_or_else(|| state.remote.tunnel_port());
    let tunnel_error = state
        .remote
        .tunnel_error
        .lock()
        .ok()
        .and_then(|g| g.clone());
    let warnings = warnings(
        &settings,
        &detected,
        tunnel_port,
        state.config.port,
        &api_ports(state),
        tunnel_error.as_deref(),
    );
    let checked_at = state
        .remote
        .detection
        .lock()
        .ok()
        .and_then(|d| d.checked_at);
    let lan = state.remote.lan();
    let installed = json!({
        "ngrok": binary("ngrok").is_some(),
        "cloudflared": binary("cloudflared").is_some(),
        "tailscale": binary("tailscale").is_some(),
    });
    let main = state.bridge.main();
    let mut bridge = json!({
        "port": state.bridge.config.port,
        "running": main.is_running(),
        "exposed": crate::bridge::control::exposed(state, &main),
    });
    // Every API server, main first, once there is more than the main one, so
    // the page can name the server a tunnel points at.
    if state.bridge.servers.has_added() {
        let servers: Vec<Value> = state
            .bridge
            .servers
            .all()
            .iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "name": s.name(),
                    "port": s.current_port(),
                    "running": s.is_running(),
                    "exposed": crate::bridge::control::exposed(state, s),
                })
            })
            .collect();
        bridge["servers"] = json!(servers);
    }
    json!({
        "lan": {
            "enabled": lan.enabled,
            "port": state.config.port,
            "addresses": lan.addresses,
            "skipped": lan.skipped,
            "fingerprint": lan.fingerprint,
            "caFingerprint": lan.ca_fingerprint,
            "caUrl": lan.ca_der.as_ref().map(|_| "/api/pair/ca.crt"),
            "mobileconfigUrl": lan.ca_der.as_ref().map(|_| "/api/pair/ca.mobileconfig"),
            "bindTailscale": settings.bind_tailscale,
            "error": lan.error,
            "note": "Pair over the LAN only on a network you control.",
        },
        "internet": {
            "enabled": settings.internet_enabled,
            "listening": listening.is_some(),
            "port": tunnel_port,
            "publicUrl": state.remote.public_url(),
            "manualUrl": settings.manual_url,
            "requireApproval": true,
            "detected": detected,
            "checkedAt": checked_at,
            "installed": installed,
            "commands": commands(tunnel_port),
            "recommended": "tailscaleServe",
            "warnings": warnings,
        },
        "bridge": bridge,
        "devices": {
            "idleDays": state.devices.idle_days(),
            "active": state.devices.active_count(),
            "pending": state.pairing.pending_count(),
        },
    })
}

/// `PUT /api/remote` `{internetEnabled?, manualUrl?, requireApproval?,
/// bindTailscale?, deviceIdleDays?}`. `manualUrl: null` or `""` clears it.
/// Internet pairings always need approval, so `requireApproval` is accepted
/// but stays `true`.
pub async fn update(state: &AppState, body: &Value) -> ApiResult<Value> {
    let Some(obj) = body.as_object() else {
        return Err(ApiError::unprocessable(
            "invalid_body",
            "Send a JSON object.",
        ));
    };
    const KNOWN: [&str; 5] = [
        "internetEnabled",
        "manualUrl",
        "requireApproval",
        "bindTailscale",
        "deviceIdleDays",
    ];
    if let Some(k) = obj.keys().find(|k| !KNOWN.contains(&k.as_str())) {
        return Err(ApiError::unprocessable(
            "invalid_body",
            format!("Unknown field `{k}`."),
        ));
    }
    let mut s = state.remote.settings();
    let before = s.clone();
    let flag = |key: &str| -> ApiResult<Option<bool>> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(ApiError::unprocessable(
                "invalid_body",
                format!("`{key}` must be true or false."),
            )),
        }
    };
    if let Some(v) = flag("internetEnabled")? {
        s.internet_enabled = v;
    }
    if let Some(v) = flag("bindTailscale")? {
        s.bind_tailscale = v;
    }
    flag("requireApproval")?;
    match obj.get("manualUrl") {
        None => {}
        Some(Value::Null) => s.manual_url = None,
        Some(Value::String(u)) if u.trim().is_empty() => s.manual_url = None,
        Some(Value::String(u)) => {
            s.manual_url = Some(super::pairing::validate_base_url(u, true)?);
        }
        Some(_) => {
            return Err(ApiError::unprocessable(
                "invalid_body",
                "`manualUrl` must be a string or null.",
            ))
        }
    }
    match obj.get("deviceIdleDays") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let days = v
                .as_u64()
                .filter(|d| (1..=3650).contains(d))
                .ok_or_else(|| {
                    ApiError::unprocessable(
                        "invalid_body",
                        "`deviceIdleDays` must be a whole number of days between 1 and 3650.",
                    )
                })?;
            s.device_idle_days = Some(days as u32);
            state.devices.set_idle_days(days as u32);
        }
    }
    state.remote.save(&s);
    crate::server::push::set_public_url(state, state.remote.public_url().as_deref());
    if s.internet_enabled != before.internet_enabled {
        apply_tunnel(state).await;
        crate::server::ws::publish_desktop_notice(
            state,
            NoticeLevel::Info,
            if s.internet_enabled {
                "Internet access is on."
            } else {
                "Internet access is off; tunnel connections were closed."
            },
        );
    }
    if s.bind_tailscale != before.bind_tailscale {
        super::listeners::refresh(state).await;
    }
    Ok(status(state).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(provider: &str, url: &str, upstream: Option<&str>, target: &str) -> DetectedTunnel {
        DetectedTunnel {
            provider: provider.into(),
            url: url.into(),
            upstream: upstream.map(str::to_string),
            upstream_port: upstream.and_then(port_of),
            inspect: false,
            kind: if url.starts_with("tcp://") {
                "tcp"
            } else {
                "http"
            }
            .into(),
            target: target.into(),
            server: None,
        }
    }

    /// The main API server on `port` alone.
    fn main_api(port: Option<u16>, open: bool) -> Vec<ApiPort> {
        vec![ApiPort {
            server: None,
            name: "Main".into(),
            port,
            open,
        }]
    }

    #[test]
    fn upstream_ports() {
        assert_eq!(port_of("http://localhost:6726"), Some(6726));
        assert_eq!(port_of("localhost:5612"), Some(5612));
        assert_eq!(port_of("5612"), Some(5612));
        assert_eq!(port_of("tcp://127.0.0.1:6725"), Some(6725));
        assert_eq!(port_of("http://127.0.0.1:6726/"), Some(6726));
        assert_eq!(port_of("http://localhost"), None);
        assert_eq!(
            classify_target(Some(6726), 6726, 6725, Some(5612)),
            "tunnel"
        );
        assert_eq!(
            classify_target(Some(5612), 6726, 6725, Some(5612)),
            "bridge"
        );
        assert_eq!(classify_target(Some(80), 6726, 6725, Some(5612)), "other");
        assert_eq!(classify_target(None, 6726, 6725, None), "unknown");
    }

    #[test]
    fn parses_ngrok_and_tailscale() {
        let v = json!({ "endpoints": [
            { "url": "https://abc.ngrok-free.app", "upstream": { "url": "http://localhost:6726" }, "inspect": true },
            { "url": "tcp://0.tcp.ngrok.io:1234", "upstream": { "url": "localhost:5612" } },
            { "nourl": true }
        ]});
        let eps = parse_ngrok_endpoints(&v);
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].1.as_deref(), Some("http://localhost:6726"));
        assert!(eps[0].2);
        let v = json!({ "tunnels": [
            { "public_url": "https://x.ngrok.app", "config": { "addr": "http://localhost:6725", "inspect": false } }
        ]});
        assert_eq!(
            parse_ngrok_tunnels(&v)[0].1.as_deref(),
            Some("http://localhost:6725")
        );
        assert!(parse_ngrok_endpoints(&json!({})).is_empty());

        let v = json!({
            "TCP": { "443": { "HTTPS": true }, "8443": { "TCPForward": "127.0.0.1:5612" } },
            "Web": { "mac.tail1.ts.net:443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:6726" } } } },
            "AllowFunnel": { "mac.tail1.ts.net:443": true }
        });
        let ts = parse_tailscale(&v);
        assert!(ts.contains(&(
            "tailscale-funnel".into(),
            "https://mac.tail1.ts.net".into(),
            Some("http://127.0.0.1:6726".into()),
            "http".into()
        )));
        assert!(ts
            .iter()
            .any(|x| x.3 == "tcp" && x.2.as_deref() == Some("127.0.0.1:5612")));
        assert!(parse_tailscale(&json!(null)).is_empty());
    }

    /// Exposure is per port: a tunnel to one API server's port is that
    /// port's, and only while the tunnel lasts.
    #[test]
    fn exposure_is_tracked_per_port() {
        let r = RemoteState::new(None);
        let to = |port: &str| t("ngrok", "tcp://0.tcp.ngrok.io:1", Some(port), "bridge");
        assert!(r.set_detected(vec![to("localhost:5613")], [5612, 5613, 5614]));
        assert!(r.port_exposed(5613));
        assert!(!r.port_exposed(5612) && !r.port_exposed(5614));
        assert!(r.bridge_exposed());
        assert_eq!(r.exposed_ports(), vec![5613]);
        // A port no server has is not tracked, however many tunnels point at it.
        assert!(!r.set_detected(
            vec![to("localhost:5613"), to("localhost:9999")],
            [5612, 5613]
        ));
        assert!(!r.port_exposed(9999));
        assert!(r.set_detected(vec![to("localhost:5612")], [5612, 5613]));
        assert_eq!(r.exposed_ports(), vec![5612]);
        assert!(r.set_detected(vec![], [5612, 5613]));
        assert!(!r.bridge_exposed());
    }

    #[test]
    fn bridge_exposure_follows_detection() {
        let r = RemoteState::new(None);
        assert!(!r.bridge_exposed());
        assert!(r.set_detected(
            vec![t(
                "ngrok",
                "tcp://0.tcp.ngrok.io:1",
                Some("localhost:5612"),
                "bridge"
            )],
            Some(5612)
        ));
        assert!(r.bridge_exposed());
        assert!(!r.set_detected(
            vec![t(
                "ngrok",
                "tcp://0.tcp.ngrok.io:1",
                Some("localhost:5612"),
                "bridge"
            )],
            Some(5612)
        ));
        assert!(r.set_detected(vec![], Some(5612)));
        assert!(!r.bridge_exposed());
    }

    #[test]
    fn public_url_prefers_manual_then_the_tunnel_port() {
        let r = RemoteState::new(None);
        assert!(r.public_url().is_none());
        r.set_detected(
            vec![
                t(
                    "ngrok",
                    "https://main.ngrok.app",
                    Some("http://localhost:6725"),
                    "main",
                ),
                t(
                    "ngrok",
                    "https://tun.ngrok.app/",
                    Some("http://localhost:6726"),
                    "tunnel",
                ),
                t("ngrok", "tcp://x:1", Some("localhost:6726"), "tunnel"),
            ],
            None,
        );
        assert_eq!(r.public_url().as_deref(), Some("https://tun.ngrok.app"));
        let mut s = r.settings();
        s.manual_url = Some("https://me.example".into());
        r.save(&s);
        assert_eq!(r.public_url().as_deref(), Some("https://me.example"));
        assert!(r.ws_origins().contains(&"wss://me.example".to_string()));
    }

    #[test]
    fn warnings_cover_the_risky_setups() {
        let s = RemoteSettings {
            internet_enabled: true,
            ..Default::default()
        };
        let mut inspect = t(
            "ngrok",
            "https://a.ngrok.app",
            Some("localhost:6726"),
            "tunnel",
        );
        inspect.inspect = true;
        let w = warnings(
            &s,
            &[
                inspect,
                t(
                    "ngrok",
                    "tcp://0.tcp.ngrok.io:9",
                    Some("localhost:5612"),
                    "bridge",
                ),
                t(
                    "ngrok",
                    "tcp://0.tcp.ngrok.io:8",
                    Some("localhost:6725"),
                    "main",
                ),
                t(
                    "cloudflared",
                    "https://x.trycloudflare.com",
                    None,
                    "unknown",
                ),
            ],
            6726,
            6725,
            &main_api(Some(5612), false),
            None,
        );
        let all = w.join("\n");
        assert!(all.contains("--inspect=false"));
        assert!(all.contains("Claude API port 5612"));
        assert!(all.contains("raw TCP tunnel"));
        assert!(all.contains("quick tunnel"));

        let w = warnings(
            &s,
            &[],
            6726,
            6725,
            &main_api(Some(5612), false),
            Some("Could not listen"),
        );
        assert!(w.iter().any(|x| x.contains("no tunnel to port 6726")));
        assert!(w.iter().any(|x| x.contains("Could not listen")));
        let off = RemoteSettings::default();
        let w = warnings(
            &off,
            &[t(
                "ngrok",
                "https://a.ngrok.app",
                Some("localhost:6726"),
                "tunnel",
            )],
            6726,
            6725,
            &main_api(None, false),
            None,
        );
        assert!(w.iter().any(|x| x.contains("Internet access is off")));

        // With the token dropped for tunnels too, the same tunnel is not
        // "switched off" — it is published, and the warning says so.
        let to_bridge = t(
            "ngrok",
            "https://me.ngrok.app",
            Some("localhost:5612"),
            "bridge",
        );
        let closed = warnings(
            &s,
            std::slice::from_ref(&to_bridge),
            6726,
            6725,
            &main_api(Some(5612), false),
            None,
        )
        .join("\n");
        assert!(closed.contains("Tokenless access on port 5612 is off"));
        let open = warnings(
            &s,
            std::slice::from_ref(&to_bridge),
            6726,
            6725,
            &main_api(Some(5612), true),
            None,
        )
        .join("\n");
        assert!(open.contains("the API is open to tunnels"));
        assert!(open.contains("turn off Open to tunnels"));
        assert!(!open.contains("is off while it runs"));

        // A tunnel to another API server's port names that server, under
        // that server's own policy.
        let mut to_work = t(
            "ngrok",
            "https://work.ngrok.app",
            Some("localhost:5613"),
            "bridge",
        );
        to_work.server = Some("work".into());
        let mut api = main_api(Some(5612), true);
        api.push(ApiPort {
            server: Some("work".into()),
            name: "Work".into(),
            port: Some(5613),
            open: false,
        });
        let w = warnings(&s, &[to_work.clone()], 6726, 6725, &api, None).join("\n");
        assert!(w.contains("the API server \"Work\" (port 5613)"), "{w}");
        assert!(w.contains("Tokenless access on port 5613 is off"), "{w}");
        assert!(
            !w.contains("5612"),
            "the main server is not the one exposed: {w}"
        );
        api[1].open = true;
        let w = warnings(&s, &[to_work], 6726, 6725, &api, None).join("\n");
        assert!(
            w.contains("\"Work\" (port 5613), which is open to tunnels"),
            "{w}"
        );
    }

    #[test]
    fn settings_persist_privately_and_tolerate_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let r = RemoteState::new(Some(dir.path()));
        let mut s = r.settings();
        s.internet_enabled = true;
        s.manual_url = Some("https://x.example".into());
        r.save(&s);
        let again = RemoteState::new(Some(dir.path()));
        assert_eq!(again.settings(), s);
        std::fs::write(dir.path().join("remote.json"), "{broken").unwrap();
        assert!(
            !RemoteState::new(Some(dir.path()))
                .settings()
                .internet_enabled
        );
    }

    #[tokio::test]
    async fn the_kill_switch_cuts_open_connections() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let switch = Arc::new(KillSwitch::default());
        let mut l = KillableListener {
            inner: listener,
            switch: switch.clone(),
        };
        let client = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });
        let (mut server_side, _) = axum::serve::Listener::accept(&mut l).await;
        let _client = client.await.unwrap();
        let reader = tokio::spawn(async move {
            let mut buf = [0u8; 8];
            tokio::io::AsyncReadExt::read(&mut server_side, &mut buf).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        switch.kill();
        let r = tokio::time::timeout(Duration::from_secs(2), reader)
            .await
            .expect("the pending read was woken")
            .unwrap();
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
    }
}
