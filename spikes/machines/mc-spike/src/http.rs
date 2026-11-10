//! The serving side: every bidirectional QUIC stream carries one HTTP/1.1
//! connection handled by an axum router (upgrades included, so WebSocket works),
//! or a raw byte stream for throughput (`RAW1` upload / `RAW2` download).

use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{Request, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use bytes::Bytes;
use futures_util::StreamExt;
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::{TokioExecutor, TokioIo},
    service::TowerToHyperService,
};
use iroh::{
    endpoint::{Connection, RecvStream, SendStream},
    Endpoint, EndpointId,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use crate::{
    args::Args,
    net::{self, ms, NetOpts},
};

/// Routes of the app this spike forwards, read-only, to a paired peer.
const FORWARD_ALLOW: &[&str] = &["/api/sessions", "/api/health"];

#[derive(Clone, Copy)]
pub struct Peer(pub EndpointId);

pub struct Forward {
    pub base: String,
    pub token: String,
    pub client: Client<HttpConnector, http_body_util::Empty<Bytes>>,
}

#[derive(Clone)]
pub struct Shared {
    pub machine: String,
    /// Local tests: any peer may use the synthetic `/spike/*` routes (never `/api`).
    pub open: bool,
    pub allowed: Arc<Mutex<HashSet<EndpointId>>>,
    pub allow_file: Option<PathBuf>,
    pub pair_code: Arc<Mutex<Option<String>>>,
    pub forward: Option<Arc<Forward>>,
}

impl Shared {
    pub fn open_for_tests() -> Self {
        Self {
            machine: hostname(),
            open: true,
            allowed: Arc::default(),
            allow_file: None,
            pair_code: Arc::default(),
            forward: None,
        }
    }

    fn paired(&self, id: &EndpointId) -> bool {
        self.allowed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(id)
    }

    fn may_use(&self, id: &EndpointId) -> bool {
        self.open || self.paired(id)
    }

    fn pairing(&self) -> bool {
        self.pair_code
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

pub fn router(sh: Shared) -> Router {
    Router::new()
        .route("/spike/hello", get(hello))
        .route("/spike/pair", post(pair))
        .route("/spike/bulk", get(bulk))
        .route("/spike/sink", post(sink))
        .route("/spike/ws", get(ws))
        .route("/api/{*rest}", get(forward))
        .with_state(sh)
}

/// Serves one connection until it closes.
pub async fn serve_conn(conn: Connection, sh: Shared) {
    let peer = conn.remote_id();
    let app = router(sh.clone()).layer(Extension(Peer(peer)));
    loop {
        let (send, mut recv) = match conn.accept_bi().await {
            Ok(s) => s,
            Err(_) => break,
        };
        let app = app.clone();
        let sh = sh.clone();
        tokio::spawn(async move {
            let mut magic = [0u8; 4];
            if recv.read_exact(&mut magic).await.is_err() {
                return;
            }
            match &magic {
                b"RAW1" if sh.may_use(&peer) => raw_sink(send, recv).await,
                b"RAW2" if sh.may_use(&peer) => raw_source(send, recv).await,
                b"RAW1" | b"RAW2" => {}
                _ => {
                    let reader = std::io::Cursor::new(magic.to_vec()).chain(recv);
                    let io = TokioIo::new(tokio::io::join(reader, send));
                    let svc = TowerToHyperService::new(app);
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, svc)
                        .with_upgrades()
                        .await;
                }
            }
        });
    }
}

/// Accepts connections forever (or until the endpoint closes).
pub async fn accept_loop(ep: Endpoint, sh: Shared) {
    while let Some(incoming) = ep.accept().await {
        let sh = sh.clone();
        tokio::spawn(async move {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("incoming connection failed: {e}");
                    return;
                }
            };
            let peer = conn.remote_id();
            if !sh.open && !sh.pairing() && !sh.paired(&peer) {
                eprintln!("refused {peer}: not paired");
                conn.close(1u32.into(), b"not paired");
                return;
            }
            eprintln!("connection from {peer} via {}", net::selected_kind(&conn));
            serve_conn(conn, sh).await;
        });
    }
}

