//! Machines, Phase 4: a session hands a task to a Claude Code session on
//! another computer (docs/PLAN-MACHINES.md §5.11, decision 6) — two app
//! states in one process, each with its own data dir, Claude home and iroh
//! endpoint, paired over real QUIC (the `tests/work.rs` harness). The asking
//! session calls `machine_task` through the real `/mcp` endpoint (an owned
//! session naming itself, as `X-MC-Session` does); its approval is answered
//! through `POST /api/sessions/{id}/respond`, as a dashboard answers it.
//! The task session there is never started: its spawn is recorded (as a
//! move's resume is), and its work is simulated in its worktree, its turn
//! ended through the hook the control stream calls on a `result`. git runs
//! without this machine's configuration. Nothing touches `~/.claude` or
//! `~/.claude.json`: `CLAUDE_CONFIG_DIR` is set; nothing starts `claude`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::model::PendingSource;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::machines::work::tasks::{self, TurnEnd};
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

/// The asking session (owned, so it may name itself to `/mcp`).
const SID: &str = "a0ee0000-1111-4222-8333-444455556666";

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

    fn name(&self) -> String {
        machines::this_name(&self.st)
    }

    /// A `tools/call` of the session through `/mcp`: the tool's result.
    async fn tool(&self, name: &str, args: Value) -> Value {
        tool_call(&self.url, self.st.mcp.token(), name, args).await
    }

    async fn tasks(&self) -> Vec<Value> {
        let (s, v) = self.call("GET", "/machines/tasks", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["tasks"].as_array().cloned().unwrap_or_default()
    }

    async fn task(&self, id: &str) -> Value {
        let (s, v) = self
            .call("GET", &format!("/machines/tasks/{id}"), None)
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    /// Waits until this computer's record of task `id` holds `pred`.
    async fn task_until(&self, id: &str, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let v = self.task(id).await;
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

    /// The task approval waiting here, when one comes.
    async fn approval(&self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if let Some(item) = self
                .st
                .pending
                .all()
                .into_iter()
                .find(|i| i.source == PendingSource::Machine)
            {
                return serde_json::to_value(item).unwrap();
            }
            assert!(Instant::now() < deadline, "no task approval came");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn no_approval(&self) -> bool {
        !self
            .st
            .pending
            .all()
            .iter()
            .any(|i| i.source == PendingSource::Machine)
    }

    async fn respond(&self, token: &str, item: &Value, body: Value) -> (StatusCode, Value) {
        let mut b = body;
        b["requestId"] = item["id"].clone();
        self.call_as(token, "POST", &format!("/sessions/{SID}/respond"), Some(b))
            .await
    }

    /// The task session this computer would have started (its spawn is
    /// recorded), once it comes.
    async fn spawned(&self, n: usize) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let all = self.st.machines.recorded_task_spawns();
            if all.len() >= n {
                return all[n - 1].clone();
            }
            assert!(Instant::now() < deadline, "no task session was started");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

async fn tool_call(url: &str, token: &str, name: &str, args: Value) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": args },
    });
    let r = reqwest::Client::new()
        .post(format!("{url}/mcp"))
        .bearer_auth(token)
        .header("x-mc-session", SID)
        .header("mcp-protocol-version", "2025-06-18")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert!(
        v.get("error").is_none(),
        "a tool's failure is never a protocol error: {v}"
    );
    v["result"].clone()
}

fn text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn is_error(result: &Value) -> bool {
    result["isError"] == true
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

/// View, drive and move; `run` and `dangerous` as asked.
fn grants(run: bool, dangerous: bool) -> Value {
    json!({
        "view": {"lan": true, "internet": true},
        "control": {"lan": true, "internet": true},
        "move": {"lan": true, "internet": true},
        "run": {"lan": run, "internet": run},
        "dangerous": {"lan": dangerous, "internet": dangerous}
    })
}

// ─── repositories ──────────────────────────────────────────────────────────────

fn git(cwd: &Path, args: &[&str]) -> String {
    let op = GitRunner::new().isolated().begin().unwrap();
    op.git(cwd, args)
        .unwrap_or_else(|e| panic!("git {} in {}: {e}", args.join(" "), cwd.display()))
}

fn git_ok(cwd: &Path, args: &[&str]) -> bool {
    let op = GitRunner::new().isolated().begin().unwrap();
    op.cmd(cwd).args(args).ok().unwrap()
}

fn put(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
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
    put(&seed, "src/main.rs", "fn main() {}\n");
    put(&seed, ".gitignore", "target/\n*.log\n");
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

/// Two paired computers (A gives B `a_gives`, B gives A `b_gives`), a root
/// mapping their projects folders, the project cloned on both, and a
/// session on A working in its checkout.
struct World {
    a: Machine,
    b: Machine,
    r: Repos,
}

async fn world_with(a_gives: Value, b_gives: Value) -> World {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
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
    while machines::roots::map_here(&b.st, &ida, &a_folder).is_none() {
        assert!(Instant::now() < deadline, "the root never reached B");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    a.st.mark_owned(SID, &a_folder, 1).await;
    World { a, b, r }
}

/// B lets A run commands (and so hand it tasks); nothing dangerous.
async fn world() -> World {
    world_with(grants(false, false), grants(true, false)).await
}

impl World {
    fn b_name(&self) -> String {
        self.b.name()
    }

    /// `machine_task` of the session, started in the background.
    fn hand_over(&self, args: Value) -> tokio::task::JoinHandle<Value> {
        let url = self.a.url.clone();
        let token = self.a.st.mcp.token().to_string();
        tokio::spawn(async move { tool_call(&url, &token, "machine_task", args).await })
    }

    /// `machine_task`, its approval allowed by the desktop: the call, the
    /// approval it raised, and the session B would have started.
    async fn allowed(&self, args: Value) -> (tokio::task::JoinHandle<Value>, Value, Value) {
        let n = self.b.st.machines.recorded_task_spawns().len();
        let call = self.hand_over(args);
        let item = self.a.approval().await;
        let token = self.a.st.auth.token.clone();
        let (s, v) = self
            .a
            .respond(&token, &item, json!({ "decision": "allow" }))
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let spawn = self.b.spawned(n + 1).await;
        (call, item, spawn)
    }

    /// The branch a task's work becomes on A.
    fn branch(&self, task: &str) -> String {
        machines::code::task_branch(&self.b_name(), task).expect("a branch")
    }
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str()
        .unwrap_or_else(|| panic!("no {k} in {v}"))
        .to_string()
}

/// Ends the task session's turn as its stream's `result` would.
fn end_turn(b: &Machine, session: &str, summary: &str) {
    tasks::turn_ended(
        &b.st,
        session,
        TurnEnd {
            summary: Some(summary.to_string()),
            ..Default::default()
        },
    );
}

// ─── the tests ─────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_run_grant_no_task_is_handed_over() {
    let w = world_with(grants(false, false), grants(false, false)).await;
    let list = w.a.tool("machine_list", json!({})).await;
    let m = &list["structuredContent"]["machines"][0];
    assert_eq!(m["mayTask"], false, "{m}");
    let r =
        w.a.tool(
            "machine_task",
            json!({ "machine": w.b_name(), "task": "Fix the Windows build." }),
        )
        .await;
    assert!(is_error(&r));
    assert!(text(&r).contains("Run permission"), "{}", text(&r));
    assert!(w.a.no_approval(), "refused before anyone is asked");
    // And B refuses on its own, whatever A thinks.
    let (st, v) = transport::request(
        &w.a.st,
        &w.b.id(),
        "POST",
        "/peer/work/tasks/prepare",
        Some(json!({ "project": w.r.a.to_string_lossy() })),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(st, 403, "{v}");
    assert_eq!(v["error"]["code"], "grant_missing");
    assert!(w.b.tasks().await.is_empty());
    assert!(w.b.st.machines.recorded_task_spawns().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_s_work_comes_back_as_a_branch_that_only_moves_forward() {
    let w = world().await;
    let list = w.a.tool("machine_list", json!({})).await;
    assert_eq!(
        list["structuredContent"]["machines"][0]["mayTask"], true,
        "{list}"
    );
    // The asking session's uncommitted change and an untracked file go along.
    put(&w.r.a, "a.txt", "alpha, changed on A\n");
    put(&w.r.a, "notes/new.txt", "untracked on A\n");
    let a_head = git(&w.r.a, &["rev-parse", "HEAD"]);
    let a_status = git(&w.r.a, &["status", "--porcelain", "-uall"]);
    let b_head = git(&w.r.b, &["rev-parse", "HEAD"]);
    let task_text = "Make the build work on Windows.\nUse backslashes where paths are shown.";
    let (call, item, spawn) = w
        .allowed(json!({ "machine": w.b_name(), "task": task_text }))
        .await;
    // The approval names the computer, the folder there and the task.
    assert_eq!(item["source"], "machine");
    assert_eq!(item["toolName"], "machine_task");
    assert_eq!(item["input"]["kind"], "task");
    assert_eq!(item["input"]["machineName"], w.b_name());
    assert_eq!(item["input"]["task"], task_text);
    assert!(item["input"]["checkout"]
        .as_str()
        .is_some_and(|c| c.ends_with("app")));
    assert!(
        s(&item, "title").contains("Make the build work on Windows."),
        "{item}"
    );
    assert_eq!(item["dangerous"], false);
    assert!(
        item["suggestions"].as_array().is_none_or(|l| l.is_empty()),
        "never an always-allow for a task: {item}"
    );
    // B started an owned session in the task's own worktree, at A's code.
    let task = s(&spawn, "task");
    let session = s(&spawn, "session");
    let folder = PathBuf::from(s(&spawn, "folder"));
    assert_eq!(
        s(&spawn, "cwd"),
        s(&spawn, "folder"),
        "the session works at the top"
    );
    assert_eq!(spawn["permissionMode"], "default");
    assert!(
        s(&spawn, "prompt").contains(&format!(
            "Mother Claude handed you this task from {}",
            w.a.name()
        )),
        "{spawn}"
    );
    assert!(s(&spawn, "prompt").ends_with(task_text), "{spawn}");
    assert!(folder.ends_with(Path::new(&task).join("app")), "{spawn}");
    assert_eq!(read(&folder, "a.txt"), "alpha, changed on A\n");
    assert_eq!(read(&folder, "notes/new.txt"), "untracked on A\n");
    // It runs, on both computers' records.
    w.b.task_until(&task, "running there", |v| v["state"] == "running")
        .await;
    w.a.task_until(&task, "running here", |v| v["state"] == "running")
        .await;
    // The session commits a change and leaves another uncommitted, then
    // its turn ends.
    put(&folder, "build.ps1", "cargo build --release\n");
    git(&folder, &["add", "build.ps1"]);
    git(&folder, &["commit", "-q", "-m", "a Windows build script"]);
    put(
        &folder,
        "src/main.rs",
        "fn main() { println!(\"C:\\\\\"); }\n",
    );
    end_turn(&w.b, &session, "Added build.ps1 and fixed the path.");
    let r = call.await.unwrap();
    assert!(!is_error(&r), "{}", text(&r));
    let t = text(&r);
    let branch = w.branch(&task);
    let short = branch.strip_prefix("refs/heads/").unwrap();
    assert!(t.contains(&format!("the branch {short}")), "{t}");
    assert!(
        t.contains("1 commit and changes it left uncommitted, 2 files changed"),
        "{t}"
    );
    assert!(t.contains("Added build.ps1 and fixed the path."), "{t}");
    assert!(t.contains("git diff "), "{t}");
    // The branch on A holds both, on top of A's code.
    let show = |rev: &str, path: &str| git(&w.r.a, &["show", &format!("{rev}:{path}")]);
    assert_eq!(show(short, "build.ps1"), "cargo build --release");
    assert!(show(short, "src/main.rs").contains("println!"));
    assert_eq!(show(short, "a.txt"), "alpha, changed on A");
    assert_eq!(show(short, "notes/new.txt"), "untracked on A");
    // Never checked out, never merged: A's checkout is as it was.
    assert_eq!(git(&w.r.a, &["rev-parse", "HEAD"]), a_head);
    assert_eq!(git(&w.r.a, &["status", "--porcelain", "-uall"]), a_status);
    assert_eq!(read(&w.r.a, "a.txt"), "alpha, changed on A\n");
    // B's own checkout: untouched.
    assert_eq!(git(&w.r.b, &["rev-parse", "HEAD"]), b_head);
    assert_eq!(git(&w.r.b, &["status", "--porcelain", "-uall"]), "");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha\n");
    let first = git(&w.r.a, &["rev-parse", short]);
    let rec = w.a.task(&task).await;
    assert_eq!(rec["state"], "done", "{rec:#}");
    assert_eq!(rec["branchName"], short);
    assert_eq!(rec["commits"], 1);
    assert_eq!(rec["uncommitted"], true);
    assert_eq!(rec["taskSession"], session.as_str());
    assert_eq!(w.b.task(&task).await["state"], "done");
    // Someone sends the session another message there: it runs again, and
    // the end of that turn moves the branch forward.
    tasks::turn_started(&w.b.st, &session);
    w.a.task_until(&task, "running again", |v| v["state"] == "running")
        .await;
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "the path fix"]);
    put(&folder, "README.md", "Build on Windows with build.ps1.\n");
    git(&folder, &["add", "README.md"]);
    git(&folder, &["commit", "-q", "-m", "how to build"]);
    end_turn(&w.b, &session, "Committed everything and wrote a README.");
    let rec =
        w.a.task_until(&task, "the second result here", |v| v["landed"] == 2)
            .await;
    assert_eq!(rec["state"], "done", "{rec:#}");
    assert_eq!(rec["commits"], 3);
    assert_eq!(rec["uncommitted"], false);
    let second = git(&w.r.a, &["rev-parse", short]);
    assert_ne!(first, second);
    assert!(
        git_ok(&w.r.a, &["merge-base", "--is-ancestor", &first, &second]),
        "the branch moved forward, never rewritten"
    );
    assert_eq!(show(short, "README.md"), "Build on Windows with build.ps1.");
    assert_eq!(git(&w.r.a, &["rev-parse", "HEAD"]), a_head);
    // machine_wait answers at once for a task that ended.
    let r = w.a.tool("machine_wait", json!({ "id": task })).await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(text(&r).contains("wrote a README"), "{}", text(&r));
    // The task is a background task of the asking session.
    let (st, d) = w.a.call("GET", &format!("/sessions/{SID}"), None).await;
    assert_eq!(st, StatusCode::OK, "{d}");
    let bg = d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == task.as_str())
        .cloned()
        .unwrap_or_else(|| panic!("the task is a background task: {d:#}"));
    assert_eq!(bg["kind"], "machine");
    assert_eq!(bg["status"], "completed");
    // Both computers audited it without the task's text.
    for m in [&w.a, &w.b] {
        let lines = m.st.devices.audit.recent(500);
        let line = lines
            .iter()
            .find(|e| {
                e.action == "machine.task.asked"
                    && e.detail
                        .as_ref()
                        .is_some_and(|d| d["task"] == task.as_str())
            })
            .expect("an audit line for the task");
        let d = line.detail.as_ref().unwrap();
        assert_eq!(d["firstLineSha256"].as_str().unwrap().len(), 64);
        for l in &lines {
            let text = l.detail.as_ref().map(Value::to_string).unwrap_or_default();
            assert!(
                !text.contains("Make the build work"),
                "never the task: {text}"
            );
        }
    }
    // Removed from A: gone there too (its worktree with it); the branch
    // stays here. A running task can't be removed; this one ended.
    let script = w.a.st.auth.script_token.clone();
    let (st, _) =
        w.a.call_as(&script, "DELETE", &format!("/machines/tasks/{task}"), None)
            .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "not with the local-script token");
    let (st, v) =
        w.a.call("DELETE", &format!("/machines/tasks/{task}"), None)
            .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["there"], "removed", "{v}");
    assert!(w.a.tasks().await.is_empty());
    assert!(w.b.tasks().await.is_empty());
    let deadline = Instant::now() + Duration::from_secs(20);
    while folder.exists() {
        assert!(Instant::now() < deadline, "the worktree stayed");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(git(&w.r.a, &["rev-parse", short]), second);
    assert!(!git(&w.r.b, &["worktree", "list"]).contains(&task));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_that_changes_nothing_makes_no_branch() {
    let w = world().await;
    let (call, _, spawn) = w
        .allowed(json!({ "machine": w.b_name(), "task": "Look around; change nothing." }))
        .await;
    let task = s(&spawn, "task");
    end_turn(&w.b, &s(&spawn, "session"), "Nothing needed changing.");
    let r = call.await.unwrap();
    assert!(!is_error(&r), "{}", text(&r));
    assert!(
        text(&r).contains("It changed nothing, so no branch was made."),
        "{}",
        text(&r)
    );
    assert!(
        text(&r).contains("Nothing needed changing."),
        "{}",
        text(&r)
    );
    let branches = git(&w.r.a, &["for-each-ref", "refs/heads/mother-claude/"]);
    assert!(branches.is_empty(), "{branches}");
    assert!(w.a.task(&task).await["branchName"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancel_stops_the_task_session() {
    let w = world().await;
    let (call, _, spawn) = w
        .allowed(json!({ "machine": w.b_name(), "task": "Refactor everything." }))
        .await;
    let task = s(&spawn, "task");
    let session = s(&spawn, "session");
    w.a.task_until(&task, "running here", |v| v["state"] == "running")
        .await;
    let c = w.a.tool("machine_cancel", json!({ "id": task })).await;
    assert!(!is_error(&c), "{}", text(&c));
    assert!(text(&c).contains("cancelled"), "{}", text(&c));
    let r = call.await.unwrap();
    assert!(text(&r).contains("cancelled"), "{}", text(&r));
    assert_eq!(w.b.task(&task).await["state"], "cancelled");
    // A later end of its session changes nothing any more.
    end_turn(&w.b, &session, "Done anyway.");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(w.b.task(&task).await["state"], "cancelled");
    assert_eq!(w.a.task(&task).await["state"], "cancelled");
    // An id that is no task of this session's: refused.
    let r =
        w.a.tool("machine_wait", json!({ "id": "task_ffffffffffffffff" }))
            .await;
    assert!(is_error(&r));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn machines_off_on_either_computer_refuses_a_task() {
    let w = world().await;
    machines::set_enabled(&w.b.st, false);
    let r =
        w.a.tool(
            "machine_task",
            json!({ "machine": w.b_name(), "task": "Fix it." }),
        )
        .await;
    assert!(is_error(&r));
    assert!(text(&r).contains("machines_off"), "{}", text(&r));
    machines::set_enabled(&w.b.st, true);
    machines::set_enabled(&w.a.st, false);
    let r =
        w.a.tool(
            "machine_task",
            json!({ "machine": w.b_name(), "task": "Fix it." }),
        )
        .await;
    assert!(is_error(&r));
    assert!(text(&r).contains("Machines is off"), "{}", text(&r));
    assert!(w.b.tasks().await.is_empty());
    assert!(w.a.no_approval());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_without_approvals_needs_the_dangerous_right_on_both_computers() {
    let w = world().await;
    let args = json!({
        "machine": w.b_name(),
        "task": "Clean up the build.",
        "permissionMode": "bypassPermissions",
    });
    // B never gave A `dangerous`: refused before anyone is asked.
    let r = w.a.tool("machine_task", args.clone()).await;
    assert!(is_error(&r));
    assert!(text(&r).contains("Dangerous permission"), "{}", text(&r));
    assert!(w.a.no_approval());
    // B gives it: the approval is a dangerous one, which the local-script
    // token can't allow.
    let (st, v) =
        w.b.call(
            "PATCH",
            &format!("/machines/{}", w.a.id()),
            Some(json!({ "grants": grants(true, true) })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    greet(&w.a, &w.b).await;
    let call = w.hand_over(args);
    let item = w.a.approval().await;
    assert_eq!(item["dangerous"], true, "{item}");
    let script = w.a.st.auth.script_token.clone();
    let (st, _) =
        w.a.respond(&script, &item, json!({ "decision": "allow" }))
            .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let desktop = w.a.st.auth.token.clone();
    let (st, v) =
        w.a.respond(&desktop, &item, json!({ "decision": "allow" }))
            .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let spawn = w.b.spawned(1).await;
    assert_eq!(spawn["permissionMode"], "bypassPermissions");
    assert_eq!(spawn["allowBypass"], true);
    let task = s(&spawn, "task");
    assert_eq!(w.b.task(&task).await["dangerous"], true);
    let c = w.a.tool("machine_cancel", json!({ "id": task })).await;
    assert!(!is_error(&c), "{}", text(&c));
    let _ = call.await.unwrap();
}
