//! Machines, Phase 4: a session runs commands on another computer at its
//! own code (docs/PLAN-MACHINES.md §5.11, decisions 1–5) — two app states in
//! one process, each with its own data dir, Claude home and iroh endpoint,
//! paired over real QUIC (the `tests/sync.rs` harness). The session calls
//! the `machine_*` tools through the real `/mcp` endpoint (an owned session
//! naming itself, as `X-MC-Session` does); its approvals are answered
//! through `POST /api/sessions/{id}/respond`, as a dashboard answers them.
//! Commands are written to run in zsh and in PowerShell alike (`echo`,
//! `cat`, `exit`, `sleep`). git runs without this machine's configuration.
//! Nothing touches `~/.claude` or `~/.claude.json`: `CLAUDE_CONFIG_DIR` is
//! set; nothing starts `claude`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::model::PendingSource;
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
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": args },
        });
        let r = reqwest::Client::new()
            .post(format!("{}/mcp", self.url))
            .bearer_auth(self.st.mcp.token())
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

    async fn runs(&self) -> Vec<Value> {
        let (s, v) = self.call("GET", "/machines/runs", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["runs"].as_array().cloned().unwrap_or_default()
    }

    async fn run(&self, id: &str) -> Value {
        let (s, v) = self
            .call("GET", &format!("/machines/runs/{id}"), None)
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    /// The run approval waiting here, when one comes.
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
            assert!(Instant::now() < deadline, "no run approval came");
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

/// B lets A run commands; nothing dangerous.
async fn world() -> World {
    world_with(grants(false, false), grants(true, false)).await
}

impl World {
    /// `machine_run` of the session, its approval answered by the desktop
    /// (`allow`, or `deny`; `always`: the standing grant too).
    async fn run(&self, args: Value, answer: &str) -> Value {
        let a = &self.a;
        let call = {
            let url = a.url.clone();
            let token = a.st.mcp.token().to_string();
            let args = args.clone();
            tokio::spawn(async move {
                let body = json!({
                    "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": "machine_run", "arguments": args },
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
                let v: Value = r.json().await.unwrap();
                v["result"].clone()
            })
        };
        if answer != "none" {
            let item = a.approval().await;
            let body = match answer {
                "allow" => json!({ "decision": "allow" }),
                "always" => json!({ "decision": "allow", "suggestion": 0 }),
                _ => json!({ "decision": "deny" }),
            };
            let token = a.st.auth.token.clone();
            let (s, v) = a.respond(&token, &item, body).await;
            assert_eq!(s, StatusCode::OK, "{v}");
        }
        call.await.unwrap()
    }

    fn b_name(&self) -> String {
        self.b.name()
    }

    /// The run's id in a result's first line.
    fn id_of(result: &Value) -> String {
        let t = text(result);
        let start = t.find("run_").unwrap_or_else(|| panic!("no run id in {t}"));
        t[start..start + 20].to_string()
    }
}

// ─── the tests ─────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_run_grant_nothing_runs_there() {
    let w = world_with(grants(false, false), grants(false, false)).await;
    // The session sees the computer, and that it may not run there.
    let list = w.a.tool("machine_list", json!({})).await;
    assert!(!is_error(&list), "{list}");
    let m = &list["structuredContent"]["machines"][0];
    assert_eq!(m["name"], w.b_name());
    assert_eq!(m["mayRun"], false, "{m}");
    assert_eq!(m["theirGrants"]["run"]["lan"], false);
    assert!(
        m["shell"].is_string(),
        "its shell is known from its hello: {m}"
    );
    // Asking anyway: refused here, before anything is asked or sent.
    let r =
        w.a.tool(
            "machine_run",
            json!({ "machine": w.b_name(), "command": "echo hi" }),
        )
        .await;
    assert!(is_error(&r));
    assert!(text(&r).contains("Run permission"), "{}", text(&r));
    assert!(w.a.no_approval());
    // And B refuses on its own, whatever A thinks.
    let (s, v) = transport::request(
        &w.a.st,
        &w.b.id(),
        "POST",
        "/peer/work/prepare",
        Some(json!({ "project": w.r.a.to_string_lossy() })),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(s, 403, "{v}");
    assert_eq!(v["error"]["code"], "grant_missing");
    assert!(w.b.runs().await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_brings_back_its_output_and_exit_code_from_a_work_folder_at_the_session_s_code() {
    let w = world().await;
    let list = w.a.tool("machine_list", json!({})).await;
    assert_eq!(
        list["structuredContent"]["machines"][0]["mayRun"], true,
        "{list}"
    );
    // The session's uncommitted change and an untracked file travel.
    put(&w.r.a, "a.txt", "alpha, changed on A\n");
    put(&w.r.a, "notes/new.txt", "untracked on A\n");
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "echo hello-from-b" }),
            "allow",
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    let t = text(&r);
    assert!(t.contains("hello-from-b"), "{t}");
    assert!(t.contains("exit code 0"), "{t}");
    let id = World::id_of(&r);
    let rec = w.a.run(&id).await;
    assert_eq!(rec["state"], "done", "{rec:#}");
    assert_eq!(rec["direction"], "out");
    assert_eq!(rec["exitCode"], 0);
    assert!(rec["durationMs"].is_u64());
    let folder = PathBuf::from(rec["folder"].as_str().unwrap());
    // B keeps it too, the other way round.
    let theirs = w.b.run(&id).await;
    assert_eq!(theirs["direction"], "in");
    assert_eq!(theirs["state"], "done");
    assert_eq!(read(&folder, "a.txt"), "alpha, changed on A\n");
    assert_eq!(read(&folder, "notes/new.txt"), "untracked on A\n");
    // A failing command: its exit code.
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "exit 3" }),
            "allow",
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(text(&r).contains("exit code 3"), "{}", text(&r));
    assert_eq!(w.a.run(&World::id_of(&r)).await["state"], "failed");
    // A build's cache (ignored) stays; a stray untracked file goes; the
    // session's code is read in the folder it works in (`cwd`).
    put(&folder, "target/cache.txt", "warm cache");
    put(&folder, "stray.txt", "left over");
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "cat target/cache.txt" }),
            "allow",
        )
        .await;
    assert!(text(&r).contains("warm cache"), "{}", text(&r));
    assert!(
        !folder.join("stray.txt").exists(),
        "an untracked file not in the snapshot goes"
    );
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "cat main.rs", "cwd": "src" }),
            "allow",
        )
        .await;
    assert!(text(&r).contains("fn main()"), "{}", text(&r));
    // The user's own checkout on B: untouched.
    assert_eq!(git(&w.r.b, &["status", "--porcelain", "-uall"]), "");
    assert_eq!(read(&w.r.b, "a.txt"), "alpha\n");
    // The run is a background task of the session, its output readable
    // the way a shell's is.
    let (s, d) = w.a.call("GET", &format!("/sessions/{SID}"), None).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    let task = d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id.as_str())
        .cloned()
        .unwrap_or_else(|| panic!("the run is a task: {d:#}"));
    assert_eq!(task["kind"], "machine");
    assert_eq!(task["status"], "completed");
    let out = reqwest::Client::new()
        .get(format!("{}/api/sessions/{SID}/tasks/{id}/output", w.a.url))
        .bearer_auth(&w.a.st.auth.token)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(out.contains("hello-from-b"), "{out}");
    // Its whole output, paged from B.
    let page =
        w.a.tool("machine_output", json!({ "id": id, "from": 0 }))
            .await;
    assert!(!is_error(&page), "{}", text(&page));
    assert!(text(&page).contains("hello-from-b"), "{}", text(&page));
    // Both computers audited it without the command line.
    for m in [&w.a, &w.b] {
        let lines = m.st.devices.audit.recent(500);
        let line = lines
            .iter()
            .find(|e| {
                e.action == "machine.run"
                    && e.detail.as_ref().is_some_and(|d| d["run"] == id.as_str())
            })
            .expect("an audit line for the run");
        let d = line.detail.as_ref().unwrap();
        assert_eq!(d["program"], "echo");
        assert_eq!(d["commandSha256"].as_str().unwrap().len(), 64);
        assert!(
            !d.to_string().contains("hello-from-b"),
            "never the command line"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_run_is_approved_unless_a_standing_grant_covers_it() {
    let w = world().await;
    // Denied: nothing runs there.
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "echo once" }),
            "deny",
        )
        .await;
    assert!(is_error(&r));
    assert!(text(&r).contains("denied"), "{}", text(&r));
    assert!(w.b.runs().await.is_empty(), "nothing reached B");
    // The approval card names the computer, the folder and the command;
    // its "always allow" is a standing grant, which needs the dangerous
    // right: the local-script token may allow once, not always.
    let call = {
        let url = w.a.url.clone();
        let token = w.a.st.mcp.token().to_string();
        let name = w.b_name();
        tokio::spawn(async move {
            let body = json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "machine_run",
                            "arguments": { "machine": name, "command": "echo granted --twice" } },
            });
            reqwest::Client::new()
                .post(format!("{url}/mcp"))
                .bearer_auth(token)
                .header("x-mc-session", SID)
                .header("mcp-protocol-version", "2025-06-18")
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()["result"]
                .clone()
        })
    };
    let item = w.a.approval().await;
    assert_eq!(item["source"], "machine");
    assert_eq!(item["input"]["machineName"], w.b_name());
    assert_eq!(item["input"]["command"], "echo granted --twice");
    assert!(item["input"]["checkout"]
        .as_str()
        .is_some_and(|c| c.ends_with("app")));
    assert_eq!(item["dangerous"], false);
    let s = &item["suggestions"][0];
    assert_eq!(s["standing"], true);
    assert_eq!(s["raw"]["prefix"], "echo granted");
    let script = w.a.st.auth.script_token.clone();
    let (st, v) =
        w.a.respond(
            &script,
            &item,
            json!({ "decision": "allow", "suggestion": 0 }),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    let desktop = w.a.st.auth.token.clone();
    let (st, v) =
        w.a.respond(
            &desktop,
            &item,
            json!({ "decision": "allow", "suggestion": 0 }),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let r = call.await.unwrap();
    assert!(!is_error(&r), "{}", text(&r));
    let (s, v) = w.a.call("GET", "/machines/run-grants", None).await;
    assert_eq!(s, StatusCode::OK);
    let grant = v["grants"][0].clone();
    assert_eq!(grant["prefix"], "echo granted");
    assert_eq!(grant["machineName"], w.b_name());
    // Covered now: no approval.
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "echo granted again" }),
            "none",
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(text(&r).contains("again"));
    assert_eq!(
        w.a.run(&World::id_of(&r)).await["grant"],
        grant["id"],
        "the grant it ran under"
    );
    // A metacharacter defeats it: asked again (and denied here).
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "echo granted; echo more" }),
            "deny",
        )
        .await;
    assert!(is_error(&r));
    // A prefix at a word boundary only.
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "echo granted-not" }),
            "deny",
        )
        .await;
    assert!(is_error(&r));
    // Revoked (the desktop only): asked again.
    let id = grant["id"].as_str().unwrap();
    let (s, _) =
        w.a.call_as(
            &script,
            "DELETE",
            &format!("/machines/run-grants/{id}"),
            None,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "the local-script token can't revoke"
    );
    let (s, v) =
        w.a.call("DELETE", &format!("/machines/run-grants/{id}"), None)
            .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["grants"].as_array().unwrap().is_empty());
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "echo granted again" }),
            "allow",
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dangerous_command_needs_the_dangerous_right_here_and_the_grant_there() {
    let w = world().await;
    // `.mcp.json` is an MCP server's configuration: dangerous.
    let cmd = "echo .mcp.json";
    let call = |w: &World| {
        let url = w.a.url.clone();
        let token = w.a.st.mcp.token().to_string();
        let name = w.b_name();
        tokio::spawn(async move {
            let body = json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "machine_run",
                            "arguments": { "machine": name, "command": cmd } },
            });
            reqwest::Client::new()
                .post(format!("{url}/mcp"))
                .bearer_auth(token)
                .header("x-mc-session", SID)
                .header("mcp-protocol-version", "2025-06-18")
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()["result"]
                .clone()
        })
    };
    let pending = call(&w);
    let item = w.a.approval().await;
    assert_eq!(item["dangerous"], true, "{item}");
    assert!(
        item["suggestions"].as_array().unwrap().is_empty(),
        "never always"
    );
    // Without the dangerous right here: refused.
    let script = w.a.st.auth.script_token.clone();
    let (s, _) =
        w.a.respond(&script, &item, json!({ "decision": "allow" }))
            .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // With it here, but B never gave A `dangerous`: B refuses.
    let desktop = w.a.st.auth.token.clone();
    let (s, v) =
        w.a.respond(&desktop, &item, json!({ "decision": "allow" }))
            .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = pending.await.unwrap();
    assert!(is_error(&r));
    assert!(text(&r).contains("Dangerous permission"), "{}", text(&r));
    assert!(w.b.runs().await.is_empty(), "nothing ran on B");
    // B gives A `dangerous` too: both rights, it runs.
    let (s, v) =
        w.b.call(
            "PATCH",
            &format!("/machines/{}", w.a.id()),
            Some(json!({ "grants": grants(true, true) })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    greet(&w.a, &w.b).await;
    let r = w
        .run(json!({ "machine": w.b_name(), "command": cmd }), "allow")
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(text(&r).contains(".mcp.json"), "{}", text(&r));
    let theirs = w.b.run(&World::id_of(&r)).await;
    assert_eq!(theirs["dangerous"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancel_or_a_timeout_stops_the_run_there() {
    let w = world().await;
    // A timeout.
    let started = Instant::now();
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "sleep 60", "timeout": 3 }),
            "allow",
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(text(&r).contains("ran out of time"), "{}", text(&r));
    assert!(started.elapsed() < Duration::from_secs(40));
    assert_eq!(w.b.run(&World::id_of(&r)).await["state"], "timedOut");
    // A cancel while it runs.
    let a = &w.a;
    let call = {
        let url = a.url.clone();
        let token = a.st.mcp.token().to_string();
        let name = w.b_name();
        tokio::spawn(async move {
            let body = json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "machine_run",
                            "arguments": { "machine": name, "command": "sleep 120" } },
            });
            reqwest::Client::new()
                .post(format!("{url}/mcp"))
                .bearer_auth(token)
                .header("x-mc-session", SID)
                .header("mcp-protocol-version", "2025-06-18")
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()["result"]
                .clone()
        })
    };
    let item = a.approval().await;
    let token = a.st.auth.token.clone();
    let (s, _) = a
        .respond(&token, &item, json!({ "decision": "allow" }))
        .await;
    assert_eq!(s, StatusCode::OK);
    let deadline = Instant::now() + Duration::from_secs(45);
    let id = loop {
        let running =
            w.b.runs()
                .await
                .into_iter()
                .find(|r| r["state"] == "running");
        if let Some(r) = running {
            break r["id"].as_str().unwrap().to_string();
        }
        assert!(Instant::now() < deadline, "the run never started");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let c = a.tool("machine_cancel", json!({ "id": id })).await;
    assert!(!is_error(&c), "{}", text(&c));
    assert!(text(&c).contains("cancelled"), "{}", text(&c));
    let r = call.await.unwrap();
    assert!(text(&r).contains("cancelled"), "{}", text(&r));
    assert_eq!(w.b.run(&id).await["state"], "cancelled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn machines_off_on_either_computer_refuses_a_run() {
    let w = world().await;
    machines::set_enabled(&w.b.st, false);
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "echo hi" }),
            "none",
        )
        .await;
    assert!(is_error(&r));
    assert!(text(&r).contains("machines_off"), "{}", text(&r));
    machines::set_enabled(&w.b.st, true);
    machines::set_enabled(&w.a.st, false);
    let r = w
        .run(
            json!({ "machine": w.b_name(), "command": "echo hi" }),
            "none",
        )
        .await;
    assert!(is_error(&r));
    assert!(text(&r).contains("Computers is off"), "{}", text(&r));
    assert!(w.b.runs().await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn files_a_run_makes_are_listed_and_brought_into_the_project_where_git_ignores_them() {
    let w = world().await;
    // `cat` of a file into another: the same in zsh and PowerShell.
    let r = w
        .run(
            json!({
                "machine": w.b_name(),
                "command": "cat a.txt > made.txt",
                "artifacts": "made.txt",
            }),
            "allow",
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    assert!(text(&r).contains("- made.txt ("), "{}", text(&r));
    let id = World::id_of(&r);
    let folder = PathBuf::from(w.a.run(&id).await["folder"].as_str().unwrap().to_string());
    let made = std::fs::read(folder.join("made.txt")).unwrap();
    let f =
        w.a.tool(
            "machine_output",
            json!({ "id": id, "artifact": "made.txt" }),
        )
        .await;
    assert!(!is_error(&f), "{}", text(&f));
    let here =
        w.r.a
            .join(".mother-claude")
            .join("artifacts")
            .join(&id)
            .join("made.txt");
    assert_eq!(std::fs::read(&here).unwrap(), made, "the same bytes");
    // git ignores it: the session's status is its own.
    let status = git(&w.r.a, &["status", "--porcelain", "-uall"]);
    assert!(!status.contains(".mother-claude"), "{status}");
    // Only a listed file comes.
    let f =
        w.a.tool("machine_output", json!({ "id": id, "artifact": "a.txt" }))
            .await;
    assert!(is_error(&f));
    // Another session's project can't reach the run.
    let w2_sid = "b0ee0000-1111-4222-8333-444455556666";
    let other = tempfile::tempdir().unwrap();
    git(other.path(), &["init", "-q"]);
    w.a.st
        .mark_owned(w2_sid, &other.path().to_string_lossy(), 1)
        .await;
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "machine_output", "arguments": { "id": id } },
    });
    let v: Value = reqwest::Client::new()
        .post(format!("{}/mcp", w.a.url))
        .bearer_auth(w.a.st.mcp.token())
        .header("x-mc-session", w2_sid)
        .header("mcp-protocol-version", "2025-06-18")
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["result"]["isError"], true, "{v}");
}