async fn raw_sink(mut send: SendStream, mut recv: RecvStream) {
    let t = Instant::now();
    let mut buf = vec![0u8; 256 * 1024];
    let mut n = 0u64;
    loop {
        match recv.read(&mut buf).await {
            Ok(Some(k)) => n += k as u64,
            Ok(None) => break,
            Err(_) => return,
        }
    }
    let micros = t.elapsed().as_micros() as u64;
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&n.to_be_bytes());
    out[8..].copy_from_slice(&micros.to_be_bytes());
    let _ = send.write_all(&out).await;
    let _ = send.finish();
    let _ = send.stopped().await;
}

async fn raw_source(mut send: SendStream, mut recv: RecvStream) {
    let mut len = [0u8; 8];
    if recv.read_exact(&mut len).await.is_err() {
        return;
    }
    let total = u64::from_be_bytes(len).min(16 << 30);
    let chunk = vec![0x5au8; 256 * 1024];
    let mut sent = 0u64;
    while sent < total {
        let k = chunk.len().min((total - sent) as usize);
        if send.write_all(&chunk[..k]).await.is_err() {
            return;
        }
        sent += k as u64;
    }
    let _ = send.finish();
    let _ = send.stopped().await;
}

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "not paired with this machine").into_response()
}

async fn hello(State(sh): State<Shared>, Extension(peer): Extension<Peer>) -> Json<Value> {
    Json(json!({
        "machine": sh.machine,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "you": peer.0.to_string(),
        "paired": sh.paired(&peer.0),
        "pairing": sh.pairing(),
        "forwards": sh.forward.is_some(),
        "timeMs": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
    }))
}

#[derive(Deserialize)]
struct PairReq {
    code: String,
}

async fn pair(
    State(sh): State<Shared>,
    Extension(peer): Extension<Peer>,
    Json(req): Json<PairReq>,
) -> Response {
    let ok = {
        let mut c = sh.pair_code.lock().unwrap_or_else(|e| e.into_inner());
        match c.as_deref() {
            Some(code) if ct_eq(code.as_bytes(), req.code.trim().as_bytes()) => {
                *c = None; // one use
                true
            }
            _ => false,
        }
    };
    if !ok {
        return (StatusCode::FORBIDDEN, "wrong or already used pairing code").into_response();
    }
    sh.allowed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(peer.0);
    if let Some(f) = &sh.allow_file {
        if let Err(e) = append_line(f, &peer.0.to_string()) {
            eprintln!("could not save the pairing: {e}");
        }
    }
    eprintln!("paired with {}", peer.0);
    Json(json!({ "paired": true, "machine": sh.machine })).into_response()
}

#[derive(Deserialize)]
struct MibQ {
    mib: Option<u64>,
}

