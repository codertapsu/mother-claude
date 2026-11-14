use std::time::Duration;

use axum::http::HeaderMap;
use serde_json::json;

use super::*;
use crate::claude::model::ResolverKind;
use crate::pending::{Decision, Remember};
use crate::server::context::{ClientContext, Ingress, Principal};
use crate::state::AppState;

const MAC: &str = "0123456789abcdef0123456789abcdef";
const PC: &str = "fedcba9876543210fedcba9876543210";

fn rule(kind: RuleKind, tool: Option<&str>, patterns: &[&str], scope: RuleScope) -> Rule {
    Rule {
        id: new_rule_id(),
        kind,
        tool: tool.map(str::to_string),
        patterns: patterns.iter().map(|p| p.to_string()).collect(),
        scope,
        session: Some("s1".into()),
        session_name: "Fix the tests".into(),
        project: Some("/Users/me/shop".into()),
        project_name: "shop".into(),
        machine: matches!(kind, RuleKind::Run | RuleKind::Task).then(|| MAC.to_string()),
        machine_name: "MacBook".into(),
        created_at: now(),
        created_by: "Mac".into(),
        used_at: None,
        uses: 0,
    }
}

fn bash_ask<'a>(session: &'a str, project: Option<&'a str>, command: &'a str) -> Ask<'a> {
    Ask {
        kind: RuleKind::Tool,
        tool: Some("Bash"),
        session,
        folder: project,
        project,
        machine: None,
        subject: Some(command),
    }
}

#[test]
fn tools_belong_to_families_and_each_says_what_its_patterns_name() {
    assert_eq!(family_of("Write"), "Edit");
    assert_eq!(family_of("MultiEdit"), "Edit");
    assert_eq!(family_of("Grep"), "Read");
    assert_eq!(family_of("Bash"), "Bash");
    assert_eq!(
        family_of("mcp__github__create_issue"),
        "mcp__github__create_issue"
    );
    assert_eq!(
        pattern_kind(RuleKind::Tool, Some("Bash")),
        Some(PatternKind::Command)
    );
    assert_eq!(
        pattern_kind(RuleKind::Tool, Some("Edit")),
        Some(PatternKind::Path)
    );
    assert_eq!(
        pattern_kind(RuleKind::Tool, Some("WebFetch")),
        Some(PatternKind::Domain)
    );
    assert_eq!(pattern_kind(RuleKind::Tool, Some("WebSearch")), None);
    assert_eq!(
        pattern_kind(RuleKind::Run, None),
        Some(PatternKind::Command)
    );
    assert_eq!(pattern_kind(RuleKind::Task, None), None);
    assert_eq!(
        subject_of("WebFetch", &json!({"url": "https://Docs.RS./tokio/latest"})).as_deref(),
        Some("docs.rs")
    );
    assert_eq!(
        subject_of("Write", &json!({"file_path": "/a/b.rs", "content": "x"})).as_deref(),
        Some("/a/b.rs")
    );
    assert_eq!(subject_of("Bash", &json!({"command": "  "})), None);
}

