//! The session tools' parts: the first line of a message, finding a session,
//! reading a transcript back, the card and the permission that answers it.

use std::time::Duration;

use serde_json::json;

use super::approve::{self, Action, Card};
use super::format::{self, Kind};
use super::target::{self, Me, Place, Reach, Target};
use crate::claude::model::{
    PendingKind, PendingSource, RuleKind, RuleScope, UiEvent, UiKind, UiRole, UiTool,
};
use crate::pending::{self, Decision, Remember};
use crate::server::context::{ClientContext, Principal};

const ME: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";
const OTHER: &str = "1a2b3c4d-5e6f-4a0b-8c1d-2e3f4a5b6c7d";
const THIRD: &str = "2b3c4d5e-6f70-4a1b-8c2d-3e4f5a6b7c8d";

fn me() -> Me {
    Me {
        session: ME.into(),
        name: "Docs writer".into(),
        project: Some("/tmp/proj".into()),
        project_name: Some("proj".into()),
        computer: "Studio Mac".into(),
    }
}

fn peer() -> Place {
    Place::Peer {
        id: "b".repeat(64),
        name: "DESKTOP-74CV1M6".into(),
    }
}

fn target(id: &str, place: Place, live: bool, at: i64) -> Target {
    Target {
        computer: place.name_for_tests(),
        place,
        id: id.into(),
        name: "Video dubbing".into(),
        project_name: "multilingual-dubbed-video".into(),
        cwd: "/tmp/video".into(),
        live,
        status: if live { "idle" } else { "ended" }.into(),
        last_activity_at: Some(at),
        ..Default::default()
    }
}

impl Place {
    fn name_for_tests(&self) -> String {
        match self {
            Place::Here => "Studio Mac".into(),
            Place::Peer { name, .. } => name.clone(),
        }
    }
}

fn state() -> crate::state::AppState {
    crate::state::Inner::new(
        crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
        crate::state::ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        crate::server::auth::Auth::ephemeral(),
    )
}

fn desktop() -> ClientContext {
    let mut c = ClientContext::unknown();
    c.principal = Principal::Desktop;
    c
}

#[test]
fn the_first_line_says_who_it_is_from_and_how_to_answer() {
    let m = me();
    let line = format::attribution(&m, Kind::Message);
    assert!(line.starts_with("[Message from the session “Docs writer” (proj, on Studio Mac)"));
    assert!(line.contains(&format!("reply with session_send to {ME} on Studio Mac]")));
    let q = format::attribution(&m, Kind::Question);
    assert!(q.starts_with("[Question from the session"));
    assert!(q.contains("it waits for your answer"));
    let s = format::attribution(&m, Kind::Start);
    assert!(s.starts_with("[Started by the session"));
    let text = format::message(&m, Kind::Message, "  ship it \n");
    assert_eq!(text, format!("{line}\n\nship it"));
    // A name can't break the line or start a command (tmux refuses `!` and
    // `/` for a session's caller).
    let odd = Me {
        name: "a\nb\u{7}".into(),
        project_name: None,
        ..me()
    };
    let line = format::attribution(&odd, Kind::Message);
    assert!(
        !line.contains('\n') && line.contains("“ab” (on Studio Mac)"),
        "{line}"
    );
    assert!(!line.starts_with('!') && !line.starts_with('/'));
}

#[test]
fn a_session_is_named_by_its_id_a_prefix_or_its_exact_name() {
    assert!(target::names(OTHER, OTHER, None, ""));
    assert!(target::names(&OTHER.to_uppercase(), OTHER, None, ""));
    assert!(target::names("1a2b3c4d", OTHER, None, ""));
    assert!(target::names("1a2b3c", OTHER, None, ""));
    assert!(
        !target::names("1a2b3", OTHER, None, ""),
        "too short a prefix"
    );
    assert!(!target::names("2b3c4d", OTHER, None, ""), "not a prefix");
    assert!(target::names(
        " video DUBBING ",
        OTHER,
        Some("Video dubbing"),
        "x"
    ));
    assert!(target::names("x", OTHER, Some("Video dubbing"), "X"));
    assert!(!target::names("video", OTHER, Some("Video dubbing"), "x"));
}

