//! Switching a session's account (docs/ACCOUNTS.md §15.1, P9.1) against the
//! FAKE `claude` (`tests/fixtures/fake-claude.py`, selected with
//! `MOTHER_CLAUDE_CLI`): an owned session goes on under another account in
//! place — same id, `--resume`, never a fork, the old process gone first —
//! with the target's environment and the settings it ran with; into a
//! directory account its conversation moves there (an undo job); busy, it
//! switches after its turn or now; a pending switch is cancelled; a session
//! nobody runs is recorded or moved for its next continue; the refusals.
//!
//! Every directory is a test's temp dir (the Default's `.claude.json` inside
//! it); no listener binds a fixed port.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};
use std::time::Duration;

use mother_claude_lib::bridge::{BridgeConfig, TokenPolicy};
use mother_claude_lib::claude::accounts::Account;
use mother_claude_lib::claude::live::LiveSession;
use mother_claude_lib::claude::owned::{self, Caller, ContinueRequest, SpawnRequest};
use mother_claude_lib::claude::secrets::Secret;
use mother_claude_lib::claude::switch::{self, Outcome, SwitchBody};
use mother_claude_lib::claude::{encode_cwd, ClaudeHome};
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::context::{ClientContext, Ingress, Principal};
use mother_claude_lib::server::ws::ServerEvent;
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use serde_json::{json, Value};

static SETUP: Once = Once::new();

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
    });
}

const TOKEN: &str = "sk-ant-oat01-SwitchAccountToken0123456789abcdefghijk";

const MAC: Caller = Caller {
    may_dangerous: true,
    desktop: true,
};

