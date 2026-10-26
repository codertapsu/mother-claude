//! Finding conversations on disk, over real HTTP (docs/ACCOUNTS.md §14): a
//! session's files in its detail, every project folder of every account, and
//! showing them in the file manager (only ever recorded here: no test opens
//! a window). Every directory is a test's temp directory; the Default's
//! `.claude.json` is inside it too.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use mother_claude_lib::claude::reveal::{command, Platform, Target};
use mother_claude_lib::claude::secrets::Secret;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

struct Fixture {
    st: AppState,
    base: String,
    home: tempfile::TempDir,
    scratch: tempfile::TempDir,
}

/// A Default whose `.claude.json` is inside its temp directory (never the
/// real `~/.claude.json`), a data dir of its own, a bridge that binds no port
/// of its own, and a file manager that is only recorded.
async fn boot() -> Fixture {
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(scratch.path().join("app-data"));
    let st = Inner::with_bridge_config(
        ClaudeHome::for_account_dir(home.path().to_path_buf()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
        mother_claude_lib::bridge::BridgeConfig {
            enabled: true,
            port: None,
            token: mother_claude_lib::bridge::TokenPolicy::Required,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    st.reveal.record_only();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::router_for(st.clone(), Ingress::Loopback);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Fixture {
        st,
        base: format!("http://{addr}"),
        home,
        scratch,
    }
}

const TOKEN: &str = "sk-ant-oat01-PastedTokenValue0123456789abcdefghijABCDEFGHIJ";

impl Fixture {
    fn desktop(&self) -> String {
        self.st.auth.token.clone()
    }

    fn script(&self) -> String {
        self.st.auth.script_token.clone()
    }

    /// A paired phone (on the LAN), and its device id.
    fn phone(&self) -> (String, String) {
        let (d, t) = self.st.devices.issue(
            "Kitchen phone",
            "ios",
            None,
            Ingress::Lan,
            "192.168.1.60".parse().unwrap(),
        );
        (d.id, t)
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let c = reqwest::Client::new();
        let url = format!("{}/api{path}", self.base);
        let req = match method {
            "GET" => c.get(url),
            "POST" => c.post(url),
            other => panic!("{other}"),
        }
        .bearer_auth(token);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        let r = req.send().await.unwrap();
        let status = r.status();
        let text = r.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// A directory account, and its directory as the registry keeps it.
    fn account_dir(&self, name: &str) -> (String, PathBuf) {
        let dir = self.scratch.path().join(format!("claude-{name}"));
        let a = self.st.accounts.add_directory(name, &dir, None).unwrap();
        (a.id, a.dir.unwrap())
    }

    /// A project folder that exists.
    fn cwd(&self, name: &str) -> String {
        let cwd = self.scratch.path().join(name);
        std::fs::create_dir_all(&cwd).unwrap();
        cwd.to_string_lossy().into_owned()
    }

    async fn refresh(&self) {
        mother_claude_lib::server::monitor::refresh_once(&self.st).await;
    }
}

fn transcript(cwd: &str, sid: &str) -> String {
    let user = json!({
        "type": "user", "sessionId": sid, "cwd": cwd, "uuid": "u-1",
        "timestamp": "2026-10-21T09:00:00.000Z",
        "message": {"role": "user", "content": "hello"}
    });
    let assistant = json!({
        "type": "assistant", "sessionId": sid, "cwd": cwd, "uuid": "a-1", "parentUuid": "u-1",
        "timestamp": "2026-10-21T09:00:05.000Z",
        "message": {"id": "msg_1", "role": "assistant", "model": "claude-haiku-4-5",
                    "content": [{"type": "text", "text": "hi"}],
                    "usage": {"input_tokens": 1, "output_tokens": 2}}
    });
    let title = json!({"type": "ai-title", "aiTitle": format!("Talk {}", &sid[..4])});
    format!("{user}\n{assistant}\n{title}\n")
}

fn project_dir(home: &Path, cwd: &str) -> PathBuf {
    home.join("projects")
        .join(mother_claude_lib::claude::encode_cwd(cwd))
}

fn transcript_path(home: &Path, cwd: &str, sid: &str) -> PathBuf {
    project_dir(home, cwd).join(format!("{sid}.jsonl"))
}

/// A conversation with every kind of file it can have (§1.4): its
/// transcript, a subagent, a tool result, file history, an empty
/// session-env directory and a todo list.
fn write_session(home: &Path, cwd: &str, sid: &str) {
    let t = transcript_path(home, cwd, sid);
    let own = t.parent().unwrap().join(sid);
    std::fs::create_dir_all(own.join("subagents")).unwrap();
    std::fs::create_dir_all(own.join("tool-results")).unwrap();
    std::fs::write(&t, transcript(cwd, sid)).unwrap();
    std::fs::write(own.join("subagents/agent-a1.jsonl"), "{}\n").unwrap();
    std::fs::write(own.join("tool-results/r1.txt"), "output").unwrap();
    std::fs::create_dir_all(home.join("file-history").join(sid)).unwrap();
    std::fs::write(home.join("file-history").join(sid).join("snap@v1"), "old").unwrap();
    std::fs::create_dir_all(home.join("session-env").join(sid)).unwrap();
    std::fs::create_dir_all(home.join("todos")).unwrap();
    std::fs::write(
        home.join("todos").join(format!("{sid}-agent-{sid}.json")),
        "[]",
    )
    .unwrap();
}

/// A pidfile of this very process (alive) running `sid` in `home`.
fn run_here(home: &Path, cwd: &str, sid: &str) {
    let pid = std::process::id();
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    std::fs::write(
        home.join("sessions").join(format!("{pid}.json")),
        json!({"pid": pid, "sessionId": sid, "cwd": cwd, "status": "idle",
               "kind": "interactive", "entrypoint": "cli"})
        .to_string(),
    )
    .unwrap();
}

const S1: &str = "11111111-1111-4111-8111-111111111111";
const S2: &str = "22222222-2222-4222-8222-222222222222";
const S3: &str = "33333333-3333-4333-8333-333333333333";
const S4: &str = "44444444-4444-4444-8444-444444444444";
const S5: &str = "55555555-5555-4555-8555-555555555555";

/// §14.2: one row per (account directory, recorded folder) — a token
/// account's rows being the shared folder's conversations attributed to it —
/// with counts, what runs, sizes from the index and the newest first; read by
/// anyone signed in, the local-script token included.
#[tokio::test]
async fn every_project_of_every_account_from_the_index() {
    let f = boot().await;
    let proj = f.cwd("proj");
    let other = f.cwd("other");
    let (work, work_dir) = f.account_dir("Work");
    let pro =
        f.st.accounts
            .add_token("Pro", Some(&Secret::new(TOKEN)), None)
            .unwrap()
            .id;
    write_session(f.home.path(), &proj, S1);
    write_session(f.home.path(), &proj, S2);
    write_session(f.home.path(), &other, S3);
    write_session(&work_dir, &proj, S4);
    // S5 is in the shared folder, attributed to Pro (it ran under it).
    write_session(f.home.path(), &proj, S5);
    f.st.accounts.remember_session(S5, &pro);
    // S2 runs (this process's pid is alive).
    run_here(f.home.path(), &proj, S2);
    f.refresh().await;

    let (s, v) = f.call("GET", "/data/projects", &f.desktop(), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let rows = v["projects"].as_array().unwrap().clone();
    let row = |account: &str, cwd: &str| {
        rows.iter()
            .find(|r| r["account"] == account && r["cwd"] == cwd)
            .cloned()
            .unwrap_or_else(|| panic!("no row ({account}, {cwd}): {rows:?}"))
    };
    assert_eq!(rows.len(), 4, "{rows:?}");
    let shared = row("default", &proj);
    assert_eq!(shared["sessions"], 2, "S1 and S2, not Pro's S5");
    assert_eq!(shared["live"], 1, "S2 runs");
    assert_eq!(shared["name"], "proj");
    assert_eq!(
        shared["folder"],
        json!(project_dir(f.home.path(), &proj).to_string_lossy())
    );
    let one = std::fs::metadata(transcript_path(f.home.path(), &proj, S1))
        .unwrap()
        .len()
        + 3; // the subagent's "{}\n"
    assert_eq!(shared["bytes"], json!(2 * one), "transcripts and subagents");
    assert!(shared["lastActivityAt"].as_i64().unwrap() > 0);
    assert_eq!(row("default", &other)["sessions"], 1);
    let w = row(&work, &proj);
    assert_eq!(w["sessions"], 1);
    assert_eq!(w["live"], 0);
    assert_eq!(
        w["folder"],
        json!(project_dir(&work_dir, &proj).to_string_lossy())
    );
    assert_eq!(row(&pro, &proj)["sessions"], 1, "Pro's own row");
    let times: Vec<i64> = rows
        .iter()
        .map(|r| r["lastActivityAt"].as_i64().unwrap())
        .collect();
    assert!(times.windows(2).all(|w| w[0] >= w[1]), "newest first");

    let (_, phone) = f.phone();
    for token in [phone, f.script()] {
        let (s, w) = f.call("GET", "/data/projects", &token, None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(w, v, "the same for a phone and a local script");
    }
}

/// §14.1: the detail names the conversation's files; the desktop alone shows
/// the transcript or a project's folder in the file manager, by paths from
/// the index.
#[tokio::test]
async fn a_sessions_files_and_showing_them() {
    let f = boot().await;
    let proj = f.cwd("proj");
    let (work, work_dir) = f.account_dir("Work");
    write_session(&work_dir, &proj, S1);
    write_session(f.home.path(), &proj, S2);
    // Only a file history and no folder of its own.
    std::fs::remove_dir_all(project_dir(f.home.path(), &proj).join(S2)).unwrap();
    // S3 runs but has written no transcript yet.
    run_here(f.home.path(), &proj, S3);
    f.refresh().await;

    let (s, v) = f
        .call("GET", &format!("/sessions/{S1}"), &f.desktop(), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let t = transcript_path(&work_dir, &proj, S1);
    assert_eq!(
        v["files"],
        json!({
            "transcript": t.to_string_lossy(),
            "folder": t.parent().unwrap().join(S1).to_string_lossy(),
            "fileHistory": work_dir.join("file-history").join(S1).to_string_lossy(),
            "account": work,
        })
    );
    let (_, v) = f
        .call("GET", &format!("/sessions/{S2}"), &f.desktop(), None)
        .await;
    assert!(v["files"].get("folder").is_none(), "{v}");
    assert_eq!(v["files"]["account"], "default");
    let (_, v) = f
        .call("GET", &format!("/sessions/{S3}"), &f.desktop(), None)
        .await;
    assert_eq!(v["id"], S3);
    assert!(v.get("files").is_none(), "no transcript, no files: {v}");

    // Showing the transcript: the desktop only.
    let (s, v) = f
        .call(
            "POST",
            &format!("/sessions/{S1}/reveal"),
            &f.desktop(),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(
        f.st.reveal.recorded(),
        [command(Platform::current(), &Target::File(t.clone()))]
    );
    let (_, phone) = f.phone();
    for token in [phone.as_str(), f.script().as_str()] {
        let (s, v) = f
            .call("POST", &format!("/sessions/{S1}/reveal"), token, None)
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
        assert_eq!(v["error"]["code"], "desktop_only");
    }
    let missing = "99999999-9999-4999-8999-999999999999";
    let (s, v) = f
        .call(
            "POST",
            &format!("/sessions/{missing}/reveal"),
            &f.desktop(),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "session_not_found");
    let (s, v) = f
        .call(
            "POST",
            &format!("/sessions/{S3}/reveal"),
            &f.desktop(),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["error"]["code"], "transcript_not_found");
    let (s, v) = f
        .call("POST", "/sessions/..%2F..%2Fetc/reveal", &f.desktop(), None)
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // A project's folder, by a row of the listing.
    let (s, v) = f
        .call(
            "POST",
            "/data/projects/reveal",
            &f.desktop(),
            Some(json!({"account": work, "cwd": format!("{proj}/")})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(
        f.st.reveal.recorded()[1],
        command(
            Platform::current(),
            &Target::Folder(project_dir(&work_dir, &proj))
        )
    );
    for body in [
        json!({"account": work, "cwd": f.cwd("elsewhere")}),
        json!({"account": "acc_000000000000", "cwd": proj}),
        json!({"account": work, "cwd": "proj"}),
    ] {
        let (s, v) = f
            .call("POST", "/data/projects/reveal", &f.desktop(), Some(body))
            .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
        assert_eq!(v["error"]["code"], "project_not_found");
    }
    let (s, v) = f
        .call(
            "POST",
            "/data/projects/reveal",
            &phone,
            Some(json!({"account": work, "cwd": proj})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(f.st.reveal.recorded().len(), 2, "nothing else was shown");
}
