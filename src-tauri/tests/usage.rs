//! Plan usage over HTTP: who may read, refresh and configure it, the refresh
//! throttle, and the served shape. Nothing here reads `~/.claude`, spawns
//! `claude` or starts the refresh scheduler.

use std::net::SocketAddr;

use mother_claude_lib::claude::model::{PlanSeverity, PlanUsageSource, PlanWindow};
use mother_claude_lib::claude::usage::{self, UsageData};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::{self, auth::Auth};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

fn state() -> AppState {
    let dir = tempfile::tempdir().unwrap();
    Inner::new(
        ClaudeHome::with_base(dir.keep()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
    )
}

async fn serve(st: &AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::router(st.clone());
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
async fn plan_usage_routes_and_their_guards() {
    let st = state();
    let base = serve(&st).await;
    let desk = st.auth.token.clone();
    let script = st.auth.script_token.clone();
    let c = reqwest::Client::new();
    let now = chrono::Utc::now().timestamp_millis();
    st.usage.set_data_for_test(UsageData {
        windows: vec![
            PlanWindow {
                key: "five_hour".into(),
                label: "Current session".into(),
                pct: 42.0,
                resets_at: Some(now + 3_600_000),
                severity: Some(PlanSeverity::Normal),
            },
            PlanWindow {
                key: "seven_day".into(),
                label: "Weekly · all models".into(),
                pct: 97.0,
                resets_at: Some(now - 1),
                severity: Some(PlanSeverity::Critical),
            },
        ],
        extra_usage: None,
        fetched_at: now - 60_000,
        source: PlanUsageSource::Cache,
    });

    // Read: any credential that may read data; nobody without one.
    let r = c.get(format!("{base}/api/usage")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    for token in [&desk, &script] {
        let r = c
            .get(format!("{base}/api/usage"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["available"], true);
        assert_eq!(v["source"], "cache");
        assert_eq!(v["intervalSecs"], 300);
        assert_eq!(v["refreshing"], false);
        assert_eq!(v["windows"][0]["pct"], 42.0);
        // A window past its reset shows 0 %.
        assert_eq!(v["windows"][1]["pct"], 0.0);
        assert!(v["windows"][1].get("severity").is_none());
    }

    // Settings: the desktop only, and only the offered intervals.
    let put = |token: &str, body: Value| {
        c.put(format!("{base}/api/usage/settings"))
            .bearer_auth(token)
            .json(&body)
            .send()
    };
    let r = put(&script, json!({ "intervalSecs": 180 })).await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "desktop_only"
    );
    for bad in [
        json!({ "intervalSecs": 120 }),
        json!({ "intervalSecs": -1 }),
        json!({ "intervalSecs": "300" }),
        json!({}),
    ] {
        let r = put(&desk, bad.clone()).await.unwrap();
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let mut rx = st.bus.subscribe();
    let r = put(&desk, json!({ "intervalSecs": 0 })).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.json::<Value>().await.unwrap()["intervalSecs"], 0);
    assert_eq!(usage::current(&st).interval_secs, 0);
    // Everyone hears about it.
    let ev = rx.recv().await.unwrap();
    let frame = serde_json::to_value(&*ev).unwrap();
    assert_eq!(frame["t"], "planUsage");
    assert_eq!(frame["usage"]["intervalSecs"], 0);

    // Refresh: 202 with `refreshing`, then 429 with Retry-After for a
    // minute.
    let r = c
        .post(format!("{base}/api/usage/refresh"))
        .bearer_auth(&script)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    assert_eq!(r.json::<Value>().await.unwrap()["refreshing"], true);
    let r = c
        .post(format!("{base}/api/usage/refresh"))
        .bearer_auth(&desk)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = r.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry));
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "rate_limited");
    let r = c
        .post(format!("{base}/api/usage/refresh"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

/// One live read through the INSTALLED CLI with the plan-usage helper
/// (`initialize` + `get_usage` in a temp dir: no model call, $0). Run with
/// `cargo test --test usage -- --ignored live_`.
#[tokio::test]
#[ignore]
async fn live_helper_reads_the_plan_usage() {
    let dir = tempfile::tempdir().unwrap();
    let helper = usage::helper_dir(dir.path());
    let started = std::time::Instant::now();
    let resp = usage::helper_read(&mother_claude_lib::claude::claude_bin(), &helper)
        .await
        .expect("get_usage answered");
    let took = started.elapsed();
    match usage::parse_get_usage(&resp).expect("a usable answer") {
        usage::UsageAnswer::Unavailable => eprintln!("no plan usage for this sign-in"),
        usage::UsageAnswer::Data {
            windows,
            extra_usage,
        } => {
            assert!(!windows.is_empty());
            assert_eq!(windows[0].key, "five_hour");
            for w in &windows {
                eprintln!(
                    "window {} ({}), reset known: {}",
                    w.key,
                    w.label,
                    w.resets_at.is_some()
                );
            }
            eprintln!("extra usage reported: {}", extra_usage.is_some());
        }
    }
    eprintln!("helper read took {} ms", took.as_millis());
    assert!(
        took < std::time::Duration::from_secs(20),
        "killed, not awaited"
    );
    // It leaves nothing behind in its directory.
    assert!(std::fs::read_dir(&helper).unwrap().next().is_none());
}
