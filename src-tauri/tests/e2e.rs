//! End-to-end smoke test of the integrated server: one state behind the
//! loopback listener (the desktop, the hook plugin) and the tunnel ingress (a
//! phone on the internet), over a synthetic `~/.claude` in a temp dir.
//!
//! The phone pairs with a code and the desktop's typed approval, reads the
//! session list (with ETag revalidation), authenticates its WebSocket with
//! the first frame, subscribes and receives a live transcript append, answers
//! a held PermissionRequest from a foreign session, uploads a file, is kept
//! off dangerous routes through the tunnel (even with a LAN grant), and is
//! locked out (WebSocket closed with 4401, REST 401) once revoked.
//!
//! Nothing here touches the real `~/.claude`, runs `claude`, or opens a
//! public tunnel.

use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mother_claude_lib::bridge::BridgeConfig;
use mother_claude_lib::claude::hooks::{write_plugin_files, write_runtime_files, PluginPaths};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, monitor, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// The foreign (VS Code) session the phone watches and answers.
const SID: &str = "e2e0e2e0-1111-4222-8333-444455556666";
const PHONE_IP: &str = "203.0.113.44";

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn transcript(base: &Path) -> std::path::PathBuf {
    base.join(format!("projects/-tmp-e2e/{SID}.jsonl"))
}

/// A live VS Code session run by this very process (so the hook binding
/// traces requests from this process to it), with a short transcript.
fn fixture(base: &Path) {
    let me = std::process::id();
    let t0 = now() - 30_000;
    std::fs::create_dir_all(base.join("projects/-tmp-e2e")).unwrap();
    std::fs::create_dir_all(base.join("sessions")).unwrap();
    std::fs::write(
        base.join(format!("sessions/{me}.json")),
        json!({
            "pid": me, "sessionId": SID, "cwd": "/tmp/e2e", "startedAt": t0 - 1000,
            "kind": "interactive", "entrypoint": "claude-vscode", "version": "2.1.282",
            "status": "busy", "statusUpdatedAt": t0 + 1000
        })
        .to_string(),
    )
    .unwrap();
    let lines = [
        json!({"type":"user","uuid":"e2e-u1","timestamp":iso(t0),"cwd":"/tmp/e2e","permissionMode":"default","origin":{"kind":"human"},"message":{"role":"user","content":"list the files"}}),
        json!({"type":"assistant","uuid":"e2e-a1","timestamp":iso(t0 + 1000),"message":{"id":"msg_1","model":"claude-haiku-4-5","content":[{"type":"text","text":"Sure."}],"usage":{"input_tokens":10,"output_tokens":2}}}),
    ];
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    std::fs::write(transcript(base), text).unwrap();
}

fn state(home: &Path, data: &Path) -> AppState {
    let mut auth = Auth::ephemeral();
    // A real data dir (devices, uploads) inside the test's temp dir.
    auth.config_dir = Some(data.to_path_buf());
    Inner::with_bridge_config(
        ClaudeHome::with_base(home),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
        BridgeConfig {
            enabled: true,
            port: None,
            require_token: false,
            api_key: None,
            allowed_origins: Vec::new(),
        },
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

fn tunnel_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    h.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static(PHONE_IP),
    );
    h
}

/// The phone: every request looks like it came through a TLS-terminating
/// tunnel from `PHONE_IP`.
fn phone() -> reqwest::Client {
    reqwest::Client::builder()
        .default_headers(tunnel_headers())
        .build()
        .unwrap()
}

fn desk() -> reqwest::Client {
    reqwest::Client::new()
}

async fn json_of(r: reqwest::Response) -> Value {
    r.json().await.unwrap()
}

async fn next_json(ws: &mut Ws) -> Option<Value> {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .ok()??
            .ok()?;
        match msg {
            Message::Text(t) => return serde_json::from_str(&t).ok(),
            Message::Close(_) => return None,
            _ => {}
        }
    }
}

