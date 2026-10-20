//! Manual helpers, not checks: the app's server serving the built SPA over
//! real Claude Code data, to see what the dashboard shows.
//!
//! `serve_the_real_home_for_ui_checks` reads the REAL `~/.claude` (read-only)
//! with a cold index (a fresh temp data dir), or warm from a COPY of a
//! checkpoint (`MC_REAL_CACHE`):
//!
//! ```sh
//! npm run build
//! MC_REAL_PORT=6798 MC_REAL_SECONDS=300 MC_REAL_AT=2,10,60 \
//!   MC_REAL_OUT=/tmp/mc-real.json MC_REAL_SAMPLES=/tmp/mc-real-samples \
//!   cargo test --manifest-path src-tauri/Cargo.toml --test real_home_ui \
//!   serve_the_real_home -- --ignored --nocapture
//! ```
//!
//! `restart_then_messages_on_a_copied_transcript` copies one real main
//! transcript (`MC_COPY_SRC`, only read) into a temp home, runs it as a live
//! VS Code session whose process started after its last message (a restart),
//! then appends a regular message without a marker (`<dataDir>/go-b`) and one
//! with an `ultra_effort_exit` (`<dataDir>/go-c`), printing what the API says
//! after each step.
//!
//! * Nothing is written under `~/.claude`: checkpoints, devices and the rest
//!   go to a temp dir; no hook plugin is installed or refreshed.
//! * The real Claude Code is never run: `MOTHER_CLAUDE_CLI` points at a stub
//!   that only answers `auth status` (signed in) and fails everything else,
//!   so no plan-usage helper, plugin listing or daemon status reaches it (the
//!   plan-usage scheduler, the bridge, push and the LAN/tunnel listeners are
//!   not started at all).
//! * `MC_REAL_OUT` / `MC_COPY_OUT` receive `{base, token, dataDir}` (0600): a
//!   throwaway desktop token of this run, dead when the process exits.

use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use mother_claude_lib::bridge::{BridgeConfig, TokenPolicy};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, hooks, monitor, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

const STUB_CLI: &str = "#!/bin/sh\n\
# Stand-in for Claude Code in the real-home UI harness: never the real CLI.\n\
if [ \"$1\" = auth ] && [ \"$2\" = status ]; then\n\
  printf '%s\\n' '{\"loggedIn\":true,\"authMethod\":\"claude.ai\",\"apiProvider\":\"firstParty\"}'\n\
  exit 0\n\
fi\n\
exit 1\n";

fn iso(ms: Option<i64>) -> String {
    ms.and_then(chrono::DateTime::from_timestamp_millis)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| "-".into())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// One line per live session: what the list card, the chips and Info read.
fn describe(s: &Value) -> String {
    let id = s["id"].as_str().unwrap_or_default();
    format!(
        "{} {:<14} {:<13} status={:<8} effort={} ({}) ultracode={} source={} at={} unknown={} before={}@{} thinking={}",
        id.get(..8).unwrap_or(id),
        s["entrypoint"].as_str().unwrap_or("-"),
        s["surface"].as_str().unwrap_or("-"),
        s["status"].as_str().unwrap_or("-"),
        s["effort"],
        s["effortSource"],
        s["ultracode"],
        s["ultracodeSource"],
        iso(s["ultracodeAt"].as_i64()),
        s["ultracodeUnknown"],
        s["ultracodeBefore"],
        iso(s["ultracodeBeforeAt"].as_i64()),
        s["thinking"],
    )
}

