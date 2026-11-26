//! Pinned sessions (`server::pins`, `GET|PUT|DELETE /api/pins`): pinned in
//! the order they were pinned, told to every client, with this computer's
//! pinned sessions as the list knows them (older ones too); only session
//! ids; a token needed. Nothing touches `~/.claude` or the user's ports.

use std::net::SocketAddr;

use mother_claude_lib::claude::model::SessionSummary;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::ws::ServerEvent;
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

const A: &str = "0b9f3c1e-2d4a-4f6b-8c7d-9e0f1a2b3c4d";
const B: &str = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
const MAC: &str = "6c229c70ddb9ad261a5e15885f45f62a";

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

#[tokio::test]
async fn sessions_are_pinned_in_order_and_every_client_is_told() {
    let st = state();
    // A session the list's own frames no longer carry (not active lately):
    // its summary comes with the pins.
    st.snapshot.write().await.sessions = vec![SessionSummary {
        id: A.into(),
        name: "auth".into(),
        ..Default::default()
    }];
    let base = serve(server::router(st.clone())).await;
    let c = reqwest::Client::new();
    let token = st.auth.token.clone();
    let mut bus = st.bus.subscribe();
    let mut told =
        move || std::iter::from_fn(|| bus.try_recv().ok()).any(|e| matches!(*e, ServerEvent::Pins));

    let r = c
        .get(format!("{base}/api/pins"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"pins": [], "sessions": []})
    );

    let put = |path: String| {
        let (c, token) = (c.clone(), token.clone());
        async move { c.put(path).bearer_auth(token).send().await.unwrap() }
    };
    let v: Value = put(format!("{base}/api/pins/{A}"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["pins"][0]["id"], A);
    assert!(v["pins"][0]["at"].as_i64().unwrap() > 0);
    assert_eq!(v["sessions"][0]["name"], "auth");
    assert!(told(), "every client hears {{t:\"pins\"}}");

    // Another computer's session: its own pin, after the first; no summary
    // here (that computer's list has it).
    let v: Value = put(format!("{base}/api/pins/{B}?machine={MAC}"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["pins"].as_array().unwrap().len(), 2);
    assert_eq!(
        (v["pins"][1]["id"].clone(), v["pins"][1]["machine"].clone()),
        (json!(B), json!(MAC))
    );
    assert_eq!(v["sessions"].as_array().unwrap().len(), 1);
    assert!(told());

    // Pinned already: unchanged, nobody told.
    let v: Value = put(format!("{base}/api/pins/{A}"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["pins"][0]["id"], A);
    assert!(!told());

    // Only session ids, only machine ids.
    for path in [
        format!("{base}/api/pins/s-auth"),
        format!("{base}/api/pins/{A}?machine=..%2Fx"),
    ] {
        let r = put(path.clone()).await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        let e: Value = r.json().await.unwrap();
        assert_eq!(e["error"]["code"], "invalid_session_id");
    }

    // Unpinned.
    let v: Value = c
        .delete(format!("{base}/api/pins/{A}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["pins"].as_array().unwrap().len(), 1);
    assert_eq!(v["pins"][0]["id"], B);
    assert!(v["sessions"].as_array().unwrap().is_empty());
    assert!(told());

    // Without a token: no.
    let r = c.get(format!("{base}/api/pins")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r = c.put(format!("{base}/api/pins/{A}")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}
