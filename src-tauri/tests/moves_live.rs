//! Machines: a session that moved away and came back is driven again on the
//! computer it came back to, straight away — its model, permission mode and
//! effort change live, as before it left (docs/PLAN-MACHINES.md §5.8).
//!
//! Two app states in one process, each with its own data dir, Claude home
//! and iroh endpoint, paired over real QUIC (the `tests/moves.rs` harness),
//! with resumes turned on: each side starts the session again as an owned
//! session of the FAKE `claude` (`tests/fixtures/fake-claude.py`, selected
//! with `MOTHER_CLAUDE_CLI`). The real CLI never starts, and nothing touches
//! `~/.claude` or `~/.claude.json`: `CLAUDE_CONFIG_DIR` is set.
#![cfg(unix)]
// Tests start their programs directly; the app's own code goes through
// `proc`, which keeps console windows from flashing up on Windows.
#![allow(clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::machines::{self, lease};
use mother_claude_lib::server::{self, auth::Auth};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

/// The process-wide part, once: a test home's `.claude.json` is inside
/// `CLAUDE_CONFIG_DIR`, and `claude` is the fake.
fn isolate() {
    static ONCE: OnceLock<PathBuf> = OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.join("unused-config"));
        let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-claude.py");
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755));
        }
        std::env::set_var("MOTHER_CLAUDE_CLI", &fake);
        std::env::remove_var("MOTHER_CLAUDE_SIDECAR");
        dir
    });
}

struct Machine {
    st: AppState,
    url: String,
    dir: TempDir,
}

