//! Sessions working together (docs/MCP.md "Sessions working together"): a
//! session reads, messages, asks and continues another through Mother
//! Claude's `session_*` tools — here, against a FAKE `claude`
//! (`tests/fixtures/fake-claude.py`, selected with `MOTHER_CLAUDE_CLI`),
//! and on another computer: two app states in one process, each with its
//! own data dir, Claude home and iroh endpoint, paired over real QUIC (the
//! `tests/work.rs` harness). Approvals are answered as a dashboard answers
//! them (`pending::respond`). Nothing touches `~/.claude` or
//! `~/.claude.json` (`CLAUDE_CONFIG_DIR` is set) or runs the real CLI.

// Tests start their programs directly; the app's own code goes through
// `proc`, which keeps console windows from flashing up on Windows.
#![allow(clippy::disallowed_methods)]
#![cfg(unix)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::model::{PendingItem, PendingSource, RuleScope};
use mother_claude_lib::claude::owned::{self, Caller, SpawnRequest};
use mother_claude_lib::claude::{encode_cwd, ClaudeHome};
use mother_claude_lib::machines;
use mother_claude_lib::mcp::{self, ToolCtx};
use mother_claude_lib::pending::{self, Decision, Remember};
use mother_claude_lib::server::context::{ClientContext, Principal};
use mother_claude_lib::server::{self, auth::Auth, monitor};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

/// The process-wide part, once: the fake CLI, and with `CLAUDE_CONFIG_DIR`
/// set, a test home's `.claude.json` is inside it, never the user's own.
fn setup() {
    static ONCE: OnceLock<PathBuf> = OnceLock::new();
    ONCE.get_or_init(|| {
        let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-claude.py");
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755));
        }
        std::env::set_var("MOTHER_CLAUDE_CLI", &fake);
        std::env::remove_var("MOTHER_CLAUDE_SIDECAR");
        let dir = tempfile::tempdir().unwrap().keep();
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.join("unused-config"));
        dir
    });
}

/// The asking session (owned, as an owned session names itself to `/mcp`).
const SID: &str = "a0ee0000-1111-4222-8333-444455556666";

const DESKTOP: Caller = Caller {
    may_dangerous: true,
    desktop: true,
};

