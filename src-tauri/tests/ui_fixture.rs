//! A manual helper, not a check: serve the built SPA and the real API over a
//! synthetic `~/.claude` so the UI can be driven in a browser against the
//! real backend (never the mock).
//!
//! ```sh
//! npm run build
//! MC_UI_PORT=6799 MC_UI_SECONDS=300 MC_UI_OUT=/tmp/mc-ui.json \
//!   cargo test --manifest-path src-tauri/Cargo.toml --test ui_fixture -- --ignored --nocapture
//! ```
//!
//! `MC_UI_OUT` receives `{base, desktopToken, hookToken, dataDir, home,
//! session}` (throwaway values of this run). The MCP server runs as in the
//! app: its token is `<dataDir>/mcp.token`, and `<dataDir>/mcp-plugin/
//! mother-claude` is the plugin copy a session loads with `--plugin-dir`; a
//! `sessions/<pid>.json` written under `home` makes that pid a session to
//! `/mcp`. A browser extension pairs with a code from `POST /api/browser/pair`
//! (the desktop token). A foreign VS Code session run by this
//! process is live, with a transcript that grows every few seconds. Creating
//! `<dataDir>/raise-permission` makes the session's hook plugin (the real
//! `mc-hook` script, run as a child of this process) hold a Bash permission
//! request, as a foreign session would. Owned sessions spawned from the UI
//! run the scripted fake CLI (`tests/fixtures/fake-claude.py`, no tokens
//! spent): send `hello`, `bash`, `ask` or `plan` to see each kind of card.
//! `MC_UI_REVEAL=record` makes *Show in Finder* record its command (printed
//! on stdout) instead of opening a window, for unattended runs.
//!
//! Two more live sessions are between turns (Claude Code's pidfile says
//! `idle`), each "run" by a `sleep` child of this process: a VS Code session
//! whose last turn left a release build running in the background (background
//! work), and a terminal `/loop` waiting for its next wakeup (scheduled).
//! Creating `<dataDir>/finish-build` appends the build's task-notification:
//! nothing resumes that session any more, and it turns idle.

// Tests start their programs directly; the app's own code goes through
// `proc`, which keeps console windows from flashing up on Windows.
#![allow(clippy::disallowed_methods)]

use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use mother_claude_lib::bridge::{BridgeConfig, TokenPolicy};
use mother_claude_lib::claude::hooks::{write_plugin_files, write_runtime_files, PluginPaths};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, hooks, monitor, push, Ingress};
use mother_claude_lib::state::{Inner, ServerConfig};
use serde_json::json;

