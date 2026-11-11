//! Machines, Phase 3a (docs/PLAN-MACHINES.md §5.7–§5.10): moving a session
//! with its code between two app states in one process, each with its own
//! data dir, Claude home and iroh endpoint, paired over real QUIC (the
//! `tests/machines.rs` harness). Sessions are ended ones on disk (or one a
//! "foreign" pidfile holds); resumes are recorded, never `claude` started;
//! git runs without this machine's configuration. Nothing touches
//! `~/.claude` or `~/.claude.json`: `CLAUDE_CONFIG_DIR` is set, so each test
//! home's `.claude.json` is its own (a carried trust lands there).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::machines::{self, code::GitRunner, lease, transport};
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
    async fn call_as(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
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
        let req = req.bearer_auth(token);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let v = resp.json::<Value>().await.unwrap_or(Value::Null);
        (status, v)
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let token = self.st.auth.token.clone();
        self.call_as(&token, method, path, body).await
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
}

fn state_of(v: &Value) -> &str {
    v["pairing"]["state"].as_str().unwrap_or("")
}

/// Pairs `a` (host) and `b` (joiner); `b`'s grants to `a` are `b_gives`,
/// `a`'s to `b` are `a_gives`. Then each greets the other, so both know
/// what the other can do (`moves`), and Machines is turned on on both, as
/// *Turn on Machines* does once the setup passed.
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

/// A hello each way (leases and capabilities travel with it).
async fn greet(a: &Machine, b: &Machine) {
    machines::greet(a.st.clone(), b.id(), key_of(a, &b.id())).await;
    machines::greet(b.st.clone(), a.id(), key_of(b, &a.id())).await;
}

/// View, drive and move both ways; no run, no dangerous.
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

/// Calls `f` until it returns `true` (45 s at most).
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

// ─── sessions and repositories ─────────────────────────────────────────────────

const SID: &str = "a0ee0000-1111-4222-8333-444455556666";