#[test]
fn patterns_are_cleaned_and_the_ones_that_would_cover_anything_refused() {
    let c = |p: &str| clean_pattern(PatternKind::Command, p);
    assert_eq!(c("  cargo   test ").as_deref(), Ok("cargo test"));
    assert_eq!(c("cargo test *").as_deref(), Ok("cargo test"));
    assert_eq!(c("npm run build*").as_deref(), Ok("npm run build"));
    assert!(c("cargo test; rm -rf ~").is_err());
    assert!(c("bash").unwrap_err().contains("runs whatever"));
    assert!(c("/usr/bin/sudo make").is_err());
    assert!(c("python").unwrap_err().contains("alone"));
    assert_eq!(c("python -m pytest").as_deref(), Ok("python -m pytest"));
    assert!(c("").is_err() && c("*").is_err());

    let p = |x: &str| clean_pattern(PatternKind::Path, x);
    assert_eq!(p("./src/**").as_deref(), Ok("src/**"));
    assert_eq!(p(r"docs\*.md").as_deref(), Ok("docs/*.md"));
    assert!(p("/etc/**").is_err() && p("../x").is_err() && p("~/x").is_err());
    assert!(p(r"C:\Windows\**").is_err());
    assert!(p("**/**/**/**/**/x").is_err());

    let d = |x: &str| clean_pattern(PatternKind::Domain, x);
    assert_eq!(d("Docs.RS").as_deref(), Ok("docs.rs"));
    assert_eq!(d("*.github.com").as_deref(), Ok("*.github.com"));
    assert_eq!(
        d("https://api.github.com/repos/x").as_deref(),
        Ok("api.github.com")
    );
    assert_eq!(d("localhost").as_deref(), Ok("localhost"));
    assert!(d("not a site").is_err() && d("-x.com").is_err() && d("com").is_err());

    assert_eq!(
        clean_patterns(
            Some(PatternKind::Command),
            &["cargo test".into(), "cargo  test".into()]
        ),
        Ok(vec!["cargo test".to_string()])
    );
    assert!(clean_patterns(None, &["x".into()]).is_err());
    let many: Vec<String> = (0..21).map(|i| format!("make t{i}")).collect();
    assert!(clean_patterns(Some(PatternKind::Command), &many).is_err());
}

#[test]
fn a_rule_applies_to_its_kind_place_and_patterns_only() {
    let shop = "/Users/me/shop";
    // Bash in one session, commands like `cargo test`.
    let r = rule(
        RuleKind::Tool,
        Some("Bash"),
        &["cargo test"],
        RuleScope::Session,
    );
    assert!(applies(&r, &bash_ask("s1", Some(shop), "cargo test --lib")));
    assert!(
        !applies(&r, &bash_ask("s2", Some(shop), "cargo test")),
        "another session"
    );
    assert!(!applies(&r, &bash_ask("s1", Some(shop), "cargo test-foo")));
    assert!(!applies(
        &r,
        &bash_ask("s1", Some(shop), "cargo test; rm -rf ~")
    ));
    let pwsh = Ask {
        tool: Some("PowerShell"),
        ..bash_ask("s1", Some(shop), "cargo test")
    };
    assert!(!applies(&r, &pwsh), "another tool");

    // Every session of the project, any command.
    let r = rule(RuleKind::Tool, Some("Bash"), &[], RuleScope::Project);
    assert!(applies(&r, &bash_ask("s9", Some(shop), "anything at all")));
    assert!(applies(&r, &bash_ask("s9", Some("/Users/me/shop/"), "x")));
    assert!(!applies(&r, &bash_ask("s9", Some("/Users/me/other"), "x")));
    assert!(
        !applies(&r, &bash_ask("s9", None, "x")),
        "no project: never by project"
    );

    // File edits: Write is one; patterns are relative to the session's folder.
    let r = rule(
        RuleKind::Tool,
        Some("Edit"),
        &["src/**"],
        RuleScope::Computer,
    );
    let write = |file: &'static str, folder: &'static str| Ask {
        kind: RuleKind::Tool,
        tool: Some("Write"),
        session: "s7",
        folder: Some(folder),
        project: Some(folder),
        machine: None,
        subject: Some(file),
    };
    assert!(applies(&r, &write("/w/app/src/main.rs", "/w/app")));
    assert!(
        applies(&r, &write("src/lib/x.rs", "/w/app")),
        "relative to the folder"
    );
    assert!(!applies(&r, &write("/w/app/docs/a.md", "/w/app")));
    assert!(
        !applies(&r, &write("/w/app/src/../../etc/x", "/w/app")),
        "never outside"
    );
    assert!(!applies(&r, &write("/elsewhere/src/a.rs", "/w/app")));

    // Sites.
    let r = rule(
        RuleKind::Tool,
        Some("WebFetch"),
        &["*.github.com", "docs.rs"],
        RuleScope::Computer,
    );
    let fetch = |host: &'static str| Ask {
        kind: RuleKind::Tool,
        tool: Some("WebFetch"),
        session: "s1",
        folder: None,
        project: None,
        machine: None,
        subject: Some(host),
    };
    assert!(applies(&r, &fetch("api.github.com")));
    assert!(applies(&r, &fetch("docs.rs")));
    assert!(!applies(&r, &fetch("github.com")), "*. is the subdomains");
    assert!(!applies(&r, &fetch("evilgithub.com")));
    assert!(!applies(&r, &fetch("docs.rs.evil.com")));

    // Runs: the computer they run on.
    let r = rule(RuleKind::Run, None, &["cargo test"], RuleScope::Computer);
    let run = |machine: &'static str, cmd: &'static str| Ask {
        kind: RuleKind::Run,
        tool: None,
        session: "s1",
        folder: Some(shop),
        project: Some(shop),
        machine: Some(machine),
        subject: Some(cmd),
    };
    assert!(applies(&r, &run(MAC, "cargo test --lib")));
    assert!(!applies(&r, &run(PC, "cargo test")), "another computer");
    assert!(
        !applies(&r, &bash_ask("s1", Some(shop), "cargo test")),
        "not a tool call"
    );
    // Tasks take no patterns.
    let t = rule(RuleKind::Task, None, &[], RuleScope::Project);
    let task = Ask {
        kind: RuleKind::Task,
        subject: None,
        ..run(MAC, "")
    };
    assert!(applies(&t, &task));
}

