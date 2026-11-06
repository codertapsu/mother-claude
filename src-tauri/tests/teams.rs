//! Teams of sessions (docs/TEAMS.md) against the FAKE `claude`
//! (`tests/fixtures/fake-claude.py`, selected with `MOTHER_CLAUDE_CLI`): a
//! team started over HTTP, its lead and two workers launched as owned
//! sessions with the team's arguments, the board worked through the MCP
//! tools as each session, the lead told about the board, the ownership
//! guard on both owned paths (the `initialize` PreToolUse callback and
//! `can_use_tool`), a phone steering the board, idle stop, resume and
//! stopping the team.
//!
//! Nothing here touches `~/.claude` or runs the real CLI.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::mcp::{self, Caller, ToolCtx, ToolOutput};
use mother_claude_lib::orchestration::launcher;
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
        std::env::remove_var("MOTHER_CLAUDE_SIDECAR");
        // A plan account: no `--max-budget-usd`.
        std::env::remove_var("ANTHROPIC_API_KEY");
    });
}

struct Fixture {
    st: AppState,
    base: String,
    project: PathBuf,
    data: PathBuf,
    _dirs: Vec<tempfile::TempDir>,
}

async fn boot() -> Fixture {
    setup();
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let project = mother_claude_lib::claude::paths::canonical_or_given(work.path()).join("shop");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    let data = scratch.path().join("app-data");
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(data.clone());
    let st = Inner::new(
        ClaudeHome::for_account_dir(home.path().to_path_buf()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
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
        project,
        data,
        _dirs: vec![home, scratch, work],
    }
}

impl Fixture {
    fn desktop(&self) -> String {
        self.st.auth.token.clone()
    }

    fn phone(&self) -> String {
        self.st
            .devices
            .issue(
                "Phone",
                "ios",
                None,
                Ingress::Lan,
                "192.168.1.60".parse().unwrap(),
            )
            .1
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
            "PATCH" => c.patch(url),
            "DELETE" => c.delete(url),
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

    fn project(&self) -> String {
        self.project.to_string_lossy().into_owned()
    }

    /// Call a team tool as a session.
    async fn tool(&self, session: Option<&str>, name: &str, args: Value) -> ToolOutput {
        let ctx = ToolCtx {
            state: self.st.clone(),
            caller: Caller {
                session_id: session.map(str::to_string),
                pid: None,
                cwd: Some(self.project()),
                owned: true,
            },
        };
        mcp::registry().call(name, ctx, args).await.unwrap()
    }

    fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(self.project.join("fake-claude.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// The argv a session was launched with.
    fn argv_of(&self, session: &str) -> Vec<String> {
        self.log()
            .into_iter()
            .filter_map(|l| l.get("argv").cloned())
            .map(|a| serde_json::from_value::<Vec<String>>(a).unwrap())
            .rfind(|a| a.iter().any(|x| x == session))
            .unwrap_or_default()
    }

    /// What a session read from stdin.
    fn received(&self, session: &str) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter(|l| l["session"] == session)
            .filter_map(|l| l.get("in").cloned())
            .collect()
    }

    /// The text of the user messages a session got.
    fn messages(&self, session: &str) -> Vec<String> {
        self.received(session)
            .into_iter()
            .filter(|m| m["type"] == "user")
            .filter_map(|m| {
                m.pointer("/message/content/0/text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    }

    fn logged(&self, key: &str) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter_map(|l| l.get(key).cloned())
            .collect()
    }

    fn role_session(&self, team: &str, role: &str) -> Option<String> {
        self.st.teams.get(team)?.slot(role).session_id
    }
}

fn flag(argv: &[String], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == name)
        .and_then(|i| argv.get(i + 1))
        .cloned()
}

async fn wait_for<F: FnMut() -> bool>(what: &str, mut f: F) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

fn json_of(out: &ToolOutput) -> Value {
    assert!(!out.is_error, "tool failed: {:?}", out.content);
    out.structured_content.clone().unwrap()
}

fn error_of(out: &ToolOutput) -> String {
    assert!(
        out.is_error,
        "expected an error: {:?}",
        out.structured_content
    );
    serde_json::to_value(&out.content).unwrap()[0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lead_and_two_workers_run_a_feature_through_the_board() {
    let f = boot().await;

    // ── the team starts: its lead comes up with the brief ──
    let (status, team) = f
        .call(
            "POST",
            "/teams",
            &f.desktop(),
            Some(json!({
                "project": f.project(),
                "template": "software-delivery",
                "brief": "Build the login page."
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let team_id = team["id"].as_str().unwrap().to_string();
    assert_eq!(team["label"], "shop");
    assert_eq!(team["roles"].as_array().unwrap().len(), 4);
    // One team per project.
    let (status, again) = f
        .call(
            "POST",
            "/teams",
            &f.desktop(),
            Some(json!({ "project": f.project(), "template": "research" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{again}");

    launcher::tick(&f.st).await;
    let lead = f.role_session(&team_id, "lead").expect("the lead started");
    wait_for("the lead's brief", || {
        f.messages(&lead)
            .iter()
            .any(|m| m.contains("Build the login page."))
    })
    .await;
    let argv = f.argv_of(&lead);
    assert_eq!(flag(&argv, "-n").as_deref(), Some("shop/lead"));
    assert_eq!(flag(&argv, "--model").as_deref(), Some("opus"));
    assert_eq!(flag(&argv, "--effort").as_deref(), Some("high"));
    assert_eq!(flag(&argv, "--permission-mode").as_deref(), Some("default"));
    assert_eq!(
        flag(&argv, "--settings").as_deref(),
        Some(r#"{"crossSessionInbound":"accept"}"#)
    );
    assert!(flag(&argv, "--append-system-prompt")
        .unwrap()
        .starts_with("# Team role: Lead"));
    let agents = PathBuf::from(flag(&argv, "--agents").unwrap());
    assert!(
        agents.starts_with(&f.data),
        "the definitions live in the app data dir"
    );
    let defs: Value = serde_json::from_str(&std::fs::read_to_string(&agents).unwrap()).unwrap();
    assert_eq!(defs["frontend"]["model"], "sonnet");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&agents).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    assert!(!argv.iter().any(|a| a == "--agent"), "never --agent");
    assert!(
        !argv.iter().any(|a| a == "-w"),
        "the lead works in the main checkout"
    );
    assert!(
        !argv.iter().any(|a| a == "--max-budget-usd"),
        "a plan account"
    );
    // Its `initialize` registered the guard's callback.
    let init = f
        .received(&lead)
        .into_iter()
        .find(|m| m.pointer("/request/subtype") == Some(&json!("initialize")))
        .unwrap();
    assert_eq!(
        init.pointer("/request/hooks/PreToolUse/0/hookCallbackIds/0"),
        Some(&json!("mc_team_guard"))
    );

    // ── who may use the board ──
    let out = f.tool(None, "task_list", json!({})).await;
    assert!(error_of(&out).contains("could not tell which Claude Code session"));
    let stranger = uuid::Uuid::new_v4().to_string();
    let out = f.tool(Some(&stranger), "task_list", json!({})).await;
    assert!(error_of(&out).ends_with("(denied)"));

    // ── the lead writes the packets; Mother Claude starts the workers ──
    let out = f
        .tool(
            Some(&lead),
            "task_create",
            json!({
                "title": "Login form",
                "goal": "A form that signs the user in.",
                "acceptance": ["Shows a wrong-password error"],
                "role": "frontend",
                "class": "implement",
                "priority": 1,
                "files": ["src/app/login.ts"]
            }),
        )
        .await;
    let t1 = json_of(&out)["task"]["id"].as_u64().unwrap();
    let out = f
        .tool(
            Some(&lead),
            "task_create",
            json!({ "title": "Login API", "goal": "POST /api/login.", "role": "backend" }),
        )
        .await;
    let t2 = json_of(&out)["task"]["id"].as_u64().unwrap();
    let out = f
        .tool(
            Some(&lead),
            "task_create",
            json!({ "title": "x", "goal": "y", "role": "nobody" }),
        )
        .await;
    assert!(error_of(&out).contains("No role \"nobody\""));
    launcher::tick(&f.st).await;
    let fe = f
        .role_session(&team_id, "frontend")
        .expect("frontend started");
    let be = f
        .role_session(&team_id, "backend")
        .expect("backend started");
    assert!(
        f.role_session(&team_id, "scout").is_none(),
        "no work, no autostart"
    );
    wait_for("the frontend's packet", || {
        f.messages(&fe).iter().any(|m| {
            m.starts_with(&format!(
                "[Mother Claude · assignment] Task {t1}: Login form"
            ))
        })
    })
    .await;
    wait_for("the backend's packet", || {
        f.messages(&be).iter().any(|m| m.contains("Login API"))
    })
    .await;
    let argv = f.argv_of(&fe);
    assert_eq!(flag(&argv, "-n").as_deref(), Some("shop/frontend"));
    assert_eq!(flag(&argv, "-w").as_deref(), Some("frontend"));
    assert_eq!(
        flag(&argv, "--permission-mode").as_deref(),
        Some("acceptEdits")
    );
    assert_eq!(flag(&argv, "--model").as_deref(), Some("sonnet"));
    let (_, board) = f
        .call(
            "GET",
            &format!("/teams/{team_id}/tasks"),
            &f.desktop(),
            None,
        )
        .await;
    let task1 = board["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == t1)
        .unwrap()
        .clone();
    assert_eq!(task1["status"], "claimed");
    assert_eq!(task1["claimedBy"], fe.as_str());
    assert_eq!(task1["model"]["model"], "sonnet");

    // ── a worker is held to its role ──
    let out = f
        .tool(
            Some(&fe),
            "task_create",
            json!({ "title": "x", "goal": "y", "role": "frontend" }),
        )
        .await;
    assert!(error_of(&out).contains("lead"));
    let out = f
        .tool(
            Some(&fe),
            "task_update",
            json!({ "id": t2, "note": "mine?" }),
        )
        .await;
    assert!(error_of(&out).contains("not yours"));
    let out = f
        .tool(Some(&fe), "session_summary", json!({ "role": "backend" }))
        .await;
    assert!(error_of(&out).ends_with("(denied)"));
    let out = f
        .tool(
            Some(&fe),
            "task_update",
            json!({ "id": t1, "status": "sideways" }),
        )
        .await;
    assert!(error_of(&out).contains("is not a status"));
    let roles = json_of(&f.tool(Some(&fe), "team_roles", json!({})).await);
    assert_eq!(roles["you"]["sessionName"], "shop/frontend");
    assert_eq!(roles["roles"][0]["sessionName"], "shop/lead");

    // ── the guard: the PreToolUse callback, then can_use_tool ──
    let wt = f.project.join(".claude/worktrees/frontend");
    let outside = wt.join("src-tauri/src/main.rs");
    let inside = wt.join("src/app/login.ts");
    mother_claude_lib::claude::owned::send(
        &f.st,
        &fe,
        &format!("write {}", outside.display()),
        &[],
    )
    .await
    .unwrap();
    mother_claude_lib::claude::owned::send(&f.st, &fe, &format!("write {}", inside.display()), &[])
        .await
        .unwrap();
    wait_for("both writes judged", || f.logged("hook").len() >= 2).await;
    let hooks = f.logged("hook");
    assert_eq!(
        hooks[0].pointer("/hookSpecificOutput/permissionDecision"),
        Some(&json!("deny"))
    );
    let reason = hooks[0]
        .pointer("/hookSpecificOutput/permissionDecisionReason")
        .and_then(Value::as_str)
        .unwrap();
    assert!(
        reason.starts_with("Mother Claude's team guard stopped this Write"),
        "{reason}"
    );
    assert!(reason.contains("src-tauri/src/main.rs"), "{reason}");
    assert!(reason.contains("shop/lead"), "{reason}");
    assert_eq!(hooks[1], json!({}), "its own path is fine");
    mother_claude_lib::claude::owned::send(
        &f.st,
        &fe,
        &format!("edit-ask {}", outside.display()),
        &[],
    )
    .await
    .unwrap();
    wait_for("the edit judged", || {
        f.logged("decision").iter().any(|d| d["behavior"] == "deny")
    })
    .await;
    assert!(f.st.pending.list(&fe).is_empty(), "denied without a card");
    let audit = f.st.devices.audit.recent(100);
    let denial = audit
        .iter()
        .find(|e| e.action == "team.guard.deny")
        .expect("audited");
    let detail = denial.detail.clone().unwrap();
    assert_eq!(detail["role"], "frontend");
    assert_eq!(detail["tool"], "Write");
    assert!(detail["target"].as_str().unwrap().ends_with("main.rs"));
    let (_, view) = f
        .call("GET", &format!("/teams/{team_id}"), &f.desktop(), None)
        .await;
    let fe_view = view["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "frontend")
        .unwrap()
        .clone();
    assert_eq!(fe_view["guardDenials"], 2);

    // ── the frontend finishes: review; the lead hears and accepts ──
    json_of(
        &f.tool(
            Some(&fe),
            "task_update",
            json!({ "id": t1, "status": "in_progress", "note": "form drafted" }),
        )
        .await,
    );
    let done = json_of(
        &f.tool(
            Some(&fe),
            "task_done",
            json!({
                "id": t1,
                "summary": "Form with validation",
                "branch": "worktree-frontend",
                "changedFiles": ["src/app/login.ts"]
            }),
        )
        .await,
    );
    assert_eq!(done["task"]["status"], "review");
    wait_for("the lead's review notice", || {
        f.messages(&lead).iter().any(|m| {
            m.starts_with(&format!(
                "[Mother Claude · board] Task {t1} (frontend) is ready for review"
            ))
        })
    })
    .await;
    let accepted = json_of(
        &f.tool(
            Some(&lead),
            "task_update",
            json!({ "id": t1, "status": "done" }),
        )
        .await,
    );
    assert_eq!(accepted["task"]["status"], "done");

    // ── the backend fails; the lead hears ──
    json_of(
        &f.tool(
            Some(&be),
            "task_done",
            json!({ "id": t2, "summary": "The auth library is missing", "outcome": "failed" }),
        )
        .await,
    );
    wait_for("the lead's failure notice", || {
        f.messages(&lead)
            .iter()
            .any(|m| m.contains(&format!("Task {t2} (backend) failed")))
    })
    .await;
    let summary = json_of(
        &f.tool(Some(&lead), "session_summary", json!({ "role": "backend" }))
            .await,
    );
    assert_eq!(summary["running"], true);
    let status = json_of(&f.tool(Some(&lead), "team_status", json!({})).await);
    assert_eq!(status["board"]["done"], 1);
    assert_eq!(status["board"]["failed"], 1);

    // ── a phone follows and steers the board ──
    let phone = f.phone();
    let (s, created) = f
        .call(
            "POST",
            &format!("/teams/{team_id}/tasks"),
            &phone,
            Some(json!({ "title": "Remember me", "goal": "A checkbox.", "role": "frontend", "priority": 3 })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let t3 = created["id"].as_u64().unwrap();
    let (s, moved) = f
        .call(
            "PATCH",
            &format!("/teams/{team_id}/tasks/{t3}"),
            &phone,
            Some(json!({ "priority": 0 })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{moved}");
    assert_eq!(moved["priority"], 0);
    wait_for("the lead hears about the user's task", || {
        f.messages(&lead)
            .iter()
            .any(|m| m.contains(&format!("The user added task {t3} for frontend")))
    })
    .await;
    // The idle frontend gets it on the next pass.
    launcher::tick(&f.st).await;
    wait_for("the frontend's next packet", || {
        f.messages(&fe).iter().any(|m| m.contains("Remember me"))
    })
    .await;
    let (s, _) = f
        .call(
            "PATCH",
            &format!("/teams/{team_id}/tasks/{t3}"),
            &phone,
            Some(json!({ "status": "cancelled" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    wait_for("the frontend hears it was cancelled", || {
        f.messages(&fe)
            .iter()
            .any(|m| m.contains(&format!("Task {t3} was cancelled")))
    })
    .await;
    // Templates: a phone may write them, a local script may not.
    let (s, _) = f
        .call(
            "POST",
            "/team-templates/software-delivery/duplicate",
            &phone,
            Some(json!({ "name": "Shop delivery" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = f
        .call(
            "POST",
            "/team-templates/research/duplicate",
            &f.st.auth.script_token,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // ── idle stop, then resume for new work ──
    let now = mother_claude_lib::claude::registry::now_ms();
    launcher::tick(&f.st).await; // the frontend is idle with nothing to do
    f.st.teams.with_runtime(&team_id, "frontend", |r| {
        r.idle_since = Some(now - 31 * 60_000);
    });
    launcher::tick(&f.st).await;
    wait_for("the idle frontend stopped", || !f.st.control.controls(&fe)).await;
    // A transcript on disk makes it resumable (the fake writes none).
    let transcript = f.st.home.transcript_path(&f.project(), &fe);
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(&transcript, "").unwrap();
    json_of(
        &f.tool(
            Some(&lead),
            "task_create",
            json!({ "title": "Logout", "goal": "A logout button.", "role": "frontend" }),
        )
        .await,
    );
    launcher::tick(&f.st).await;
    wait_for("the frontend resumed", || {
        f.st.control.controls(&fe) && f.argv_of(&fe).iter().any(|a| a == "--resume")
    })
    .await;
    let argv = f.argv_of(&fe);
    assert_eq!(
        flag(&argv, "--resume").as_deref(),
        Some(fe.as_str()),
        "{argv:?}"
    );
    assert!(
        !argv.iter().any(|a| a == "-w"),
        "a resume finds its worktree itself"
    );

    // ── stopping the team stops every role session ──
    let (s, view) = f
        .call(
            "POST",
            &format!("/teams/{team_id}/stop"),
            &f.desktop(),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(view["stopped"], true);
    for id in [&lead, &fe, &be] {
        wait_for("a role session to stop", || !f.st.control.controls(id)).await;
    }
    launcher::tick(&f.st).await;
    assert!(
        !f.st.control.controls(&lead),
        "a stopped team starts nothing"
    );
    let (s, _) = f
        .call("DELETE", &format!("/teams/{team_id}"), &f.desktop(), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(Path::new(&f.data.join("teams.json")).is_file());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starting_a_team_checks_the_folder_the_template_and_the_rights() {
    let f = boot().await;
    // A bypass role needs the dangerous right: a phone without the grant
    // cannot start such a team.
    let (s, t) = f
        .call(
            "POST",
            "/team-templates/research/duplicate",
            &f.desktop(),
            Some(json!({ "name": "Fast research" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let mut template = t.clone();
    template["roles"][1]["permissionMode"] = json!("bypassPermissions");
    let id = template["id"].as_str().unwrap().to_string();
    let (s, saved) = f
        .call(
            "PUT",
            &format!("/team-templates/{id}"),
            &f.desktop(),
            Some(template),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    let (s, err) = f
        .call(
            "POST",
            "/teams",
            &f.phone(),
            Some(json!({ "project": f.project(), "template": id })),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{err}");
    // A folder that is not a git repository cannot hold worktree roles.
    let plain = f.project.parent().unwrap().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let (s, err) = f
        .call(
            "POST",
            "/teams",
            &f.desktop(),
            Some(json!({ "project": plain.to_string_lossy(), "template": "software-delivery" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["error"]["code"], "not_a_git_repository");
    // Overrides are checked, then applied.
    let (s, err) = f
        .call(
            "POST",
            "/teams",
            &f.desktop(),
            Some(json!({
                "project": f.project(),
                "template": "software-delivery",
                "overrides": { "frontend": { "effort": "ludicrous" } }
            })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    let (s, team) = f
        .call(
            "POST",
            "/teams",
            &f.desktop(),
            Some(json!({
                "project": f.project(),
                "template": "software-delivery",
                "overrides": { "frontend": { "model": "opus" }, "lead": { "autostart": false } }
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let fe = team["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "frontend")
        .unwrap();
    assert_eq!(fe["model"], "opus");
    // Validation answers without saving.
    let (s, v) = f
        .call(
            "POST",
            "/team-templates/validate",
            &f.phone(),
            Some(json!({ "id": "x", "name": "X", "roles": [] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["path"], "roles");
    // The templates list carries the built-ins and the copy.
    let (_, list) = f.call("GET", "/team-templates", &f.phone(), None).await;
    let ids: Vec<&str> = list["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"software-delivery") && ids.contains(&id.as_str()));
}
