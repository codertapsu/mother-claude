//! Reading, messaging, asking and starting sessions (`session_read`,
//! `session_send`, `session_ask`, `session_start`): here through what the
//! dashboard uses, on a paired computer through its `/api/sessions/*`
//! routes.
//!
//! **Waiting for an answer.** A session Mother Claude runs here says when
//! its turn ends (its stream's `result`, [`super::turn_ended`]), and its
//! last message with it. Any other session is watched: its status (busy,
//! then not) and its transcript from where it ended before the message
//! (for a conversation that starts with the message, a continued fork or a
//! new session: after its last prompt).

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::approve::{self, Action, Allowed, Card};
use super::format::{self, Kind};
use super::target::{self, Me, Place, Reach, Target, CALL};
use crate::claude::model::{Delivery, UiEvent};
use crate::claude::owned::{self, ContinueRequest, SpawnRequest};
use crate::claude::uievents;
use crate::machines::grants::Grant;
use crate::mcp::ToolError;
use crate::server::context::{ClientContext, Principal};
use crate::server::error::ApiError;
use crate::state::AppState;

/// Continuing or starting a session on a paired computer.
const START_CALL: Duration = Duration::from_secs(90);
/// How often a session here, or there, is looked at while waiting.
const POLL_HERE: Duration = Duration::from_millis(500);
const POLL_THERE: Duration = Duration::from_secs(3);
/// Pages a read goes back at most.
const PAGES: usize = 5;

fn api_err(e: ApiError) -> ToolError {
    ToolError::new(e.code, e.message)
}

fn internal(e: impl std::fmt::Display) -> ToolError {
    ToolError::new("internal", e.to_string())
}

// ─── transcripts ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct Page {
    events: Vec<UiEvent>,
    before: Option<String>,
    tail: String,
    reset: bool,
}