const SID: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";
const ENDED: &str = "1a2b3c4d-5e6f-4a0b-8c1d-2e3f4a5b6c7d";
/// Between turns, its release build running in the background.
const BUILD: &str = "2b3c4d5e-6f70-4a81-9b2c-3d4e5f607182";
/// Between turns, a `/loop` wakeup ahead.
const LOOP: &str = "3c4d5e6f-7081-4b92-8c3d-4e5f60718293";

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn fixture(base: &Path) {
    let me = std::process::id();
    let t0 = now() - 120_000;
    let proj = base.join("projects/-tmp-ui-demo");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::create_dir_all(base.join("sessions")).unwrap();
    std::fs::write(
        base.join(format!("sessions/{me}.json")),
        json!({
            "pid": me, "sessionId": SID, "cwd": "/tmp/ui-demo", "startedAt": t0,
            "kind": "interactive", "entrypoint": "claude-vscode", "version": "2.1.282",
            "status": "busy", "statusUpdatedAt": now()
        })
        .to_string(),
    )
    .unwrap();
    let main = [
        json!({"type":"user","uuid":"ui-u1","timestamp":iso(t0),"cwd":"/tmp/ui-demo","gitBranch":"main","permissionMode":"default","origin":{"kind":"human"},"message":{"role":"user","content":"Add a README section about pairing"}}),
        json!({"type":"assistant","uuid":"ui-a1","timestamp":iso(t0 + 2000),"message":{"id":"msg_ui1","model":"claude-haiku-4-5","content":[{"type":"text","text":"I'll look at the docs first."},{"type":"tool_use","id":"tu_ui1","name":"Read","input":{"file_path":"/tmp/ui-demo/README.md"}}],"usage":{"input_tokens":1200,"output_tokens":40,"cache_read_input_tokens":3000}}}),
        json!({"type":"user","uuid":"ui-r1","timestamp":iso(t0 + 2500),"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_ui1","content":"# Demo\n"}]}}),
        json!({"type":"assistant","uuid":"ui-a2","timestamp":iso(t0 + 4000),"message":{"id":"msg_ui2","model":"claude-haiku-4-5","content":[{"type":"tool_use","id":"tu_ui2","name":"TodoWrite","input":{"todos":[{"content":"Read the docs","status":"completed"},{"content":"Write the section","status":"in_progress","activeForm":"Writing the section"}]}}],"usage":{"input_tokens":10,"output_tokens":30}}}),
        json!({"type":"ai-title","aiTitle":"README pairing section"}),
    ];
    let text: String = main.iter().map(|l| l.to_string() + "\n").collect();
    std::fs::write(proj.join(format!("{SID}.jsonl")), text).unwrap();
    let ended = [
        json!({"type":"user","uuid":"ui-e1","timestamp":iso(t0 - 3_600_000),"cwd":"/tmp/ui-demo","message":{"content":"fix the flaky test"}}),
        json!({"type":"assistant","uuid":"ui-e2","timestamp":iso(t0 - 3_590_000),"message":{"id":"msg_ue","model":"claude-haiku-4-5","content":[{"type":"text","text":"Fixed: the timeout was too short."}],"usage":{"input_tokens":500,"output_tokens":20}}}),
    ];
    let text: String = ended.iter().map(|l| l.to_string() + "\n").collect();
    std::fs::write(proj.join(format!("{ENDED}.jsonl")), text).unwrap();
}

