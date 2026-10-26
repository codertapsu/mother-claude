//! Finding conversations on disk and deleting them in bulk, over real HTTP
//! (docs/ACCOUNTS.md §14): a session's files in its detail, every project
//! folder of every account, showing them in the file manager (only ever
//! recorded here: no test opens a window), and `POST /api/data/delete` with
//! its jobs, skips, undo and guards. Every directory is a test's temp
//! directory; the Default's `.claude.json` is inside it too.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mother_claude_lib::claude::reveal::{command, Platform, Target};
use mother_claude_lib::claude::secrets::Secret;
use mother_claude_lib::claude::summaries::session_account;
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

    /// `POST /api/data/delete` from the desktop.
    async fn delete(&self, body: Value) -> (StatusCode, Value) {
        self.call("POST", "/data/delete", &self.desktop(), Some(body))
            .await
    }

    async fn finished(&self, job: &str) -> Value {
        for _ in 0..400 {
            let (_, j) = self
                .call("GET", &format!("/data/jobs/{job}"), &self.desktop(), None)
                .await;
            if j["state"] != "running" {
                return j;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("job {job} did not finish");
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

/// Every file under `dir`, relative, with its bytes.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                walk(root, &p, out);
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&p).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

type Bus = tokio::sync::broadcast::Receiver<std::sync::Arc<mother_claude_lib::state::ServerEvent>>;

/// The notices the desktop was shown so far.
fn desktop_notices(rx: &mut Bus) -> Vec<String> {
    use mother_claude_lib::state::ServerEvent;
    use tokio::sync::broadcast::error::TryRecvError;
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => {
                if let ServerEvent::DesktopNotice { text, .. } = &*ev {
                    out.push(text.clone());
                }
            }
            Err(TryRecvError::Lagged(_)) => continue,
            Err(_) => return out,
        }
    }
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

/// A conversation's plan rows' and skips' ids and codes.
fn ids_of(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn codes_of(list: &Value) -> Vec<(String, String)> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["id"]
                    .as_str()
                    .or(s["cwd"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                s["code"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// §14.3: the dry run lists what goes (id, title, cwd, account, files,
/// bytes) and what is skipped and why, with the totals, and changes nothing;
/// the run takes every file of each conversation and no shared file — it
/// runs while another conversation of the directory does — and its undo puts
/// every file back.
#[tokio::test]
async fn a_delete_skips_what_runs_and_its_undo_brings_everything_back() {
    let f = boot().await;
    let home = f.home.path();
    let proj = f.cwd("proj");
    write_session(home, &proj, S1);
    write_session(home, &proj, S2);
    write_session(home, &proj, S3);
    // S3 runs (this very process's pid is alive).
    run_here(home, &proj, S3);
    std::fs::write(
        home.join(".claude.json"),
        json!({"projects": {&proj: {"hasTrustDialogAccepted": true}}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        home.join("history.jsonl"),
        format!(
            "{}\n",
            json!({"display": "hi", "project": &proj, "timestamp": 1})
        ),
    )
    .unwrap();
    f.refresh().await;
    let missing = "99999999-9999-4999-8999-999999999999";
    let body = |dry: bool| json!({"items": {"sessions": [S1, S2, S3, missing, S1]}, "dryRun": dry});
    let before = tree(home);

    let (s, v) = f.delete(body(true)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let plan = &v["plan"];
    assert!(plan.get("from").is_none(), "{plan}");
    assert_eq!(plan["undo"], true, "kept for seven days by default");
    assert_eq!(plan["jobs"], 1);
    assert_eq!(ids_of(&plan["sessions"]), [S1, S2]);
    let one = &plan["sessions"][0];
    assert_eq!(one["title"], "Talk 1111");
    assert_eq!(one["cwd"], json!(proj));
    assert_eq!(one["account"], "default", "a plain account id");
    assert_eq!(
        one["files"], 5,
        "transcript, subagent, tool result, history, todo"
    );
    let size = |p: PathBuf| std::fs::metadata(p).unwrap().len();
    let bytes = size(transcript_path(home, &proj, S1)) + 3 + 6 + 3 + 2;
    assert_eq!(one["bytes"], json!(bytes));
    assert_eq!(
        codes_of(&plan["skipped"]),
        [
            (missing.to_string(), "not_found".to_string()),
            (S3.to_string(), "session_live".to_string())
        ]
    );
    assert_eq!(plan["skipped"][1]["title"], "Talk 3333", "titled");
    assert!(plan["skipped"][0].get("title").is_none());
    assert_eq!(plan["sessionsTotal"], 2);
    assert_eq!(plan["skippedTotal"], 2);
    assert_eq!(plan["files"], 10);
    assert_eq!(plan["bytes"], json!(2 * bytes));
    assert!(plan.get("truncated").is_none());
    assert_eq!(plan["folders"], json!([]), "S3 keeps the folder");
    assert_eq!(tree(home), before, "a dry run changes nothing");

    let (s, v) = f.delete(body(false)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["job"]["id"], v["jobs"][0]["id"], "`job` is the first");
    assert_eq!(v["job"]["kind"], "delete");
    assert_eq!(v["job"]["account"], "default");
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = f.finished(&job).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["deleted"], json!([S1, S2]));
    assert_eq!(
        codes_of(&done["result"]["skipped"]),
        [
            (missing.to_string(), "not_found".to_string()),
            (S3.to_string(), "session_live".to_string())
        ]
    );
    assert_eq!(done["result"]["skipped"][1]["title"], "Talk 3333");
    assert_eq!(done["sessions"], json!([S1, S2]));
    assert!(done["undoUntil"].as_i64().is_some());
    for id in [S1, S2] {
        assert!(!transcript_path(home, &proj, id).exists(), "{id}");
        assert!(!project_dir(home, &proj).join(id).exists());
        assert!(!home.join("file-history").join(id).exists());
        assert!(!home.join("session-env").join(id).exists());
        assert!(!home
            .join("todos")
            .join(format!("{id}-agent-{id}.json"))
            .exists());
    }
    assert!(
        transcript_path(home, &proj, S3).is_file(),
        "the running one stays"
    );
    let shared = |name: &str| std::fs::read(home.join(name)).unwrap();
    assert_eq!(
        shared(".claude.json"),
        before
            .iter()
            .find(|(p, _)| p == Path::new(".claude.json"))
            .unwrap()
            .1
    );
    assert_eq!(
        shared("history.jsonl"),
        before
            .iter()
            .find(|(p, _)| p == Path::new("history.jsonl"))
            .unwrap()
            .1
    );
    let (s, _) = f
        .call("GET", &format!("/sessions/{S1}"), &f.desktop(), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "gone from the list");
    let audit = f.st.devices.audit.recent(50);
    let line = audit.iter().find(|e| e.action == "data.delete").unwrap();
    assert_eq!(line.detail.as_ref().unwrap()["sessions"], 2);

    let (s, v) = f
        .call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &f.desktop(),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "undone");
    assert_eq!(v["kind"], "delete");
    assert_eq!(tree(home), before, "every file back where it was");
    let (s, _) = f
        .call("GET", &format!("/sessions/{S1}"), &f.desktop(), None)
        .await;
    assert_eq!(s, StatusCode::OK, "listed again");
}

/// §14.3: a project's every conversation goes, and its folder with them
/// when that leaves it empty (back with Undo); `undo: false` keeps no copy
/// and offers no Undo.
#[tokio::test]
async fn a_project_goes_with_its_empty_folder_with_or_without_undo() {
    let f = boot().await;
    let home = f.home.path();
    let kept = f.cwd("kept");
    let gone = f.cwd("gone");
    write_session(home, &kept, S1);
    write_session(home, &kept, S2);
    write_session(home, &gone, S3);
    f.refresh().await;
    let before = tree(home);
    let (s, v) = f
        .delete(
            json!({"from": "default", "items": {"projects": [format!("{kept}/")]},
                       "dryRun": true}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["from"], "default", "a plain account id");
    assert_eq!(v["plan"]["projects"], json!([kept]));
    assert_eq!(
        v["plan"]["folders"],
        json!([project_dir(home, &kept).to_string_lossy()])
    );
    assert_eq!(ids_of(&v["plan"]["sessions"]), [S1, S2]);
    let (_, v) = f
        .delete(json!({"from": {"account": "default"}, "items": {"projects": [kept]}}))
        .await;
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = f.finished(&job).await;
    assert_eq!(done["result"]["deleted"], json!([S1, S2]), "{done}");
    assert!(!project_dir(home, &kept).exists(), "left empty, it goes");
    let (s, v) = f
        .call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &f.desktop(),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(tree(home), before, "the folder and both conversations back");

    // Gone for good.
    let (s, v) = f
        .delete(json!({"items": {"projects": [gone]}, "undo": false}))
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = f.finished(&job).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["deleted"], json!([S3]));
    assert!(done.get("undoUntil").is_none(), "no Undo");
    assert!(!project_dir(home, &gone).exists());
    assert!(!home.join("file-history").join(S3).exists());
    let undo_dir = f.scratch.path().join("app-data/undo").join(&job);
    assert!(!undo_dir.exists(), "no copy is kept");
    let (s, v) = f
        .call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &f.desktop(),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let (_, list) = f.call("GET", "/data/jobs", &f.desktop(), None).await;
    let listed = list["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == json!(job))
        .unwrap()
        .clone();
    assert_eq!(listed["kind"], "delete");
    assert!(listed.get("undoUntil").is_none());
}

/// §14.3: without `from`, every directory that has them, one job per
/// directory — a conversation copied into two directories goes from both —
/// each job naming its directory's account.
#[tokio::test]
async fn without_from_every_directory_one_job_each() {
    let f = boot().await;
    let home = f.home.path();
    let proj = f.cwd("proj");
    let (work, work_dir) = f.account_dir("Work");
    write_session(home, &proj, S1);
    write_session(&work_dir, &proj, S2);
    write_session(home, &proj, S4);
    write_session(&work_dir, &proj, S4);
    f.refresh().await;
    let body = |dry: bool| json!({"items": {"sessions": [S1, S2, S4]}, "dryRun": dry});
    let (s, v) = f.delete(body(true)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let plan = &v["plan"];
    assert_eq!(plan["jobs"], 2);
    let rows: Vec<(String, String)> = plan["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["id"].as_str().unwrap().to_string(),
                s["account"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (S1.to_string(), "default".to_string()),
            (S4.to_string(), "default".to_string()),
            (S2.to_string(), work.clone()),
            (S4.to_string(), work.clone()),
        ]
    );
    assert_eq!(plan["sessionsTotal"], 4);

    let (s, v) = f.delete(body(false)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let jobs = v["jobs"].as_array().unwrap().clone();
    assert_eq!(jobs.len(), 2);
    let mut seen = Vec::new();
    for j in &jobs {
        let done = f.finished(j["id"].as_str().unwrap()).await;
        assert_eq!(done["state"], "done", "{done}");
        seen.push((
            done["account"].as_str().unwrap().to_string(),
            done["result"]["deleted"].clone(),
        ));
    }
    assert_eq!(
        seen,
        [
            ("default".to_string(), json!([S1, S4])),
            (work.clone(), json!([S2, S4]))
        ]
    );
    for (dir, id) in [
        (home, S1),
        (home, S4),
        (work_dir.as_path(), S2),
        (work_dir.as_path(), S4),
    ] {
        assert!(!transcript_path(dir, &proj, id).exists(), "{id}");
    }
}

/// §14.3: a shared-folder account's conversations are those attributed to
/// it; a token account's delete names it; a token no account holds counts
/// as the Default's; refusals of the request as a whole.
#[tokio::test]
async fn a_token_accounts_conversations_are_those_attributed_to_it() {
    let f = boot().await;
    let home = f.home.path();
    let proj = f.cwd("proj");
    let pro =
        f.st.accounts
            .add_token("Pro", Some(&Secret::new(TOKEN)), None)
            .unwrap()
            .id;
    write_session(home, &proj, S1);
    write_session(home, &proj, S5);
    f.st.accounts.remember_session(S5, &pro);
    f.refresh().await;

    let (s, v) = f
        .delete(json!({"from": pro, "items": {"sessions": [S1, S5]}, "dryRun": true}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(ids_of(&v["plan"]["sessions"]), [S5]);
    assert_eq!(v["plan"]["sessions"][0]["account"], json!(pro));
    assert_eq!(v["plan"]["skipped"][0]["id"], S1);
    assert_eq!(v["plan"]["skipped"][0]["code"], "not_found");
    assert!(
        v["plan"]["skipped"][0]["message"]
            .as_str()
            .unwrap()
            .contains("Default"),
        "{v}"
    );
    let (_, v) = f
        .delete(json!({"from": "default", "items": {"projects": [proj]}, "dryRun": true}))
        .await;
    assert_eq!(ids_of(&v["plan"]["sessions"]), [S1], "not Pro's");
    let (_, v) = f
        .delete(json!({"from": "unregistered", "items": {"sessions": [S1]}, "dryRun": true}))
        .await;
    assert_eq!(v["plan"]["from"], "default", "{v}");
    assert_eq!(ids_of(&v["plan"]["sessions"]), [S1]);

    let (s, v) = f
        .delete(json!({"from": pro, "items": {"projects": [proj]}}))
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["job"]["account"], json!(pro), "named after Pro");
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = f.finished(&job).await;
    assert_eq!(done["result"]["deleted"], json!([S5]), "{done}");
    assert!(
        transcript_path(home, &proj, S1).is_file(),
        "the Default's stays"
    );
    assert!(!transcript_path(home, &proj, S5).exists());
    // Undo brings it back as Pro's, even once this run no longer remembers
    // whose it was (the memory of it is pruned an hour after it is gone).
    f.st.accounts.runtime.remember(S5, "default");
    assert_eq!(session_account(&f.st, S5), "default");
    let (s, v) = f
        .call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &f.desktop(),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(transcript_path(home, &proj, S5).is_file());
    assert_eq!(session_account(&f.st, S5), pro);

    for (body, status, code) in [
        (
            json!({"from": "acc_000000000000", "items": {"sessions": [S1]}}),
            404,
            "not_found",
        ),
        (
            json!({"from": {"storage": "backup", "set": "Work"}, "items": {"sessions": [S1]}}),
            422,
            "invalid_transfer",
        ),
        (json!({"items": {}}), 422, "invalid_transfer"),
        (json!({"from": "default"}), 422, "invalid_body"),
    ] {
        let (s, v) = f.delete(body.clone()).await;
        assert_eq!(s.as_u16(), status, "{body}: {v}");
        assert_eq!(v["error"]["code"], code, "{body}");
    }
}

/// §14.3, §13.9: deleting is dangerous — a phone needs the right (a dry run
/// too), the local-script token never may; a granted phone's delete is
/// audited and named to the desktop; its Undo is an ordinary data job's.
#[tokio::test]
async fn who_may_delete() {
    let f = boot().await;
    let home = f.home.path();
    let proj = f.cwd("proj");
    write_session(home, &proj, S1);
    f.refresh().await;
    let body = |dry: bool| json!({"items": {"sessions": [S1]}, "dryRun": dry});
    let (device, phone) = f.phone();
    for dry in [true, false] {
        let (s, v) = f
            .call("POST", "/data/delete", &phone, Some(body(dry)))
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
        assert_eq!(v["error"]["code"], "dangerous_blocked");
    }
    let (s, v) = f
        .call("POST", "/data/delete", &f.script(), Some(body(true)))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "desktop_only");
    assert!(transcript_path(home, &proj, S1).is_file());

    let mut bus = f.st.bus.subscribe();
    f.st.devices
        .set_grants(&device, Some(true), Some(true))
        .unwrap();
    let (s, v) = f
        .call("POST", "/data/delete", &phone, Some(body(true)))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = f
        .call("POST", "/data/delete", &phone, Some(body(false)))
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = f.finished(&job).await;
    assert_eq!(done["result"]["deleted"], json!([S1]), "{done}");
    let audit = f.st.devices.audit.recent(50);
    assert!(audit
        .iter()
        .any(|e| e.action == "data.delete" && e.device_id.as_deref() == Some(device.as_str())));
    let notices = desktop_notices(&mut bus);
    assert!(
        notices.iter().any(|n| n.contains("\"Kitchen phone\"")
            && n.contains("started to delete 1 conversation(s)")),
        "{notices:?}"
    );

    // Undo needs no dangerous right: it only puts files back.
    f.st.devices
        .set_grants(&device, Some(false), Some(false))
        .unwrap();
    let (s, v) = f
        .call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &phone,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(transcript_path(home, &proj, S1).is_file());
}

/// §14.3: a conversation open in the API's host runs, so it is skipped; one
/// an API server joins or names that was deleted is joined or named no more
/// — the main server's and an added server's, whose saved defaults are
/// written again.
#[tokio::test]
async fn api_servers_forget_a_deleted_conversation() {
    use mother_claude_lib::bridge::control::{add_server, BridgeDefaults};
    let f = boot().await;
    let home = f.home.path();
    let proj = f.cwd("proj");
    write_session(home, &proj, S1);
    write_session(home, &proj, S2);
    write_session(home, &proj, S3);
    f.refresh().await;
    f.st.bridge
        .insert_thread(mother_claude_lib::bridge::ThreadRecord {
            thread_id: S3.to_string(),
            cwd: proj.clone(),
            created_at: 0,
            model: None,
            effort: None,
            permission_mode: "default".into(),
            title: None,
            persisted: true,
            active_turn: None,
            account: "default".into(),
            server: "main".into(),
        })
        .await;
    let main = f.st.bridge.main();
    main.set_active_thread(Some(S1.to_string()));
    main.update_saved(|d| d.default_thread = Some(S1.to_string()));
    let side = add_server(
        &f.st,
        "Side",
        Some(0),
        BridgeDefaults {
            default_thread: Some(S2.to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let saved =
        || std::fs::read_to_string(f.scratch.path().join("app-data/api-servers.json")).unwrap();
    assert!(saved().contains(S2));

    let (s, v) = f.delete(json!({"items": {"sessions": [S1, S2, S3]}})).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let done = f.finished(v["job"]["id"].as_str().unwrap()).await;
    assert_eq!(done["result"]["deleted"], json!([S1, S2]), "{done}");
    assert_eq!(done["result"]["skipped"][0]["id"], S3);
    assert_eq!(done["result"]["skipped"][0]["code"], "session_live");
    assert!(transcript_path(home, &proj, S3).is_file());

    assert_eq!(main.active_thread(), None);
    assert_eq!(main.defaults().default_thread, None);
    assert_eq!(main.saved().default_thread, None);
    assert_eq!(side.active_thread(), None);
    assert_eq!(side.saved().default_thread, None);
    assert!(!saved().contains(S2), "written again without it");
    let (_, v) = f.call("GET", "/bridge", &f.desktop(), None).await;
    assert_eq!(v["activeThread"], Value::Null, "{v}");
    assert_eq!(v["servers"][1]["defaults"]["defaultThread"], Value::Null);
}

/// §14.3: each list of a plan holds at most 1000 entries, `truncated` says
/// so, and its total counts them all.
#[tokio::test]
async fn a_capped_list_keeps_its_count() {
    let f = boot().await;
    f.refresh().await;
    let ids: Vec<String> = (0..1001)
        .map(|i| format!("{i:08x}-0000-4000-8000-000000000000"))
        .collect();
    let (s, v) = f
        .delete(json!({"items": {"sessions": ids}, "dryRun": true}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let plan = &v["plan"];
    assert_eq!(plan["truncated"], true);
    assert_eq!(plan["skipped"].as_array().unwrap().len(), 1000);
    assert_eq!(plan["skippedTotal"], 1001);
    assert_eq!(plan["sessionsTotal"], 0);
    assert_eq!(plan["skipped"][0]["code"], "not_found");
}
