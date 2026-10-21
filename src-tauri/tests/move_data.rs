//! Move data anywhere over real HTTP (docs/ACCOUNTS.md §13, P6.1–P6.6): the
//! `items` body — bulk moves that skip what cannot go, a job per source when
//! `from` is left out, assigning between accounts that share a directory,
//! moving into and out of a storage's set, customizations — each with its
//! undo; and goal 7. Every directory is a test's temp directory; the fake
//! `claude` (tests/fixtures/fake-claude.py) stands in for the CLI.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

use mother_claude_lib::claude::secrets::Secret;
use mother_claude_lib::claude::summaries::session_account;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

static SETUP: Once = Once::new();

fn setup() {
    SETUP.call_once(|| {
        let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-claude.py");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755));
        }
        std::env::set_var("MOTHER_CLAUDE_CLI", &fake);
    });
}

struct Fixture {
    st: AppState,
    base: String,
    home: tempfile::TempDir,
    scratch: tempfile::TempDir,
}

/// A Default whose `.claude.json` is inside its temp directory (never the
/// real `~/.claude.json`).
async fn boot() -> Fixture {
    setup();
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

    fn phone(&self) -> (String, String) {
        let (d, t) = self.st.devices.issue(
            "Phone",
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
            "PUT" => c.put(url),
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

    async fn move_data(&self, body: Value) -> (StatusCode, Value) {
        self.call("POST", "/data/move", &self.desktop(), Some(body))
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

    async fn undo(&self, job: &str) -> (StatusCode, Value) {
        self.call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &self.desktop(),
            Some(json!({})),
        )
        .await
    }

    /// The Work directory account.
    fn work(&self) -> (String, PathBuf) {
        let dir = self.scratch.path().join("claude-work");
        let a = self.st.accounts.add_directory("Work", &dir, None).unwrap();
        (a.id, dir)
    }

    fn cwd(&self) -> String {
        let cwd = self.scratch.path().join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        cwd.to_string_lossy().into_owned()
    }

    /// A backup folder and a passphrase.
    async fn backups(&self) -> (PathBuf, String) {
        let folder = self.scratch.path().join("iCloud/Mother Claude");
        let (s, _) = self
            .call(
                "PUT",
                "/backup/settings",
                &self.desktop(),
                Some(json!({ "folder": folder })),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        let (_, v) = self
            .call(
                "POST",
                "/backup/passphrase",
                &self.desktop(),
                Some(json!({"generate": true})),
            )
            .await;
        (folder, v["passphrase"].as_str().unwrap().to_string())
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
    format!("{user}\n{assistant}\n")
}

fn transcript_path(home: &Path, cwd: &str, sid: &str) -> PathBuf {
    home.join("projects")
        .join(mother_claude_lib::claude::encode_cwd(cwd))
        .join(format!("{sid}.jsonl"))
}

fn write_session(home: &Path, cwd: &str, sid: &str, live_pid: Option<u32>) {
    let t = transcript_path(home, cwd, sid);
    std::fs::create_dir_all(t.parent().unwrap()).unwrap();
    std::fs::write(&t, transcript(cwd, sid)).unwrap();
    if let Some(pid) = live_pid {
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        std::fs::write(
            home.join("sessions").join(format!("{pid}.json")),
            json!({"pid": pid, "sessionId": sid, "cwd": cwd, "status": "idle",
                   "kind": "interactive", "entrypoint": "cli"})
            .to_string(),
        )
        .unwrap();
    }
}

const S1: &str = "11111111-1111-4111-8111-111111111111";
const S2: &str = "22222222-2222-4222-8222-222222222222";
const S3: &str = "33333333-3333-4333-8333-333333333333";
const S4: &str = "44444444-4444-4444-8444-444444444444";

/// §13.3 rules 1–2: the `items` body moves what can go and lists what cannot
/// with its code; the old body refuses with that code, as before.
#[tokio::test]
async fn a_bulk_move_skips_what_cannot_go_and_the_old_body_still_refuses() {
    let f = boot().await;
    let cwd = f.cwd();
    let (work, work_dir) = f.work();
    write_session(f.home.path(), &cwd, S1, None);
    write_session(f.home.path(), &cwd, S2, None);
    // This very process's pid is alive: S3 runs.
    write_session(f.home.path(), &cwd, S3, Some(std::process::id()));
    mother_claude_lib::server::monitor::refresh_once(&f.st).await;
    let missing = "99999999-9999-4999-8999-999999999999";
    let body = |dry: bool| {
        json!({"from": "default", "to": {"account": work}, "mode": "move", "dryRun": dry,
               "items": {"sessions": [S1, S2, S3, missing]}})
    };
    let (s, v) = f.move_data(body(true)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let plan = &v["plan"];
    assert_eq!(plan["from"], json!({"account": "default"}));
    assert_eq!(plan["to"], json!({"account": work}));
    assert_eq!(plan["jobs"], 1);
    let ids: Vec<&str> = plan["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [S1, S2]);
    assert_eq!(plan["sessions"][0]["action"], "move");
    assert_eq!(plan["sessions"][0]["from"], json!({"account": "default"}));
    assert_eq!(plan["sessions"][0]["cwd"], json!(cwd));
    let codes: Vec<(&str, &str)> = plan["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["id"].as_str().unwrap(), s["code"].as_str().unwrap()))
        .collect();
    assert_eq!(codes, [(S3, "session_live"), (missing, "not_found")]);
    assert!(
        work_dir.join("projects").read_dir().is_err(),
        "a dry run writes nothing"
    );

    let (s, v) = f.move_data(body(false)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["job"]["id"], v["jobs"][0]["id"], "`job` is the first");
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = f.finished(&job).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["kind"], "move");
    assert_eq!(done["from"], "default");
    assert_eq!(done["to"], json!(work));
    assert_eq!(done["result"]["moved"], json!([S1, S2]));
    assert_eq!(done["result"]["skipped"][0]["code"], "session_live");
    assert!(transcript_path(&work_dir, &cwd, S1).is_file());
    assert!(!transcript_path(f.home.path(), &cwd, S1).exists());

    // The old body: the live one refuses the whole request, as before.
    let (s, v) = f
        .move_data(
            json!({"what": "session", "id": S3, "from": "default", "to": work,
                          "mode": "move", "dryRun": true}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "session_live");
    assert!(v.get("plan").is_none());

    let (s, v) = f.undo(&job).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(transcript_path(f.home.path(), &cwd, S1).is_file());
    assert!(!transcript_path(&work_dir, &cwd, S1).exists());
    let audit = f.st.devices.audit.recent(50);
    assert!(audit.iter().any(|e| e.action == "data.move"));
}

/// §13.3 rule 1: with only `items.sessions`, `from` may be left out; each
/// conversation goes from where it is, one job per source.
#[tokio::test]
async fn conversations_go_from_where_they_are_one_job_per_source() {
    let f = boot().await;
    let cwd = f.cwd();
    let (work, work_dir) = f.work();
    let other_dir = f.scratch.path().join("claude-other");
    let other =
        f.st.accounts
            .add_directory("Other", &other_dir, None)
            .unwrap()
            .id;
    write_session(f.home.path(), &cwd, S1, None);
    write_session(&work_dir, &cwd, S2, None);
    write_session(&work_dir, &cwd, S4, None);
    write_session(&other_dir, &cwd, S3, None);
    mother_claude_lib::server::monitor::refresh_once(&f.st).await;
    let body = |dry: bool| {
        json!({"to": other, "mode": "copy", "dryRun": dry,
               "items": {"sessions": [S1, S2, S3, S4]}})
    };
    let (s, v) = f.move_data(body(true)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let plan = &v["plan"];
    assert!(plan.get("from").is_none(), "{plan}");
    assert_eq!(plan["jobs"], 2);
    let from_of = |id: &str| {
        plan["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .map(|s| s["from"].clone())
    };
    assert_eq!(from_of(S1), Some(json!({"account": "default"})));
    assert_eq!(from_of(S2), Some(json!({"account": work})));
    assert_eq!(plan["skipped"][0]["id"], S3, "Other has it already");
    assert_eq!(plan["skipped"][0]["code"], "already_exists");

    let (s, v) = f.move_data(body(false)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let jobs = v["jobs"].as_array().unwrap().clone();
    assert_eq!(jobs.len(), 2);
    let mut froms = Vec::new();
    for j in &jobs {
        let done = f.finished(j["id"].as_str().unwrap()).await;
        assert_eq!(done["state"], "done", "{done}");
        assert_eq!(done["kind"], "copy");
        froms.push(done["from"].as_str().unwrap().to_string());
    }
    froms.sort();
    let mut want = vec!["default".to_string(), work.clone()];
    want.sort();
    assert_eq!(froms, want);
    for id in [S1, S2, S4] {
        assert!(transcript_path(&other_dir, &cwd, id).is_file(), "{id}");
    }
}

/// §13.2, P6.2: between accounts that share a directory a move is an
/// assign — nothing moves, the conversations are recorded as the target's —
/// and its undo puts the previous account back; a copy is refused; a
/// running one is skipped.
#[tokio::test]
async fn an_assign_between_shared_folder_accounts_and_its_undo() {
    let f = boot().await;
    let cwd = f.cwd();
    let pro =
        f.st.accounts
            .add_token("Pro", Some(&Secret::new(TOKEN)), None)
            .unwrap()
            .id;
    write_session(f.home.path(), &cwd, S1, None);
    write_session(f.home.path(), &cwd, S2, Some(std::process::id()));
    mother_claude_lib::server::monitor::refresh_once(&f.st).await;
    assert_eq!(session_account(&f.st, S1), "default");
    let (s, v) = f
        .move_data(json!({"from": "default", "to": pro, "mode": "copy",
                          "items": {"sessions": [S1]}}))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "same_directory");

    let (s, v) = f
        .move_data(
            json!({"from": "default", "to": pro, "mode": "move", "dryRun": true,
                          "items": {"sessions": [S1, S2]}}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["sessions"][0]["action"], "assign");
    assert_eq!(v["plan"]["files"], 0, "nothing is written");
    assert_eq!(v["plan"]["skipped"][0]["code"], "session_live");
    let (s, v) = f
        .move_data(json!({"from": "default", "to": pro, "mode": "move",
                          "items": {"sessions": [S1, S2]}}))
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = f.finished(&job).await;
    assert_eq!(done["result"]["assigned"], json!([S1]), "{done}");
    assert!(done["undoUntil"].as_i64().is_some());
    assert_eq!(session_account(&f.st, S1), pro);
    assert!(
        transcript_path(f.home.path(), &cwd, S1).is_file(),
        "nothing moved"
    );
    let remembered = std::fs::read_to_string(
        f.st.auth
            .config_dir
            .clone()
            .unwrap()
            .join("account-sessions.json"),
    )
    .unwrap();
    assert!(remembered.contains(S1), "kept across restarts");

    let (s, v) = f.undo(&job).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "undone");
    assert_eq!(session_account(&f.st, S1), "default");
    // Earlier runs' assigns are listed from their manifests.
    let (_, list) = f.call("GET", "/data/jobs", &f.desktop(), None).await;
    let listed = list["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == json!(job))
        .unwrap()
        .clone();
    assert_eq!(listed["to"], json!(pro));
}

/// §13.2, P6.4: a copy into a storage's set is a push of just those; a move
/// archives them — removed here, marked in the set — a pull never brings
/// them back, and the undo puts the files back and clears the marks.
#[tokio::test]
async fn a_move_into_storage_archives_and_its_undo_brings_it_back() {
    let f = boot().await;
    let cwd = f.cwd();
    let (work, work_dir) = f.work();
    write_session(&work_dir, &cwd, S1, None);
    write_session(&work_dir, &cwd, S2, None);
    let (folder, _) = f.backups().await;
    // A copy: the set gets S2, nothing leaves Work.
    let (s, v) = f
        .move_data(
            json!({"from": work, "to": {"storage": "backup"}, "mode": "copy",
                          "items": {"sessions": [S2]}}),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["job"]["toStorage"], "backup");
    assert_eq!(v["job"]["set"], "Work", "the account's own set by default");
    let done = f.finished(v["job"]["id"].as_str().unwrap()).await;
    assert_eq!(done["result"]["copied"], json!([S2]), "{done}");
    assert!(
        done.get("undoUntil").is_none(),
        "a copy into a set is a backup"
    );
    assert!(transcript_path(&work_dir, &cwd, S2).is_file());

    // A move: S1 archived.
    let (s, v) = f
        .move_data(
            json!({"from": work, "to": {"storage": "backup", "set": "Work"},
                          "mode": "move", "dryRun": true, "items": {"sessions": [S1]}}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["sessions"][0]["action"], "archive");
    let (_, v) = f
        .move_data(
            json!({"from": work, "to": {"storage": "backup", "set": "Work"},
                          "mode": "move", "items": {"sessions": [S1]}}),
        )
        .await;
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = f.finished(&job).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["archived"], json!([S1]));
    assert_eq!(done["from"], json!(work));
    assert_eq!(done["toStorage"], "backup");
    assert!(
        !transcript_path(&work_dir, &cwd, S1).exists(),
        "removed here"
    );
    let (s, v) = f
        .call(
            "GET",
            "/backup/index?storage=backup&set=Work",
            &f.desktop(),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let item = |v: &Value, id: &str| {
        v["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .cloned()
            .unwrap()
    };
    assert!(item(&v, S1)["archived"]["machine"].is_string(), "{v}");
    assert!(item(&v, S2).get("archived").is_none());
    let (_, b) = f.call("GET", "/backup", &f.desktop(), None).await;
    let set = b["storage"][0]["sets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Work")
        .unwrap()
        .clone();
    assert_eq!(set["archived"], 1, "{set}");
    // A pull never brings it back here.
    let (s, v) = f
        .call(
            "POST",
            "/backup/pull",
            &f.desktop(),
            Some(json!({ "account": work })),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let pulled = f.finished(v["jobs"][0].as_str().unwrap()).await;
    assert_eq!(pulled["state"], "done", "{pulled}");
    assert!(!transcript_path(&work_dir, &cwd, S1).exists());

    let (s, v) = f.undo(&job).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(transcript_path(&work_dir, &cwd, S1).is_file(), "back here");
    let (_, v) = f
        .call(
            "GET",
            "/backup/index?storage=backup&set=Work",
            &f.desktop(),
            None,
        )
        .await;
    assert!(
        item(&v, S1).get("archived").is_none(),
        "the mark is cleared"
    );
    let _ = folder;
}

/// §13.2, P6.5: a move out of a set of another storage restores into any
/// account and takes the entries out of the set; the undo puts them back
/// there first, then takes the restored files away.
#[tokio::test]
async fn a_move_out_of_storage_and_its_undo_puts_the_entries_back() {
    let f = boot().await;
    let cwd = f.cwd();
    let (work, work_dir) = f.work();
    let (_, pass) = f.backups().await;
    std::fs::create_dir_all(f.scratch.path().join("Disk")).unwrap();
    let (s, disk) = f
        .call(
            "POST",
            "/backup/storage",
            &f.desktop(),
            Some(json!({"path": f.scratch.path().join("Disk/Claude"), "label": "Disk"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{disk}");
    let disk_id = disk["id"].as_str().unwrap().to_string();
    // The Default's conversations copied into the set "Archive" there.
    write_session(f.home.path(), &cwd, S1, None);
    write_session(f.home.path(), &cwd, S2, None);
    let (s, v) = f
        .move_data(
            json!({"from": "default", "to": {"storage": disk_id, "set": "Archive"},
                          "mode": "copy", "items": {"all": true}}),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let done = f.finished(v["job"]["id"].as_str().unwrap()).await;
    assert_eq!(done["result"]["copied"], json!([S1, S2]), "{done}");

    // Out of the set into Work.
    let (s, v) = f
        .move_data(
            json!({"from": {"storage": disk_id, "set": "Archive"}, "to": work,
                          "mode": "move", "dryRun": true, "items": {"sessions": [S1]}}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["sessions"][0]["action"], "restore");
    assert_eq!(
        v["plan"]["sessions"][0]["from"],
        json!({"storage": disk_id, "set": "Archive"})
    );
    let (_, v) = f
        .move_data(
            json!({"from": {"storage": disk_id, "set": "Archive"}, "to": work,
                          "mode": "move", "items": {"sessions": [S1]}}),
        )
        .await;
    let job = v["job"]["id"].as_str().unwrap().to_string();
    assert_eq!(v["job"]["fromStorage"], json!(disk_id));
    assert_eq!(v["job"]["set"], "Archive");
    let done = f.finished(&job).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["restored"], json!([S1]));
    assert!(transcript_path(&work_dir, &cwd, S1).is_file());
    let index = |f: &Fixture| {
        let set = mother_claude_lib::claude::backup::Set::new(
            &f.scratch.path().join("Disk/Claude"),
            "Archive",
        );
        let keys =
            mother_claude_lib::claude::backup::open_keys(&set, &Secret::new(pass.clone())).unwrap();
        mother_claude_lib::claude::backup::read_index(&set, &keys).unwrap()
    };
    assert!(!index(&f).sessions.contains_key(S1), "out of the set");
    assert!(index(&f).sessions.contains_key(S2));

    let (s, v) = f.undo(&job).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(index(&f).sessions.contains_key(S1), "back in the set");
    assert!(!transcript_path(&work_dir, &cwd, S1).exists());
}

/// §13.3 rule 7, §13.9: customizations between accounts, copied only; the
/// settings and MCP servers need the dangerous right, the rest does not.
#[tokio::test]
async fn customizations_over_http() {
    let f = boot().await;
    let (work, work_dir) = f.work();
    let home = f.home.path();
    std::fs::create_dir_all(home.join("skills/review")).unwrap();
    std::fs::write(home.join("skills/review/SKILL.md"), "review").unwrap();
    std::fs::write(home.join("settings.json"), "{\"model\": \"opus\"}").unwrap();
    let (device, phone) = f.phone();
    let body = |what: Value, mode: &str, dry: bool| {
        json!({"from": "default", "to": work, "mode": mode, "dryRun": dry,
               "items": {"customizations": what}})
    };
    let (s, v) = f
        .call(
            "POST",
            "/data/move",
            &phone,
            Some(body(json!(["settings"]), "copy", true)),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "dangerous_blocked");
    let (s, v) = f
        .call(
            "POST",
            "/data/move",
            &phone,
            Some(body(json!(["skills"]), "copy", false)),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let done = f.finished(v["job"]["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["customizations"][0]["name"], "review");
    assert_eq!(done["result"]["customizations"][0]["action"], "add");
    assert!(work_dir.join("skills/review/SKILL.md").is_file());
    // Never moved; the desktop (or a granted phone) may copy the settings.
    let (s, v) = f.move_data(body(json!(["skills"]), "move", true)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    f.st.devices
        .set_grants(&device, Some(true), Some(true))
        .unwrap();
    let (s, v) = f
        .call(
            "POST",
            "/data/move",
            &phone,
            Some(body(json!(["settings"]), "copy", true)),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["customizations"][0]["action"], "add");
}

/// Goal 7 (§13.3 rule 9): with only the Default and only the backup folder,
/// what the backup routes answer is what they answered before §13.
#[tokio::test]
async fn with_only_the_default_every_response_is_as_before() {
    let f = boot().await;
    let cwd = f.cwd();
    write_session(f.home.path(), &cwd, S1, None);
    f.backups().await;
    let (_, v) = f
        .call("POST", "/backup/push", &f.desktop(), Some(json!({})))
        .await;
    let done = f.finished(v["jobs"][0].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    for key in ["fromStorage", "toStorage", "set"] {
        assert!(done.get(key).is_none(), "{key}: {done}");
    }
    let (_, b) = f.call("GET", "/backup", &f.desktop(), None).await;
    assert!(b.get("storage").is_none(), "{b}");
    assert!(b["settings"].get("storages").is_none(), "{b}");
    let (_, idx) = f
        .call("GET", "/backup/index?account=default", &f.desktop(), None)
        .await;
    let item = &idx["sessions"][0];
    assert_eq!(item["id"], S1);
    assert!(item.get("archived").is_none(), "{item}");
    assert!(idx.get("storage").is_none(), "{idx}");
    assert!(f.st.accounts.is_default_only());
    assert!(!f
        .st
        .auth
        .config_dir
        .clone()
        .unwrap()
        .join("account-sessions.json")
        .exists());
}

/// §13.2, P6.7: a move from a set of one storage to another — the target
/// set's name defaults to the source's — and its undo.
#[tokio::test]
async fn conversations_move_between_storages_and_back() {
    let f = boot().await;
    let cwd = f.cwd();
    let (work, work_dir) = f.work();
    write_session(&work_dir, &cwd, S1, None);
    write_session(&work_dir, &cwd, S2, None);
    let (_, pass) = f.backups().await;
    let (_, v) = f
        .call(
            "POST",
            "/backup/push",
            &f.desktop(),
            Some(json!({ "account": work })),
        )
        .await;
    f.finished(v["jobs"][0].as_str().unwrap()).await;
    std::fs::create_dir_all(f.scratch.path().join("Disk")).unwrap();
    let (_, disk) = f
        .call(
            "POST",
            "/backup/storage",
            &f.desktop(),
            Some(json!({"path": f.scratch.path().join("Disk/Claude"), "label": "Disk"})),
        )
        .await;
    let disk_id = disk["id"].as_str().unwrap().to_string();
    let body = |dry: bool| {
        json!({"from": {"storage": "backup", "set": "Work"}, "to": {"storage": disk_id},
               "mode": "move", "dryRun": dry, "items": {"sessions": [S1]}})
    };
    let (s, v) = f.move_data(body(true)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["sessions"][0]["action"], "move");
    let (s, v) = f.move_data(body(false)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["job"]["fromStorage"], "backup");
    assert_eq!(v["job"]["set"], "Work");
    assert_eq!(v["job"]["toStorage"], json!(disk_id));
    assert_eq!(v["job"]["toSet"], "Work", "the source's name by default");
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = f.finished(&job).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["moved"], json!([S1]));
    let ids_in = |storage: &str| {
        let f = &f;
        let storage = storage.to_string();
        async move {
            let (_, v) = f
                .call(
                    "GET",
                    &format!(
                        "/backup/index?storage={storage}&set=Work&into={}",
                        "default"
                    ),
                    &f.desktop(),
                    None,
                )
                .await;
            v["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(ids_in(&disk_id).await, [S1]);
    assert_eq!(ids_in("backup").await, [S2]);
    // The same set is no target; storage → storage takes no customizations.
    let (s, v) = f
        .move_data(json!({"from": {"storage": "backup", "set": "Work"},
                          "to": {"storage": "backup", "set": "Work"},
                          "mode": "copy", "items": {"all": true}}))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

    let (s, v) = f.undo(&job).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "undone");
    let mut back = ids_in("backup").await;
    back.sort();
    assert_eq!(back, [S1, S2]);
    assert!(ids_in(&disk_id).await.is_empty());
    let _ = pass;
}

/// A storage job whose conversations were all skipped says so, and never
/// opens or makes the set it would have written.
#[tokio::test]
async fn a_storage_job_with_nothing_to_take_makes_no_set() {
    let f = boot().await;
    let (work, _) = f.work();
    let (folder, _) = f.backups().await;
    let missing = "99999999-9999-4999-8999-999999999999";
    let (s, v) = f
        .move_data(
            json!({"from": work, "to": {"storage": "backup", "set": "Fresh"},
                          "mode": "move", "items": {"sessions": [missing]}}),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let done = f.finished(v["job"]["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["archived"], json!([]));
    assert_eq!(done["result"]["skipped"][0]["code"], "not_found");
    assert!(done.get("undoUntil").is_none());
    assert!(!folder.join("Fresh").exists(), "no set was made");
}
