//! Teams against the REAL `claude` (docs/TEAMS.md "Spike"): a role session
//! launched by Mother Claude's own code, with its name, `crossSessionInbound:
//! accept` and the guard's `initialize` callback, in a throwaway git
//! repository — the guard denies a write outside the role's paths, and a
//! `claude -p` lead's `SendMessage` starts the worker's turn unattended.
//!
//! `#[ignore]`d: it spends a few cents (Haiku, `--max-budget-usd 0.25` per
//! session) and needs `claude` signed in. Every child runs with
//! `MOTHER_CLAUDE_OWNED=1`, so an installed Mother Claude plugin stays
//! quiet. Run it with
//! `cargo test --manifest-path src-tauri/Cargo.toml --test teams_live -- --ignored`.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mother_claude_lib::claude::control::SpawnOptions;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::orchestration::launcher::ROLE_SETTINGS;
use mother_claude_lib::orchestration::teams::{Starter, Team};
use mother_claude_lib::orchestration::templates::{Integration, RoleDef, Template};
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::state::{Inner, ServerConfig};

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "live")
        .env("GIT_AUTHOR_EMAIL", "live@example.invalid")
        .env("GIT_COMMITTER_NAME", "live")
        .env("GIT_COMMITTER_EMAIL", "live@example.invalid")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "spends tokens: needs the real claude CLI, signed in"]
async fn a_real_role_session_is_guarded_and_takes_a_leads_message() {
    std::env::remove_var("MOTHER_CLAUDE_CLI");
    let home = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = mother_claude_lib::claude::paths::canonical_or_given(work.path()).join("live");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("README.md"), "# live\n").unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);

    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(data.path().to_path_buf());
    // The Default account as the app finds it (the real sign-in).
    let st = Inner::new(
        ClaudeHome::for_account_dir(home.path().to_path_buf()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
    );
    let tag = &uuid::Uuid::new_v4().simple().to_string()[..6];
    let label = format!("live-{tag}");
    let worker_id = uuid::Uuid::new_v4().to_string();
    let lead_role = RoleDef {
        id: "lead".into(),
        title: "Lead".into(),
        lead: true,
        ..Default::default()
    };
    let worker_role = RoleDef {
        id: "worker".into(),
        title: "Worker".into(),
        paths: vec!["src/**".into()],
        ..Default::default()
    };
    let team = Team {
        id: "tm_1234567890ab".into(),
        project: repo.to_string_lossy().into_owned(),
        label: label.clone(),
        template_id: "live".into(),
        template: Template {
            id: "live".into(),
            name: "Live".into(),
            roles: vec![lead_role, worker_role],
            integration: Integration::default(),
            ..Default::default()
        },
        account: "default".into(),
        overrides: BTreeMap::new(),
        created_at: 0,
        stopped: false,
        starter: Starter::default(),
        roles: BTreeMap::new(),
        brief: None,
        extra: BTreeMap::new(),
    };
    st.teams.insert(team);
    st.teams
        .bind("tm_1234567890ab", "worker", &worker_id, false, 0);

    let worker_name = format!("{label}/worker");
    let id = st
        .control
        .spawn(
            &st,
            SpawnOptions {
                cwd: repo.to_string_lossy().into_owned(),
                model: Some("haiku".into()),
                permission_mode: Some("acceptEdits".into()),
                session_id: Some(worker_id.clone()),
                name: Some(worker_name.clone()),
                settings_json: Some(ROLE_SETTINGS.into()),
                max_budget_usd: Some(0.25),
                guard_hooks: true,
                prompt: "Use the Write tool to create the file forbidden.txt in the current \
                         folder, containing the word x. If it is refused, say so in one line."
                    .into(),
                quiet: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(id, worker_id);
    let idle = |st: &mother_claude_lib::state::AppState| {
        st.control
            .state(&worker_id)
            .is_some_and(|l| l.started && !l.turn_active)
    };
    for _ in 0..1200 {
        if idle(&st) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(idle(&st), "the worker's first turn ended");
    assert!(
        !repo.join("forbidden.txt").exists(),
        "the guard denied the write"
    );
    assert!(
        st.devices
            .audit
            .recent(50)
            .iter()
            .any(|e| e.action == "team.guard.deny"),
        "and audited it"
    );

    // A lead in another `claude -p` messages the worker by name: its turn
    // starts unattended (crossSessionInbound: accept).
    let turned = Arc::new(AtomicBool::new(false));
    {
        let (st, worker_id, turned) = (st.clone(), worker_id.clone(), turned.clone());
        tokio::spawn(async move {
            for _ in 0..2400 {
                if st.control.state(&worker_id).is_some_and(|l| l.turn_active) {
                    turned.store(true, Ordering::SeqCst);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        });
    }
    let lead = tokio::process::Command::new("claude")
        // `--allowedTools` takes several values: never right before the
        // prompt.
        .args([
            "-p",
            "--allowedTools",
            "SendMessage",
            "-n",
            &format!("{label}/lead"),
            "--model",
            "haiku",
            "--max-budget-usd",
            "0.25",
            "--permission-mode",
            "default",
            &format!(
                "Use SendMessage to send the session named {worker_name} this exact text: \
                 'Reply with the single word OK.' Do not subscribe to anything. Then stop."
            ),
        ])
        .current_dir(&repo)
        .env("MOTHER_CLAUDE_OWNED", "1")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .output()
        .await
        .unwrap();
    assert!(
        lead.status.success(),
        "{}",
        String::from_utf8_lossy(&lead.stderr)
    );
    for _ in 0..600 {
        if turned.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        turned.load(Ordering::SeqCst),
        "the lead's message started a worker turn Mother Claude saw"
    );
    let _ = st.control.kill(&worker_id).await;
}
