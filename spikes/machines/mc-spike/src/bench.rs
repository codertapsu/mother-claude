//! The client side: HTTP/1.1 and WebSocket over a fresh QUIC stream per
//! connection, raw stream throughput, and the measurement suite.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full, StreamBody};
use hyper::{
    body::{Frame, Incoming},
    Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use iroh::endpoint::Connection;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

use crate::{
    args::Args,
    net::{self, ms, NetOpts},
};

type Body = BoxBody<Bytes, std::io::Error>;

fn empty() -> Body {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn full(b: impl Into<Bytes>) -> Body {
    Full::new(b.into()).map_err(|never| match never {}).boxed()
}

/// One HTTP/1.1 connection on a new bidirectional stream.
pub async fn send(conn: &Connection, req: Request<Body>) -> Result<Response<Incoming>> {
    let (send, recv) = conn.open_bi().await?;
    let io = TokioIo::new(tokio::io::join(recv, send));
    let (mut tx, c) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        let _ = c.await;
    });
    Ok(tx.send_request(req).await?)
}

pub async fn get(conn: &Connection, path: &str) -> Result<(StatusCode, Bytes)> {
    let req = Request::get(path).header("host", "peer").body(empty())?;
    let resp = send(conn, req).await?;
    let status = resp.status();
    Ok((status, resp.into_body().collect().await?.to_bytes()))
}

pub async fn get_json(conn: &Connection, path: &str) -> Result<Value> {
    let (status, body) = get(conn, path).await?;
    if !status.is_success() {
        bail!("GET {path}: {status}: {}", String::from_utf8_lossy(&body));
    }
    Ok(serde_json::from_slice(&body)?)
}

pub async fn post_json(conn: &Connection, path: &str, v: &Value) -> Result<(StatusCode, Bytes)> {
    let req = Request::post(path)
        .header("host", "peer")
        .header("content-type", "application/json")
        .body(full(serde_json::to_vec(v)?))?;
    let resp = send(conn, req).await?;
    let status = resp.status();
    Ok((status, resp.into_body().collect().await?.to_bytes()))
}

fn stats(mut v: Vec<Duration>) -> Value {
    if v.is_empty() {
        return Value::Null;
    }
    v.sort();
    let pick = |q: f64| ms(v[((v.len() - 1) as f64 * q).round() as usize]);
    json!({ "n": v.len(), "minMs": ms(v[0]), "p50Ms": pick(0.5), "p95Ms": pick(0.95), "maxMs": ms(v[v.len() - 1]) })
}

fn rate(bytes: u64, d: Duration) -> Value {
    let mib = bytes as f64 / (1u64 << 20) as f64;
    let s = d.as_secs_f64().max(1e-9);
    json!({ "MiB": (mib * 100.0).round() / 100.0, "secs": (s * 1000.0).round() / 1000.0,
            "MiBps": (mib / s * 10.0).round() / 10.0, "Mbitps": (bytes as f64 * 8.0 / 1e6 / s).round() })
}

async fn raw_upload(conn: &Connection, mib: u64) -> Result<Value> {
    let (mut send, mut recv) = conn.open_bi().await?;
    let t = Instant::now();
    send.write_all(b"RAW1").await?;
    let chunk = vec![0x5au8; 256 * 1024];
    let total = mib << 20;
    let mut sent = 0u64;
    while sent < total {
        send.write_all(&chunk).await?;
        sent += chunk.len() as u64;
    }
    send.finish()?;
    let mut ack = [0u8; 16];
    recv.read_exact(&mut ack).await?;
    let got = u64::from_be_bytes(ack[..8].try_into()?);
    if got != total {
        bail!("raw upload: sent {total}, the peer got {got}");
    }
    Ok(rate(total, t.elapsed()))
}

async fn raw_download(conn: &Connection, mib: u64) -> Result<Value> {
    let (mut send, mut recv) = conn.open_bi().await?;
    let t = Instant::now();
    let total = mib << 20;
    let mut hdr = b"RAW2".to_vec();
    hdr.extend_from_slice(&total.to_be_bytes());
    send.write_all(&hdr).await?;
    send.finish()?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut n = 0u64;
    while let Some(k) = recv.read(&mut buf).await? {
        n += k as u64;
    }
    if n != total {
        bail!("raw download: asked {total}, got {n}");
    }
    Ok(rate(n, t.elapsed()))
}