/// The sessions between turns, their processes `build_pid` and `loop_pid`.
fn between_turns(base: &Path, build_pid: u32, loop_pid: u32) {
    let t0 = now() - 600_000;
    let pidfile = |pid: u32, sid: &str, cwd: &str, entrypoint: &str| {
        std::fs::write(
            base.join(format!("sessions/{pid}.json")),
            json!({
                "pid": pid, "sessionId": sid, "cwd": cwd, "startedAt": t0,
                "kind": "interactive", "entrypoint": entrypoint, "version": "2.1.314",
                "status": "idle", "statusUpdatedAt": now() - 60_000
            })
            .to_string(),
        )
        .unwrap();
    };
    let write = |dir: &str, sid: &str, lines: &[serde_json::Value]| {
        let proj = base.join("projects").join(dir);
        std::fs::create_dir_all(&proj).unwrap();
        let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
        std::fs::write(proj.join(format!("{sid}.jsonl")), text).unwrap();
    };
    pidfile(build_pid, BUILD, "/tmp/ui-release", "claude-vscode");
    write(
        "-tmp-ui-release",
        BUILD,
        &[
            json!({"type":"user","uuid":"rb-u1","timestamp":iso(t0 + 1000),"cwd":"/tmp/ui-release","gitBranch":"release/v0.5.1","origin":{"kind":"human"},"message":{"role":"user","content":"Cut v0.5.1 and notarize it"}}),
            json!({"type":"assistant","uuid":"rb-a1","timestamp":iso(t0 + 3000),"message":{"id":"msg_rb1","model":"claude-opus-5-5","stop_reason":"tool_use","content":[{"type":"tool_use","id":"tu_build","name":"Bash","input":{"command":"scripts/release/release-macos.sh 0.5.1","description":"Release build","run_in_background":true}}],"usage":{"input_tokens":900,"output_tokens":60}}}),
            json!({"type":"user","uuid":"rb-r1","timestamp":iso(t0 + 3500),"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_build","content":"Command running in background with ID: brel1."}]},"toolUseResult":{"stdout":"","backgroundTaskId":"brel1"}}),
            json!({"type":"assistant","uuid":"rb-a2","timestamp":iso(t0 + 6000),"message":{"id":"msg_rb2","model":"claude-opus-5-5","stop_reason":"end_turn","content":[{"type":"text","text":"The release build is running: gate, archive, notarization of the app and the disk image, then verification. When it finishes I'll verify the artifacts and report back."}],"usage":{"input_tokens":20,"output_tokens":45}}}),
            json!({"type":"ai-title","aiTitle":"Build and notarize v0.5.1"}),
        ],
    );
    pidfile(loop_pid, LOOP, "/tmp/ui-infra", "cli");
    write(
        "-tmp-ui-infra",
        LOOP,
        &[
            json!({"type":"user","uuid":"lp-u1","timestamp":iso(t0 + 1000),"cwd":"/tmp/ui-infra","origin":{"kind":"human"},"message":{"role":"user","content":"/loop watch the staging rollout"}}),
            json!({"type":"assistant","uuid":"lp-a1","timestamp":iso(t0 + 4000),"message":{"id":"msg_lp1","model":"claude-sonnet-5","stop_reason":"tool_use","content":[{"type":"tool_use","id":"tu_wake","name":"ScheduleWakeup","input":{"delaySeconds":900,"reason":"Check whether the staging rollout finished","prompt":"/loop watch the staging rollout","noop":false}}],"usage":{"input_tokens":700,"output_tokens":50}}}),
            json!({"type":"user","uuid":"lp-r1","timestamp":iso(now() - 120_000),"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_wake","content":"Next wakeup scheduled."}]},"toolUseResult":{"scheduledFor":now() + 780_000,"clampedDelaySeconds":900,"wasClamped":false}}),
            json!({"type":"assistant","uuid":"lp-a2","timestamp":iso(now() - 110_000),"message":{"id":"msg_lp2","model":"claude-sonnet-5","stop_reason":"end_turn","content":[{"type":"text","text":"The rollout is at 60%. I'll check again in 15 minutes."}],"usage":{"input_tokens":20,"output_tokens":15}}}),
            json!({"type":"ai-title","aiTitle":"Watch the staging rollout"}),
        ],
    );
}

