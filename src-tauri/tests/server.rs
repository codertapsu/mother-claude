//! The foundation's server contract over real HTTP/WebSocket: who counts as
//! local, which credential works where, pending decisions end to end, and the
//! WebSocket v2 handshake. Nothing here touches `~/.claude` or spawns `claude`.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
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

async fn boot() -> (String, AppState) {
    let st = state();
    let base = serve(server::router(st.clone())).await;
    (base, st)
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test]
async fn credentials_are_accepted_only_where_they_belong() {
    let (base, st) = boot().await;
    let master = st.auth.token.clone();
    let hook = st.auth.hook_token.clone();
    let c = http();

    // Public health; everything else needs a credential, with the envelope.
    let r = c.get(format!("{base}/api/health")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = c.get(format!("{base}/api/sessions")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "unauthorized"
    );

    // The master token works for a genuinely local client…
    let v: Value = c
        .get(format!("{base}/api/sessions"))
        .bearer_auth(&master)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(v["sessions"].as_array().unwrap().is_empty());
    let me: Value = c
        .get(format!("{base}/api/me"))
        .bearer_auth(&master)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["principal"], "desktop");
    assert_eq!(me["ingress"], "loopback");
    assert_eq!(me["local"], true);
    // …also as ?token= (local scripts, bridge SSE)…
    let r = c
        .get(format!("{base}/api/sessions?token={master}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    // …but not through a tunnel (ngrok http 6725: loopback peer, public Host,
    // forwarding headers), header or query.
    for (name, value) in [
        ("x-forwarded-for", "203.0.113.9"),
        ("tailscale-funnel-request", "?1"),
        ("cf-connecting-ip", "198.51.100.4"),
        ("ngrok-trace-id", "abc"),
    ] {
        let r = c
            .get(format!("{base}/api/sessions"))
            .bearer_auth(&master)
            .header(name, value)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{name}");
    }
    let r = c
        .get(format!("{base}/api/sessions?token={master}"))
        .header("host", "abc.ngrok-free.app")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // DNS rebinding: a foreign hostname pointing at 127.0.0.1.
    let r = c
        .get(format!("{base}/api/sessions"))
        .bearer_auth(&master)
        .header("host", "evil.example:6725")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // A web page on another origin cannot ride the local desktop token.
    let r = c
        .get(format!("{base}/api/sessions"))
        .bearer_auth(&master)
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // The SPA's own skip-warning header is not a proxy fingerprint.
    let r = c
        .get(format!("{base}/api/sessions"))
        .bearer_auth(&master)
        .header("ngrok-skip-browser-warning", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    // The hook token: /hooks only.
    let r = c
        .post(format!("{base}/hooks/event"))
        .bearer_auth(&hook)
        .json(&json!({ "hook_event_name": "PreToolUse", "session_id": "s-hook" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({}));
    assert!(st.hooks.last_event_at("s-hook").is_some());
    let r = c
        .get(format!("{base}/api/sessions"))
        .bearer_auth(&hook)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = c
        .post(format!("{base}/hooks/event"))
        .bearer_auth(&master)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = c
        .post(format!("{base}/hooks/event"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

    // Desktop-only routes refuse… nobody else can be desktop here, so check the
    // guard answers (the legacy pairing view never carries a token any more).
    let pairing: Value = c
        .get(format!("{base}/api/pairing"))
        .bearer_auth(&master)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(pairing["svg"].as_str().unwrap().contains("<svg"));
    assert!(!pairing.to_string().contains(&master));

    // Unregistered owners answer 501 with the envelope.
    let r = c
        .post(format!("{base}/api/pair/exchange"))
        .json(&json!({ "code": "X" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "not_implemented"
    );

    // The retired settings.json hook installer is gone.
    let r = c
        .post(format!("{base}/api/hooks/install"))
        .bearer_auth(&master)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::GONE);
}

#[tokio::test]
async fn failed_credentials_are_rate_limited() {
    let (base, _st) = boot().await;
    let c = http();
    let mut last = StatusCode::OK;
    for _ in 0..25 {
        last = c
            .get(format!("{base}/api/sessions"))
            .bearer_auth("mcd_not-a-device")
            .send()
            .await
            .unwrap()
            .status();
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

/// The sidecar raises a prompt and blocks; the dashboard answers through
/// `/respond`; the sidecar unblocks with the decision. A second answer is a
/// conflict; the old aliases still work.
#[tokio::test]
async fn pending_round_trip_through_respond_and_aliases() {
    let (base, st) = boot().await;
    let master = st.auth.token.clone();
    let sidecar = st.auth.sidecar_token.clone();
    let c = http();

    // Only the sidecar credential may raise prompts.
    let r = c
        .post(format!("{base}/api/sessions/s1/permission-request"))
        .bearer_auth(&master)
        .json(&json!({ "kind": "permission", "tool": "Bash" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = c
        .get(format!("{base}/api/sessions"))
        .bearer_auth(&sidecar)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    let raise = |body: Value| {
        let (c, base, sidecar) = (c.clone(), base.clone(), sidecar.clone());
        tokio::spawn(async move {
            c.post(format!("{base}/api/sessions/s1/permission-request"))
                .bearer_auth(&sidecar)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        })
    };

    async fn inbox(c: &reqwest::Client, base: &str, token: &str, n: usize) -> Vec<Value> {
        for _ in 0..100 {
            let v: Value = c
                .get(format!("{base}/api/inbox"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let items = v["items"].as_array().cloned().unwrap_or_default();
            if items.len() >= n {
                return items;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("inbox never reached {n} items");
    }

    // Two concurrent prompts in one session: both stay visible.
    let first = raise(json!({ "kind": "permission", "tool": "Bash", "requestId": "chosen" }));
    let second =
        raise(json!({ "kind": "question", "prompt": "Which DB?", "options": ["pg", "sqlite"] }));
    let items = inbox(&c, &base, &master, 2).await;
    assert!(items
        .iter()
        .all(|i| i["sessionId"] == "s1" && i["answerable"] == true));
    assert!(
        items.iter().all(|i| i["id"] != "chosen"),
        "ids are server-generated"
    );
    let perm = items.iter().find(|i| i["kind"] == "permission").unwrap();
    let question = items.iter().find(|i| i["kind"] == "question").unwrap();
    assert_eq!(perm["source"], "sidecar");

    // Wrong session → 404; right session → ok.
    let r = c
        .post(format!("{base}/api/sessions/other/respond"))
        .bearer_auth(&master)
        .json(&json!({ "requestId": perm["id"], "decision": "allow" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = c
        .post(format!("{base}/api/sessions/s1/respond"))
        .bearer_auth(&master)
        .json(&json!({ "requestId": perm["id"], "decision": "allow" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(first.await.unwrap()["behavior"], "allow");

    // Late second answer → 409 already_decided.
    let r = c
        .post(format!("{base}/api/sessions/s1/respond"))
        .bearer_auth(&master)
        .json(&json!({ "requestId": perm["id"], "decision": "deny" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "already_decided"
    );

    // Legacy /answer resolves the question with the typed answer.
    let r = c
        .post(format!("{base}/api/sessions/s1/answer"))
        .bearer_auth(&master)
        .json(&json!({ "answer": "sqlite", "requestId": question["id"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let answered = second.await.unwrap();
    assert_eq!(answered["behavior"], "answer");
    assert_eq!(answered["answer"], "sqlite");

    // Legacy /permission with no requestId takes the oldest item.
    let third = raise(json!({ "kind": "permission", "tool": "Edit" }));
    inbox(&c, &base, &master, 1).await;
    let r = c
        .post(format!("{base}/api/sessions/s1/permission"))
        .bearer_auth(&master)
        .json(&json!({ "decision": "deny" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(third.await.unwrap()["behavior"], "deny");
    assert!(st.pending.is_empty());
}

async fn ws_connect(
    base: &str,
    query: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("{}/ws{query}", base.replacen("http", "ws", 1));
    tokio_tungstenite::connect_async(url).await.unwrap().0
}

async fn next_json<S>(ws: &mut S) -> Option<Value>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        match msg {
            Message::Text(t) => {
                let v: Value = serde_json::from_str(&t).ok()?;
                if v["t"] == "ping" {
                    continue;
                }
                return Some(v);
            }
            Message::Close(frame) => {
                return Some(json!({ "t": "close", "code": frame.map(|f| u16::from(f.code)) }))
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn websocket_auth_and_pending_diffs() {
    let (base, st) = boot().await;
    let master = st.auth.token.clone();

    // A wrong first frame closes with 4401.
    let mut ws = ws_connect(&base, "").await;
    ws.send(Message::Text(
        json!({ "t": "auth", "token": "nope" }).to_string().into(),
    ))
    .await
    .unwrap();
    let closed = next_json(&mut ws).await.unwrap();
    assert_eq!(closed["t"], "close");
    assert_eq!(closed["code"], 4401);

    // A local client may still use ?token=.
    let mut ws = ws_connect(&base, &format!("?token={master}")).await;
    let hello = next_json(&mut ws).await.unwrap();
    assert_eq!(hello["t"], "hello");
    assert_eq!(hello["me"]["principal"], "desktop");
    let full = next_json(&mut ws).await.unwrap();
    assert_eq!(full["full"], true);

    // An owned session appears, then its pending prompt arrives as a diff,
    // then vanishes when withdrawn.
    st.mark_owned("own-1", "/tmp/proj", chrono::Utc::now().timestamp_millis())
        .await;
    let v = next_json(&mut ws).await.unwrap();
    assert_eq!(v["t"], "sessions");
    assert_eq!(v["upsert"][0]["id"], "own-1");
    assert_eq!(v["upsert"][0]["owned"], true);
    assert_eq!(v["upsert"][0]["control"]["channel"], "owned");

    let item = mother_claude_lib::pending::PendingItem {
        session_id: "own-1".into(),
        title: "Run tests?".into(),
        source: mother_claude_lib::claude::model::PendingSource::Owned,
        ..Default::default()
    };
    let (guard, _rx) = mother_claude_lib::pending::raise(&st, item).await;
    let v = next_json(&mut ws).await.unwrap();
    assert_eq!(v["upsert"][0]["status"], "waiting");
    assert_eq!(v["upsert"][0]["pending"][0]["title"], "Run tests?");
    drop(guard);
    // Everyone learns why it went away (`pendingResolved`), then the diff.
    let v = next_json(&mut ws).await.unwrap();
    assert_eq!(v["t"], "pendingResolved");
    assert_eq!(v["session"], "own-1");
    assert_eq!(v["outcome"], "cancelled");
    let v = next_json(&mut ws).await.unwrap();
    assert!(v["upsert"][0]["pending"].as_array().unwrap().is_empty());

    // Session-scoped frames reach subscribers only. The desktop-only
    // `devices` frame published right after proves (the bus is ordered) that
    // the unsubscribed stream frame was seen and dropped.
    mother_claude_lib::server::ws::publish_stream(
        &st,
        "own-1",
        mother_claude_lib::claude::model::StreamFrame {
            delta: Some("not subscribed".into()),
            ..Default::default()
        },
    );
    mother_claude_lib::server::ws::publish_devices(&st);
    assert_eq!(next_json(&mut ws).await.unwrap()["t"], "devices");
    ws.send(Message::Text(
        json!({ "t": "sub", "session": "own-1" }).to_string().into(),
    ))
    .await
    .unwrap();
    // Round-trip a ping so the sub is surely processed before publishing.
    ws.send(Message::Text(r#"{"t":"ping"}"#.into()))
        .await
        .unwrap();
    assert_eq!(next_json(&mut ws).await.unwrap()["t"], "pong");
    mother_claude_lib::server::ws::publish_stream(
        &st,
        "own-1",
        mother_claude_lib::claude::model::StreamFrame {
            delta: Some("hello".into()),
            ..Default::default()
        },
    );
    let v = next_json(&mut ws).await.unwrap();
    assert_eq!(v["t"], "stream");
    assert_eq!(v["delta"], "hello");

    // WebSocket upgrades from a foreign origin are refused.
    let url = format!("{}/ws", base.replacen("http", "ws", 1));
    let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        url.as_str(),
    )
    .unwrap();
    req.headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(req).await.is_err());
}

#[tokio::test]
async fn tunnel_ingress_never_serves_hooks_or_the_bridge() {
    let st = state();
    let base = serve(server::router_for(st.clone(), Ingress::Tunnel)).await;
    let c = http();
    // Even the right credentials: hooks and /v1 are simply not routed here.
    let r = c
        .post(format!("{base}/hooks/event"))
        .bearer_auth(&st.auth.hook_token)
        .json(&json!({ "session_id": "t1" }))
        .send()
        .await
        .unwrap();
    assert_ne!(r.status(), StatusCode::OK);
    assert!(st.hooks.last_event_at("t1").is_none());
    let r = c
        .get(format!("{base}/v1/capabilities"))
        .send()
        .await
        .unwrap();
    assert!(
        r.json::<Value>().await.is_err(),
        "no bridge JSON on the tunnel"
    );
    // And nothing is ever local there, not even with the master token.
    let r = c
        .get(format!("{base}/api/sessions"))
        .bearer_auth(&st.auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_tokenless_bridge_port_refuses_tunnelled_requests() {
    let st = state();
    let base = serve(mother_claude_lib::bridge::listener_router(
        st.clone(),
        false,
    ))
    .await;
    let c = http();
    let r = c.get(format!("{base}/capabilities")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = c
        .get(format!("{base}/capabilities"))
        .header("x-forwarded-for", "203.0.113.9")
        .header("host", "abc.ngrok-free.app")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "tunneled_request_requires_token"
    );
}
