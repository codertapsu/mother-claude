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

/// Boot the server, open a WebSocket, drop a synthetic transcript into a temp
/// CLAUDE_CONFIG_DIR, refresh, and assert the session is broadcast to the client.
#[tokio::test]
async fn server_broadcasts_session_from_temp_home() {
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

    // Connect a WebSocket client (token via query string).
    let ws_url = format!("ws://{addr}/ws?token={token}");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(ws_url).await.unwrap();

    // First message is the (empty) initial snapshot.
    let first = ws.next().await.unwrap().unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(snapshot["kind"], "sessions");

    // Drop a synthetic transcript into the temp home.
    let cwd = "/Users/dev/itest";
    let tdir = home.transcript_dir(cwd);
    std::fs::create_dir_all(&tdir).unwrap();
    std::fs::write(
        tdir.join("itest-session.jsonl"),
        "{\"type\":\"user\",\"cwd\":\"/Users/dev/itest\",\"timestamp\":\"2026-06-23T08:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
    )
    .unwrap();

    // Refresh and look for a non-empty sessions broadcast.
    let mut found = false;
    'outer: for _ in 0..20 {
        monitor::refresh_once(&state).await;
        for _ in 0..5 {
            match tokio::time::timeout(std::time::Duration::from_millis(200), ws.next()).await {
                Ok(Some(Ok(msg))) => {
                    if let Ok(text) = msg.to_text() {
                        let v: serde_json::Value = match serde_json::from_str(text) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        if v["kind"] == "sessions"
                            && v["data"].as_array().map(|a| !a.is_empty()).unwrap_or(false)
                        {
                            assert_eq!(v["data"][0]["id"], "itest-session");
                            assert_eq!(v["data"][0]["cwd"], "/Users/dev/itest");
                            found = true;
                            break 'outer;
                        }
                    }
                }
                _ => break,
            }
        }
    }
    assert!(found, "expected a non-empty sessions broadcast over WS");

    let _ = ws.close(None).await;
}