/// Skip frames until one of type `t` (panics after 5 s of silence).
async fn until(ws: &mut Ws, t: &str) -> Value {
    loop {
        let v = next_json(ws)
            .await
            .unwrap_or_else(|| panic!("no {t} frame"));
        if v["t"] == t {
            return v;
        }
    }
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

#[tokio::test]
async fn a_phone_pairs_watches_answers_and_is_locked_out_after_revocation() {
    let home = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(home.path());
    let st = state(home.path(), data.path());
    monitor::refresh_once(&st).await;
    let local = serve(server::router_for(st.clone(), Ingress::Loopback)).await;
    let tunnel = serve(server::router_for(st.clone(), Ingress::Tunnel)).await;
    let desktop = st.auth.token.clone();

    // ── Pairing: code on the desktop, exchange on the phone, typed approval.
    let code = json_of(
        desk()
            .post(format!("{local}/api/pair/codes"))
            .bearer_auth(&desktop)
            .json(&json!({ "target": "internet", "baseUrl": "https://mc.example.ts.net" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let code = code["code"].as_str().unwrap().to_string();
    let r = phone()
        .post(format!("{tunnel}/api/pair/exchange"))
        .json(&json!({ "code": code, "deviceName": "E2E phone", "platform": "ios" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED, "internet pairings wait");
    let pending = json_of(r).await;
    let pending_id = pending["pendingId"].as_str().unwrap().to_string();
    let r = desk()
        .post(format!("{local}/api/devices/pending/{pending_id}/approve"))
        .bearer_auth(&desktop)
        .json(&json!({ "fingerprint": pending["fingerprint"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let approved = json_of(
        phone()
            .get(format!("{tunnel}/api/pair/pending/{pending_id}"))
            .header("x-pair-secret", pending["pollSecret"].as_str().unwrap())
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(approved["status"], "approved");
    let token = approved["token"].as_str().unwrap().to_string();
    let device_id = approved["deviceId"].as_str().unwrap().to_string();
    assert!(token.starts_with("mcd_"));

    // ── Who am I, and the session list (with ETag revalidation).
    let me = json_of(
        phone()
            .get(format!("{tunnel}/api/me"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(me["principal"], "device");
    assert_eq!(me["allowDangerous"], false);
    let r = phone()
        .get(format!("{tunnel}/api/sessions"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let etag = r.headers()["etag"].to_str().unwrap().to_string();
    let list = json_of(r).await;
    let row = list["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == SID)
        .cloned()
        .expect("the foreign session is listed");
    assert_eq!(row["live"], true);
    assert_eq!(row["surface"], "vscode");
    assert!(row["control"]["limits"].is_array());
    let r = phone()
        .get(format!("{tunnel}/api/sessions?scope=active"))
        .bearer_auth(&token)
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_MODIFIED);

    // ── WebSocket through the tunnel: first-frame auth, then a subscription.
    let ws_url = format!("{}/ws", tunnel.replacen("http", "ws", 1));
    let mut req = ws_url.as_str().into_client_request().unwrap();
    for (k, v) in tunnel_headers().iter() {
        req.headers_mut().insert(k.clone(), v.clone());
    }
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    send(&mut ws, json!({ "t": "auth", "token": token })).await;
    let hello = until(&mut ws, "hello").await;
    assert_eq!(hello["v"], 2);
    assert_eq!(hello["me"]["principal"], "device");
    assert_eq!(hello["me"]["deviceId"], device_id.as_str());
    let full = until(&mut ws, "sessions").await;
    assert_eq!(full["full"], true);
    send(&mut ws, json!({ "t": "sub", "session": SID })).await;
    send(&mut ws, json!({ "t": "ping" })).await;
    until(&mut ws, "pong").await;
    assert!(st.hub.is_subscribed(SID, None));

    // A line lands in the transcript: the subscriber gets it.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(transcript(home.path()))
        .unwrap();
    writeln!(
        f,
        "{}",
        json!({"type":"assistant","uuid":"e2e-a2","timestamp":iso(now()),"message":{"id":"msg_2","model":"claude-haiku-4-5","content":[{"type":"text","text":"Here they are."}],"usage":{"input_tokens":1,"output_tokens":1}}})
    )
    .unwrap();
    drop(f);
    monitor::refresh_once(&st).await;
    let frame = until(&mut ws, "transcript").await;
    assert_eq!(frame["session"], SID);
    assert!(frame["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["text"] == "Here they are." && e["messageId"] == "msg_2"));
    assert!(frame["tail"].as_str().is_some());

    // ── A held PermissionRequest from the foreign session's hook plugin: the
    // generated `mc-hook` script runs as a child of the session's process
    // (this test), so the server can trace the connection back to it.
    let plugin = PluginPaths::new(data.path());
    write_plugin_files(&plugin).unwrap();
    let port: u16 = local.rsplit(':').next().unwrap().parse().unwrap();
    write_runtime_files(&plugin, port, &st.auth.hook_token).unwrap();
    let hook = {
        let script = plugin.plugin.mc_hook.clone();
        let body = json!({
            "hook_event_name": "PermissionRequest", "session_id": SID,
            "tool_name": "Bash", "tool_input": { "command": "ls", "description": "List files" },
            "permission_suggestions": [{ "type": "addRules", "behavior": "allow",
                "destination": "localSettings",
                "rules": [{ "toolName": "Bash", "ruleContent": "ls:*" }] }]
        })
        .to_string();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut child = tokio::process::Command::new(script)
                .args(["PermissionRequest", "60", "decide"])
                .env_clear()
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(body.as_bytes()).await.unwrap();
            drop(stdin);
            let out = child.wait_with_output().await.unwrap();
            assert!(out.status.success(), "mc-hook always exits 0");
            serde_json::from_slice::<Value>(&out.stdout).unwrap_or(Value::Null)
        })
    };
    let item = until(&mut ws, "pending").await["item"].clone();
    assert_eq!(item["sessionId"], SID);
    assert_eq!(item["source"], "hook");
    assert_eq!(item["toolName"], "Bash");
    assert_eq!(item["answerable"], true);
    let request_id = item["id"].as_str().unwrap().to_string();
    // The settings-file rule is dangerous for this phone; its session-scoped
    // copy is not.
    let suggestions = item["suggestions"].as_array().unwrap();
    let standing = suggestions
        .iter()
        .find(|s| s["destination"] == "localSettings")
        .unwrap()["index"]
        .clone();
    let r = phone()
        .post(format!("{tunnel}/api/sessions/{SID}/respond"))
        .bearer_auth(&token)
        .json(&json!({ "requestId": request_id, "decision": "allow", "suggestion": standing }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        StatusCode::FORBIDDEN,
        "standing grant from a phone"
    );
    let r = phone()
        .post(format!("{tunnel}/api/sessions/{SID}/respond"))
        .bearer_auth(&token)
        .json(&json!({ "requestId": request_id, "decision": "allow" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let decided = tokio::time::timeout(Duration::from_secs(10), hook)
        .await
        .expect("the hook request is answered")
        .unwrap();
    assert_eq!(
        decided["hookSpecificOutput"]["hookEventName"],
        "PermissionRequest"
    );
    assert_eq!(
        decided["hookSpecificOutput"]["decision"]["behavior"],
        "allow"
    );
    let resolved = until(&mut ws, "pendingResolved").await;
    assert_eq!(resolved["id"], request_id.as_str());
    assert_eq!(resolved["outcome"], "allowed");
    assert_eq!(resolved["by"]["kind"], "device");
    // A late second answer is refused.
    let r = phone()
        .post(format!("{tunnel}/api/sessions/{SID}/respond"))
        .bearer_auth(&token)
        .json(&json!({ "requestId": request_id, "decision": "deny" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);

    // ── An attachment is kept for an @path reference.
    let up = phone()
        .post(format!("{tunnel}/api/sessions/{SID}/uploads"))
        .bearer_auth(&token)
        .json(&json!({ "name": "../notes.txt", "mediaType": "text/plain", "data": "aGVsbG8=" }))
        .send()
        .await
        .unwrap();
    assert_eq!(up.status(), StatusCode::OK);
    let up = json_of(up).await;
    let path = std::path::PathBuf::from(up["path"].as_str().unwrap());
    assert!(path.starts_with(data.path().join("uploads").join(SID)));
    assert!(path.to_string_lossy().ends_with("-notes.txt"));
    assert_eq!(std::fs::read(&path).unwrap(), b"hello");

    // ── Through the tunnel the phone never reaches dangerous or admin routes.
    let rm = || async {
        phone()
            .post(format!("{tunnel}/api/sessions/{SID}/rm"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
    };
    let r = rm().await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"]["code"], "dangerous_blocked");
    // A LAN grant does not apply to a tunnelled request.
    let r = desk()
        .patch(format!("{local}/api/devices/{device_id}"))
        .bearer_auth(&desktop)
        .json(&json!({ "allowDangerous": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(rm().await.status(), StatusCode::FORBIDDEN);
    // Proxy headers on the loopback listener count as the internet too.
    let r = phone()
        .post(format!("{local}/api/sessions/{SID}/rm"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // Admin routes are desktop-only.
    let r = phone()
        .post(format!("{tunnel}/api/pair/codes"))
        .bearer_auth(&token)
        .json(&json!({ "target": "lan" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = phone()
        .get(format!("{tunnel}/api/devices"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // The desktop token is refused from anything that isn't this machine.
    let r = phone()
        .get(format!("{local}/api/sessions"))
        .bearer_auth(&desktop)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // The hook channel and the bridge are not served through a tunnel.
    let r = phone()
        .post(format!("{tunnel}/hooks/event"))
        .bearer_auth(&st.auth.hook_token)
        .json(&json!({ "hook_event_name": "Stop", "session_id": SID }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = phone()
        .get(format!("{tunnel}/v1/models"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    // Without the https hop the tunnel listener refuses outright.
    let r = reqwest::Client::new()
        .get(format!("{tunnel}/api/sessions"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);

    // ── Revocation: the socket closes with 4401, REST answers 401.
    let r = desk()
        .delete(format!("{local}/api/devices/{device_id}"))
        .bearer_auth(&desktop)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let closed = loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Close(frame)))) => break frame.map(|f| u16::from(f.code)),
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(_))) | Ok(None) => break None,
            Err(_) => panic!("the revoked device's socket stayed open"),
        }
    };
    assert_eq!(closed, Some(4401));
    let r = phone()
        .get(format!("{tunnel}/api/sessions"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_bodies_are_capped_except_where_a_route_needs_more() {
    let home = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(home.path());
    let st = state(home.path(), data.path());
    monitor::refresh_once(&st).await;
    let local = serve(server::router_for(st.clone(), Ingress::Loopback)).await;
    let big = "x".repeat(100 * 1024);
    // A command route keeps the small default.
    let r = desk()
        .post(format!("{local}/api/sessions/{SID}/rename"))
        .bearer_auth(&st.auth.token)
        .json(&json!({ "name": big }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // A message may carry much more (images); it gets past the limit.
    let r = desk()
        .post(format!("{local}/api/sessions/{SID}/message"))
        .bearer_auth(&st.auth.token)
        .json(&json!({ "text": big }))
        .send()
        .await
        .unwrap();
    assert_ne!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
