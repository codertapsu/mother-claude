//! Machines, code sync (docs/PLAN-MACHINES.md §5.15): a project kept the
//! same on two computers, both ways — two app states in one process, each
//! with its own data dir, Claude home and iroh endpoint, paired over real
//! QUIC (the `tests/moves.rs` harness). Steps run through *Sync now* (the
//! watchers are the app's; their debounce has its own test in
//! `machines::sync::watch`); git runs without this machine's configuration.
//! Nothing touches `~/.claude` or `~/.claude.json`: `CLAUDE_CONFIG_DIR` is
//! set.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::machines::{self, code::GitRunner, transport};
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
    _dir: TempDir,
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
        _dir: dir,
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

    /// `GET /api/machines/sync`'s only pair (none: `Null`).
    async fn pair(&self) -> Value {
        let (s, v) = self.call("GET", "/machines/sync", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["pairs"][0].clone()
    }

    async fn sync_now(&self, id: &str) -> Value {
        let (s, v) = self
            .call("POST", &format!("/machines/sync/{id}/now"), None)
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    /// Waits until its pair's status is `state` (and `reason`, when said).
    async fn status_is(&self, what: &str, state: &str, reason: Option<&str>) -> Value {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let p = self.pair().await;
            let st = &p["status"];
            if st["state"] == state && reason.is_none_or(|r| st["reason"] == r) {
                return p;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {p:#}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn state_of(v: &Value) -> &str {
    v["pairing"]["state"].as_str().unwrap_or("")
}

/// Pairs `a` (host) and `b` (joiner) with the grants each gives the other,
/// greets both ways and turns Machines on on both.
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

/// A hello each way (grants and capabilities travel with it).
async fn greet(a: &Machine, b: &Machine) {
    machines::greet(a.st.clone(), b.id(), key_of(a, &b.id())).await;
    machines::greet(b.st.clone(), a.id(), key_of(b, &a.id())).await;
}

/// View, drive and move both ways; carrying ignored files only with
/// `dangerous`.
fn grants(dangerous: bool) -> Value {
    json!({
        "view": {"lan": true, "internet": true},
        "control": {"lan": true, "internet": true},
        "move": {"lan": true, "internet": true},
        "run": {"lan": false, "internet": false},
        "dangerous": {"lan": dangerous, "internet": dangerous}
    })
}

fn no_move() -> Value {
    let mut g = grants(false);
    g["move"] = json!({"lan": false, "internet": false});
    g
}

// ─── repositories ──────────────────────────────────────────────────────────────

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

fn staged(dir: &Path) -> String {
    git(dir, &["diff", "--cached", "--name-only"])
}

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

/// An origin with one commit, and a clone of it in each computer's projects
/// folder (`<tmp>/a/app`, `<tmp>/b/app`).
struct Repos {
    _tmp: TempDir,
    a_root: PathBuf,
    b_root: PathBuf,
    a: PathBuf,
    b: PathBuf,
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
    put(&seed, ".gitignore", "target/\n.env.local\n");
    git(&seed, &["add", "-A"]);
    git(&seed, &["commit", "-q", "-m", "seed"]);
    git(&seed, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
    let (a_root, b_root) = (root.join("a"), root.join("b"));
    let (a, b) = (a_root.join("app"), b_root.join("app"));
    for to in [&a, &b] {
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
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
    Repos {
        _tmp: tmp,
        a_root,
        b_root,
        a,
        b,
    }
}

/// Two paired computers, a root mapping their projects folders, and the
/// project cloned on both.
struct World {
    a: Machine,
    b: Machine,
    r: Repos,
}

async fn world_with(a_gives: Value, b_gives: Value) -> World {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    // B takes A's roots only when it gave A `move` (roots steer writes).
    let b_takes_roots = b_gives["move"]["lan"] == true;
    pair(&a, &b, a_gives, b_gives).await;
    let r = repos();
    let (ida, idb) = (a.id(), b.id());
    let (s, v) = a
        .call(
            "PUT",
            "/roots",
            Some(json!({ "upsert": [{ "name": "projects", "paths": {
                &ida: r.a_root.to_string_lossy(), &idb: r.b_root.to_string_lossy()
            } }] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let a_folder = r.a.to_string_lossy().into_owned();
    let deadline = Instant::now() + Duration::from_secs(45);
    while b_takes_roots && machines::roots::map_here(&b.st, &ida, &a_folder).is_none() {
        assert!(Instant::now() < deadline, "the root never reached B");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    World { a, b, r }
}

async fn world() -> World {
    world_with(grants(false), grants(false)).await
}

impl World {
    /// A turns it on; B's desktop confirms. Answers the pair's id.
    async fn sync_on(&self) -> String {
        let (s, v) = self
            .a
            .call(
                "PUT",
                "/machines/sync",
                Some(json!({ "project": self.r.a, "machine": self.b.id(), "on": true })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let id = v["pairs"][0]["id"].as_str().expect("a pair").to_string();
        let ask = self
            .b
            .wait("the ask on B", |v| {
                v["asks"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|a| a["kind"] == "sync"))
            })
            .await["asks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["kind"] == "sync")
            .cloned()
            .unwrap();
        let (s, v) = self
            .b
            .call(
                "POST",
                &format!("/machines/asks/{}", ask["id"].as_str().unwrap()),
                Some(json!({ "answer": "confirm" })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["ask"]["state"], "done", "{v:#}");
        self.a.status_is("A in sync", "inSync", None).await;
        self.b.status_is("B in sync", "inSync", None).await;
        id
    }
}

// ─── the tests ─────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn it_is_off_until_both_computers_turned_it_on() {
    let w = world().await;
    // Off by default: nothing listed, nothing travels.
    assert!(w.a.pair().await.is_null());
    assert!(w.b.pair().await.is_null());

    // A turns it on: B is asked; until its desktop confirms, nothing syncs.
    put(&w.r.a, "a.txt", "alpha, before it was on\n");
    let (s, v) =
        w.a.call(
            "PUT",
            "/machines/sync",
            Some(json!({ "project": w.r.a, "machine": w.b.id(), "on": true })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let p = &v["pairs"][0];
    assert_eq!(p["status"]["state"], "waiting", "{p:#}");
    assert_eq!(p["here"]["on"], true);
    assert_eq!(p["peer"]["on"], false);
    assert_eq!(p["there"], w.r.b.to_string_lossy().as_ref());
    let id = p["id"].as_str().unwrap().to_string();
    let on_b = w.b.view().await;
    let ask = on_b["asks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "sync")
        .cloned()
        .expect("asked on B");
    assert_eq!(ask["folder"], w.r.b.to_string_lossy().as_ref());
    assert_eq!(ask["state"], "asked");
    assert!(w.b.pair().await.is_null(), "nothing on B until it confirms");
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "waiting");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha\n", "nothing travelled");

    // Only the desktop may turn it on, and only B's desktop may confirm.
    let (s, _) = transport::request(
        &w.a.st,
        &w.b.id(),
        "POST",
        &format!("/api/machines/asks/{}", ask["id"].as_str().unwrap()),
        Some(json!({ "answer": "confirm" })),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(s, 404, "not a route another computer reaches");

    // B confirms: on on both, and the first sync brings A's change (B was
    // clean at the same commit).
    let (s, v) =
        w.b.call(
            "POST",
            &format!("/machines/asks/{}", ask["id"].as_str().unwrap()),
            Some(json!({ "answer": "confirm" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ask"]["state"], "done", "{v:#}");
    let pa = w.a.status_is("A in sync", "inSync", None).await;
    let pb = w.b.status_is("B in sync", "inSync", None).await;
    assert_eq!(pa["peer"]["on"], true);
    assert_eq!(pb["id"], id.as_str(), "one id on both computers");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha, before it was on\n");
    assert_eq!(pa["last"]["direction"], "out");
    assert_eq!(pb["last"]["direction"], "in");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn edits_commits_and_a_branch_switch_travel_both_ways() {
    let w = world().await;
    let id = w.sync_on().await;

    // Staged, unstaged and untracked on A reach B as they are.
    put(&w.r.a, "a.txt", "alpha, staged\n");
    git(&w.r.a, &["add", "a.txt"]);
    put(&w.r.a, "b.txt", "bravo, not staged\n");
    put(&w.r.a, "notes/new.md", "- untracked\n");
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "inSync", "{p:#}");
    for f in ["a.txt", "b.txt", "notes/new.md"] {
        assert_eq!(read(&w.r.b, f), read(&w.r.a, f), "{f}");
    }
    assert_eq!(staged(&w.r.b), "a.txt");
    assert_eq!(
        git(&w.r.b, &["status", "--porcelain"]),
        git(&w.r.a, &["status", "--porcelain"])
    );

    // A commit on B reaches A as a fast-forward.
    git(&w.r.b, &["add", "-A"]);
    git(&w.r.b, &["commit", "-q", "-m", "made on B"]);
    let p = w.b.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "inSync", "{p:#}");
    assert_eq!(head(&w.r.a), head(&w.r.b));
    assert_eq!(git(&w.r.a, &["status", "--porcelain"]), "");
    assert_eq!(read(&w.r.a, "notes/new.md"), "- untracked\n");

    // Sync now on the side that did not change asks the other to send.
    put(&w.r.b, "c.txt", "charlie, from B\n");
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "inSync", "{p:#}");
    assert_eq!(read(&w.r.a, "c.txt"), "charlie, from B\n");

    // A branch switch travels.
    git(&w.r.a, &["switch", "-q", "-c", "feature"]);
    put(&w.r.a, "f.txt", "on feature\n");
    git(&w.r.a, &["add", "-A"]);
    git(&w.r.a, &["commit", "-q", "-m", "feature work"]);
    w.a.sync_now(&id).await;
    assert_eq!(git(&w.r.b, &["symbolic-ref", "HEAD"]), "refs/heads/feature");
    assert_eq!(head(&w.r.b), head(&w.r.a));
    assert_eq!(read(&w.r.b, "f.txt"), "on feature\n");

    // Nothing changed: in sync, nothing travels.
    let before = w.b.pair().await["last"]["step"].clone();
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "inSync");
    assert_eq!(w.b.pair().await["last"]["step"], before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_on_both_pause_both_and_either_side_can_be_kept() {
    let w = world().await;
    let id = w.sync_on().await;

    // Both change: paused on both, nothing overwritten.
    put(&w.r.a, "a.txt", "alpha, from A\n");
    put(&w.r.b, "b.txt", "bravo, from B\n");
    put(&w.r.b, "b-own.txt", "B's untracked\n");
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "paused", "{p:#}");
    assert_eq!(p["status"]["reason"], "conflict");
    assert_eq!(p["conflict"]["here"]["files"], 1);
    assert_eq!(p["conflict"]["there"]["files"], 2);
    let pb = w.b.status_is("B paused", "paused", Some("conflict")).await;
    assert_eq!(pb["conflict"]["here"]["files"], 2, "{pb:#}");
    assert_eq!(read(&w.r.a, "b.txt"), "bravo\n");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha\n");
    // Sync now while it waits for a choice: it stays paused.
    let p = w.b.sync_now(&id).await;
    assert_eq!(p["status"]["reason"], "conflict");

    // Keep A's: B's changes are kept under a ref on B, then A's land.
    let (s, p) =
        w.a.call(
            "POST",
            &format!("/machines/sync/{id}/resolve"),
            Some(json!({ "keep": "here" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["status"]["state"], "inSync", "{p:#}");
    let kept = p["kept"][0]["ref"]
        .as_str()
        .expect("a kept ref")
        .to_string();
    assert!(
        kept.starts_with(&format!("refs/mother-claude/sync/{id}/kept-")),
        "{kept}"
    );
    assert_eq!(p["kept"][0]["side"], "there");
    w.b.status_is("B in sync", "inSync", None).await;
    assert_eq!(read(&w.r.b, "a.txt"), "alpha, from A\n");
    assert_eq!(read(&w.r.b, "b.txt"), "bravo\n");
    assert!(!w.r.b.join("b-own.txt").exists());
    assert_eq!(
        git(&w.r.b, &["show", &format!("{kept}:b.txt")]),
        "bravo, from B"
    );
    assert_eq!(
        git(&w.r.b, &["show", &format!("{kept}:b-own.txt")]),
        "B's untracked"
    );
    assert_eq!(w.b.pair().await["kept"][0]["ref"], kept.as_str());

    // Again; this time B's is kept and A's go under a ref on A.
    put(&w.r.a, "a.txt", "alpha, from A again\n");
    put(&w.r.b, "b.txt", "bravo, committed on B\n");
    git(&w.r.b, &["add", "-A"]);
    git(&w.r.b, &["commit", "-q", "-m", "committed on B"]);
    let p = w.b.sync_now(&id).await;
    assert_eq!(p["status"]["reason"], "conflict", "{p:#}");
    w.a.status_is("A paused", "paused", Some("conflict")).await;
    let (s, p) =
        w.a.call(
            "POST",
            &format!("/machines/sync/{id}/resolve"),
            Some(json!({ "keep": "there" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let pa = w.a.status_is("A in sync", "inSync", None).await;
    w.b.status_is("B in sync", "inSync", None).await;
    let kept = pa["kept"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["side"] == "here")
        .and_then(|k| k["ref"].as_str())
        .expect("kept on A")
        .to_string();
    assert_eq!(
        git(&w.r.a, &["show", &format!("{kept}:a.txt")]),
        "alpha, from A again"
    );
    assert_eq!(head(&w.r.a), head(&w.r.b), "B's commit came");
    assert_eq!(read(&w.r.a, "a.txt"), "alpha, from A\n");
    let (s, v) =
        w.a.call(
            "POST",
            &format!("/machines/sync/{id}/resolve"),
            Some(json!({ "keep": "here" })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "nothing to choose: {v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_change_on_both_is_no_conflict() {
    let w = world().await;
    let id = w.sync_on().await;
    for dir in [&w.r.a, &w.r.b] {
        put(dir, "a.txt", "alpha, the same on both\n");
        put(dir, "same.txt", "new on both\n");
    }
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "inSync", "{p:#}");
    assert_eq!(p["last"]["direction"], "both");
    assert!(p["conflict"].is_null());
    let pb = w.b.pair().await;
    assert_eq!(pb["status"]["state"], "inSync", "{pb:#}");
    // And they go on from there.
    put(&w.r.b, "b.txt", "bravo, after\n");
    w.b.sync_now(&id).await;
    assert_eq!(read(&w.r.a, "b.txt"), "bravo, after\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_move_grant_it_waits() {
    // B never gave A `move`: turning it on is refused.
    let w = world_with(grants(false), no_move()).await;
    let (s, v) =
        w.a.call(
            "PUT",
            "/machines/sync",
            Some(json!({ "project": w.r.a, "machine": w.b.id(), "on": true })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "sync_no_grant");

    // On with both grants; then B takes `move` back: A's steps wait.
    let w = world().await;
    let id = w.sync_on().await;
    let (s, v) =
        w.b.call(
            "PATCH",
            &format!("/machines/{}", w.a.id()),
            Some(json!({ "grants": no_move() })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    greet(&w.a, &w.b).await;
    put(&w.r.a, "a.txt", "alpha, not allowed\n");
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "paused", "{p:#}");
    assert_eq!(p["status"]["reason"], "noGrant");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha\n");
    // B's own steps wait too (A would land into nothing it may not).
    put(&w.r.b, "b.txt", "bravo, B's\n");
    let p = w.b.sync_now(&id).await;
    assert_eq!(p["status"]["reason"], "noGrant", "{p:#}");
    // Given back: it goes on from the checkouts as they are.
    let (s, v) =
        w.b.call(
            "PATCH",
            &format!("/machines/{}", w.a.id()),
            Some(json!({ "grants": grants(false) })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    greet(&w.a, &w.b).await;
    let p = w.a.sync_now(&id).await;
    assert_eq!(
        p["status"]["reason"], "conflict",
        "both changed meanwhile: {p:#}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn machines_off_on_either_computer_pauses_it() {
    let w = world().await;
    // Off on B: B is not even asked.
    machines::set_enabled(&w.b.st, false);
    let (s, v) =
        w.a.call(
            "PUT",
            "/machines/sync",
            Some(json!({ "project": w.r.a, "machine": w.b.id(), "on": true })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "machines_off", "{v}");
    assert!(w.a.pair().await.is_null());
    assert!(w.b.view().await["asks"].as_array().unwrap().is_empty());
    machines::set_enabled(&w.b.st, true);
    let id = w.sync_on().await;
    put(&w.r.a, "a.txt", "alpha, while off\n");
    machines::set_enabled(&w.b.st, false);
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "paused", "{p:#}");
    assert_eq!(p["status"]["reason"], "machinesOff");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha\n");
    let pb = w.b.pair().await;
    assert_eq!(pb["status"]["reason"], "machinesOff", "{pb:#}");
    machines::set_enabled(&w.b.st, true);
    machines::set_enabled(&w.a.st, false);
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["reason"], "machinesOff", "{p:#}");
    assert!(p["status"]["detail"]
        .as_str()
        .unwrap()
        .contains("Computers is off on"));
    // On again: it goes on, nothing was queued — the checkout as it is now.
    put(&w.r.a, "a.txt", "alpha, once on again\n");
    machines::set_enabled(&w.a.st, true);
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "inSync", "{p:#}");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha, once on again\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_it_waits_and_goes_on_when_the_other_is_back() {
    let w = world().await;
    let id = w.sync_on().await;
    transport::go_offline(&w.b.st).await;
    put(&w.r.a, "a.txt", "alpha, while B is away\n");
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "paused", "{p:#}");
    assert_eq!(p["status"]["reason"], "offline");
    transport::come_back(&w.b.st);
    transport::ensure_started(&w.b.st).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        greet(&w.a, &w.b).await;
        let p = w.a.sync_now(&id).await;
        if p["status"]["state"] == "inSync" {
            break;
        }
        assert!(Instant::now() < deadline, "never back: {p:#}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert_eq!(read(&w.r.b, "a.txt"), "alpha, while B is away\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ignored_files_stay_unless_the_project_carries_them() {
    // B lets A carry dangerous things (ignored files); A sets its list.
    let w = world_with(grants(true), grants(true)).await;
    let id = w.sync_on().await;
    put(&w.r.a, ".env.local", "SECRET=from-a\n");
    put(&w.r.a, "target/out.bin", "build output\n");
    put(&w.r.a, "a.txt", "alpha, with an ignored file\n");
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "inSync", "{p:#}");
    assert!(!w.r.b.join(".env.local").exists(), "not listed: it stays");
    assert!(!w.r.b.join("target").exists());
    assert_eq!(read(&w.r.b, "a.txt"), "alpha, with an ignored file\n");

    let (s, v) =
        w.a.call(
            "PUT",
            "/machines/settings",
            Some(json!({ "projects": { "upsert": [{
                "project": w.r.a,
                "code": {},
                "recipe": { "carryIgnored": [".env.local"] }
            }] } })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    put(&w.r.b, ".env.local", "SECRET=b's own\n");
    put(&w.r.a, "a.txt", "alpha, carrying it\n");
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "inSync", "{p:#}");
    assert_eq!(read(&w.r.b, ".env.local"), "SECRET=from-a\n");
    assert!(!w.r.b.join("target").exists(), "only what is listed");
    // B's own copy it replaced is kept in B's data folder.
    let replaced =
        w.b._dir
            .path()
            .join("data/machines/sync")
            .join(&id)
            .join("replaced");
    let sets: Vec<_> = std::fs::read_dir(&replaced).unwrap().flatten().collect();
    assert_eq!(sets.len(), 1);
    assert_eq!(read(&sets[0].path(), ".env.local"), "SECRET=b's own\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_step_that_fails_leaves_both_checkouts_as_they_were_and_off_is_immediate() {
    let w = world().await;
    let id = w.sync_on().await;
    put(&w.r.a, "a.txt", "alpha, the step fails\n");
    put(&w.r.a, "new.txt", "untracked\n");
    let b_head = head(&w.r.b);
    w.b.st.machines.fail_sync_at(Some("land"));
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "paused", "{p:#}");
    assert_eq!(p["status"]["reason"], "error");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha\n");
    assert!(!w.r.b.join("new.txt").exists());
    assert_eq!(head(&w.r.b), b_head);
    assert_eq!(git(&w.r.b, &["status", "--porcelain"]), "");
    assert_eq!(read(&w.r.a, "a.txt"), "alpha, the step fails\n");
    // Tried again, it goes.
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "inSync", "{p:#}");
    assert_eq!(read(&w.r.b, "new.txt"), "untracked\n");

    // Off on A: gone here at once, B is told and stops.
    let (s, v) =
        w.a.call(
            "PUT",
            "/machines/sync",
            Some(json!({ "project": w.r.a, "machine": w.b.id(), "on": false })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["pairs"].as_array().unwrap().is_empty(), "{v}");
    let pb = w.b.status_is("B told", "off", None).await;
    assert!(pb["status"]["detail"]
        .as_str()
        .unwrap()
        .starts_with("Turned off on"));
    put(&w.r.b, "b.txt", "bravo, after off\n");
    let (s, _) =
        w.b.call("POST", &format!("/machines/sync/{id}/now"), None)
            .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(read(&w.r.a, "b.txt"), "bravo\n", "nothing travels once off");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn it_waits_while_a_session_working_in_the_project_moves() {
    let w = world().await;
    let id = w.sync_on().await;
    // A move of a session working in A's project is under way.
    let moves = w.a._dir.path().join("data").join("machines").join("moves");
    std::fs::create_dir_all(&moves).unwrap();
    let mv = "mv_0123456789abcdef";
    let record = json!({
        "id": mv, "role": "out", "session": "a0ee0000-1111-4222-8333-444455556666",
        "from": { "id": w.a.id(), "name": "A", "os": "macos" },
        "to": { "id": w.b.id(), "name": "B", "os": "windows" },
        "state": "sending", "sourceCwd": w.r.a.join("src").to_string_lossy(),
        "createdAt": 1, "updatedAt": 1,
    });
    std::fs::write(moves.join(format!("{mv}.json")), record.to_string()).unwrap();
    put(&w.r.a, "a.txt", "alpha, while it moves\n");
    let p = w.a.sync_now(&id).await;
    assert_eq!(p["status"]["state"], "paused", "{p:#}");
    assert_eq!(p["status"]["reason"], "moveRunning");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha\n");
    // The other side waits for it too.
    put(&w.r.b, "b.txt", "bravo, while it moves there\n");
    let p = w.b.sync_now(&id).await;
    assert_eq!(p["status"]["reason"], "moveRunning", "{p:#}");
    // The move ended: it goes on.
    std::fs::remove_file(moves.join(format!("{mv}.json"))).unwrap();
    let p = w.a.sync_now(&id).await;
    assert_eq!(
        p["status"]["reason"], "conflict",
        "both changed meanwhile: {p:#}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turning_it_off_before_it_is_confirmed_withdraws_the_request() {
    let w = world().await;
    let body = |on: bool| json!({ "project": w.r.a, "machine": w.b.id(), "on": on });
    let (s, v) = w.a.call("PUT", "/machines/sync", Some(body(true))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ask_open = |v: &Value| {
        v["asks"].as_array().is_some_and(|l| {
            l.iter()
                .any(|a| a["kind"] == "sync" && a["state"] == "asked")
        })
    };
    w.b.wait("the request on B", ask_open).await;
    let (s, v) = w.a.call("PUT", "/machines/sync", Some(body(false))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["pairs"].as_array().unwrap().is_empty(), "{v}");
    let on_b =
        w.b.wait("the request withdrawn on B", |v| !ask_open(v))
            .await;
    let ask = on_b["asks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "sync")
        .cloned()
        .unwrap();
    assert_eq!(ask["state"], "failed", "{ask:#}");
    assert!(ask["detail"]
        .as_str()
        .unwrap()
        .contains("turned it off before it was confirmed"));
    // Answering it now changes nothing.
    let (s, _) =
        w.b.call(
            "POST",
            &format!("/machines/asks/{}", ask["id"].as_str().unwrap()),
            Some(json!({ "answer": "confirm" })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(w.b.pair().await.is_null());
}
