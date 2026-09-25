//! Security and remote access over real HTTP/WebSocket (design §2.3–2.9, §10
//! B1): pairing (TTL, single use, typed approval, lockout), device tokens and
//! revocation, the tunnel ingress, forged proxy headers on 6725 and 5612, the
//! bridge's dangerous gates, response hardening, rate limits and log hygiene.
//!
//! Nothing here touches `~/.claude` (it is only read, for trust checks), spawns
//! `claude`, starts the bridge host, or opens a public tunnel.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mother_claude_lib::bridge::BridgeConfig;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::context::ClientContext;
use mother_claude_lib::server::remote::DetectedTunnel;
use mother_claude_lib::server::{self, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

fn state() -> AppState {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.keep();
    Inner::with_bridge_config(
        ClaudeHome::with_base(&base),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
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

/// One state behind two listeners: the loopback one (the desktop) and the
/// tunnel ingress (phones on the internet).
async fn boot() -> (String, String, AppState) {
    let st = state();
    let local = serve(server::router_for(st.clone(), Ingress::Loopback)).await;
    let tunnel = serve(server::router_for(st.clone(), Ingress::Tunnel)).await;
    (local, tunnel, st)
}

/// A client whose requests look like they came through a TLS-terminating
/// tunnel from `ip`.
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

async fn internet_code(local: &str, st: &AppState) -> String {
    let v = json_of(
        desk()
            .post(format!("{local}/api/pair/codes"))
            .bearer_auth(&st.auth.token)
            .json(&json!({ "target": "internet", "baseUrl": "https://mc.example.ts.net" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    v["code"].as_str().unwrap().to_string()
}

/// Pair a device through the tunnel with desktop approval; returns
/// `(deviceId, token)`.
async fn pair_device(local: &str, tunnel: &str, st: &AppState, ip: &str) -> (String, String) {
    let code = internet_code(local, st).await;
    let phone = via_tunnel(ip);
    let r = phone
        .post(format!("{tunnel}/api/pair/exchange"))
        .json(&json!({ "code": code, "deviceName": "Test phone", "platform": "ios" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let p = json_of(r).await;
    let id = p["pendingId"].as_str().unwrap().to_string();
    let r = desk()
        .post(format!("{local}/api/devices/pending/{id}/approve"))
        .bearer_auth(&st.auth.token)
        .json(&json!({ "fingerprint": p["fingerprint"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v = json_of(
        phone
            .get(format!("{tunnel}/api/pair/pending/{id}"))
            .header("x-pair-secret", p["pollSecret"].as_str().unwrap())
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(v["status"], "approved");
    (
        v["deviceId"].as_str().unwrap().to_string(),
        v["token"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn internet_pairing_needs_typed_desktop_approval_and_hands_the_token_out_once() {
    let (local, tunnel, st) = boot().await;
    let d = st.auth.token.clone();

    // Internet codes need a public URL: none detected, none given.
    let r = desk()
        .post(format!("{local}/api/pair/codes"))
        .bearer_auth(&d)
        .json(&json!({ "target": "internet" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(json_of(r).await["error"]["code"], "no_public_url");
    // Plain http is refused for an internet base URL.
    let r = desk()
        .post(format!("{local}/api/pair/codes"))
        .bearer_auth(&d)
        .json(&json!({ "target": "internet", "baseUrl": "http://mc.example" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let v = json_of(
        desk()
            .post(format!("{local}/api/pair/codes"))
            .bearer_auth(&d)
            .json(&json!({ "target": "internet", "baseUrl": "https://mc.example.ts.net/" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let code = v["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 12);
    // The code travels in the fragment, never in a path or query.
    assert_eq!(
        v["url"],
        format!("https://mc.example.ts.net/#/pair?c={code}")
    );
    assert!(v["svg"].as_str().unwrap().contains("<svg"));
    assert!(v["expiresAt"].as_i64().unwrap() > chrono::Utc::now().timestamp_millis());

    // The phone exchanges a typed, lower-case, dashed form of it.
    let phone = via_tunnel("203.0.113.10");
    let typed = format!(
        "{}-{}-{}",
        &code[..4].to_lowercase(),
        &code[4..8],
        &code[8..]
    );
    let r = phone
        .post(format!("{tunnel}/api/pair/exchange"))
        .header("user-agent", "Mozilla/5.0 (iPhone)")
        .json(&json!({ "code": typed, "deviceName": "Pixel\u{1b}[2J 9", "platform": "android" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let p = json_of(r).await;
    let pending_id = p["pendingId"].as_str().unwrap().to_string();
    let secret = p["pollSecret"].as_str().unwrap().to_string();
    let fingerprint = p["fingerprint"].as_str().unwrap().to_string();
    assert_eq!(fingerprint.len(), 4);

    // The desktop sees who is asking — never the digits or the secret.
    let list = json_of(
        desk()
            .get(format!("{local}/api/devices"))
            .bearer_auth(&d)
            .send()
            .await
            .unwrap(),
    )
    .await;
    let pending = &list["pending"][0];
    assert_eq!(pending["id"], pending_id);
    assert_eq!(pending["clientIp"], "203.0.113.10");
    assert_eq!(pending["ingress"], "tunnel");
    assert_eq!(pending["deviceName"], "Pixel [2J 9");
    assert_eq!(pending["userAgent"], "Mozilla/5.0 (iPhone)");
    assert!(pending.get("fingerprint").is_none());
    assert!(pending.get("pollSecret").is_none());
    assert!(!list.to_string().contains(&secret));

    let poll = |secret: String| {
        let (phone, tunnel, id) = (phone.clone(), tunnel.clone(), pending_id.clone());
        async move {
            phone
                .get(format!("{tunnel}/api/pair/pending/{id}"))
                .header("x-pair-secret", secret)
                .send()
                .await
                .unwrap()
        }
    };
    assert_eq!(
        json_of(poll(secret.clone()).await).await["status"],
        "pending"
    );
    assert_eq!(
        poll("mcps_wrong".into()).await.status(),
        StatusCode::NOT_FOUND
    );

    // Approval is typed: the wrong digits are refused, with attempts left.
    let wrong = if fingerprint == "0000" {
        "1111"
    } else {
        "0000"
    };
    let r = desk()
        .post(format!("{local}/api/devices/pending/{pending_id}/approve"))
        .bearer_auth(&d)
        .json(&json!({ "fingerprint": wrong }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let e = json_of(r).await;
    assert_eq!(e["error"]["code"], "fingerprint_mismatch");
    assert_eq!(e["error"]["attemptsLeft"], 2);
    // A device can never approve.
    let r = desk()
        .post(format!("{local}/api/devices/pending/{pending_id}/approve"))
        .bearer_auth(&st.auth.script_token)
        .json(&json!({ "fingerprint": fingerprint }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    let r = desk()
        .post(format!("{local}/api/devices/pending/{pending_id}/approve"))
        .bearer_auth(&d)
        .json(&json!({ "fingerprint": fingerprint }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    // The token comes out exactly once.
    let v = json_of(poll(secret.clone()).await).await;
    assert_eq!(v["status"], "approved");
    let token = v["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("mcd_"));
    let again = json_of(poll(secret.clone()).await).await;
    assert_eq!(again["status"], "approved");
    assert!(again.get("token").is_none());

    // It works through the tunnel, as a device without dangerous rights.
    let me = json_of(
        phone
            .get(format!("{tunnel}/api/me"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(me["principal"], "device");
    assert_eq!(me["ingress"], "tunnel");
    assert_eq!(me["effectiveIngress"], "tunnel");
    assert_eq!(me["local"], false);
    assert_eq!(me["allowDangerous"], false);
    assert_eq!(me["clientIp"], "203.0.113.10");
    assert_eq!(me["platform"], "android");
    // …and never gets desktop routes.
    let r = phone
        .get(format!("{tunnel}/api/devices"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // The desktop token is refused through the tunnel.
    let r = phone
        .get(format!("{tunnel}/api/me"))
        .bearer_auth(&d)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

    // Denying afterwards is a conflict.
    let r = desk()
        .post(format!("{local}/api/devices/pending/{pending_id}/deny"))
        .bearer_auth(&d)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn three_wrong_fingerprints_deny_the_request() {
    let (local, tunnel, st) = boot().await;
    let code = internet_code(&local, &st).await;
    let phone = via_tunnel("203.0.113.11");
    let p = json_of(
        phone
            .post(format!("{tunnel}/api/pair/exchange"))
            .json(&json!({ "code": code, "deviceName": "x" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let id = p["pendingId"].as_str().unwrap();
    let fp = p["fingerprint"].as_str().unwrap();
    let wrong = if fp == "0000" { "1111" } else { "0000" };
    for _ in 0..3 {
        let r = desk()
            .post(format!("{local}/api/devices/pending/{id}/approve"))
            .bearer_auth(&st.auth.token)
            .json(&json!({ "fingerprint": wrong }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let r = desk()
        .post(format!("{local}/api/devices/pending/{id}/approve"))
        .bearer_auth(&st.auth.token)
        .json(&json!({ "fingerprint": fp }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let v = json_of(
        phone
            .get(format!("{tunnel}/api/pair/pending/{id}"))
            .header("x-pair-secret", p["pollSecret"].as_str().unwrap())
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(v["status"], "denied");
}

#[tokio::test]
async fn codes_are_single_use_and_a_replay_contests_the_request() {
    let (local, tunnel, st) = boot().await;
    let code = internet_code(&local, &st).await;
    let a = via_tunnel("203.0.113.20");
    let b = via_tunnel("198.51.100.20");
    let pa = json_of(
        a.post(format!("{tunnel}/api/pair/exchange"))
            .json(&json!({ "code": code, "deviceName": "real phone" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    // A second device presents the same code: refused, and the first request
    // can no longer be approved.
    let r = b
        .post(format!("{tunnel}/api/pair/exchange"))
        .json(&json!({ "code": code, "deviceName": "attacker" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(json_of(r).await["error"]["code"], "code_already_used");
    let id = pa["pendingId"].as_str().unwrap();
    let r = desk()
        .post(format!("{local}/api/devices/pending/{id}/approve"))
        .bearer_auth(&st.auth.token)
        .json(&json!({ "fingerprint": pa["fingerprint"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(json_of(r).await["error"]["code"], "contested");
    let v = json_of(
        a.get(format!("{tunnel}/api/pair/pending/{id}"))
            .header("x-pair-secret", pa["pollSecret"].as_str().unwrap())
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(v["status"], "contested");
    let list = json_of(
        desk()
            .get(format!("{local}/api/devices"))
            .bearer_auth(&st.auth.token)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(list["pending"][0]["contested"], true);
    assert!(list["devices"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn lan_codes_pair_automatically_only_on_the_lan() {
    let (local, tunnel, st) = boot().await;
    let code = json_of(
        desk()
            .post(format!("{local}/api/pair/codes"))
            .bearer_auth(&st.auth.token)
            .json(&json!({ "target": "lan", "baseUrl": "https://192.168.1.5:6725" }))
            .send()
            .await
            .unwrap(),
    )
    .await["code"]
        .as_str()
        .unwrap()
        .to_string();
    // A photographed LAN QR redeemed through a tunnel: refused, not consumed.
    let r = via_tunnel("203.0.113.30")
        .post(format!("{tunnel}/api/pair/exchange"))
        .json(&json!({ "code": code }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"]["code"], "wrong_network");
    // Nor on the loopback listener (a raw TCP forward looks like this).
    let r = desk()
        .post(format!("{local}/api/pair/exchange"))
        .json(&json!({ "code": code }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    let exchange = |peer: &str| {
        let mut h = axum::http::HeaderMap::new();
        h.insert("host", "192.168.1.5:6725".parse().unwrap());
        let ctx = ClientContext::new(Ingress::Lan, peer.parse().unwrap(), &h, false);
        mother_claude_lib::server::pairing::exchange(
            &st,
            &ctx,
            &h,
            serde_json::from_value(json!({ "code": code, "deviceName": "Home phone" })).unwrap(),
        )
    };
    // A port-forwarded public peer on the LAN listener counts as a tunnel.
    let err = exchange("203.0.113.9:4000").unwrap_err();
    assert_eq!(err.code, "wrong_network");
    // A private LAN peer: paired at once.
    let (status, v) = exchange("192.168.1.9:4000").unwrap();
    assert_eq!(status, StatusCode::CREATED);
    let token = v["token"].as_str().unwrap();
    let device = st.devices.get(v["deviceId"].as_str().unwrap()).unwrap();
    assert_eq!(device.paired_via.as_deref(), Some("lan"));
    assert!(token.starts_with("mcd_"));
    // Single use.
    let err = exchange("192.168.1.10:4000").unwrap_err();
    assert_eq!(err.code, "code_already_used");
}

#[tokio::test]
async fn pairing_failures_are_limited_per_client_ip() {
    let (_local, tunnel, _st) = boot().await;
    let attacker = via_tunnel("203.0.113.40");
    for _ in 0..10 {
        let r = attacker
            .post(format!("{tunnel}/api/pair/exchange"))
            .json(&json!({ "code": "ZZZZZZZZZZZZ" }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }
    let r = attacker
        .post(format!("{tunnel}/api/pair/exchange"))
        .json(&json!({ "code": "ZZZZZZZZZZZZ" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(r.headers().contains_key("retry-after"));
    // Polling with guessed secrets counts too, from that address only.
    let r = attacker
        .get(format!("{tunnel}/api/pair/pending/pp_x"))
        .header("x-pair-secret", "x")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    // Somebody else (a different tunnel client) is not locked out.
    let r = via_tunnel("198.51.100.40")
        .post(format!("{tunnel}/api/pair/exchange"))
        .json(&json!({ "code": "ZZZZZZZZZZZZ" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    // Oversized bodies are refused before parsing.
    let r = via_tunnel("198.51.100.41")
        .post(format!("{tunnel}/api/pair/exchange"))
        .header("content-type", "application/json")
        .body(format!("{{\"code\":\"{}\"}}", "A".repeat(8192)))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error());
}

#[tokio::test]
async fn at_most_five_pairings_wait_for_approval() {
    let (local, tunnel, st) = boot().await;
    for i in 0..5 {
        let code = internet_code(&local, &st).await;
        let r = via_tunnel(&format!("203.0.113.{}", 50 + i))
            .post(format!("{tunnel}/api/pair/exchange"))
            .json(&json!({ "code": code }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::ACCEPTED);
    }
    let code = internet_code(&local, &st).await;
    let r = via_tunnel("203.0.113.60")
        .post(format!("{tunnel}/api/pair/exchange"))
        .json(&json!({ "code": code }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
}

async fn ws_via_tunnel(
    tunnel: &str,
    ip: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("{}/ws", tunnel.replacen("http", "ws", 1));
    let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        url.as_str(),
    )
    .unwrap();
    req.headers_mut()
        .insert("x-forwarded-proto", "https".parse().unwrap());
    req.headers_mut()
        .insert("x-forwarded-for", ip.parse().unwrap());
    tokio_tungstenite::connect_async(req).await.unwrap().0
}

async fn next_frame<S>(ws: &mut S) -> Option<Value>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??;
        match msg {
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(&t).ok()?;
                if v["t"] == "ping" {
                    continue;
                }
                return Some(v);
            }
            Ok(Message::Close(frame)) => {
                return Some(json!({ "t": "close", "code": frame.map(|f| u16::from(f.code)) }))
            }
            Ok(_) => continue,
            Err(_) => return Some(json!({ "t": "error" })),
        }
    }
}

#[tokio::test]
async fn revoking_a_device_closes_its_sockets_and_kills_every_token() {
    let (local, tunnel, st) = boot().await;
    let (id, token) = pair_device(&local, &tunnel, &st, "203.0.113.70").await;
    let phone = via_tunnel("203.0.113.70");

    // The Home Screen app gets a second token for the same device.
    let v = json_of(
        phone
            .post(format!("{tunnel}/api/pair/codes"))
            .bearer_auth(&token)
            .json(&json!({ "target": "self" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let self_code = v["code"].as_str().unwrap().to_string();
    assert_eq!(v["url"], format!("/#/pair?c={self_code}"));
    // A device may not mint codes for new devices.
    let r = phone
        .post(format!("{tunnel}/api/pair/codes"))
        .bearer_auth(&token)
        .json(&json!({ "target": "internet", "baseUrl": "https://x.example" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = phone
        .post(format!("{tunnel}/api/pair/exchange"))
        .json(&json!({ "code": self_code, "deviceName": "Home Screen" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let second = json_of(r).await;
    assert_eq!(second["deviceId"], id.as_str());
    let token2 = second["token"].as_str().unwrap().to_string();
    assert_ne!(token2, token);

    // A WebSocket through the tunnel, first-frame auth.
    let mut ws = ws_via_tunnel(&tunnel, "203.0.113.70").await;
    ws.send(Message::Text(
        json!({ "t": "auth", "token": token2 }).to_string().into(),
    ))
    .await
    .unwrap();
    let hello = next_frame(&mut ws).await.unwrap();
    assert_eq!(hello["t"], "hello");
    assert_eq!(hello["me"]["deviceId"], id.as_str());
    assert_eq!(next_frame(&mut ws).await.unwrap()["t"], "sessions");

    let listed = json_of(
        desk()
            .get(format!("{local}/api/devices"))
            .bearer_auth(&st.auth.token)
            .send()
            .await
            .unwrap(),
    )
    .await;
    let dev = &listed["devices"][0];
    assert_eq!(dev["tokenCount"], 2);
    assert_eq!(dev["sockets"], 1);
    assert!(!listed.to_string().contains(&token));
    // The device's recent actions show on the desktop.
    assert!(dev["recent"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["action"] == "POST /api/pair/codes"));

    let r = desk()
        .delete(format!("{local}/api/devices/{id}"))
        .bearer_auth(&st.auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let closed = next_frame(&mut ws).await.unwrap();
    assert_eq!(closed["t"], "close");
    assert_eq!(closed["code"], 4401);
    for t in [&token, &token2] {
        let r = phone
            .get(format!("{tunnel}/api/me"))
            .bearer_auth(t)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
    let dev = st.devices.get(&id).unwrap();
    assert!(dev.revoked && dev.token_count == 0);
    // Deleting a revoked device forgets it.
    let r = desk()
        .delete(format!("{local}/api/devices/{id}"))
        .bearer_auth(&st.auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(json_of(r).await["forgotten"], true);
    assert!(st.devices.get(&id).is_none());
    // The audit log recorded the device's own action and the revocation.
    let audit = st.devices.audit.recent(100);
    assert!(audit.iter().any(|e| e.action == "pair.self"));
    assert!(audit.iter().any(|e| e.action == "device.revoked"));
    assert!(audit
        .iter()
        .any(|e| e.action == "http" && e.device_id.as_deref() == Some(id.as_str())));
}

#[tokio::test]
async fn a_device_can_unpair_itself() {
    let (local, tunnel, st) = boot().await;
    let (id, token) = pair_device(&local, &tunnel, &st, "203.0.113.71").await;
    let phone = via_tunnel("203.0.113.71");
    let r = phone
        .delete(format!("{tunnel}/api/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(st.devices.get(&id).unwrap().revoked);
    let r = phone
        .get(format!("{tunnel}/api/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // The desktop cannot "unpair itself".
    let r = desk()
        .delete(format!("{local}/api/me"))
        .bearer_auth(&st.auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn dangerous_rights_follow_the_principal_and_the_ingress_grant() {
    let (local, tunnel, st) = boot().await;
    let (id, token) = pair_device(&local, &tunnel, &st, "203.0.113.80").await;
    let allow = |who: reqwest::Client, base: String, t: String| async move {
        json_of(
            who.get(format!("{base}/api/me"))
                .bearer_auth(t)
                .send()
                .await
                .unwrap(),
        )
        .await["allowDangerous"]
            .clone()
    };
    let phone = via_tunnel("203.0.113.80");

    // A device token over a raw TCP forward (`Host: 127.0.0.1`, loopback
    // peer, no proxy headers) is "local" by address — and still gets nothing.
    let me = json_of(
        desk()
            .get(format!("{local}/api/me"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(me["local"], true);
    assert_eq!(me["allowDangerous"], false);
    let r = desk()
        .post(format!("{local}/api/sessions/nope/rm"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"]["code"], "dangerous_blocked");

    // The LAN grant does not reach the tunnel or the loopback listener.
    let r = desk()
        .patch(format!("{local}/api/devices/{id}"))
        .bearer_auth(&st.auth.token)
        .json(&json!({ "allowDangerous": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        allow(phone.clone(), tunnel.clone(), token.clone()).await,
        false
    );
    assert_eq!(allow(desk(), local.clone(), token.clone()).await, false);
    // The internet grant does.
    desk()
        .patch(format!("{local}/api/devices/{id}"))
        .bearer_auth(&st.auth.token)
        .json(&json!({ "allowDangerousInternet": true, "name": "Renamed\u{7}" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        allow(phone.clone(), tunnel.clone(), token.clone()).await,
        true
    );
    assert_eq!(st.devices.get(&id).unwrap().name, "Renamed");
    // A device can never edit devices.
    let r = phone
        .patch(format!("{tunnel}/api/devices/{id}"))
        .bearer_auth(&token)
        .json(&json!({ "allowDangerous": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    // The desktop token is dangerous-capable; the on-disk script token is not,
    // and never reaches admin routes.
    assert_eq!(
        allow(desk(), local.clone(), st.auth.token.clone()).await,
        true
    );
    let me = json_of(
        desk()
            .get(format!("{local}/api/me"))
            .bearer_auth(&st.auth.script_token)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(me["principal"], "local-script");
    assert_eq!(me["allowDangerous"], false);
    for (method, path) in [
        ("GET", "/api/devices"),
        ("PUT", "/api/remote"),
        ("POST", "/api/pair/codes"),
    ] {
        let r = desk()
            .request(method.parse().unwrap(), format!("{local}{path}"))
            .bearer_auth(&st.auth.script_token)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }
    let r = desk()
        .get(format!("{local}/api/sessions"))
        .bearer_auth(&st.auth.script_token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // The script token is local-only too.
    let r = phone
        .get(format!("{tunnel}/api/sessions"))
        .bearer_auth(&st.auth.script_token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn forged_proxy_headers_and_query_tokens_never_count_as_local() {
    let (local, tunnel, st) = boot().await;
    let d = st.auth.token.clone();
    for (name, value) in [
        ("forwarded", "for=203.0.113.9"),
        ("x-forwarded-host", "evil.example"),
        ("x-forwarded-port", "443"),
        ("x-real-ip", "203.0.113.9"),
        ("cdn-loop", "cloudflare"),
        ("cf-ray", "abc"),
        ("tailscale-user-login", "x@example.com"),
    ] {
        let r = desk()
            .get(format!("{local}/api/me"))
            .bearer_auth(&d)
            .header(name, value)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{name}");
    }
    for host in [
        "localhost.evil.com",
        "127.0.0.1.nip.io",
        "0.0.0.0:6725",
        "[::]:6725",
    ] {
        let r = desk()
            .get(format!("{local}/api/me"))
            .bearer_auth(&d)
            .header("host", host)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{host}");
    }
    for origin in ["null", "http://localhost.evil.com", "https://example.com"] {
        let r = desk()
            .get(format!("{local}/api/me"))
            .bearer_auth(&d)
            .header("origin", origin)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{origin}");
    }
    // ?token= only for genuinely local callers.
    let r = desk()
        .get(format!("{local}/api/me?token={d}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = desk()
        .get(format!("{local}/api/me?token={d}"))
        .header("x-forwarded-for", "203.0.113.9")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r = via_tunnel("203.0.113.9")
        .get(format!("{tunnel}/api/me?token={d}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // A proxied request on 6725 gets the reduced router: no /v1, no /hooks.
    let r = desk()
        .get(format!("{local}/v1/capabilities"))
        .bearer_auth(&d)
        .header("x-forwarded-for", "203.0.113.9")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = desk()
        .post(format!("{local}/hooks/event"))
        .bearer_auth(&st.auth.hook_token)
        .header("ngrok-trace-id", "t")
        .json(&json!({ "session_id": "s" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

/// The bridge port (5612): tokenless and origin-open for this machine, but
/// never dangerous for another website, never tokenless through a tunnel, and
/// never tokenless while a tunnel points at it.
#[tokio::test]
async fn the_bridge_port_keeps_local_tokenless_access_but_not_dangerous_work() {
    let st = state();
    mother_claude_lib::bridge::control::start(&st, Default::default())
        .await
        .unwrap();
    let base = serve(mother_claude_lib::bridge::listener_router(
        st.clone(),
        false,
    ))
    .await;
    let c = reqwest::Client::new();
    let page = "https://example.com";
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_string_lossy().to_string();

    // Another website on this machine: tokenless for ordinary routes…
    let r = c
        .get(format!("{base}/capabilities"))
        .header("origin", page)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // …never for skipping approvals…
    let r = c
        .post(format!("{base}/threads"))
        .header("origin", page)
        .json(&json!({ "cwd": cwd, "permission_mode": "bypassPermissions" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"]["code"], "dangerous_blocked");
    // …or for options that run code nobody approves…
    for body in [
        json!({ "cwd": cwd, "mcp_servers": { "x": { "command": "sh" } } }),
        json!({ "cwd": cwd, "allowed_tools": ["Bash"] }),
        json!({ "cwd": cwd, "additional_directories": ["/"] }),
    ] {
        let r = c
            .post(format!("{base}/threads"))
            .header("origin", page)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{body}");
        assert_eq!(json_of(r).await["error"]["code"], "dangerous_blocked");
    }
    // …or in a folder the user never trusted.
    let r = c
        .post(format!("{base}/threads"))
        .header("origin", page)
        .json(&json!({ "cwd": cwd }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"]["code"], "untrusted_cwd");
    // Work sent into a conversation whose approvals are off is dangerous too.
    let thread = uuid::Uuid::new_v4().to_string();
    st.bridge
        .insert_thread(mother_claude_lib::bridge::ThreadRecord {
            thread_id: thread.clone(),
            cwd: cwd.clone(),
            created_at: 0,
            model: None,
            effort: None,
            permission_mode: "bypassPermissions".into(),
            title: None,
            persisted: false,
            active_turn: None,
        })
        .await;
    let r = c
        .post(format!("{base}/threads/{thread}/turns"))
        .header("origin", page)
        .json(&json!({ "input": "rm -rf ~" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"]["code"], "dangerous_blocked");

    // Through a tunnel, a token is required, with or without a dangerous body.
    let r = c
        .post(format!("{base}/threads"))
        .header("x-forwarded-for", "203.0.113.9")
        .header("host", "abc.ngrok-free.app")
        .json(&json!({ "cwd": cwd, "permission_mode": "bypassPermissions" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        json_of(r).await["error"]["code"],
        "tunneled_request_requires_token"
    );

    // A detected tunnel pointing at this port switches tokenless mode off.
    st.remote.set_detected(
        vec![DetectedTunnel {
            provider: "ngrok".into(),
            url: "tcp://0.tcp.ngrok.io:12345".into(),
            upstream: Some("localhost:5612".into()),
            upstream_port: Some(5612),
            inspect: false,
            kind: "tcp".into(),
            target: "bridge".into(),
        }],
        Some(5612),
    );
    let r = c.get(format!("{base}/capabilities")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        json_of(r).await["error"]["code"],
        "bridge_exposed_requires_token"
    );
    let r = c
        .get(format!("{base}/capabilities"))
        .bearer_auth(&st.auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    st.remote.set_detected(Vec::new(), Some(5612));
    let r = c.get(format!("{base}/capabilities")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn trusted_folders_are_the_ones_claude_already_knows() {
    let st = state();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_string_lossy().to_string();
    use mother_claude_lib::bridge::routes::threads::cwd_is_trusted;
    assert!(!cwd_is_trusted(&st.home, &cwd));
    assert!(!cwd_is_trusted(&st.home, "relative/path"));
    std::fs::create_dir_all(st.home.transcript_dir(&cwd)).unwrap();
    assert!(cwd_is_trusted(&st.home, &cwd));
}

#[tokio::test]
async fn responses_carry_the_hardening_headers() {
    let (local, tunnel, _st) = boot().await;
    let r = desk()
        .get(format!("{local}/api/health"))
        .send()
        .await
        .unwrap();
    let h = r.headers();
    let csp = h["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("script-src 'self';"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains("connect-src 'self'"));
    assert!(!csp.contains("unsafe-eval"));
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["cache-control"], "no-store");
    assert!(h.get("strict-transport-security").is_none());

    // The SPA (or its placeholder) gets the CSP too.
    let r = desk().get(format!("{local}/")).send().await.unwrap();
    assert!(r.headers().contains_key("content-security-policy"));

    // Through the tunnel: HTTPS is required, and HSTS is sent.
    let r = desk()
        .get(format!("{tunnel}/api/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_of(r).await["error"]["code"], "https_required");
    let r = desk()
        .get(format!("{tunnel}/api/health"))
        .header("x-forwarded-proto", "http, https")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK, "the LAST value counts");
    let r = via_tunnel("203.0.113.90")
        .get(format!("{tunnel}/api/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["strict-transport-security"], "max-age=31536000");
    assert_eq!(r.headers()["cross-origin-opener-policy"], "same-origin");

    // CORS: the desktop webview's origin locally; nobody else.
    let preflight = |base: String, origin: &'static str, client: reqwest::Client| async move {
        client
            .request(reqwest::Method::OPTIONS, format!("{base}/api/sessions"))
            .header("origin", origin)
            .header("access-control-request-method", "GET")
            .header("access-control-request-headers", "authorization")
            .send()
            .await
            .unwrap()
    };
    let r = preflight(local.clone(), "tauri://localhost", desk()).await;
    assert_eq!(
        r.headers()["access-control-allow-origin"],
        "tauri://localhost"
    );
    let r = preflight(local.clone(), "https://evil.example", desk()).await;
    assert!(r.headers().get("access-control-allow-origin").is_none());
    let r = preflight(
        tunnel.clone(),
        "https://phone.example",
        via_tunnel("203.0.113.91"),
    )
    .await;
    assert!(r.headers().get("access-control-allow-origin").is_none());

    // Compression for large responses; never for pairing answers.
    let r = desk()
        .get(format!("{local}/"))
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();
    let body_len = r.headers().get("content-length").cloned();
    if r.status() == StatusCode::OK {
        assert_eq!(r.headers()["content-encoding"], "gzip", "{body_len:?}");
    }
    let r = desk()
        .post(format!("{local}/api/pair/exchange"))
        .header("accept-encoding", "gzip, br")
        .json(&json!({ "code": "ZZZZZZZZZZZZ", "deviceName": "x".repeat(200) }))
        .send()
        .await
        .unwrap();
    assert!(r.headers().get("content-encoding").is_none());
}

#[tokio::test]
async fn unauthenticated_websockets_are_capped_per_client() {
    let (_local, tunnel, _st) = boot().await;
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(ws_via_tunnel(&tunnel, "203.0.113.95").await);
    }
    let url = format!("{}/ws", tunnel.replacen("http", "ws", 1));
    let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        url.as_str(),
    )
    .unwrap();
    req.headers_mut()
        .insert("x-forwarded-proto", "https".parse().unwrap());
    req.headers_mut()
        .insert("x-forwarded-for", "203.0.113.95".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(req).await.is_err());
    // Another client is unaffected.
    let mut other = ws_via_tunnel(&tunnel, "198.51.100.95").await;
    // An oversized first frame is refused.
    other
        .send(Message::Text("x".repeat(8 * 1024).into()))
        .await
        .unwrap();
    assert_eq!(next_frame(&mut other).await.unwrap()["t"], "close");
    // Failing auth frees the slots.
    for ws in &mut held {
        ws.send(Message::Text(r#"{"t":"auth","token":"mcd_nope"}"#.into()))
            .await
            .unwrap();
        assert_eq!(next_frame(ws).await.unwrap()["code"], 4401);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _again = ws_via_tunnel(&tunnel, "203.0.113.95").await;
}

#[tokio::test]
async fn internet_access_binds_and_cuts_the_tunnel_listener() {
    let (local, tunnel_router, st) = boot().await;
    let (_id, token) = pair_device(&local, &tunnel_router, &st, "203.0.113.100").await;
    st.remote.set_tunnel_port(0);
    let d = st.auth.token.clone();

    let v = json_of(
        desk()
            .get(format!("{local}/api/remote"))
            .bearer_auth(&d)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(v["internet"]["enabled"], false);
    assert_eq!(v["internet"]["listening"], false);
    assert_eq!(v["internet"]["requireApproval"], true);
    assert!(v["internet"]["commands"]["ngrok"]
        .as_str()
        .unwrap()
        .contains("--inspect=false"));
    assert_eq!(v["lan"]["enabled"], false);
    assert_eq!(v["bridge"]["exposed"], false);
    assert_eq!(v["devices"]["active"], 1);

    // Validation.
    for body in [
        json!({ "manualUrl": "http://insecure.example" }),
        json!({ "manualUrl": "https://x.example/path" }),
        json!({ "bogus": 1 }),
        json!({ "internetEnabled": "yes" }),
        json!({ "deviceIdleDays": 0 }),
    ] {
        let r = desk()
            .put(format!("{local}/api/remote"))
            .bearer_auth(&d)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    // Only the desktop.
    let r = via_tunnel("203.0.113.100")
        .put(format!("{tunnel_router}/api/remote"))
        .bearer_auth(&token)
        .json(&json!({ "internetEnabled": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    let v = json_of(
        desk()
            .put(format!("{local}/api/remote"))
            .bearer_auth(&d)
            .json(&json!({
                "internetEnabled": true,
                "manualUrl": "https://me.example/",
                "deviceIdleDays": 14,
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(v["internet"]["enabled"], true);
    assert_eq!(v["internet"]["listening"], true);
    assert_eq!(v["internet"]["publicUrl"], "https://me.example");
    assert_eq!(v["devices"]["idleDays"], 14);
    let port = v["internet"]["port"].as_u64().unwrap();
    assert_ne!(port, 0);
    let tunnel = format!("http://127.0.0.1:{port}");

    // An internet code now defaults to the public URL.
    let code = json_of(
        desk()
            .post(format!("{local}/api/pair/codes"))
            .bearer_auth(&d)
            .json(&json!({ "target": "internet" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert!(code["url"]
        .as_str()
        .unwrap()
        .starts_with("https://me.example/#/pair?c="));

    // The real tunnel listener serves the device, REST and WebSocket.
    let phone = via_tunnel("203.0.113.100");
    let r = phone
        .get(format!("{tunnel}/api/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let mut ws = ws_via_tunnel(&tunnel, "203.0.113.100").await;
    ws.send(Message::Text(
        json!({ "t": "auth", "token": token }).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_frame(&mut ws).await.unwrap()["t"], "hello");

    // Switching it off cuts the open WebSocket and refuses new connections.
    let v = json_of(
        desk()
            .put(format!("{local}/api/remote"))
            .bearer_auth(&d)
            .json(&json!({ "internetEnabled": false, "manualUrl": null }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(v["internet"]["listening"], false);
    assert!(v["internet"]["publicUrl"].is_null());
    let ended = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "the tunnel WebSocket was cut");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(phone
        .get(format!("{tunnel}/api/me"))
        .bearer_auth(&token)
        .send()
        .await
        .is_err());
}