/// The release build finishes: its task-notification lands in the
/// transcript, and nothing else will wake the session.
fn finish_build(base: &Path) {
    let path = base.join(format!("projects/-tmp-ui-release/{BUILD}.jsonl"));
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = writeln!(
            f,
            "{}",
            json!({"type":"user","uuid":"rb-n1","timestamp":iso(now()),"origin":{"kind":"task-notification"},"message":{"role":"user","content":"<task-notification>\n<task-id>brel1</task-id>\n<tool-use-id>tu_build</tool-use-id>\n<status>completed</status>\n<summary>Release build finished</summary>\n</task-notification>"}})
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual helper: serves the SPA over a synthetic home"]
async fn serve_a_synthetic_home_for_ui_checks() {
    let port: u16 = std::env::var("MC_UI_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(6799);
    let seconds: u64 = std::env::var("MC_UI_SECONDS")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(120);
    let home = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    fixture(home.path());
    // The processes of the sessions between turns (alive while we serve).
    let sleeper = || {
        tokio::process::Command::new("sleep")
            .arg((seconds + 60).to_string())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    };
    let (build_proc, loop_proc) = (sleeper(), sleeper());
    between_turns(
        home.path(),
        build_proc.id().unwrap(),
        loop_proc.id().unwrap(),
    );
    // Owned sessions run the scripted fake CLI, never the real one.
    let fake =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-claude.py");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755));
    }
    std::env::set_var("MOTHER_CLAUDE_CLI", &fake);
    std::env::remove_var("MOTHER_CLAUDE_SIDECAR");
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(data.path().to_path_buf());
    // The Default's `.claude.json` inside the synthetic home: with `with_base`
    // it would be the real `~/.claude.json`, which moves with the trust
    // opt-in and MCP customizations write.
    let st = Inner::with_bridge_config(
        ClaudeHome::for_account_dir(home.path().to_path_buf()),
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
    if std::env::var("MC_UI_REVEAL").as_deref() == Ok("record") {
        st.reveal.record_only();
        let st = st.clone();
        tokio::spawn(async move {
            let mut shown = 0;
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let recorded = st.reveal.recorded();
                for cmd in &recorded[shown..] {
                    println!("reveal: {cmd:?}");
                }
                shown = recorded.len();
            }
        });
    }
    tokio::spawn(monitor::run(st.clone()));
    hooks::start(&st);
    push::start(&st);
    // Every other subsystem the app starts: the MCP server (its token and
    // the owned sessions' plugin copy, in the synthetic data dir), the
    // browser extension's sweeps and the scheduler.
    mother_claude_lib::mcp::server::start(&st);
    mother_claude_lib::extension::start(&st);
    mother_claude_lib::orchestration::scheduler::start(&st);
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
    let info = json!({
        "base": format!("http://127.0.0.1:{port}"),
        "desktopToken": st.auth.token,
        "hookToken": st.auth.hook_token,
        "dataDir": data.path(),
        "home": home.path(),
        "session": SID,
        "buildSession": BUILD,
        "loopSession": LOOP,
        "workDir": work.path(),
        "pid": std::process::id(),
    });
    match std::env::var("MC_UI_OUT") {
        Ok(out) => std::fs::write(out, info.to_string()).unwrap(),
        Err(_) => println!("{info}"),
    }
    let plugin = PluginPaths::new(data.path());
    write_plugin_files(&plugin).unwrap();
    write_runtime_files(&plugin, port, &st.auth.hook_token).unwrap();
    let trigger = data.path().join("raise-permission");
    let build_done = data.path().join("finish-build");
    // Keep the live session talking.
    let transcript = home
        .path()
        .join(format!("projects/-tmp-ui-demo/{SID}.jsonl"));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut n = 0u32;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if std::fs::remove_file(&build_done).is_ok() {
            finish_build(home.path());
        }
        if std::fs::remove_file(&trigger).is_ok() {
            let script = plugin.plugin.mc_hook.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let body = json!({
                    "hook_event_name": "PermissionRequest", "session_id": SID,
                    "tool_name": "Bash",
                    "tool_input": { "command": "npm test -- --watch=false", "description": "Run the tests" },
                    "permission_suggestions": [{ "type": "addRules", "behavior": "allow",
                        "destination": "localSettings",
                        "rules": [{ "toolName": "Bash", "ruleContent": "npm test:*" }] }]
                })
                .to_string();
                // As Claude Code starts an exec-form hook (PowerShell on Windows).
                let mut cmd = tokio::process::Command::from(
                    mother_claude_lib::claude::hooks::script_command(&script),
                );
                cmd.env_clear();
                if cfg!(windows) {
                    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
                    cmd.env("PATH", format!(r"{root}\System32"))
                        .env("SystemRoot", root);
                } else {
                    cmd.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
                }
                let Ok(mut child) = cmd
                    .args(["PermissionRequest", "300", "decide"])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                else {
                    return;
                };
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(body.as_bytes()).await;
                }
                if let Ok(out) = child.wait_with_output().await {
                    println!("hook answered: {}", String::from_utf8_lossy(&out.stdout));
                }
            });
        }
        n += 1;
        if !n.is_multiple_of(4) {
            continue;
        }
        let n = n / 4;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&transcript) {
            let _ = writeln!(
                f,
                "{}",
                json!({"type":"assistant","uuid":format!("ui-live-{n}"),"timestamp":iso(now()),"message":{"id":format!("msg_live{n}"),"model":"claude-haiku-4-5","content":[{"type":"text","text":format!("Progress update {n}.")}],"usage":{"input_tokens":5,"output_tokens":5}}})
            );
        }
    }
}
