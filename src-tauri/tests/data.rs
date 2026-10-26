//! The data layer end to end over real HTTP/WebSocket, with a synthetic
//! `~/.claude` in a temp dir: live roster from pidfiles, the incremental
//! index, cost/usage (dedup, cost-state, subagents), agents and workflows,
//! observed pending items, transcript paging/normalisation, media, task
//! output, id validation (`..%2F` traversal), the hook body limit, and the WS
//! hub (agent subscriptions, appends, bounded replay).

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, monitor};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

const LIVE: &str = "11111111-2222-4333-8444-555555555555";
const ENDED: &str = "66666666-7777-4888-8999-aaaaaaaaaaaa";
const PNG: [u8; 12] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn lines(v: &[Value]) -> String {
    v.iter().map(|l| l.to_string() + "\n").collect()
}

/// Removes the task-output fixture dir even when a test panics.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        // The per-user root, when we created it and it is now empty.
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// Build the synthetic home; the returned guard removes the task-output
/// fixture (it lives under the CLI's per-user temp root).
fn fixture(base: &Path) -> Cleanup {
    let me = std::process::id();
    let t0 = now() - 60_000;
    let proj = base.join("projects/-tmp-proj");
    let sdir = proj.join(LIVE);
    std::fs::create_dir_all(sdir.join("subagents/workflows/wf_run-9")).unwrap();
    std::fs::create_dir_all(base.join("sessions")).unwrap();

    // Task output under the CLI's temp root (the per-user dir in $TMPDIR).
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let out_dir = std::env::temp_dir()
        .join(format!("claude-{uid}"))
        .join(format!("mc-data-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(out_dir.join("tasks")).unwrap();
    let out_file = out_dir.join("tasks/bsh1.output");
    std::fs::write(&out_file, "compiling…\nready on :3000\n").unwrap();
    let outside = base.join("secret.output");
    std::fs::write(&outside, "SECRET").unwrap();

    // Live pidfile: this test process, waiting on a permission prompt.
    std::fs::write(
        base.join(format!("sessions/{me}.json")),
        json!({
            "pid": me, "sessionId": LIVE, "cwd": "/tmp/proj", "startedAt": t0 - 1000,
            "kind": "interactive", "entrypoint": "claude-vscode", "version": "2.1.282",
            "name": "proj-12", "nameSource": "derived",
            "status": "waiting", "waitingFor": "permission prompt", "statusUpdatedAt": t0 + 50_000
        })
        .to_string(),
    )
    .unwrap();

    let b64 = base64::engine::general_purpose::STANDARD.encode(PNG);
    let svg = base64::engine::general_purpose::STANDARD.encode(b"<svg onload=alert(1)>");
    let usage = |i: u64, o: u64| json!({"input_tokens": i, "output_tokens": o, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0});
    let main = lines(&[
        json!({"type":"user","uuid":"u-1","timestamp":iso(t0),"cwd":"/tmp/proj","gitBranch":"feat/x","permissionMode":"default","origin":{"kind":"human"},"message":{"role":"user","content":"please build it"}}),
        json!({"type":"attachment","uuid":"at-1","attachment":{"type":"model","identity":{"modelId":"claude-haiku-4-5[1m]"}}}),
        json!({"type":"user","uuid":"img-1","timestamp":iso(t0+100),"message":{"role":"user","content":[
            {"type":"text","text":"screenshot"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":b64}},
            {"type":"image","source":{"type":"base64","media_type":"image/svg+xml","data":svg}}
        ]}}),
        // One API message split over three records (partial usage) ⇒ counted once, last usage.
        // Every record of a request carries its effort, as the CLI writes them.
        json!({"type":"assistant","uuid":"a-1","timestamp":iso(t0+1000),"effort":"high","message":{"id":"msg_A","model":"claude-haiku-4-5-20251001","content":[{"type":"thinking","thinking":"hmm"}],"usage":usage(100, 8)}}),
        json!({"type":"assistant","uuid":"a-2","timestamp":iso(t0+1100),"effort":"high","message":{"id":"msg_A","model":"claude-haiku-4-5-20251001","content":[{"type":"tool_use","id":"tu_agent","name":"Agent","input":{"description":"Explore the repo","subagent_type":"Explore"}}],"usage":usage(100, 50)}}),
        json!({"type":"assistant","uuid":"a-3","timestamp":iso(t0+1200),"effort":"high","message":{"id":"msg_A","model":"claude-haiku-4-5-20251001","stop_reason":"tool_use","content":[{"type":"tool_use","id":"tu_sh","name":"Bash","input":{"command":"npm run dev","description":"Start dev server","run_in_background":true}}],"usage":usage(100, 200)}}),
        json!({"type":"user","uuid":"r-1","timestamp":iso(t0+1300),"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_sh","content":"started"}]},"toolUseResult":{"backgroundTaskId":"bsh1"}}),
        json!({"type":"attachment","uuid":"at-2","timestamp":iso(t0+1301),"attachment":{"type":"task_status","taskId":"bsh1","taskType":"local_bash","description":"Start dev server","status":"running","outputFilePath":out_file.to_string_lossy()}}),
        json!({"type":"attachment","uuid":"at-3","timestamp":iso(t0+1302),"attachment":{"type":"task_status","taskId":"bout","taskType":"local_bash","description":"Escapes","status":"running","outputFilePath":outside.to_string_lossy()}}),
        json!({"type":"assistant","uuid":"a-4","timestamp":iso(t0+2000),"effort":"high","message":{"id":"msg_B","model":"claude-haiku-4-5-20251001","stop_reason":"tool_use","content":[
            {"type":"tool_use","id":"tu_todo","name":"TodoWrite","input":{"todos":[{"content":"a","status":"completed"},{"content":"b","status":"in_progress","activeForm":"Building b"}]}},
            {"type":"tool_use","id":"tu_edit","name":"Edit","input":{"file_path":"/tmp/proj/src/a.rs","old_string":"x","new_string":"y"}}
        ],"usage":usage(10, 20)}}),
        json!({"type":"user","uuid":"r-2","timestamp":iso(t0+2100),"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_edit","content":"ok"}]},"toolUseResult":{"filePath":"/tmp/proj/src/a.rs","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":2,"lines":["-x","+y","+z"]}]}}),
        json!({"type":"assistant","uuid":"a-5","timestamp":iso(t0+3000),"effort":"high","message":{"id":"msg_C","model":"claude-haiku-4-5-20251001","stop_reason":"tool_use","content":[{"type":"tool_use","id":"tu_rm","name":"Bash","input":{"command":"rm -rf build","description":"Clean build"}}],"usage":usage(5, 5)}}),
        json!({"type":"last-prompt","lastPrompt":"please build it"}),
        json!({"type":"ai-title","aiTitle":"Build the thing"}),
        json!({"type":"permission-mode","permissionMode":"acceptEdits"}),
        json!({"type":"attachment","uuid":"noise","attachment":{"type":"hook_non_blocking_error","stderr":"ECONNREFUSED"}}),
    ]);
    std::fs::write(proj.join(format!("{LIVE}.jsonl")), main).unwrap();

    // Running foreground subagent (its Agent call has no result yet).
    let sub = sdir.join("subagents");
    std::fs::write(
        sub.join("agent-aexp1.meta.json"),
        json!({"agentType":"Explore","description":"Explore the repo","toolUseId":"tu_agent","spawnDepth":1,"requestShape":"foreground"}).to_string(),
    )
    .unwrap();
    std::fs::write(
        sub.join("agent-aexp1.jsonl"),
        lines(&[
            json!({"type":"assistant","isSidechain":true,"agentId":"aexp1","uuid":"s-1","timestamp":iso(now()-2000),"message":{"id":"msg_S","model":"claude-haiku-4-5-20251001","content":[{"type":"tool_use","id":"stu1","name":"Grep","input":{"pattern":"TODO"}}],"usage":{"input_tokens":7,"output_tokens":3}}}),
        ]),
    )
    .unwrap();
    // Workflow: 2 started, 1 result ⇒ 1 running.
    let wf = sub.join("workflows/wf_run-9");
    std::fs::write(
        wf.join("journal.jsonl"),
        lines(&[
            json!({"type":"launched"}),
            json!({"type":"started","key":"k1","agentId":"aw1","label":"plan","phase":"Plan"}),
            json!({"type":"started","key":"k2","agentId":"aw2","label":"build","phase":"Build"}),
            json!({"type":"result","key":"k1","agentId":"aw1","result":"done"}),
        ]),
    )
    .unwrap();

    // Ended session: cost-state, then usage after it (main + subagent).
    let ended = proj.join(ENDED);
    std::fs::create_dir_all(ended.join("subagents")).unwrap();
    let e0 = now() - 3_600_000;
    std::fs::write(
        proj.join(format!("{ENDED}.jsonl")),
        lines(&[
            json!({"type":"user","uuid":"e-u1","timestamp":iso(e0),"cwd":"/tmp/proj","message":{"content":"hi"}}),
            json!({"type":"assistant","uuid":"e-a1","timestamp":iso(e0+1000),"message":{"id":"msg_E1","model":"claude-haiku-4-5","content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":1000000,"output_tokens":0}}}),
            json!({"type":"cost-state","totalCostUSD":2.5}),
            // Queued, never dequeued: nothing is queued once the process is gone.
            json!({"type":"queue-operation","operation":"enqueue","timestamp":iso(e0+2000),"content":"and then this"}),
            json!({"type":"assistant","uuid":"e-a2","timestamp":iso(e0+5000),"message":{"id":"msg_E2","model":"claude-haiku-4-5","content":[{"type":"text","text":"again"}],"usage":{"input_tokens":0,"output_tokens":200000}}}),
        ]),
    )
    .unwrap();
    std::fs::write(
        ended.join("subagents/agent-aold.jsonl"),
        lines(&[
            // Before the cost-state (already in its total): not added again.
            json!({"type":"assistant","isSidechain":true,"timestamp":iso(e0+500),"message":{"id":"msg_SB","model":"claude-haiku-4-5","content":[{"type":"text","text":"x"}],"usage":{"input_tokens":1000000,"output_tokens":0}}}),
            // After it: added.
            json!({"type":"assistant","isSidechain":true,"timestamp":iso(e0+6000),"message":{"id":"msg_SA","model":"claude-haiku-4-5","content":[{"type":"text","text":"y"}],"usage":{"input_tokens":0,"output_tokens":100000}}}),
        ]),
    )
    .unwrap();
    Cleanup(out_dir)
}