/// A page's events as another computer sent them, each read on its own (one
/// a newer build added is skipped, not the page).
fn page_of(v: &Value) -> Page {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Page {
        events: v
            .get("events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| serde_json::from_value::<UiEvent>(e.clone()).ok())
            .collect(),
        before: s("before"),
        tail: s("tail").unwrap_or_default(),
        reset: v.get("reset").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// A page of `id`'s main transcript ending before `before` (or at its end).
async fn page(
    state: &AppState,
    place: &Place,
    id: &str,
    before: Option<&str>,
    limit: usize,
) -> Result<Page, ToolError> {
    match place {
        Place::Here => {
            let Some(path) = state.transcript_path(id).await else {
                return Ok(Page {
                    tail: "0".into(),
                    ..Default::default()
                });
            };
            let before = before.and_then(uievents::parse_cursor);
            let sid = id.to_string();
            let p = tokio::task::spawn_blocking(move || {
                uievents::read_page_opts(
                    &sid,
                    &path,
                    uievents::PageOpts {
                        before,
                        limit,
                        agent: None,
                    },
                )
            })
            .await
            .map_err(internal)?
            .map_err(|e| {
                ToolError::new("unreadable", format!("Could not read its transcript: {e}"))
            })?;
            Ok(Page {
                events: p.events,
                before: p.before,
                tail: p.tail,
                reset: false,
            })
        }
        Place::Peer { id: machine, .. } => {
            let mut path = format!("/api/sessions/{id}/transcript?limit={limit}");
            if let Some(b) = before {
                let b: String = url::form_urlencoded::byte_serialize(b.as_bytes()).collect();
                path.push_str(&format!("&before={b}"));
            }
            let (s, v) = target::peer_call(state, machine, "GET", &path, None, false, CALL).await?;
            if !(200..300).contains(&s) {
                return Err(target::refused(state, machine, s, &v));
            }
            Ok(page_of(&v))
        }
    }
}

/// `id`'s events after the cursor `from`.
async fn after(state: &AppState, place: &Place, id: &str, from: &str) -> Result<Page, ToolError> {
    match place {
        Place::Here => {
            let Some(path) = state.transcript_path(id).await else {
                return Ok(Page {
                    tail: from.to_string(),
                    ..Default::default()
                });
            };
            let from_at = uievents::parse_cursor(from).unwrap_or(0);
            let sid = id.to_string();
            let r = tokio::task::spawn_blocking(move || {
                uievents::read_range(&sid, &path, from_at, None, None)
            })
            .await
            .map_err(internal)?
            .map_err(|e| {
                ToolError::new("unreadable", format!("Could not read its transcript: {e}"))
            })?;
            Ok(Page {
                events: r.events,
                before: None,
                tail: r.tail.to_string(),
                reset: r.reset,
            })
        }
        Place::Peer { id: machine, .. } => {
            let f: String = url::form_urlencoded::byte_serialize(from.as_bytes()).collect();
            let path = format!("/api/sessions/{id}/transcript?after={f}");
            let (s, v) = target::peer_call(state, machine, "GET", &path, None, false, CALL).await?;
            if !(200..300).contains(&s) {
                return Err(target::refused(state, machine, s, &v));
            }
            Ok(page_of(&v))
        }
    }
}

/// The lines of `t`'s last pages, enough for `last` messages.
async fn recent_lines(
    state: &AppState,
    t: &Target,
    last: usize,
    tools: bool,
) -> Result<Vec<format::Line>, ToolError> {
    let mut before: Option<String> = None;
    let mut lines: Vec<format::Line> = Vec::new();
    for _ in 0..PAGES {
        let p = page(
            state,
            &t.place,
            &t.id,
            before.as_deref(),
            uievents::PAGE_MAX,
        )
        .await?;
        let mut older = format::lines(&p.events, tools);
        older.append(&mut lines);
        lines = older;
        if lines.iter().filter(|l| l.message).count() >= last {
            break;
        }
        match p.before {
            Some(b) => before = Some(b),
            None => break,
        }
    }
    Ok(lines)
}

// ─── status ────────────────────────────────────────────────────────────────────

/// How a session stands while it is waited for.
#[derive(Clone, Debug, Default)]
struct Status {
    status: String,
    waiting_for: Option<String>,
    /// Its turn runs (an HTTP API conversation's own record, else its
    /// status).
    busy: bool,
}

async fn status_of(state: &AppState, place: &Place, id: &str) -> Option<Status> {
    match place {
        Place::Here => {
            let s = state.find_session(id).await?;
            let t = target::from_summary(state, &s).await;
            let api_turn = if t.reach == Reach::Api {
                state
                    .bridge
                    .thread(id)
                    .await
                    .map(|th| th.active_turn.is_some())
            } else {
                None
            };
            Some(Status {
                busy: api_turn.unwrap_or_else(|| t.busy()),
                status: t.status,
                waiting_for: t.waiting_for,
            })
        }
        Place::Peer { id: machine, .. } => {
            let (s, v) = target::peer_call(
                state,
                machine,
                "GET",
                &format!("/api/sessions/{id}"),
                None,
                false,
                CALL,
            )
            .await
            .ok()?;
            if !(200..300).contains(&s) {
                return None;
            }
            let t = target::from_value(&v, machine, "")?;
            Some(Status {
                busy: t.busy(),
                status: t.status,
                waiting_for: t.waiting_for,
            })
        }
    }
}

// ─── delivering ────────────────────────────────────────────────────────────────

/// How a message went.
#[derive(Clone, Debug)]
pub(crate) struct Delivered {
    /// `sent`, `queued`, `relayed` or `started` (an ended session continued
    /// with it).
    pub delivery: &'static str,
    /// The session that has it: a continued fork's new id.
    pub session: String,
    pub forked: bool,
}

fn delivery_word(d: Delivery) -> &'static str {
    match d {
        Delivery::Sent => "sent",
        Delivery::Queued => "queued",
        Delivery::Relayed => "relayed",
    }
}

fn ended(t: &Target) -> ToolError {
    ToolError::new(
        "ended",
        format!(
            "“{}” has ended. Pass start_if_ended: true to continue it with the message.",
            t.name
        ),
    )
}

/// Why a session (that runs) can't be reached, and what would reach it.
fn unreachable(t: &Target) -> ToolError {
    ToolError::new(
        "unreachable",
        format!(
            "“{}” runs in another app and Mother Claude can't reach it: install Mother Claude's \
             plugin for Claude Code (the session then takes messages at its next step), run it \
             in tmux, or wait until it ends — then a message continues it.",
            t.name
        ),
    )
}

/// Delivers `text` to `t` (`dangerous`: a dangerous approval was allowed).
async fn deliver(
    state: &AppState,
    t: &Target,
    text: &str,
    start_if_ended: bool,
    dangerous: bool,
) -> Result<Delivered, ToolError> {
    match &t.place {
        Place::Here => deliver_here(state, t, text, start_if_ended, dangerous).await,
        Place::Peer { id, .. } => {
            deliver_there(state, id, t, text, start_if_ended, dangerous).await
        }
    }
}

async fn deliver_here(
    state: &AppState,
    t: &Target,
    text: &str,
    start_if_ended: bool,
    dangerous: bool,
) -> Result<Delivered, ToolError> {
    let mut ctx = ClientContext::unknown();
    // A session never holds the dangerous right; a dangerous card was
    // allowed by someone who does (`pending::resolve` checks it), and the
    // delivery acts with that right — as `machine_run`'s `dangerous=1`.
    ctx.principal = if dangerous {
        Principal::Desktop
    } else {
        Principal::Mcp
    };
    match crate::server::routes::control::deliver_message(state, &ctx, &t.id, text).await {
        Ok(r) => Ok(Delivered {
            delivery: delivery_word(r.delivery),
            session: t.id.clone(),
            forked: false,
        }),
        Err(e) if e.code == "no_channel" && !t.live && start_if_ended => {
            let id = owned::continue_as(
                state,
                owned::Caller {
                    may_dangerous: dangerous,
                    desktop: false,
                },
                &t.id,
                &t.cwd,
                ContinueRequest {
                    prompt: Some(text.to_string()),
                    ..Default::default()
                },
            )
            .await
            .map_err(api_err)?;
            Ok(Delivered {
                delivery: "started",
                forked: id != t.id,
                session: id,
            })
        }
        Err(e) if e.code == "no_channel" && !t.live => Err(ended(t)),
        Err(e) if e.code == "no_channel" => Err(unreachable(t)),
        Err(e) => Err(api_err(e)),
    }
}

async fn deliver_there(
    state: &AppState,
    machine: &str,
    t: &Target,
    text: &str,
    start_if_ended: bool,
    dangerous: bool,
) -> Result<Delivered, ToolError> {
    let (s, v) = target::peer_call(
        state,
        machine,
        "POST",
        &format!("/api/sessions/{}/message", t.id),
        Some(&json!({ "text": text })),
        dangerous,
        CALL,
    )
    .await?;
    if (200..300).contains(&s) {
        let delivery = match v.get("delivery").and_then(Value::as_str) {
            Some("queued") => "queued",
            Some("relayed") => "relayed",
            _ => "sent",
        };
        return Ok(Delivered {
            delivery,
            session: t.id.clone(),
            forked: false,
        });
    }
    let code = v
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if code != "no_channel" {
        return Err(target::refused(state, machine, s, &v));
    }
    if t.live {
        return Err(unreachable(t));
    }
    if !start_if_ended {
        return Err(ended(t));
    }
    let (s, v) = target::peer_call(
        state,
        machine,
        "POST",
        &format!("/api/sessions/{}/continue", t.id),
        Some(&json!({ "prompt": text })),
        dangerous,
        START_CALL,
    )
    .await?;
    if !(200..300).contains(&s) {
        return Err(target::refused(state, machine, s, &v));
    }
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .filter(|i| crate::claude::ids::is_session_id(i))
        .unwrap_or(&t.id)
        .to_string();
    Ok(Delivered {
        delivery: "started",
        forked: id != t.id,
        session: id,
    })
}

/// Why messaging `t` is dangerous: it runs without permission prompts, so
/// whatever it is told runs with nobody approving it.
async fn unapproved(state: &AppState, t: &Target) -> Option<String> {
    let bypass = match &t.place {
        Place::Here => owned::runs_unapproved(state, &t.id).await,
        Place::Peer { .. } => false,
    } || owned::is_dangerous_mode(t.permission_mode.as_deref());
    bypass.then(|| {
        format!(
            "“{}” runs without permission prompts ({}): whatever it is told runs with nobody \
             approving it.",
            approve::clip(&t.name, 60),
            t.permission_mode.as_deref().unwrap_or("bypassPermissions")
        )
    })
}

fn note_for(d: &Delivered, t: &Target) -> String {
    match d.delivery {
        "sent" => "It has the message now.".to_string(),
        "queued" if t.reach == Reach::Owned => {
            "It is in the middle of a turn: it takes the message at its next step.".to_string()
        }
        "queued" => "It takes the message when it next runs a tool or its turn ends.".to_string(),
        "relayed" => "It has the message now.".to_string(),
        _ if d.forked => format!(
            "It had ended and another process holds it: Mother Claude continued it as a new \
             session, {}, with the message.",
            d.session
        ),
        _ => "It had ended: Mother Claude continued it with the message.".to_string(),
    }
}

fn delivered_json(t: &Target, d: &Delivered, a: &Allowed) -> Value {
    approve::strip_nulls(json!({
        "delivered": d.delivery,
        "session": d.session,
        "name": t.name,
        "computer": t.computer,
        "forked": d.forked.then_some(true),
        "approvedBy": a.by,
        "note": note_for(d, t),
    }))
}

/// The card for a read, a message or a question to `t`.
fn card<'a>(
    action: Action,
    me: &'a Me,
    t: &'a Target,
    message: Option<&'a str>,
    last: Option<usize>,
    dangerous: Option<String>,
) -> Card<'a> {
    Card {
        action,
        me,
        target: Some(t),
        computer: t.computer.clone(),
        machine: t.place.machine().map(str::to_string),
        folder: Some(t.cwd.clone()).filter(|c| !c.is_empty()),
        project_name: Some(t.project_name.clone()).filter(|p| !p.is_empty()),
        message,
        last,
        dangerous,
    }
}

