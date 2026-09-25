//! End-to-end integration tests for the adapters and the embedded server.

use std::path::PathBuf;

use futures_util::StreamExt;
use mother_claude_lib::claude::{self, ClaudeHome};
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, monitor};
use mother_claude_lib::state::{Inner, ServerConfig};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn parses_every_transcript_event_type_from_fixture() {
    let events = claude::read_transcript(fixture("transcript_sample.jsonl")).unwrap();
    // Nine lines, all parse (tolerant parser skips none here).
    assert_eq!(events.len(), 9);

    let summary = claude::summarize_transcript("fixture-1", &events);
    assert_eq!(summary.model.as_deref(), Some("claude-opus-4-8"));
    assert_eq!(summary.title.as_deref(), Some("Fixture session"));
    assert_eq!(summary.surface, claude::Surface::VsCode);
    assert_eq!(summary.cwd.as_deref(), Some("/Users/dev/fixture"));
    // Two user + two assistant messages.
    assert_eq!(summary.message_count, 4);
    // 120+35+2000+500 + 80+20 = 2755 tokens across both assistant turns.
    assert_eq!(summary.usage.total_tokens, 2755);
}

#[test]
fn parses_agents_and_roster_fixtures() {
    let agents: Vec<claude::AgentEntry> =
        serde_json::from_str(&std::fs::read_to_string(fixture("agents.json")).unwrap()).unwrap();
    assert_eq!(agents.len(), 2);
    assert_eq!(agents[0].session_id.as_deref(), Some("fixture-1"));
    assert_eq!(agents[1].kind.as_deref(), Some("background"));

    let roster: claude::Roster =
        serde_json::from_str(&std::fs::read_to_string(fixture("roster.json")).unwrap()).unwrap();
    assert_eq!(roster.supervisor_pid, Some(37911));
    assert!(roster.workers.is_empty());
}

/// Read the next JSON text frame (skipping pings), with a timeout.
async fn next_json<S>(ws: &mut S) -> Option<serde_json::Value>
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ok(text) = msg.to_text() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                if v["t"] == "ping" {
                    continue;
                }
                return Some(v);
            }
        }
    }
}

/// Boot the server, open a WebSocket (first-frame auth, no token in the URL),
/// drop a synthetic transcript into a temp CLAUDE_CONFIG_DIR, refresh, and
/// assert the session arrives as a `sessions` diff; then subscribe with a
/// cursor and get the transcript replayed as UI events.
#[tokio::test]
async fn server_broadcasts_session_from_temp_home() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;

    let dir = tempfile::tempdir().unwrap();
    let home = ClaudeHome::with_base(dir.path());

    let state = Inner::new(
        home.clone(),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
    );
    let token = state.auth.token.clone();

    // Serve the router on an ephemeral port.
    let app = server::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    // Connect with no token in the URL and authenticate with the first frame.
    let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    ws.send(Message::Text(
        serde_json::json!({ "t": "auth", "token": token })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    let hello = next_json(&mut ws).await.expect("hello");
    assert_eq!(hello["t"], "hello");
    assert_eq!(hello["v"], 2);
    assert_eq!(hello["me"]["principal"], "desktop");
    assert_eq!(hello["me"]["local"], true);
    let full = next_json(&mut ws).await.expect("full snapshot");
    assert_eq!(full["t"], "sessions");
    assert_eq!(full["full"], true);

    // Drop a synthetic transcript (recent, so it is in the `active` scope).
    let cwd = "/Users/dev/itest";
    let tdir = home.transcript_dir(cwd);
    std::fs::create_dir_all(&tdir).unwrap();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let line = format!(
        "{{\"type\":\"user\",\"uuid\":\"u1\",\"cwd\":\"/Users/dev/itest\",\"timestamp\":\"{now}\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hi\"}}]}}}}\n"
    );
    std::fs::write(tdir.join("itest-session.jsonl"), &line).unwrap();

    // Refresh and look for a sessions diff carrying it.
    let mut found = false;
    'outer: for _ in 0..10 {
        monitor::refresh_once(&state).await;
        while let Some(v) = next_json(&mut ws).await {
            if v["t"] == "sessions" && v["full"] == false {
                let hit = v["upsert"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|s| s["id"] == "itest-session");
                if let Some(s) = hit {
                    assert_eq!(s["cwd"], "/Users/dev/itest");
                    assert_eq!(s["projectName"], "itest");
                    assert!(s["control"]["notes"].is_array());
                    assert!(v["seq"].as_u64().unwrap() > full["seq"].as_u64().unwrap());
                    found = true;
                    break 'outer;
                }
            }
        }
    }
    assert!(found, "expected a sessions diff with the new session");

    // An unchanged refresh sends no diff for it again.
    monitor::refresh_once(&state).await;

    // Subscribe with a cursor: the server replays the file after it.
    ws.send(Message::Text(
        serde_json::json!({ "t": "sub", "session": "itest-session", "since": "0" })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let mut replayed = false;
    while let Some(v) = next_json(&mut ws).await {
        if v["t"] == "sessions" {
            assert!(
                !v["upsert"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|s| s["id"] == "itest-session"),
                "an unchanged session must not be re-sent"
            );
            continue;
        }
        if v["t"] == "transcript" {
            assert_eq!(v["session"], "itest-session");
            assert_eq!(v["events"][0]["kind"], "prompt");
            assert_eq!(v["events"][0]["text"], "hi");
            assert_eq!(v["tail"], line.len().to_string());
            replayed = true;
            break;
        }
    }
    assert!(replayed, "expected a transcript replay");

    // Client ping → server pong.
    ws.send(Message::Text(r#"{"t":"ping"}"#.into()))
        .await
        .unwrap();
    let mut ponged = false;
    while let Some(v) = next_json(&mut ws).await {
        if v["t"] == "pong" {
            ponged = true;
            break;
        }
    }
    assert!(ponged);

    let _ = ws.close(None).await;
}