#[test]
fn a_rule_says_what_it_allows() {
    let r = rule(
        RuleKind::Tool,
        Some("Bash"),
        &["cargo test"],
        RuleScope::Project,
    );
    assert_eq!(describe(&r), "commands like cargo test in shop");
    let r = rule(RuleKind::Tool, Some("Edit"), &[], RuleScope::Session);
    assert_eq!(describe(&r), "file edits in this session");
    let r = rule(RuleKind::Run, None, &[], RuleScope::Computer);
    assert_eq!(describe(&r), "commands on MacBook from any project");
    let r = rule(RuleKind::Tool, Some("mcp__x__y"), &[], RuleScope::Computer);
    assert!(describe(&r).starts_with("mcp__x__y calls everywhere on this "));
}

#[test]
fn the_store_finds_the_narrowest_rule_counts_it_and_keeps_rules_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Rules::load(Some(dir.path()));
    let wide = store.add(rule(RuleKind::Tool, Some("Bash"), &[], RuleScope::Computer));
    let narrow = store.add(rule(
        RuleKind::Tool,
        Some("Bash"),
        &["cargo test"],
        RuleScope::Session,
    ));
    // The same thing again: the one there.
    let again = store.add(rule(
        RuleKind::Tool,
        Some("Bash"),
        &["cargo test"],
        RuleScope::Session,
    ));
    assert_eq!(again.id, narrow.id);
    assert_eq!(store.list().0.len(), 2);

    let hit = store
        .covering(&bash_ask("s1", None, "cargo test --lib"))
        .unwrap();
    assert_eq!(hit.id, narrow.id, "the narrowest");
    assert_eq!(hit.uses, 1);
    let hit = store.covering(&bash_ask("s2", None, "ls")).unwrap();
    assert_eq!(hit.id, wide.id);
    assert!(store.would_cover(&bash_ask("s3", None, "ls")));

    // Paused: nothing is allowed, nothing lost.
    store.set_paused(true);
    assert!(store
        .covering(&bash_ask("s1", None, "cargo test"))
        .is_none());
    assert!(!store.would_cover(&bash_ask("s1", None, "cargo test")));
    let back = Rules::load(Some(dir.path()));
    assert!(back.paused());
    assert_eq!(back.list().0.len(), 2);
    back.set_paused(false);

    // Changing one: narrower is not widening; more is.
    let (r, widened) = back
        .update(&wide.id, Some(&["npm test".into()]), None)
        .unwrap();
    assert!(!widened && r.patterns == ["npm test"]);
    let (_, widened) = back
        .update(&wide.id, Some(&["npm test".into(), "make".into()]), None)
        .unwrap();
    assert!(widened, "a pattern added");
    let (_, widened) = back
        .update(&narrow.id, None, Some(RuleScope::Project))
        .unwrap();
    assert!(widened, "a wider place");
    assert!(matches!(
        back.update(&narrow.id, Some(&["bash".into()]), None),
        Err(RuleError::Invalid(_))
    ));
    assert_eq!(
        back.update("pr_000000000000", None, None).unwrap_err(),
        RuleError::NotFound
    );

    assert!(back.remove(&wide.id).is_some());
    assert!(back.remove(&wide.id).is_none());
    assert_eq!(Rules::load(Some(dir.path())).list().0.len(), 1);
}

