//! Owned-session control (Path B "full") against a FAKE `claude`
//! (`tests/fixtures/fake-claude.py`, selected with `MOTHER_CLAUDE_CLI`): the
//! spawn line and environment, the nested control protocol in both
//! directions, approvals / questions / plans / elicitations, stream frames,
//! queued messages, live control requests with CLI errors, fork-on-takeover,
//! images, the bypass guard, and the HTTP routes on top.
//!
//! Nothing here touches `~/.claude` or runs the real CLI. The one live check
//! against the installed CLI is `#[ignore]`d (it spends a few cents).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};
use std::time::Duration;

use axum::http::HeaderMap;
use base64::Engine as _;
use mother_claude_lib::claude::model::{
    Delivery, EffortSource, ImageInput, PendingKind, PendingOutcome, PendingSource,
    PlanUsageSource, ResolverKind, SessionSummary, StreamKind, ThinkingInfo, ThinkingMode,
    ThinkingSource, UltracodeSource, UltracodeState,
};
use mother_claude_lib::claude::owned::{self, Caller, ContinueRequest, SpawnRequest};
use mother_claude_lib::claude::usage;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::pending::{self, Decision, RespondError};
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::context::{ClientContext, Ingress, Principal};
use mother_claude_lib::server::ws::{self, ServerEvent};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use serde_json::{json, Value};

static SETUP: Once = Once::new();

/// Point `MOTHER_CLAUDE_CLI` at the fake, and give the test process the
/// inherited variables the spawn must scrub (or keep).
fn setup() {
    SETUP.call_once(|| {
        let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-claude.py");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755));
        }
        std::env::set_var("MOTHER_CLAUDE_CLI", &fake);
        std::env::remove_var("MOTHER_CLAUDE_SIDECAR");
        // What a parent Claude Code session would leak into us.
        std::env::set_var("CLAUDECODE", "1");
        std::env::set_var("CLAUDE_CODE_ENTRYPOINT", "claude-vscode");
        std::env::set_var("CLAUDE_CODE_SESSION_ID", "parent-session");
        std::env::set_var("MOTHER_CLAUDE_TOKEN", "must-not-leak");
        // Provider config that must survive the scrub.
        std::env::set_var("CLAUDE_CODE_USE_BEDROCK", "keep-me");
    });
}

struct Fixture {
    st: AppState,
    _home: tempfile::TempDir,
    cwd: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        setup();
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let st = Inner::new(
            ClaudeHome::with_base(home.path()),
            ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            Auth::ephemeral(),
        );
        Self {
            st,
            _home: home,
            cwd,
        }
    }

    fn cwd(&self) -> String {
        self.cwd.path().to_string_lossy().into_owned()
    }

    fn log(&self) -> Vec<Value> {
        read_log(self.cwd.path())
    }

    /// Everything the fake read from stdin.
    fn received(&self) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter_map(|l| l.get("in").cloned())
            .collect()
    }

    fn logged(&self, key: &str) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter_map(|l| l.get(key).cloned())
            .collect()
    }

    async fn spawn(&self, caller: Caller, req: SpawnRequest) -> String {
        owned::spawn_as(
            &self.st,
            caller,
            SpawnRequest {
                cwd: self.cwd(),
                ..req
            },
        )
        .await
        .unwrap()
    }

    async fn spawn_text(&self, prompt: &str) -> String {
        self.spawn(
            MAC,
            SpawnRequest {
                prompt: Some(prompt.into()),
                ..Default::default()
            },
        )
        .await
    }
}

