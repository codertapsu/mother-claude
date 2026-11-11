//! The setup's rest (docs/PLAN-MACHINES.md §5.14, Phase 3a): two app states
//! in one process, each with its own data dir, Claude home and iroh
//! endpoint, paired through the real dashboard API (as `tests/machines.rs`
//! does). Accounts and repositories' origins cross between them; a root is
//! proposed from the origins both have; a clone asked of the other computer
//! runs only when someone there confirms it (from a local bare repository,
//! never a real remote); the code options are offered and taken only when
//! accepted; the readiness test runs its git round trip both ways.
//!
//! `claude` is a stub that only answers `--version` and `auth status`, and
//! `CLAUDE_CONFIG_DIR` points every `.claude.json` read into the test homes:
//! nothing here reads or writes the real `~/.claude`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::{AuthStatus, ClaudeHome};
use mother_claude_lib::machines::{self, code::GitRunner, transport};
use mother_claude_lib::server::{self, auth::Auth};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

/// The process-wide part, once: a stub `claude` and a `CLAUDE_CONFIG_DIR`,
/// so each test home's `.claude.json` is its own.
fn isolate() {
    static ONCE: OnceLock<PathBuf> = OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        let stub = if cfg!(windows) {
            let p = dir.join("claude-stub.cmd");
            std::fs::write(
                &p,
                "@echo off\r\nif \"%~1\"==\"--version\" goto version\r\nif \"%~1\"==\"auth\" goto auth\r\nexit /b 1\r\n:version\r\necho 2.1.330 (Claude Code)\r\nexit /b 0\r\n:auth\r\necho {\"loggedIn\":true,\"authMethod\":\"claude.ai\"}\r\nexit /b 0\r\n",
            )
            .unwrap();
            p
        } else {
            let p = dir.join("claude-stub.sh");
            std::fs::write(
                &p,
                "#!/bin/sh\ncase \"$1\" in\n  --version) echo '2.1.330 (Claude Code)';;\n  auth) echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}';;\n  *) exit 1;;\nesac\n",
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            p
        };
        std::env::set_var("MOTHER_CLAUDE_CLI", &stub);
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
    // Signed in, as the setup would have read it a moment ago.
    st.accounts.runtime.set_auth(
        "default",
        AuthStatus {
            logged_in: true,
            ..AuthStatus::default()
        },
    );
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
        let req = req.bearer_auth(self.st.auth.token.clone());
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let v = resp.json::<Value>().await.unwrap_or(Value::Null);
        (status, v)
    }

    async fn get(&self, path: &str) -> Value {
        let (s, v) = self.call("GET", path, None).await;
        assert_eq!(s, StatusCode::OK, "GET {path}: {v}");
        v
    }

    /// Polls `GET path` until `pred` holds (45 s at most).
    async fn wait(&self, path: &str, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let v = self.get(path).await;
            if pred(&v) {
                return v;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {v:#}"
            );
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    fn id(&self) -> String {
        machines::machine_info(&self.st).id
    }

    /// A folder of this test computer (created).
    fn folder(&self, parts: &[&str]) -> PathBuf {
        let mut p = self.dir.path().to_path_buf();
        for part in parts {
            p.push(part);
        }
        std::fs::create_dir_all(&p).unwrap();
        dunce::canonicalize(&p).unwrap()
    }
}

fn state_of(v: &Value) -> &str {
    v["pairing"]["state"].as_str().unwrap_or("")
}

/// Pairs `a` (host) and `b` (joiner); each lets the other see, drive and
/// move on the local network.
async fn pair(a: &Machine, b: &Machine) {
    let g = json!({
        "view": {"lan": true, "internet": true},
        "control": {"lan": true, "internet": true},
        "move": {"lan": true, "internet": true},
        "run": {"lan": false, "internet": false},
        "dangerous": {"lan": false, "internet": false}
    });
    let (s, v) = a.call("POST", "/machines/pair", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let code = v["pairing"]["code"].as_str().expect("a code").to_string();
    let (s, v) = b
        .call(
            "POST",
            "/machines/join",
            Some(json!({ "code": code, "name": "Joiner" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    b.wait("/machines", "the joiner's digits", |v| {
        state_of(v) == "confirm"
    })
    .await;
    a.wait("/machines", "the host's digits", |v| {
        state_of(v) == "confirm"
    })
    .await;
    let (s, v) = b
        .call("POST", "/machines/confirm", Some(json!({ "grants": g })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = a
        .call("POST", "/machines/confirm", Some(json!({ "grants": g })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    b.wait("/machines", "the joiner to finish", |v| {
        state_of(v) == "paired"
    })
    .await;
    // Both online, as the hello says.
    a.wait("/machines", "B online", |v| v["peers"][0]["online"] == true)
        .await;
    b.wait("/machines", "A online", |v| v["peers"][0]["online"] == true)
        .await;
}

/// Runs git (the tests' own, isolated from this computer's configuration).
fn git(cwd: &Path, args: &[&str]) {
    let op = GitRunner::new().isolated().begin().unwrap();
    op.git(cwd, args)
        .unwrap_or_else(|e| panic!("git {}: {e}", args.join(" ")));
}

/// A bare repository with one commit on `main`; its `file://` URL.
fn bare(base: &Path, name: &str) -> String {
    let remote = base.join(format!("{name}.git"));
    let work = base.join(format!("{name}-seed"));
    std::fs::create_dir_all(&work).unwrap();
    git(base, &["init", "--bare", "-q", remote.to_str().unwrap()]);
    git(&work, &["init", "-q"]);
    std::fs::write(work.join("README.md"), format!("# {name}\n")).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "first"]);
    git(
        &work,
        &[
            "push",
            "-q",
            remote.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    );
    url::Url::from_file_path(dunce::canonicalize(&remote).unwrap())
        .unwrap()
        .to_string()
}

/// A repository at `path` whose origin is `origin` (no commit needed).
fn repo(path: &Path, origin: &str) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q"]);
    git(path, &["remote", "add", "origin", origin]);
}

fn names(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|l| {
            l.iter()
                .filter_map(|x| x["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accounts_and_origins_cross_and_propose_a_root() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b).await;
    let (ida, idb) = (a.id(), b.id());

    // Accounts: Work on both, signed in on A only; Side on B only.
    let work_a =
        a.st.accounts
            .add_directory("Work", &a.dir.path().join("cfg-work"), None)
            .unwrap();
    a.st.accounts.runtime.set_auth(
        &work_a.id,
        AuthStatus {
            logged_in: true,
            email: Some("someone@example.test".into()),
            ..AuthStatus::default()
        },
    );
    let work_b =
        b.st.accounts
            .add_directory("work", &b.dir.path().join("cfg-work"), None)
            .unwrap();
    b.st.accounts
        .runtime
        .set_auth(&work_b.id, AuthStatus::default());
    let side_b =
        b.st.accounts
            .add_directory("Side", &b.dir.path().join("cfg-side"), None)
            .unwrap();
    b.st.accounts.runtime.set_auth(
        &side_b.id,
        AuthStatus {
            logged_in: true,
            ..AuthStatus::default()
        },
    );

    // What B tells another computer: names, kinds, sign-ins — nothing else.
    let (s, raw) = transport::request(
        &a.st,
        &idb,
        "GET",
        "/peer/accounts",
        None,
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(s, 200, "{raw}");
    for acc in raw["accounts"].as_array().unwrap() {
        let mut keys: Vec<&str> = acc
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(keys, ["kind", "name", "state"], "{acc}");
    }
    let text = raw.to_string();
    assert!(!text.contains("cfg-work") && !text.contains("acc_") && !text.contains('@'));

    let v = a.get("/machines/accounts").await;
    assert_eq!(v["machine"], ida.as_str());
    assert_eq!(names(&v["accounts"]), ["Default", "Work", "Side"]);
    let row = |n: &str| {
        v["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == n)
            .unwrap()
            .clone()
    };
    assert_eq!(row("Default")["cells"][&idb]["state"], "signedIn");
    assert_eq!(row("Work")["cells"][&ida]["state"], "signedIn");
    assert_eq!(
        row("Work")["cells"][&idb]["state"],
        "signedOut",
        "matched by name"
    );
    assert_eq!(row("Work")["cells"][&idb]["name"], "work");
    assert_eq!(row("Side")["cells"][&ida]["state"], "missing");
    assert!(
        !v.to_string().contains("someone@example.test"),
        "never an email"
    );

    // The setup's checks: Work's sign-in on B blocks Work's sessions only.
    let setup = b.get("/machines/setup?fresh=true").await;
    let work_check = setup["self"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == format!("account:{}", work_b.id))
        .expect("an account check")
        .clone();
    assert_eq!(work_check["state"], "blocker");
    assert_eq!(work_check["scope"], "account");
    assert_eq!(work_check["title"], "work");
    let gating = setup["gating"].as_u64().unwrap();
    let blockers = setup["blockers"].as_u64().unwrap();
    assert!(
        blockers > gating,
        "a scoped blocker does not gate: {setup:#}"
    );
    let git_check = setup["self"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "git")
        .unwrap()
        .clone();
    assert_eq!(git_check["state"], "ok", "{git_check}");
    let claude_check = setup["self"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "claudeCode")
        .unwrap()
        .clone();
    assert_eq!(claude_check["detail"], "Claude Code 2.1.330", "the stub's");

    // Repositories: `app` on both (one origin, two spellings of it), `side`
    // on A only, `win` on B only, `notes` on A without a remote. Each
    // computer has its folder as a root naming only itself so far: the
    // folders right under a root are looked at.
    let remotes = a.folder(&["remotes"]);
    let app_url = bare(&remotes, "app");
    let side_url = bare(&remotes, "side");
    let win_url = bare(&remotes, "win");
    let pa = a.folder(&["projects"]);
    let pb = b.folder(&["work", "projects"]);
    repo(&pa.join("app"), &app_url);
    repo(&pa.join("side"), &side_url);
    repo(&pb.join("app"), &format!("{app_url}/"));
    repo(&pb.join("win"), &win_url);
    std::fs::create_dir_all(pa.join("notes").join(".git")).unwrap();
    let (s, v) = a
        .call(
            "PUT",
            "/roots",
            Some(json!({ "upsert": [{ "name": "projects", "paths": { &ida: pa } }] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let root_id = v["roots"][0]["id"].as_str().unwrap().to_string();
    let (s, v) = b
        .call(
            "PUT",
            "/roots",
            Some(json!({ "upsert": [{ "name": "pc", "paths": { &idb: pb } }] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    a.wait("/roots", "both roots on A", |v| {
        names(&v["roots"]).len() == 2
    })
    .await;

    let v = a.get("/machines/folders?fresh=true").await;
    assert_eq!(v["machine"], ida.as_str());
    assert_eq!(names(&v["noOrigin"]), ["notes"]);
    let with_b = &v["peers"][0];
    assert_eq!(with_b["machine"], idb.as_str());
    assert_eq!(with_b["error"], Value::Null, "{with_b:#}");
    let proposals = with_b["proposals"].as_array().unwrap();
    assert_eq!(proposals.len(), 1, "{with_b:#}");
    let p = &proposals[0];
    assert_eq!(p["here"], pa.to_string_lossy().as_ref());
    assert_eq!(p["there"], pb.to_string_lossy().as_ref());
    assert_eq!(p["root"], root_id.as_str(), "the root naming it here grows");
    assert_eq!(p["name"], "projects");
    assert_eq!(names(&p["repos"]), ["app"]);
    assert_eq!(names(&with_b["onlyHere"]), ["side"]);
    assert_eq!(
        with_b["onlyHere"][0]["root"],
        Value::Null,
        "no root maps it yet"
    );
    assert_eq!(names(&with_b["onlyThere"]), ["win"]);

    // Accepting the proposal: B's folder joins the root, on both; the
    // projects map.
    let (s, roots) = a
        .call(
            "PUT",
            "/roots",
            Some(json!({ "upsert": [{ "id": root_id, "name": p["name"], "paths": { &ida: p["here"], &idb: p["there"] } }] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{roots}");
    b.wait("/roots", "the root on B", |v| {
        v["roots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == root_id.as_str() && r["paths"].get(&idb).is_some())
    })
    .await;
    let v = a.get("/machines/folders?fresh=true").await;
    let with_b = &v["peers"][0];
    assert!(
        with_b["proposals"].as_array().unwrap().is_empty(),
        "{with_b:#}"
    );
    assert_eq!(names(&with_b["matched"]), ["app"]);
    let side = &with_b["onlyHere"][0];
    assert_eq!(side["root"], root_id.as_str());
    assert_eq!(side["rel"], "side");
    assert_eq!(
        side["mapped"],
        pb.join("side").to_string_lossy().as_ref(),
        "where a clone lands on B"
    );
    let win = &with_b["onlyThere"][0];
    assert_eq!(win["rel"], "win");
    assert_eq!(win["mapped"], pa.join("win").to_string_lossy().as_ref());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_asked_clone_runs_only_when_confirmed_there() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b).await;
    let (ida, idb) = (a.id(), b.id());
    let remotes = a.folder(&["remotes"]);
    let side_url = bare(&remotes, "side");
    let pa = a.folder(&["projects"]);
    let pb = b.folder(&["projects"]);
    let (s, roots) = a
        .call(
            "PUT",
            "/roots",
            Some(json!({ "upsert": [{ "name": "projects", "paths": { &ida: pa, &idb: pb } }] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{roots}");
    let root = roots["roots"][0]["id"].as_str().unwrap().to_string();
    b.wait("/roots", "the root on B", |v| {
        names(&v["roots"]) == ["projects"]
    })
    .await;

    // Refused before anything is kept: a folder outside the root, a remote
    // with a password, an account B does not have.
    for (body, reason) in [
        (
            json!({ "machine": idb, "check": "clone", "origin": side_url, "root": root, "rel": "../escape" }),
            "machines_bad_folder",
        ),
        (
            json!({ "machine": idb, "check": "clone", "origin": "https://user:secret@example.invalid/x.git", "root": root, "rel": "x" }),
            "machines_bad_origin",
        ),
        (
            json!({ "machine": idb, "check": "signIn", "account": "Nobody" }),
            "machines_unknown_account",
        ),
    ] {
        let (s, v) = a.call("POST", "/machines/setup/ask", Some(body)).await;
        assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
        assert_eq!(v["error"]["reason"], reason, "{v}");
    }
    assert!(b.get("/machines").await["asks"]
        .as_array()
        .unwrap()
        .is_empty());

    // Asked: kept and shown on B, nothing cloned.
    let (s, v) = a
        .call(
            "POST",
            "/machines/setup/ask",
            Some(json!({ "machine": idb, "check": "clone", "origin": side_url, "root": root, "rel": "side" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ask_id = v["ask"]["id"].as_str().expect("the ask").to_string();
    assert_eq!(v["ask"]["state"], "asked");
    let target = pb.join("side");
    assert_eq!(v["ask"]["folder"], target.to_string_lossy().as_ref());
    let on_b = b.get("/machines").await;
    let asks = on_b["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1);
    assert_eq!(asks[0]["kind"], "clone");
    assert_eq!(asks[0]["from"], ida.as_str());
    assert_eq!(asks[0]["state"], "asked");
    assert!(!target.exists(), "nothing runs because A asked");
    assert!(b.st.machines.recorded_fixes().is_empty());
    // A keeps what it asked, for its screens.
    let setup = a.get("/machines/setup").await;
    assert_eq!(setup["peers"][0]["asks"][0]["id"], ask_id.as_str());

    // A peer cannot answer it: that is B's desktop's.
    let (s, _) = transport::request(
        &a.st,
        &idb,
        "POST",
        &format!("/api/machines/asks/{ask_id}"),
        Some(json!({ "answer": "confirm" })),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(s, 404, "not a route another computer reaches");
    assert!(!target.exists());

    // Confirmed on B: cloned, recorded, and A is told.
    let (s, v) = b
        .call(
            "POST",
            &format!("/machines/asks/{ask_id}"),
            Some(json!({ "answer": "confirm" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ask"]["state"], "done", "{v}");
    assert!(target.join(".git").exists());
    assert!(target.join("README.md").exists());
    assert!(b
        .st
        .machines
        .recorded_fixes()
        .iter()
        .any(|f| f.starts_with("clone ") && f.ends_with(&target.to_string_lossy().to_string())));
    a.wait("/machines/setup", "A told the clone is done", |v| {
        v["peers"][0]["asks"].as_array().is_some_and(|l| {
            l.iter()
                .any(|x| x["id"] == ask_id.as_str() && x["state"] == "done")
        })
    })
    .await;
    let (s, _) = b
        .call(
            "POST",
            &format!("/machines/asks/{ask_id}"),
            Some(json!({ "answer": "confirm" })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "answered once");

    // A second clone into the same folder is refused when asked.
    let (s, v) = a
        .call(
            "POST",
            "/machines/setup/ask",
            Some(json!({ "machine": idb, "check": "clone", "origin": side_url, "root": root, "rel": "side" })),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
    assert_eq!(v["error"]["reason"], "machines_folder_taken");

    // Declined: nothing happens there, and A is told.
    let (s, v) = a
        .call(
            "POST",
            "/machines/setup/ask",
            Some(json!({ "machine": idb, "check": "addAccount", "account": "Client X" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let add_id = v["ask"]["id"].as_str().unwrap().to_string();
    let (s, v) = b
        .call(
            "POST",
            &format!("/machines/asks/{add_id}"),
            Some(json!({ "answer": "decline" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ask"]["state"], "declined");
    assert!(b
        .st
        .accounts
        .accounts()
        .iter()
        .all(|x| x.name != "Client X"));
    a.wait("/machines/setup", "A told it was declined", |v| {
        v["peers"][0]["asks"].as_array().is_some_and(|l| {
            l.iter()
                .any(|x| x["id"] == add_id.as_str() && x["state"] == "declined")
        })
    })
    .await;

    // A sign-in asked and confirmed is only recorded until the app turns
    // fixes on: no `claude auth login`, no browser.
    let (s, v) = a
        .call(
            "POST",
            "/machines/setup/ask",
            Some(json!({ "machine": idb, "check": "signIn", "account": "default" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let sign_id = v["ask"]["id"].as_str().unwrap().to_string();
    assert!(!b
        .st
        .machines
        .recorded_fixes()
        .iter()
        .any(|f| f.starts_with("sign in")));
    let (s, v) = b
        .call(
            "POST",
            &format!("/machines/asks/{sign_id}"),
            Some(json!({ "answer": "confirm" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["next"], "/accounts");
    assert!(b
        .st
        .machines
        .recorded_fixes()
        .contains(&"sign in Default".to_string()));

    // *Clone it here*: B's own desktop clones into a root's folder at once.
    let win_url = bare(&remotes, "win");
    let (s, v) = b
        .call(
            "POST",
            "/machines/clone",
            Some(json!({ "origin": win_url, "root": root, "rel": "win" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(pb.join("win").join("README.md").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_options_are_offered_and_taken_only_when_accepted() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b).await;

    let v = a.get("/machines/settings").await;
    assert_eq!(
        v["code"],
        json!({ "transfer": "bundle", "apply": "checkout", "keepRemoteRefs": false })
    );
    let (s, v) = a
        .call(
            "PUT",
            "/machines/settings",
            Some(json!({ "code": { "transfer": "remote", "apply": "worktree" } })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["code"]["transfer"], "remote");
    let (s, v) = a
        .call(
            "PUT",
            "/machines/settings",
            Some(json!({ "code": { "transfer": "ftp" } })),
        )
        .await;
    assert!(s.is_client_error(), "{v}");

    // B is offered it, and keeps its own until accepted there.
    let vb = b
        .wait("/machines", "the offer on B", |v| !v["offer"].is_null())
        .await;
    assert_eq!(vb["offer"]["code"]["transfer"], "remote");
    assert_eq!(vb["settings"]["code"]["transfer"], "bundle");
    let (s, v) = b
        .call(
            "POST",
            "/machines/settings/offer",
            Some(json!({ "accept": true })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["settings"]["code"]["transfer"], "remote");
    assert_eq!(v["settings"]["code"]["apply"], "worktree");
    assert!(v["offer"].is_null());

    // A project's own options and recipe, kept by its root.
    let pa = a.folder(&["projects"]);
    let pb = b.folder(&["projects"]);
    let (ida, idb) = (a.id(), b.id());
    let (s, v) = a
        .call(
            "PUT",
            "/roots",
            Some(json!({ "upsert": [{ "name": "projects", "paths": { &ida: pa, &idb: pb } }] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let app = pa.join("app");
    let (s, v) = a
        .call(
            "PUT",
            "/machines/settings",
            Some(json!({ "projects": { "upsert": [{
                "project": app,
                "code": { "apply": "checkout" },
                "recipe": { "afterMove": { "windows": ["npm ci"] }, "carryIgnored": [".env.local"] }
            }] } })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let p = &v["projects"][0];
    assert!(p["key"].as_str().unwrap().ends_with("/app"), "{p}");
    assert!(p["key"].as_str().unwrap().starts_with("rt_"), "{p}");
    assert_eq!(p["project"], app.to_string_lossy().as_ref());
    assert_eq!(p["recipe"]["carryIgnored"], json!([".env.local"]));
    let eff = machines::setup::options::effective(&a.st, &app.to_string_lossy());
    assert_eq!(eff.code.apply.as_str(), "checkout", "its own");
    assert_eq!(eff.code.transfer.as_str(), "remote", "the default");
    // A bad recipe changes nothing.
    let (s, v) = a
        .call(
            "PUT",
            "/machines/settings",
            Some(json!({ "projects": { "upsert": [{ "project": app, "recipe": { "carryIgnored": ["../x"] } }] } })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(
        a.get("/machines/settings").await["projects"][0]["recipe"]["carryIgnored"],
        json!([".env.local"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_readiness_test_moves_code_both_ways() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b).await;
    let idb = b.id();
    // The tools are read by the checks.
    a.get("/machines/setup?fresh=true").await;
    b.get("/machines/setup?fresh=true").await;

    let (s, v) = a
        .call("POST", "/machines/test", Some(json!({ "machine": idb })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let lines = v["results"][0]["tests"].as_array().unwrap();
    let ids: Vec<&str> = lines.iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        ["connect", "transfer", "tools", "code", "accounts", "folders"]
    );
    let line = |id: &str| lines.iter().find(|l| l["id"] == id).unwrap().clone();
    assert_eq!(line("connect")["state"], "ok", "{v:#}");
    assert_eq!(line("tools")["state"], "ok", "{v:#}");
    assert_eq!(
        line("tools")["detail"],
        "git and Claude Code 2.1.330 on both."
    );
    assert_eq!(line("code")["state"], "ok", "{v:#}");
    assert!(line("code")["detail"]
        .as_str()
        .unwrap()
        .starts_with("Both ways in "));
    assert_eq!(line("accounts")["state"], "ok", "{v:#}");
    assert_eq!(
        line("folders")["state"],
        "warning",
        "nothing maps yet: {v:#}"
    );
    // Scratch folders only, and gone after.
    for m in [&a, &b] {
        let scratch = m.dir.path().join("data").join("machines").join("test");
        let left: Vec<_> = std::fs::read_dir(&scratch)
            .map(|d| d.flatten().collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "{left:?}");
    }
}
