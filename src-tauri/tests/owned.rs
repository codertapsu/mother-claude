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
    Delivery, ImageInput, PendingKind, PendingOutcome, PendingSource, ResolverKind, SessionSummary,
    StreamKind,
};
use mother_claude_lib::claude::owned::{self, Caller, ContinueRequest, SpawnRequest};
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
    std::fs::write(recorded.join("old.jsonl"), "{}\n").unwrap();
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