fn canon(p: &Path) -> String {
    dunce::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn user_line(cwd: &str, n: u32, text: &str) -> String {
    json!({"type": "user", "uuid": format!("u-{n}"), "sessionId": SID, "cwd": cwd,
           "timestamp": "2026-11-11T09:00:00.000Z", "permissionMode": "default",
           "message": {"role": "user", "content": text}})
    .to_string()
}

fn assistant_line(cwd: &str, n: u32) -> String {
    json!({"type": "assistant", "uuid": format!("a-{n}"), "parentUuid": format!("u-{n}"),
           "sessionId": SID, "cwd": cwd, "timestamp": "2026-11-11T09:00:05.000Z",
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

/// A finished session of two turns in `cwd`, on `m`'s disk.
async fn session_on(m: &Machine, cwd: &str) -> PathBuf {
    let t = transcript_in(&m.home(), cwd, SID);
    std::fs::create_dir_all(t.parent().unwrap()).unwrap();
    let text = [
        user_line(cwd, 1, "make the tray icon work"),
        assistant_line(cwd, 1),
    ]
    .iter()
    .map(|l| format!("{l}\n"))
    .collect::<String>();
    std::fs::write(&t, text).unwrap();
    m.refresh().await;
    t
}

/// The session went on: one more turn appended (in `cwd`).
fn went_on(transcript: &Path, cwd: &str, n: u32, text: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(transcript)
        .unwrap();
    writeln!(f, "{}", user_line(cwd, n, text)).unwrap();
    writeln!(f, "{}", assistant_line(cwd, n)).unwrap();
}

/// Claude Code on `m` has worked in `dir` before (its trust): a transcript
/// of another session recorded there.
fn trusted_on(m: &Machine, dir: &str) {
    let other = "b0ee0000-1111-4222-8333-444455556666";
    let t = transcript_in(&m.home(), dir, other);
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

fn git(cwd: &Path, args: &[&str]) -> String {
    let op = GitRunner::new().isolated().begin().unwrap();
    op.git(cwd, args)
        .unwrap_or_else(|e| panic!("git {} in {}: {e}", args.join(" "), cwd.display()))
}

fn put(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// An origin with one commit, and a clone of it on each machine's side.
struct Repos {
    _tmp: TempDir,
    /// The source's checkout.
    s: PathBuf,
    /// The target's checkout.
    t: PathBuf,
}

fn repos() -> Repos {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap();
    let origin = root.join("origin.git");
    git(&root, &["init", "--bare", "-q", origin.to_str().unwrap()]);
    let seed = root.join("seed");
    git(
        &root,
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            seed.to_str().unwrap(),
        ],
    );
    put(&seed, "a.txt", "alpha\n");
    put(&seed, "b.txt", "bravo\n");
    put(&seed, "src/main.rs", "fn main() {}\n");
    put(&seed, ".gitignore", "target/\n.env.local\n*.local.json\n");
    git(&seed, &["add", "-A"]);
    git(&seed, &["commit", "-q", "-m", "seed"]);
    git(&seed, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
    let s = root.join("s-app");
    let t = root.join("t-app");
    for to in [&s, &t] {
        git(
            &root,
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                to.to_str().unwrap(),
            ],
        );
    }
    Repos { _tmp: tmp, s, t }
}

/// The source's work in progress: staged, unstaged and untracked.
fn dirty(s: &Path) {
    put(s, "a.txt", "alpha, staged\n");
    git(s, &["add", "a.txt"]);
    put(s, "b.txt", "bravo, not staged\n");
    put(s, "notes/new.md", "- untracked\n");
}

fn staged(dir: &Path) -> String {
    git(dir, &["diff", "--cached", "--name-only"])
}

// ─── moving ────────────────────────────────────────────────────────────────────

/// Starts a move on `m` and waits until it ended (its record says so).
async fn move_session(m: &Machine, body: Value) -> Value {
    let (s, v) = m
        .call("POST", &format!("/sessions/{SID}/move"), Some(body))
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id = v["id"].as_str().expect("a move id").to_string();
    wait_move(m, &id, |r| {
        r["finished"] == true || (r["state"] == "committed" && !r["resumeError"].is_null())
    })
    .await
}

async fn wait_move(m: &Machine, id: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let (s, v) = m.call("GET", &format!("/moves/{id}"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        if pred(&v) {
            return v;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the move: {v:#}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn preflight(m: &Machine, body: Value) -> Value {
    let (s, v) = m
        .call(
            "POST",
            &format!("/sessions/{SID}/move/preflight"),
            Some(body),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

fn codes(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|l| {
            l.iter()
                .filter_map(|c| c["code"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn lease_home(m: &Machine) -> Option<String> {
    lease::lease_of(&m.st, SID)
        .filter(|l| l.epoch > 0)
        .map(|l| l.home)
}

/// Two paired machines and the repositories; the session works in the
/// source's checkout, and the target has worked in its own before.
struct World {
    s: Machine,
    t: Machine,
    repos: Repos,
    s_cwd: String,
    t_cwd: String,
    transcript: PathBuf,
}

async fn world() -> World {
    let lookup = MemoryLookup::new();
    let s = machine(&lookup).await;
    let t = machine(&lookup).await;
    pair(&s, &t, grants(), grants()).await;
    let repos = repos();
    let s_cwd = canon(&repos.s);
    let t_cwd = canon(&repos.t);
    trusted_on(&t, &t_cwd);
    let transcript = session_on(&s, &s_cwd).await;
    World {
        s,
        t,
        repos,
        s_cwd,
        t_cwd,
        transcript,
    }
}

fn to_t(w: &World, extra: Value) -> Value {
    let mut body =
        json!({ "to": w.t.id(), "folder": w.t_cwd, "code": "bundle", "apply": "checkout" });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    body
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_moves_with_its_code_and_comes_back_as_a_fast_forward() {
    let w = world().await;
    dirty(&w.repos.s);

    // The sheet's checks: nothing in the way, nothing written.
    let pre = preflight(&w.s, to_t(&w, json!({}))).await;
    assert_eq!(pre["ok"], true, "{pre:#}");
    assert_eq!(pre["code"]["changed"], 3, "{pre:#}");
    assert_eq!(pre["target"]["folder"]["how"], "chosen");
    assert_eq!(pre["target"]["code"]["clean"], true);
    assert!(lease_home(&w.s).is_none());

    let rec = move_session(&w.s, to_t(&w, json!({ "recipe": true }))).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    let steps: Vec<&str> = rec["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["state"].as_str().unwrap())
        .collect();
    assert_eq!(
        steps,
        [
            "preflight",
            "freezing",
            "packaging",
            "sending",
            "landing",
            "landed",
            "committed",
            "resumed"
        ]
    );
    assert_eq!(rec["bytesSent"], rec["bytesTotal"]);

    // The code is on the target as it was here: staged, unstaged, untracked.
    for f in ["a.txt", "b.txt", "notes/new.md"] {
        assert_eq!(read(&w.repos.t, f), read(&w.repos.s, f), "{f}");
    }
    assert_eq!(staged(&w.repos.t), "a.txt");
    // The conversation too, under the folder it works in there.
    let landed = transcript_in(&w.t.home(), &w.t_cwd, SID);
    assert_eq!(
        std::fs::read(&landed).unwrap(),
        std::fs::read(&w.transcript).unwrap()
    );
    // It lives there now, on both machines; the source keeps its copy.
    assert_eq!(lease_home(&w.s).as_deref(), Some(w.t.id().as_str()));
    assert_eq!(lease_home(&w.t).as_deref(), Some(w.t.id().as_str()));
    assert!(w.transcript.exists());
    // Resumed there in its folder, the note waiting for its next message.
    let resumed = w.t.st.machines.recorded_resumes();
    assert_eq!(resumed.len(), 1, "{resumed:?}");
    assert_eq!(resumed[0]["session"], SID);
    assert_eq!(resumed[0]["fork"], false);
    assert_eq!(resumed[0]["cwd"], w.t_cwd.as_str());
    assert_eq!(resumed[0]["note"], true);
    let note = lease::pending_note(&w.t.st, SID).expect("a note");
    assert!(note.contains(&w.s_cwd) && note.contains(&w.t_cwd), "{note}");
    // The target's record says the same.
    let (s, list) = w.t.call("GET", "/moves", None).await;
    assert_eq!(s, StatusCode::OK);
    let theirs = &list["moves"][0];
    assert_eq!(theirs["role"], "in");
    assert_eq!(theirs["state"], "resumed");
    assert_eq!(theirs["landing"]["action"], "new");

    // It went on there; then it comes back.
    went_on(&landed, &w.t_cwd, 2, "and on Windows too");
    put(&w.repos.t, "c.txt", "made there\n");
    w.t.refresh().await;
    let (s, v) =
        w.t.call(
            "POST",
            &format!("/sessions/{SID}/move"),
            Some(json!({ "to": w.s.id(), "code": "bundle" })),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let back = wait_move(&w.t, v["id"].as_str().unwrap(), |r| r["finished"] == true).await;
    assert_eq!(back["state"], "resumed", "{back:#}");
    // Where it came from, found from the first move.
    assert_eq!(back["targetCwd"], w.s_cwd.as_str());
    let (_, mine) =
        w.s.call(
            "GET",
            &format!("/moves/{}", back["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(mine["landing"]["action"], "fastForward", "{mine:#}");
    assert_eq!(
        std::fs::read(&w.transcript).unwrap(),
        std::fs::read(&landed).unwrap(),
        "a fast-forward: what happened there is added here"
    );
    assert_eq!(read(&w.repos.s, "c.txt"), "made there\n");
    assert_eq!(read(&w.repos.s, "b.txt"), "bravo, not staged\n");
    assert_eq!(lease_home(&w.s).as_deref(), Some(w.s.id().as_str()));
    assert_eq!(lease_home(&w.t).as_deref(), Some(w.s.id().as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lease_keeps_a_second_writer_away() {
    let w = world().await;
    let rec = move_session(&w.s, to_t(&w, json!({ "code": "none" }))).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    w.s.refresh().await;

    // Here it is a copy: continuing it in place, or messaging it, is refused.
    let (s, v) =
        w.s.call(
            "POST",
            &format!("/sessions/{SID}/continue"),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "session_lives_elsewhere");
    assert_eq!(v["error"]["machine"], w.t.id().as_str());
    let (s, v) =
        w.s.call(
            "POST",
            &format!("/sessions/{SID}/message"),
            Some(json!({ "text": "hello?" })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "session_lives_elsewhere");
    // The list says where it lives.
    let (_, one) = w.s.call("GET", &format!("/sessions/{SID}"), None).await;
    let lives = &one["session"]["livesOn"];
    let lives = if lives.is_null() {
        &one["livesOn"]
    } else {
        lives
    };
    assert_eq!(lives["machine"], w.t.id().as_str(), "{one:#}");
    // And a move must start where it lives.
    let (s, v) =
        w.s.call(
            "POST",
            &format!("/sessions/{SID}/move/preflight"),
            Some(json!({ "to": w.t.id() })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "session_lives_elsewhere");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failure_at_any_step_before_landing_leaves_both_machines_as_they_were() {
    let w = world().await;
    dirty(&w.repos.s);
    let t_head = git(&w.repos.t, &["rev-parse", "HEAD"]);
    let before = std::fs::read(&w.transcript).unwrap();
    for (who, step) in [
        ("s", "freeze"),
        ("s", "package"),
        ("s", "send"),
        ("t", "land"),
    ] {
        let m = if who == "s" { &w.s } else { &w.t };
        m.st.machines.fail_move_at(Some(step));
        let rec = move_session(&w.s, to_t(&w, json!({}))).await;
        assert_eq!(rec["state"], "failed", "{step}: {rec:#}");
        assert_eq!(rec["error"]["code"], "injected", "{step}: {rec:#}");
        // The source: as it was, and its session still lives here.
        assert!(lease_home(&w.s).is_none(), "{step}");
        assert_eq!(std::fs::read(&w.transcript).unwrap(), before, "{step}");
        assert_eq!(read(&w.repos.s, "b.txt"), "bravo, not staged\n", "{step}");
        // The target: nothing of it.
        assert!(
            !transcript_in(&w.t.home(), &w.t_cwd, SID).exists(),
            "{step}: no conversation there"
        );
        assert_eq!(git(&w.repos.t, &["rev-parse", "HEAD"]), t_head, "{step}");
        assert_eq!(
            git(&w.repos.t, &["status", "--porcelain"]),
            "",
            "{step}: its checkout untouched"
        );
        assert!(lease_home(&w.t).is_none(), "{step}");
        assert!(lease::pending_note(&w.t.st, SID).is_none(), "{step}");
        let id = rec["id"].as_str().unwrap();
        eventually("the target to drop it", || async {
            let (s, v) = w.t.call("GET", &format!("/moves/{id}"), None).await;
            s == StatusCode::NOT_FOUND || v["state"] == "failed"
        })
        .await;
        assert!(
            !w.t.data().join("machines").join("moves").join(id).exists(),
            "{step}: what arrived is gone"
        );
    }
    // Consistent: the next move goes through.
    let rec = move_session(&w.s, to_t(&w, json!({}))).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    assert_eq!(read(&w.repos.t, "b.txt"), "bravo, not staged\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_landing_a_lost_commit_or_resume_finishes_later() {
    let w = world().await;
    // The lease does not reach the target: it is placed there, not running.
    w.s.st.machines.fail_move_at(Some("commit"));
    let rec = move_session(&w.s, to_t(&w, json!({ "code": "none" }))).await;
    assert_eq!(rec["state"], "committed", "{rec:#}");
    assert!(rec["resumeError"].is_string());
    assert_eq!(lease_home(&w.s).as_deref(), Some(w.t.id().as_str()));
    assert!(w.t.st.machines.recorded_resumes().is_empty());
    // The next hello carries the lease: it starts there, and the source learns.
    let id = rec["id"].as_str().unwrap().to_string();
    eventually("it to start there after a hello", || async {
        greet(&w.s, &w.t).await;
        let (_, v) = w.s.call("GET", &format!("/moves/{id}"), None).await;
        v["state"] == "resumed"
    })
    .await;
    assert_eq!(w.t.st.machines.recorded_resumes().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn placed_but_not_started_is_tried_again_without_sending_twice() {
    let w = world().await;
    w.t.st.machines.fail_move_at(Some("resume"));
    let rec = move_session(&w.s, to_t(&w, json!({ "code": "none" }))).await;
    assert_eq!(rec["state"], "committed", "{rec:#}");
    assert!(rec["resumeError"].as_str().unwrap_or("").contains("resume"));
    let id = rec["id"].as_str().unwrap();
    let (s, v) = w.s.call("POST", &format!("/moves/{id}/retry"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "resumed", "{v:#}");
    assert_eq!(w.t.st.machines.recorded_resumes().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_that_went_on_on_both_keeps_both_as_a_conflict_copy() {
    let w = world().await;
    let rec = move_session(&w.s, to_t(&w, json!({ "code": "none" }))).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    let there = transcript_in(&w.t.home(), &w.t_cwd, SID);
    went_on(&there, &w.t_cwd, 2, "said there while it was away");

    // The target goes away; this machine takes over from its own copy.
    transport::go_offline(&w.t.st).await;
    let (s, v) =
        w.s.call(
            "POST",
            &format!("/sessions/{SID}/move"),
            Some(json!({ "to": w.s.id(), "takeOver": true })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["takenOver"], true);
    assert_eq!(v["epoch"], 2);
    assert_eq!(lease_home(&w.s).as_deref(), Some(w.s.id().as_str()));
    went_on(&w.transcript, &w.s_cwd, 3, "said here meanwhile");

    // It is back, learns the lease, and the session moves there again.
    transport::come_back(&w.t.st);
    transport::ensure_started(&w.t.st).await.unwrap();
    eventually("the target to learn the lease", || async {
        greet(&w.s, &w.t).await;
        lease_home(&w.t).as_deref() == Some(w.s.id().as_str())
    })
    .await;
    w.s.refresh().await;
    let rec = move_session(&w.s, to_t(&w, json!({ "code": "none" }))).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    assert_eq!(rec["landing"]["action"], "conflict", "{rec:#}");
    // The session there is this machine's version; its own turns are beside it.
    assert_eq!(
        std::fs::read(&there).unwrap(),
        std::fs::read(&w.transcript).unwrap()
    );
    let copy = PathBuf::from(rec["landing"]["conflict"].as_str().unwrap());
    let kept = std::fs::read_to_string(&copy).unwrap();
    assert!(kept.contains("said there while it was away"), "{kept}");
    assert!(!kept.contains("said here meanwhile"));
    assert!(copy
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with(&format!("{SID}.conflict-")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_travels_through_the_remote_into_a_worktree_and_comes_home_to_its_branch() {
    let w = world().await;
    dirty(&w.repos.s);
    // The target's checkout has work of its own: caught before anything moves.
    put(&w.repos.t, "mine.txt", "the target's own\n");
    let pre = preflight(&w.s, to_t(&w, json!({ "code": "remote" }))).await;
    assert_eq!(pre["ok"], false, "{pre:#}");
    let dirty_check = pre["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["code"] == "checkout_dirty")
        .unwrap_or_else(|| panic!("{pre:#}"));
    assert_eq!(dirty_check["offer"], "worktree");
    let (s, v) =
        w.s.call(
            "POST",
            &format!("/sessions/{SID}/move"),
            Some(to_t(&w, json!({ "code": "remote" }))),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "move_blocked");
    let (_, list) = w.s.call("GET", "/moves", None).await;
    assert_eq!(
        list["moves"].as_array().unwrap().len(),
        0,
        "nothing started"
    );

    // In a separate folder (a worktree) instead, through the remote.
    let rec = move_session(
        &w.s,
        to_t(&w, json!({ "code": "remote", "apply": "worktree" })),
    )
    .await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    let wt = PathBuf::from(rec["targetCwd"].as_str().unwrap());
    assert!(
        wt.starts_with(w.t.data().join("machines").join("worktrees")),
        "{wt:?}"
    );
    for f in ["a.txt", "b.txt", "notes/new.md"] {
        assert_eq!(read(&wt, f), read(&w.repos.s, f), "{f}");
    }
    // The target's checkout was not touched.
    assert_eq!(read(&w.repos.t, "mine.txt"), "the target's own\n");
    assert_eq!(read(&w.repos.t, "a.txt"), "alpha\n");
    // The note says so.
    let note = lease::pending_note(&w.t.st, SID).unwrap();
    assert!(note.contains("worktree"), "{note}");

    // Home again: to its folder here and its own branch.
    put(&wt, "made-in-worktree.txt", "from the worktree\n");
    w.t.refresh().await;
    let (s, v) =
        w.t.call(
            "POST",
            &format!("/sessions/{SID}/move"),
            Some(json!({ "to": w.s.id(), "code": "bundle" })),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let back = wait_move(&w.t, v["id"].as_str().unwrap(), |r| r["finished"] == true).await;
    assert_eq!(back["state"], "resumed", "{back:#}");
    assert_eq!(back["targetCwd"], w.s_cwd.as_str());
    assert_eq!(
        read(&w.repos.s, "made-in-worktree.txt"),
        "from the worktree\n"
    );
    assert_eq!(
        git(&w.repos.s, &["symbolic-ref", "HEAD"]),
        "refs/heads/main",
        "home to its branch"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_another_process_runs_moves_only_as_a_copy() {
    let w = world().await;
    // A terminal runs it here (this very process's pid is alive).
    let pid = std::process::id();
    std::fs::create_dir_all(w.s.home().join("sessions")).unwrap();
    std::fs::write(
        w.s.home().join("sessions").join(format!("{pid}.json")),
        json!({"pid": pid, "sessionId": SID, "cwd": w.s_cwd, "status": "idle",
               "kind": "interactive", "entrypoint": "cli"})
        .to_string(),
    )
    .unwrap();
    w.s.refresh().await;

    let pre = preflight(&w.s, to_t(&w, json!({ "code": "none" }))).await;
    assert_eq!(pre["ok"], false);
    let c = codes(&pre["blockers"]);
    assert!(c.contains(&"session_foreign".to_string()), "{pre:#}");

    let rec = move_session(&w.s, to_t(&w, json!({ "code": "none", "asCopy": true }))).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    assert_eq!(rec["asCopy"], true);
    let steps: Vec<&str> = rec["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["state"].as_str().unwrap())
        .collect();
    assert!(
        !steps.contains(&"committed"),
        "a copy moves no lease: {steps:?}"
    );
    // Forked there under a new id; the original still lives here.
    let resumed = w.t.st.machines.recorded_resumes();
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0]["fork"], true);
    assert_ne!(resumed[0]["id"], SID);
    assert_eq!(lease_home(&w.s).as_deref(), Some(w.s.id().as_str()));
    assert_eq!(lease_home(&w.t).as_deref(), Some(w.s.id().as_str()));
    // So the target never resumes the original in place.
    w.t.refresh().await;
    let (s, v) =
        w.t.call(
            "POST",
            &format!("/sessions/{SID}/continue"),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "session_lives_elsewhere");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_blocker_stops_a_move_before_anything_is_frozen() {
    let lookup = MemoryLookup::new();
    let s = machine(&lookup).await;
    let t = machine(&lookup).await;
    pair(&s, &t, grants(), grants()).await;
    // A folder the target does not have, and an account it does not have.
    let scratch = tempfile::tempdir().unwrap();
    let gone = canon(scratch.path()).to_string() + "-gone";
    let work_dir = scratch.path().join("claude-work");
    s.st.accounts
        .add_directory("Work", &work_dir, None)
        .unwrap();
    let t_path = transcript_in(&work_dir, &gone, SID);
    std::fs::create_dir_all(t_path.parent().unwrap()).unwrap();
    std::fs::write(
        &t_path,
        format!(
            "{}\n{}\n",
            user_line(&gone, 1, "hi"),
            assistant_line(&gone, 1)
        ),
    )
    .unwrap();
    s.st.index.discover_homes(&s.st.accounts.homes());
    s.refresh().await;

    let body = json!({ "to": t.id(), "code": "none" });
    let pre = preflight(&s, body.clone()).await;
    assert_eq!(pre["ok"], false, "{pre:#}");
    let c = codes(&pre["blockers"]);
    assert!(c.contains(&"account_missing".to_string()), "{pre:#}");
    assert!(c.contains(&"folder_elsewhere".to_string()), "{pre:#}");
    let (st, v) = s
        .call("POST", &format!("/sessions/{SID}/move"), Some(body))
        .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "move_blocked");
    assert!(v["error"]["preflight"]["blockers"].is_array());
    // Nothing started, nothing changed, on either machine.
    for m in [&s, &t] {
        let (_, list) = m.call("GET", "/moves", None).await;
        assert_eq!(list["moves"].as_array().unwrap().len(), 0);
        assert!(lease_home(m).is_none());
    }
    assert!(t_path.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_move_permission_a_move_is_refused_either_way() {
    let lookup = MemoryLookup::new();
    let s = machine(&lookup).await;
    let t = machine(&lookup).await;
    // S lets T see and drive, not move; T lets S do the same.
    pair(&s, &t, no_move(), no_move()).await;
    let repos = repos();
    let s_cwd = canon(&repos.s);
    session_on(&s, &s_cwd).await;

    // S → T: T refuses S.
    let pre = preflight(&s, json!({ "to": t.id(), "code": "none" })).await;
    assert_eq!(pre["ok"], false);
    assert!(
        codes(&pre["blockers"]).contains(&"grant_missing".to_string()),
        "{pre:#}"
    );
    // T asking S (through the proxy) to move S's session: S refuses T.
    let desktop = t.st.auth.token.clone();
    let (st, v) = t
        .call_as(
            &desktop,
            "POST",
            &format!("/machines/{}/proxy/sessions/{SID}/move", s.id()),
            Some(json!({ "to": t.id() })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "grant_missing");
    // A peer's lease is not taken without the grant either.
    let (st, v) = transport::request(
        &t.st,
        &s.id(),
        "POST",
        "/peer/leases",
        Some(json!({ "leases": [{ "session": SID, "home": t.id(), "epoch": 9 }] })),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(st, 403, "{v}");
    assert!(lease_home(&s).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_other_computers_half_of_a_move_reaches_this_dashboard() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let w = world().await;
    let t_id = w.t.id();
    let ws_url = format!("{}/ws", w.s.url.replace("http://", "ws://"));
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    ws.send(Message::Text(
        json!({"t": "auth", "token": w.s.st.auth.token})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let mut next_where = async |what: &str, pred: &dyn Fn(&Value) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let Ok(Some(Ok(Message::Text(text)))) =
                tokio::time::timeout(Duration::from_secs(5), ws.next()).await
            else {
                continue;
            };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if pred(&v) {
                return;
            }
        }
        panic!("timed out waiting for {what}");
    };
    // The source's link to the target is up (its session list arrived)…
    next_where("the link to the target", &|v| {
        v["t"] == "remote" && v["machine"] == t_id.as_str() && v["frame"]["t"] == "sessions"
    })
    .await;
    // …so the target's own record of the move arrives here as it goes.
    let (s, v) =
        w.s.call(
            "POST",
            &format!("/sessions/{SID}/move"),
            Some(to_t(&w, json!({ "code": "none" }))),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let id = v["id"].as_str().expect("a move id").to_string();
    next_where("the target's half of the move", &|v| {
        v["t"] == "remote"
            && v["machine"] == t_id.as_str()
            && v["frame"]["t"] == "move"
            && v["frame"]["move"]["id"] == id.as_str()
            && v["frame"]["move"]["role"] == "in"
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_root_maps_the_folder_there_without_asking() {
    let w = world().await;
    let (s_id, t_id) = (w.s.id(), w.t.id());
    // One root: the source's checkout here is the target's checkout there.
    let (s, v) = w
        .s
        .call(
            "PUT",
            "/roots",
            Some(json!({ "upsert": [{ "name": "app", "paths": { &s_id: w.s_cwd, &t_id: w.t_cwd } }] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    eventually("the root on the target", || async {
        machines::roots::map_here(&w.t.st, &s_id, &w.s_cwd).as_deref() == Some(w.t_cwd.as_str())
    })
    .await;

    let body = json!({ "to": t_id, "code": "none" });
    let pre = preflight(&w.s, body.clone()).await;
    assert_eq!(pre["ok"], true, "{pre:#}");
    assert_eq!(pre["target"]["folder"]["how"], "root", "{pre:#}");
    assert_eq!(pre["target"]["folder"]["path"], w.t_cwd.as_str());
    let rec = move_session(&w.s, body).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    assert_eq!(rec["targetCwd"], w.t_cwd.as_str());
    assert!(transcript_in(&w.t.home(), &w.t_cwd, SID).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_move_follows_the_options_set_for_its_project() {
    let w = world().await;
    let body = json!({ "to": w.t.id(), "folder": w.t_cwd });
    // Nothing said: machine to machine, into the checkout, no recipe.
    let pre = preflight(&w.s, body.clone()).await;
    assert_eq!(pre["code"]["mode"], "bundle", "{pre:#}");
    assert_eq!(pre["code"]["apply"], "checkout", "{pre:#}");
    assert_eq!(pre["recipe"], json!([]), "{pre:#}");

    // This project lands in a worktree, and Claude is asked to install first.
    let (s, v) =
        w.s.call(
            "PUT",
            "/machines/settings",
            Some(json!({ "projects": { "upsert": [{
                "project": w.s_cwd,
                "code": { "apply": "worktree" },
                "recipe": { "afterMove": {
                    "macos": ["npm ci"], "windows": ["npm ci"], "linux": ["npm ci"]
                } }
            }] } })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let pre = preflight(&w.s, body).await;
    assert_eq!(pre["code"]["apply"], "worktree", "{pre:#}");
    assert_eq!(pre["recipe"], json!(["npm ci"]), "{pre:#}");

    // What a move says itself still wins.
    let pre = preflight(
        &w.s,
        json!({ "to": w.t.id(), "folder": w.t_cwd, "apply": "checkout" }),
    )
    .await;
    assert_eq!(pre["code"]["apply"], "checkout", "{pre:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn machines_off_on_either_computer_stops_a_move_before_it_starts() {
    let w = world().await;
    let body = to_t(&w, json!({ "code": "none" }));
    let move_path = format!("/sessions/{SID}/move");
    let preflight_path = format!("/sessions/{SID}/move/preflight");

    // Off here: neither the checks nor the move start.
    machines::set_enabled(&w.s.st, false);
    for path in [&preflight_path, &move_path] {
        let (s, v) = w.s.call("POST", path, Some(body.clone())).await;
        assert_eq!(s, StatusCode::CONFLICT, "{path}: {v}");
        assert_eq!(v["error"]["code"], "machines_off");
    }
    // The other computer can't start it through its proxy either.
    let (s, v) =
        w.t.call(
            "POST",
            &format!("/machines/{}/proxy/sessions/{SID}/move", w.s.id()),
            Some(json!({ "to": w.t.id(), "code": "none" })),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "machines_off");

    // Off there: the target refuses, which the checks say.
    machines::set_enabled(&w.s.st, true);
    machines::set_enabled(&w.t.st, false);
    let pre = preflight(&w.s, body.clone()).await;
    assert_eq!(pre["ok"], false, "{pre:#}");
    let off = pre["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["code"] == "machines_off")
        .unwrap_or_else(|| panic!("{pre:#}"));
    assert_eq!(
        off["message"],
        format!("Machines is off on {}.", machines::this_name(&w.t.st))
    );
    let (s, v) = w.s.call("POST", &move_path, Some(body.clone())).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "move_blocked");
    let (_, list) = w.s.call("GET", "/moves", None).await;
    assert_eq!(
        list["moves"].as_array().unwrap().len(),
        0,
        "nothing started"
    );
    assert!(lease_home(&w.s).is_none());

    // On on both: it moves.
    machines::set_enabled(&w.t.st, true);
    let rec = move_session(&w.s, body).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
}

/// [`grants`] and the dangerous right on the local network.
fn risky() -> Value {
    let mut g = grants();
    g["dangerous"] = json!({"lan": true, "internet": false});
    g
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_move_started_from_the_other_computer_needs_the_dangerous_grant_on_both_hops() {
    let lookup = MemoryLookup::new();
    let s = machine(&lookup).await;
    let t = machine(&lookup).await;
    pair(&s, &t, risky(), risky()).await;
    let repos = repos();
    let s_cwd = canon(&repos.s);
    let t_cwd = canon(&repos.t);
    // The source trusts its folder (its own test home's `.claude.json`);
    // the target has never opened its: the trust must be carried.
    std::fs::write(
        s.home().join(".claude.json"),
        json!({ "projects": { &s_cwd: { "hasTrustDialogAccepted": true } } }).to_string(),
    )
    .unwrap();
    session_on(&s, &s_cwd).await;
    let (s_id, t_id) = (s.id(), t.id());
    let body = json!({ "to": t_id, "folder": t_cwd, "code": "none", "trust": true });
    // T's desktop asks S, through S's proxy here (the second hop is S
    // asking T).
    let via_proxy = |what: &str| format!("/machines/{s_id}/proxy/sessions/{SID}/{what}");
    let refused_for_trust = |pre: &Value| {
        pre["ok"] == false && codes(&pre["blockers"]).contains(&"trust_needs_dangerous".to_string())
    };

    // S has not given T the dangerous right: S refuses.
    let mut no_danger = risky();
    no_danger["dangerous"] = json!({"lan": false, "internet": false});
    let (st, v) = s
        .call(
            "PATCH",
            &format!("/machines/{t_id}"),
            Some(json!({ "grants": no_danger })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, pre) = t
        .call("POST", &via_proxy("move/preflight"), Some(body.clone()))
        .await;
    assert_eq!(st, StatusCode::OK, "{pre}");
    assert!(refused_for_trust(&pre), "{pre:#}");
    let (st, v) = t.call("POST", &via_proxy("move"), Some(body.clone())).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "move_blocked");

    // S gives it; T has not given S the dangerous right: T refuses the
    // second hop, though the person acting is T's own desktop.
    let (st, v) = s
        .call(
            "PATCH",
            &format!("/machines/{t_id}"),
            Some(json!({ "grants": risky() })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, v) = t
        .call(
            "PATCH",
            &format!("/machines/{s_id}"),
            Some(json!({ "grants": no_danger })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, pre) = t
        .call("POST", &via_proxy("move/preflight"), Some(body.clone()))
        .await;
    assert_eq!(st, StatusCode::OK, "{pre}");
    assert!(refused_for_trust(&pre), "{pre:#}");
    let msg = pre["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["code"] == "trust_needs_dangerous")
        .unwrap()["message"]
        .clone();
    assert_eq!(
        msg,
        format!(
            "Carrying the folder's trust needs {}'s Dangerous permission for this computer.",
            machines::this_name(&t.st)
        ),
        "the target's refusal"
    );
    let (_, list) = s.call("GET", "/moves", None).await;
    assert_eq!(
        list["moves"].as_array().unwrap().len(),
        0,
        "nothing started"
    );

    // Both ways: it moves, and the trust is carried there.
    let (st, v) = t
        .call(
            "PATCH",
            &format!("/machines/{s_id}"),
            Some(json!({ "grants": risky() })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, pre) = t
        .call("POST", &via_proxy("move/preflight"), Some(body.clone()))
        .await;
    assert_eq!(st, StatusCode::OK, "{pre}");
    assert_eq!(pre["ok"], true, "{pre:#}");
    let (st, v) = t.call("POST", &via_proxy("move"), Some(body)).await;
    assert_eq!(st, StatusCode::ACCEPTED, "{v}");
    assert_eq!(
        v["by"],
        format!("machine {}", machines::peer_name(&s.st, &t_id))
    );
    let id = v["id"].as_str().unwrap().to_string();
    let rec = wait_move(&s, &id, |r| r["finished"] == true).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    assert_eq!(rec["landing"]["trusted"], true, "{rec:#}");
    assert_eq!(lease_home(&s).as_deref(), Some(t_id.as_str()));
    // Its own `.claude.json` (on Windows Claude Code keys `C:/…`).
    let there: Value =
        serde_json::from_str(&std::fs::read_to_string(t.home().join(".claude.json")).unwrap())
            .unwrap();
    let slashed = |p: &str| p.replace('\\', "/");
    let trusted = there["projects"].as_object().is_some_and(|p| {
        p.iter()
            .any(|(k, e)| slashed(k) == slashed(&t_cwd) && e["hasTrustDialogAccepted"] == true)
    });
    assert!(trusted, "{there:#}");
}

// ─── files git ignores that a project carries ──────────────────────────────────

/// S's project carries `files` although git ignores them (set from S's
/// desktop: setting them is a dangerous act).
async fn carry(w: &World, files: &[&str]) {
    let (s, v) =
        w.s.call(
            "PUT",
            "/machines/settings",
            Some(json!({ "projects": { "upsert": [{
                "project": w.s_cwd,
                "recipe": { "carryIgnored": files }
            }] } })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

/// The target gives the source its dangerous grant too.
async fn t_trusts_s_with_danger(w: &World) {
    let (s, v) =
        w.t.call(
            "PATCH",
            &format!("/machines/{}", w.s.id()),
            Some(json!({ "grants": risky() })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_files_git_ignores_a_project_lists_land_in_the_checkout_and_undo_puts_back_its_own() {
    let w = world().await;
    put(&w.repos.s, ".env.local", "FROM=source\n");
    put(&w.repos.s, "config/dev.local.json", "{}\n");
    // The target has its own .env.local (git ignores it there too: its
    // checkout is clean), and no config folder.
    put(&w.repos.t, ".env.local", "FROM=target\n");
    assert_eq!(git(&w.repos.t, &["status", "--porcelain"]), "");
    carry(
        &w,
        &[".env.local", "config/dev.local.json", "not-there.env"],
    )
    .await;
    let body = to_t(&w, json!({}));

    // The sheet lists them (the one not on this computer is left out).
    let pre = preflight(&w.s, body.clone()).await;
    assert_eq!(
        pre["code"]["ignored"],
        json!([".env.local", "config/dev.local.json"]),
        "{pre:#}"
    );
    assert_eq!(pre["code"]["carryIgnored"], true);
    // The target has not given this computer its dangerous grant: refused.
    assert_eq!(pre["ok"], false, "{pre:#}");
    assert!(
        codes(&pre["blockers"]).contains(&"ignored_needs_dangerous".to_string()),
        "{pre:#}"
    );
    let (s, v) =
        w.s.call("POST", &format!("/sessions/{SID}/move"), Some(body.clone()))
            .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "move_blocked");
    // A phone without the dangerous right here that asks for them is
    // refused here too (and there: its missing right goes along)…
    let phone =
        w.s.st
            .devices
            .issue(
                "Phone",
                "ios",
                None,
                server::Ingress::Lan,
                "192.168.1.20".parse().unwrap(),
            )
            .1;
    t_trusts_s_with_danger(&w).await;
    let phone_preflight = |body: Value| {
        let phone = phone.clone();
        let s = &w.s;
        async move {
            let (st, pre) = s
                .call_as(
                    &phone,
                    "POST",
                    &format!("/sessions/{SID}/move/preflight"),
                    Some(body),
                )
                .await;
            assert_eq!(st, StatusCode::OK, "{pre}");
            pre
        }
    };
    let mut asking = body.clone();
    asking["carryIgnored"] = json!(true);
    let pre = phone_preflight(asking).await;
    let c = codes(&pre["blockers"]);
    assert!(
        !c.is_empty() && c.iter().all(|c| c == "ignored_needs_dangerous"),
        "{pre:#}"
    );
    assert_eq!(
        pre["blockers"][0]["message"],
        "Carrying files git ignores needs the dangerous permission (set on the desktop)."
    );
    // …and left unsaid they stay where they are.
    let pre = phone_preflight(body.clone()).await;
    assert_eq!(pre["ok"], true, "{pre:#}");
    assert_eq!(pre["code"]["carryIgnored"], false);
    assert_eq!(pre["code"]["ignored"].as_array().map(Vec::len), Some(2));

    // With the dangerous right on both computers they travel.
    let pre = preflight(&w.s, body.clone()).await;
    assert_eq!(pre["ok"], true, "{pre:#}");
    let rec = move_session(&w.s, body).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    assert!(
        rec["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "ignored.tar.gz"),
        "{rec:#}"
    );
    assert_eq!(read(&w.repos.t, ".env.local"), "FROM=source\n");
    assert_eq!(read(&w.repos.t, "config/dev.local.json"), "{}\n");
    assert!(
        codes(&rec["warnings"]).contains(&"ignored_replaced".to_string()),
        "the target says it kept its own: {rec:#}"
    );
    // The source's are where they were.
    assert_eq!(read(&w.repos.s, ".env.local"), "FROM=source\n");

    // Undo: the target's own copy is back, what came is gone.
    let id = rec["id"].as_str().unwrap();
    let (s, v) = w.s.call("POST", &format!("/moves/{id}/undo"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "undone", "{v:#}");
    assert_eq!(read(&w.repos.t, ".env.local"), "FROM=target\n");
    assert!(!w.repos.t.join("config").exists(), "what it added is gone");
    assert_eq!(git(&w.repos.t, &["status", "--porcelain"]), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_files_git_ignores_land_in_a_worktree_too() {
    let w = world().await;
    put(&w.repos.s, ".env.local", "FROM=source\n");
    carry(&w, &[".env.local"]).await;
    t_trusts_s_with_danger(&w).await;
    let rec = move_session(&w.s, to_t(&w, json!({ "apply": "worktree" }))).await;
    assert_eq!(rec["state"], "resumed", "{rec:#}");
    let wt = PathBuf::from(rec["targetCwd"].as_str().unwrap());
    assert!(
        wt.starts_with(w.t.data().join("machines").join("worktrees")),
        "{wt:?}"
    );
    assert_eq!(read(&wt, ".env.local"), "FROM=source\n");
    assert!(
        !w.repos.t.join(".env.local").exists(),
        "the project folder is not touched"
    );
    let id = rec["id"].as_str().unwrap();
    let (s, v) = w.s.call("POST", &format!("/moves/{id}/undo"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!wt.join(".env.local").exists(), "{v:#}");
}

/// `link` is a link to the folder `target` (a junction on Windows, which
/// needs no privilege there).
fn link_dir(link: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listed_path_outside_the_project_or_through_a_link_is_refused_at_preflight() {
    let w = world().await;
    t_trusts_s_with_danger(&w).await;
    let outside = tempfile::tempdir().unwrap();
    put(outside.path(), "secret.env", "TOKEN=outside\n");
    link_dir(&w.repos.s.join("linked"), outside.path());
    carry(&w, &["linked/secret.env"]).await;
    // A path that leaves the project can't be set; written by hand into the
    // settings, it is refused all the same.
    w.s.st.machines.store_write(|f| {
        for p in f.projects.values_mut() {
            if let Some(r) = p.recipe.as_mut() {
                r.carry_ignored.push("../escape.env".into());
            }
        }
    });
    put(w.repos.s.parent().unwrap(), "escape.env", "TOKEN=escape\n");
    let body = to_t(&w, json!({}));
    let pre = preflight(&w.s, body.clone()).await;
    assert_eq!(pre["ok"], false, "{pre:#}");
    let c = codes(&pre["blockers"]);
    assert!(c.contains(&"ignored_link".to_string()), "{pre:#}");
    assert!(c.contains(&"ignored_bad_path".to_string()), "{pre:#}");
    assert_eq!(pre["code"]["ignored"], json!([]), "nothing of them travels");
    let (s, v) =
        w.s.call("POST", &format!("/sessions/{SID}/move"), Some(body.clone()))
            .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "move_blocked");
    // Left here, the move goes on without them.
    let mut leave = body;
    leave["carryIgnored"] = json!(false);
    leave["code"] = json!("none");
    let pre = preflight(&w.s, leave).await;
    assert_eq!(pre["ok"], true, "{pre:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_through_the_remote_needs_it_to_answer_on_both_computers() {
    let w = world().await;
    let body = to_t(&w, json!({ "code": "remote" }));
    let find = |pre: &Value, code: &str| {
        pre["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["code"] == code)
            .cloned()
    };
    // A local bare repository answers on both: nothing in the way.
    let pre = preflight(&w.s, body.clone()).await;
    assert_eq!(pre["ok"], true, "{pre:#}");

    // Gone here: refused before anything starts, in the setup's words.
    let origin = git(&w.repos.s, &["remote", "get-url", "origin"]);
    let missing = w.repos.s.parent().unwrap().join("missing.git");
    let missing = missing.to_str().unwrap();
    git(&w.repos.s, &["remote", "set-url", "origin", missing]);
    let pre = preflight(&w.s, body.clone()).await;
    assert_eq!(pre["ok"], false, "{pre:#}");
    let b = find(&pre, "remote_unreachable").unwrap_or_else(|| panic!("{pre:#}"));
    assert_eq!(
        b["message"],
        "The remote origin doesn't answer from this computer without a prompt."
    );
    assert!(b["fix"].as_str().is_some_and(|f| !f.is_empty()), "{pre:#}");
    assert!(find(&pre, "remote_unreachable_there").is_none(), "{pre:#}");
    let (s, v) =
        w.s.call("POST", &format!("/sessions/{SID}/move"), Some(body.clone()))
            .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "move_blocked");

    // Gone there: the other computer says so, naming itself.
    git(&w.repos.s, &["remote", "set-url", "origin", &origin]);
    git(&w.repos.t, &["remote", "set-url", "origin", missing]);
    let pre = preflight(&w.s, body.clone()).await;
    let b = find(&pre, "remote_unreachable_there").unwrap_or_else(|| panic!("{pre:#}"));
    let there = machines::this_name(&w.t.st);
    assert_eq!(
        b["message"],
        format!("The remote origin doesn't answer on {there} without a prompt.")
    );
    assert!(
        b["fix"]
            .as_str()
            .is_some_and(|f| f.starts_with(&format!("On {there}: "))),
        "{pre:#}"
    );
    assert!(find(&pre, "remote_unreachable").is_none(), "{pre:#}");

    // Machine to machine needs no remote.
    let pre = preflight(&w.s, to_t(&w, json!({ "code": "bundle" }))).await;
    assert!(find(&pre, "remote_unreachable_there").is_none(), "{pre:#}");
}