#[test]
fn unreadable_rules_are_skipped_and_old_session_rules_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let old = Rule {
        created_at: 1,
        ..rule(RuleKind::Tool, Some("Bash"), &[], RuleScope::Session)
    };
    let kept = rule(RuleKind::Tool, Some("Bash"), &[], RuleScope::Project);
    std::fs::write(
        dir.path().join(FILE),
        json!({
            "version": 1,
            "rules": [
                old,
                kept,
                {"id": "nope", "kind": "tool"},
                {"id": "pr_0123456789ab", "kind": "run", "scope": "computer", "createdAt": 1},
                {"id": "pr_0123456789ac", "kind": "tool", "tool": "Bash", "scope": "computer",
                 "patterns": ["bash"], "createdAt": 1}
            ]
        })
        .to_string(),
    )
    .unwrap();
    let store = Rules::load(Some(dir.path()));
    let ids: Vec<String> = store.list().0.into_iter().map(|r| r.id).collect();
    assert_eq!(ids, [kept.id]);
}

#[test]
fn the_old_standing_grants_for_runs_become_rules_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("machines")).unwrap();
    std::fs::write(
        dir.path().join("machines").join("run-grants.json"),
        json!({"version": 1, "grants": [
            {"id": "rg_0123456789ab", "project": "/Users/me/shop", "projectName": "shop",
             "machine": MAC, "prefix": "cargo test", "createdAt": 5, "createdBy": "Mac", "uses": 2}
        ]})
        .to_string(),
    )
    .unwrap();
    let store = Rules::load(Some(dir.path()));
    let (list, _) = store.list();
    assert_eq!(list.len(), 1);
    let r = &list[0];
    assert_eq!(
        (
            r.kind,
            r.scope,
            r.patterns.clone(),
            r.machine.as_deref(),
            r.uses
        ),
        (
            RuleKind::Run,
            RuleScope::Project,
            vec!["cargo test".to_string()],
            Some(MAC),
            2
        )
    );
    // Taken over once: removing it sticks.
    store.remove(&r.id);
    assert!(Rules::load(Some(dir.path())).list().0.is_empty());
}

#[test]
fn a_project_is_its_repository_and_a_worktree_belongs_to_its_project() {
    let dir = tempfile::tempdir().unwrap();
    let shop = dir.path().join("shop");
    std::fs::create_dir_all(shop.join(".git")).unwrap();
    std::fs::create_dir_all(shop.join("src").join("app")).unwrap();
    let wt = shop.join(".claude").join("worktrees").join("frontend");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(wt.join(".git"), "gitdir: x").unwrap();
    let top = shop.display().to_string();
    assert_eq!(
        project_of(&shop.join("src").join("app").display().to_string()),
        top
    );
    assert_eq!(project_of(&wt.display().to_string()), top);
    let plain = dir.path().join("notes");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(
        project_of(&plain.display().to_string()),
        plain.display().to_string(),
        "no repository: the folder itself"
    );
}