/// A paired computer that must let this one see (`View`) or drive
/// (`Control`) its sessions, checked before anyone is asked.
fn may(state: &AppState, t: &Target, grant: Grant) -> Result<(), ToolError> {
    if let Some(m) = t.place.machine() {
        if let Some(why) = target::why_not(state, m, grant) {
            return Err(ToolError::new("not_allowed", why));
        }
    }
    Ok(())
}

// ─── the tools ─────────────────────────────────────────────────────────────────

/// `session_read`.
pub(crate) async fn read(
    state: &AppState,
    me: &Me,
    wanted: &str,
    computer: Option<&str>,
    last: usize,
    tools: bool,
) -> Result<String, ToolError> {
    let t = target::resolve(state, me, wanted, computer, false).await?;
    may(state, &t, Grant::View)?;
    approve::approve(state, card(Action::Read, me, &t, None, Some(last), None)).await?;
    let lines = recent_lines(state, &t, last, tools).await?;
    approve::audit(
        state,
        "session.read",
        me,
        Some(&t.id),
        t.place.machine(),
        None,
        json!({ "last": last, "tools": tools }),
    );
    Ok(format::render(&t, &lines, last))
}

/// `session_send`.
pub(crate) async fn send(
    state: &AppState,
    me: &Me,
    wanted: &str,
    computer: Option<&str>,
    message: &str,
    start_if_ended: bool,
) -> Result<Value, ToolError> {
    let t = target::resolve(state, me, wanted, computer, true).await?;
    may(state, &t, Grant::Control)?;
    let dangerous = unapproved(state, &t).await;
    let allowed = approve::approve(
        state,
        card(Action::Send, me, &t, Some(message), None, dangerous),
    )
    .await?;
    let text = format::message(me, Kind::Message, message);
    let d = deliver(state, &t, &text, start_if_ended, allowed.dangerous).await?;
    approve::audit(
        state,
        "session.send",
        me,
        Some(&t.id),
        t.place.machine(),
        Some(message),
        json!({
            "delivery": d.delivery,
            "session": d.session,
            "dangerous": allowed.dangerous,
            "rule": allowed.rule,
        }),
    );
    Ok(delivered_json(&t, &d, &allowed))
}