fn write_private(path: &str, text: &str) {
    std::fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

struct Harness {
    st: AppState,
    base: String,
    t0: Instant,
    changes: Arc<Mutex<Vec<String>>>,
    data: tempfile::TempDir,
}

/// The server as `server::serve` starts it (the parts that read the files),
/// over `home`, with a dashboard connected over the WebSocket from the start.
async fn launch(
    home: ClaudeHome,
    port: u16,
    cache: Option<PathBuf>,
    out: Option<String>,
) -> Harness {
    let data = tempfile::tempdir().unwrap();
    let stub = data.path().join("claude-stub.sh");
    std::fs::write(&stub, STUB_CLI).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("MOTHER_CLAUDE_CLI", &stub);
    std::env::remove_var("MOTHER_CLAUDE_SIDECAR");
    match cache {
        Some(src) => {
            std::fs::copy(&src, data.path().join("index-cache.json")).unwrap();
            println!("index: warm, from a copy of {}", src.display());
        }
        None => println!("index: cold (no checkpoint)"),
    }
    println!("claude home (read-only): {}", home.base().display());
    let mut auth = Auth::ephemeral();
    // Checkpoints, devices, hooks state: the temp dir, as the app's data dir.
    auth.config_dir = Some(data.path().to_path_buf());
    let st = Inner::with_bridge_config(
        home,
        ServerConfig {
            host: "127.0.0.1".into(),
            port,
        },
        auth,
        BridgeConfig {
            enabled: true,
            port: None,
            token: TokenPolicy::Local,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    let signed_in = st.refresh_claude_auth().await;
    println!("claude auth (stub): {}", signed_in.summary());

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    let app = server::router_for(st.clone(), Ingress::Loopback);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let t0 = Instant::now();
    tokio::spawn(monitor::run(st.clone()));
    hooks::start(&st);

    let base = format!("http://127.0.0.1:{port}");
    let info = json!({ "base": base, "token": st.auth.token, "dataDir": data.path() });
    match out {
        Some(out) => {
            write_private(&out, &info.to_string());
            println!("info written to {out} (data dir {})", data.path().display());
        }
        None => println!("base {base} (set the OUT variable to get the token)"),
    }

    let changes: Arc<Mutex<Vec<String>>> = Arc::default();
    {
        let changes = changes.clone();
        let token = st.auth.token.clone();
        let url = format!("ws://127.0.0.1:{port}/ws");
        tokio::spawn(async move {
            let Ok((ws, _)) = tokio_tungstenite::connect_async(url).await else {
                return;
            };
            let (mut tx, mut rx) = ws.split();
            let _ = tx
                .send(Message::Text(
                    json!({ "t": "auth", "token": token }).to_string().into(),
                ))
                .await;
            let mut seen: HashMap<String, Value> = HashMap::new();
            let key = |s: &Value| {
                [
                    "ultracode",
                    "ultracodeSource",
                    "ultracodeAt",
                    "ultracodeUnknown",
                    "ultracodeBefore",
                    "effort",
                ]
                .map(|k| s[k].clone())
            };
            while let Some(Ok(msg)) = rx.next().await {
                let Message::Text(text) = msg else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if v["t"] == "ping" {
                    let _ = tx.send(Message::Text(r#"{"t":"pong"}"#.into())).await;
                    continue;
                }
                if v["t"] != "sessions" {
                    continue;
                }
                let full = v["full"] == true;
                let list = if full { &v["sessions"] } else { &v["upsert"] };
                for s in list.as_array().into_iter().flatten() {
                    let id = s["id"].as_str().unwrap_or_default().to_string();
                    if s["live"] == true && seen.get(&id).map(key) != Some(key(s)) {
                        changes.lock().unwrap().push(format!(
                            "  {:>7.2}s seq={} full={full} {}",
                            t0.elapsed().as_secs_f64(),
                            v["seq"],
                            describe(s)
                        ));
                    }
                    seen.insert(id, s.clone());
                }
            }
        });
    }
    Harness {
        st,
        base,
        t0,
        changes,
        data,
    }
}

impl Harness {
    /// `GET /api/sessions` now: print every live session (and save the body).
    async fn sample(&self, label: &str, save: Option<&Path>) -> Vec<Value> {
        let secs = self.t0.elapsed().as_secs_f64();
        let body: Value = match reqwest::Client::new()
            .get(format!("{}/api/sessions", self.base))
            .bearer_auth(&self.st.auth.token)
            .send()
            .await
        {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(e) => {
                println!("{label} t={secs:.2}s: /api/sessions failed: {e}");
                return Vec::new();
            }
        };
        if let Some(dir) = save {
            let _ = std::fs::write(
                dir.join(format!("sessions-{label}.json")),
                serde_json::to_vec_pretty(&body).unwrap_or_default(),
            );
        }
        let sessions = body["sessions"].as_array().cloned().unwrap_or_default();
        let live: Vec<Value> = sessions
            .iter()
            .filter(|s| s["live"] == true)
            .cloned()
            .collect();
        println!(
            "{label} t={secs:.2}s: /api/sessions → {} sessions, {} live, {} index jobs queued",
            sessions.len(),
            live.len(),
            self.st.index.pending_jobs()
        );
        for s in &live {
            println!("  {}", describe(s));
        }
        live
    }

    fn print_changes(&self) {
        println!("live-session changes the dashboard received over the WebSocket:");
        for c in self.changes.lock().unwrap().iter() {
            println!("{c}");
        }
    }

    /// Wait for `<dataDir>/<name>` (created by whoever drives the browser),
    /// at most `max`.
    async fn wait_for(&self, name: &str, max: Duration) -> bool {
        let path = self.data.path().join(name);
        let until = Instant::now() + max;
        while Instant::now() < until {
            if path.exists() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        false
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual helper: serves the SPA over the real ~/.claude (read-only)"]
async fn serve_the_real_home_for_ui_checks() {
    let port: u16 = env_or("MC_REAL_PORT", 6798);
    let seconds: u64 = env_or("MC_REAL_SECONDS", 120);
    let at: Vec<f64> = std::env::var("MC_REAL_AT")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .filter(|v: &Vec<f64>| !v.is_empty())
        .unwrap_or_else(|| vec![2.0, 10.0, 60.0]);
    let samples = std::env::var("MC_REAL_SAMPLES").ok().map(PathBuf::from);
    if let Some(dir) = &samples {
        std::fs::create_dir_all(dir).unwrap();
    }
    let cache = std::env::var("MC_REAL_CACHE").ok().map(PathBuf::from);
    let h = launch(
        ClaudeHome::resolve().unwrap(),
        port,
        cache,
        std::env::var("MC_REAL_OUT").ok(),
    )
    .await;

    for t in &at {
        let wait = Duration::from_secs_f64(*t).saturating_sub(h.t0.elapsed());
        tokio::time::sleep(wait).await;
        h.sample(&format!("{t}s"), samples.as_deref()).await;
    }
    h.print_changes();

    // Keep serving for the browser.
    let rest = Duration::from_secs(seconds).saturating_sub(h.t0.elapsed());
    tokio::time::sleep(rest).await;
    h.print_changes();
    if let Ok(keep) = std::env::var("MC_REAL_KEEP_CACHE") {
        let st = h.st.clone();
        tokio::task::spawn_blocking(move || st.index.persist(true))
            .await
            .unwrap();
        std::fs::copy(h.data.path().join("index-cache.json"), &keep).unwrap();
        println!("checkpoint kept at {keep}");
    }
}

/// The last `cwd` and `uuid` written in a transcript (its tail only).
fn tail_facts(path: &Path) -> (String, Option<String>) {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).unwrap();
    let len = f.metadata().unwrap().len();
    let from = len.saturating_sub(4 << 20);
    f.seek(SeekFrom::Start(from)).unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    let (mut cwd, mut uuid) = (String::new(), None);
    for line in buf.split(|b| *b == b'\n').skip(usize::from(from > 0)) {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if let Some(c) = v["cwd"].as_str() {
            cwd = c.to_string();
        }
        if let Some(u) = v["uuid"].as_str() {
            uuid = Some(u.to_string());
        }
    }
    (cwd, uuid)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual helper: a restarted live session over a copy of a real transcript"]
async fn restart_then_messages_on_a_copied_transcript() {
    let src = PathBuf::from(std::env::var("MC_COPY_SRC").expect("MC_COPY_SRC: a main transcript"));
    let port: u16 = env_or("MC_COPY_PORT", 6797);
    let max = Duration::from_secs(env_or("MC_COPY_WAIT", 600));
    let sid = src.file_stem().unwrap().to_string_lossy().to_string();

    // A home with only this conversation, run by this process as a VS Code
    // session that (re)started now: after the conversation's last message.
    let home = tempfile::tempdir().unwrap();
    let proj = home
        .path()
        .join("projects")
        .join(src.parent().unwrap().file_name().unwrap());
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::create_dir_all(home.path().join("sessions")).unwrap();
    let copy = proj.join(format!("{sid}.jsonl"));
    std::fs::copy(&src, &copy).unwrap();
    // The source may be mid-write: keep whole lines only.
    let bytes = std::fs::read(&copy).unwrap();
    let whole = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    if whole < bytes.len() {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&copy)
            .unwrap()
            .set_len(whole as u64)
            .unwrap();
    }
    let (cwd, mut parent) = tail_facts(&copy);
    assert!(!cwd.is_empty(), "no cwd in the transcript's tail");
    let me = std::process::id();
    let started = now_ms() - 2_000;
    std::fs::write(
        home.path().join(format!("sessions/{me}.json")),
        json!({
            "pid": me, "sessionId": sid, "cwd": cwd, "startedAt": started,
            "kind": "interactive", "entrypoint": "claude-vscode", "version": "2.1.284",
            "status": "idle", "statusUpdatedAt": now_ms()
        })
        .to_string(),
    )
    .unwrap();
    println!(
        "copied {} → {}; process start {}",
        src.display(),
        copy.display(),
        iso(Some(started))
    );

    let h = launch(
        ClaudeHome::with_base(home.path()),
        port,
        None,
        std::env::var("MC_COPY_OUT").ok(),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    h.sample("A-restarted", None).await;

    let base = |kind: &str, uuid: &str, parent: &Option<String>, ts: i64| {
        json!({
            "parentUuid": parent, "isSidechain": false, "type": kind, "uuid": uuid,
            "timestamp": iso(Some(ts)), "userType": "external", "entrypoint": "claude-vscode",
            "cwd": cwd, "sessionId": sid, "version": "2.1.284"
        })
    };
    let append = |lines: &[Value]| {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&copy)
            .unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    };
    let mut n = 0;
    let mut message = |exit_marker: bool, parent: &mut Option<String>| -> (Vec<Value>, i64) {
        n += 1;
        let ts = now_ms();
        let mut lines = Vec::new();
        let mut prompt = base("user", &format!("verify-prompt-{n}"), parent, ts);
        prompt["promptId"] = json!(format!("verify-promptid-{n}"));
        prompt["permissionMode"] = json!("bypassPermissions");
        prompt["origin"] = json!({ "kind": "human" });
        prompt["message"] = json!({ "role": "user", "content": [{ "type": "text", "text": format!("verifier message {n}") }] });
        *parent = Some(format!("verify-prompt-{n}"));
        lines.push(prompt);
        if exit_marker {
            let mut marker = base("attachment", &format!("verify-marker-{n}"), parent, ts - 1);
            marker["attachment"] = json!({ "type": "ultra_effort_exit" });
            *parent = Some(format!("verify-marker-{n}"));
            lines.push(marker);
        }
        let mut reply = base(
            "assistant",
            &format!("verify-reply-{n}"),
            parent,
            ts + 1_500,
        );
        reply["message"] = json!({
            "id": format!("msg_verify_{n}"), "type": "message", "role": "assistant",
            "model": "claude-opus-5-5", "content": [{ "type": "text", "text": "ok" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 1, "output_tokens": 1, "speed": "standard" }
        });
        reply["effort"] = json!("max");
        reply["perTurnEffort"] = json!("max");
        *parent = Some(format!("verify-reply-{n}"));
        lines.push(reply);
        (lines, ts)
    };

    if !h.wait_for("go-b", max).await {
        h.print_changes();
        return;
    }
    let (lines, ts) = message(false, &mut parent);
    append(&lines);
    println!(
        "B: appended a regular message at {} (no marker)",
        iso(Some(ts))
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    h.sample("B-silent-message", None).await;

    if !h.wait_for("go-c", max).await {
        h.print_changes();
        return;
    }
    let (lines, ts) = message(true, &mut parent);
    append(&lines);
    println!(
        "C: appended a regular message at {} with ultra_effort_exit",
        iso(Some(ts))
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    h.sample("C-exit-marker", None).await;

    h.wait_for("go-end", max).await;
    h.print_changes();
}