#[test]
fn a_rule_is_made_from_what_the_card_offers() {
    let offer = GrantOffer {
        kind: RuleKind::Tool,
        tool: Some("Bash".into()),
        patterns: Some(PatternKind::Command),
        suggested: vec!["cargo test".into()],
        session: "s1".into(),
        session_name: Some("Fix".into()),
        project: Some("/Users/me/shop".into()),
        project_name: Some("shop".into()),
        ..Default::default()
    };
    let r = rule_from_offer(&offer, RuleScope::Project, &["cargo test *".into()], "Mac").unwrap();
    assert_eq!(r.patterns, ["cargo test"]);
    assert_eq!(r.tool.as_deref(), Some("Bash"));
    assert!(is_rule_id(&r.id));
    let no_project = GrantOffer {
        project: None,
        ..offer.clone()
    };
    assert!(rule_from_offer(&no_project, RuleScope::Project, &[], "Mac").is_err());
    assert!(rule_from_offer(&no_project, RuleScope::Computer, &[], "Mac").is_ok());
    let blocked = GrantOffer {
        blocked: Some("Dangerous.".into()),
        ..offer.clone()
    };
    assert!(rule_from_offer(&blocked, RuleScope::Session, &[], "Mac").is_err());
    let task = GrantOffer {
        kind: RuleKind::Task,
        tool: None,
        patterns: None,
        machine: Some(MAC.into()),
        ..offer
    };
    assert!(rule_from_offer(&task, RuleScope::Session, &["x".into()], "Mac").is_err());
}

// ─────────────────────────────── with the pending store ───────────────────────────────

fn state() -> AppState {
    crate::state::Inner::new(
        crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
        crate::state::ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        crate::server::auth::Auth::ephemeral(),
    )
}

fn bash(session: &str, command: &str) -> PendingItem {
    PendingItem {
        session_id: session.into(),
        kind: PendingKind::Permission,
        source: PendingSource::Owned,
        tool_name: Some("Bash".into()),
        title: format!("Bash: {command}"),
        input: Some(json!({ "command": command })),
        ..Default::default()
    }
}

fn desktop() -> ClientContext {
    let mut h = HeaderMap::new();
    h.insert("host", "127.0.0.1:6725".parse().unwrap());
    let mut c = ClientContext::new(Ingress::Loopback, "127.0.0.1:9".parse().unwrap(), &h, false);
    c.principal = Principal::Desktop;
    c
}

fn phone(allow_dangerous: bool) -> ClientContext {
    let mut h = HeaderMap::new();
    h.insert("host", "192.168.1.2:6725".parse().unwrap());
    let mut c = ClientContext::new(Ingress::Lan, "192.168.1.9:9".parse().unwrap(), &h, false);
    c.principal = Principal::Device {
        id: "d1".into(),
        allow_dangerous,
    };
    c
}

fn remember(scope: RuleScope, patterns: &[&str]) -> Decision {
    Decision {
        remember: Some(Remember {
            scope,
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
        }),
        ..Decision::allow()
    }
}