/// Where an answer will show, taken before the message goes.
struct Mark {
    /// Turns the owned session here ended so far.
    turns: u64,
    /// Its transcript's end.
    tail: Option<String>,
}

async fn mark(state: &AppState, t: &Target) -> Mark {
    Mark {
        turns: state.mcp.turns().get(&t.id).count,
        tail: page(state, &t.place, &t.id, None, 1)
            .await
            .ok()
            .map(|p| p.tail),
    }
}

/// What came of waiting.
#[derive(Debug, Default)]
struct Answer {
    finished: bool,
    text: Option<String>,
    status: Option<Status>,
    note: Option<String>,
}

impl Answer {
    fn json(self, waited: Duration) -> Value {
        let (status, waiting_for) = match &self.status {
            Some(s) => (Some(s.status.clone()), s.waiting_for.clone()),
            None => (None, None),
        };
        approve::strip_nulls(json!({
            "finished": self.finished,
            "answer": self.text,
            "status": status,
            "waitingFor": waiting_for,
            "waitedSeconds": waited.as_secs(),
            "note": self.note,
        }))
    }
}

/// An owned session here: until a turn ends after `base` with nothing
/// queued behind it, it stops, or `deadline`.
async fn wait_owned(state: &AppState, id: &str, base: u64, deadline: Instant) -> Answer {
    loop {
        let end = state.mcp.turns().get(id);
        if end.count > base && !end.more {
            let (text, note) = match (end.is_error, end.text) {
                (true, t) => (
                    None,
                    Some(format!(
                        "Its turn ended with an error: {}",
                        approve::clip(t.as_deref().unwrap_or("unknown"), 500)
                    )),
                ),
                (false, Some(t)) if !t.trim().is_empty() => {
                    (Some(format::keep_end(t.trim(), format::ANSWER_MAX)), None)
                }
                _ => (
                    None,
                    Some("It finished its turn without writing an answer.".to_string()),
                ),
            };
            return Answer {
                finished: true,
                text,
                status: status_of(state, &Place::Here, id).await,
                note,
            };
        }
        if !owned::has_handle(state, id) {
            return Answer {
                finished: false,
                status: status_of(state, &Place::Here, id).await,
                note: Some("It stopped before it answered.".into()),
                ..Default::default()
            };
        }
        if Instant::now() >= deadline {
            return timed_out(state, &Place::Here, id, None).await;
        }
        tokio::time::sleep(POLL_HERE).await;
    }
}