struct Fixture {
    st: AppState,
    home: tempfile::TempDir,
    cwd: tempfile::TempDir,
    dirs: tempfile::TempDir,
    _data: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        setup();
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let dirs = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        // A CLI with pidfiles: a fresh transcript alone is not a running
        // session.
        std::fs::create_dir_all(home.path().join("sessions")).unwrap();
        let mut auth = Auth::ephemeral();
        auth.config_dir = Some(data.path().to_path_buf());
        let st = Inner::with_bridge_config(
            ClaudeHome::for_account_dir(home.path()),
            ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            auth,
            BridgeConfig {
                enabled: true,
                port: None,
                token: TokenPolicy::Required,
                api_key: None,
                allowed_origins: Vec::new(),
            },
        );
        Self {
            st,
            home,
            cwd,
            dirs,
            _data: data,
        }
    }

    fn cwd(&self) -> String {
        self.cwd.path().to_string_lossy().into_owned()
    }

    fn pro(&self) -> Account {
        self.st
            .accounts
            .add_token("Pro", Some(&Secret::new(TOKEN)), None)
            .unwrap()
    }

    fn work(&self) -> Account {
        self.st
            .accounts
            .add_directory("Work", &self.dirs.path().join("claude-work"), None)
            .unwrap()
    }

    fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(self.cwd.path().join("fake-claude.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// The first line of every process the fake ran here, in order.
    fn processes(&self) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter(|l| l.get("argv").is_some())
            .collect()
    }

    /// The first `n` processes, once the last of them has started.
    async fn processes_once(&self, n: usize) -> Vec<Value> {
        wait_for(&format!("{n} processes"), || self.processes().len() >= n).await;
        let procs = self.processes();
        assert_eq!(procs.len(), n, "{procs:?}");
        procs
    }

    /// What process `n` (from 0) read from stdin.
    fn received_by(&self, n: usize) -> Vec<Value> {
        let mut seen = 0;
        let mut out = Vec::new();
        for line in self.log() {
            if line.get("argv").is_some() {
                seen += 1;
                continue;
            }
            if seen == n + 1 {
                if let Some(v) = line.get("in") {
                    out.push(v.clone());
                }
            }
        }
        out
    }

    async fn spawn(&self, req: SpawnRequest) -> String {
        let id = owned::spawn_as(
            &self.st,
            MAC,
            SpawnRequest {
                cwd: self.cwd(),
                ..req
            },
        )
        .await
        .unwrap();
        // Settled: its turn done and `get_settings` answered.
        wait_for("the session to settle", || {
            owned::live_state(&self.st, &id)
                .is_some_and(|l| l.started && !l.turn_active && l.applied.is_some())
        })
        .await;
        id
    }

    /// The transcript the CLI keeps for `id`, in a config directory.
    fn transcript_in(&self, dir: &Path, id: &str) -> PathBuf {
        dir.join("projects")
            .join(encode_cwd(&self.cwd()))
            .join(format!("{id}.jsonl"))
    }

    /// Write what the CLI would have written for `id` in `dir`.
    fn write_transcript(&self, dir: &Path, id: &str) {
        let path = self.transcript_in(dir, id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let cwd = self.cwd();
        let user = json!({"type": "user", "sessionId": id, "cwd": cwd, "uuid": "u-1",
            "timestamp": "2026-10-28T09:00:00.000Z",
            "message": {"role": "user", "content": "hello"}});
        let assistant = json!({"type": "assistant", "sessionId": id, "cwd": cwd, "uuid": "a-1",
            "parentUuid": "u-1", "timestamp": "2026-10-28T09:00:05.000Z",
            "message": {"id": "msg_1", "role": "assistant", "model": "claude-opus-5-5",
                        "content": [{"type": "text", "text": "hi"}],
                        "usage": {"input_tokens": 1, "output_tokens": 2}}});
        std::fs::write(path, format!("{user}\n{assistant}\n")).unwrap();
    }

    async fn switch(&self, ctx: &ClientContext, id: &str, to: &str, when: Option<&str>) -> Outcome {
        switch::switch(&self.st, ctx, id, &body(to, when))
            .await
            .unwrap_or_else(|e| panic!("switch to {to}: {} {}", e.code, e.message))
    }

    async fn refused(&self, ctx: &ClientContext, id: &str, to: &str, when: Option<&str>) -> String {
        match switch::switch(&self.st, ctx, id, &body(to, when)).await {
            Ok(o) => panic!("switched: {o:?}"),
            Err(e) => e.code,
        }
    }

    async fn stop(&self, id: &str) {
        self.st.control.kill(id).await.unwrap();
        wait_for("the session to end", || !owned::has_handle(&self.st, id)).await;
    }
}

fn body(to: &str, when: Option<&str>) -> SwitchBody {
    SwitchBody {
        account: to.to_string(),
        when: when.map(str::to_string),
    }
}

fn desktop() -> ClientContext {
    let mut h = axum::http::HeaderMap::new();
    h.insert("host", "127.0.0.1:6725".parse().unwrap());
    let mut c = ClientContext::new(Ingress::Loopback, "127.0.0.1:9".parse().unwrap(), &h, false);
    c.principal = Principal::Desktop;
    c
}

fn phone() -> ClientContext {
    let mut h = axum::http::HeaderMap::new();
    h.insert("host", "192.168.1.2:6725".parse().unwrap());
    let mut c = ClientContext::new(Ingress::Lan, "192.168.1.9:9".parse().unwrap(), &h, false);
    c.principal = Principal::Device {
        id: "d1".into(),
        allow_dangerous: false,
    };
    c
}

async fn wait_for<F: FnMut() -> bool>(what: &str, mut f: F) {
    for _ in 0..750 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

fn argv(process: &Value) -> String {
    let parts: Vec<String> = serde_json::from_value(process["argv"].clone()).unwrap();
    parts.join(" ")
}

fn alive(pid: u32) -> bool {
    mother_claude_lib::claude::live::pid_exists(pid)
}

/// A token account and the Default share a directory: the session goes on
/// in place — same id, `--resume`, the first process gone before the next
/// starts — with the target's environment (not the source's) and the
/// settings it ran with; and back again.
#[tokio::test]
async fn an_owned_session_switches_between_a_token_account_and_the_default_in_place() {
    let f = Fixture::new();
    let pro = f.pro();
    let id = f
        .spawn(SpawnRequest {
            prompt: Some("hello".into()),
            model: Some("opus".into()),
            effort: Some("high".into()),
            permission_mode: Some("acceptEdits".into()),
            thinking: Some("off".into()),
            account: Some(pro.id.clone()),
            ..Default::default()
        })
        .await;
    f.write_transcript(f.home.path(), &id);
    let first = f.processes()[0].clone();
    assert_eq!(
        first["account"]["oauthTokenSha256"],
        Secret::new(TOKEN).sha256_hex()
    );

    let mut notices = f.st.bus.subscribe();
    let out = f.switch(&desktop(), &id, "default", None).await;
    assert_eq!(
        out,
        Outcome::Switched {
            account: "default".into(),
            moved: false,
            job: None
        }
    );
    // One notice says where it goes on; the resume itself announces nothing.
    let mut texts = Vec::new();
    while let Ok(ev) = notices.try_recv() {
        if let ServerEvent::Notice { text, .. } = &*ev {
            texts.push(text.clone());
        }
    }
    assert!(
        !texts.iter().any(|t| t.contains("owned session")),
        "{texts:?}"
    );
    // Same id, one process, under the Default.
    assert!(owned::has_handle(&f.st, &id));
    assert_eq!(f.st.control.account_of(&id).as_deref(), Some("default"));
    let procs = f.processes_once(2).await;
    assert!(
        !alive(first["pid"].as_u64().unwrap() as u32),
        "the first process ended before the second started"
    );
    let line = argv(&procs[1]);
    assert!(line.contains(&format!("--resume {id}")), "{line}");
    assert!(!line.contains("--fork-session") && !line.contains("--session-id"));
    // The settings it ran with, explicit.
    for part in [
        "--permission-mode acceptEdits",
        "--model claude-opus-5-5",
        "--effort high",
    ] {
        assert!(line.contains(part), "{part:?} missing from {line}");
    }
    assert_eq!(procs[1]["env"]["MAX_THINKING_TOKENS"], "0", "thinking off");
    // The Default's environment, never the token account's.
    let account = &procs[1]["account"];
    assert_ne!(
        account["oauthTokenSha256"],
        json!(Secret::new(TOKEN).sha256_hex())
    );
    assert_eq!(
        account["configDir"].as_str(),
        std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref()
    );
    let row = f.st.find_session(&id).await.unwrap();
    assert_eq!(row.account.unwrap().id, "default");
    assert!(row.switching_to.is_none() && row.live && row.owned);

    // It takes messages under its new account.
    owned::send(&f.st, &id, "hello", &[]).await.unwrap();
    wait_for("the turn", || {
        f.received_by(1)
            .iter()
            .any(|m| m["type"] == "user" && m["message"]["content"][0]["text"] == "hello")
    })
    .await;
    wait_for("the turn to end", || {
        f.st.control.is_idle(&id) == Some(true)
    })
    .await;

    // And back: the token account's token, the overrides gone.
    let out = f.switch(&desktop(), &id, &pro.id, None).await;
    assert!(matches!(out, Outcome::Switched { moved: false, .. }));
    let procs = f.processes_once(3).await;
    assert_eq!(
        procs[2]["account"]["oauthTokenSha256"],
        Secret::new(TOKEN).sha256_hex()
    );
    assert_eq!(
        procs[2]["account"]["overrides"],
        json!(["CLAUDE_CODE_OAUTH_TOKEN"])
    );
    assert!(argv(&procs[2]).contains(&format!("--resume {id}")));
    assert_eq!(
        mother_claude_lib::claude::summaries::session_account(&f.st, &id),
        pro.id
    );

    // Its own account: nothing to do.
    let out = f.switch(&desktop(), &id, &pro.id, None).await;
    assert!(matches!(out, Outcome::Switched { moved: false, .. }));
    assert_eq!(f.processes().len(), 3, "no process was started");
    f.stop(&id).await;
}

/// Into a directory account the conversation moves first — only once the
/// desktop allowed copies between the two — and is resumed there; the move
/// is a data job whose undo puts it back once the session stopped. A move
/// that fails resumes it where it was, under its own account, and says so.
#[tokio::test]
async fn into_a_directory_account_the_conversation_moves_and_is_resumed_there() {
    let f = Fixture::new();
    let work = f.work();
    let work_dir = work.dir.clone().unwrap();
    let id = f
        .spawn(SpawnRequest {
            prompt: Some("hello".into()),
            ..Default::default()
        })
        .await;
    f.write_transcript(f.home.path(), &id);

    // No consent: refused, and nothing changed.
    let err = switch::switch(&f.st, &desktop(), &id, &body(&work.id, None))
        .await
        .unwrap_err();
    assert_eq!(err.code, "copy_consent_required");
    assert_eq!(err.body()["error"]["from"], "default");
    assert_eq!(err.body()["error"]["to"], json!(work.id));
    assert!(owned::has_handle(&f.st, &id));
    assert_eq!(f.processes().len(), 1, "still the first process");
    assert!(f.transcript_in(f.home.path(), &id).is_file());

    // A move that fails (Work's directory cannot be written): resumed where
    // it was, under the Default, and the answer says so.
    f.st.accounts
        .set_copy_consent("default", &work.id, true)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&work_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let err = switch::switch(&f.st, &desktop(), &id, &body(&work.id, None))
            .await
            .unwrap_err();
        std::fs::set_permissions(&work_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            err.body()["error"]["account"],
            "default",
            "{:?}",
            err.body()
        );
        assert!(
            err.message.contains("It goes on under Default"),
            "{}",
            err.message
        );
        assert!(owned::has_handle(&f.st, &id), "it runs again");
        assert_eq!(f.st.control.account_of(&id).as_deref(), Some("default"));
        let procs = f.processes_once(2).await;
        assert!(argv(&procs[1]).contains(&format!("--resume {id}")));
        assert_eq!(
            procs[1]["account"]["configDir"].as_str(),
            std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref()
        );
        assert!(f.transcript_in(f.home.path(), &id).is_file());
    }

    // Moved and resumed there.
    let before = f.processes().len();
    let out = f.switch(&desktop(), &id, &work.id, None).await;
    let Outcome::Switched {
        account,
        moved: true,
        job: Some(job),
    } = out
    else {
        panic!("{out:?}")
    };
    assert_eq!(account, work.id);
    assert!(f.transcript_in(&work_dir, &id).is_file());
    assert!(!f.transcript_in(f.home.path(), &id).exists());
    let procs = f.processes_once(before + 1).await;
    let last = procs.last().unwrap();
    assert!(argv(last).contains(&format!("--resume {id}")));
    assert!(!argv(last).contains("--fork-session"));
    assert_eq!(
        last["account"]["configDir"].as_str().map(PathBuf::from),
        Some(work_dir.clone())
    );
    assert_eq!(last["account"]["oauthToken"], false);
    let view = f.st.jobs.get(&job).unwrap();
    assert_eq!(view.kind, "move");
    assert_eq!(view.from.as_deref(), Some("default"));
    assert_eq!(view.to.as_deref(), Some(work.id.as_str()));
    assert!(view.undo_until.is_some());
    let row = f.st.find_session(&id).await.unwrap();
    assert_eq!(row.account.unwrap().id, work.id);

    // Undo waits until it stopped, then puts it back.
    let err = mother_claude_lib::claude::transfer::ops::undo(&f.st, &job)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "session_live");
    f.stop(&id).await;
    mother_claude_lib::server::monitor::tick_once(&f.st).await;
    mother_claude_lib::claude::transfer::ops::undo(&f.st, &job)
        .await
        .unwrap();
    assert!(f.transcript_in(f.home.path(), &id).is_file());
    assert!(!f.transcript_in(&work_dir, &id).exists());
}