#[tokio::test]
async fn a_permission_answers_before_a_card_shows_and_a_remembered_answer_makes_one() {
    let st = state();
    // Nothing allows it yet: a card, with what it could remember.
    let (g, _rx) = crate::pending::raise(&st, bash("s1", "cargo test --lib")).await;
    let item = st.pending.get(g.id()).unwrap();
    let offer = item.grant.clone().unwrap();
    assert_eq!(offer.tool.as_deref(), Some("Bash"));
    assert_eq!(offer.patterns, Some(PatternKind::Command));
    assert_eq!(offer.suggested, ["cargo test"]);
    assert!(offer.blocked.is_none());

    // A phone without the dangerous right may remember it for the session,
    // not for the computer.
    let err = crate::pending::respond(
        &st,
        "s1",
        g.id(),
        remember(RuleScope::Computer, &[]),
        &phone(false),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, crate::pending::RespondError::Forbidden(_)));
    let bad = crate::pending::respond(
        &st,
        "s1",
        g.id(),
        remember(RuleScope::Session, &["bash"]),
        &phone(false),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        bad,
        crate::pending::RespondError::Unrememberable(_)
    ));
    crate::pending::respond(
        &st,
        "s1",
        g.id(),
        remember(RuleScope::Session, &["cargo test"]),
        &phone(false),
    )
    .await
    .unwrap();
    let (rules, _) = st.rules.list();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].scope, RuleScope::Session);
    assert_eq!(rules[0].created_by, "a paired device");
    drop(g);

    // The next matching one never shows a card.
    let (g2, rx2) = crate::pending::raise(&st, bash("s1", "cargo test -- --nocapture")).await;
    let d = tokio::time::timeout(Duration::from_secs(2), rx2)
        .await
        .unwrap()
        .unwrap();
    assert!(d.is_allow());
    assert!(st.pending.list("s1").is_empty());
    let r = st.pending.resolution(g2.id()).unwrap();
    assert_eq!(
        r.by.as_ref().map(|b| b.kind),
        Some(ResolverKind::Permission)
    );
    assert_eq!(
        r.text.as_deref(),
        Some("Allowed by your permission · commands like cargo test in this session")
    );
    assert_eq!(st.rules.list().0[0].uses, 1);

    // Another session, another command: cards.
    let (_g3, _rx3) = crate::pending::raise(&st, bash("s2", "cargo test")).await;
    let (_g4, _rx4) = crate::pending::raise(&st, bash("s1", "npm test")).await;
    assert_eq!(st.pending.list("s2").len(), 1);
    assert_eq!(st.pending.list("s1").len(), 1);

    // The desktop may allow every command everywhere — but a dangerous one
    // still asks, and its card says it can't be remembered.
    let id4 = st.pending.list("s1")[0].id.clone();
    crate::pending::respond(
        &st,
        "s1",
        &id4,
        remember(RuleScope::Computer, &[]),
        &desktop(),
    )
    .await
    .unwrap();
    let (_g5, rx5) = crate::pending::raise(&st, bash("s3", "make all")).await;
    assert!(tokio::time::timeout(Duration::from_secs(2), rx5)
        .await
        .unwrap()
        .unwrap()
        .is_allow());
    let (g6, _rx6) = crate::pending::raise(&st, bash("s3", "echo x >> ~/.zshrc")).await;
    let six = st.pending.get(g6.id()).unwrap();
    assert!(six.dangerous);
    assert!(six.grant.unwrap().blocked.is_some());

    // Paused: everything asks again.
    st.rules.set_paused(true);
    let (g7, _rx7) = crate::pending::raise(&st, bash("s3", "make all")).await;
    assert!(st.pending.get(g7.id()).is_some());
}

#[tokio::test]
async fn questions_plans_and_observed_prompts_are_never_answered_by_a_permission() {
    let st = state();
    st.rules.add(Rule {
        tool: Some("AskUserQuestion".into()),
        ..rule(RuleKind::Tool, Some("Bash"), &[], RuleScope::Computer)
    });
    let q = PendingItem {
        kind: PendingKind::Question,
        tool_name: Some("AskUserQuestion".into()),
        ..bash("s1", "")
    };
    let (g, _rx) = crate::pending::raise(&st, q).await;
    let item = st.pending.get(g.id()).unwrap();
    assert!(item.grant.is_none());
}

#[tokio::test]
async fn saving_a_permission_answers_the_cards_like_it_already_waiting() {
    let st = state();
    let (g1, _rx1) = crate::pending::raise(&st, bash("s1", "cargo test --lib")).await;
    let (_g2, rx2) = crate::pending::raise(&st, bash("s1", "cargo test --doc")).await;
    let (_g3, _rx3) = crate::pending::raise(&st, bash("s1", "rm -rf build")).await;
    let (_g4, _rx4) = crate::pending::raise(&st, bash("s2", "cargo test")).await;
    assert_eq!(st.pending.all().len(), 4);
    crate::pending::respond(
        &st,
        "s1",
        g1.id(),
        remember(RuleScope::Session, &["cargo test"]),
        &desktop(),
    )
    .await
    .unwrap();
    // The other `cargo test` of that session went ahead; the rest still ask.
    assert!(tokio::time::timeout(Duration::from_secs(2), rx2)
        .await
        .unwrap()
        .unwrap()
        .is_allow());
    let left: Vec<String> = st
        .pending
        .all()
        .into_iter()
        .map(|i| {
            format!(
                "{} {}",
                i.session_id,
                i.input.unwrap()["command"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left.contains(&"s1 rm -rf build".to_string()));
    assert!(left.contains(&"s2 cargo test".to_string()));
}
