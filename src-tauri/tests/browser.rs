//! The browser extension's channel end to end, with a fake extension over a
//! real socket on an ephemeral port (docs/BROWSER-PROTOCOL.md): pairing,
//! who may connect, commands and their deadlines, Stop, approvals answered
//! from the side panel and from the API, site modes, and the `browser_*`
//! tools through `mcp::Registry::call`. Nothing here touches `~/.claude`,
//! the Keychain or the user's ports.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::mcp::{self, Caller, Content, ToolCtx, ToolOutput};
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const ORIGIN: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";
const OTHER_ORIGIN: &str = "chrome-extension://ponmlkjihgfedcbaponmlkjihgfedcba";
const SESSION: &str = "11111111-2222-4333-8444-555555555555";
const OTHER_SESSION: &str = "66666666-7777-4888-9999-000000000000";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

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

async fn boot() -> (String, AppState) {
    let st = state();
    let base = serve(server::router(st.clone())).await;
    (base, st)
}

fn ws_url(base: &str) -> String {
    format!("{}/ws/browser", base.replacen("http", "ws", 1))
}

async fn try_connect(
    base: &str,
    headers: &[(&str, &str)],
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut req = ws_url(base).as_str().into_client_request().unwrap();
    for (k, v) in headers {
        req.headers_mut().insert(
            axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
            v.parse().unwrap(),
        );
    }
    tokio_tungstenite::connect_async(req)
        .await
        .map(|(ws, _)| ws)
}

async fn connect(base: &str) -> Ws {
    try_connect(base, &[("origin", ORIGIN)]).await.unwrap()
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// The next JSON frame, or `None` when the socket closed.
async fn next(ws: &mut Ws) -> Option<Value> {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("a frame within 10 s")?;
        match msg.ok()? {
            Message::Text(t) => return serde_json::from_str(&t).ok(),
            Message::Close(_) => return None,
            _ => continue,
        }
    }
}

/// The next frame of `kind`, skipping the others (`state` frames come often).
async fn next_of(ws: &mut Ws, kind: &str) -> Value {
    loop {
        let v = next(ws)
            .await
            .unwrap_or_else(|| panic!("closed before {kind}"));
        if v["type"] == kind {
            return v;
        }
    }
}

/// The socket ends (after any frames still queued).
async fn closes(ws: &mut Ws) -> Vec<Value> {
    let mut seen = Vec::new();
    while let Some(v) = next(ws).await {
        seen.push(v);
    }
    seen
}

