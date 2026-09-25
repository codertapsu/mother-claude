//! End-to-end push pipeline against a local mock push service: triggers →
//! suppression → coalescing → per-device selection → presence → encrypted,
//! VAPID-signed delivery → outcome handling (404/410 removal, redirects not
//! followed, rate limits).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use serde_json::Value;
use web_push_native::Auth;

use super::*;
use crate::claude::model::{
    PendingKind, PendingSource, SessionStatus, SessionSummary, UiEvent, UiKind, UiTool,
};

#[derive(Debug, Clone)]
struct Hit {
    name: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Clone, Default)]
struct Mock {
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Mock {
    fn hits(&self, name: &str) -> Vec<Hit> {
        lock(&self.hits)
            .iter()
            .filter(|h| h.name == name)
            .cloned()
            .collect()
    }

    fn total(&self) -> usize {
        lock(&self.hits).len()
    }
}

async fn mock_push(
    State(m): State<Mock>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    lock(&m.hits).push(Hit {
        name: name.clone(),
        headers,
        body: body.to_vec(),
    });
    match name.as_str() {
        n if n.starts_with("gone") => StatusCode::GONE.into_response(),
        n if n.starts_with("missing") => StatusCode::NOT_FOUND.into_response(),
        n if n.starts_with("rate") => {
            (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "30")]).into_response()
        }
        n if n.starts_with("forbidden") => StatusCode::FORBIDDEN.into_response(),
        n if n.starts_with("err") => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        n if n.starts_with("redirect") => (
            StatusCode::TEMPORARY_REDIRECT,
            [("location", "/push/target")],
        )
            .into_response(),
        _ => StatusCode::CREATED.into_response(),
    }
}

async fn mock_server() -> (String, Mock) {
    let mock = Mock::default();
    let app = Router::new()
        .route("/push/{name}", post(mock_push))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/push"), mock)
}

fn state() -> AppState {
    crate::state::Inner::new(
        crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
        crate::state::ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        crate::server::auth::Auth::ephemeral(),
    )
}

/// A started push service that talks to the mock.
fn started(window_ms: i64) -> AppState {
    let st = state();
    st.push.use_test_transport();
    st.push.set_window_ms(window_ms);
    start(&st);
    st
}

/// Subscribe `device` to the mock endpoint `name`; returns the browser key.
fn subscribe_mock(
    st: &AppState,
    base: &str,
    device: &str,
    name: &str,
) -> web_push_native::p256::SecretKey {
    let (sk, mut sub) = payload_test_subscription();
    sub.endpoint = format!("{base}/{name}");
    st.push.insert_unchecked(device, sub);
    sk
}