#[test]
fn a_session_never_picks_itself_and_copies_are_told_apart() {
    let m = me();
    let err =
        target::choose(&m, ME, vec![target(ME, Place::Here, true, 1)], true, false).unwrap_err();
    assert_eq!(err.code, "self");
    let err = target::choose(&m, "nope", vec![], true, false).unwrap_err();
    assert_eq!(err.code, "not_found");

    // One conversation here (ended) and there (running): the running one.
    let here = target(OTHER, Place::Here, false, 5);
    let there = target(OTHER, peer(), true, 3);
    let t = target::choose(&m, OTHER, vec![here.clone(), there.clone()], true, false).unwrap();
    assert_eq!(t.place, peer());

    // Neither runs: a write must be told which; a read takes the newest.
    let there_ended = target(OTHER, peer(), false, 3);
    let err = target::choose(
        &m,
        OTHER,
        vec![here.clone(), there_ended.clone()],
        true,
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, "ambiguous");
    assert!(err.message.contains("say which with computer"), "{err}");
    let t = target::choose(
        &m,
        OTHER,
        vec![here.clone(), there_ended.clone()],
        false,
        false,
    )
    .unwrap();
    assert_eq!(t.place, Place::Here);

    // The lease says it lives there now: there, even to write.
    let leased = Target {
        lives_on: Some("b".repeat(64)),
        ..here.clone()
    };
    let t = target::choose(&m, OTHER, vec![leased, there_ended], true, false).unwrap();
    assert_eq!(t.place, peer());

    // Two sessions with one name: named by id instead.
    let err = target::choose(
        &m,
        "Video dubbing",
        vec![here, target(THIRD, Place::Here, true, 9)],
        false,
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, "ambiguous");
    assert!(err.message.contains(OTHER) && err.message.contains(THIRD));

    // The same conversation heard twice from one computer counts once.
    let t = target::choose(
        &m,
        OTHER,
        vec![
            target(OTHER, peer(), true, 1),
            target(OTHER, peer(), true, 2),
        ],
        true,
        true,
    )
    .unwrap();
    assert_eq!(t.place, peer());
}

#[test]
fn another_computers_summary_reads_back() {
    let v = json!({
        "id": OTHER, "name": "x", "title": "Video dubbing", "projectName": "multilingual-dubbed-video",
        "cwd": "D:\\development\\projects\\multilingual-dubbed-video", "live": true,
        "status": "working", "permissionMode": "default", "lastActivityAt": 7,
        "control": { "channel": "hooks" }
    });
    let t = target::from_value(&v, "m1", "DESKTOP").unwrap();
    assert_eq!(t.name, "Video dubbing");
    assert_eq!(t.reach, Reach::Hooks);
    assert!(t.busy());
    assert_eq!(t.computer, "DESKTOP");
    let mut ended = v.clone();
    ended["live"] = json!(false);
    assert_eq!(
        target::from_value(&ended, "m1", "D").unwrap().reach,
        Reach::Continue
    );
    assert!(target::from_value(&json!({"id": "../x"}), "m1", "D").is_none());
    let row = target::row(&t);
    assert_eq!(row["reach"], "hooks");
    assert_eq!(row["here"], false);
    assert!(row.get("waitingFor").is_none(), "nulls left out: {row}");
}

fn ev(kind: UiKind, role: UiRole, text: &str) -> UiEvent {
    UiEvent {
        kind,
        role,
        text: Some(text.into()),
        ..Default::default()
    }
}

