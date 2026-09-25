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
//! `MC_UI_OUT` receives `{base, desktopToken, hookToken, dataDir, session}`
//! (throwaway values of this run). A foreign VS Code session run by this
//! process is live, with a transcript that grows every few seconds. Creating
//! `<dataDir>/raise-permission` makes the session's hook plugin (the real
//! `mc-hook` script, run as a child of this process) hold a Bash permission
//! request, as a foreign session would. Owned sessions spawned from the UI
//! run the scripted fake CLI (`tests/fixtures/fake-claude.py`, no tokens
//! spent): send `hello`, `bash`, `ask` or `plan` to see each kind of card.

use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use mother_claude_lib::bridge::BridgeConfig;
use mother_claude_lib::claude::hooks::{write_plugin_files, write_runtime_files, PluginPaths};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, hooks, monitor, push, Ingress};
use mother_claude_lib::state::{Inner, ServerConfig};
use serde_json::json;

const SID: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";
const ENDED: &str = "1a2b3c4d-5e6f-4a0b-8c1d-2e3f4a5b6c7d";

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
    let st = Inner::with_bridge_config(
        ClaudeHome::with_base(home.path()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port,
        },
        auth,
        BridgeConfig {
            enabled: true,
            port: None,
            require_token: false,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    tokio::spawn(monitor::run(st.clone()));
    hooks::start(&st);
    push::start(&st);
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
        "session": SID,
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
    // Keep the live session talking.
    let transcript = home
        .path()
        .join(format!("projects/-tmp-ui-demo/{SID}.jsonl"));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut n = 0u32;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(1)).await;
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
                let Ok(mut child) = tokio::process::Command::new(script)
                    .args(["PermissionRequest", "300", "decide"])
                    .env_clear()
                    .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
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
