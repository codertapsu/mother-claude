//! Web Push routes over real HTTP: guards, the VAPID key, and the desktop's
//! (device-less) view. Device-side subscribe/prefs/test and the delivery
//! pipeline are covered by the unit tests in `server/push` (they need a paired
//! device and a mock push service). Nothing here touches the network beyond
//! 127.0.0.1.

use std::net::SocketAddr;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

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

async fn boot() -> (String, AppState) {
    let st = state();
    let app = server::router(st.clone());
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
    (format!("http://{addr}"), st)
}

#[tokio::test]
async fn push_routes_are_guarded_and_the_desktop_cannot_subscribe() {
    let (base, st) = boot().await;
    let c = reqwest::Client::new();
    let master = st.auth.token.clone();

    // Anonymous: 401 with the envelope; the hook token is not a user.
    let r = c
        .get(format!("{base}/api/push/vapid"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r = c
        .get(format!("{base}/api/push/vapid"))
        .bearer_auth(&st.auth.hook_token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    // The desktop reads the key and the hints…
    let v: Value = c
        .get(format!("{base}/api/push/vapid"))
        .bearer_auth(&master)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = URL_SAFE_NO_PAD
        .decode(v["publicKey"].as_str().unwrap())
        .unwrap();
    assert_eq!(key.len(), 65);
    assert_eq!(key[0], 4);
    assert_eq!(v["available"], false);
    assert_eq!(v["secureContextHint"]["code"], "desktop");
    assert_eq!(v["publicKey"], st.push.public_key());

    // …and default prefs flagged unavailable…
    let v: Value = c
        .get(format!("{base}/api/push/prefs"))
        .bearer_auth(&master)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["available"], false);
    assert_eq!(v["permission"], true);
    assert_eq!(v["idle"], false);
    assert_eq!(v["showDetails"], false);

    // …but has no device id, so it cannot subscribe, test or change prefs.
    let sub = json!({
        "endpoint": "https://fcm.googleapis.com/fcm/send/abc",
        "keys": {"p256dh": "x", "auth": "y"}
    });
    let r = c
        .post(format!("{base}/api/push/subscribe"))
        .bearer_auth(&master)
        .json(&sub)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "push_device_only"
    );
    for (method, path) in [
        (reqwest::Method::DELETE, "/api/push/subscribe"),
        (reqwest::Method::POST, "/api/push/test"),
        (reqwest::Method::PUT, "/api/push/prefs"),
    ] {
        let r = c
            .request(method, format!("{base}{path}"))
            .bearer_auth(&master)
            .json(&json!({"endpoint": "https://fcm.googleapis.com/x"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{path}");
    }
    assert!(!st.push.has_subscriptions());

    // A malformed body is a 422 envelope, not a crash.
    let r = c
        .put(format!("{base}/api/push/prefs"))
        .bearer_auth(&master)
        .header("content-type", "application/json")
        .body("{\"permission\": \"yes\"")
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error());
}
