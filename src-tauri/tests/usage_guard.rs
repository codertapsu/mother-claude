//! The usage guard (Settings › Usage limits, `claude::usage_guard`): its
//! routes, and a plan window at the threshold pausing its account — once per
//! reset, resumed by hand, never paused again before that reset. No session
//! runs and nothing touches `~/.claude`, the Keychain or the user's ports.

use std::net::SocketAddr;

use mother_claude_lib::claude::usage::{self, StreamUpdate};
use mother_claude_lib::claude::usage_guard::{self, Ended, SettingsPatch};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

const HOUR: i64 = 3_600_000;

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

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The 5-hour window of the Default account at `pct`, resetting at `resets`
/// (what an owned session's `rate_limit_event` says).
fn five_hour_at(st: &AppState, pct: f64, resets: i64) {
    usage::merge_stream_for(
        st,
        "default",
        StreamUpdate {
            windows: vec![("five_hour".into(), pct, Some(resets))],
            rejected: false,
        },
    );
}

#[tokio::test]
async fn the_routes_read_and_change_the_settings() {
    let st = state();
    let base = serve(server::router(st.clone())).await;
    let c = reqwest::Client::new();
    let token = st.auth.token.clone();

    let r = c
        .get(format!("{base}/api/usage-guard"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        v["settings"],
        json!({"enabled": true, "thresholdPct": 90, "autoResume": true, "bufferSecs": 60,
               "includeElsewhere": true})
    );
    assert_eq!(v["pauses"], json!([]));

    let put = |body: Value| {
        let (c, base, token) = (c.clone(), base.clone(), token.clone());
        async move {
            c.put(format!("{base}/api/usage-guard/settings"))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await
                .unwrap()
        }
    };
    let r = put(json!({"thresholdPct": 85, "autoResume": false})).await;
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["settings"]["thresholdPct"], 85);
    assert_eq!(v["settings"]["autoResume"], false);
    assert_eq!(v["settings"]["enabled"], true);
    // Out of range, or a field it does not know: refused, nothing changes.
    let r = put(json!({"thresholdPct": 120})).await;
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let e: Value = r.json().await.unwrap();
    assert_eq!(e["error"]["code"], "invalid_settings");
    assert_ne!(put(json!({"threshold": 80})).await.status(), StatusCode::OK);
    assert_eq!(usage_guard::view(&st).settings.threshold_pct, 85);

    // Nothing paused: nothing to resume, hold or dismiss.
    let r = c
        .post(format!("{base}/api/usage-guard/resume"))
        .bearer_auth(&token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    for path in ["up_0123456789ab", "nope"] {
        let r = c
            .delete(format!("{base}/api/usage-guard/pauses/{path}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{path}");
    }
    // Without a token: no.
    let r = c
        .get(format!("{base}/api/usage-guard"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_window_at_the_threshold_pauses_once_and_resumes_by_hand() {
    let st = state();
    let resets = now() + HOUR;
    five_hour_at(&st, 95.0, resets);
    usage_guard::tick(&st).await;
    let v = usage_guard::view(&st);
    assert_eq!(v.pauses.len(), 1);
    let p = &v.pauses[0];
    assert!(p.active());
    assert_eq!(
        (p.window.as_str(), p.pct, p.threshold_pct),
        ("five_hour", 95.0, 90)
    );
    assert_eq!(p.resets_at, resets);
    // The timer: the reset plus a minute.
    assert_eq!(p.resume_at, resets + 60_000);
    assert!(p.sessions.is_empty(), "no session ran");
    // Teams start nothing on that account meanwhile.
    let why = usage_guard::admission_reason(&st, "default", "opus", now()).unwrap();
    assert!(why.contains("95 % of the 5-hour limit"), "{why}");
    assert!(why.contains("it resumes in 1 h"), "{why}");
    assert!(usage_guard::admission_reason(&st, "acc_other", "opus", now()).is_none());

    // Another look: still the one pause.
    usage_guard::tick(&st).await;
    assert_eq!(usage_guard::active_count(&st), 1);

    // Don't resume, then Resume now: ended by hand, and the full window does
    // not pause again before it resets.
    let id = p.id.clone();
    usage_guard::set_hold(&st, &id, true).unwrap();
    assert!(usage_guard::view(&st).pauses[0].hold);
    usage_guard::resume_now(&st, Some(&id)).await.unwrap();
    let v = usage_guard::view(&st);
    assert_eq!(v.pauses[0].ended, Some(Ended::ResumedByHand));
    usage_guard::tick(&st).await;
    assert_eq!(usage_guard::active_count(&st), 0);
    assert!(usage_guard::admission_reason(&st, "default", "opus", now()).is_none());

    // The next window (a later reset) pauses again.
    five_hour_at(&st, 93.0, resets + 5 * HOUR);
    usage_guard::tick(&st).await;
    assert_eq!(usage_guard::active_count(&st), 1);
    // Dismissed: forgotten without resuming, and not paused again either.
    let id = usage_guard::view(&st).pauses[0].id.clone();
    usage_guard::dismiss(&st, &id).unwrap();
    usage_guard::tick(&st).await;
    assert_eq!(usage_guard::active_count(&st), 0);
}

#[tokio::test]
async fn below_the_threshold_or_off_nothing_pauses() {
    let st = state();
    five_hour_at(&st, 85.0, now() + HOUR);
    usage_guard::tick(&st).await;
    assert_eq!(usage_guard::active_count(&st), 0);
    // Off: not even at 99 %.
    usage_guard::set_settings(
        &st,
        &SettingsPatch {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    five_hour_at(&st, 99.0, now() + HOUR);
    usage_guard::tick(&st).await;
    assert_eq!(usage_guard::active_count(&st), 0);
    // A lower threshold, on again: 85 % pauses.
    usage_guard::set_settings(
        &st,
        &SettingsPatch {
            enabled: Some(true),
            threshold_pct: Some(80),
            ..Default::default()
        },
    )
    .unwrap();
    five_hour_at(&st, 85.0, now() + HOUR);
    usage_guard::tick(&st).await;
    assert_eq!(usage_guard::active_count(&st), 1);
}