async fn pair_code(base: &str, st: &AppState) -> String {
    let r = reqwest::Client::new()
        .post(format!("{base}/api/browser/pair"))
        .bearer_auth(&st.auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let v: Value = r.json().await.unwrap();
    v["display"].as_str().unwrap().to_string()
}

/// Pair and authenticate a fake extension: its socket (after `welcome` and
/// the first `state`), token and browser id.
async fn paired(base: &str, st: &AppState) -> (Ws, String, String) {
    let code = pair_code(base, st).await;
    let mut ws = connect(base).await;
    send(
        &mut ws,
        json!({"type": "pair", "id": "p1", "code": code, "name": "Chrome on Test",
               "browser": {"name": "Chrome", "version": "156.0"}, "extensionVersion": "0.1.0"}),
    )
    .await;
    let paired = next_of(&mut ws, "paired").await;
    let token = paired["token"].as_str().unwrap().to_string();
    let id = paired["browserId"].as_str().unwrap().to_string();
    send(
        &mut ws,
        json!({"type": "hello", "id": "h1", "token": token, "extensionVersion": "0.1.0",
               "browser": {"name": "Chrome", "version": "156.0"},
               "capabilities": {"debugger": true, "sidePanel": true, "tabGroups": true}}),
    )
    .await;
    let welcome = next_of(&mut ws, "welcome").await;
    assert_eq!(welcome["browserId"], id.as_str());
    let _ = next_of(&mut ws, "state").await;
    (ws, token, id)
}

fn ctx(st: &AppState, session: &str) -> ToolCtx {
    ToolCtx {
        state: st.clone(),
        caller: Caller {
            session_id: Some(session.into()),
            ..Default::default()
        },
    }
}

/// Run a tool in the background (the test plays the extension meanwhile).
fn call(
    st: &AppState,
    session: &str,
    tool: &str,
    args: Value,
) -> tokio::task::JoinHandle<ToolOutput> {
    let (ctx, tool) = (ctx(st, session), tool.to_string());
    tokio::spawn(async move {
        mcp::registry()
            .call(&tool, ctx, args)
            .await
            .expect("a registered tool")
    })
}

fn text_of(out: &ToolOutput) -> String {
    out.content
        .iter()
        .filter_map(|c| match c {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn set_mode(st: &AppState, origin: &str, mode: &str) {
    st.extension
        .settings
        .update(|s| {
            let key = mother_claude_lib::extension::sites::normalize_key(origin).unwrap();
            let m = mother_claude_lib::extension::sites::SiteMode::parse(mode).unwrap();
            if key == "*" {
                s.default_mode = m;
            } else {
                s.sites.insert(key, m);
            }
            Ok(())
        })
        .unwrap();
}

/// Answer the `lease.open` of a first call, then a `tabs.list` with one tab.
async fn open_lease_with_tab(ws: &mut Ws, url: &str) -> String {
    let open = next_of(ws, "lease.open").await;
    let lease = open["lease"].as_str().unwrap().to_string();
    assert_eq!(open["session"]["id"], SESSION);
    assert!(open["color"].is_string());
    send(
        ws,
        json!({"type": "lease.opened", "id": open["id"], "lease": lease, "groupId": 5}),
    )
    .await;
    let list = next_of(ws, "command").await;
    assert_eq!(list["action"], "tabs.list");
    assert_eq!(list["lease"], lease.as_str());
    send(
        ws,
        json!({"type": "result", "id": list["id"], "ok": true, "data": {"tabs": [
            {"tabId": 7, "url": url, "title": "Example", "active": true, "inLease": true}
        ]}}),
    )
    .await;
    lease
}

// ───────────────────────────────────────────────────────── pairing

#[tokio::test]
async fn a_browser_pairs_with_a_code_and_says_hello() {
    let (base, st) = boot().await;
    // A ping before `hello` is answered (keepalive while the user pairs).
    let mut ws = connect(&base).await;
    send(&mut ws, json!({"type": "ping", "id": "k1"})).await;
    assert_eq!(next(&mut ws).await.unwrap()["type"], "pong");
    // A wrong code, then the right one, once.
    send(
        &mut ws,
        json!({"type": "pair", "id": "p0", "code": "ZZZZ-ZZZZ"}),
    )
    .await;
    let e = next(&mut ws).await.unwrap();
    assert_eq!(
        (e["type"].as_str(), e["code"].as_str()),
        (Some("error"), Some("bad_code"))
    );
    assert_eq!(e["id"], "p0");
    let code = pair_code(&base, &st).await;
    assert_eq!(code.len(), 9, "ABCD-EFGH");
    send(
        &mut ws,
        json!({"type": "pair", "id": "p1", "code": code.to_lowercase(), "name": "Chrome on Test",
               "browser": {"name": "Chrome", "version": "156.0"}, "extensionVersion": "0.1.0"}),
    )
    .await;
    let paired = next(&mut ws).await.unwrap();
    assert_eq!(paired["type"], "paired");
    assert_eq!(paired["id"], "p1");
    let token = paired["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("mcx_"));
    let id = paired["browserId"].as_str().unwrap().to_string();
    assert!(id.starts_with("brw_"));
    send(&mut ws, json!({"type": "pair", "id": "p2", "code": code})).await;
    assert_eq!(next(&mut ws).await.unwrap()["code"], "expired_code");

    // hello → welcome, then state.
    send(
        &mut ws,
        json!({"type": "hello", "id": "h1", "token": token, "extensionVersion": "0.1.0",
               "browser": {"name": "Chrome", "version": "156.0"},
               "capabilities": {"debugger": true}}),
    )
    .await;
    let w = next(&mut ws).await.unwrap();
    assert_eq!(w["type"], "welcome");
    assert_eq!(w["id"], "h1");
    assert_eq!(w["browserId"], id.as_str());
    assert_eq!(w["protocol"], 1);
    assert!(w["appVersion"].is_string());
    assert_eq!(w["siteModes"]["*"], "off");
    assert_eq!(w["settings"]["screenshots"], true);
    let s = next(&mut ws).await.unwrap();
    assert_eq!(s["type"], "state");
    assert!(s["leases"].as_array().unwrap().is_empty());
    assert!(s["pending"].as_array().unwrap().is_empty());
    // Unknown frames are answered, the connection stays.
    send(&mut ws, json!({"type": "fromTheFuture", "id": "f1"})).await;
    let e = next(&mut ws).await.unwrap();
    assert_eq!(
        (e["code"].as_str(), e["id"].as_str()),
        (Some("unknown_frame"), Some("f1"))
    );
    send(&mut ws, json!({"type": "ping"})).await;
    assert_eq!(next_of(&mut ws, "pong").await["type"], "pong");

    // The page sees it connected; the token is never shown again.
    let overview: Value = reqwest::Client::new()
        .get(format!("{base}/api/browser"))
        .bearer_auth(&st.auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(overview["browsers"][0]["id"], id.as_str());
    assert_eq!(overview["browsers"][0]["connected"], true);
    assert_eq!(overview["browsers"][0]["origin"], ORIGIN);
    assert!(!overview.to_string().contains(&token));
    // The audit has the pairing, never the token.
    let audit = serde_json::to_string(&st.devices.audit.recent(50)).unwrap();
    assert!(audit.contains("browser.paired"));
    assert!(!audit.contains(&token));
}

// ───────────────────────────────────────────────────────── who may connect

#[tokio::test]
async fn only_the_paired_extension_on_this_machine_gets_in() {
    let (base, st) = boot().await;
    // The upgrade itself: an extension Origin, from this machine, no proxy.
    for headers in [
        vec![],
        vec![("origin", "http://localhost:6725")],
        vec![("origin", "https://evil.example")],
        vec![("origin", "moz-extension://abcdefghijklmnopabcdefghijklmnop")],
        vec![("origin", "chrome-extension://short")],
        vec![("origin", ORIGIN), ("x-forwarded-for", "203.0.113.9")],
        vec![("origin", ORIGIN), ("cf-connecting-ip", "198.51.100.4")],
        vec![("origin", ORIGIN), ("host", "abc.ngrok-free.app")],
    ] {
        assert!(
            try_connect(&base, &headers).await.is_err(),
            "{headers:?} must be refused"
        );
    }
    // The LAN listener does not serve it at all.
    let lan = serve(server::router_for(st.clone(), Ingress::Lan)).await;
    assert!(try_connect(&lan, &[("origin", ORIGIN)]).await.is_err());

    // Anything but pair/ping/hello first: unauthenticated, closed.
    let mut ws = connect(&base).await;
    send(
        &mut ws,
        json!({"type": "setSiteMode", "id": "x", "origin": "*", "mode": "act"}),
    )
    .await;
    let seen = closes(&mut ws).await;
    assert_eq!(seen[0]["code"], "unauthenticated");
    // A bad token: unauthenticated, closed.
    let mut ws = connect(&base).await;
    send(
        &mut ws,
        json!({"type": "hello", "id": "h", "token": "mcx_nope"}),
    )
    .await;
    assert_eq!(closes(&mut ws).await[0]["code"], "unauthenticated");

    // A real token from another extension id: unauthenticated (pinned).
    let (mut first, token, id) = paired(&base, &st).await;
    let mut other = try_connect(&base, &[("origin", OTHER_ORIGIN)])
        .await
        .unwrap();
    send(
        &mut other,
        json!({"type": "hello", "id": "h", "token": token}),
    )
    .await;
    assert_eq!(closes(&mut other).await[0]["code"], "unauthenticated");

    // A second connection of the same browser replaces the first.
    let mut second = connect(&base).await;
    send(
        &mut second,
        json!({"type": "hello", "id": "h2", "token": token}),
    )
    .await;
    assert_eq!(
        next_of(&mut second, "welcome").await["browserId"],
        id.as_str()
    );
    let seen = closes(&mut first).await;
    assert!(seen.iter().any(|f| f["code"] == "replaced"), "{seen:?}");

    // Revoked while connected: told `revoked`, closed; later hellos too.
    let r = reqwest::Client::new()
        .delete(format!("{base}/api/browser/browsers/{id}"))
        .bearer_auth(&st.auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let seen = closes(&mut second).await;
    assert!(seen.iter().any(|f| f["code"] == "revoked"), "{seen:?}");
    let mut again = connect(&base).await;
    send(
        &mut again,
        json!({"type": "hello", "id": "h3", "token": token}),
    )
    .await;
    assert_eq!(closes(&mut again).await[0]["code"], "revoked");
}

#[tokio::test]
async fn pairing_and_revoking_need_the_desktop() {
    let (base, st) = boot().await;
    let c = reqwest::Client::new();
    // The local-script token never pairs or revokes.
    let r = c
        .post(format!("{base}/api/browser/pair"))
        .bearer_auth(&st.auth.script_token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // A paired phone without the dangerous grant reads, but cannot pair or
    // raise a sensitive site to act; it may set an ordinary site's mode.
    let (_info, phone) = st.devices.issue(
        "Phone",
        "ios",
        None,
        Ingress::Lan,
        "192.168.1.50".parse().unwrap(),
    );
    let r = c
        .get(format!("{base}/api/browser"))
        .bearer_auth(&phone)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = c
        .post(format!("{base}/api/browser/pair"))
        .bearer_auth(&phone)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    for (origin, mode, want) in [
        ("https://www.paypal.com", "act", StatusCode::FORBIDDEN),
        ("*", "act", StatusCode::FORBIDDEN),
        ("https://*.example.com", "act", StatusCode::FORBIDDEN),
        ("https://github.com", "act", StatusCode::OK),
        ("https://www.paypal.com", "read", StatusCode::OK),
    ] {
        let r = c
            .put(format!("{base}/api/browser/sites"))
            .bearer_auth(&phone)
            .json(&json!({"origin": origin, "mode": mode}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), want, "{origin} {mode}");
    }
    // The desktop may.
    let r = c
        .put(format!("{base}/api/browser/sites"))
        .bearer_auth(&st.auth.token)
        .json(&json!({"origin": "https://www.paypal.com", "mode": "act"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    let paypal = v["sites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["origin"] == "https://www.paypal.com")
        .unwrap();
    assert_eq!(paypal["mode"], "act");
    assert_eq!(paypal["sensitive"], "payment");
    // Standing grants need the right; removing one does not.
    let r = c
        .post(format!("{base}/api/browser/grants"))
        .bearer_auth(&phone)
        .json(&json!({"action": "eval", "origin": "http://localhost:3000"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = c
        .post(format!("{base}/api/browser/grants"))
        .bearer_auth(&st.auth.token)
        .json(&json!({"action": "eval", "origin": "http://localhost:3000/x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let gid = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = c
        .delete(format!("{base}/api/browser/grants/{gid}"))
        .bearer_auth(&phone)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let audit = serde_json::to_string(&st.devices.audit.recent(100)).unwrap();
    assert!(audit.contains("browser.site_mode") && audit.contains("browser.grant.add"));
}

// ───────────────────────────────────────────────────────── commands

#[tokio::test]
async fn a_snapshot_round_trips_and_is_redacted() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    set_mode(&st, "https://example.test", "read").await;
    let task = call(
        &st,
        SESSION,
        "browser_snapshot",
        json!({"interactiveOnly": true}),
    );
    let lease = open_lease_with_tab(&mut ws, "https://example.test/a?token=zzz").await;
    let cmd = next_of(&mut ws, "command").await;
    assert_eq!(cmd["action"], "snapshot");
    assert_eq!(cmd["lease"], lease.as_str());
    assert_eq!(cmd["params"]["interactiveOnly"], true);
    assert_eq!(cmd["params"]["maxNodes"], 1500);
    assert_eq!(cmd["timeoutMs"], 30000);
    send(
        &mut ws,
        json!({"type": "result", "id": cmd["id"], "ok": true, "data": {
            "snapshot": "- main\n  - link \"Home\" [ref=e1]\n  - textbox \"Password\" [ref=e2] value=\"hunter2\"\n  - textbox \"Key\" [ref=e3] value=\"ghp_abcdefghijklmnopqrstuvwxyz0123456789\"",
            "url": "https://example.test/a?token=zzz", "title": "Example", "truncated": false, "nodes": 3
        }}),
    )
    .await;
    let out = task.await.unwrap();
    let text = text_of(&out);
    assert!(!out.is_error, "{text}");
    assert!(text.contains("[ref=e1]"));
    assert!(!text.contains("hunter2"));
    assert!(!text.contains("ghp_"));
    assert!(text.contains("«redacted»"));
    // The activity and the audit name the action, never values or the query.
    let entries = st.extension.activity.recent(Some(SESSION), 10);
    let snap = entries
        .iter()
        .find(|e| e.tool == "browser_snapshot")
        .unwrap();
    assert_eq!(snap.outcome, "ok");
    assert_eq!(snap.origin.as_deref(), Some("https://example.test"));
    assert_eq!(snap.page.as_deref(), Some("https://example.test/a"));
    let audit = serde_json::to_string(&st.devices.audit.recent(100)).unwrap();
    assert!(audit.contains("browser.action"));
    assert!(!audit.contains("hunter2") && !audit.contains("zzz"));
    // The lease shows in the state frame and on the page.
    let browser = st.extension.hub.connected()[0].browser.clone();
    let s = mother_claude_lib::extension::state_frame(&st, &browser);
    assert_eq!(s["leases"][0]["lease"], lease.as_str());
    assert_eq!(s["leases"][0]["groupId"], 5);
    let overview = mother_claude_lib::extension::routes::overview(&st);
    assert_eq!(overview["leases"][0]["session"], SESSION);
}

#[tokio::test]
async fn site_modes_are_enforced_before_anything_is_sent() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    // Off by default: refused after learning the tab, no snapshot sent.
    let task = call(&st, SESSION, "browser_snapshot", json!({}));
    open_lease_with_tab(&mut ws, "https://blocked.test/").await;
    let out = task.await.unwrap();
    let text = text_of(&out);
    assert!(out.is_error && text.contains("site_off") && text.contains("https://blocked.test"));
    // Read-only: reading works, acting does not.
    set_mode(&st, "https://blocked.test", "read").await;
    let out = call(&st, SESSION, "browser_click", json!({"ref": "e1"}))
        .await
        .unwrap();
    assert!(
        text_of(&out).contains("site_read_only"),
        "{}",
        text_of(&out)
    );
    // Navigating to an off site is refused before it is sent.
    let out = call(
        &st,
        SESSION,
        "browser_navigate",
        json!({"url": "https://elsewhere.test/"}),
    )
    .await
    .unwrap();
    assert!(text_of(&out).contains("site_off"));
    let out = call(
        &st,
        SESSION,
        "browser_navigate",
        json!({"url": "javascript:alert(1)"}),
    )
    .await
    .unwrap();
    assert!(text_of(&out).contains("invalid_arguments"));
    // Nothing reached the extension in the meantime.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let pending = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
    if let Ok(Some(Ok(Message::Text(t)))) = pending {
        let v: Value = serde_json::from_str(&t).unwrap();
        assert_ne!(v["type"], "command", "{v}");
    }
}

#[tokio::test]
async fn a_command_without_a_result_times_out() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    set_mode(&st, "https://example.test", "read").await;
    let started = std::time::Instant::now();
    let task = call(
        &st,
        SESSION,
        "browser_navigate",
        json!({"url": "https://example.test/slow", "timeoutMs": 1000}),
    );
    let open = next_of(&mut ws, "lease.open").await;
    send(
        &mut ws,
        json!({"type": "lease.opened", "id": open["id"], "lease": open["lease"], "groupId": 1}),
    )
    .await;
    let cmd = next_of(&mut ws, "command").await;
    assert_eq!(cmd["action"], "navigate");
    assert_eq!(cmd["timeoutMs"], 1000);
    // …and the extension never answers.
    let out = task.await.unwrap();
    assert!(text_of(&out).contains("(timeout)"), "{}", text_of(&out));
    assert!(started.elapsed() < Duration::from_secs(6));
    // A late result is ignored.
    send(
        &mut ws,
        json!({"type": "result", "id": cmd["id"], "ok": true, "data": {}}),
    )
    .await;
    send(&mut ws, json!({"type": "ping"})).await;
    assert_eq!(next_of(&mut ws, "pong").await["type"], "pong");
}

#[tokio::test]
async fn stop_fails_the_command_and_refuses_new_ones_until_resumed() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    set_mode(&st, "https://example.test", "act").await;
    let task = call(&st, SESSION, "browser_click", json!({"ref": "e1"}));
    let lease = open_lease_with_tab(&mut ws, "https://example.test/").await;
    let cmd = next_of(&mut ws, "command").await;
    assert_eq!(cmd["action"], "click");
    // The user presses Stop instead of the click finishing.
    send(
        &mut ws,
        json!({"type": "event", "event": "lease.revoked", "lease": lease, "by": "user"}),
    )
    .await;
    let out = task.await.unwrap();
    assert!(text_of(&out).contains("(stopped)"), "{}", text_of(&out));
    // New calls are refused at once, nothing is sent.
    let out = call(&st, SESSION, "browser_snapshot", json!({}))
        .await
        .unwrap();
    assert!(text_of(&out).contains("(stopped)") && text_of(&out).contains("resume"));
    // Another session in this browser now needs an approval to act.
    assert!(st
        .extension
        .leases
        .after_stop(&st.extension.hub.connected()[0].browser, OTHER_SESSION));
    // The desktop resumes it.
    let r = reqwest::Client::new()
        .post(format!("{base}/api/browser/sessions/{SESSION}/resume"))
        .bearer_auth(&st.auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let task = call(&st, SESSION, "browser_tabs", json!({}));
    let open = next_of(&mut ws, "lease.open").await;
    send(
        &mut ws,
        json!({"type": "lease.opened", "id": open["id"], "lease": open["lease"], "groupId": 2}),
    )
    .await;
    let list = next_of(&mut ws, "command").await;
    send(
        &mut ws,
        json!({"type": "result", "id": list["id"], "ok": true, "data": {"tabs": [
            {"tabId": 9, "url": "https://example.test/x", "title": "X", "active": true, "inLease": true},
            {"tabId": 10, "url": "https://secret.test/inbox", "title": "Inbox (3)", "active": false, "inLease": true}
        ]}}),
    )
    .await;
    let out = task.await.unwrap();
    let text = text_of(&out);
    assert!(!out.is_error, "{text}");
    assert!(text.contains("https://example.test/x"));
    // A tab on a site the session may not see: no title, no path.
    assert!(
        !text.contains("Inbox") && !text.contains("/inbox"),
        "{text}"
    );
}

// ───────────────────────────────────────────────────────── approvals

/// Wait until a browser approval is pending; its id.
async fn pending_approval(st: &AppState) -> String {
    for _ in 0..200 {
        if let Some(a) = st.extension.approvals_for(None).first() {
            return a.id.clone();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no approval appeared");
}

#[tokio::test]
async fn eval_waits_for_the_side_panel() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    set_mode(&st, "https://example.test", "act").await;
    let task = call(
        &st,
        SESSION,
        "browser_eval",
        json!({"expression": "document.title"}),
    );
    open_lease_with_tab(&mut ws, "https://example.test/").await;
    let id = pending_approval(&st).await;
    // The card: a browser item, dangerous, saying why; the side panel sees it.
    let item = st.pending.get(&id).unwrap();
    assert_eq!(serde_json::to_value(item.source).unwrap(), json!("browser"));
    assert!(item.dangerous);
    assert_eq!(item.tool_name.as_deref(), Some("browser_eval"));
    assert_eq!(item.input.as_ref().unwrap()["expression"], "document.title");
    assert_eq!(item.suggestions.len(), 1, "always allow scripts here");
    let s = loop {
        let s = next_of(&mut ws, "state").await;
        if !s["pending"].as_array().unwrap().is_empty() {
            break s;
        }
    };
    assert_eq!(s["pending"][0]["id"], id.as_str());
    assert_eq!(s["pending"][0]["session"], SESSION);
    assert_eq!(s["pending"][0]["dangerous"], true);
    // The side panel allows it; a second answer is too late.
    send(
        &mut ws,
        json!({"type": "answer", "id": "a1", "pending": id, "allow": true}),
    )
    .await;
    let ok = next_of(&mut ws, "ok").await;
    assert_eq!(ok["id"], "a1");
    let cmd = next_of(&mut ws, "command").await;
    assert_eq!(cmd["action"], "eval");
    send(
        &mut ws,
        json!({"type": "result", "id": cmd["id"], "ok": true, "data": {"value": {"title": "Example", "token": "abc"}}}),
    )
    .await;
    let out = task.await.unwrap();
    let text = text_of(&out);
    assert!(!out.is_error, "{text}");
    assert!(text.contains("Example") && !text.contains("abc"), "{text}");
    send(
        &mut ws,
        json!({"type": "answer", "id": "a2", "pending": id, "allow": true}),
    )
    .await;
    let e = next_of(&mut ws, "error").await;
    assert_eq!(e["code"], "already_answered");
    let r = st.pending.resolution(&id).unwrap();
    assert!(r.text.unwrap_or_default().contains("side panel"));
    let entry = st
        .extension
        .activity
        .recent(Some(SESSION), 10)
        .into_iter()
        .find(|e| e.tool == "browser_eval")
        .unwrap();
    assert!(entry.approved);
}

#[tokio::test]
async fn a_caller_that_gives_up_leaves_no_approval_behind() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    set_mode(&st, "https://example.test", "act").await;
    let task = call(&st, SESSION, "browser_eval", json!({"expression": "1"}));
    open_lease_with_tab(&mut ws, "https://example.test/").await;
    let id = pending_approval(&st).await;
    // The MCP client went away: its call is dropped mid-approval.
    task.abort();
    let _ = task.await;
    for _ in 0..100 {
        if st.extension.approvals_for(None).is_empty() && st.pending.get(&id).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(st.extension.approvals_for(None).is_empty(), "untracked");
    assert!(st.pending.get(&id).is_none(), "withdrawn");
    // The side panel hears it is gone.
    let s = loop {
        let s = next_of(&mut ws, "state").await;
        if s["pending"].as_array().unwrap().is_empty() {
            break s;
        }
    };
    assert!(s["leases"].as_array().unwrap().len() == 1);
}

#[tokio::test]
async fn approvals_are_answered_from_the_api_too() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    set_mode(&st, "https://example.test", "act").await;
    let c = reqwest::Client::new();
    let respond = |token: String, id: String, decision: &'static str| {
        let c = c.clone();
        let base = base.clone();
        async move {
            c.post(format!("{base}/api/sessions/{SESSION}/respond"))
                .bearer_auth(token)
                .json(&json!({"requestId": id, "decision": decision}))
                .send()
                .await
                .unwrap()
        }
    };
    // A phone without the dangerous grant cannot allow it, only deny.
    let (_info, phone) = st.devices.issue(
        "Phone",
        "ios",
        None,
        Ingress::Lan,
        "192.168.1.50".parse().unwrap(),
    );
    let task = call(&st, SESSION, "browser_eval", json!({"expression": "1+1"}));
    open_lease_with_tab(&mut ws, "https://example.test/").await;
    let id = pending_approval(&st).await;
    let r = respond(phone.clone(), id.clone(), "allow").await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = respond(st.auth.token.clone(), id.clone(), "deny").await;
    assert_eq!(r.status(), StatusCode::OK);
    let out = task.await.unwrap();
    assert!(text_of(&out).contains("(denied)"), "{}", text_of(&out));
    assert!(st.extension.approvals_for(None).is_empty());
    // Settled approvals are audited, whoever answered.
    let audit = st.devices.audit.recent(100);
    let line = audit
        .iter()
        .find(|e| e.action == "browser.approval")
        .expect("an audit line");
    let detail = line.detail.as_ref().unwrap();
    assert_eq!(detail["outcome"], "denied");
    assert_eq!(detail["session"], SESSION);
    assert_eq!(detail["by"]["kind"], "desktop");
    assert!(!detail.to_string().contains("1+1"), "never the script");

    // The desktop allows the next one, with "always allow": the third runs
    // without asking.
    let task = call(&st, SESSION, "browser_eval", json!({"expression": "2+2"}));
    let id = pending_approval(&st).await;
    let r = c
        .post(format!("{base}/api/sessions/{SESSION}/respond"))
        .bearer_auth(&st.auth.token)
        .json(&json!({"requestId": id, "decision": "allow", "suggestion": 0}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let cmd = next_of(&mut ws, "command").await;
    assert_eq!(cmd["action"], "eval");
    send(
        &mut ws,
        json!({"type": "result", "id": cmd["id"], "ok": true, "data": {"value": 4}}),
    )
    .await;
    assert!(!task.await.unwrap().is_error);
    let settings = st.extension.settings.get();
    assert_eq!(settings.grants.len(), 1);
    assert_eq!(settings.grants[0].origin, "https://example.test");
    let task = call(&st, SESSION, "browser_eval", json!({"expression": "3+3"}));
    let cmd = next_of(&mut ws, "command").await;
    assert_eq!(cmd["action"], "eval", "no approval this time");
    send(
        &mut ws,
        json!({"type": "result", "id": cmd["id"], "ok": true, "data": {"value": 6}}),
    )
    .await;
    assert!(!task.await.unwrap().is_error);
    assert!(st.extension.approvals_for(None).is_empty());
}

#[tokio::test]
async fn secret_fields_are_refused_and_their_forms_need_approval() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    set_mode(&st, "https://login.test", "act").await;
    let task = call(&st, SESSION, "browser_snapshot", json!({}));
    open_lease_with_tab(&mut ws, "https://login.test/").await;
    let cmd = next_of(&mut ws, "command").await;
    send(
        &mut ws,
        json!({"type": "result", "id": cmd["id"], "ok": true, "data": {
            "snapshot": "- main\n  - textbox \"Username\" [ref=e1] value=\"\"\n  - textbox \"Password\" [ref=e2] value=«redacted»\n  - button \"Sign in\" [ref=e3]",
            "url": "https://login.test/", "title": "Sign in"
        }}),
    )
    .await;
    assert!(!task.await.unwrap().is_error);
    // Typing into the password field: refused outright, nothing sent.
    let out = call(
        &st,
        SESSION,
        "browser_type",
        json!({"ref": "e2", "text": "hunter2"}),
    )
    .await
    .unwrap();
    assert!(
        text_of(&out).contains("(secret_field)"),
        "{}",
        text_of(&out)
    );
    // Filling the username of that form asks first; the extension is told
    // to refuse secret fields too.
    let task = call(
        &st,
        SESSION,
        "browser_fill_form",
        json!({"fields": [{"ref": "e1", "value": "marcus"}], "submit": true}),
    );
    let id = pending_approval(&st).await;
    let item = st.pending.get(&id).unwrap();
    assert!(item.title.contains("Fill a form"), "{}", item.title);
    assert!(item.detail.unwrap().contains("password"));
    send(
        &mut ws,
        json!({"type": "answer", "id": "a", "pending": id, "allow": true}),
    )
    .await;
    let cmd = next_of(&mut ws, "command").await;
    assert_eq!(cmd["action"], "fill");
    assert_eq!(cmd["params"]["refuseSecret"], true);
    send(
        &mut ws,
        json!({"type": "result", "id": cmd["id"], "ok": true, "data": {"filled": 1}}),
    )
    .await;
    assert!(!task.await.unwrap().is_error);
    // Nothing typed reached the audit.
    let audit = serde_json::to_string(&st.devices.audit.recent(100)).unwrap();
    assert!(!audit.contains("marcus") && !audit.contains("hunter2"));
}

#[tokio::test]
async fn the_side_panel_sets_site_modes_within_limits() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    send(
        &mut ws,
        json!({"type": "setSiteMode", "id": "s1", "origin": "https://GitHub.com/issues", "mode": "read"}),
    )
    .await;
    assert_eq!(next_of(&mut ws, "ok").await["id"], "s1");
    let s = next_of(&mut ws, "state").await;
    assert_eq!(s["siteModes"]["https://github.com"], "read");
    for (origin, id) in [
        ("https://www.paypal.com", "s2"),
        ("*", "s3"),
        ("https://*.github.com", "s4"),
    ] {
        send(
            &mut ws,
            json!({"type": "setSiteMode", "id": id, "origin": origin, "mode": "act"}),
        )
        .await;
        let e = next_of(&mut ws, "error").await;
        assert_eq!(
            (e["id"].as_str(), e["code"].as_str()),
            (Some(id), Some("needs_desktop"))
        );
    }
    send(
        &mut ws,
        json!({"type": "setSiteMode", "id": "s5", "origin": "not a site", "mode": "read"}),
    )
    .await;
    assert_eq!(next_of(&mut ws, "error").await["code"], "invalid");
    // `null` removes the site's own mode.
    send(
        &mut ws,
        json!({"type": "setSiteMode", "id": "s6", "origin": "https://github.com", "mode": null}),
    )
    .await;
    assert_eq!(next_of(&mut ws, "ok").await["id"], "s6");
    assert!(st.extension.settings.get().sites.is_empty());
    let audit = serde_json::to_string(&st.devices.audit.recent(50)).unwrap();
    assert!(audit.contains("browser.site_mode") && audit.contains("\"principal\":\"extension\""));
}

#[tokio::test]
async fn oversized_frames_are_refused_and_fail_their_request() {
    let (base, st) = boot().await;
    let (mut ws, _, _) = paired(&base, &st).await;
    set_mode(&st, "https://example.test", "read").await;
    let task = call(&st, SESSION, "browser_screenshot", json!({}));
    open_lease_with_tab(&mut ws, "https://example.test/").await;
    let cmd = next_of(&mut ws, "command").await;
    assert_eq!(cmd["action"], "screenshot");
    let big = format!(
        r#"{{"type":"result","id":{},"ok":true,"data":{{"data":"{}","mimeType":"image/png"}}}}"#,
        cmd["id"],
        "A".repeat(8 * 1024 * 1024 + 10)
    );
    ws.send(Message::Text(big.into())).await.unwrap();
    let e = next_of(&mut ws, "error").await;
    assert_eq!(e["code"], "frame_too_large");
    let out = task.await.unwrap();
    assert!(
        text_of(&out).contains("frame_too_large"),
        "{}",
        text_of(&out)
    );
}

// ───────────────────────────────────────────────────────── dashboards

/// The next dashboard frame (`/ws`) `pick` accepts.
async fn dash_frame(ws: &mut Ws, pick: impl Fn(&Value) -> bool) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("a dashboard frame within 10 s")
            .expect("open")
            .expect("a frame");
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t).unwrap();
            if pick(&v) {
                return v;
            }
        }
    }
}

#[tokio::test]
async fn dashboards_hear_of_changes_over_the_realtime_socket() {
    let (base, st) = boot().await;
    let url = format!(
        "{}/ws?token={}",
        base.replacen("http", "ws", 1),
        st.auth.token
    );
    let mut dash = tokio_tungstenite::connect_async(url).await.unwrap().0;
    dash_frame(&mut dash, |v| v["t"] == "hello").await;
    let is = |what: &'static str| move |v: &Value| v["t"] == "browser" && v["what"] == what;

    // A site mode set on the page.
    let r = reqwest::Client::new()
        .put(format!("{base}/api/browser/sites"))
        .bearer_auth(&st.auth.token)
        .json(&json!({"origin": "https://example.test", "mode": "act"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    dash_frame(&mut dash, is("sites")).await;

    // A browser connects; an approval is raised, then answered.
    let (mut ws, _, _) = paired(&base, &st).await;
    dash_frame(&mut dash, is("browsers")).await;
    let task = call(&st, SESSION, "browser_eval", json!({"expression": "1"}));
    open_lease_with_tab(&mut ws, "https://example.test/").await;
    dash_frame(&mut dash, is("leases")).await;
    let id = pending_approval(&st).await;
    dash_frame(&mut dash, is("approvals")).await;
    let r = reqwest::Client::new()
        .post(format!("{base}/api/sessions/{SESSION}/respond"))
        .bearer_auth(&st.auth.token)
        .json(&json!({"requestId": id, "decision": "deny"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(text_of(&task.await.unwrap()).contains("(denied)"));
    dash_frame(&mut dash, is("approvals")).await;
    // Every action shows in the activity, soon after.
    dash_frame(&mut dash, is("activity")).await;
}
