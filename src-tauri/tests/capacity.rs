//! The scheduler routes over real HTTP (docs/ROUTING.md): every route needs
//! a credential; reads take any signed-in principal; settings take the
//! desktop or a paired device (never the local-script token); the routing
//! policy is edited on the desktop only. Nothing here touches `~/.claude`,
//! the Keychain or a real Claude.

use std::net::SocketAddr;

use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self};
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

const READS: [&str; 5] = [
    "/api/host",
    "/api/capacity",
    "/api/routing/policy",
    "/api/routing/report",
    "/api/routing/report.csv",
];

#[tokio::test]
async fn every_scheduler_route_needs_the_right_principal() {
    let (base, st) = boot().await;
    let c = reqwest::Client::new();
    let desktop = st.auth.token.clone();
    let script = st.auth.script_token.clone();
    let hook = st.auth.hook_token.clone();

    // No credential: 401 everywhere, with the envelope.
    for path in READS {
        let r = c.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    for (method, path) in [
        (reqwest::Method::PUT, "/api/capacity/settings"),
        (reqwest::Method::PUT, "/api/routing/policy"),
        (reqwest::Method::DELETE, "/api/routing/policy"),
        (reqwest::Method::POST, "/api/routing/classify"),
    ] {
        let r = c
            .request(method, format!("{base}{path}"))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    // The hook credential reaches none of them.
    let r = c
        .get(format!("{base}/api/capacity"))
        .bearer_auth(&hook)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    // The desktop and a local script read everything.
    for token in [&desktop, &script] {
        for path in READS {
            let r = c
                .get(format!("{base}{path}"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::OK, "{path}");
        }
    }
    let v: Value = c
        .get(format!("{base}/api/capacity"))
        .bearer_auth(&desktop)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["caps"]["userCap"], 3);
    assert_eq!(v["canEditPolicy"], true);
    let csv = c
        .get(format!("{base}/api/routing/report.csv"))
        .bearer_auth(&script)
        .send()
        .await
        .unwrap();
    assert_eq!(
        csv.headers()["content-type"].to_str().unwrap(),
        "text/csv; charset=utf-8"
    );
    assert!(csv.text().await.unwrap().starts_with("class,model,effort,"));

    // A local script may not change settings or the policy.
    let r = c
        .put(format!("{base}/api/capacity/settings"))
        .bearer_auth(&script)
        .json(&json!({ "userCap": 4 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = c
        .delete(format!("{base}/api/routing/policy"))
        .bearer_auth(&script)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    // The desktop may; a bad value is a 422 naming its field.
    let r = c
        .put(format!("{base}/api/capacity/settings"))
        .bearer_auth(&desktop)
        .json(&json!({ "userCap": 4 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(st.scheduler.limits().user_cap, 4);
    let r = c
        .put(format!("{base}/api/capacity/settings"))
        .bearer_auth(&desktop)
        .json(&json!({ "budgets": { "nearPct": 120 } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let e: Value = r.json().await.unwrap();
    assert_eq!(e["error"]["field"], "budgets.nearPct");
    let r = c
        .delete(format!("{base}/api/routing/policy"))
        .bearer_auth(&desktop)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = c
        .post(format!("{base}/api/routing/classify"))
        .bearer_auth(&script)
        .json(&json!({ "title": "Summarise the incidents" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.json::<Value>().await.unwrap()["class"], "summarize");
}

#[tokio::test]
async fn the_desktop_token_does_not_work_through_a_tunnel() {
    let (base, st) = boot().await;
    let r = reqwest::Client::new()
        .put(format!("{base}/api/capacity/settings"))
        .bearer_auth(&st.auth.token)
        .header("x-forwarded-for", "203.0.113.9")
        .json(&json!({ "userCap": 9 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(st.scheduler.limits().user_cap, 3, "unchanged");
}