async fn timed_out(state: &AppState, place: &Place, id: &str, partial: Option<String>) -> Answer {
    let status = status_of(state, place, id).await;
    let doing = match &status {
        Some(s) if s.status == "waiting" => format!(
            "It is waiting for its user{}.",
            s.waiting_for
                .as_deref()
                .map(|w| format!(" ({w})"))
                .unwrap_or_default()
        ),
        Some(s) if s.busy => "It is still working.".to_string(),
        _ => "It has not answered yet.".to_string(),
    };
    Answer {
        finished: false,
        text: partial,
        status,
        note: Some(format!(
            "{doing} session_read shows its answer later; session_ask can wait longer."
        )),
    }
}

/// Any other session: its transcript from `from` (`None`: after its last
/// prompt) until its turn has run and ended, or `deadline`.
async fn wait_transcript(
    state: &AppState,
    place: &Place,
    id: &str,
    from: Option<String>,
    deadline: Instant,
) -> Answer {
    let poll = if place.is_here() {
        POLL_HERE * 2
    } else {
        POLL_THERE
    };
    let mut cursor = from;
    let mut said: Vec<UiEvent> = Vec::new();
    let mut seen_busy = false;
    loop {
        match &cursor {
            Some(c) => {
                if let Ok(p) = after(state, place, id, c).await {
                    if p.reset {
                        // Too much to read at once: what follows its last
                        // prompt will do.
                        if let Ok(last) = page(state, place, id, None, uievents::PAGE_MAX).await {
                            said = format::after_last_prompt(last.events);
                            cursor = Some(last.tail);
                        }
                    } else {
                        said.extend(p.events);
                        cursor = Some(p.tail);
                    }
                }
            }
            None => {
                if let Ok(p) = page(state, place, id, None, 100).await {
                    said = format::after_last_prompt(p.events);
                }
            }
        }
        let status = status_of(state, place, id).await;
        let busy = status.as_ref().is_some_and(|s| s.busy);
        let waiting = status.as_ref().is_some_and(|s| s.status == "waiting");
        seen_busy |= busy;
        let answer = format::answer_of(&said);
        if (seen_busy || answer.is_some()) && !busy && !waiting {
            let note = answer
                .is_none()
                .then(|| "It finished its turn without writing an answer.".to_string());
            return Answer {
                finished: true,
                text: answer,
                status,
                note,
            };
        }
        if Instant::now() >= deadline {
            return timed_out(state, place, id, answer).await;
        }
        tokio::time::sleep(poll).await;
    }
}