async fn http_download(conn: &Connection, mib: u64) -> Result<Value> {
    let t = Instant::now();
    let req = Request::get(format!("/spike/bulk?mib={mib}"))
        .header("host", "peer")
        .body(empty())?;
    let resp = send(conn, req).await?;
    if !resp.status().is_success() {
        bail!("bulk: {}", resp.status());
    }
    let mut body = resp.into_body();
    let mut n = 0u64;
    while let Some(frame) = BodyExt::frame(&mut body).await {
        if let Ok(data) = frame?.into_data() {
            n += data.len() as u64;
        }
    }
    Ok(rate(n, t.elapsed()))
}

async fn http_upload(conn: &Connection, mib: u64) -> Result<Value> {
    let chunk = Bytes::from(vec![0x5au8; 64 * 1024]);
    let total = mib << 20;
    let n = (total / chunk.len() as u64) as usize;
    let stream = futures_util::stream::iter(
        (0..n).map(move |_| Ok::<_, std::io::Error>(Frame::data(chunk.clone()))),
    );
    let body = BodyExt::boxed(StreamBody::new(stream));
    let t = Instant::now();
    let req = Request::post("/spike/sink")
        .header("host", "peer")
        .header("content-length", total.to_string())
        .body(body)?;
    let resp = send(conn, req).await?;
    let status = resp.status();
    let bytes = resp.into_body().collect().await?.to_bytes();
    if !status.is_success() {
        bail!("sink: {status}: {}", String::from_utf8_lossy(&bytes));
    }
    let v: Value = serde_json::from_slice(&bytes)?;
    if v["bytes"].as_u64() != Some(total) {
        bail!("sink got {:?} of {total}", v["bytes"]);
    }
    Ok(rate(total, t.elapsed()))
}

async fn ws_echo(conn: &Connection, n: usize) -> Result<Value> {
    let (send, recv) = conn.open_bi().await?;
    let (mut ws, _) =
        tokio_tungstenite::client_async("ws://peer/spike/ws", tokio::io::join(recv, send))
            .await
            .context("websocket handshake over the stream")?;
    let mut lat = Vec::with_capacity(n);
    for i in 0..n {
        let t = Instant::now();
        ws.send(Message::text(format!("ping {i}"))).await?;
        loop {
            match ws.next().await {
                Some(Ok(m)) if m.is_text() => break,
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(e.into()),
                None => bail!("websocket closed"),
            }
        }
        lat.push(t.elapsed());
    }
    let _ = ws.close(None).await;
    Ok(stats(lat))
}

pub struct Suite {
    pub mib: u64,
    pub requests: usize,
    pub ws_msgs: usize,
    pub concurrent: usize,
}

pub async fn run_suite(conn: &Connection, s: &Suite) -> Result<Value> {
    let mut out = serde_json::Map::new();
    let t = Instant::now();
    let hello = get_json(conn, "/spike/hello").await?;
    out.insert("firstRequestMs".into(), json!(ms(t.elapsed())));
    out.insert("hello".into(), hello);

    let mut lat = Vec::new();
    for _ in 0..s.requests {
        let t = Instant::now();
        get_json(conn, "/spike/hello").await?;
        lat.push(t.elapsed());
    }
    out.insert("requestOnNewStream".into(), stats(lat));

    let t = Instant::now();
    let futs = (0..s.concurrent).map(|_| get_json(conn, "/spike/hello"));
    futures_util::future::try_join_all(futs).await?;
    out.insert(
        "concurrent".into(),
        json!({ "n": s.concurrent, "totalMs": ms(t.elapsed()) }),
    );

    out.insert("websocketEcho".into(), ws_echo(conn, s.ws_msgs).await?);
    out.insert("rawUpload".into(), raw_upload(conn, s.mib).await?);
    out.insert("rawDownload".into(), raw_download(conn, s.mib).await?);
    out.insert("httpDownload".into(), http_download(conn, s.mib).await?);
    out.insert("httpUpload".into(), http_upload(conn, s.mib).await?);
    out.insert("pathsAtEnd".into(), net::paths(conn));
    Ok(Value::Object(out))
}

