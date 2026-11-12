//! Machines, Phase 3b (docs/PLAN-MACHINES.md §5.9): keeping copies current
//! between two app states in one process, each with its own data dir,
//! Claude home and iroh endpoint, paired over real QUIC (the
//! `tests/moves.rs` harness). Rounds are driven by `copies::source::tick`
//! (the app's loop is off in tests), resumes are recorded, never `claude`
//! started. Nothing touches `~/.claude` or `~/.claude.json`:
//! `CLAUDE_CONFIG_DIR` is set, so each test home's `.claude.json` is its own.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::machines::{self, copies, lease, transport};
use mother_claude_lib::server::{self, auth::Auth};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

/// The process-wide part, once: with `CLAUDE_CONFIG_DIR` set, a test home's
/// `.claude.json` is inside it, never the user's own.
fn isolate() {
    static ONCE: OnceLock<PathBuf> = OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.join("unused-config"));
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
    // A round goes as soon as a turn ended (the app waits five seconds).
    st.machines.copies.set_debounce_ms(0);
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
            "PUT" => client.put(url),
            "PATCH" => client.patch(url),
            "DELETE" => client.delete(url),
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

    async fn view(&self) -> Value {
        let (s, v) = self.call("GET", "/machines", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    async fn wait(&self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let v = self.view().await;
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

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    async fn refresh(&self) {
        server::monitor::refresh_once(&self.st).await;
    }

    /// One look of the copies loop, after the session list caught up.
    async fn copy_now(&self) {
        self.refresh().await;
        copies::source::tick(&self.st).await;
    }
}

fn state_of(v: &Value) -> &str {
    v["pairing"]["state"].as_str().unwrap_or("")
}

/// Pairs `a` (host) and `b` (joiner) with these grants, greets both ways
/// and turns Machines on on both.
async fn pair(a: &Machine, b: &Machine, a_gives: Value, b_gives: Value) {
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
    let (s, v) = b
        .call(
            "POST",
            "/machines/confirm",
            Some(json!({ "grants": b_gives })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = a
        .call(
            "POST",
            "/machines/confirm",
            Some(json!({ "grants": a_gives })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    b.wait("the joiner to finish", |v| state_of(v) == "paired")
        .await;
    greet(a, b).await;
    machines::set_enabled(&a.st, true);
    machines::set_enabled(&b.st, true);
}

fn key_of(m: &Machine, peer: &str) -> String {
    m.st.machines
        .store_read(|f| f.peer(peer).map(|p| p.key.clone()))
        .expect("paired")
}

async fn greet(a: &Machine, b: &Machine) {
    machines::greet(a.st.clone(), b.id(), key_of(a, &b.id())).await;
    machines::greet(b.st.clone(), a.id(), key_of(b, &a.id())).await;
}

fn grants() -> Value {
    json!({
        "view": {"lan": true, "internet": true},
        "control": {"lan": true, "internet": true},
        "move": {"lan": true, "internet": true},
        "run": {"lan": false, "internet": false},
        "dangerous": {"lan": false, "internet": false}
    })
}

fn no_move() -> Value {
    let mut g = grants();
    g["move"] = json!({"lan": false, "internet": false});
    g
}

async fn eventually<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(45);
    while !f().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

// ─── sessions ──────────────────────────────────────────────────────────────────

const SID: &str = "c0ee0000-1111-4222-8333-444455556666";
const SID2: &str = "c1ee0000-1111-4222-8333-444455556666";

fn canon(p: &Path) -> String {
    dunce::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn user_line(sid: &str, cwd: &str, n: u32, text: &str) -> String {
    json!({"type": "user", "uuid": format!("u-{n}"), "sessionId": sid, "cwd": cwd,
           "timestamp": "2026-11-12T09:00:00.000Z", "permissionMode": "default",
           "message": {"role": "user", "content": text}})
    .to_string()
}

fn assistant_line(sid: &str, cwd: &str, n: u32) -> String {
    json!({"type": "assistant", "uuid": format!("a-{n}"), "parentUuid": format!("u-{n}"),
           "sessionId": sid, "cwd": cwd, "timestamp": "2026-11-12T09:00:05.000Z",
           "message": {"id": format!("msg_{n}"), "role": "assistant", "model": "claude-haiku-4-5",
                       "content": [{"type": "text", "text": "done"}],
                       "usage": {"input_tokens": 1, "output_tokens": 1}}})
    .to_string()
}

fn transcript_in(home: &Path, cwd: &str, sid: &str) -> PathBuf {
    home.join("projects")
        .join(mother_claude_lib::claude::encode_cwd(cwd))
        .join(format!("{sid}.jsonl"))
}

fn subagent_in(home: &Path, cwd: &str, sid: &str) -> PathBuf {
    home.join("projects")
        .join(mother_claude_lib::claude::encode_cwd(cwd))
        .join(sid)
        .join("subagents")
        .join("agent-a1.jsonl")
}

fn append(path: &Path, text: &str) {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

/// A session of one turn in `cwd`, with a subagent and a todo list.
async fn session_on(m: &Machine, sid: &str, cwd: &str) -> PathBuf {
    let t = transcript_in(&m.home(), cwd, sid);
    append(
        &t,
        &format!(
            "{}\n{}\n",
            user_line(sid, cwd, 1, "make the tray icon work"),
            assistant_line(sid, cwd, 1)
        ),
    );
    append(
        &subagent_in(&m.home(), cwd, sid),
        &format!("{}\n", user_line(sid, cwd, 1, "look around")),
    );
    std::fs::create_dir_all(m.home().join("todos")).unwrap();
    std::fs::write(
        m.home().join("todos").join(format!("{sid}.json")),
        r#"[{"content":"tray","status":"pending"}]"#,
    )
    .unwrap();
    m.refresh().await;
    t
}

/// One more turn, in the transcript and its subagent's (with a padding so
/// the files grow past the check's windows over a few turns).
fn turn(m: &Machine, sid: &str, cwd: &str, n: u32) {
    let pad = "x".repeat(20_000);
    append(
        &transcript_in(&m.home(), cwd, sid),
        &format!(
            "{}\n{}\n",
            user_line(sid, cwd, n, &format!("turn {n} {pad}")),
            assistant_line(sid, cwd, n)
        ),
    );
    append(
        &subagent_in(&m.home(), cwd, sid),
        &format!("{}\n", user_line(sid, cwd, n, &format!("sub {n}"))),
    );
}

/// Claude Code on `m` has worked in `dir` before (its trust).
fn trusted_on(m: &Machine, dir: &str) {
    let other = "b0ee0000-1111-4222-8333-444455556666";
    append(
        &transcript_in(&m.home(), dir, other),
        &format!(
            "{}\n",
            json!({"type": "user", "uuid": "x", "sessionId": other, "cwd": dir,
                   "message": {"role": "user", "content": "hi"}})
        ),
    );
}

/// Where `t` keeps its copy of `s`'s session `sid` (`rel` under it).
fn in_copy(t: &Machine, s: &Machine, sid: &str, rel: &Path) -> PathBuf {
    t.data()
        .join("machines")
        .join("copies")
        .join(s.id())
        .join("default")
        .join(sid)
        .join(rel)
}

fn rel_of(m: &Machine, p: &Path) -> PathBuf {
    p.strip_prefix(m.home()).unwrap().to_path_buf()
}

/// Every file of the session on `s`, byte for byte in `t`'s copy.
fn assert_copy_is_source(s: &Machine, t: &Machine, sid: &str, cwd: &str) {
    for p in [
        transcript_in(&s.home(), cwd, sid),
        subagent_in(&s.home(), cwd, sid),
        s.home().join("todos").join(format!("{sid}.json")),
    ] {
        let copy = in_copy(t, s, sid, &rel_of(s, &p));
        assert_eq!(
            std::fs::read(&copy).unwrap_or_else(|e| panic!("{}: {e}", copy.display())),
            std::fs::read(&p).unwrap(),
            "{}",
            p.display()
        );
    }
}

/// The audit lines `m` wrote for `action`, oldest first.
fn audited(m: &Machine, action: &str) -> Vec<Value> {
    let mut v: Vec<Value> =
        m.st.devices
            .audit
            .recent(500)
            .into_iter()
            .filter(|e| e.action == action)
            .map(|e| e.detail.unwrap_or(Value::Null))
            .collect();
    v.sort_by_key(|d| d["more"]["round"].as_str().map(str::to_string));
    v
}

fn sent_rounds(m: &Machine) -> usize {
    m.st.devices
        .audit
        .recent(500)
        .into_iter()
        .filter(|e| e.action == "machine.copy.sent")
        .count()
}

fn last_sent(m: &Machine) -> Value {
    m.st.devices
        .audit
        .recent(500)
        .into_iter()
        .rev()
        .find(|e| e.action == "machine.copy.sent")
        .and_then(|e| e.detail)
        .expect("a round")
}

async fn keep_copy(s: &Machine, sid: &str, on: &[String]) -> Value {
    let (st, v) = s
        .call(
            "PUT",
            &format!("/sessions/{sid}/copies"),
            Some(json!({ "machines": on })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    v
}

fn status_on(v: &Value, machine: &str) -> Value {
    v["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == machine)
        .map(|m| m["status"].clone())
        .unwrap_or(Value::Null)
}

/// Two paired machines; the session works in a folder of the source's and
/// the target has worked in its own folder before.
struct World {
    s: Machine,
    t: Machine,
    _folders: TempDir,
    s_cwd: String,
    t_cwd: String,
    transcript: PathBuf,
}

async fn world_with(s_gives: Value, t_gives: Value) -> World {
    let lookup = MemoryLookup::new();
    let s = machine(&lookup).await;
    let t = machine(&lookup).await;
    pair(&s, &t, s_gives, t_gives).await;
    let folders = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(folders.path().join("s-app")).unwrap();
    std::fs::create_dir_all(folders.path().join("t-app")).unwrap();
    let s_cwd = canon(&folders.path().join("s-app"));
    let t_cwd = canon(&folders.path().join("t-app"));
    trusted_on(&t, &t_cwd);
    let transcript = session_on(&s, SID, &s_cwd).await;
    World {
        s,
        t,
        _folders: folders,
        s_cwd,
        t_cwd,
        transcript,
    }
}

async fn world() -> World {
    world_with(grants(), grants()).await
}

// ─── the tests ─────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deltas_across_many_turns_equal_the_source_byte_for_byte() {
    let w = world().await;
    let t_id = w.t.id();
    // Off by default: nothing is copied.
    w.s.copy_now().await;
    assert_eq!(sent_rounds(&w.s), 0);
    let v = keep_copy(&w.s, SID, std::slice::from_ref(&t_id)).await;
    assert_eq!(v["explicit"], true, "{v:#}");

    // The first round sends every file whole.
    w.s.copy_now().await;
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);
    let first = last_sent(&w.s);
    assert_eq!(first["more"]["appended"], 0, "{first:#}");
    assert_eq!(first["more"]["whole"], 3, "{first:#}");

    // Turn after turn, only what grew travels — and stays byte for byte.
    for n in 2..=8 {
        turn(&w.s, SID, &w.s_cwd, n);
        let before = std::fs::metadata(&w.transcript).unwrap().len();
        w.s.copy_now().await;
        assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);
        let round = last_sent(&w.s);
        assert_eq!(round["more"]["appended"], 2, "turn {n}: {round:#}");
        assert_eq!(round["more"]["whole"], 0, "turn {n}: {round:#}");
        let sent = round["more"]["sent"].as_u64().unwrap();
        assert!(
            sent < 25_000 && sent < before,
            "turn {n}: {sent} bytes for a {before}-byte transcript"
        );
    }
    // Nothing new: nothing travels.
    let rounds = sent_rounds(&w.s);
    w.s.copy_now().await;
    assert_eq!(
        sent_rounds(&w.s),
        rounds,
        "a quiet session is not copied again"
    );

    // Both sides say so.
    let (_, v) =
        w.s.call("GET", &format!("/sessions/{SID}/copies"), None)
            .await;
    let st = status_on(&v, &t_id);
    assert_eq!(st["state"], "copied", "{v:#}");
    assert!(st["copiedAt"].is_i64(), "{v:#}");
    let (s, held) = w.t.call("GET", "/machines/copies", None).await;
    assert_eq!(s, StatusCode::OK, "{held}");
    let h = &held["held"][0];
    assert_eq!(h["session"], SID, "{held:#}");
    assert_eq!(h["machine"], w.s.id());
    assert_eq!(h["cwd"], w.s_cwd.as_str());
    assert!(h["bytes"].as_u64().unwrap() > 140_000, "{held:#}");
    assert_eq!(h["complete"], true);
    let (_, kept) = w.s.call("GET", "/machines/copies", None).await;
    assert_eq!(kept["kept"][0]["machine"], t_id.as_str(), "{kept:#}");
    assert_eq!(kept["kept"][0]["from"], "session");
    // The audit lines name sizes, never content.
    let received = audited(&w.t, "machine.copy.received");
    assert!(!received.is_empty());
    assert!(!format!("{received:?}").contains("tray"));
    // It lives on the source, and the target knows it.
    assert_eq!(
        lease::lease_of(&w.t.st, SID).map(|l| l.home),
        Some(w.s.id())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hash_mismatch_or_a_rewrite_resends_the_whole_file() {
    let w = world().await;
    let t_id = w.t.id();
    keep_copy(&w.s, SID, std::slice::from_ref(&t_id)).await;
    turn(&w.s, SID, &w.s_cwd, 2);
    turn(&w.s, SID, &w.s_cwd, 3);
    w.s.copy_now().await;
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);

    // The copy there is damaged (same size, a byte near its end): the next
    // append does not fit it, and the file goes again whole.
    let copy = in_copy(&w.t, &w.s, SID, &rel_of(&w.s, &w.transcript));
    let mut bytes = std::fs::read(&copy).unwrap();
    let at = bytes.len() - 100;
    bytes[at] ^= 0x20;
    std::fs::write(&copy, &bytes).unwrap();
    turn(&w.s, SID, &w.s_cwd, 4);
    w.s.copy_now().await;
    let round = last_sent(&w.s);
    assert_eq!(round["more"]["resend"], 1, "{round:#}");
    w.s.copy_now().await;
    let round = last_sent(&w.s);
    assert_eq!(round["more"]["whole"], 1, "{round:#}");
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);

    // Rewritten here (its first bytes, the same size): sent whole.
    let mut src = std::fs::read(&w.transcript).unwrap();
    src[2] = b' ';
    std::fs::write(&w.transcript, &src).unwrap();
    w.s.copy_now().await;
    let round = last_sent(&w.s);
    assert_eq!(round["more"]["whole"], 1, "{round:#}");
    assert_eq!(round["more"]["appended"], 0, "{round:#}");
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);

    // Truncated here: sent whole.
    let src = std::fs::read(&w.transcript).unwrap();
    std::fs::write(&w.transcript, &src[..src.len() / 2]).unwrap();
    w.s.copy_now().await;
    let round = last_sent(&w.s);
    assert_eq!(round["more"]["whole"], 1, "{round:#}");
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);

    // A file the session has no more goes there too (at its next turn).
    std::fs::remove_file(w.s.home().join("todos").join(format!("{SID}.json"))).unwrap();
    turn(&w.s, SID, &w.s_cwd, 5);
    w.s.copy_now().await;
    assert_eq!(last_sent(&w.s)["more"]["removed"], 1);
    assert!(!in_copy(
        &w.t,
        &w.s,
        SID,
        Path::new("todos").join(format!("{SID}.json")).as_path()
    )
    .exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn never_a_mid_turn_copy() {
    let w = world().await;
    let t_id = w.t.id();
    // A terminal runs it here, in the middle of a turn.
    let pid = std::process::id();
    let pidfile = w.s.home().join("sessions").join(format!("{pid}.json"));
    std::fs::create_dir_all(pidfile.parent().unwrap()).unwrap();
    let write_pid = |status: &str| {
        std::fs::write(
            &pidfile,
            json!({"pid": pid, "sessionId": SID, "cwd": w.s_cwd, "status": status,
                   "kind": "interactive", "entrypoint": "cli"})
            .to_string(),
        )
        .unwrap();
    };
    write_pid("busy");
    keep_copy(&w.s, SID, std::slice::from_ref(&t_id)).await;
    for n in 2..=3 {
        turn(&w.s, SID, &w.s_cwd, n);
        w.s.copy_now().await;
        assert_eq!(sent_rounds(&w.s), 0, "nothing while a turn runs");
        assert!(
            !in_copy(&w.t, &w.s, SID, &rel_of(&w.s, &w.transcript)).exists(),
            "no copy there yet"
        );
    }
    let (_, v) =
        w.s.call("GET", &format!("/sessions/{SID}/copies"), None)
            .await;
    assert_eq!(status_on(&v, &t_id)["state"], "waiting", "{v:#}");
    // Even when asked by hand, a round waits for the turn.
    assert_eq!(
        copies::source::round(&w.s.st, SID, &t_id, false).await,
        copies::source::Outcome::Waiting
    );

    // The turn ends: copied, whole.
    write_pid("idle");
    w.s.copy_now().await;
    assert_eq!(sent_rounds(&w.s), 1);
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);
    let (_, v) =
        w.s.call("GET", &format!("/sessions/{SID}/copies"), None)
            .await;
    assert_eq!(status_on(&v, &t_id)["state"], "copied", "{v:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_small_cap_takes_the_oldest_copy_away_never_one_that_lives_here() {
    let w = world().await;
    let t_id = w.t.id();
    session_on(&w.s, SID2, &w.s_cwd).await;
    for n in 2..=3 {
        turn(&w.s, SID, &w.s_cwd, n);
        turn(&w.s, SID2, &w.s_cwd, n);
    }
    // The target keeps little of this computer's copies.
    let size = |sid: &str| -> u64 {
        std::fs::metadata(transcript_in(&w.s.home(), &w.s_cwd, sid))
            .unwrap()
            .len()
    };
    let cap = size(SID) + size(SID2) / 2 + 2_000;
    w.t.st
        .machines
        .store_write(|f| f.settings.copies_cap_bytes = cap);
    keep_copy(&w.s, SID, std::slice::from_ref(&t_id)).await;
    w.s.copy_now().await;
    assert!(in_copy(&w.t, &w.s, SID, &rel_of(&w.s, &w.transcript)).exists());
    keep_copy(&w.s, SID2, std::slice::from_ref(&t_id)).await;
    w.s.copy_now().await;
    // The second one pushed the first (refreshed longest ago) out.
    let (_, held) = w.t.call("GET", "/machines/copies", None).await;
    let sessions: Vec<&str> = held["held"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["session"].as_str().unwrap())
        .collect();
    assert_eq!(sessions, [SID2], "{held:#}");
    assert!(!w
        .t
        .data()
        .join("machines")
        .join("copies")
        .join(w.s.id())
        .join("default")
        .join(SID)
        .exists());
    assert!(!audited(&w.t, "machine.copy.evicted").is_empty());
    let (_, v) =
        w.s.call("GET", &format!("/sessions/{SID}/copies"), None)
            .await;
    assert_eq!(status_on(&v, &t_id)["reason"], "tooLarge", "{v:#}");

    // The first comes back after its next turn (taking the other out), and
    // once its session lives on the target, nothing takes it away.
    turn(&w.s, SID, &w.s_cwd, 4);
    w.s.copy_now().await;
    let (s, v) =
        w.s.call(
            "POST",
            &format!("/sessions/{SID}/move"),
            Some(json!({ "to": t_id, "folder": w.t_cwd, "code": "none" })),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    eventually("the move to end", || async {
        lease::lease_of(&w.t.st, SID).is_some_and(|l| l.home == t_id)
    })
    .await;
    turn(&w.s, SID2, &w.s_cwd, 4);
    w.s.copy_now().await;
    let (_, held) = w.t.call("GET", "/machines/copies", None).await;
    let mut sessions: Vec<&str> = held["held"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["session"].as_str().unwrap())
        .collect();
    sessions.sort();
    assert_eq!(sessions, [SID, SID2], "{held:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_move_of_a_copied_session_sends_almost_nothing_and_lands() {
    let w = world().await;
    let t_id = w.t.id();
    for n in 2..=4 {
        turn(&w.s, SID, &w.s_cwd, n);
    }
    keep_copy(&w.s, SID, std::slice::from_ref(&t_id)).await;
    w.s.copy_now().await;
    // One more turn that the loop has not copied yet: the move brings it.
    turn(&w.s, SID, &w.s_cwd, 5);
    w.s.refresh().await;
    // The preflight says the copy is there.
    let body = json!({ "to": t_id, "folder": w.t_cwd, "code": "none" });
    let (s, pre) =
        w.s.call(
            "POST",
            &format!("/sessions/{SID}/move/preflight"),
            Some(body.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{pre}");
    assert_eq!(pre["ok"], true, "{pre:#}");
    assert_eq!(pre["target"]["copy"]["held"], true, "{pre:#}");

    let (s, v) =
        w.s.call("POST", &format!("/sessions/{SID}/move"), Some(body))
            .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(90);
    let rec = loop {
        let (_, r) = w.s.call("GET", &format!("/moves/{id}"), None).await;
        if r["finished"] == true {
            break r;
        }
        assert!(Instant::now() < deadline, "{r:#}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    assert_eq!(rec["fromCopy"], true, "{rec:#}");
    assert_eq!(
        rec["bytesTotal"], 0,
        "the conversation did not travel: {rec:#}"
    );
    assert!(rec["files"].as_array().unwrap().is_empty(), "{rec:#}");
    // Landed whole there, as it was here (the last turn too).
    let landed = transcript_in(&w.t.home(), &w.t_cwd, SID);
    assert_eq!(
        std::fs::read(&landed).unwrap(),
        std::fs::read(&w.transcript).unwrap()
    );
    let sub = subagent_in(&w.t.home(), &w.t_cwd, SID);
    assert_eq!(
        std::fs::read(sub).unwrap(),
        std::fs::read(subagent_in(&w.s.home(), &w.s_cwd, SID)).unwrap()
    );
    let (_, theirs) = w.t.call("GET", &format!("/moves/{id}"), None).await;
    assert_eq!(theirs["landing"]["action"], "new", "{theirs:#}");
    assert_eq!(theirs["fromCopy"], true);
    assert_eq!(
        lease::lease_of(&w.s.st, SID).map(|l| l.home),
        Some(t_id.clone())
    );
    let resumed = w.t.st.machines.recorded_resumes();
    assert_eq!(resumed.len(), 1, "{resumed:?}");
    assert_eq!(resumed[0]["cwd"], w.t_cwd.as_str());
    // It lives there now: the source keeps no copy of it current any more.
    turn(&w.s, SID, &w.s_cwd, 6);
    let rounds = sent_rounds(&w.s);
    w.s.copy_now().await;
    assert_eq!(sent_rounds(&w.s), rounds);
    let (_, v) =
        w.s.call("GET", &format!("/sessions/{SID}/copies"), None)
            .await;
    assert_eq!(status_on(&v, &t_id)["reason"], "livesElsewhere", "{v:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_its_home_offline_a_session_goes_on_from_the_copy_and_its_own_turns_meanwhile_are_kept(
) {
    let w = world().await;
    let (s_id, t_id) = (w.s.id(), w.t.id());
    for n in 2..=3 {
        turn(&w.s, SID, &w.s_cwd, n);
    }
    keep_copy(&w.s, SID, std::slice::from_ref(&t_id)).await;
    w.s.copy_now().await;
    let copied = std::fs::read(&w.transcript).unwrap();

    // The source goes to sleep. The target keeps its copy, and goes on with
    // it (the take over itself checks that the source does not answer).
    transport::go_offline(&w.s.st).await;
    let (s, held) = w.t.call("GET", "/machines/copies", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(held["held"][0]["session"], SID, "{held:#}");
    assert_eq!(held["held"][0]["machine"], s_id.as_str());
    let (s, v) =
        w.t.call(
            "POST",
            &format!("/sessions/{SID}/move"),
            Some(json!({ "to": t_id, "takeOver": true, "folder": w.t_cwd })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["takenOver"], true);
    assert_eq!(v["fromCopy"], true, "{v:#}");
    assert_eq!(v["resumed"], true, "{v:#}");
    assert_eq!(v["landing"]["action"], "new", "{v:#}");
    assert_eq!(v["epoch"], 2, "{v:#}");
    let landed = transcript_in(&w.t.home(), &w.t_cwd, SID);
    assert_eq!(std::fs::read(&landed).unwrap(), copied);
    assert_eq!(
        lease::lease_of(&w.t.st, SID).map(|l| l.home),
        Some(t_id.clone())
    );
    let resumed = w.t.st.machines.recorded_resumes();
    assert_eq!(resumed.len(), 1, "{resumed:?}");
    assert_eq!(resumed[0]["session"], SID);
    assert_eq!(resumed[0]["fork"], false);
    assert_eq!(resumed[0]["cwd"], w.t_cwd.as_str());
    assert_eq!(
        resumed[0]["note"], true,
        "the note waits for its next message"
    );
    // It goes on here.
    append(
        &landed,
        &format!(
            "{}\n",
            user_line(
                SID,
                &w.t_cwd,
                30,
                "said on the target while the source slept"
            )
        ),
    );

    // Meanwhile the source went on with it too (it did not know).
    append(
        &w.transcript,
        &format!(
            "{}\n",
            user_line(SID, &w.s_cwd, 40, "said on the source meanwhile")
        ),
    );

    // It comes back and learns where the session lives now.
    transport::come_back(&w.s.st);
    transport::ensure_started(&w.s.st).await.unwrap();
    eventually("the source to learn the lease", || async {
        greet(&w.s, &w.t).await;
        lease::lease_of(&w.s.st, SID).is_some_and(|l| l.home == t_id)
    })
    .await;
    // Its copy is not refreshed any more from there.
    let rounds = sent_rounds(&w.s);
    w.s.copy_now().await;
    assert_eq!(sent_rounds(&w.s), rounds);

    // Brought back: the target's version is the session; the source's own
    // turns are kept beside it — nothing lost.
    w.t.refresh().await;
    let (s, v) =
        w.t.call(
            "POST",
            &format!("/sessions/{SID}/move"),
            Some(json!({ "to": s_id, "folder": w.s_cwd, "code": "none" })),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let (_, r) = w.t.call("GET", &format!("/moves/{id}"), None).await;
        if r["finished"] == true {
            assert_eq!(r["state"], "resumed", "{r:#}");
            break;
        }
        assert!(Instant::now() < deadline, "{r:#}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (_, mine) = w.s.call("GET", &format!("/moves/{id}"), None).await;
    assert_eq!(mine["landing"]["action"], "conflict", "{mine:#}");
    let kept = PathBuf::from(mine["landing"]["conflict"].as_str().unwrap());
    let kept = std::fs::read_to_string(kept).unwrap();
    assert!(kept.contains("said on the source meanwhile"), "{kept}");
    let now = std::fs::read_to_string(&w.transcript).unwrap();
    assert!(now.contains("said on the target while the source slept"));
    assert!(!now.contains("said on the source meanwhile"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_move_grant_or_with_machines_off_a_copy_pauses_and_says_why() {
    // The target lets the source see and drive, not move.
    let w = world_with(grants(), no_move()).await;
    let t_id = w.t.id();
    keep_copy(&w.s, SID, std::slice::from_ref(&t_id)).await;
    w.s.copy_now().await;
    let (_, v) =
        w.s.call("GET", &format!("/sessions/{SID}/copies"), None)
            .await;
    let st = status_on(&v, &t_id);
    assert_eq!(st["state"], "paused", "{v:#}");
    assert_eq!(st["reason"], "noGrant", "{v:#}");
    let (_, held) = w.t.call("GET", "/machines/copies", None).await;
    assert!(held["held"].as_array().unwrap().is_empty(), "{held:#}");
    assert_eq!(sent_rounds(&w.s), 0);

    // Given: it goes at the next look after its wait (cleared when the
    // other computer comes back).
    let (s, v) =
        w.t.call(
            "PATCH",
            &format!("/machines/{}", w.s.id()),
            Some(json!({ "grants": grants() })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    greet(&w.s, &w.t).await;
    copies::on_peer_back(&w.s.st, &t_id);
    w.s.copy_now().await;
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);

    // Machines off there: paused, nothing queued; on again: what grew goes.
    machines::set_enabled(&w.t.st, false);
    turn(&w.s, SID, &w.s_cwd, 2);
    w.s.copy_now().await;
    let (_, v) =
        w.s.call("GET", &format!("/sessions/{SID}/copies"), None)
            .await;
    assert_eq!(status_on(&v, &t_id)["reason"], "machinesOff", "{v:#}");
    machines::set_enabled(&w.t.st, true);
    copies::on_peer_back(&w.s.st, &t_id);
    w.s.copy_now().await;
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);
    // Machines off here: nothing even tried.
    machines::set_enabled(&w.s.st, false);
    turn(&w.s, SID, &w.s_cwd, 3);
    let rounds = sent_rounds(&w.s);
    w.s.copy_now().await;
    assert_eq!(sent_rounds(&w.s), rounds);
    let (_, v) =
        w.s.call("GET", &format!("/sessions/{SID}/copies"), None)
            .await;
    assert_eq!(status_on(&v, &t_id)["reason"], "machinesOff", "{v:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_copy_can_be_removed_on_either_side() {
    let w = world().await;
    let (s_id, t_id) = (w.s.id(), w.t.id());
    keep_copy(&w.s, SID, std::slice::from_ref(&t_id)).await;
    w.s.copy_now().await;
    // Held there: removed there.
    let (s, v) =
        w.t.call(
            "DELETE",
            &format!("/machines/copies/held.{s_id}.{SID}"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["held"].as_array().unwrap().is_empty(), "{v:#}");
    assert!(!in_copy(&w.t, &w.s, SID, &rel_of(&w.s, &w.transcript)).exists());
    // It comes back after the next turn, as the source still keeps it…
    turn(&w.s, SID, &w.s_cwd, 2);
    w.s.copy_now().await;
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);
    // …until the source stops: then the copy there goes too.
    let (s, v) =
        w.s.call(
            "DELETE",
            &format!("/machines/copies/kept.{t_id}.{SID}"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["kept"].as_array().unwrap().is_empty(), "{v:#}");
    assert!(!in_copy(&w.t, &w.s, SID, &rel_of(&w.s, &w.transcript)).exists());
    turn(&w.s, SID, &w.s_cwd, 3);
    let rounds = sent_rounds(&w.s);
    w.s.copy_now().await;
    assert_eq!(sent_rounds(&w.s), rounds);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_project_keeps_copies_of_its_sessions() {
    let w = world().await;
    let t_id = w.t.id();
    let (s, v) =
        w.s.call(
            "PUT",
            "/machines/settings",
            Some(json!({ "projects": { "upsert": [{
                "project": w.s_cwd,
                "copies": [t_id]
            }] } })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    w.s.copy_now().await;
    assert_copy_is_source(&w.s, &w.t, SID, &w.s_cwd);
    let (_, v) =
        w.s.call("GET", &format!("/sessions/{SID}/copies"), None)
            .await;
    assert_eq!(v["explicit"], false, "{v:#}");
    let m = v["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == t_id.as_str())
        .unwrap()
        .clone();
    assert_eq!(m["from"], "project", "{v:#}");
    // The project's other choices left out keep its copies.
    let (s, v) =
        w.s.call(
            "PUT",
            "/machines/settings",
            Some(json!({ "projects": { "upsert": [{
                "project": w.s_cwd,
                "code": { "apply": "worktree" }
            }] } })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let p = v["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["copies"].is_array())
        .cloned()
        .unwrap_or_else(|| panic!("{v:#}"));
    assert_eq!(p["copies"], json!([t_id]));
    // The session's own choice wins: none.
    keep_copy(&w.s, SID, &[]).await;
    turn(&w.s, SID, &w.s_cwd, 2);
    let rounds = sent_rounds(&w.s);
    w.s.copy_now().await;
    assert_eq!(sent_rounds(&w.s), rounds);
}