/// Waits for the turn `d` started in `t` (or the conversation it continued
/// into) to end.
async fn wait(
    state: &AppState,
    t: &Target,
    d: &Delivered,
    mark: Mark,
    deadline: Instant,
) -> Answer {
    let same = d.session == t.id;
    if t.place.is_here() && owned::has_handle(state, &d.session) {
        let base = if same { mark.turns } else { 0 };
        return wait_owned(state, &d.session, base, deadline).await;
    }
    // A relay waiting for a session that runs nothing: it reaches it only
    // when it next runs a tool or ends a turn.
    if d.delivery == "queued" && t.reach != Reach::Owned && !t.busy() {
        return Answer {
            finished: false,
            note: Some(
                "The message waits for it: it takes it when it next runs a tool or ends a turn. \
                 session_read shows its answer later."
                    .into(),
            ),
            ..Default::default()
        };
    }
    let from = if same && d.delivery != "started" {
        mark.tail
    } else {
        None
    };
    wait_transcript(state, &t.place, &d.session, from, deadline).await
}

/// `session_ask`.
pub(crate) async fn ask(
    state: &AppState,
    me: &Me,
    wanted: &str,
    computer: Option<&str>,
    question: &str,
    timeout: Duration,
    start_if_ended: bool,
) -> Result<Value, ToolError> {
    let t = target::resolve(state, me, wanted, computer, true).await?;
    may(state, &t, Grant::Control)?;
    let dangerous = unapproved(state, &t).await;
    let allowed = approve::approve(
        state,
        card(Action::Ask, me, &t, Some(question), None, dangerous),
    )
    .await?;
    let m = mark(state, &t).await;
    let text = format::message(me, Kind::Question, question);
    let d = deliver(state, &t, &text, start_if_ended, allowed.dangerous).await?;
    approve::audit(
        state,
        "session.ask",
        me,
        Some(&t.id),
        t.place.machine(),
        Some(question),
        json!({
            "delivery": d.delivery,
            "session": d.session,
            "dangerous": allowed.dangerous,
            "rule": allowed.rule,
        }),
    );
    let began = Instant::now();
    let answer = wait(state, &t, &d, m, began + timeout).await;
    let mut out = delivered_json(&t, &d, &allowed);
    if let (Some(o), Value::Object(a)) = (out.as_object_mut(), answer.json(began.elapsed())) {
        o.extend(a);
    }
    Ok(out)
}

/// What `session_start` asks.
pub(crate) struct StartArgs {
    pub folder: Option<String>,
    pub project: Option<String>,
    pub computer: Option<String>,
    pub prompt: String,
    pub model: Option<String>,
    pub wait: bool,
    pub timeout: Duration,
}