/// Connects, retrying while the address lookups catch up (bounded).
pub async fn connect(
    ep: &iroh::Endpoint,
    addr: iroh::EndpointAddr,
    within: Duration,
) -> Result<(Connection, Duration)> {
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < within {
        let t = Instant::now();
        match tokio::time::timeout(Duration::from_secs(15), ep.connect(addr.clone(), net::ALPN))
            .await
        {
            Ok(Ok(c)) => return Ok((c, t.elapsed())),
            Ok(Err(e)) => last = Some(e.to_string()),
            Err(_) => last = Some("timed out after 15s".into()),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(anyhow!(
        "could not connect within {within:?}: {}",
        last.unwrap_or_default()
    ))
}

/// Watches the selected path for a while and reports when it changed.
pub async fn watch_path(conn: &Connection, secs: u64) -> Value {
    let start = Instant::now();
    let mut last = net::selected_kind(conn);
    let mut changes = vec![json!({ "atMs": 0, "path": last })];
    while start.elapsed() < Duration::from_secs(secs) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let k = net::selected_kind(conn);
        if k != last {
            changes.push(json!({ "atMs": ms(start.elapsed()), "path": k }));
            last = k;
        }
    }
    Value::Array(changes)
}

pub async fn cmd_bench(a: &Args) -> Result<()> {
    let target = a.pos.first().context("bench TICKET|ID")?;
    let o = NetOpts::from_args(a)?;
    let key = net::load_or_create_key(&net::key_path(a, "client"))?;
    let ep = net::bind(key, &o, false).await?;
    if let Err(e) = net::wait_online(&ep, &o, 15).await {
        eprintln!("warning: {e}");
    }
    let addr = net::target_addr(target, &o)?;
    let (conn, connect) = connect(&ep, addr, Duration::from_secs(60)).await?;
    let mut out = serde_json::Map::new();
    out.insert("me".into(), json!(ep.id().to_string()));
    out.insert("net".into(), o.describe());
    out.insert("connectMs".into(), json!(ms(connect)));
    out.insert("pathsAtConnect".into(), net::paths(&conn));
    if let Some(code) = a.get("pair-code") {
        let (status, body) = post_json(&conn, "/spike/pair", &json!({ "code": code })).await?;
        out.insert(
            "pair".into(),
            json!({ "status": status.as_u16(), "body": String::from_utf8_lossy(&body) }),
        );
    }
    out.insert(
        "pathChanges".into(),
        watch_path(&conn, a.num("watch", 3)?).await,
    );
    let suite = Suite {
        mib: a.num("mib", 64)?,
        requests: a.num("requests", 50)? as usize,
        ws_msgs: a.num("ws", 200)? as usize,
        concurrent: a.num("concurrent", 32)? as usize,
    };
    out.insert("suite".into(), run_suite(&conn, &suite).await?);
    println!("{}", serde_json::to_string_pretty(&Value::Object(out))?);
    conn.close(0u32.into(), b"done");
    ep.close().await;
    Ok(())
}

pub async fn cmd_get(a: &Args) -> Result<()> {
    let target = a.pos.first().context("get TICKET|ID PATH")?;
    let path = a.pos.get(1).context("get TICKET|ID PATH")?;
    let o = NetOpts::from_args(a)?;
    let key = net::load_or_create_key(&net::key_path(a, "client"))?;
    let ep = net::bind(key, &o, false).await?;
    if let Err(e) = net::wait_online(&ep, &o, 15).await {
        eprintln!("warning: {e}");
    }
    let (conn, connect) =
        connect(&ep, net::target_addr(target, &o)?, Duration::from_secs(60)).await?;
    if let Some(code) = a.get("pair-code") {
        let (status, _) = post_json(&conn, "/spike/pair", &json!({ "code": code })).await?;
        eprintln!("pairing: {status}");
    }
    let t = Instant::now();
    let (status, body) = get(&conn, path).await?;
    let took = t.elapsed();
    eprintln!(
        "connected in {} ms via {}, {status} in {} ms, {} bytes",
        ms(connect),
        net::selected_kind(&conn),
        ms(took),
        body.len()
    );
    if path.starts_with("/api/sessions") && status.is_success() {
        // A summary only: how many sessions and in which states, never their content.
        let v: Value = serde_json::from_slice(&body)?;
        let list = v["sessions"].as_array().cloned().unwrap_or_default();
        let mut by_status = std::collections::BTreeMap::<String, u64>::new();
        for s in &list {
            let k = s["status"]
                .as_str()
                .or_else(|| s["activity"].as_str())
                .unwrap_or("?")
                .to_string();
            *by_status.entry(k).or_default() += 1;
        }
        println!(
            "{}",
            json!({ "sessions": list.len(), "byStatus": by_status, "bytes": body.len() })
        );
    } else {
        println!("{}", String::from_utf8_lossy(&body));
    }
    conn.close(0u32.into(), b"done");
    ep.close().await;
    Ok(())
}
