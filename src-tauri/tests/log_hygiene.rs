//! Log hygiene (design §10 B1): with every `tracing` event captured at TRACE,
//! a full pairing, WebSocket first-frame auth, a local `?token=` request and
//! rejected credentials leave no token, pairing code or poll secret in the
//! logs, and spans carry paths without query strings.
//!
//! Its own test binary: a thread-local subscriber is only reliable when no
//! other test runs in the same process.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

fn state() -> AppState {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.keep();
    Inner::new(
        ClaudeHome::with_base(&base),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
    )
}

async fn serve(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    format!("http://{addr}")
}

fn via_tunnel(ip: &str) -> reqwest::Client {
    let mut h = HeaderMap::new();
    h.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    h.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_str(ip).unwrap(),
    );
    reqwest::Client::builder()
        .default_headers(h)
        .build()
        .unwrap()
}

fn desk() -> reqwest::Client {
    reqwest::Client::new()
}

async fn json_of(r: reqwest::Response) -> Value {
    r.json().await.unwrap()
}

async fn next_frame<S>(ws: &mut S) -> Option<Value>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t).ok()?;
            if v["t"] != "ping" {
                return Some(v);
            }
        }
    }
}

/// A `tracing` writer that collects everything.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn secrets_never_reach_the_logs() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(capture.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let st = state();
    let local = serve(server::router_for(st.clone(), Ingress::Loopback)).await;
    let tunnel = serve(server::router_for(st.clone(), Ingress::Tunnel)).await;
    let d = st.auth.token.clone();
    // A pairing, end to end.
    let code = json_of(
        desk()
            .post(format!("{local}/api/pair/codes"))
            .bearer_auth(&d)
            .json(&json!({ "target": "internet", "baseUrl": "https://mc.example.ts.net" }))
            .send()
            .await
            .unwrap(),
    )
    .await["code"]
        .as_str()
        .unwrap()
        .to_string();
    let phone = via_tunnel("203.0.113.110");
    let p = json_of(
        phone
            .post(format!("{tunnel}/api/pair/exchange"))
            .json(&json!({ "code": code, "deviceName": "Logged phone" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let secret = p["pollSecret"].as_str().unwrap().to_string();
    let id = p["pendingId"].as_str().unwrap().to_string();
    desk()
        .post(format!("{local}/api/devices/pending/{id}/approve"))
        .bearer_auth(&d)
        .json(&json!({ "fingerprint": p["fingerprint"] }))
        .send()
        .await
        .unwrap();
    let token = json_of(
        phone
            .get(format!("{tunnel}/api/pair/pending/{id}"))
            .header("x-pair-secret", &secret)
            .send()
            .await
            .unwrap(),
    )
    .await["token"]
        .as_str()
        .unwrap()
        .to_string();
    // WebSocket first-frame auth.
    let url = format!("{}/ws", tunnel.replacen("http", "ws", 1));
    let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        url.as_str(),
    )
    .unwrap();
    req.headers_mut()
        .insert("x-forwarded-proto", "https".parse().unwrap());
    req.headers_mut()
        .insert("x-forwarded-for", "203.0.113.110".parse().unwrap());
    let mut ws = tokio_tungstenite::connect_async(req).await.unwrap().0;
    ws.send(Message::Text(
        json!({ "t": "auth", "token": token }).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_frame(&mut ws).await.unwrap()["t"], "hello");
    // A local ?token= request and failed credentials.
    desk()
        .get(format!("{local}/api/sessions?token={d}"))
        .send()
        .await
        .unwrap();
    let bogus = "mcd_this-is-a-bogus-token-value";
    phone
        .get(format!("{tunnel}/api/me"))
        .bearer_auth(bogus)
        .send()
        .await
        .unwrap();
    desk()
        .get(format!("{local}/api/me"))
        .bearer_auth(&st.auth.script_token)
        .header("x-forwarded-for", "203.0.113.111")
        .send()
        .await
        .unwrap();
    drop(ws);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(
        logs.contains("/api/pair/exchange"),
        "spans were captured:\n{logs}"
    );
    assert!(logs.contains("rejected credential"));
    for (what, secret) in [
        ("desktop token", d.as_str()),
        ("script token", st.auth.script_token.as_str()),
        ("hook token", st.auth.hook_token.as_str()),
        ("device token", token.as_str()),
        ("bogus token", bogus),
        ("pairing code", code.as_str()),
        ("poll secret", secret.as_str()),
    ] {
        assert!(!logs.contains(secret), "the {what} leaked into the logs");
    }
    let leaks: Vec<&str> = logs.lines().filter(|l| l.contains("?token=")).collect();
    assert!(leaks.is_empty(), "no query strings in spans: {leaks:#?}");
}