fn decrypt(hit: &Hit, sk: &web_push_native::p256::SecretKey) -> Value {
    let plain = web_push_native::decrypt(hit.body.clone(), sk, &Auth::clone_from_slice(&[9u8; 16]))
        .expect("decrypts with the browser key");
    serde_json::from_slice(&plain).unwrap()
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..250 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

/// Let in-flight work settle, then assert nothing more arrived.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

async fn session(st: &AppState, id: &str, name: &str, status: SessionStatus) {
    crate::server::ws::upsert_session(
        st,
        SessionSummary {
            id: id.into(),
            name: name.into(),
            project_name: name.into(),
            status,
            live: true,
            ..Default::default()
        },
    )
    .await;
}

fn permission(session: &str, tool: &str, detail: &str) -> PendingItem {
    PendingItem {
        session_id: session.into(),
        kind: PendingKind::Permission,
        source: PendingSource::Hook,
        tool_name: Some(tool.into()),
        title: format!("{tool}: {detail}"),
        detail: Some(detail.into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn delivers_an_encrypted_vapid_signed_notification() {
    let (base, mock) = mock_server().await;
    let st = started(200);
    let sk = subscribe_mock(&st, &base, "d1", "phone");
    session(&st, "s1", "my-repo", SessionStatus::Working).await;

    let (_guard, _rx) =
        crate::pending::raise(&st, permission("s1", "Bash", "rm -rf ~/secret")).await;
    wait_until("the push", || !mock.hits("phone").is_empty()).await;
    let hit = &mock.hits("phone")[0];
    let h = &hit.headers;
    assert_eq!(h["content-encoding"], "aes128gcm");
    assert_eq!(h["content-type"], "application/octet-stream");
    assert_eq!(h["ttl"], "3600");
    assert_eq!(h["urgency"], "high");
    assert_eq!(h["topic"], payload::topic_for("s1").as_str());
    let auth = h["authorization"].to_str().unwrap();
    assert!(auth.starts_with("vapid t="), "{auth}");
    assert!(
        auth.ends_with(&format!(", k={}", st.push.public_key())),
        "{auth}"
    );

    let v = decrypt(hit, &sk);
    assert_eq!(v["title"], "my-repo");
    assert_eq!(v["body"], "Needs approval · Bash");
    assert_eq!(v["sessionId"], "s1");
    assert_eq!(v["kind"], "permission");
    assert_eq!(v["renotify"], true);
    let pending_id = st.pending.list("s1")[0].id.clone();
    assert_eq!(v["pendingId"], pending_id.as_str());
    assert_eq!(v["tag"], pending_id.as_str());
    let text = v.to_string();
    assert!(!text.contains("secret"), "no details by default: {text}");
    assert!(!text.contains("http"), "no URL in the payload: {text}");

    // The status flip to `waiting` that the card causes is not a second push.
    settle().await;
    assert_eq!(mock.hits("phone").len(), 1);
}

#[tokio::test]
async fn details_only_for_devices_that_opt_in() {
    let (base, mock) = mock_server().await;
    let st = started(200);
    let sk = subscribe_mock(&st, &base, "d1", "detailed");
    let patch: PrefsPatch =
        serde_json::from_value(serde_json::json!({"showDetails": true})).unwrap();
    st.push.update_prefs("d1", patch).unwrap();
    let (_g, _rx) = crate::pending::raise(&st, permission("s1", "Bash", "npm test")).await;
    wait_until("the push", || !mock.hits("detailed").is_empty()).await;
    let v = decrypt(&mock.hits("detailed")[0], &sk);
    assert_eq!(v["body"], "Needs approval · Bash: npm test");
}

#[tokio::test]
async fn coalesces_per_session_and_skips_devices_looking_at_the_app() {
    let (base, mock) = mock_server().await;
    // A generous window so a slow CI scheduler cannot split the burst.
    let st = started(1_000);
    let sk = subscribe_mock(&st, &base, "away", "away");
    subscribe_mock(&st, &base, "watching", "watching");
    // `watching` has the app open and visible.
    st.presence.connect("watching", 1);

    let (_g1, _r1) = crate::pending::raise(&st, permission("s1", "Bash", "a")).await;
    let (_g2, _r2) = crate::pending::raise(&st, permission("s1", "Edit", "b")).await;
    let (_g3, _r3) = crate::pending::raise(
        &st,
        PendingItem {
            kind: PendingKind::Question,
            ..permission("s1", "AskUserQuestion", "c")
        },
    )
    .await;
    let (_g4, _r4) = crate::pending::raise(&st, permission("s2", "Write", "d")).await;

    // s1: one leading push, then ONE trailing push for the other two; s2:
    // its own leading push.
    wait_until("three pushes", || mock.hits("away").len() >= 3).await;
    settle().await;
    let hits = mock.hits("away");
    assert_eq!(hits.len(), 3, "coalesced");
    let bodies: Vec<Value> = hits.iter().map(|h| decrypt(h, &sk)).collect();
    let s1: Vec<&Value> = bodies.iter().filter(|b| b["sessionId"] == "s1").collect();
    assert_eq!(s1.len(), 2);
    assert_eq!(s1[0]["body"], "Needs approval · Bash");
    assert_eq!(
        s1[1]["body"],
        "2 updates · Needs approval · Edit, Has a question"
    );
    assert_eq!(s1[1]["tag"], "s1");
    assert_eq!(s1[1]["pendingIds"].as_array().unwrap().len(), 2);
    assert!(bodies.iter().any(|b| b["sessionId"] == "s2"));
    // The foreground device got nothing.
    assert!(mock.hits("watching").is_empty());
    // Once it goes to the background it is notified again.
    st.presence.set_visible("watching", 1, false);
    let (_g5, _r5) = crate::pending::raise(&st, permission("s3", "Bash", "e")).await;
    wait_until("background push", || !mock.hits("watching").is_empty()).await;
}

#[tokio::test]
async fn answered_cards_are_not_pushed_in_the_trailing_batch() {
    let (base, mock) = mock_server().await;
    let st = started(1_000);
    subscribe_mock(&st, &base, "d1", "p");
    let (_g1, _r1) = crate::pending::raise(&st, permission("s1", "Bash", "a")).await;
    let (g2, _r2) = crate::pending::raise(&st, permission("s1", "Edit", "b")).await;
    // Answered at the Mac before the window closed.
    drop(g2);
    wait_until("leading push", || mock.hits("p").len() == 1).await;
    // Past the end of the window: the trailing batch had nothing left.
    tokio::time::sleep(Duration::from_millis(1_400)).await;
    assert_eq!(mock.hits("p").len(), 1);
}

#[tokio::test]
async fn gone_and_unusable_subscriptions_are_removed() {
    let (base, mock) = mock_server().await;
    let st = started(100);
    subscribe_mock(&st, &base, "d1", "ok");
    subscribe_mock(&st, &base, "d1", "gone");
    subscribe_mock(&st, &base, "d1", "missing");
    subscribe_mock(&st, &base, "d1", "rate");
    let (_, mut bad) = payload_test_subscription();
    bad.endpoint = format!("{base}/badkeys");
    bad.auth = "AAAA".into();
    st.push.insert_unchecked("d1", bad);

    let (_g, _r) = crate::pending::raise(&st, permission("s1", "Bash", "x")).await;
    wait_until("four attempts", || mock.total() >= 4).await;
    settle().await;
    let left = st.push.device_endpoints("d1");
    assert!(left.iter().any(|e| e.ends_with("/ok")));
    assert!(left.iter().any(|e| e.ends_with("/rate")), "429 keeps it");
    assert!(!left.iter().any(|e| e.ends_with("/gone")), "410 removes it");
    assert!(
        !left.iter().any(|e| e.ends_with("/missing")),
        "404 removes it"
    );
    assert!(
        !left.iter().any(|e| e.ends_with("/badkeys")),
        "unusable keys remove it"
    );
    assert!(mock.hits("badkeys").is_empty(), "never sent");

    // The rate-limited endpoint is skipped until Retry-After passes.
    let (_g2, _r2) = crate::pending::raise(&st, permission("s9", "Bash", "y")).await;
    wait_until("second push", || mock.hits("ok").len() == 2).await;
    settle().await;
    assert_eq!(mock.hits("rate").len(), 1);
}

#[tokio::test]
async fn redirects_are_not_followed_and_rejections_accumulate() {
    let (base, mock) = mock_server().await;
    let st = state();
    st.push.use_test_transport();
    subscribe_mock(&st, &base, "d1", "redirect");
    subscribe_mock(&st, &base, "d1", "forbidden");
    for i in 0..MAX_REJECTIONS {
        // /push/test is rate limited per device; reset between rounds.
        lock(&st.push.last_test).clear();
        let ok = send_test(&st, "d1").await.unwrap();
        assert!(!ok, "round {i}");
    }
    assert!(mock.hits("target").is_empty(), "redirect not followed");
    assert_eq!(mock.hits("redirect").len() as u32, MAX_REJECTIONS);
    // After MAX_REJECTIONS consecutive rejections both are dropped.
    assert!(st.push.device_endpoints("d1").is_empty());
}

#[tokio::test]
async fn test_notification_reports_only_ok() {
    let (base, mock) = mock_server().await;
    let st = state();
    st.push.use_test_transport();
    let sk = subscribe_mock(&st, &base, "d1", "ok");
    assert!(send_test(&st, "d1").await.unwrap());
    let v = decrypt(&mock.hits("ok")[0], &sk);
    assert_eq!(v["kind"], "test");
    assert_eq!(v["title"], "Mother Claude");
    assert!(v.get("sessionId").is_none());
    assert_eq!(mock.hits("ok")[0].headers["ttl"], "300");
    // Too soon.
    assert_eq!(send_test(&st, "d1").await.unwrap_err().status.as_u16(), 429);
    // A device whose only endpoint errors: just `false`.
    subscribe_mock(&st, &base, "d2", "err");
    assert!(!send_test(&st, "d2").await.unwrap());
}

#[tokio::test]
async fn unknown_or_revoked_devices_are_dropped_not_notified() {
    let (base, mock) = mock_server().await;
    let st = state();
    // Plain transport but NOT trusting devices: the device store (no devices)
    // decides.
    st.push.install_test_transport("142.250.74.10", true);
    st.push.set_window_ms(100);
    start(&st);
    subscribe_mock(&st, &base, "ghost", "ghost");
    let (_g, _r) = crate::pending::raise(&st, permission("s1", "Bash", "x")).await;
    wait_until("the device to be dropped", || {
        st.push.subscription_count("ghost") == 0
    })
    .await;
    settle().await;
    assert_eq!(mock.total(), 0);
}

#[tokio::test]
async fn turn_finished_threshold_and_one_shot() {
    let (base, mock) = mock_server().await;
    let st = started(100);
    let sk = subscribe_mock(&st, &base, "d1", "p");
    // Short turn, nothing armed: no push.
    on_turn_finished(&st, "s1", 5_000);
    settle().await;
    assert_eq!(mock.total(), 0);
    // Arm the one-shot for s2: a short turn now notifies, once.
    let arm: PrefsPatch =
        serde_json::from_value(serde_json::json!({"turnFinished": {"s2": true}})).unwrap();
    st.push.update_prefs("d1", arm).unwrap();
    on_turn_finished(&st, "s2", 2_000);
    wait_until("one-shot push", || mock.total() == 1).await;
    let v = decrypt(&mock.hits("p")[0], &sk);
    assert_eq!(v["body"], "Finished after 2 s");
    assert!(
        st.push.prefs("d1", true).turn_finished.is_empty(),
        "consumed"
    );
    // A long turn notifies by default.
    on_turn_finished(&st, "s3", 95_000);
    wait_until("long-turn push", || mock.total() == 2).await;
    let v = decrypt(&mock.hits("p")[1], &sk);
    assert_eq!(v["body"], "Finished after 1 min");
    assert_eq!(mock.hits("p")[1].headers["urgency"], "normal");
    // The same finish reported again (e.g. by the bus) is one push.
    on_turn_finished(&st, "s3", 96_000);
    settle().await;
    assert_eq!(mock.total(), 2);
    // Muted sessions stay quiet.
    let mute: PrefsPatch =
        serde_json::from_value(serde_json::json!({"sessions": {"s4": "mute"}})).unwrap();
    st.push.update_prefs("d1", mute).unwrap();
    on_turn_finished(&st, "s4", 120_000);
    on_session_failed(&st, "s4", "boom");
    settle().await;
    assert_eq!(mock.total(), 2);
}

#[tokio::test]
async fn bus_derived_waits_and_claude_notifications() {
    let (base, mock) = mock_server().await;
    let st = started(100);
    let sk = subscribe_mock(&st, &base, "d1", "p");
    // First sight, then a wait nobody raised a card for (no hooks installed).
    session(&st, "s1", "web", SessionStatus::Working).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    crate::server::ws::patch_session(&st, "s1", |s| {
        s.status = SessionStatus::Waiting;
        s.waiting_for = Some("permission prompt".into());
    })
    .await;
    wait_until("waiting push", || mock.total() == 1).await;
    let v = decrypt(&mock.hits("p")[0], &sk);
    assert_eq!(v["title"], "web");
    assert_eq!(v["body"], "Waiting at the Mac · permission prompt");
    assert_eq!(v["kind"], "waiting");

    // Claude called PushNotification (seen in a live transcript append).
    crate::server::ws::publish_transcript(
        &st,
        "s2",
        vec![UiEvent {
            id: "r1:0".into(),
            ts: Some(crate::claude::registry::now_ms()),
            kind: UiKind::Tool,
            tool: Some(UiTool {
                id: "toolu_1".into(),
                name: "PushNotification".into(),
                summary: String::new(),
                input: Some(serde_json::json!({"message": "Deploy done", "status": "proactive"})),
            }),
            ..Default::default()
        }],
        "10".into(),
    );
    wait_until("claude push", || mock.total() == 2).await;
    let v = decrypt(&mock.hits("p")[1], &sk);
    assert_eq!(v["body"], "Claude sent a notification");
    assert_eq!(v["kind"], "claude");
    // The same tool call reported directly (hook / owned stream) is deduped.
    on_claude_notification(&st, "s2", Some("Deploy done"), Some("toolu_1"));
    // Notification hooks: permission_prompt is covered by cards; idle is off
    // by default.
    on_notification_hook(&st, "s2", Some("permission_prompt"), Some("x"));
    on_notification_hook(&st, "s2", Some("idle_prompt"), Some("waiting"));
    settle().await;
    assert_eq!(mock.total(), 2);
}

#[tokio::test]
async fn no_subscriptions_means_no_work() {
    let st = started(100);
    let (_g, _r) = crate::pending::raise(&st, permission("s1", "Bash", "x")).await;
    on_turn_finished(&st, "s1", 100_000);
    settle().await;
    assert!(!st.push.has_subscriptions());
}

#[test]
fn persistence_round_trip_and_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let key = {
        let svc = PushService::new(Some(dir.path()));
        let (_, mut sub) = payload_test_subscription();
        sub.endpoint = "https://fcm.googleapis.com/fcm/send/x".into();
        lock(&svc.data).subscribe("d1", sub, 1);
        svc.persist();
        svc.update_prefs(
            "d1",
            serde_json::from_value(serde_json::json!({"sessions": {"s1": "follow"}})).unwrap(),
        )
        .unwrap();
        svc.public_key().to_string()
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for f in [store::FILE, vapid::FILE] {
            let mode = std::fs::metadata(dir.path().join(f))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{f}");
        }
    }
    let svc = PushService::new(Some(dir.path()));
    assert_eq!(svc.public_key(), key, "same VAPID key after restart");
    assert_eq!(svc.subscription_count("d1"), 1);
    assert_eq!(
        svc.prefs("d1", true).sessions.get("s1").map(String::as_str),
        Some("follow")
    );
    svc.forget_device("d1");
    let svc = PushService::new(Some(dir.path()));
    assert_eq!(svc.subscription_count("d1"), 0, "revocation persisted");

    // A subscription whose endpoint no longer validates is dropped at load.
    let raw = serde_json::json!({
        "version": 1,
        "vapidPublicKey": key,
        "devices": {"d2": {"subscriptions": [
            {"endpoint": "http://127.0.0.1:5612/v1", "p256dh": "x", "auth": "y"}
        ]}}
    });
    std::fs::write(dir.path().join(store::FILE), raw.to_string()).unwrap();
    let svc = PushService::new(Some(dir.path()));
    assert_eq!(svc.subscription_count("d2"), 0);
}

#[test]
fn vapid_subject() {
    assert!(valid_subject("mailto:me@example.com"));
    assert!(!valid_subject("mailto:me@localhost"));
    assert!(!valid_subject("mailto:me@x.invalid"));
    assert!(!valid_subject("mailto:nobody"));
    assert!(valid_subject("https://abc.ngrok-free.app"));
    assert!(!valid_subject("http://abc.ngrok-free.app"));
    assert_eq!(
        https_origin("https://abc.ngrok-free.app/#/pair?c=X").as_deref(),
        Some("https://abc.ngrok-free.app")
    );
    assert_eq!(
        https_origin("https://mac.tail1234.ts.net:8443/x").as_deref(),
        Some("https://mac.tail1234.ts.net:8443")
    );
    assert_eq!(https_origin("https://127.0.0.1/"), None);
    assert_eq!(https_origin("https://localhost/"), None);
    let st = state();
    assert_eq!(st.push.contact(), DEFAULT_CONTACT);
    set_public_url(&st, Some("https://abc.ngrok-free.app/"));
    assert_eq!(st.push.contact(), "https://abc.ngrok-free.app");
    set_public_url(&st, Some("http://192.168.1.2:6725"));
    assert_eq!(st.push.contact(), DEFAULT_CONTACT);
}