#[test]
fn a_read_counts_messages_and_keeps_the_tools_between_them() {
    let tool = UiEvent {
        kind: UiKind::Tool,
        tool: Some(UiTool {
            name: "Bash".into(),
            summary: "cargo test".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let events = vec![
        ev(UiKind::Prompt, UiRole::User, "first"),
        ev(UiKind::Text, UiRole::Assistant, "one"),
        tool.clone(),
        ev(UiKind::Thinking, UiRole::Assistant, "hidden"),
        ev(UiKind::Text, UiRole::Assistant, "two"),
        ev(UiKind::Prompt, UiRole::User, "second"),
        ev(UiKind::Text, UiRole::Assistant, "three"),
    ];
    let plain = format::lines(&events, false);
    assert_eq!(plain.len(), 5, "no thinking, no tools");
    let last = format::last_messages(&plain, 2);
    assert_eq!(last.len(), 2);
    assert!(last[0].contains("second") && last[1].contains("three"));
    let with_tools = format::lines(&events, true);
    let last = format::last_messages(&with_tools, 3);
    assert_eq!(last.len(), 3);
    let all = format::last_messages(&with_tools, 4);
    assert_eq!(all.len(), 5, "the tool between two messages comes along");
    assert!(all[1].contains("[tool Bash] cargo test"), "{all:?}");
    let t = target(OTHER, Place::Here, true, 0);
    let read = format::render(&t, &plain, 10);
    assert!(read.starts_with("Session “Video dubbing” (multilingual-dubbed-video, on Studio Mac)"));
    assert!(read.contains(&format!("Session id: {OTHER}")));
    assert!(read.ends_with("[assistant]\nthree"), "{read}");
    assert!(format::render(&t, &[], 3).ends_with("No messages yet."));
}

#[test]
fn an_answer_is_what_it_wrote_after_the_question() {
    let events = vec![
        ev(UiKind::Prompt, UiRole::User, "old"),
        ev(UiKind::Text, UiRole::Assistant, "old answer"),
        ev(
            UiKind::Prompt,
            UiRole::User,
            "[Question from …]\n\nwhat now?",
        ),
        ev(UiKind::Text, UiRole::Assistant, "Let me look."),
        ev(UiKind::Text, UiRole::Assistant, "Ship it."),
    ];
    let after = format::after_last_prompt(events);
    assert_eq!(after.len(), 2);
    assert_eq!(
        format::answer_of(&after).as_deref(),
        Some("Let me look.\n\nShip it.")
    );
    assert_eq!(format::answer_of(&[]), None);
    assert_eq!(format::keep_end("abcdef", 4), "…def");
    assert_eq!(format::keep_end("abc", 4), "abc");
}

fn send_card<'a>(m: &'a Me, t: &'a Target, dangerous: Option<String>) -> Card<'a> {
    Card {
        action: Action::Send,
        me: m,
        target: Some(t),
        computer: t.computer.clone(),
        machine: t.place.machine().map(str::to_string),
        folder: Some(t.cwd.clone()),
        project_name: Some(t.project_name.clone()),
        message: Some("ship the docs"),
        last: None,
        dangerous,
    }
}

#[test]
fn the_card_names_the_action_and_offers_talking_to_sessions() {
    let m = me();
    let t = target(OTHER, Place::Here, true, 0);
    let item = approve::item(&send_card(&m, &t, None));
    assert_eq!(item.session_id, ME, "on the calling session");
    assert_eq!(item.source, PendingSource::Session);
    assert_eq!(item.kind, PendingKind::Permission);
    assert_eq!(item.tool_name.as_deref(), Some("session_send"));
    assert_eq!(item.title, "Message the session “Video dubbing”");
    assert!(!item.dangerous);
    let input = item.input.clone().unwrap();
    assert_eq!(input["action"], "send");
    assert_eq!(input["session"], OTHER);
    assert_eq!(input["message"], "ship the docs");
    assert_eq!(
        input["project"], "/tmp/proj",
        "the asking session's project"
    );
    assert_eq!(input["here"], true);
    assert!(input.get("machine").is_none());
    let offer = item.grant.unwrap();
    assert_eq!(offer.kind, RuleKind::Sessions);
    assert_eq!(offer.session, ME);
    assert_eq!(offer.project.as_deref(), Some("/tmp/proj"));
    assert!(offer.blocked.is_none() && offer.machine.is_none() && offer.patterns.is_none());

    let there = target(OTHER, peer(), true, 0);
    let item = approve::item(&send_card(&m, &there, Some("it bypasses".into())));
    assert_eq!(
        item.title,
        "Message the session “Video dubbing” on DESKTOP-74CV1M6"
    );
    assert!(item.dangerous);
    assert_eq!(item.dangerous_reason.as_deref(), Some("it bypasses"));
    assert!(item.grant.unwrap().blocked.is_some(), "never remembered");
}

async fn waiting_card(st: &crate::state::AppState) -> crate::claude::model::PendingItem {
    for _ in 0..200 {
        if let Some(i) = st
            .pending
            .list(ME)
            .into_iter()
            .find(|i| i.source == PendingSource::Session)
        {
            return i;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no card came");
}

#[tokio::test]
async fn a_card_waits_for_a_person_and_a_saved_permission_answers_the_next() {
    let st = state();
    let m = me();
    let t = target(OTHER, Place::Here, true, 0);

    // A card, allowed with "for this session".
    let task = {
        let (st, m, t) = (st.clone(), m.clone(), t.clone());
        tokio::spawn(async move { approve::approve(&st, send_card(&m, &t, None)).await })
    };
    let card = waiting_card(&st).await;
    let allow = Decision {
        remember: Some(Remember {
            scope: RuleScope::Session,
            patterns: Vec::new(),
        }),
        ..Decision::allow()
    };
    pending::respond(&st, ME, &card.id, allow, &desktop())
        .await
        .unwrap();
    let allowed = task.await.unwrap().unwrap();
    assert!(allowed.rule.is_none() && !allowed.dangerous);
    let (rules, _) = st.rules.list();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].kind, RuleKind::Sessions);
    assert_eq!(rules[0].scope, RuleScope::Session);
    assert!(rules[0].machine.is_none() && rules[0].tool.is_none());

    // The next one: no card.
    let allowed = approve::approve(&st, send_card(&m, &t, None))
        .await
        .unwrap();
    assert_eq!(allowed.rule.as_deref(), Some(rules[0].id.as_str()));
    assert_eq!(
        allowed.by.as_deref(),
        Some("your permission (talking to other sessions from this session)")
    );
    assert!(st.pending.list(ME).is_empty());

    // A dangerous one asks all the same; denied, it says so.
    let task = {
        let (st, m, t) = (st.clone(), m.clone(), t.clone());
        tokio::spawn(async move {
            approve::approve(&st, send_card(&m, &t, Some("bypasses".into()))).await
        })
    };
    let card = waiting_card(&st).await;
    assert!(card.dangerous);
    pending::respond(
        &st,
        ME,
        &card.id,
        Decision::deny(Some("not now".into())),
        &desktop(),
    )
    .await
    .unwrap();
    let err = task.await.unwrap().unwrap_err();
    assert_eq!(err.code, "denied");
    assert!(err.message.contains("not now"), "{err}");
}

#[tokio::test]
async fn a_permission_saved_for_the_project_settles_the_cards_still_waiting() {
    let st = state();
    let m = me();
    let a = target(OTHER, Place::Here, true, 0);
    let b = target(THIRD, Place::Here, true, 0);
    let first = {
        let (st, m, a) = (st.clone(), m.clone(), a.clone());
        tokio::spawn(async move { approve::approve(&st, send_card(&m, &a, None)).await })
    };
    let card = waiting_card(&st).await;
    let second = {
        let (st, m, b) = (st.clone(), m.clone(), b.clone());
        tokio::spawn(async move { approve::approve(&st, send_card(&m, &b, None)).await })
    };
    for _ in 0..200 {
        if st.pending.list(ME).len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(st.pending.list(ME).len(), 2);
    let allow = Decision {
        remember: Some(Remember {
            scope: RuleScope::Project,
            patterns: Vec::new(),
        }),
        ..Decision::allow()
    };
    pending::respond(&st, ME, &card.id, allow, &desktop())
        .await
        .unwrap();
    assert!(first.await.unwrap().is_ok());
    assert!(second.await.unwrap().is_ok(), "the other card is covered");
    let (rules, _) = st.rules.list();
    assert_eq!(rules[0].scope, RuleScope::Project);
    assert_eq!(
        crate::pending::rules::describe(&rules[0]),
        "talking to other sessions from proj"
    );
}

#[test]
fn every_tool_is_registered_with_a_schema() {
    let r = crate::mcp::registry();
    for name in [
        "session_list",
        "session_read",
        "session_send",
        "session_ask",
        "session_start",
    ] {
        let t = r.get(name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(t.def.input_schema["type"], "object", "{name}");
        assert!(t.def.description.len() > 80, "{name}");
    }
    assert!(r.get("session_list").unwrap().limit.is_none());
    assert!(r.get("session_ask").unwrap().limit.unwrap() > Duration::from_secs(30 * 60));
}

#[tokio::test]
async fn tools_need_to_know_who_calls_them() {
    let st = state();
    let r = crate::mcp::registry();
    let ctx = crate::mcp::ToolCtx {
        state: st.clone(),
        caller: crate::mcp::Caller::default(),
    };
    let out = r
        .call(
            "session_send",
            ctx.clone(),
            json!({ "session": OTHER, "message": "hi" }),
        )
        .await
        .unwrap();
    assert!(out.is_error);
    let out = r
        .call("session_send", ctx.clone(), json!({ "session": OTHER }))
        .await
        .unwrap();
    assert!(out.is_error, "no message");
    // Listing works without knowing the caller.
    let out = r.call("session_list", ctx, json!({})).await.unwrap();
    assert!(!out.is_error, "{out:?}");
    assert!(out.structured_content.unwrap()["sessions"].is_array());
}