async fn bulk(
    State(sh): State<Shared>,
    Extension(peer): Extension<Peer>,
    Query(q): Query<MibQ>,
) -> Response {
    if !sh.may_use(&peer.0) {
        return forbidden();
    }
    let total = q.mib.unwrap_or(16).min(4096) << 20;
    let chunk = Bytes::from(vec![0x5au8; 64 * 1024]);
    let n = (total / chunk.len() as u64) as usize;
    let stream =
        futures_util::stream::iter((0..n).map(move |_| Ok::<_, std::io::Error>(chunk.clone())));
    Response::builder()
        .header("content-length", total.to_string())
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn sink(State(sh): State<Shared>, Extension(peer): Extension<Peer>, body: Body) -> Response {
    if !sh.may_use(&peer.0) {
        return forbidden();
    }
    let t = Instant::now();
    let mut n = 0u64;
    let mut s = body.into_data_stream();
    while let Some(c) = s.next().await {
        match c {
            Ok(b) => n += b.len() as u64,
            Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        }
    }
    Json(json!({ "bytes": n, "ms": ms(t.elapsed()) })).into_response()
}

async fn ws(
    State(sh): State<Shared>,
    Extension(peer): Extension<Peer>,
    up: WebSocketUpgrade,
) -> Response {
    if !sh.may_use(&peer.0) {
        return forbidden();
    }
    up.on_upgrade(|mut socket: WebSocket| async move {
        while let Some(Ok(msg)) = socket.recv().await {
            match msg {
                Message::Text(_) | Message::Binary(_) => {
                    if socket.send(msg).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    })
}

/// Forwards a read-only GET to this machine's Mother Claude, as a local script
/// (its `script-token`, a loopback Host, no proxy header), for a paired peer only.
async fn forward(State(sh): State<Shared>, Extension(peer): Extension<Peer>, uri: Uri) -> Response {
    let Some(f) = sh.forward.clone() else {
        return (StatusCode::NOT_FOUND, "this machine forwards nothing").into_response();
    };
    if !sh.paired(&peer.0) {
        return forbidden();
    }
    if !FORWARD_ALLOW.contains(&uri.path()) {
        return (
            StatusCode::FORBIDDEN,
            "not a read-only route this spike forwards",
        )
            .into_response();
    }
    let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let req = match Request::get(format!("{}{}", f.base, path))
        .header("authorization", format!("Bearer {}", f.token))
        .header("accept", "application/json")
        .body(http_body_util::Empty::<Bytes>::new())
    {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    match f.client.request(req).await {
        Ok(resp) => {
            let (parts, body) = resp.into_parts();
            Response::from_parts(parts, Body::new(body))
        }
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

pub async fn cmd_serve(a: &Args) -> Result<()> {
    let o = NetOpts::from_args(a)?;
    let name = a.get("name").unwrap_or("spike").to_string();
    let key = net::load_or_create_key(&net::key_path(a, "spike"))?;
    let ep = net::bind(key, &o, true).await?;

    let allow_file = Some(match a.get("allow-file") {
        Some(p) => PathBuf::from(p),
        None => net::spike_dir().join(format!("{name}.allowed")),
    });
    let mut allowed = HashSet::new();
    for s in a.all("allow") {
        allowed.insert(EndpointId::from_str(&s).with_context(|| format!("--allow {s}"))?);
    }
    if let Some(Ok(text)) = allow_file.as_ref().map(fs::read_to_string) {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Ok(id) = EndpointId::from_str(line) {
                allowed.insert(id);
            }
        }
    }
    let pair_code = a.get("pair-code").map(|c| {
        if c == "auto" {
            format!("{:06}", rand::random::<u32>() % 1_000_000)
        } else {
            c.to_string()
        }
    });
    let forward = match a.get("forward") {
        Some(base) => {
            let tf = match a.get("token-file") {
                Some("auto") | None => {
                    app_script_token_path().context("no app data dir on this machine")?
                }
                Some(p) => PathBuf::from(p),
            };
            let token = fs::read_to_string(&tf)
                .with_context(|| format!("reading the script token at {}", tf.display()))?
                .trim()
                .to_string();
            Some(Arc::new(Forward {
                base: base.trim_end_matches('/').to_string(),
                token,
                client: Client::builder(TokioExecutor::new()).build_http(),
            }))
        }
        None => None,
    };
    let open = a.has("open");
    if open && forward.is_some() {
        bail!("--open and --forward cannot be combined");
    }
    let sh = Shared {
        machine: hostname(),
        open,
        allowed: Arc::new(Mutex::new(allowed)),
        allow_file,
        pair_code: Arc::new(Mutex::new(pair_code.clone())),
        forward: forward.clone(),
    };
    if let Err(e) = net::wait_online(&ep, &o, 15).await {
        eprintln!("warning: {e}");
    }
    println!(
        "{}",
        json!({
            "id": ep.id().to_string(),
            "ticket": net::ticket(&ep),
            "net": o.describe(),
            "pairCode": pair_code,
            "paired": sh.allowed.lock().map(|s| s.len()).unwrap_or(0),
            "forwards": forward.is_some(),
            "open": open,
        })
    );
    accept_loop(ep, sh).await;
    Ok(())
}

/// The app's own data dir (`persist::app_data_dir`): the same `directories` call.
pub fn app_script_token_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "motherclaude", "Mother Claude")
        .map(|d| d.data_dir().join("script-token"))
}

pub fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn append_line(path: &PathBuf, line: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")
}
