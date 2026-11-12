//! Machines, Phase 4 part 3: a team role that runs on another computer
//! (docs/PLAN-MACHINES.md §5.11, decision 9) — two app states in one
//! process, each with its own data dir, Claude home and iroh endpoint,
//! paired over real QUIC (the `tests/tasks.rs` harness). The team runs on
//! A, the lead's computer; its `windows` role names B (`WIN-BOX`). A's
//! launcher starts it there through the peer link; B admits it and starts
//! it as an owned session in a worktree of its own at A's code.
//!
//! No `claude` ever starts: B records the role session's spawn (as a
//! task's), A's lead is never started (the launcher's loop does not run
//! here, and `MOTHER_CLAUDE_CLI` names no program). The role session's board
//! calls go through B's real `/mcp` endpoint — the session marked owned
//! there, as its spawn would — and reach A's board with the binding; its
//! work is simulated in its worktree. git runs without this machine's
//! configuration. Nothing touches `~/.claude`: `CLAUDE_CONFIG_DIR` is set.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::machines::work::roles;
use mother_claude_lib::machines::{self, code::GitRunner, transport};
use mother_claude_lib::orchestration::{delegation, launcher};
use mother_claude_lib::server::{self, auth::Auth};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

/// The process-wide part, once: a test home's `.claude.json` is inside
/// `CLAUDE_CONFIG_DIR`, never the user's own, and no `claude` can start.
fn isolate() {
    static ONCE: OnceLock<PathBuf> = OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.join("unused-config"));
        std::env::set_var("MOTHER_CLAUDE_CLI", dir.join("no-claude-here"));
        dir
    });
}

struct Machine {
    st: AppState,
    url: String,
    _dir: TempDir,
}