/// Busy with a turn: `idle` waits for it to end (`switchingTo` meanwhile),
/// `now` interrupts it — what was queued behind it is taken back and handed
/// to the resumed process; a waiting switch can be cancelled.
#[tokio::test]
async fn a_busy_session_switches_after_its_turn_or_now() {
    let f = Fixture::new();
    let pro = f.pro();
    let id = f
        .spawn(SpawnRequest {
            prompt: Some("hello".into()),
            ..Default::default()
        })
        .await;
    f.write_transcript(f.home.path(), &id);

    // After the turn.
    owned::send(&f.st, &id, "slow", &[]).await.unwrap();
    let out = f.switch(&desktop(), &id, &pro.id, Some("idle")).await;
    assert_eq!(
        out,
        Outcome::Pending {
            account: pro.id.clone()
        }
    );
    let row = f.st.find_session(&id).await.unwrap();
    assert_eq!(row.switching_to.as_deref(), Some(pro.id.as_str()));
    assert_eq!(
        serde_json::to_value(&row).unwrap()["switchingTo"],
        json!(pro.id)
    );
    assert_eq!(f.processes().len(), 1, "it waits for the turn");
    wait_for("the switch after the turn", || {
        f.st.control.account_of(&id).as_deref() == Some(pro.id.as_str())
    })
    .await;
    assert!(
        !f.received_by(0)
            .iter()
            .any(|m| m["request"]["subtype"] == "interrupt"),
        "the turn ran to its end"
    );
    assert!(f
        .received_by(0)
        .iter()
        .any(|m| m["type"] == "user" && m["message"]["content"][0]["text"] == "slow"),);
    wait_for("switchingTo to clear", || {
        f.st.control.switches.target(&id).is_none()
    })
    .await;
    let row = f.st.find_session(&id).await.unwrap();
    assert!(row.switching_to.is_none());
    f.processes_once(2).await;

    // Cancelled while it waits: nothing happens after the turn.
    owned::send(&f.st, &id, "slow", &[]).await.unwrap();
    assert!(matches!(
        f.switch(&desktop(), &id, "default", None).await,
        Outcome::Pending { .. }
    ));
    assert_eq!(
        switch::cancel(&f.st, &id).await.unwrap(),
        pro.id,
        "the account it stays on"
    );
    assert_eq!(
        switch::cancel(&f.st, &id).await.unwrap_err().code,
        "switch_not_pending"
    );
    assert!(f.st.find_session(&id).await.unwrap().switching_to.is_none());
    wait_for("the turn to end", || {
        f.st.control.is_idle(&id) == Some(true)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(f.processes().len(), 2, "not switched");
    assert_eq!(
        f.st.control.account_of(&id).as_deref(),
        Some(pro.id.as_str())
    );

    // Now: the turn is interrupted; the message queued behind it goes to
    // the resumed process, whole.
    owned::send(&f.st, &id, "slow", &[]).await.unwrap();
    let long = format!("after {}", "x".repeat(3000));
    let receipt = owned::send(&f.st, &id, &long, &[]).await.unwrap();
    assert_eq!(
        receipt.delivery,
        mother_claude_lib::claude::model::Delivery::Queued
    );
    let out = f.switch(&desktop(), &id, "default", Some("now")).await;
    assert!(matches!(out, Outcome::Switched { moved: false, .. }));
    let second = f.received_by(1);
    assert!(second
        .iter()
        .any(|m| m["request"]["subtype"] == "cancel_async_message"
            && m["request"]["message_uuid"] == json!(receipt.message_id)));
    assert!(second
        .iter()
        .any(|m| m["request"]["subtype"] == "interrupt"));
    f.processes_once(3).await;
    wait_for("the queued message in the resumed process", || {
        f.received_by(2)
            .iter()
            .any(|m| m["type"] == "user" && m["message"]["content"][0]["text"] == json!(long))
    })
    .await;
    assert_eq!(f.st.control.account_of(&id).as_deref(), Some("default"));
    f.stop(&id).await;
}

/// What cannot switch says why, and changes nothing.
#[tokio::test]
async fn the_refusals() {
    let f = Fixture::new();
    let pro = f.pro();
    let id = f
        .spawn(SpawnRequest {
            prompt: Some("hello".into()),
            ..Default::default()
        })
        .await;
    let d = desktop();
    assert_eq!(
        f.refused(&d, "--help", &pro.id, None).await,
        "invalid_session_id"
    );
    assert_eq!(
        f.refused(&d, &id, "acc_000000000000", None).await,
        "unknown_account"
    );
    assert_eq!(f.refused(&d, &id, "", None).await, "unknown_account");
    assert_eq!(
        f.refused(&d, &id, &pro.id, Some("later")).await,
        "invalid_when"
    );
    let empty = f.st.accounts.add_token("Empty", None, None).unwrap();
    assert_eq!(
        f.refused(&d, &id, &empty.id, None).await,
        "account_token_missing"
    );
    assert_eq!(
        f.refused(&d, "0b0e9c5e-3a52-4d4c-9d59-5f0b9a1e2c3d", &pro.id, None)
            .await,
        "session_not_found"
    );
    // A session another process runs.
    let foreign = "1c1e9c5e-3a52-4d4c-9d59-5f0b9a1e2c3d";
    f.st.live.insert_for_test(LiveSession {
        pid: Some(std::process::id()),
        session_id: Some(foreign.into()),
        cwd: Some(f.cwd()),
        home_account: Some("default".into()),
        ..Default::default()
    });
    assert_eq!(
        f.refused(&d, foreign, &pro.id, None).await,
        "session_not_owned"
    );
    assert_eq!(f.processes().len(), 1, "nothing was stopped");
    assert_eq!(f.st.control.account_of(&id).as_deref(), Some("default"));
    f.stop(&id).await;

    // A session without permission prompts is resumed that way: only a
    // caller allowed dangerous actions may switch it.
    let id = f
        .spawn(SpawnRequest {
            prompt: Some("hello".into()),
            permission_mode: Some("bypassPermissions".into()),
            ..Default::default()
        })
        .await;
    assert_eq!(
        f.refused(&phone(), &id, &pro.id, None).await,
        "dangerous_blocked"
    );
    assert!(matches!(
        f.switch(&d, &id, &pro.id, None).await,
        Outcome::Switched { .. }
    ));
    let line = argv(&f.processes_once(3).await[2]);
    assert!(
        line.contains("--permission-mode bypassPermissions"),
        "{line}"
    );
    assert!(
        line.contains("--allow-dangerously-skip-permissions"),
        "{line}"
    );
    f.stop(&id).await;
}

/// A session nobody runs: within the shared folder it is recorded as the
/// other account's, and its next continue runs there; into a directory
/// account it moves (with the pair's consent).
#[tokio::test]
async fn a_session_nobody_runs_switches_for_its_next_continue() {
    let f = Fixture::new();
    let pro = f.pro();
    let work = f.work();
    let id = "2d2e9c5e-3a52-4d4c-9d59-5f0b9a1e2c3d";
    f.write_transcript(f.home.path(), id);
    mother_claude_lib::server::monitor::refresh_once(&f.st).await;
    let d = desktop();

    let out = f.switch(&d, id, &pro.id, None).await;
    assert_eq!(
        out,
        Outcome::Switched {
            account: pro.id.clone(),
            moved: false,
            job: None
        }
    );
    assert_eq!(
        mother_claude_lib::claude::summaries::session_account(&f.st, id),
        pro.id
    );
    assert_eq!(
        f.st.find_session(id).await.unwrap().account.unwrap().id,
        pro.id
    );
    let resumed = owned::continue_as(
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
    assert_eq!(resumed, id, "resumed in place");
    wait_for("it to start", || !f.processes().is_empty()).await;
    assert_eq!(
        f.processes()[0]["account"]["oauthTokenSha256"],
        Secret::new(TOKEN).sha256_hex()
    );
    wait_for("the turn", || {
        owned::live_state(&f.st, id).is_some_and(|l| l.started && !l.turn_active)
    })
    .await;
    f.stop(id).await;
    mother_claude_lib::server::monitor::tick_once(&f.st).await;

    // Into Work's directory: with the consent only, as a move.
    assert_eq!(
        f.refused(&d, id, &work.id, None).await,
        "copy_consent_required"
    );
    f.st.accounts
        .set_copy_consent(&pro.id, &work.id, true)
        .unwrap();
    let Outcome::Switched {
        moved: true,
        job: Some(job),
        ..
    } = f.switch(&d, id, &work.id, None).await
    else {
        panic!("not moved")
    };
    let work_dir = work.dir.clone().unwrap();
    assert!(f.transcript_in(&work_dir, id).is_file());
    assert!(!f.transcript_in(f.home.path(), id).exists());
    assert_eq!(f.st.jobs.get(&job).unwrap().kind, "move");
    assert_eq!(
        mother_claude_lib::claude::summaries::session_account(&f.st, id),
        work.id
    );
}

// ─────────────────────────────── over HTTP ───────────────────────────────

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

/// The routes: a phone may switch (audited, the desktop told), a pending
/// switch answers 202 and `DELETE` cancels it.
#[tokio::test]
async fn over_http_a_switch_is_audited_and_a_device_is_named_to_the_desktop() {
    let f = Fixture::new();
    let pro = f.pro();
    let id = f
        .spawn(SpawnRequest {
            prompt: Some("hello".into()),
            ..Default::default()
        })
        .await;
    // The CLI recorded the folder: a phone may resume there.
    f.write_transcript(f.home.path(), &id);
    let base = serve(mother_claude_lib::server::router(f.st.clone())).await;
    let phone =
        f.st.devices
            .issue(
                "Phone",
                "ios",
                None,
                Ingress::Lan,
                "192.168.1.60".parse().unwrap(),
            )
            .1;
    let c = reqwest::Client::new();
    let url = format!("{base}/api/sessions/{id}/account");
    let mut rx = f.st.bus.subscribe();

    let r = c
        .post(&url)
        .bearer_auth(&phone)
        .json(&json!({ "account": pro.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v, json!({ "id": id, "account": pro.id, "moved": false }));
    let entry =
        f.st.devices
            .audit
            .recent(50)
            .into_iter()
            .rfind(|e| e.action == "session.account")
            .expect("audited");
    let detail = entry.detail.unwrap();
    assert_eq!(
        (detail["from"].clone(), detail["to"].clone()),
        (json!("default"), json!(pro.id))
    );
    let mut told = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let ServerEvent::DesktopNotice { text, .. } = &*ev {
            told.push(text.clone());
        }
    }
    assert!(
        told.iter()
            .any(|t| t.contains("\"Phone\"") && t.contains("switched session")),
        "{told:?}"
    );

    // Busy: 202, then cancelled with DELETE (the desktop's token).
    owned::send(&f.st, &id, "slow", &[]).await.unwrap();
    let desk = f.st.auth.token.clone();
    let r = c
        .post(&url)
        .bearer_auth(&desk)
        .json(&json!({ "account": "default", "when": "idle" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["pending"], true);
    let r = c.delete(&url).bearer_auth(&desk).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v, json!({ "id": id, "account": pro.id, "cancelled": true }));
    let r = c.delete(&url).bearer_auth(&desk).send().await.unwrap();
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "switch_not_pending");
    assert!(f
        .st
        .devices
        .audit
        .recent(50)
        .iter()
        .any(|e| e.action == "session.account.cancel"));
    let _ = Arc::clone(&f.st);
    f.stop(&id).await;
}