fn state(base: &Path) -> AppState {
    Inner::new(
        ClaudeHome::with_base(base),
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

async fn get(c: &reqwest::Client, url: String, token: &str) -> reqwest::Response {
    c.get(url).bearer_auth(token).send().await.unwrap()
}

async fn get_json(c: &reqwest::Client, url: String, token: &str) -> Value {
    let r = get(c, url, token).await;
    assert_eq!(r.status(), StatusCode::OK);
    r.json().await.unwrap()
}

#[tokio::test]
async fn data_routes_over_a_synthetic_home() {
    let dir = tempfile::tempdir().unwrap();
    let _cleanup = fixture(dir.path());
    let st = state(dir.path());
    monitor::refresh_once(&st).await;
    let base = serve(server::router(st.clone())).await;
    let token = st.auth.token.clone();
    let c = reqwest::Client::new();

    // ── list ──
    let v = get_json(&c, format!("{base}/api/sessions?scope=all"), &token).await;
    let sessions = v["sessions"].as_array().unwrap();
    let live = sessions
        .iter()
        .find(|s| s["id"] == LIVE)
        .expect("live session");
    assert_eq!(live["live"], true);
    assert_eq!(live["status"], "waiting");
    assert_eq!(live["waitingFor"], "permission prompt");
    assert_eq!(live["surface"], "vscode");
    assert_eq!(live["name"], "Build the thing");
    assert_eq!(live["cliName"], "proj-12");
    assert_eq!(live["gitBranch"], "feat/x");
    assert_eq!(live["model"]["id"], "claude-haiku-4-5[1m]");
    assert_eq!(live["model"]["contextWindow"], 1_000_000);
    assert_eq!(live["effort"], "high");
    // The permission-mode record overrides the prompt's mode.
    assert_eq!(live["permissionMode"], "acceptEdits");
    // msg_A counted once with its LAST usage (in 100 / out 200).
    assert_eq!(live["usage"]["output"], 200 + 20 + 5 + 3);
    assert_eq!(live["usage"]["input"], 100 + 10 + 5 + 7);
    assert!(live["costUsd"].as_f64().unwrap() > 0.0);
    assert_eq!(live["context"]["used"], 1005);
    assert_eq!(live["todos"]["total"], 2);
    assert_eq!(live["todos"]["activeForm"], "Building b");
    assert_eq!(live["lastPrompt"], "please build it");
    // Observed (unanswerable) pending item: the waiting tool, as a preview.
    let p = &live["pending"][0];
    assert_eq!(p["source"], "observed");
    assert_eq!(p["answerable"], false);
    assert_eq!(p["toolName"], "Bash");
    assert_eq!(p["detail"], "Clean build");
    assert!(p.get("input").is_none(), "previews never carry the input");
    assert_eq!(live["activity"]["label"], "Waiting for approval");
    assert_eq!(live["activity"]["tool"], "Bash");
    // Agents: the running subagent + the running workflow agent; the shell.
    assert_eq!(live["agents"]["running"], 2);
    assert_eq!(live["agents"]["workflowsRunning"], 1);
    assert_eq!(live["agents"]["backgroundRunning"], 2);
    assert!(live["control"]["limits"].is_array());

    // Ended session: cost-state + later usage (main by position, subagent by
    // timestamp). haiku-4-5: $1 in / $5 out per MTok.
    let ended = sessions.iter().find(|s| s["id"] == ENDED).unwrap();
    assert_eq!(ended["live"], false);
    assert_eq!(ended["status"], "ended");
    let cost = ended["costUsd"].as_f64().unwrap();
    let expect = 2.5 + 200_000.0 * 5.0 / 1e6 + 100_000.0 * 5.0 / 1e6;
    assert!((cost - expect).abs() < 1e-9, "cost {cost} != {expect}");
    assert!(
        ended.get("costFinal").is_none(),
        "usage after the last cost-state"
    );
    assert_eq!(ended["queued"], 0);
    // Cost today is what was priced since local midnight, never the lifetime
    // total (the cost-state baseline cannot be split by day).
    let midnight = mother_claude_lib::claude::summaries::local_midnight_ms(now());
    if now() - 3_700_000 > midnight {
        let today = ended["costToday"].as_f64().unwrap();
        let priced = (1_000_000.0 + 200_000.0 * 5.0 + 1_000_000.0 + 100_000.0 * 5.0) / 1e6;
        assert!((today - priced).abs() < 1e-6, "today {today} != {priced}");
        assert!(today < cost);
        let live_today = live["costToday"].as_f64().unwrap();
        assert!((live_today - live["costUsd"].as_f64().unwrap()).abs() < 1e-6);
    }

    // Active scope keeps both (live, recent).
    let v = get_json(&c, format!("{base}/api/sessions"), &token).await;
    assert!(v["sessions"].as_array().unwrap().len() >= 2);

    // ── detail ──
    let d = get_json(&c, format!("{base}/api/sessions/{LIVE}"), &token).await;
    let agents = d["agentsDetail"].as_array().unwrap();
    let sub = agents.iter().find(|a| a["id"] == "aexp1").unwrap();
    assert_eq!(sub["status"], "running");
    assert_eq!(sub["type"], "Explore");
    assert_eq!(sub["tokens"], 10);
    assert_eq!(sub["activity"]["label"], "Running Grep");
    assert!(agents
        .iter()
        .any(|a| a["id"] == "aw2" && a["status"] == "running"));
    assert!(agents
        .iter()
        .any(|a| a["id"] == "aw1" && a["status"] == "done"));
    assert_eq!(d["workflows"][0]["agents"]["running"], 1);
    assert!(d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["id"] == "bsh1" && t["outputAvailable"] == true));
    assert_eq!(d["todosDetail"].as_array().unwrap().len(), 2);
    let f = &d["filesChanged"][0];
    assert_eq!(f["path"], "src/a.rs");
    assert_eq!(f["added"], 2);
    assert_eq!(f["removed"], 1);
    assert_eq!(d["pendingDetail"][0]["input"]["command"], "rm -rf build");
    assert!(d["agentsDetail"].is_array() && d.get("agents_detail").is_none());

    // ── inbox ──
    let v = get_json(&c, format!("{base}/api/inbox"), &token).await;
    assert!(v["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["sessionId"] == LIVE && i["source"] == "observed"));

    // ── fleet ──
    let v = get_json(&c, format!("{base}/api/agents"), &token).await;
    let fleet = v["sessions"].as_array().unwrap();
    assert!(fleet.iter().any(|s| s["sessionId"] == LIVE));
    assert!(v["summary"]["agentsRunning"].as_u64().unwrap() >= 2);
    assert_eq!(v["summary"]["workflowsRunning"], 1);
    let v = get_json(&c, format!("{base}/api/sessions/{LIVE}/agents"), &token).await;
    assert!(v["agents"].as_array().unwrap().len() >= 3);

    // ── transcript paging ──
    let page = get_json(
        &c,
        format!("{base}/api/sessions/{LIVE}/transcript?limit=3"),
        &token,
    )
    .await;
    assert!(page["events"].as_array().unwrap().len() >= 3);
    let before = page["before"].as_str().expect("more before").to_string();
    let mut all: Vec<Value> = page["events"].as_array().unwrap().clone();
    let mut cursor = Some(before);
    while let Some(b) = cursor {
        let p = get_json(
            &c,
            format!("{base}/api/sessions/{LIVE}/transcript?limit=3&before={b}"),
            &token,
        )
        .await;
        let mut evs = p["events"].as_array().unwrap().clone();
        evs.extend(all);
        all = evs;
        cursor = p["before"].as_str().map(str::to_string);
    }
    let ids: Vec<&str> = all.iter().map(|e| e["id"].as_str().unwrap()).collect();
    let mut dedup = ids.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(dedup.len(), ids.len(), "paging never repeats an event");
    assert_eq!(all[0]["kind"], "prompt");
    let body = serde_json::to_string(&all).unwrap();
    assert!(!body.contains("ECONNREFUSED"), "noise filtered");
    assert!(
        !body.contains(&base64::engine::general_purpose::STANDARD.encode(PNG)),
        "no inline base64"
    );
    let imgs: Vec<&Value> = all.iter().filter(|e| e["kind"] == "image").collect();
    assert_eq!(imgs.len(), 2);
    let r = imgs[0]["image"]["ref"].as_str().unwrap().to_string();
    let svg_ref = imgs[1]["image"]["ref"].as_str().unwrap().to_string();
    assert!(r.starts_with(&format!("/api/sessions/{LIVE}/media/")));
    assert!(all.iter().any(|e| e["kind"] == "todos"));
    assert!(all.iter().any(|e| e["messageId"] == "msg_A"));

    // ── media ──
    let m = get(&c, format!("{base}{r}"), &token).await;
    assert_eq!(m.status(), StatusCode::OK);
    assert_eq!(m.headers()["content-type"], "image/png");
    assert_eq!(m.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        m.headers()["content-security-policy"],
        "default-src 'none'; sandbox"
    );
    assert_eq!(m.headers()["cache-control"], "private, no-store");
    assert_eq!(m.bytes().await.unwrap().as_ref(), PNG);
    let m = get(&c, format!("{base}{svg_ref}"), &token).await;
    assert_eq!(m.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let m = get(
        &c,
        format!("{base}/api/sessions/{LIVE}/media/0/img-1/1"),
        &token,
    )
    .await;
    assert_eq!(m.status(), StatusCode::NOT_FOUND);

    // ── task output ──
    let o = get(
        &c,
        format!("{base}/api/sessions/{LIVE}/tasks/bsh1/output"),
        &token,
    )
    .await;
    assert_eq!(o.status(), StatusCode::OK);
    assert_eq!(o.headers()["content-type"], "text/plain; charset=utf-8");
    assert_eq!(o.headers()["x-content-type-options"], "nosniff");
    assert!(o.text().await.unwrap().contains("ready on :3000"));
    let o = get(
        &c,
        format!("{base}/api/sessions/{LIVE}/tasks/bout/output"),
        &token,
    )
    .await;
    assert_eq!(
        o.status(),
        StatusCode::NOT_FOUND,
        "outside the CLI temp root"
    );

    // ── id validation / traversal ──
    for url in [
        format!("{base}/api/sessions/..%2F..%2Fetc%2Fpasswd"),
        format!("{base}/api/sessions/..%2F..%2Fetc/transcript"),
        format!("{base}/api/sessions/{LIVE}/transcript?agent=..%2F..%2Fx"),
        format!("{base}/api/sessions/{LIVE}/tasks/..%2F..%2Fetc%2Fpasswd/output"),
        format!("{base}/api/sessions/{LIVE}/media/0/..%2Fx/1"),
        format!("{base}/api/sessions/{LIVE}/media/0/img-1/1.x"),
        format!("{base}/api/sessions/{LIVE}/file-patch?path=..%2F..%2Fetc%2Fpasswd"),
        format!("{base}/api/sessions/{}/agents", ENDED.to_uppercase()),
    ] {
        let r = get(&c, url.clone(), &token).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{url}");
    }
    let r = get(
        &c,
        format!("{base}/api/sessions/{}/transcript", uuid::Uuid::new_v4()),
        &token,
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // ── agent transcript + `after` cursor ──
    let p = get_json(
        &c,
        format!("{base}/api/sessions/{LIVE}/transcript?agent=aexp1"),
        &token,
    )
    .await;
    assert_eq!(p["events"][0]["agentId"], "aexp1");
    assert_eq!(p["events"][0]["tool"]["name"], "Grep");
    let tail = page["tail"].as_str().unwrap().to_string();
    let p = get_json(
        &c,
        format!("{base}/api/sessions/{LIVE}/transcript?after={tail}"),
        &token,
    )
    .await;
    assert!(p["events"].as_array().unwrap().is_empty());
    assert_eq!(p["tail"], tail);

    // ── projects ──
    let v = get_json(&c, format!("{base}/api/projects"), &token).await;
    assert!(v["projects"].is_array());
}

#[tokio::test]
async fn hooks_accept_large_post_tool_use_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let st = state(dir.path());
    let base = serve(server::router(st.clone())).await;
    let big = "x".repeat(5 * 1024 * 1024);
    let body = json!({
        "hook_event_name": "PostToolUse",
        "session_id": LIVE,
        "tool_name": "Read",
        "tool_input": {"file_path": "/a"},
        "tool_response": {"content": big},
    });
    let r = reqwest::Client::new()
        .post(format!("{base}/hooks/event"))
        .bearer_auth(&st.auth.hook_token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
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
        if let Ok(text) = msg.to_text() {
            if let Ok(v) = serde_json::from_str::<Value>(text) {
                if v["t"] == "ping" {
                    continue;
                }
                return Some(v);
            }
        }
    }
}

async fn until<S>(ws: &mut S, t: &str) -> Value
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let v = next_json(ws)
            .await
            .unwrap_or_else(|| panic!("no {t} frame"));
        if v["t"] == t {
            return v;
        }
    }
}

#[tokio::test]
async fn websocket_subscriptions_get_appends_and_bounded_replays() {
    let dir = tempfile::tempdir().unwrap();
    let _cleanup = fixture(dir.path());
    let st = state(dir.path());
    monitor::refresh_once(&st).await;
    let base = serve(server::router(st.clone())).await;
    let url = format!("{}/ws", base.replacen("http", "ws", 1));
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(Message::Text(
        json!({"t":"auth","token": st.auth.token})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(until(&mut ws, "hello").await["v"], 2);
    let full = until(&mut ws, "sessions").await;
    assert_eq!(full["full"], true);
    assert!(full["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == LIVE));

    // Subscribe to the main transcript and to the subagent.
    for f in [
        json!({"t":"sub","session":LIVE}),
        json!({"t":"sub","session":LIVE,"agent":"aexp1"}),
        json!({"t":"subFleet"}),
        json!({"t":"ping"}),
    ] {
        ws.send(Message::Text(f.to_string().into())).await.unwrap();
    }
    let fleet = until(&mut ws, "fleet").await;
    assert!(fleet["summary"].is_object());
    until(&mut ws, "pong").await;
    assert!(st.hub.is_subscribed(LIVE, None));
    assert!(st.hub.is_subscribed(LIVE, Some("aexp1")));

    // Append to both files; one refresh publishes both appends.
    let main = dir.path().join(format!("projects/-tmp-proj/{LIVE}.jsonl"));
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&main)
        .unwrap();
    writeln!(
        f,
        "{}",
        json!({"type":"assistant","uuid":"new-1","timestamp":iso(now()),"message":{"id":"msg_N","model":"claude-haiku-4-5","content":[{"type":"text","text":"appended"}],"usage":{"input_tokens":1,"output_tokens":1}}})
    )
    .unwrap();
    let agent = dir.path().join(format!(
        "projects/-tmp-proj/{LIVE}/subagents/agent-aexp1.jsonl"
    ));
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&agent)
        .unwrap();
    writeln!(
        f,
        "{}",
        json!({"type":"assistant","isSidechain":true,"uuid":"sub-2","timestamp":iso(now()),"message":{"id":"msg_S2","content":[{"type":"text","text":"sub says hi"}]}})
    )
    .unwrap();
    monitor::refresh_once(&st).await;
    let mut got_main = false;
    let mut got_agent = false;
    while !(got_main && got_agent) {
        let v = until(&mut ws, "transcript").await;
        match v["agent"].as_str() {
            None => {
                assert_eq!(v["events"][0]["text"], "appended");
                assert_eq!(v["events"][0]["messageId"], "msg_N");
                got_main = true;
            }
            Some("aexp1") => {
                assert_eq!(v["events"][0]["text"], "sub says hi");
                got_agent = true;
            }
            Some(other) => panic!("unexpected agent {other}"),
        }
    }

    // A replay from far behind is capped: transcriptReset instead of a flood.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&main)
        .unwrap();
    for i in 0..400 {
        writeln!(
            f,
            "{}",
            json!({"type":"user","uuid":format!("flood-{i}"),"message":{"content":format!("p{i}")}})
        )
        .unwrap();
    }
    ws.send(Message::Text(
        json!({"t":"sub","session":LIVE,"since":"0"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let reset = until(&mut ws, "transcriptReset").await;
    assert_eq!(reset["session"], LIVE);

    // Bad agent ids are ignored.
    ws.send(Message::Text(
        json!({"t":"sub","session":LIVE,"agent":"../x"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    ws.send(Message::Text(r#"{"t":"ping"}"#.into()))
        .await
        .unwrap();
    until(&mut ws, "pong").await;
    assert!(!st.hub.is_subscribed(LIVE, Some("../x")));
    let _ = ws.close(None).await;
}

/// The production loop (`monitor::run`) indexes history in the background,
/// writes `index-cache.json`, and on the next start adopts the checkpoints
/// instead of rescanning: a marker planted in the checkpoint shows up.
#[tokio::test]
async fn monitor_run_writes_and_resumes_from_checkpoints() {
    let home_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let proj = home_dir.path().join("projects/-tmp-cp");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::create_dir_all(home_dir.path().join("sessions")).unwrap();
    std::fs::write(
        proj.join(format!("{ENDED}.jsonl")),
        lines(&[
            json!({"type":"user","uuid":"c1","timestamp":iso(now()-5000),"cwd":"/tmp/cp","message":{"content":"hi"}}),
            json!({"type":"custom-title","customTitle":"From the transcript"}),
        ]),
    )
    .unwrap();
    let boot = || {
        let mut auth = Auth::ephemeral();
        auth.config_dir = Some(data_dir.path().to_path_buf());
        Inner::new(
            ClaudeHome::with_base(home_dir.path()),
            ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            auth,
        )
    };
    async fn name_of(st: &AppState) -> Option<String> {
        for _ in 0..100 {
            if let Some(s) = st.find_session(ENDED).await {
                return Some(s.name);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        None
    }
    let cache = data_dir.path().join("index-cache.json");

    // First run: indexed by the worker, checkpoint written when it drains.
    let st = boot();
    let task = tokio::spawn(monitor::run(st.clone()));
    let mut named = None;
    for _ in 0..100 {
        named = st.find_session(ENDED).await.map(|s| s.name);
        if named.as_deref() == Some("From the transcript") && cache.is_file() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    task.abort();
    assert_eq!(named.as_deref(), Some("From the transcript"));
    assert!(cache.is_file(), "checkpoint written");

    // Plant a marker in the checkpoint; an adopted checkpoint shows it.
    let mut v: Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    for f in v["files"].as_array_mut().unwrap() {
        f["agg"]["customTitle"] = json!("FROM-CACHE");
    }
    std::fs::write(&cache, serde_json::to_vec(&v).unwrap()).unwrap();
    let st = boot();
    let task = tokio::spawn(monitor::run(st.clone()));
    let name = name_of(&st).await;
    task.abort();
    assert_eq!(name.as_deref(), Some("FROM-CACHE"));
}

/// The list's search and filters on the server: every earlier session that
/// matches, not only the pages the client has read (*Select all*).
#[tokio::test]
async fn the_session_list_searches_and_filters_on_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let _cleanup = fixture(dir.path());
    let st = state(dir.path());
    monitor::refresh_once(&st).await;
    let base = serve(server::router(st.clone())).await;
    let token = st.auth.token.clone();
    let c = reqwest::Client::new();
    let ids_of = |v: &Value| -> Vec<String> {
        v["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_string())
            .collect()
    };
    let list = |query: &str| {
        let url = format!("{base}/api/sessions?scope=all{query}");
        let (c, token) = (c.clone(), token.clone());
        async move { get_json(&c, url, &token).await }
    };

    // The search, in any case, over the name, branch and prompt.
    assert_eq!(ids_of(&list("&q=BUILD").await), vec![LIVE.to_string()]);
    assert_eq!(ids_of(&list("&q=feat%2Fx").await), vec![LIVE.to_string()]);
    let both = ids_of(&list("&q=proj").await);
    assert!(both.contains(&LIVE.to_string()) && both.contains(&ENDED.to_string()));
    assert!(ids_of(&list("&q=no-such-thing").await).is_empty());
    // A blank search says nothing.
    assert_eq!(
        ids_of(&list("&q=%20%20").await).len(),
        ids_of(&list("").await).len()
    );

    // The other filters.
    assert_eq!(ids_of(&list("&only=needs").await), vec![LIVE.to_string()]);
    assert_eq!(ids_of(&list("&only=live").await), vec![LIVE.to_string()]);
    let vscode = list("&surface=vscode").await;
    assert!(ids_of(&vscode).contains(&LIVE.to_string()));
    assert!(vscode["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["surface"] == "vscode"));
    assert_eq!(ids_of(&list("&project=proj").await).len(), both.len());
    assert!(ids_of(&list("&account=acc_000000000000").await).is_empty());

    // Every match's id, unpaged, for Select all.
    let v = list("&q=build&view=ids&limit=1").await;
    assert_eq!(v["ids"], json!([LIVE]));
    assert!(v.get("sessions").is_none());
    let v = list("&view=ids&limit=1").await;
    let all: Vec<&str> = v["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_str().unwrap())
        .collect();
    assert!(
        all.contains(&LIVE) && all.contains(&ENDED),
        "not paged: {v}"
    );
}