/// The folder a start runs in: `folder` as given, else the project by
/// name (the computer's project list), else (here) the calling session's
/// project.
async fn folder_for(
    state: &AppState,
    me: &Me,
    place: &Place,
    folder: Option<&str>,
    project: Option<&str>,
) -> Result<String, ToolError> {
    if let Some(f) = folder {
        return Ok(f.to_string());
    }
    let Some(wanted) = project else {
        return match (place, &me.project) {
            (Place::Here, Some(p)) => Ok(p.clone()),
            _ => Err(ToolError::invalid(
                "Name the folder (an absolute path on that computer) or the project.",
            )),
        };
    };
    let known: Vec<(String, String)> = match place {
        Place::Here => crate::server::routes::data::project_list(state)
            .await
            .into_iter()
            .map(|p| (p.name, p.path))
            .collect(),
        Place::Peer { id, .. } => {
            let (s, v) =
                target::peer_call(state, id, "GET", "/api/projects", None, false, CALL).await?;
            if !(200..300).contains(&s) {
                return Err(target::refused(state, id, s, &v));
            }
            v.get("projects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|p| {
                    let path = p.get("path")?.as_str()?.to_string();
                    let name = p
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| crate::machines::setup::folders::name_of(&path));
                    Some((name, path))
                })
                .collect()
        }
    };
    let w = wanted.trim().to_lowercase();
    if let Some((_, path)) = known.iter().find(|(name, path)| {
        name.to_lowercase() == w
            || crate::machines::setup::folders::name_of(path).to_lowercase() == w
    }) {
        return Ok(path.clone());
    }
    let names: Vec<&str> = known.iter().take(20).map(|(n, _)| n.as_str()).collect();
    Err(ToolError::new(
        "unknown_project",
        if names.is_empty() {
            format!(
                "No project called {wanted:?} on {}. Name its folder instead.",
                place.name(state)
            )
        } else {
            format!(
                "No project called {wanted:?} on {}. Its projects: {}. Or name a folder.",
                place.name(state),
                names.join(", ")
            )
        },
    ))
}

/// `session_start`.
pub(crate) async fn start(state: &AppState, me: &Me, a: StartArgs) -> Result<Value, ToolError> {
    let place = match a.computer.as_deref() {
        Some(c) => target::place_of(state, c)?,
        None => Place::Here,
    };
    if let Some(m) = place.machine() {
        if let Some(why) = target::why_not(state, m, Grant::Control) {
            return Err(ToolError::new("not_allowed", why));
        }
    }
    let folder = folder_for(state, me, &place, a.folder.as_deref(), a.project.as_deref()).await?;
    let computer = place.name(state);
    let project_name = crate::machines::setup::folders::name_of(&folder);
    let allowed = approve::approve(
        state,
        Card {
            action: Action::Start,
            me,
            target: None,
            computer: computer.clone(),
            machine: place.machine().map(str::to_string),
            folder: Some(folder.clone()),
            project_name: Some(project_name.clone()),
            message: Some(&a.prompt),
            last: None,
            dangerous: None,
        },
    )
    .await?;
    let text = format::message(me, Kind::Start, &a.prompt);
    let id = match &place {
        Place::Here => owned::spawn_as(
            state,
            owned::Caller {
                may_dangerous: false,
                desktop: false,
            },
            SpawnRequest {
                cwd: folder.clone(),
                prompt: Some(text),
                model: a.model.clone(),
                ..Default::default()
            },
        )
        .await
        .map_err(api_err)?,
        Place::Peer { id: machine, .. } => {
            let mut body = json!({ "cwd": folder, "prompt": text });
            if let Some(m) = &a.model {
                body["model"] = json!(m);
            }
            let (s, v) = target::peer_call(
                state,
                machine,
                "POST",
                "/api/sessions",
                Some(&body),
                false,
                START_CALL,
            )
            .await?;
            if !(200..300).contains(&s) {
                return Err(target::refused(state, machine, s, &v));
            }
            v.get("id")
                .and_then(Value::as_str)
                .filter(|i| crate::claude::ids::is_session_id(i))
                .map(str::to_string)
                .ok_or_else(|| {
                    ToolError::new("machine_refused", format!("{computer} answered oddly."))
                })?
        }
    };
    approve::audit(
        state,
        "session.start",
        me,
        Some(&id),
        place.machine(),
        Some(&a.prompt),
        json!({ "folder": folder, "rule": allowed.rule }),
    );
    let mut out = approve::strip_nulls(json!({
        "started": true,
        "session": id,
        "computer": computer,
        "folder": folder,
        "approvedBy": allowed.by,
    }));
    if a.wait {
        let began = Instant::now();
        let deadline = began + a.timeout;
        let answer = if place.is_here() {
            wait_owned(state, &id, 0, deadline).await
        } else {
            wait_transcript(state, &place, &id, None, deadline).await
        };
        if let (Some(o), Value::Object(more)) = (out.as_object_mut(), answer.json(began.elapsed()))
        {
            o.extend(more);
        }
    } else {
        out["note"] = json!(
            "It is working on it. session_read shows what it says; session_ask asks it more."
        );
    }
    Ok(out)
}