fn read_log(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("fake-claude.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

const MAC: Caller = Caller {
    may_dangerous: true,
    desktop: true,
};

/// Desktop without dangerous rights (e.g. a local script).
const PLAIN: Caller = Caller {
    may_dangerous: false,
    desktop: true,
};

fn desktop() -> ClientContext {
    let mut h = HeaderMap::new();
    h.insert("host", "127.0.0.1:6725".parse().unwrap());
    let mut c = ClientContext::new(Ingress::Loopback, "127.0.0.1:9".parse().unwrap(), &h, false);
    c.principal = Principal::Desktop;
    c
}

fn phone() -> ClientContext {
    let mut h = HeaderMap::new();
    h.insert("host", "192.168.1.2:6725".parse().unwrap());
    let mut c = ClientContext::new(Ingress::Lan, "192.168.1.9:9".parse().unwrap(), &h, false);
    c.principal = Principal::Device {
        id: "d1".into(),
        allow_dangerous: false,
    };
    c
}

async fn wait_for<F: FnMut() -> bool>(what: &str, mut f: F) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn wait_pending(st: &AppState, id: &str, kind: PendingKind) -> pending::PendingItem {
    let mut found = None;
    wait_for("a pending item", || {
        found = st.pending.list(id).into_iter().find(|i| i.kind == kind);
        found.is_some()
    })
    .await;
    found.unwrap()
}

async fn wait_turn_done(st: &AppState, id: &str) {
    wait_for("the turn to finish", || {
        owned::live_state(st, id).is_some_and(|l| l.started && !l.turn_active)
    })
    .await;
}

fn drain(rx: &mut tokio::sync::broadcast::Receiver<Arc<ServerEvent>>) -> Vec<Arc<ServerEvent>> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

fn png() -> ImageInput {
    let bytes = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 24]].concat();
    ImageInput {
        media_type: "image/png".into(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

#[tokio::test]
async fn spawn_line_env_initialize_and_stream() {
    let f = Fixture::new();
    let mut rx = f.st.bus.subscribe();
    let id = f
        .spawn(
            MAC,
            SpawnRequest {
                prompt: Some("hello".into()),
                model: Some("haiku".into()),
                effort: Some("low".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(owned::is_owned(&f.st, &id));
    assert!(owned::has_handle(&f.st, &id));
    assert!(f.st.is_owned(&id).await);
    wait_turn_done(&f.st, &id).await;

    // The spawn line mirrors VS Code, with an explicit mode and our id.
    let first = f.log()[0].clone();
    let argv: Vec<String> = serde_json::from_value(first["argv"].clone()).unwrap();
    let line = argv.join(" ");
    for part in [
        "-p --output-format stream-json --input-format stream-json --verbose",
        "--permission-prompt-tool stdio",
        "--include-partial-messages --replay-user-messages",
        "--setting-sources user,project,local",
        "--permission-mode default",
        "--allow-dangerously-skip-permissions",
        "--model haiku --effort low",
    ] {
        assert!(line.contains(part), "{part:?} missing from {line}");
    }
    assert!(line.ends_with(&format!("--session-id {id}")), "{line}");
    assert!(!line.contains("hello"), "the prompt never goes in argv");

    // Environment: parent session variables and our token scrubbed, provider
    // config kept, owned marker set, CLAUDE_CONFIG_DIR not invented.
    let env = &first["env"];
    assert!(env["CLAUDECODE"].is_null());
    assert!(env["CLAUDE_CODE_ENTRYPOINT"].is_null());
    assert!(env["CLAUDE_CODE_SESSION_ID"].is_null());
    assert!(env["MOTHER_CLAUDE_TOKEN"].is_null());
    assert_eq!(env["MOTHER_CLAUDE_OWNED"], "1");
    assert_eq!(env["CLAUDE_CODE_USE_BEDROCK"], "keep-me");
    assert_eq!(
        env["CLAUDE_CONFIG_DIR"].as_str(),
        std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref()
    );
    // The Default account's part of the environment is what the app found
    // (docs/ACCOUNTS.md D5): no directory invented, overrides kept.
    let account = &first["account"];
    assert_eq!(
        account["configDir"].as_str(),
        std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref()
    );
    assert_eq!(
        account["oauthToken"],
        std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").is_some()
    );
    assert!(
        account["overrides"]
            .as_array()
            .unwrap()
            .contains(&json!("CLAUDE_CODE_USE_BEDROCK")),
        "{account}"
    );

    // `initialize` goes first, then the user message with a uuid.
    let got = f.received();
    assert_eq!(got[0]["type"], "control_request");
    assert_eq!(got[0]["request"]["subtype"], "initialize");
    assert_eq!(got[1]["type"], "user");
    assert!(got[1]["parent_tool_use_id"].is_null());
    assert!(owned::is_uuid(got[1]["uuid"].as_str().unwrap()));
    assert_eq!(got[1]["message"]["content"][0]["text"], "hello");

    // Cached from initialize, `/`-prefixed.
    let commands = owned::commands(&f.st, &id).await.unwrap();
    assert_eq!(commands[0].name, "/compact");
    let live = owned::live_state(&f.st, &id).unwrap();
    assert_eq!(live.model.as_deref(), Some("haiku"));
    assert_eq!(live.permission_mode.as_deref(), Some("default"));
    assert!(live.bypass_available);

    // Stream frames carry messageId + index; deltas are coalesced.
    let events = drain(&mut rx);
    let text: String = events
        .iter()
        .filter_map(|e| match &**e {
            ServerEvent::Stream { session, frame } if session == &id => Some(frame.clone()),
            _ => None,
        })
        .filter(|f| f.kind == StreamKind::Text)
        .map(|f| {
            assert!(f.message_id.as_deref().unwrap().starts_with("msg_"));
            assert_eq!(f.index, Some(0));
            f.delta.unwrap_or_default()
        })
        .collect();
    assert_eq!(text, "Hello there (0 images)");
    assert!(events.iter().any(|e| matches!(&**e,
        ServerEvent::Notice { text, .. } if text.contains("finished a turn"))));

    // The snapshot row is owned, with the launch mode and model.
    let s = f.st.find_session(&id).await.unwrap();
    assert!(s.owned && s.live);
    assert_eq!(s.permission_mode.as_deref(), Some("default"));
    assert_eq!(s.version.as_deref(), Some("9.9.9-fake"));

    // Stop: SIGTERM, reaper clears ownership.
    owned::kill(&f.st, &id).await.unwrap();
    wait_for("the reaper", || !owned::has_handle(&f.st, &id)).await;
    wait_for("unmark", || !owned::is_owned(&f.st, &id)).await;
    let s = f.st.find_session(&id).await.unwrap();
    assert!(!s.owned && !s.live);
}

#[tokio::test]
async fn approvals_questions_and_plans_round_trip() {
    let f = Fixture::new();
    let mut rx = f.st.bus.subscribe();
    let id = f.spawn_text("bash").await;

    // Permission with the session-scoped "always allow" copy first.
    let item = wait_pending(&f.st, &id, PendingKind::Permission).await;
    assert_eq!(item.source, PendingSource::Owned);
    assert_eq!(item.tool_name.as_deref(), Some("Bash"));
    assert_eq!(item.detail.as_deref(), Some("Create file x1"));
    assert!(!item.dangerous);
    let dests: Vec<Option<&str>> = item
        .suggestions
        .iter()
        .map(|s| s.destination.as_deref())
        .collect();
    assert_eq!(
        dests,
        vec![Some("session"), Some("localSettings"), Some("session")]
    );
    assert!(!item.suggestions[0].standing && item.suggestions[1].standing);
    // A phone may not pick the settings-file grant, but may pick the session one.
    let mut standing = Decision::allow();
    standing.suggestion = Some(1);
    assert!(matches!(
        pending::respond(&f.st, &id, &item.id, standing, &phone()).await,
        Err(RespondError::Forbidden(_))
    ));
    let mut session_rule = Decision::allow();
    session_rule.suggestion = Some(0);
    pending::respond(&f.st, &id, &item.id, session_rule, &phone())
        .await
        .unwrap();
    wait_turn_done(&f.st, &id).await;
    let decision = f.logged("decision").pop().unwrap();
    assert_eq!(decision["behavior"], "allow");
    assert_eq!(decision["updatedInput"]["command"], "touch x1");
    assert_eq!(decision["updatedPermissions"][0]["destination"], "session");
    assert!(decision["toolUseID"]
        .as_str()
        .unwrap()
        .starts_with("toolu_"));
    assert!(f.cwd.path().join("x1").exists());
    // A late second answer: 409 with the resolution remembered.
    assert_eq!(
        pending::respond(&f.st, &id, &item.id, Decision::allow(), &desktop())
            .await
            .unwrap_err(),
        RespondError::AlreadyDecided
    );
    let r = f.st.pending.resolution(&item.id).unwrap();
    assert_eq!(r.outcome, PendingOutcome::Allowed);
    assert_eq!(r.by.unwrap().kind, ResolverKind::Device);
    let resolved: Vec<_> = drain(&mut rx)
        .into_iter()
        .filter_map(|e| match &*e {
            ServerEvent::PendingResolved(r) => Some(r.clone()),
            _ => None,
        })
        .collect();
    assert!(resolved
        .iter()
        .any(|r| r.id == item.id && r.outcome == PendingOutcome::Allowed));

    // Native AskUserQuestion: answers go back in updatedInput.
    owned::send(&f.st, &id, "ask", &[]).await.unwrap();
    let q = wait_pending(&f.st, &id, PendingKind::Question).await;
    assert_eq!(q.title, "Favourite colour?");
    assert_eq!(q.questions.as_ref().unwrap()[0].options.len(), 2);
    let mut answer = Decision::allow();
    answer.answers = Some([("Favourite colour?".to_string(), "Blue".to_string())].into());
    pending::respond(&f.st, &id, &q.id, answer, &phone())
        .await
        .unwrap();
    wait_turn_done(&f.st, &id).await;
    let d = f.logged("decision").pop().unwrap();
    assert_eq!(d["updatedInput"]["answers"]["Favourite colour?"], "Blue");
    assert_eq!(
        f.st.pending.resolution(&q.id).unwrap().outcome,
        PendingOutcome::Answered
    );

    // ExitPlanMode: approve and switch to acceptEdits.
    owned::send(&f.st, &id, "plan", &[]).await.unwrap();
    let p = wait_pending(&f.st, &id, PendingKind::Plan).await;
    assert_eq!(p.plan.as_deref(), Some("1. Create the file\n2. Done"));
    let mut approve = Decision::allow();
    approve.mode = Some("acceptEdits".into());
    pending::respond(&f.st, &id, &p.id, approve, &desktop())
        .await
        .unwrap();
    wait_turn_done(&f.st, &id).await;
    let d = f.logged("decision").pop().unwrap();
    assert_eq!(d["updatedPermissions"][0]["type"], "setMode");
    assert_eq!(d["updatedPermissions"][0]["mode"], "acceptEdits");
    wait_for("system/status", || {
        owned::live_state(&f.st, &id)
            .and_then(|l| l.permission_mode)
            .as_deref()
            == Some("acceptEdits")
    })
    .await;

    // A write to .claude/ is classified dangerous server-side.
    owned::send(&f.st, &id, "danger", &[]).await.unwrap();
    let dz = wait_pending(&f.st, &id, PendingKind::Permission).await;
    assert!(dz.dangerous);
    assert!(dz.dangerous_reason.unwrap().contains(".claude"));
    assert!(matches!(
        pending::respond(&f.st, &id, &dz.id, Decision::allow(), &phone()).await,
        Err(RespondError::Forbidden(_))
    ));
    let mut deny = Decision::deny(Some("not that file".into()));
    deny.interrupt = true;
    pending::respond(&f.st, &id, &dz.id, deny, &phone())
        .await
        .unwrap();
    wait_turn_done(&f.st, &id).await;
    let d = f.logged("decision").pop().unwrap();
    assert_eq!(d["behavior"], "deny");
    assert_eq!(d["message"], "not that file");
    assert_eq!(d["interrupt"], true);
    owned::kill(&f.st, &id).await.unwrap();
}

#[tokio::test]
async fn cancels_elicitations_and_unknown_requests_are_answered() {
    let f = Fixture::new();
    let mut rx = f.st.bus.subscribe();
    let id = f.spawn_text("cancel").await;
    let item = wait_pending(&f.st, &id, PendingKind::Permission).await;
    // control_cancel_request withdraws the card; no reply is sent.
    wait_for("the withdrawal", || f.st.pending.list(&id).is_empty()).await;
    wait_turn_done(&f.st, &id).await;
    assert_eq!(
        f.st.pending.resolution(&item.id).unwrap().outcome,
        PendingOutcome::Cancelled
    );
    assert!(f.logged("cancel_reply").pop().unwrap().is_null());
    assert!(drain(&mut rx).iter().any(|e| matches!(&**e,
        ServerEvent::PendingResolved(r) if r.id == item.id)));

    // request_user_dialog → cancelled; unknown subtypes → error.
    owned::send(&f.st, &id, "dialog", &[]).await.unwrap();
    wait_turn_done(&f.st, &id).await;
    wait_for("the dialog log", || !f.logged("dialog").is_empty()).await;
    let log = f.log();
    let entry = log.iter().find(|l| l.get("dialog").is_some()).unwrap();
    assert_eq!(entry["dialog"]["response"]["subtype"], "success");
    assert_eq!(
        entry["dialog"]["response"]["response"]["behavior"],
        "cancelled"
    );
    assert_eq!(entry["unknown"]["response"]["subtype"], "error");
    assert_eq!(entry["unknown"]["response"]["error"], "unsupported");

    // Elicitation with nobody watching: declined at once.
    owned::send(&f.st, &id, "elicit", &[]).await.unwrap();
    wait_turn_done(&f.st, &id).await;
    wait_for("the elicitation", || !f.logged("decision").is_empty()).await;
    assert_eq!(
        f.logged("decision").pop().unwrap(),
        json!({ "action": "decline" })
    );
    owned::kill(&f.st, &id).await.unwrap();
}

#[tokio::test]
async fn a_cancel_buffered_behind_its_request_raises_no_card() {
    let f = Fixture::new();
    let mut rx = f.st.bus.subscribe();
    let id = f.spawn_text("cancel-fast").await;
    wait_turn_done(&f.st, &id).await;
    wait_for("the log", || !f.logged("cancel_fast_reply").is_empty()).await;
    // The cancel reached the request before its handler ran: no card was
    // raised (or left behind), and nothing was answered.
    assert!(f.st.pending.list(&id).is_empty());
    assert!(!drain(&mut rx)
        .iter()
        .any(|e| matches!(&**e, ServerEvent::PendingResolved(r) if r.session == id)));
    assert!(f.logged("cancel_fast_reply").pop().unwrap().is_null());
    owned::kill(&f.st, &id).await.unwrap();
}

#[tokio::test]
async fn live_control_requests_and_cli_errors() {
    let f = Fixture::new();
    // Launched by a caller without dangerous rights: no bypass available.
    let id = f
        .spawn(
            PLAIN,
            SpawnRequest {
                prompt: Some("hello".into()),
                ..Default::default()
            },
        )
        .await;
    wait_turn_done(&f.st, &id).await;
    let argv = f.log()[0]["argv"].to_string();
    assert!(!argv.contains("--allow-dangerously-skip-permissions"));

    owned::set_model(&f.st, &id, "opus").await.unwrap();
    assert_eq!(
        owned::live_state(&f.st, &id).unwrap().model.as_deref(),
        Some("opus")
    );
    owned::set_mode(&f.st, &id, "plan").await.unwrap();
    let e = owned::set_mode(&f.st, &id, "auto").await.unwrap_err();
    assert_eq!(e.status.as_u16(), 422);
    assert_eq!(e.code, "cli_error");
    assert_eq!(e.body()["error"]["errorCode"], "auto_mode_model");
    assert_eq!(
        owned::set_mode(&f.st, &id, "bypassPermissions")
            .await
            .unwrap_err()
            .code,
        "bypass_unavailable"
    );
    owned::set_effort(&f.st, &id, "high").await.unwrap();
    owned::rename(&f.st, &id, "  My\u{7} session ")
        .await
        .unwrap();
    let usage = owned::context_usage(&f.st, &id).await.unwrap();
    assert_eq!(usage["totalTokens"], 1234);
    let mcp = owned::mcp_status(&f.st, &id).await.unwrap();
    assert_eq!(mcp["servers"][0]["name"], "gh");
    assert!(
        mcp["servers"][0].get("config").is_none(),
        "config is never echoed"
    );

    let sent: Vec<Value> = f
        .received()
        .into_iter()
        .filter(|l| l["type"] == "control_request")
        .map(|l| l["request"].clone())
        .collect();
    assert!(sent.contains(&json!({ "subtype": "set_model", "model": "opus" })));
    assert!(sent.contains(&json!({ "subtype": "set_permission_mode", "mode": "plan" })));
    assert!(sent.contains(&json!({ "subtype": "apply_flag_settings",
        "settings": { "effortLevel": "high" } })));
    assert!(sent.contains(&json!({ "subtype": "rename_session", "title": "My session" })));
    assert!(!sent.iter().any(|r| r["mode"] == "bypassPermissions"));
    let s = f.st.find_session(&id).await.unwrap();
    assert_eq!(s.name, "My session");
    assert_eq!(s.effort.as_deref(), Some("high"));

    // Tasks: reported live, stoppable.
    owned::send(&f.st, &id, "task", &[]).await.unwrap();
    wait_turn_done(&f.st, &id).await;
    wait_for("the task", || !owned::tasks(&f.st, &id).0.is_empty()).await;
    let (agents, _) = owned::tasks(&f.st, &id);
    assert_eq!(agents[0].agent_type.as_deref(), Some("Explore"));
    assert!(agents[0].can_stop);
    assert_eq!(
        owned::stop_task(&f.st, &id, "../../x")
            .await
            .unwrap_err()
            .code,
        "invalid_task_id"
    );
    assert_eq!(
        owned::stop_task(&f.st, &id, "nope").await.unwrap_err().code,
        "cli_error"
    );
    owned::stop_task(&f.st, &id, "t1").await.unwrap();
    wait_for("the stopped task", || {
        owned::tasks(&f.st, &id)
            .0
            .first()
            .is_some_and(|a| !a.can_stop)
    })
    .await;

    // Interrupt a running turn.
    owned::send(&f.st, &id, "slow", &[]).await.unwrap();
    owned::interrupt(&f.st, &id).await.unwrap();
    wait_turn_done(&f.st, &id).await;

    // A failed turn records the error.
    owned::send(&f.st, &id, "fail", &[]).await.unwrap();
    wait_for("the error", || {
        owned::live_state(&f.st, &id).is_some_and(|l| l.last_error.is_some())
    })
    .await;
    owned::kill(&f.st, &id).await.unwrap();
    wait_for("exit", || !owned::has_handle(&f.st, &id)).await;
    // Control requests to a dead session are a clean 409.
    assert_eq!(
        owned::interrupt(&f.st, &id).await.unwrap_err().code,
        "owned_session_unavailable"
    );
}

/// The control requests (bodies) the fake received.
fn requests(f: &Fixture) -> Vec<Value> {
    f.received()
        .into_iter()
        .filter(|l| l["type"] == "control_request")
        .map(|l| l["request"].clone())
        .collect()
}

async fn session(st: &AppState, id: &str) -> SessionSummary {
    st.find_session(id).await.unwrap()
}

#[tokio::test]
async fn settings_are_read_back_and_switched_without_slash_commands() {
    let f = Fixture::new();
    let id = f
        .spawn(
            MAC,
            SpawnRequest {
                prompt: Some("hello".into()),
                model: Some("opus".into()),
                effort: Some("max".into()),
                thinking: Some("off".into()),
                ..Default::default()
            },
        )
        .await;
    wait_turn_done(&f.st, &id).await;
    assert_eq!(f.log()[0]["env"]["MAX_THINKING_TOKENS"], "0");
    // `get_settings` follows `initialize`: what the CLI really applies.
    wait_for("the applied settings", || {
        owned::live_state(&f.st, &id).is_some_and(|l| l.applied.is_some())
    })
    .await;
    let mut s = session(&f.st, &id).await;
    for _ in 0..100 {
        if s.effort_source == Some(EffortSource::Applied) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        s = session(&f.st, &id).await;
    }
    assert_eq!(s.effort.as_deref(), Some("max"));
    assert_eq!(s.effort_source, Some(EffortSource::Applied));
    assert_eq!(s.ultracode, Some(UltracodeState::Off));
    assert_eq!(s.ultracode_source, Some(UltracodeSource::Applied));
    assert_eq!(
        s.thinking,
        Some(ThinkingInfo {
            mode: ThinkingMode::Off,
            source: ThinkingSource::Control,
            budget: None
        })
    );
    assert!(s.control.set_thinking && s.control.set_ultracode);

    // A model without `max` runs it as `high`, as the CLI applies it…
    owned::set_model(&f.st, &id, "sonnet").await.unwrap();
    let s = session(&f.st, &id).await;
    assert_eq!(s.effort.as_deref(), Some("high"));
    assert_eq!(s.effort_source, Some(EffortSource::Applied));
    // …and the ultracode switch sends its key alone: the reduced level is
    // never pinned over the `max` asked for.
    owned::set_ultracode(&f.st, &id, true).await.unwrap();
    assert!(
        requests(&f).contains(&json!({ "subtype": "apply_flag_settings",
        "settings": { "ultracode": true } }))
    );
    // This model cannot run it: requested but unavailable, and the switch
    // stays to withdraw it.
    let s = session(&f.st, &id).await;
    assert_eq!(s.ultracode, Some(UltracodeState::Unavailable));
    assert_eq!(s.ultracode_source, Some(UltracodeSource::Applied));
    assert!(s.control.set_ultracode);
    // Back on a model with both: max again, ultracode on.
    owned::set_model(&f.st, &id, "opus").await.unwrap();
    let s = session(&f.st, &id).await;
    assert_eq!(
        s.effort.as_deref(),
        Some("max"),
        "the level asked for stood"
    );
    assert_eq!(s.ultracode, Some(UltracodeState::On));
    // An effort change keeps ultracode on (session flag layer, never /effort).
    owned::set_effort(&f.st, &id, "high").await.unwrap();
    assert!(
        requests(&f).contains(&json!({ "subtype": "apply_flag_settings",
        "settings": { "effortLevel": "high", "ultracode": true } }))
    );
    let s = session(&f.st, &id).await;
    assert_eq!(s.effort.as_deref(), Some("high"));
    assert_eq!(s.effort_source, Some(EffortSource::Applied));
    assert_eq!(s.ultracode, Some(UltracodeState::On));

    // Thinking on: adaptive on an adaptive-capable model; then off again.
    owned::set_thinking(&f.st, &id, true).await.unwrap();
    let s = session(&f.st, &id).await;
    assert_eq!(
        s.thinking,
        Some(ThinkingInfo {
            mode: ThinkingMode::Adaptive,
            source: ThinkingSource::Control,
            budget: None
        })
    );
    owned::set_thinking(&f.st, &id, false).await.unwrap();
    assert_eq!(
        session(&f.st, &id).await.thinking.map(|t| t.mode),
        Some(ThinkingMode::Off)
    );
    let sent = requests(&f);
    assert!(sent.contains(&json!({ "subtype": "set_max_thinking_tokens",
        "max_thinking_tokens": 31_999 })));
    assert!(sent.contains(&json!({ "subtype": "set_max_thinking_tokens",
        "max_thinking_tokens": 0 })));

    // A model that cannot run ultracode leaves it requested but
    // unavailable (the switch withdraws it), and it has no effort: the CLI
    // says so (`applied`, no level).
    owned::set_model(&f.st, &id, "haiku").await.unwrap();
    let s = session(&f.st, &id).await;
    assert_eq!(s.ultracode, Some(UltracodeState::Unavailable));
    assert_eq!(s.effort, None);
    assert_eq!(s.effort_source, Some(EffortSource::Applied));
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["effortSource"], "applied");
    assert!(v.get("effort").is_none());
    assert!(s.control.set_ultracode && s.control.set_thinking);
    owned::set_ultracode(&f.st, &id, false).await.unwrap();
    let s = session(&f.st, &id).await;
    assert_eq!(s.ultracode, Some(UltracodeState::Off));
    assert!(!s.control.set_ultracode, "nothing left to switch here");
    // No slash command was ever typed.
    assert!(!f
        .received()
        .iter()
        .filter(|l| l["type"] == "user")
        .any(|l| l.to_string().contains("\"/")));

    // Plan usage for free from the stream…
    owned::send(&f.st, &id, "limits", &[]).await.unwrap();
    wait_turn_done(&f.st, &id).await;
    let u = usage::current(&f.st);
    assert_eq!(u.source, Some(PlanUsageSource::Stream));
    assert_eq!(u.windows[0].key, "five_hour");
    assert_eq!(u.windows[0].pct, 42.4);
    assert_eq!(u.windows[0].resets_at, Some(4_070_908_800_000));
    assert_eq!(u.windows[1].key, "seven_day");
    // …and a refresh reads `get_usage` through this session.
    usage::start(&f.st);
    assert!(usage::request_refresh(&f.st).unwrap().refreshing);
    for _ in 0..250 {
        if usage::current(&f.st).source == Some(PlanUsageSource::Live) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let u = usage::current(&f.st);
    assert_eq!(u.source, Some(PlanUsageSource::Live));
    assert!(!u.refreshing);
    assert_eq!(u.windows.len(), 2, "codename windows never come out");
    assert_eq!(u.windows[0].pct, 42.0);
    assert!(requests(&f).contains(&json!({ "subtype": "get_usage", "skip_behaviors": true })));
    assert!(usage::request_refresh(&f.st).is_err(), "one read a minute");
    owned::kill(&f.st, &id).await.unwrap();
}

#[tokio::test]
async fn queued_messages_can_be_taken_back() {
    let f = Fixture::new();
    let id = f.spawn_text("slow").await;
    wait_for("the first turn", || !f.received().is_empty()).await;
    let r = owned::send(&f.st, &id, "hello", &[]).await.unwrap();
    assert_eq!(r.delivery, Delivery::Queued);
    assert_eq!(owned::queued(&f.st, &id).len(), 1);
    assert!(owned::queued(&f.st, &id)[0].cancellable);
    owned::cancel_queued(&f.st, &id, &r.message_id)
        .await
        .unwrap();
    assert!(owned::queued(&f.st, &id).is_empty());
    assert_eq!(
        owned::cancel_queued(&f.st, &id, &r.message_id)
            .await
            .unwrap_err()
            .code,
        "already_delivered"
    );
    let sent: Vec<Value> = f
        .received()
        .into_iter()
        .filter(|l| l["request"]["subtype"] == "cancel_async_message")
        .collect();
    assert_eq!(sent[0]["request"]["message_uuid"], r.message_id);

    // A second queued message is answered after the slow turn.
    let r2 = owned::send(&f.st, &id, "hello", &[]).await.unwrap();
    assert_eq!(r2.delivery, Delivery::Queued);
    wait_for("the queued message to be answered", || {
        owned::queued(&f.st, &id).is_empty()
    })
    .await;
    wait_turn_done(&f.st, &id).await;
    let r3 = owned::send(&f.st, &id, "hello", &[]).await.unwrap();
    assert_eq!(r3.delivery, Delivery::Sent);
    owned::kill(&f.st, &id).await.unwrap();
}

#[tokio::test]
async fn switching_to_bypass_takes_back_what_restricted_callers_queued() {
    let f = Fixture::new();
    let id = f.spawn_text("slow").await;
    wait_for("the first turn", || !f.received().is_empty()).await;
    let from_phone = owned::send_as(&f.st, false, &id, "from the phone", &[])
        .await
        .unwrap();
    let from_mac = owned::send_as(&f.st, true, &id, "from the Mac", &[])
        .await
        .unwrap();
    assert_eq!(from_phone.delivery, Delivery::Queued);
    assert_eq!(from_mac.delivery, Delivery::Queued);
    owned::set_mode(&f.st, &id, "bypassPermissions")
        .await
        .unwrap();
    let cancelled: Vec<Value> = f
        .received()
        .into_iter()
        .filter(|l| l["request"]["subtype"] == "cancel_async_message")
        .map(|l| l["request"]["message_uuid"].clone())
        .collect();
    assert_eq!(cancelled, vec![Value::from(from_phone.message_id.clone())]);
    let left: Vec<String> = owned::queued(&f.st, &id)
        .into_iter()
        .map(|q| q.id)
        .collect();
    assert_eq!(left, vec![from_mac.message_id]);
    // The cancel reached the CLI before the mode switch did.
    let order: Vec<String> = f
        .received()
        .into_iter()
        .filter_map(|l| l["request"]["subtype"].as_str().map(str::to_string))
        .filter(|s| s == "cancel_async_message" || s == "set_permission_mode")
        .collect();
    assert_eq!(order, vec!["cancel_async_message", "set_permission_mode"]);
    owned::kill(&f.st, &id).await.unwrap();
}

#[tokio::test]
async fn images_are_content_blocks() {
    let f = Fixture::new();
    let id = f
        .spawn(
            MAC,
            SpawnRequest {
                images: vec![png()],
                ..Default::default()
            },
        )
        .await;
    wait_turn_done(&f.st, &id).await;
    owned::send(&f.st, &id, "hello", &[png(), png()])
        .await
        .unwrap();
    wait_turn_done(&f.st, &id).await;
    let users: Vec<Value> = f
        .received()
        .into_iter()
        .filter(|l| l["type"] == "user")
        .collect();
    let first = users[0]["message"]["content"].as_array().unwrap().clone();
    assert_eq!(first.len(), 1, "no empty text block");
    assert_eq!(first[0]["type"], "image");
    assert_eq!(first[0]["source"]["type"], "base64");
    assert_eq!(first[0]["source"]["media_type"], "image/png");
    let second = users[1]["message"]["content"].as_array().unwrap().clone();
    assert_eq!(second.len(), 3);
    assert_eq!(second[0]["text"], "hello");
    let bad = ImageInput {
        media_type: "image/jpeg".into(),
        data: png().data,
    };
    assert_eq!(
        owned::send(&f.st, &id, "hello", &[bad])
            .await
            .unwrap_err()
            .code,
        "invalid_image"
    );
    owned::kill(&f.st, &id).await.unwrap();
}

#[tokio::test]
async fn a_session_that_comes_up_in_bypass_is_stopped_for_plain_callers() {
    let f = Fixture::new();
    let id = f
        .spawn(
            PLAIN,
            SpawnRequest {
                prompt: Some("hello".into()),
                model: Some("fake-bypass".into()),
                ..Default::default()
            },
        )
        .await;
    wait_for("the bypass guard", || !owned::has_handle(&f.st, &id)).await;
    assert!(!owned::is_owned(&f.st, &id));

    // A caller allowed dangerous actions keeps it.
    let g = Fixture::new();
    let id = g
        .spawn(
            MAC,
            SpawnRequest {
                prompt: Some("hello".into()),
                model: Some("fake-bypass".into()),
                ..Default::default()
            },
        )
        .await;
    wait_turn_done(&g.st, &id).await;
    assert!(owned::has_handle(&g.st, &id));
    assert!(owned::runs_unapproved(&g.st, &id).await);
    owned::kill(&g.st, &id).await.unwrap();

    // Launch policy: bypass needs L, and the mode is always explicit.
    let e = owned::spawn_as(
        &f.st,
        PLAIN,
        SpawnRequest {
            cwd: f.cwd(),
            permission_mode: Some("bypassPermissions".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "dangerous_blocked");
    let e = owned::spawn_as(
        &f.st,
        MAC,
        SpawnRequest {
            cwd: "relative/dir".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "invalid_cwd");
}

#[tokio::test]
async fn continue_forks_when_live_elsewhere_and_resumes_when_ended() {
    let f = Fixture::new();
    let original = "0b0e9c5e-3a52-4d4c-9d59-5f0b9a1e2c3d";
    // Seen live in another process.
    ws::upsert_session(
        &f.st,
        SessionSummary {
            id: original.into(),
            cwd: f.cwd(),
            live: true,
            ..Default::default()
        },
    )
    .await;
    let new_id = owned::continue_as(
        &f.st,
        PLAIN,
        original,
        &f.cwd(),
        ContinueRequest {
            prompt: Some("hello".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_ne!(new_id, original);
    assert!(owned::is_uuid(&new_id));
    wait_turn_done(&f.st, &new_id).await;
    let line = f.log()[0]["argv"].to_string();
    assert!(line.contains(&format!(
        "\"--resume\",\"{original}\",\"--fork-session\",\"--session-id\",\"{new_id}\""
    )));
    assert!(line.contains("\"--permission-mode\",\"default\""));
    owned::kill(&f.st, &new_id).await.unwrap();

    // Ended: resumes in place, same id.
    let g = Fixture::new();
    let ended = "1c1e9c5e-3a52-4d4c-9d59-5f0b9a1e2c3d";
    let same = owned::continue_as(&g.st, PLAIN, ended, &g.cwd(), ContinueRequest::default())
        .await
        .unwrap();
    assert_eq!(same, ended);
    wait_for("argv", || !g.log().is_empty()).await;
    let line = g.log()[0]["argv"].to_string();
    assert!(line.contains(&format!("\"--resume\",\"{ended}\"")));
    assert!(!line.contains("--fork-session"));
    // Already driven here.
    assert_eq!(
        owned::continue_as(&g.st, PLAIN, ended, &g.cwd(), ContinueRequest::default())
            .await
            .unwrap_err()
            .code,
        "already_owned"
    );
    owned::kill(&g.st, &same).await.unwrap();
    // Not a session id.
    assert_eq!(
        owned::continue_as(&g.st, PLAIN, "--help", &g.cwd(), ContinueRequest::default())
            .await
            .unwrap_err()
            .code,
        "invalid_session_id"
    );
}

#[tokio::test]
async fn non_desktop_callers_need_a_trusted_cwd() {
    let f = Fixture::new();
    let phone_caller = Caller {
        may_dangerous: false,
        desktop: false,
    };
    // Transcripts recorded for the directory make it trusted.
    let canonical = std::fs::canonicalize(f.cwd.path()).unwrap();
    let recorded =
        f.st.home
            .projects_dir()
            .join(mother_claude_lib::claude::encode_cwd(
                &canonical.to_string_lossy(),
            ));
    std::fs::create_dir_all(&recorded).unwrap();
    std::fs::write(
        recorded.join("old.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({ "type": "user", "cwd": canonical.to_string_lossy() })
        ),
    )
    .unwrap();
    let id = owned::spawn_as(
        &f.st,
        phone_caller,
        SpawnRequest {
            cwd: f.cwd(),
            prompt: Some("hello".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    wait_turn_done(&f.st, &id).await;
    let argv = f.log()[0]["argv"].to_string();
    assert!(!argv.contains("--allow-dangerously-skip-permissions"));
    owned::kill(&f.st, &id).await.unwrap();
}

// ─────────────────────────────── over HTTP ───────────────────────────────────

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
async fn http_routes_drive_an_owned_session() {
    let f = Fixture::new();
    let base = serve(mother_claude_lib::server::router(f.st.clone())).await;
    let token = f.st.auth.token.clone();
    let c = reqwest::Client::new();
    let post = |path: &str, body: Value| {
        c.post(format!("{base}/api{path}"))
            .bearer_auth(&token)
            .json(&body)
            .send()
    };

    let r = post("/sessions", json!({ "cwd": f.cwd(), "prompt": "bash" }))
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let item = wait_pending(&f.st, &id, PendingKind::Permission).await;

    // Strict respond body: unknown fields and wrong-kind fields are refused.
    let r = post(
        &format!("/sessions/{id}/respond"),
        json!({ "requestId": item.id, "decision": "allow", "always": true }),
    )
    .await
    .unwrap();
    assert_eq!(r.status(), 422);
    let r = post(
        &format!("/sessions/{id}/respond"),
        json!({ "requestId": item.id, "decision": "allow", "mode": "plan" }),
    )
    .await
    .unwrap();
    assert_eq!(r.status(), 422);
    let r = post(
        &format!("/sessions/{id}/respond"),
        json!({ "requestId": item.id, "decision": "allow" }),
    )
    .await
    .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["resolution"]["outcome"], "allowed");
    assert_eq!(v["resolution"]["by"]["name"], "Mac");
    // Too late: 409 with how it was resolved.
    let r = post(
        &format!("/sessions/{id}/respond"),
        json!({ "requestId": item.id, "decision": "deny" }),
    )
    .await
    .unwrap();
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "already_decided");
    assert_eq!(v["error"]["resolution"]["outcome"], "allowed");
    wait_turn_done(&f.st, &id).await;

    // Message → sent, with the message id.
    let r = post(
        &format!("/sessions/{id}/message"),
        json!({ "text": "hello" }),
    )
    .await
    .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["delivery"], "sent");
    assert!(owned::is_uuid(v["messageId"].as_str().unwrap()));
    wait_turn_done(&f.st, &id).await;

    // Commands from initialize.
    let v: Value = c
        .get(format!("{base}/api/sessions/{id}/commands"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["commands"][0]["name"], "/compact");

    // Live controls; CLI refusals are 422.
    for (path, body) in [
        ("model", json!({ "model": "opus" })),
        ("mode", json!({ "mode": "acceptEdits" })),
        ("effort", json!({ "effort": "max" })),
        ("rename", json!({ "name": "Renamed" })),
        ("interrupt", json!({})),
    ] {
        let r = post(&format!("/sessions/{id}/{path}"), body).await.unwrap();
        assert_eq!(r.status(), 200, "{path}");
    }
    // Session-scoped switches answer 204; a malformed body is a 422.
    for (path, body) in [
        ("ultracode", json!({ "enabled": true })),
        ("ultracode", json!({ "enabled": false })),
        ("thinking", json!({ "enabled": false })),
    ] {
        let r = post(&format!("/sessions/{id}/{path}"), body).await.unwrap();
        assert_eq!(r.status(), 204, "{path}");
    }
    let r = post(&format!("/sessions/{id}/thinking"), json!({ "on": "yes" }))
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    let r = post(&format!("/sessions/{id}/mode"), json!({ "mode": "auto" }))
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["errorCode"],
        "auto_mode_model"
    );
    let r = post(
        &format!("/sessions/{id}/model"),
        json!({ "model": "--help" }),
    )
    .await
    .unwrap();
    assert_eq!(r.status(), 422);
    let v: Value = c
        .get(format!("{base}/api/sessions/{id}/context"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["totalTokens"], 1234);
    let v: Value = c
        .get(format!("{base}/api/sessions/{id}/mcp"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["servers"][0]["status"], "connected");

    // Background task output comes from the CLI (`get_task_output`), not
    // from a temp file this test never wrote.
    let o = c
        .get(format!("{base}/api/sessions/{id}/tasks/t1/output"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(o.status(), 200);
    assert_eq!(o.headers()["x-output-size"], "9000");
    assert_eq!(o.headers()["x-output-truncated"], "true");
    assert!(o.text().await.unwrap().contains("[t1] ready on :3000"));

    // Stop kills the child; respawn of an owned session is refused.
    let r = post(&format!("/sessions/{id}/stop"), json!({}))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    wait_for("exit", || !owned::has_handle(&f.st, &id)).await;

    // Lifecycle of a foreign session needs a SHORT job id.
    let foreign = "2d2e9c5e-3a52-4d4c-9d59-5f0b9a1e2c3d";
    ws::upsert_session(
        &f.st,
        SessionSummary {
            id: foreign.into(),
            cwd: f.cwd(),
            live: true,
            ..Default::default()
        },
    )
    .await;
    let r = post(&format!("/sessions/{foreign}/stop"), json!({}))
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "not_a_background_session"
    );
    // Ultracode and thinking switch only in sessions Mother Claude owns.
    for path in ["ultracode", "thinking"] {
        let r = post(
            &format!("/sessions/{foreign}/{path}"),
            json!({ "enabled": true }),
        )
        .await
        .unwrap();
        assert_eq!(r.status(), 409, "{path}");
        assert_eq!(
            r.json::<Value>().await.unwrap()["error"]["code"],
            "not_supported"
        );
    }
    // With a job id it goes through `claude stop <short>`, and the terminal
    // view is the rendered `claude logs` screen.
    ws::patch_session(&f.st, foreign, |s| s.job_id = Some("2d2e9c5e".into())).await;
    let r = post(&format!("/sessions/{foreign}/stop"), json!({}))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["output"], "stop 2d2e9c5e");
    let v: Value = c
        .get(format!("{base}/api/sessions/{foreign}/terminal"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["rows"][0], "fake screen");
    assert_eq!(v["rows"][2], "\u{276f} ok");
    assert!(!v["text"].as_str().unwrap().contains('\u{1b}'));
}

/// One live round trip against the INSTALLED CLI (haiku, a temp dir, project
/// settings only): initialize, a `can_use_tool` approval that creates a file,
/// and `set_permission_mode`. Spends about a cent. Run with
/// `cargo test --test owned -- --ignored live_`.
#[tokio::test]
#[ignore]
async fn live_haiku_approval_and_mode_switch() {
    // Real CLI: no fake, and nothing of ours in its environment.
    std::env::remove_var("MOTHER_CLAUDE_CLI");
    for key in [
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_USE_BEDROCK",
        "MOTHER_CLAUDE_TOKEN",
    ] {
        std::env::remove_var(key);
    }
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let st = Inner::new(
        ClaudeHome::resolve().unwrap_or_else(|_| ClaudeHome::with_base(home.path())),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
    );
    let cwd_s = std::fs::canonicalize(cwd.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let id = st
        .control
        .spawn(
            &st,
            mother_claude_lib::claude::SpawnOptions {
                cwd: cwd_s.clone(),
                prompt: "Use the Bash tool to run exactly: touch probe.txt   Then reply DONE."
                    .into(),
                model: Some("haiku".into()),
                permission_mode: Some("default".into()),
                setting_sources: Some("project".into()),
                extra_args: vec!["--strict-mcp-config".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut item = None;
    for _ in 0..600 {
        item = st
            .pending
            .list(&id)
            .into_iter()
            .find(|i| i.kind == PendingKind::Permission);
        if item.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let item = item.expect("a can_use_tool prompt from the real CLI");
    assert_eq!(item.tool_name.as_deref(), Some("Bash"));
    // While the turn waits on us, a second message queues and can be taken
    // back (`cancel_async_message`) before it ever runs.
    let queued = owned::send(&st, &id, "Reply with the single word QUEUED.", &[])
        .await
        .unwrap();
    assert_eq!(queued.delivery, Delivery::Queued);
    owned::cancel_queued(&st, &id, &queued.message_id)
        .await
        .unwrap();
    assert!(owned::queued(&st, &id).is_empty());
    pending::respond(&st, &id, &item.id, Decision::allow(), &desktop())
        .await
        .unwrap();
    for _ in 0..600 {
        if owned::live_state(&st, &id).is_some_and(|l| l.started && !l.turn_active) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        Path::new(&cwd_s).join("probe.txt").exists(),
        "the approved tool ran"
    );
    let commands = owned::commands(&st, &id).await.unwrap_or_default();
    assert!(!commands.is_empty(), "initialize returned commands");
    owned::set_mode(&st, &id, "plan").await.unwrap();
    for _ in 0..50 {
        if owned::live_state(&st, &id)
            .and_then(|l| l.permission_mode)
            .as_deref()
            == Some("plan")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        owned::live_state(&st, &id)
            .unwrap()
            .permission_mode
            .as_deref(),
        Some("plan")
    );
    owned::kill(&st, &id).await.unwrap();
    for _ in 0..100 {
        if !owned::has_handle(&st, &id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A $0 live check against the INSTALLED CLI (haiku, a temp dir, project
/// settings only, nothing persisted, no user turn — control requests only):
/// `get_settings` after `initialize`, and the session-only ultracode, effort
/// and thinking switches read back. Run with
/// `cargo test --test owned -- --ignored live_settings`.
#[tokio::test]
#[ignore]
async fn live_settings_are_read_back_without_a_turn() {
    std::env::remove_var("MOTHER_CLAUDE_CLI");
    for key in [
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_USE_BEDROCK",
        "MOTHER_CLAUDE_TOKEN",
    ] {
        std::env::remove_var(key);
    }
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let st = Inner::new(
        ClaudeHome::with_base(home.path()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
    );
    let cwd_s = std::fs::canonicalize(cwd.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let id = st
        .control
        .spawn(
            &st,
            mother_claude_lib::claude::SpawnOptions {
                cwd: cwd_s,
                model: Some("haiku".into()),
                permission_mode: Some("default".into()),
                thinking: Some("off".into()),
                setting_sources: Some("project".into()),
                extra_args: vec![
                    "--strict-mcp-config".into(),
                    "--no-session-persistence".into(),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for _ in 0..300 {
        if owned::live_state(&st, &id).is_some_and(|l| l.applied.is_some()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let live = owned::live_state(&st, &id).unwrap();
    let applied = live
        .applied
        .expect("get_settings answered after initialize");
    assert!(applied
        .model
        .as_deref()
        .is_some_and(|m| m.contains("haiku")));
    assert_eq!(applied.effort, None, "haiku has no effort");
    assert!(!applied.ultracode_available);
    assert!(
        live.fast_mode_state.is_some(),
        "initialize reports fast mode"
    );
    let s = st.find_session(&id).await.unwrap();
    assert_eq!(s.ultracode, Some(UltracodeState::Off));
    assert_eq!(s.ultracode_source, Some(UltracodeSource::Applied));
    assert_eq!(
        s.thinking.map(|t| (t.mode, t.source)),
        Some((ThinkingMode::Off, ThinkingSource::Control))
    );
    assert!(s.control.set_thinking && !s.control.set_ultracode);

    owned::set_ultracode(&st, &id, true).await.unwrap();
    let s = st.find_session(&id).await.unwrap();
    assert_eq!(
        s.ultracode,
        Some(UltracodeState::Unavailable),
        "requested, but haiku cannot run it"
    );
    assert!(s.control.set_ultracode, "the request can be withdrawn");
    assert_eq!(
        s.effort_source,
        Some(EffortSource::Applied),
        "no effort levels"
    );
    owned::set_effort(&st, &id, "high").await.unwrap();
    let applied = owned::live_state(&st, &id).unwrap().applied.unwrap();
    assert!(applied.ultracode_requested, "the effort change kept it");
    assert_eq!(applied.effort, None);
    owned::set_ultracode(&st, &id, false).await.unwrap();
    assert_eq!(
        st.find_session(&id).await.unwrap().ultracode,
        Some(UltracodeState::Off)
    );
    owned::set_thinking(&st, &id, true).await.unwrap();
    owned::set_thinking(&st, &id, false).await.unwrap();
    assert_eq!(
        st.find_session(&id).await.unwrap().thinking.map(|t| t.mode),
        Some(ThinkingMode::Off)
    );
    owned::kill(&st, &id).await.unwrap();
    for _ in 0..100 {
        if !owned::has_handle(&st, &id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Spawning under each kind of account composes the child's environment as
/// docs/ACCOUNTS.md D5 says, and the session carries its account.
#[tokio::test]
async fn sessions_run_under_the_account_they_are_given() {
    use mother_claude_lib::claude::secrets::Secret;
    let f = Fixture::new();
    let token = Secret::new("sk-ant-oat01-AccountBTokenValue0123456789abcdefghij");
    let pro = f.st.accounts.add_token("Pro", Some(&token), None).unwrap();
    let dirs = tempfile::tempdir().unwrap();
    let work =
        f.st.accounts
            .add_directory("Work", &dirs.path().join("claude-work"), None)
            .unwrap();

    // A token account: overrides removed, its token set, the directory as found.
    let id = f
        .spawn(
            MAC,
            SpawnRequest {
                prompt: Some("hello".into()),
                account: Some(pro.id.clone()),
                ..Default::default()
            },
        )
        .await;
    wait_turn_done(&f.st, &id).await;
    let account = f.log()[0]["account"].clone();
    assert_eq!(account["oauthToken"], true, "{account}");
    assert_eq!(account["oauthTokenSha256"], token.sha256_hex());
    assert_eq!(account["overrides"], json!(["CLAUDE_CODE_OAUTH_TOKEN"]));
    assert_eq!(
        account["configDir"].as_str(),
        std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref()
    );
    let summary = f.st.find_session(&id).await.unwrap();
    assert_eq!(
        summary.account.as_ref().map(|a| a.id.as_str()),
        Some(pro.id.as_str())
    );
    assert_eq!(summary.account.as_ref().unwrap().name, "Pro");
    f.st.control.kill(&id).await.unwrap();

    // A directory account: its own directory, no overrides at all.
    let g = Fixture::new();
    let work2 =
        g.st.accounts
            .add_directory("Work", &dirs.path().join("claude-work-2"), None)
            .unwrap();
    let id = g
        .spawn(
            MAC,
            SpawnRequest {
                prompt: Some("hello".into()),
                account: Some(work2.id.clone()),
                ..Default::default()
            },
        )
        .await;
    wait_turn_done(&g.st, &id).await;
    let account = g.log()[0]["account"].clone();
    assert_eq!(
        account["configDir"].as_str().map(PathBuf::from),
        work2.dir.clone(),
        "{account}"
    );
    assert_eq!(account["oauthToken"], false);
    assert_eq!(account["overrides"], json!([]));
    g.st.control.kill(&id).await.unwrap();

    // The project's account is the default for a launch that names none.
    f.st.accounts
        .set_defaults(mother_claude_lib::claude::accounts::AccountDefaults {
            by_project: [(f.cwd(), work.id.clone())].into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(f.st.accounts.account_for_cwd(&f.cwd()), work.id);

    // An unknown account, or a token account without its token, is refused.
    let err = owned::spawn_as(
        &f.st,
        MAC,
        SpawnRequest {
            cwd: f.cwd(),
            account: Some("acc_000000000000".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "unknown_account");
    let empty = f.st.accounts.add_token("Empty", None, None).unwrap();
    let err = owned::spawn_as(
        &f.st,
        MAC,
        SpawnRequest {
            cwd: f.cwd(),
            account: Some(empty.id.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "account_token_missing");
}

/// "Continue on B": a token account forks the conversation under B; a
/// directory account needs the conversation copied first.
#[tokio::test]
async fn continuing_on_another_account_forks_it() {
    use mother_claude_lib::claude::secrets::Secret;
    let f = Fixture::new();
    let id = f.spawn_text("hello").await;
    wait_turn_done(&f.st, &id).await;
    f.st.control.kill(&id).await.unwrap();
    wait_for("the first session to end", || {
        !owned::has_handle(&f.st, &id)
    })
    .await;

    let token = Secret::new("sk-ant-oat01-AccountBTokenValue0123456789abcdefghij");
    let pro = f.st.accounts.add_token("Pro", Some(&token), None).unwrap();

    // A conversation Mother Claude drives right now (it hit A's limit) can
    // go on under B: a fork, the original left running.
    let running = f.spawn_text("hello").await;
    wait_turn_done(&f.st, &running).await;
    let same = owned::continue_as(&f.st, MAC, &running, &f.cwd(), ContinueRequest::default())
        .await
        .unwrap_err();
    assert_eq!(
        same.code, "already_owned",
        "the same account: send it a message"
    );
    let branched = owned::continue_as(
        &f.st,
        MAC,
        &running,
        &f.cwd(),
        ContinueRequest {
            prompt: Some("go on".into()),
            account: Some(pro.id.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_ne!(branched, running);
    assert!(
        owned::has_handle(&f.st, &running),
        "the original keeps running"
    );
    wait_turn_done(&f.st, &branched).await;
    f.st.control.kill(&branched).await.unwrap();
    f.st.control.kill(&running).await.unwrap();

    let forked = owned::continue_as(
        &f.st,
        MAC,
        &id,
        &f.cwd(),
        ContinueRequest {
            prompt: Some("go on".into()),
            account: Some(pro.id.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_ne!(forked, id, "another account always forks");
    wait_turn_done(&f.st, &forked).await;
    let line = f
        .log()
        .into_iter()
        .rfind(|l| l.get("argv").is_some())
        .unwrap();
    let argv = line["argv"].to_string();
    assert!(
        argv.contains(&format!("\"--resume\",\"{id}\",\"--fork-session\"")),
        "{argv}"
    );
    assert_eq!(line["account"]["oauthTokenSha256"], token.sha256_hex());
    assert_eq!(
        f.st.control.account_of(&forked).as_deref(),
        Some(pro.id.as_str())
    );
    f.st.control.kill(&forked).await.unwrap();
}

/// "Continue on" a directory account (P3.3): the conversation is copied into
/// its directory — once the desktop allowed copies between the two — and
/// forked there.
#[tokio::test]
async fn continuing_on_a_directory_account_copies_then_forks() {
    setup();
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(data.path().to_path_buf());
    let st = Inner::new(
        ClaudeHome::with_base(home.path()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
    );
    let cwd_s = cwd.path().to_string_lossy().into_owned();
    // A finished conversation in the Default's directory.
    let id = "6b0b2a3c-1d2e-4f50-8a9b-0c1d2e3f4a5b";
    let proj = home
        .path()
        .join("projects")
        .join(mother_claude_lib::claude::encode_cwd(&cwd_s));
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join(format!("{id}.jsonl")),
        format!("{{\"type\":\"user\",\"cwd\":\"{cwd_s}\",\"sessionId\":\"{id}\"}}\n"),
    )
    .unwrap();
    let dirs = tempfile::tempdir().unwrap();
    let work = st
        .accounts
        .add_directory("Work", &dirs.path().join("claude-work"), None)
        .unwrap();
    let ask = || ContinueRequest {
        prompt: Some("go on".into()),
        account: Some(work.id.clone()),
        ..Default::default()
    };
    let err = owned::continue_as(&st, MAC, id, &cwd_s, ask())
        .await
        .unwrap_err();
    assert_eq!(err.code, "copy_consent_required");
    assert!(
        !work.dir.as_ref().unwrap().join("projects").exists(),
        "nothing copied"
    );

    st.accounts
        .set_copy_consent("default", &work.id, true)
        .unwrap();
    let forked = owned::continue_as(&st, MAC, id, &cwd_s, ask())
        .await
        .unwrap();
    assert_ne!(forked, id);
    let copy = work
        .dir
        .as_ref()
        .unwrap()
        .join("projects")
        .join(mother_claude_lib::claude::encode_cwd(&cwd_s))
        .join(format!("{id}.jsonl"));
    assert!(copy.is_file(), "copied into Work's directory");
    wait_turn_done(&st, &forked).await;
    let line = read_log(cwd.path())
        .into_iter()
        .rfind(|l| l.get("argv").is_some())
        .unwrap();
    assert_eq!(
        line["account"]["configDir"].as_str().map(PathBuf::from),
        work.dir.clone()
    );
    assert!(line["argv"].to_string().contains("--fork-session"));
    // The copy is a data job with an undo.
    let jobs = st.jobs.list();
    assert_eq!(jobs[0].kind, "copy");
    assert!(jobs[0].undo_until.is_some());
    st.control.kill(&forked).await.unwrap();

    // A second continue-on reuses the copy.
    wait_for("the fork to end", || !owned::has_handle(&st, &forked)).await;
    let again = owned::continue_as(&st, MAC, id, &cwd_s, ask())
        .await
        .unwrap();
    assert_ne!(again, forked);
    assert_eq!(st.jobs.list().len(), 1, "no second copy");
    st.control.kill(&again).await.unwrap();
}

/// A terminal session that runs with a token no account holds ("Token
/// (unregistered)") continues under the project's account when none is named.
#[tokio::test]
async fn an_unregistered_token_session_continues_under_the_default() {
    use mother_claude_lib::claude::live::LiveSession;
    use mother_claude_lib::claude::secrets::Secret;
    let f = Fixture::new();
    let pro =
        f.st.accounts
            .add_token(
                "Pro",
                Some(&Secret::new(
                    "sk-ant-oat01-AccountBTokenValue0123456789abcdefghij",
                )),
                None,
            )
            .unwrap();
    let id = "7b0b2a3c-1d2e-4f50-8a9b-0c1d2e3f4a5b";
    f.st.live.insert_for_test(LiveSession {
        pid: Some(std::process::id()),
        session_id: Some(id.into()),
        cwd: Some(f.cwd()),
        home_account: Some("default".into()),
        token_sha256: Some("0".repeat(64)),
        ..Default::default()
    });
    assert_eq!(
        mother_claude_lib::claude::summaries::session_account(&f.st, id),
        "unregistered"
    );
    let forked = owned::continue_as(
        &f.st,
        MAC,
        id,
        &f.cwd(),
        ContinueRequest {
            prompt: Some("go on".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_ne!(forked, id, "live elsewhere and another account: a fork");
    assert_eq!(f.st.control.account_of(&forked).as_deref(), Some("default"));
    let _ = pro;
    wait_turn_done(&f.st, &forked).await;
    f.st.control.kill(&forked).await.unwrap();
}

/// A terminal session that runs with a token of its own. While no token
/// account exists that token says nothing about it (goal 7): it is the
/// Default's, and Continue after it ended resumes it in place, as in v0.4.
/// With a token account registered its chip says "unregistered", but for
/// every decision it is still the Default's: Continue resumes it in place,
/// and continuing on a directory account asks for the (Default, that
/// account) copy consent — one the desktop can grant.
#[tokio::test]
async fn a_session_with_a_token_of_its_own_is_the_defaults_for_decisions() {
    use mother_claude_lib::claude::live::LiveSession;
    use mother_claude_lib::claude::secrets::Secret;
    use mother_claude_lib::claude::summaries::{acting_account, session_account};
    setup();
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(data.path().to_path_buf());
    let st = Inner::new(
        ClaudeHome::with_base(home.path()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
    );
    let cwd_s = cwd.path().to_string_lossy().into_owned();
    let proj = home
        .path()
        .join("projects")
        .join(mother_claude_lib::claude::encode_cwd(&cwd_s));
    std::fs::create_dir_all(&proj).unwrap();
    // It ran in a terminal with CLAUDE_CODE_OAUTH_TOKEN set, then ended.
    let ran_with_a_token = |id: &str| {
        std::fs::write(
            proj.join(format!("{id}.jsonl")),
            format!("{{\"type\":\"user\",\"cwd\":\"{cwd_s}\",\"sessionId\":\"{id}\"}}\n"),
        )
        .unwrap();
        st.live.insert_for_test(LiveSession {
            pid: Some(std::process::id()),
            session_id: Some(id.into()),
            cwd: Some(cwd_s.clone()),
            home_account: Some("default".into()),
            token_sha256: Some("0".repeat(64)),
            ..Default::default()
        });
        let seen = session_account(&st, id);
        st.live.remove_for_test(id);
        seen
    };
    let go_on = |account: Option<&str>| ContinueRequest {
        prompt: Some("go on".into()),
        account: account.map(str::to_string),
        ..Default::default()
    };
    let finish = |id: String| {
        let st = st.clone();
        async move {
            wait_turn_done(&st, &id).await;
            st.control.kill(&id).await.unwrap();
            wait_for("it to end", || !owned::has_handle(&st, &id)).await;
        }
    };

    let a = "8b0b2a3c-1d2e-4f50-8a9b-0c1d2e3f4a5b";
    assert_eq!(ran_with_a_token(a), "default", "only the Default: no chip");
    let resumed = owned::continue_as(&st, MAC, a, &cwd_s, go_on(None))
        .await
        .unwrap();
    assert_eq!(resumed, a, "resumed in place, as in v0.4");
    finish(resumed).await;

    st.accounts
        .add_token(
            "Pro",
            Some(&Secret::new(
                "sk-ant-oat01-AccountProTokenValue0123456789abcdefghij",
            )),
            None,
        )
        .unwrap();
    let b = "9b0b2a3c-1d2e-4f50-8a9b-0c1d2e3f4a5b";
    assert_eq!(ran_with_a_token(b), "unregistered");
    assert_eq!(acting_account(&st, b), "default");
    let resumed = owned::continue_as(&st, MAC, b, &cwd_s, go_on(None))
        .await
        .unwrap();
    assert_eq!(resumed, b, "the Default's: resumed in place");
    assert_eq!(st.control.account_of(&resumed).as_deref(), Some("default"));
    finish(resumed).await;

    let dirs = tempfile::tempdir().unwrap();
    let work = st
        .accounts
        .add_directory("Work", &dirs.path().join("claude-work"), None)
        .unwrap();
    let c = "ab0b2a3c-1d2e-4f50-8a9b-0c1d2e3f4a5b";
    assert_eq!(ran_with_a_token(c), "unregistered");
    let err = owned::continue_as(&st, MAC, c, &cwd_s, go_on(Some(&work.id)))
        .await
        .unwrap_err();
    assert_eq!(err.code, "copy_consent_required");
    st.accounts
        .set_copy_consent("default", &work.id, true)
        .unwrap();
    let forked = owned::continue_as(&st, MAC, c, &cwd_s, go_on(Some(&work.id)))
        .await
        .unwrap();
    assert_ne!(forked, c, "another account: a fork of the copy");
    assert_eq!(
        st.control.account_of(&forked).as_deref(),
        Some(work.id.as_str())
    );
    finish(forked).await;
}
