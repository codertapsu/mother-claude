//! What one session sends another (its first line says who from and how to
//! answer), and how another session's transcript reads back.

use super::approve::clip;
use super::target::{Me, Target};
use crate::claude::model::{UiEvent, UiKind, UiRole};

/// One message at most this long in a read.
pub(crate) const ENTRY_MAX: usize = 4_000;
/// A read at most this long (the oldest lines go first).
pub(crate) const READ_MAX_CHARS: usize = 60_000;
/// An answer at most this long (its end is kept).
pub(crate) const ANSWER_MAX: usize = 30_000;

/// What the first line says the text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Message,
    Question,
    Start,
}

/// A name for a line: no control characters, at most `max` characters.
fn clean(s: &str, max: usize) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).collect();
    clip(&t, max)
}

/// The first line of whatever one session sends another: who it is from
/// (the session, its project, its computer) and how to answer.
pub(crate) fn attribution(me: &Me, kind: Kind) -> String {
    let name = clean(&me.name, 80);
    let computer = clean(&me.computer, 64);
    let whence = match me
        .project_name
        .as_deref()
        .map(|p| clean(p, 80))
        .filter(|p| !p.is_empty())
    {
        Some(p) => format!("{p}, on {computer}"),
        None => format!("on {computer}"),
    };
    let reply = format!("session_send to {} on {computer}", me.session);
    match kind {
        Kind::Message => format!(
            "[Message from the session “{name}” ({whence}) through Mother Claude — reply with \
             {reply}]"
        ),
        Kind::Question => format!(
            "[Question from the session “{name}” ({whence}) through Mother Claude — it waits for \
             your answer: reply in this turn (to write to it later, use {reply})]"
        ),
        Kind::Start => format!(
            "[Started by the session “{name}” ({whence}) through Mother Claude — to write back to \
             it, use {reply}]"
        ),
    }
}

/// The text a session receives: the first line, a blank line, the message.
pub(crate) fn message(me: &Me, kind: Kind, text: &str) -> String {
    format!("{}\n\n{}", attribution(me, kind), text.trim())
}

/// One entry of a read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    /// A message (what `last` counts), not a tool line.
    pub message: bool,
    pub text: String,
}

/// `2026-11-17 09:30 UTC`.
pub(crate) fn stamp(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
}

/// RFC 3339, to the second.
pub(crate) fn iso(ms: Option<i64>) -> Option<String> {
    ms.and_then(chrono::DateTime::from_timestamp_millis)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn line(e: &UiEvent, tools: bool) -> Option<Line> {
    let when =
        e.ts.and_then(stamp)
            .map(|t| format!(" · {t}"))
            .unwrap_or_default();
    let text = e
        .text
        .as_deref()
        .map(|t| clip(t, ENTRY_MAX))
        .filter(|t| !t.is_empty());
    let (message, s) = match (e.kind, e.role) {
        (UiKind::Prompt, _) | (UiKind::Text, UiRole::User) => {
            (true, format!("[user{when}]\n{}", text?))
        }
        (UiKind::Text, _) => (true, format!("[assistant{when}]\n{}", text?)),
        (UiKind::Relayed, _) => (true, format!("[relayed to it{when}]\n{}", text?)),
        (UiKind::Question, _) => (
            true,
            format!(
                "[it asked its user{when}]\n{}",
                text.unwrap_or_else(|| "(a question)".into())
            ),
        ),
        (UiKind::Plan, _) => (
            true,
            format!(
                "[its plan{when}]\n{}",
                text.unwrap_or_else(|| "(a plan)".into())
            ),
        ),
        (UiKind::Error, _) => (true, format!("[error{when}] {}", text?)),
        (UiKind::Compact, _) => (false, format!("[the conversation was compacted{when}]")),
        (UiKind::Tool, _) if tools => {
            let t = e.tool.as_ref()?;
            (
                false,
                format!("[tool {}{when}] {}", t.name, clip(&t.summary, 300)),
            )
        }
        (UiKind::Result, _) if tools => {
            let r = e.result.as_ref()?;
            (
                false,
                format!(
                    "[result{}] {}",
                    if r.is_error { " (error)" } else { "" },
                    clip(r.text.as_deref().unwrap_or_default(), 1_000)
                ),
            )
        }
        _ => return None,
    };
    Some(Line { message, text: s })
}

/// The entries of a page of events (the main conversation's, not its
/// subagents').
pub(crate) fn lines(events: &[UiEvent], tools: bool) -> Vec<Line> {
    events
        .iter()
        .filter(|e| e.agent_id.is_none())
        .filter_map(|e| line(e, tools))
        .collect()
}

/// The last `n` messages (and the tool lines between them), at most
/// [`READ_MAX_CHARS`] all told.
pub(crate) fn last_messages(lines: &[Line], n: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut count = 0;
    let mut size = 0;
    for l in lines.iter().rev() {
        if l.message {
            if count == n {
                break;
            }
            count += 1;
        } else if count == n {
            break;
        }
        size += l.text.len() + 2;
        if size > READ_MAX_CHARS && !out.is_empty() {
            out.push("[… older messages left out]".into());
            break;
        }
        out.push(l.text.clone());
    }
    out.reverse();
    out
}

/// Who the target is: name, project, computer, status, when, where.
pub(crate) fn header(t: &Target) -> String {
    let project = if t.project_name.is_empty() {
        String::new()
    } else {
        format!("{}, ", t.project_name)
    };
    let waiting = t
        .waiting_for
        .as_deref()
        .filter(|w| !w.is_empty())
        .map(|w| format!(" ({w})"))
        .unwrap_or_default();
    let last = t
        .last_activity_at
        .and_then(stamp)
        .map(|s| format!(", last active {s}"))
        .unwrap_or_default();
    let state = if t.live {
        format!("running, {}{waiting}", t.status)
    } else {
        "not running".to_string()
    };
    format!(
        "Session “{}” ({project}on {}) — {state}{last}.\nFolder: {}\nSession id: {}",
        t.name, t.computer, t.cwd, t.id
    )
}

/// A read: the header, then the last `n` messages, oldest first.
pub(crate) fn render(t: &Target, lines: &[Line], n: usize) -> String {
    let picked = last_messages(lines, n);
    if picked.is_empty() {
        return format!("{}\n\nNo messages yet.", header(t));
    }
    format!("{}\n\n{}", header(t), picked.join("\n\n"))
}

/// What a session said in `events`: its texts, joined (the end kept when
/// long).
pub(crate) fn answer_of(events: &[UiEvent]) -> Option<String> {
    let texts: Vec<&str> = events
        .iter()
        .filter(|e| e.agent_id.is_none() && e.kind == UiKind::Text && e.role != UiRole::User)
        .filter_map(|e| e.text.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    if texts.is_empty() {
        return None;
    }
    Some(keep_end(&texts.join("\n\n"), ANSWER_MAX))
}

/// `s`, or its last `n` characters after `…`.
pub(crate) fn keep_end(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_string();
    }
    let tail: String = s.chars().skip(count - (n - 1)).collect();
    format!("…{tail}")
}

/// The events after the last prompt (a conversation that starts with the
/// message: a continued fork, a new session).
pub(crate) fn after_last_prompt(events: Vec<UiEvent>) -> Vec<UiEvent> {
    let at = events
        .iter()
        .rposition(|e| e.agent_id.is_none() && e.kind == UiKind::Prompt);
    match at {
        Some(i) => events.into_iter().skip(i + 1).collect(),
        None => Vec::new(),
    }
}