async fn machine(lookup: &MemoryLookup, name: &str) -> Machine {
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
    let m = Machine {
        st,
        url: format!("http://{addr}"),
        _dir: dir,
    };
    let (s, v) = m
        .call("PATCH", "/machines/self", Some(json!({ "name": name })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    m
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

    fn name(&self) -> String {
        machines::this_name(&self.st)
    }

    /// A team tool as the session `session` calls it, through `/mcp`.
    async fn tool(&self, session: &str, name: &str, args: Value) -> Value {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": args },
        });
        let r = reqwest::Client::new()
            .post(format!("{}/mcp", self.url))
            .bearer_auth(self.st.mcp.token())
            .header("x-mc-session", session)
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

    async fn team(&self, id: &str) -> Value {
        let (s, v) = self.call("GET", &format!("/teams/{id}"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    async fn task(&self, team: &str, n: u64) -> Value {
        let (s, v) = self
            .call("GET", &format!("/teams/{team}/tasks/{n}"), None)
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
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

/// View, drive and move; `run` as asked; nothing dangerous.
fn grants(run: bool) -> Value {
    json!({
        "view": {"lan": true, "internet": true},
        "control": {"lan": true, "internet": true},
        "move": {"lan": true, "internet": true},
        "run": {"lan": run, "internet": run},
        "dangerous": {"lan": false, "internet": false}
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
    put(&seed, ".gitignore", "target/\n.claude/\n");
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

/// Two paired computers — A (`MAC-LEAD`) runs the team, B (`WIN-BOX`) may
/// run its role when it gave A `run` — a root mapping their projects
/// folders, the project cloned on both.
struct World {
    a: Machine,
    b: Machine,
    r: Repos,
}

async fn world_with(b_gives_run: bool) -> World {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup, "MAC-LEAD").await;
    let b = machine(&lookup, "WIN-BOX").await;
    pair(&a, &b, grants(false), grants(b_gives_run)).await;
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
    World { a, b, r }
}

/// A template whose `windows` role runs on `machine`; its lead runs on A.
fn template(machine: &str) -> Value {
    json!({
        "id": "cross-platform",
        "name": "Cross-platform",
        "roles": [
            {"id": "lead", "title": "Lead", "lead": true, "model": "opus",
             "paths": ["docs/**"]},
            {"id": "windows", "title": "Windows", "model": "sonnet", "effort": "medium",
             "permissionMode": "acceptEdits", "paths": ["src/**", "build.ps1"],
             "machine": machine}
        ],
        "integration": {"branch": "main", "mergeBy": "lead", "review": true},
        "idleStopMinutes": 30
    })
}

impl World {
    /// The template saved on A and a team started on A's project: its id.
    async fn team(&self, machine: &str) -> String {
        let (s, v) = self
            .a
            .call("POST", "/team-templates", Some(template(machine)))
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        let (s, v) = self
            .a
            .call(
                "POST",
                "/teams",
                Some(json!({
                    "project": self.r.a.to_string_lossy(),
                    "template": "cross-platform",
                })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v["id"].as_str().unwrap().to_string()
    }

    /// A task for the `windows` role, written by the user on A.
    async fn task_for_windows(&self, team: &str, title: &str) -> u64 {
        let (s, v) = self
            .a
            .call(
                "POST",
                &format!("/teams/{team}/tasks"),
                Some(json!({ "title": title, "goal": "Make it build on Windows.", "role": "windows", "class": "implement" })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v["id"].as_u64().unwrap()
    }
}

/// The role session's spawn B recorded for `binding`'s `n`-th start.
async fn spawned(b: &Machine, n: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let all = b.st.machines.recorded_role_spawns();
        if all.len() >= n {
            return all[n - 1].clone();
        }
        assert!(Instant::now() < deadline, "no role session was started");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn flag(argv: &[Value], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == name)
        .and_then(|i| argv.get(i + 1))
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ─── the tests ─────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_role_on_another_computer_starts_there_works_the_lead_s_board_and_brings_its_work_back() {
    let w = world_with(true).await;
    let team = w.team("WIN-BOX").await;
    // The lead's uncommitted change goes along with its code.
    put(&w.r.a, "a.txt", "alpha, changed on the Mac\n");
    let a_head = git(&w.r.a, &["rev-parse", "HEAD"]);
    let a_status = git(&w.r.a, &["status", "--porcelain", "-uall"]);
    let b_head = git(&w.r.b, &["rev-parse", "HEAD"]);
    let first = w.task_for_windows(&team, "Windows build script").await;
    let second = w.task_for_windows(&team, "Windows paths").await;

    // ── A's launcher starts the role on B ──
    let session = launcher::start_role(&w.a.st, &team, "windows", Some(first), &[])
        .await
        .expect("started on B");
    let spawn = spawned(&w.b, 1).await;
    assert_eq!(spawn["session"], session.as_str());
    assert_eq!(spawn["role"], "windows");
    assert_eq!(spawn["team"], team.as_str());
    // The same options a local role gets: its name, the one setting, the
    // team tools by exact name, the guard, the role definitions.
    let argv = spawn["argv"].as_array().unwrap().clone();
    assert_eq!(flag(&argv, "-n").as_deref(), Some("app/windows"));
    assert_eq!(
        flag(&argv, "--settings").as_deref(),
        Some(r#"{"crossSessionInbound":"accept"}"#)
    );
    let allowed = flag(&argv, "--allowedTools").unwrap();
    assert!(allowed.starts_with("mcp__plugin_mother-claude_mc__task_create,"));
    assert_eq!(allowed.split(',').count(), 10);
    assert_eq!(
        flag(&argv, "--permission-mode").as_deref(),
        Some("acceptEdits")
    );
    assert_eq!(flag(&argv, "--model").as_deref(), Some("sonnet"));
    assert!(!argv.iter().any(|a| a == "--agent"), "never --agent");
    assert!(
        !argv.iter().any(|a| a == "-w"),
        "its worktree is Mother Claude's"
    );
    assert_eq!(spawn["guardHooks"], true);
    assert_eq!(spawn["paths"], json!(["src/**", "build.ps1"]));
    let agents = PathBuf::from(flag(&argv, "--agents").expect("the role definitions"));
    let defs: Value = serde_json::from_str(&std::fs::read_to_string(&agents).unwrap()).unwrap();
    assert!(
        defs["windows"].is_object() && defs["lead"].is_object(),
        "{defs}"
    );
    let charter = flag(&argv, "--append-system-prompt").unwrap();
    assert!(charter.contains("You run on WIN-BOX"), "{charter}");
    assert!(charter.contains("The lead, the other roles and the task board are on MAC-LEAD"));
    assert!(
        charter.contains("mother-claude/WIN-BOX/app/windows"),
        "{charter}"
    );
    assert!(spawn["prompt"].as_str().unwrap().starts_with(&format!(
        "[Mother Claude · assignment] Task {first}: Windows build script"
    )));
    // In a worktree of B's checkout at A's code; both checkouts untouched.
    let folder = PathBuf::from(spawn["cwd"].as_str().unwrap());
    assert_eq!(read(&folder, "a.txt"), "alpha, changed on the Mac\n");
    assert_eq!(git(&w.r.b, &["rev-parse", "HEAD"]), b_head);
    assert_eq!(git(&w.r.b, &["status", "--porcelain", "-uall"]), "");
    assert_eq!(git(&w.r.a, &["rev-parse", "HEAD"]), a_head);
    // The task is its, claimed before it started.
    let t = w.a.task(&team, first).await;
    assert_eq!(t["status"], "claimed", "{t:#}");
    assert_eq!(t["claimedBy"], session.as_str());
    // The team view says where it runs; it counts on B, never on A.
    let view = w.a.team(&team).await;
    let role = view["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "windows")
        .unwrap()
        .clone();
    assert_eq!(role["state"], "idle", "{role:#}");
    assert_eq!(role["machine"]["name"], "WIN-BOX");
    assert_eq!(role["machine"]["spec"], "WIN-BOX");
    assert!(launcher::running_workers(&w.a.st).await.is_empty());
    let there = launcher::running_workers(&w.b.st).await;
    assert_eq!(there.len(), 1, "{there:?}");
    assert_eq!(there[0].session_id, session);
    assert_eq!(there[0].for_machine.as_deref(), Some("MAC-LEAD"));

    // ── its board calls reach A's board ──
    w.b.st
        .mark_owned(&session, &folder.to_string_lossy(), 1)
        .await;
    let r =
        w.b.tool(
            &session,
            "task_update",
            json!({ "id": first, "status": "in_progress", "note": "Writing build.ps1" }),
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    let t = w.a.task(&team, first).await;
    assert_eq!(t["status"], "in_progress", "{t:#}");
    assert_eq!(t["progress"][0]["text"], "Writing build.ps1");
    let r =
        w.b.tool(&session, "task_list", json!({ "mine": true }))
            .await;
    assert!(!is_error(&r), "{}", text(&r));
    let rows = r["structuredContent"]["tasks"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["id"], first);
    let r =
        w.b.tool(&session, "task_claim", json!({ "id": second }))
            .await;
    assert!(!is_error(&r), "{}", text(&r));
    assert_eq!(w.a.task(&team, second).await["claimedBy"], session.as_str());
    // The board's rules hold as for a role here: no task writing.
    let r =
        w.b.tool(
            &session,
            "task_create",
            json!({ "title": "x", "goal": "y", "role": "lead" }),
        )
        .await;
    assert!(is_error(&r));
    assert!(text(&r).contains("lead"), "{}", text(&r));
    // The lead's messages reach its session there.
    let t = w.a.st.teams.get(&team).unwrap();
    assert!(delegation::deliver(&w.a.st, &t, "windows", "[Mother Claude · board] hello").await);
    let said = w.b.st.machines.recorded_role_messages();
    assert_eq!(said[0]["text"], "[Mother Claude · board] hello");

    // ── a forged or wrong binding is refused ──
    let binding = t.slot("windows").remote.unwrap().binding;
    let call = |body: Value| {
        let st = w.b.st.clone();
        let a = w.a.id();
        async move {
            transport::request(
                &st,
                &a,
                "POST",
                "/peer/teams/board",
                Some(body),
                Duration::from_secs(20),
            )
            .await
            .unwrap()
        }
    };
    let good = json!({
        "binding": binding, "secret": "0".repeat(64), "team": team, "role": "windows",
        "session": session, "tool": "task_list", "args": {},
    });
    let (s, v) = call(good.clone()).await;
    assert_eq!(s, 403, "a guessed secret: {v}");
    assert_eq!(v["error"]["code"], "binding_refused");
    let mut other = good.clone();
    other["binding"] = json!("rb_0123456789abcdef");
    assert_eq!(call(other).await.0, 403);
    let mut lead = good.clone();
    lead["role"] = json!("lead");
    assert_eq!(call(lead).await.0, 403);
    assert!(w
        .a
        .st
        .devices
        .audit
        .recent(200)
        .iter()
        .any(|e| e.action == "team.remote.refused"));

    // ── task_done: its work comes back as its branch, forward only ──
    put(&folder, "build.ps1", "cargo build --release\n");
    git(&folder, &["add", "build.ps1"]);
    git(&folder, &["commit", "-q", "-m", "a Windows build script"]);
    put(
        &folder,
        "src/main.rs",
        "fn main() { println!(\"C:\\\\\"); }\n",
    );
    let r =
        w.b.tool(
            &session,
            "task_done",
            json!({ "id": first, "summary": "Added build.ps1.", "branch": "worktree-windows" }),
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    let branch = "mother-claude/WIN-BOX/app/windows";
    assert!(text(&r).contains(branch), "{}", text(&r));
    let t = w.a.task(&team, first).await;
    assert_eq!(t["status"], "review", "{t:#}");
    assert_eq!(t["result"]["branch"], branch, "never the worker's own name");
    assert_eq!(t["remote"]["branch"], branch);
    assert_eq!(t["remote"]["machineName"], "WIN-BOX");
    assert_eq!(t["remote"]["commits"], 1);
    assert_eq!(t["remote"]["uncommitted"], true);
    let show = |rev: &str, path: &str| git(&w.r.a, &["show", &format!("{rev}:{path}")]);
    assert_eq!(show(branch, "build.ps1"), "cargo build --release");
    assert!(show(branch, "src/main.rs").contains("println!"));
    assert_eq!(show(branch, "a.txt"), "alpha, changed on the Mac");
    let tip1 = git(&w.r.a, &["rev-parse", branch]);
    // Never checked out or merged: A's checkout is as it was.
    assert_eq!(git(&w.r.a, &["rev-parse", "HEAD"]), a_head);
    assert_eq!(git(&w.r.a, &["status", "--porcelain", "-uall"]), a_status);
    // The second task's end moves the branch forward.
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "the path fix"]);
    put(&folder, "src/paths.rs", "pub const SEP: char = '\\\\';\n");
    let r =
        w.b.tool(
            &session,
            "task_done",
            json!({ "id": second, "summary": "Fixed the paths." }),
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    let tip2 = git(&w.r.a, &["rev-parse", branch]);
    assert_ne!(tip1, tip2);
    assert!(
        git_ok(&w.r.a, &["merge-base", "--is-ancestor", &tip1, &tip2]),
        "the branch moved forward, never rewritten"
    );
    assert!(show(branch, "src/paths.rs").contains("SEP"));
    assert_eq!(w.a.task(&team, second).await["remote"]["branch"], branch);
    // Both computers audited it, never a task's text.
    for (m, action) in [(&w.a, "team.remote.result"), (&w.b, "machine.role.result")] {
        let lines = m.st.devices.audit.recent(500);
        assert!(lines.iter().any(|e| e.action == action), "{action}");
        for l in &lines {
            let text = l.detail.as_ref().map(Value::to_string).unwrap_or_default();
            assert!(
                !text.contains("Windows build script"),
                "never a task: {text}"
            );
        }
    }

    // ── stop from the lead stops it there ──
    launcher::stop_role(&w.a.st, &team, "windows", "the user stopped it").await;
    assert_eq!(w.b.st.machines.recorded_role_stops(), vec![session.clone()]);
    let (s, v) = w.b.call("GET", "/machines/roles", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["roles"][0]["state"], "stopped", "{v:#}");
    assert!(v["roles"][0].get("secret").is_none(), "never shown");
    let role = w.a.team(&team).await["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "windows")
        .cloned()
        .unwrap();
    assert_eq!(role["state"], "stopped", "{role:#}");
    assert!(launcher::running_workers(&w.b.st).await.is_empty());
    // Its next start is there again, under the same binding, in the same
    // worktree — which holds work A has not merged, so it keeps it rather
    // than take A's newer code.
    let third = w.task_for_windows(&team, "Windows icons").await;
    let again = launcher::start_role(&w.a.st, &team, "windows", Some(third), &[])
        .await
        .expect("started again");
    let spawn2 = spawned(&w.b, 2).await;
    assert_eq!(spawn2["session"], again.as_str());
    assert_eq!(spawn2["cwd"], spawn["cwd"]);
    assert_eq!(read(&folder, "build.ps1"), "cargo build --release\n");
    assert!(folder.join("src/paths.rs").is_file());
    let (_, v) = w.b.call("GET", "/machines/roles", None).await;
    assert!(
        v["roles"][0]["keptNote"]
            .as_str()
            .is_some_and(|n| n.contains("not merged")),
        "{v:#}"
    );
    assert_eq!(
        w.a.st
            .teams
            .get(&team)
            .unwrap()
            .slot("windows")
            .remote
            .unwrap()
            .binding,
        binding
    );
    // It finishes that task with nothing more to bring back; once A merged
    // the role's branch, its next start (for a new task) takes A's newer
    // code.
    w.b.st
        .mark_owned(&again, &folder.to_string_lossy(), 2)
        .await;
    let r =
        w.b.tool(
            &again,
            "task_done",
            json!({ "id": third, "summary": "The icons were fine already." }),
        )
        .await;
    assert!(!is_error(&r), "{}", text(&r));
    assert_eq!(git(&w.r.a, &["rev-parse", branch]), tip2, "nothing new");
    launcher::stop_role(&w.a.st, &team, "windows", "between tasks").await;
    git(&w.r.a, &["stash", "-q"]);
    git(&w.r.a, &["merge", "-q", "--no-edit", branch]);
    git(&w.r.a, &["stash", "pop", "-q"]);
    put(&w.r.a, "docs/plan.md", "The lead's newer work.\n");
    let fourth = w.task_for_windows(&team, "Windows installer").await;
    let again = launcher::start_role(&w.a.st, &team, "windows", Some(fourth), &[])
        .await
        .expect("started a third time");
    assert_eq!(spawned(&w.b, 3).await["session"], again.as_str());
    assert_eq!(read(&folder, "docs/plan.md"), "The lead's newer work.\n");
    assert_eq!(
        read(&folder, "build.ps1"),
        "cargo build --release\n",
        "merged"
    );
    launcher::stop_role(&w.a.st, &team, "windows", "done").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_run_or_with_machines_off_the_role_does_not_start_there() {
    let w = world_with(false).await;
    // A computer nobody paired: a warning on the template, a refusal at the
    // team's start.
    let (s, v) =
        w.a.call("POST", "/team-templates/validate", Some(template("NOBODY")))
            .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["warnings"][0]["path"], "roles[1].machine", "{v}");
    let (s, v) =
        w.a.call("POST", "/team-templates", Some(template("NOBODY")))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) =
        w.a.call(
            "POST",
            "/teams",
            Some(json!({ "project": w.r.a.to_string_lossy(), "template": "cross-platform" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "machine_unknown", "{v}");
    let (s, _) =
        w.a.call("DELETE", "/team-templates/cross-platform", None)
            .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // os:windows names B only when B runs Windows; name it directly here.
    let team = w.team(&w.b.name()).await;
    let task = w.task_for_windows(&team, "Windows build").await;
    // B never gave A `run`: refused before anything is sent, in words.
    let e = launcher::start_role(&w.a.st, &team, "windows", Some(task), &[])
        .await
        .unwrap_err();
    assert!(e.to_string().contains("Run permission"), "{e}");
    assert!(w.b.st.machines.recorded_role_spawns().is_empty());
    let role = &w.a.team(&team).await["roles"][1];
    assert!(
        role["lastError"]
            .as_str()
            .unwrap_or_default()
            .contains("Run permission"),
        "{role:#}"
    );
    // And B refuses on its own, whatever A thinks.
    let (st, v) = transport::request(
        &w.a.st,
        &w.b.id(),
        "POST",
        "/peer/teams/roles/prepare",
        Some(
            json!({ "binding": "rb_0123456789abcdef", "team": team, "role": "windows",
                     "project": w.r.a.to_string_lossy(), "model": "sonnet" }),
        ),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(st, 403, "{v}");
    assert_eq!(v["error"]["code"], "grant_missing");
    // B gives it; Machines off there refuses, in B's words.
    let (s, v) =
        w.b.call(
            "PATCH",
            &format!("/machines/{}", w.a.id()),
            Some(json!({ "grants": grants(true) })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    greet(&w.a, &w.b).await;
    machines::set_enabled(&w.b.st, false);
    let e = launcher::start_role(&w.a.st, &team, "windows", Some(task), &[])
        .await
        .unwrap_err();
    assert!(e.to_string().contains("Machines is off"), "{e}");
    // Machines off here: refused before anything is asked.
    machines::set_enabled(&w.b.st, true);
    machines::set_enabled(&w.a.st, false);
    let e = launcher::start_role(&w.a.st, &team, "windows", Some(task), &[])
        .await
        .unwrap_err();
    assert!(
        e.to_string().contains("Machines is off on this computer"),
        "{e}"
    );
    assert!(w.b.st.machines.recorded_role_spawns().is_empty());
    // On again, with `run`: it starts there.
    machines::set_enabled(&w.a.st, true);
    let session = launcher::start_role(&w.a.st, &team, "windows", Some(task), &[])
        .await
        .expect("started");
    assert_eq!(spawned(&w.b, 1).await["session"], session.as_str());
    // A lead not heard from while its role sat idle: B stops it by the
    // launcher's idle rule (the lead starts it again when it has work).
    let (_, v) = w.b.call("GET", "/machines/roles", None).await;
    let heard = v["roles"][0]["heardAt"].as_i64().expect("heard");
    let minute = 60_000;
    assert!(roles::host::stop_unattended(&w.b.st, heard + 29 * minute)
        .await
        .is_empty());
    assert_eq!(
        roles::host::stop_unattended(&w.b.st, heard + 30 * minute)
            .await
            .len(),
        1
    );
    assert_eq!(w.b.st.machines.recorded_role_stops(), vec![session.clone()]);
    // The lead's next look sees it stopped; the role starts there again.
    roles::lead::refresh(&w.a.st, &team, "windows").await;
    let role = &w.a.team(&team).await["roles"][1];
    assert_eq!(role["state"], "stopped", "{role:#}");
    let again = launcher::start_role(&w.a.st, &team, "windows", Some(task), &[])
        .await
        .expect("started again");
    assert_eq!(spawned(&w.b, 2).await["session"], again.as_str());
    // B takes `run` back: its role stops there.
    let (s, v) =
        w.b.call(
            "PATCH",
            &format!("/machines/{}", w.a.id()),
            Some(json!({ "grants": grants(false) })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let deadline = Instant::now() + Duration::from_secs(20);
    while w.b.st.machines.recorded_role_stops().len() < 2 {
        assert!(Instant::now() < deadline, "the role kept running on B");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(w.b.st.machines.recorded_role_stops()[1], again);
}

/// The lead's Capacity page lists its teams' roles on other computers —
/// where they run, with that computer's last admission decision — apart
/// from its own role sessions, never counted here (they count there).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_lead_s_capacity_page_lists_its_roles_elsewhere_never_counted_here() {
    use mother_claude_lib::orchestration::scheduler::{self, RateLimitNote};
    let w = world_with(true).await;
    let team = w.team("WIN-BOX").await;
    let task = w.task_for_windows(&team, "Windows build").await;

    // That computer makes it wait (its account hit a limit), and says why.
    scheduler::note_rate_limited(&w.b.st, "default");
    let e = launcher::start_role(&w.a.st, &team, "windows", Some(task), &[])
        .await
        .unwrap_err();
    assert!(e.to_string().contains("WIN-BOX"), "{e}");
    let (s, v) = w.a.call("GET", "/capacity", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let rows = v["remoteRoles"].as_array().expect("its roles elsewhere");
    assert_eq!(rows.len(), 1, "{v:#}");
    let r = &rows[0];
    assert_eq!(r["role"], "windows");
    assert_eq!(r["roleTitle"], "Windows");
    assert_eq!(r["team"], team.as_str());
    assert_eq!(r["state"], "waiting", "{r:#}");
    assert!(
        r["reasons"][0]
            .as_str()
            .is_some_and(|x| x.starts_with("on WIN-BOX: ")),
        "that computer's words: {r:#}"
    );
    assert_eq!(v["caps"]["workersRunning"], 0);

    // Admitted there: it runs there, listed here, counted there only.
    w.b.st.scheduler.note_rate(RateLimitNote {
        at: chrono::Utc::now().timestamp_millis() + 1,
        account: "default".into(),
        status: "allowed".into(),
        limit: None,
        resets_at: None,
        pct: Some(10.0),
        source: "stream",
    });
    let session = launcher::start_role(&w.a.st, &team, "windows", Some(task), &[])
        .await
        .expect("started there");
    let (_, v) = w.a.call("GET", "/capacity", None).await;
    let r = &v["remoteRoles"][0];
    assert_eq!(r["state"], "running", "{r:#}");
    assert_eq!(r["machineName"], "WIN-BOX");
    assert_eq!(r["machine"], w.b.id().as_str());
    assert_eq!(r["session"], session.as_str());
    assert_eq!(r["reasons"], json!([]));
    assert_eq!(v["caps"]["workersRunning"], 0, "never counted here");
    assert!(v["running"].as_array().unwrap().is_empty(), "{v:#}");
    // That computer leads no team of its own: nothing listed there.
    let (_, there) = w.b.call("GET", "/capacity", None).await;
    assert!(
        there.get("remoteRoles").is_none(),
        "none of its own elsewhere"
    );
    launcher::stop_role(&w.a.st, &team, "windows", "done").await;
}