fn desktop() -> ClientContext {
    let mut c = ClientContext::unknown();
    c.principal = Principal::Desktop;
    c
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn read_log(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("fake-claude.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// The texts of the user messages the fake read in `dir`.
fn received_texts(dir: &Path) -> Vec<String> {
    read_log(dir)
        .into_iter()
        .filter_map(|l| l.get("in").cloned())
        .filter(|m| m["type"] == "user")
        .filter_map(|m| {
            m["message"]["content"][0]["text"]
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

async fn wait_for<F: FnMut() -> bool>(what: &str, mut f: F) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn turn_done(st: &AppState, id: &str) {
    wait_for("the turn to end", || {
        owned::live_state(st, id).is_some_and(|l| l.started && !l.turn_active)
    })
    .await;
}

/// A transcript Claude Code would have written for `id` in `cwd`.
fn transcript(home: &Path, cwd: &str, id: &str, prompt: &str, answer: &str) {
    let dir = home.join("projects").join(encode_cwd(cwd));
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = now() - 60_000;
    let lines = [
        json!({"type":"user","uuid":"u-1","timestamp":iso(t0),"cwd":cwd,"sessionId":id,
               "message":{"role":"user","content":prompt}}),
        json!({"type":"assistant","uuid":"a-1","timestamp":iso(t0+1000),"cwd":cwd,"sessionId":id,
               "message":{"id":"msg_1","model":"claude-haiku-4-5","role":"assistant",
                          "content":[{"type":"text","text":answer}],
                          "usage":{"input_tokens":10,"output_tokens":5}}}),
    ];
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    std::fs::write(dir.join(format!("{id}.jsonl")), text).unwrap();
}

/// The session card waiting on the asking session.
async fn card(st: &AppState) -> PendingItem {
    let mut found = None;
    wait_for("a session card", || {
        found = st
            .pending
            .list(SID)
            .into_iter()
            .find(|i| i.source == PendingSource::Session);
        found.is_some()
    })
    .await;
    found.unwrap()
}

async fn allow(st: &AppState, item: &PendingItem, remember: Option<RuleScope>) {
    let d = Decision {
        remember: remember.map(|scope| Remember {
            scope,
            patterns: Vec::new(),
        }),
        ..Decision::allow()
    };
    pending::respond(st, SID, &item.id, d, &desktop())
        .await
        .unwrap();
}

/// A tool call of the asking session, running on its own.
fn tool(
    st: &AppState,
    cwd: &str,
    name: &'static str,
    args: Value,
) -> tokio::task::JoinHandle<Value> {
    let ctx = ToolCtx {
        state: st.clone(),
        caller: mcp::Caller {
            session_id: Some(SID.into()),
            pid: None,
            cwd: Some(cwd.into()),
            owned: true,
        },
    };
    tokio::spawn(async move {
        let out = mcp::registry().call(name, ctx, args).await.unwrap();
        serde_json::to_value(out).unwrap()
    })
}

fn text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

// ─── on this computer ──────────────────────────────────────────────────────────

struct Here {
    st: AppState,
    home: TempDir,
    /// The asking session's folder, and the other session's.
    a: TempDir,
    b: TempDir,
}

impl Here {
    async fn new() -> Self {
        setup();
        let home = tempfile::tempdir().unwrap();
        // A home with a roster (`sessions/`): a fresh transcript alone is
        // not a running session.
        std::fs::create_dir_all(home.path().join("sessions")).unwrap();
        let st = Inner::new(
            ClaudeHome::with_base(home.path()),
            ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            Auth::ephemeral(),
        );
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        st.mark_owned(SID, &a.path().to_string_lossy(), 1).await;
        Self { st, home, a, b }
    }

    fn a(&self) -> String {
        self.a.path().to_string_lossy().into_owned()
    }

    fn b(&self) -> String {
        self.b.path().to_string_lossy().into_owned()
    }

    /// The other session: Mother Claude runs it, its first turn done.
    async fn other(&self) -> String {
        let id = owned::spawn_as(
            &self.st,
            DESKTOP,
            SpawnRequest {
                cwd: self.b(),
                prompt: Some("hello".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        turn_done(&self.st, &id).await;
        id
    }

    fn tool(&self, name: &'static str, args: Value) -> tokio::task::JoinHandle<Value> {
        tool(&self.st, &self.a(), name, args)
    }
}

#[tokio::test]
async fn a_session_messages_and_asks_another_once_the_user_allows_it() {
    let h = Here::new().await;
    let b = h.other().await;

    // A message: a card on the asking session first.
    let call = h.tool(
        "session_send",
        json!({ "session": b, "message": "Please update the docs." }),
    );
    let item = card(&h.st).await;
    assert_eq!(item.tool_name.as_deref(), Some("session_send"));
    let input = item.input.clone().unwrap();
    assert_eq!(input["session"], b.as_str());
    assert_eq!(input["message"], "Please update the docs.");
    assert_eq!(
        serde_json::to_value(item.grant.as_ref().unwrap()).unwrap()["kind"],
        "sessions"
    );
    allow(&h.st, &item, None).await;
    let out = call.await.unwrap();
    assert!(out.get("isError").is_none(), "{out}");
    let v = &out["structuredContent"];
    assert_eq!(v["delivered"], "sent", "{v}");
    assert_eq!(v["session"], b.as_str());

    // It got the message, saying who from and how to answer.
    let b_dir = h.b.path().to_path_buf();
    let mut got = String::new();
    wait_for("the message", || {
        got = received_texts(&b_dir)
            .into_iter()
            .find(|t| t.contains("Please update the docs."))
            .unwrap_or_default();
        !got.is_empty()
    })
    .await;
    assert!(got.starts_with("[Message from the session"), "{got}");
    assert!(got.contains(&format!("session_send to {SID} on ")), "{got}");
    assert!(got.ends_with("\n\nPlease update the docs."), "{got}");

    // The audit keeps the text's hash, never the text.
    let entries = h.st.devices.audit.recent(50);
    let sent = entries
        .iter()
        .find(|e| e.action == "session.send")
        .expect("audited");
    let line = serde_json::to_string(sent).unwrap();
    assert!(!line.contains("update the docs"), "{line}");
    assert_eq!(
        sent.detail.as_ref().unwrap()["textSha256"],
        mother_claude_lib::claude::secrets::sha256_hex(b"Please update the docs.")
    );

    // A question, allowed for this session from now on: its answer comes
    // back once its turn ends.
    let call = h.tool(
        "session_ask",
        json!({ "session": b, "question": "What is 2+2?", "timeout_s": 20 }),
    );
    let item = card(&h.st).await;
    assert_eq!(item.tool_name.as_deref(), Some("session_ask"));
    allow(&h.st, &item, Some(RuleScope::Session)).await;
    let out = call.await.unwrap();
    let v = &out["structuredContent"];
    assert_eq!(v["finished"], true, "{out}");
    assert_eq!(v["answer"], "You asked: What is 2+2?");

    // The next one needs no card: the permission covers it (named by the
    // first characters of its id).
    let out = h
        .tool(
            "session_ask",
            json!({ "session": &b[..8], "question": "slow", "timeout_s": 20 }),
        )
        .await
        .unwrap();
    let v = &out["structuredContent"];
    assert_eq!(v["answer"], "You asked: slow", "{out}");
    assert!(
        v["approvedBy"]
            .as_str()
            .unwrap()
            .starts_with("your permission (talking to other sessions"),
        "{v}"
    );
    assert!(h.st.pending.list(SID).is_empty());

    // Too short a wait: what it knows.
    let out = h
        .tool(
            "session_ask",
            json!({ "session": b, "question": "slow", "timeout_s": 1 }),
        )
        .await
        .unwrap();
    let v = &out["structuredContent"];
    assert_eq!(v["finished"], false, "{out}");
    assert!(v["note"].as_str().unwrap().contains("session_read"), "{v}");

    // Never itself.
    let out = h
        .tool("session_send", json!({ "session": SID, "message": "hi" }))
        .await
        .unwrap();
    assert_eq!(out["isError"], true);
    assert!(text(&out).ends_with("(self)"), "{out}");
    owned::kill(&h.st, &b).await.unwrap();
}

#[tokio::test]
async fn a_session_reads_another_and_continues_one_that_ended() {
    let h = Here::new().await;
    let b = h.other().await;
    // What Claude Code wrote for it: the folder is one it was used in.
    transcript(
        h.home.path(),
        &h.b(),
        &b,
        "Plan the release",
        "Ship it on Friday.",
    );
    owned::kill(&h.st, &b).await.unwrap();
    wait_for("the reaper", || !owned::has_handle(&h.st, &b)).await;
    wait_for("unmark", || !owned::is_owned(&h.st, &b)).await;
    monitor::refresh_once(&h.st).await;

    // Reading: a card, then its last messages.
    let call = h.tool("session_read", json!({ "session": b, "last": 5 }));
    let item = card(&h.st).await;
    assert_eq!(item.tool_name.as_deref(), Some("session_read"));
    allow(&h.st, &item, None).await;
    let out = call.await.unwrap();
    let read = text(&out);
    assert!(read.contains("not running"), "{read}");
    assert!(read.contains("[user"), "{read}");
    assert!(read.contains("Plan the release"), "{read}");
    assert!(read.contains("Ship it on Friday."), "{read}");

    // A message to it continues it, as a session Mother Claude runs.
    let call = h.tool(
        "session_send",
        json!({ "session": b, "message": "Pick it up again." }),
    );
    let item = card(&h.st).await;
    allow(&h.st, &item, None).await;
    let out = call.await.unwrap();
    let v = &out["structuredContent"];
    assert_eq!(v["delivered"], "started", "{out}");
    assert_eq!(v["session"], b.as_str(), "resumed in place");
    let b_dir = h.b.path().to_path_buf();
    wait_for("the resume", || {
        received_texts(&b_dir)
            .iter()
            .any(|t| t.ends_with("\n\nPick it up again."))
    })
    .await;
    let argv: Vec<String> = read_log(&b_dir)
        .into_iter()
        .filter_map(|l| l.get("argv").cloned())
        .filter_map(|a| serde_json::from_value::<Vec<String>>(a).ok())
        .map(|a| a.join(" "))
        .collect();
    assert!(
        argv.iter().any(|a| a.contains(&format!("--resume {b}"))),
        "{argv:?}"
    );
    assert!(owned::has_handle(&h.st, &b));
    owned::kill(&h.st, &b).await.unwrap();
}

#[tokio::test]
async fn a_session_starts_another_in_a_project_and_waits_for_its_answer() {
    let h = Here::new().await;
    // The folder Claude Code was used in.
    transcript(
        h.home.path(),
        &h.b(),
        "b0b00000-1111-4222-8333-444455556666",
        "hi",
        "hello",
    );
    monitor::refresh_once(&h.st).await;
    let call = h.tool(
        "session_start",
        json!({ "folder": h.b(), "prompt": "What is 2+2?", "wait": true, "timeout_s": 20 }),
    );
    let item = card(&h.st).await;
    assert_eq!(item.tool_name.as_deref(), Some("session_start"));
    assert_eq!(item.input.as_ref().unwrap()["folder"], h.b().as_str());
    allow(&h.st, &item, None).await;
    let out = call.await.unwrap();
    let v = &out["structuredContent"];
    assert_eq!(v["started"], true, "{out}");
    assert_eq!(v["finished"], true, "{out}");
    // Its first message starts with who started it.
    assert_eq!(v["answer"], "You asked: What is 2+2?", "{out}");
    let id = v["session"].as_str().unwrap().to_string();
    assert!(owned::has_handle(&h.st, &id));
    owned::kill(&h.st, &id).await.unwrap();

    // A folder Claude Code was never used in is refused (after the card).
    let fresh = tempfile::tempdir().unwrap();
    let call = h.tool(
        "session_start",
        json!({ "folder": fresh.path().to_string_lossy(), "prompt": "hi" }),
    );
    let item = card(&h.st).await;
    allow(&h.st, &item, None).await;
    let out = call.await.unwrap();
    assert_eq!(out["isError"], true);
    assert!(text(&out).ends_with("(untrusted_cwd)"), "{out}");
}

// ─── on another computer ───────────────────────────────────────────────────────

struct Machine {
    st: AppState,
    url: String,
    dir: TempDir,
}

async fn machine(lookup: &MemoryLookup) -> Machine {
    setup();
    let dir = tempfile::tempdir().unwrap();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(dir.path().join("data"));
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    let st = Inner::new(
        ClaudeHome::with_base(dir.path().join("home")),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
    );
    st.machines.use_test_lookup(lookup.clone());
    st.machines.use_isolated_git();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::router(st.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Machine {
        st,
        url: format!("http://{addr}"),
        dir,
    }
}

impl Machine {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let client = reqwest::Client::new();
        let url = format!("{}/api{path}", self.url);
        let req = match method {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PATCH" => client.patch(url),
            m => panic!("method {m}"),
        };
        let req = req.bearer_auth(&self.st.auth.token);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        let resp = req.send().await.unwrap();
        let status = resp.status();
        (status, resp.json::<Value>().await.unwrap_or(Value::Null))
    }

    async fn wait(&self, what: &str, pred: impl Fn(&Value) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let (s, v) = self.call("GET", "/machines", None).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            if pred(&v) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {v:#}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn id(&self) -> String {
        machines::machine_info(&self.st).id
    }
}

fn state_of(v: &Value) -> &str {
    v["pairing"]["state"].as_str().unwrap_or("")
}

fn key_of(m: &Machine, peer: &str) -> String {
    m.st.machines
        .store_read(|f| f.peer(peer).map(|p| p.key.clone()))
        .expect("paired")
}

/// View and drive, both ways; Machines on on both.
async fn pair(a: &Machine, b: &Machine) {
    let grants = json!({
        "view": {"lan": true, "internet": true},
        "control": {"lan": true, "internet": true},
        "move": {"lan": false, "internet": false},
        "run": {"lan": false, "internet": false},
        "dangerous": {"lan": false, "internet": false}
    });
    let (s, v) = a.call("POST", "/machines/pair", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let code = v["pairing"]["code"].as_str().expect("a code").to_string();
    let (s, v) = b
        .call("POST", "/machines/join", Some(json!({ "code": code })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    b.wait("the joiner's digits", |v| state_of(v) == "confirm")
        .await;
    a.wait("the host's digits", |v| state_of(v) == "confirm")
        .await;
    for m in [b, a] {
        let (s, v) = m
            .call(
                "POST",
                "/machines/confirm",
                Some(json!({ "grants": grants })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    b.wait("the joiner to finish", |v| state_of(v) == "paired")
        .await;
    machines::greet(a.st.clone(), b.id(), key_of(a, &b.id())).await;
    machines::greet(b.st.clone(), a.id(), key_of(b, &a.id())).await;
    machines::set_enabled(&a.st, true);
    machines::set_enabled(&b.st, true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_reads_and_messages_one_on_another_computer() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    for (m, name) in [(&a, "Studio"), (&b, "Tower")] {
        let (s, v) = m
            .call("PATCH", "/machines/self", Some(json!({ "name": name })))
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    pair(&a, &b).await;
    let a_folder = a.dir.path().join("work");
    std::fs::create_dir_all(&a_folder).unwrap();
    let a_folder = a_folder.to_string_lossy().into_owned();
    a.st.mark_owned(SID, &a_folder, 1).await;

    // On B: a session Mother Claude runs there, with what it said so far.
    let b_folder = tempfile::tempdir().unwrap();
    let b_cwd = b_folder.path().to_string_lossy().into_owned();
    let t = owned::spawn_as(
        &b.st,
        DESKTOP,
        SpawnRequest {
            cwd: b_cwd.clone(),
            prompt: Some("hello".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    turn_done(&b.st, &t).await;
    transcript(
        &b.dir.path().join("home"),
        &b_cwd,
        &t,
        "Dub the trailer",
        "The French track is done.",
    );
    monitor::refresh_once(&b.st).await;
    let b_name = machines::this_name(&b.st);

    // Read from A: a card on A, then B's transcript.
    let call = tool(
        &a.st,
        &a_folder,
        "session_read",
        json!({ "session": t, "computer": b_name }),
    );
    let item = card(&a.st).await;
    let input = item.input.clone().unwrap();
    assert_eq!(input["here"], false);
    assert_eq!(input["machine"], b.id().as_str());
    assert!(
        item.title.ends_with(&format!(" on {b_name}")),
        "{}",
        item.title
    );
    allow(&a.st, &item, None).await;
    let out = call.await.unwrap();
    let read = text(&out);
    assert!(read.contains(&format!("on {b_name})")), "{read}");
    assert!(read.contains("The French track is done."), "{read}");

    // A message from A reaches it on B, saying it comes from A.
    let call = tool(
        &a.st,
        &a_folder,
        "session_send",
        json!({ "session": t, "computer": b_name, "message": "Start on the German track." }),
    );
    let item = card(&a.st).await;
    allow(&a.st, &item, None).await;
    let out = call.await.unwrap();
    let v = &out["structuredContent"];
    assert_eq!(v["delivered"], "sent", "{out}");
    assert_eq!(v["computer"], b_name.as_str());
    let a_name = machines::this_name(&a.st);
    let dir = b_folder.path().to_path_buf();
    let mut got = String::new();
    wait_for("the message on B", || {
        got = received_texts(&dir)
            .into_iter()
            .find(|t| t.contains("German track"))
            .unwrap_or_default();
        !got.is_empty()
    })
    .await;
    assert!(
        got.contains(&format!("session_send to {SID} on {a_name}]")),
        "{got}"
    );
    owned::kill(&b.st, &t).await.unwrap();
}