async fn machine(lookup: &MemoryLookup) -> Machine {
    isolate();
    let dir = tempfile::tempdir().unwrap();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(dir.path().join("data"));
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    // A pidfile folder, as every current CLI keeps: liveness comes from
    // pidfiles, never from a transcript written a moment ago.
    std::fs::create_dir_all(dir.path().join("home").join("sessions")).unwrap();
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
    // Moved sessions start again for real (the fake CLI).
    st.machines.enable_resumes();
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
            m => panic!("method {m}"),
        };
        let req = req.bearer_auth(&self.st.auth.token);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let v = resp.json::<Value>().await.unwrap_or(Value::Null);
        (status, v)
    }

    async fn wait(&self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let (s, v) = self.call("GET", "/machines", None).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            if pred(&v) {
                return v;
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

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    async fn refresh(&self) {
        server::monitor::refresh_once(&self.st).await;
    }

    /// The session as this computer's list shows it.
    async fn session(&self) -> Value {
        let (s, v) = self.call("GET", &format!("/sessions/{SID}"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v.get("session").cloned().unwrap_or(v)
    }
}

fn state_of(v: &Value) -> &str {
    v["pairing"]["state"].as_str().unwrap_or("")
}

/// Pairs `a` (host) and `b` (joiner) with `grants` both ways, greets, and
/// turns Machines on on both.
async fn pair(a: &Machine, b: &Machine) {
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
                Some(json!({ "grants": grants() })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    b.wait("the joiner to finish", |v| state_of(v) == "paired")
        .await;
    for (x, y) in [(a, b), (b, a)] {
        let key =
            x.st.machines
                .store_read(|f| f.peer(&y.id()).map(|p| p.key.clone()))
                .expect("paired");
        machines::greet(x.st.clone(), y.id(), key).await;
    }
    machines::set_enabled(&a.st, true);
    machines::set_enabled(&b.st, true);
}

/// View, drive, move and the dangerous right both ways (the session runs
/// without prompts, as the one reported did); no run.
fn grants() -> Value {
    json!({
        "view": {"lan": true, "internet": true},
        "control": {"lan": true, "internet": true},
        "move": {"lan": true, "internet": true},
        "run": {"lan": false, "internet": false},
        "dangerous": {"lan": true, "internet": true}
    })
}

const SID: &str = "c0ee0000-1111-4222-8333-444455556666";

fn canon(p: &Path) -> String {
    dunce::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn transcript_in(home: &Path, cwd: &str) -> PathBuf {
    home.join("projects")
        .join(mother_claude_lib::claude::encode_cwd(cwd))
        .join(format!("{SID}.jsonl"))
}

/// A finished turn of the session in `cwd`, on `m`'s disk.
fn session_on(m: &Machine, cwd: &str) {
    let t = transcript_in(&m.home(), cwd);
    std::fs::create_dir_all(t.parent().unwrap()).unwrap();
    let lines = [
        json!({"type": "user", "uuid": "u-1", "sessionId": SID, "cwd": cwd,
               "timestamp": "2026-11-16T09:00:00.000Z", "permissionMode": "bypassPermissions",
               "message": {"role": "user", "content": "make the tray icon work"}}),
        json!({"type": "assistant", "uuid": "a-1", "parentUuid": "u-1", "sessionId": SID,
               "cwd": cwd, "timestamp": "2026-11-16T09:00:05.000Z",
               "message": {"id": "msg_1", "role": "assistant", "model": "claude-opus-5-5",
                           "content": [{"type": "text", "text": "done"}],
                           "usage": {"input_tokens": 1, "output_tokens": 1}}}),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&t, text).unwrap();
}

/// Claude Code on `m` has worked in `dir` before (its trust).
fn trusted_on(m: &Machine, dir: &str) {
    let other = "d0ee0000-1111-4222-8333-444455556666";
    let t = m
        .home()
        .join("projects")
        .join(mother_claude_lib::claude::encode_cwd(dir))
        .join(format!("{other}.jsonl"));
    std::fs::create_dir_all(t.parent().unwrap()).unwrap();
    std::fs::write(
        &t,
        format!(
            "{}\n",
            json!({"type": "user", "uuid": "x", "sessionId": other, "cwd": dir,
                   "message": {"role": "user", "content": "hi"}})
        ),
    )
    .unwrap();
}

/// Starts a move on `m` and waits until its record says it ended.
async fn move_on(m: &Machine, body: Value) -> Value {
    let (s, v) = m
        .call("POST", &format!("/sessions/{SID}/move"), Some(body))
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id = v["id"].as_str().expect("a move id").to_string();
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let (s, r) = m.call("GET", &format!("/moves/{id}"), None).await;
        assert_eq!(s, StatusCode::OK, "{r}");
        if r["finished"] == true || !r["resumeError"].is_null() {
            return r;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the move: {r:#}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits until `m`'s summary of the session passes `pred`.
async fn until_session(m: &Machine, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        m.refresh().await;
        let s = m.session().await;
        if pred(&s) {
            return s;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {s:#}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn drivable(s: &Value) -> bool {
    s["owned"] == true
        && s["live"] == true
        && s["livesOn"].is_null()
        && s["control"]["channel"] == "owned"
        && s["control"]["setMode"] == "any"
        && s["control"]["setEffort"] == true
}

/// Changes the mode and the effort through the routes the chips use, and
/// waits until the list shows both.
async fn settings_change_on(m: &Machine, mode: &str, effort: &str) {
    let (s, v) = m
        .call(
            "POST",
            &format!("/sessions/{SID}/mode"),
            Some(json!({ "mode": mode })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "mode: {v}");
    let (s, v) = m
        .call(
            "POST",
            &format!("/sessions/{SID}/effort"),
            Some(json!({ "effort": effort })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "effort: {v}");
    until_session(m, "the new settings", |s| {
        s["permissionMode"] == mode && s["effort"] == effort
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_brought_back_takes_setting_changes_at_once() {
    let lookup = MemoryLookup::new();
    let s = machine(&lookup).await;
    let t = machine(&lookup).await;
    pair(&s, &t).await;
    let folders = tempfile::tempdir().unwrap();
    let s_dir = folders.path().join("s-app");
    let t_dir = folders.path().join("t-app");
    std::fs::create_dir_all(&s_dir).unwrap();
    std::fs::create_dir_all(&t_dir).unwrap();
    let (s_cwd, t_cwd) = (canon(&s_dir), canon(&t_dir));
    trusted_on(&s, &s_cwd);
    trusted_on(&t, &t_cwd);
    session_on(&s, &s_cwd);
    s.refresh().await;

    // It runs here, owned, without prompts.
    let (st, v) = s
        .call(
            "POST",
            &format!("/sessions/{SID}/continue"),
            Some(json!({ "permissionMode": "bypassPermissions", "effort": "max" })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["id"], SID);
    until_session(&s, "the session to run here", drivable).await;

    // Away to the other computer; it runs there.
    let away = move_on(
        &s,
        json!({ "to": t.id(), "folder": t_cwd, "code": "none", "when": "now" }),
    )
    .await;
    assert_eq!(away["state"], "resumed", "{away:#}");
    until_session(&t, "the session to run there", drivable).await;
    settings_change_on(&t, "acceptEdits", "high").await;

    // Brought back, from this computer (through the other one, as *Bring
    // back to this computer* does): the move runs there.
    let (st, v) = s
        .call(
            "POST",
            &format!("/machines/{}/proxy/sessions/{SID}/move", t.id()),
            Some(json!({ "to": s.id(), "code": "none", "when": "now" })),
        )
        .await;
    let back = if st == StatusCode::NOT_FOUND {
        // No proxy route by that name: started on the computer it lives on.
        move_on(&t, json!({ "to": s.id(), "code": "none", "when": "now" })).await
    } else {
        assert_eq!(st, StatusCode::ACCEPTED, "{v}");
        let id = v["id"].as_str().expect("a move id").to_string();
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let (st, r) = t.call("GET", &format!("/moves/{id}"), None).await;
            assert_eq!(st, StatusCode::OK, "{r}");
            if r["finished"] == true || !r["resumeError"].is_null() {
                break r;
            }
            assert!(Instant::now() < deadline, "the move back: {r:#}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    assert_eq!(back["state"], "resumed", "{back:#}");
    assert_eq!(
        lease::lease_of(&s.st, SID).map(|l| l.home),
        Some(s.id()),
        "it lives here again"
    );

    // Here again: driven at once, the settings it ran with there carried.
    let here = until_session(&s, "the session to run here again", drivable).await;
    assert_eq!(here["permissionMode"], "acceptEdits", "{here:#}");
    settings_change_on(&s, "plan", "medium").await;
    settings_change_on(&s, "bypassPermissions", "max").await;
}
