//! Who agrees to one session talking to another: a permission the user saved
//! ("talking to other sessions", `pending::rules`), else a card on the
//! calling session (`PendingSource::Session`) — and the audit line each
//! read, send, ask and start leaves.

use serde_json::{json, Value};

use super::target::{Me, Target};
use super::APPROVAL_TIMEOUT;
use crate::claude::model::{GrantOffer, PendingItem, PendingKind, PendingSource, RuleKind};
use crate::mcp::ToolError;
use crate::pending::{self, rules, WaitEnd};
use crate::server::devices::audit::AuditEntry;
use crate::state::AppState;

/// What the calling session asks to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Read,
    Send,
    Ask,
    Start,
}

impl Action {
    /// The tool, as the card names it (`tool_name`).
    pub(crate) fn tool(self) -> &'static str {
        match self {
            Action::Read => "session_read",
            Action::Send => "session_send",
            Action::Ask => "session_ask",
            Action::Start => "session_start",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Action::Read => "read",
            Action::Send => "send",
            Action::Ask => "ask",
            Action::Start => "start",
        }
    }
}

/// What a card shows.
pub(crate) struct Card<'a> {
    pub action: Action,
    pub me: &'a Me,
    /// The session it reads, messages or asks (`None`: a start).
    pub target: Option<&'a Target>,
    /// Where: this computer's name or the paired computer's.
    pub computer: String,
    /// The paired computer's id (`None`: here).
    pub machine: Option<String>,
    /// The target's folder, or the one a start runs in.
    pub folder: Option<String>,
    /// The target's project, or the started one's.
    pub project_name: Option<String>,
    /// The message, question or first prompt (as the caller wrote it).
    pub message: Option<&'a str>,
    /// `session_read`: how many messages.
    pub last: Option<usize>,
    /// Why it is dangerous (the target runs without permission prompts).
    pub dangerous: Option<String>,
}

/// Who agreed.
#[derive(Clone, Debug, Default)]
pub(crate) struct Allowed {
    /// A person's name, or the permission that covered it.
    pub by: Option<String>,
    /// The covering permission's id.
    pub rule: Option<String>,
    /// A dangerous card was allowed (someone with the dangerous right).
    pub dangerous: bool,
}

pub(crate) fn clip(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(n.saturating_sub(1)).collect::<String>()
        )
    }
}

/// The card's title (the server's words; the dashboard words its own).
pub(crate) fn title(c: &Card) -> String {
    let on = if c.machine.is_some() {
        format!(" on {}", c.computer)
    } else {
        String::new()
    };
    let name = c.target.map(|t| clip(&t.name, 60)).unwrap_or_default();
    match c.action {
        Action::Read => format!("Read the session “{name}”{on}"),
        Action::Send => format!("Message the session “{name}”{on}"),
        Action::Ask => format!("Ask the session “{name}”{on}"),
        Action::Start => format!(
            "Start a session in {}{on}",
            c.project_name
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(|p| clip(p, 60))
                .unwrap_or_else(|| "a project".into())
        ),
    }
}

/// One line under the title.
fn detail(c: &Card) -> String {
    let from = clip(&c.me.name, 60);
    let said = c.message.map(|m| clip(m, 160)).unwrap_or_default();
    match c.action {
        Action::Read => format!(
            "“{from}” wants to read its last {} messages.",
            c.last.unwrap_or(super::READ_DEFAULT)
        ),
        Action::Send => format!("“{from}” sends: {said}"),
        Action::Ask => format!("“{from}” asks, and waits for the answer: {said}"),
        Action::Start => format!(
            "“{from}” starts it in {} with: {said}",
            c.folder.as_deref().unwrap_or_default()
        ),
    }
}

/// The card itself.
pub(crate) fn item(c: &Card) -> PendingItem {
    let t = c.target;
    let input = json!({
        "action": c.action.key(),
        "session": t.map(|t| t.id.clone()),
        "sessionName": t.map(|t| t.name.clone()),
        "sessionProject": c.project_name,
        "folder": c.folder,
        "computer": c.computer,
        "machine": c.machine,
        "here": c.machine.is_none(),
        "live": t.map(|t| t.live),
        "status": t.map(|t| t.status.clone()),
        "permissionMode": t.and_then(|t| t.permission_mode.clone()),
        "message": c.message,
        "last": c.last,
        "from": c.me.name,
        "fromSession": c.me.session,
        "project": c.me.project,
        "projectName": c.me.project_name,
    });
    let input = strip_nulls(input);
    PendingItem {
        session_id: c.me.session.clone(),
        kind: PendingKind::Permission,
        source: PendingSource::Session,
        tool_name: Some(c.action.tool().into()),
        title: title(c),
        detail: Some(detail(c)),
        input: Some(input),
        dangerous: c.dangerous.is_some(),
        dangerous_reason: c.dangerous.clone(),
        grant: Some(GrantOffer {
            kind: RuleKind::Sessions,
            tool: None,
            patterns: None,
            suggested: Vec::new(),
            session: c.me.session.clone(),
            session_name: Some(c.me.name.clone()),
            project: c.me.project.clone(),
            project_name: c.me.project_name.clone(),
            machine: None,
            machine_name: None,
            blocked: c.dangerous.as_ref().map(|_| {
                "Talking to a session that runs without permission prompts always asks: this \
                 can't be remembered."
                    .to_string()
            }),
        }),
        ..Default::default()
    }
}

/// An object without its `null` members.
pub(crate) fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(_, v)| !v.is_null()).collect()),
        other => other,
    }
}

/// A permission covers it, else a person allows it on the card (or it is
/// refused, unanswered or withdrawn).
pub(crate) async fn approve(state: &AppState, c: Card<'_>) -> Result<Allowed, ToolError> {
    if c.dangerous.is_none() {
        let covered = state.rules.covering(&rules::Ask {
            kind: RuleKind::Sessions,
            tool: None,
            session: &c.me.session,
            folder: c.me.project.as_deref(),
            project: c.me.project.as_deref(),
            machine: None,
            subject: None,
        });
        if let Some(rule) = covered {
            return Ok(Allowed {
                by: Some(format!("your permission ({})", rules::describe(&rule))),
                rule: Some(rule.id),
                dangerous: false,
            });
        }
    }
    let what = match c.action {
        Action::Read => "reading that session",
        Action::Send => "this message",
        Action::Ask => "this question",
        Action::Start => "starting that session",
    };
    let dangerous = c.dangerous.is_some();
    let (guard, rx) = pending::raise(state, item(&c)).await;
    let id = guard.id().to_string();
    match pending::wait_end(rx, APPROVAL_TIMEOUT).await {
        WaitEnd::Decided(d) if d.is_allow() => Ok(Allowed {
            by: state
                .pending
                .resolution(&id)
                .and_then(|r| r.by)
                .and_then(|b| b.name),
            rule: None,
            dangerous,
        }),
        WaitEnd::Decided(d) => {
            let note = d
                .message
                .filter(|m| !m.trim().is_empty())
                .map(|m| format!(" Their note: {}", clip(&m, 500)))
                .unwrap_or_default();
            Err(ToolError::new(
                "denied",
                format!(
                    "The user denied {what}. Don't retry it; ask the user what to do instead.{note}"
                ),
            ))
        }
        WaitEnd::TimedOut => {
            guard.expire();
            Err(ToolError::new(
                "approval_timeout",
                format!(
                    "Nobody approved {what} within {} minutes. Ask the user to approve it in \
                     Mother Claude, then try again.",
                    APPROVAL_TIMEOUT.as_secs() / 60
                ),
            ))
        }
        WaitEnd::Closed => Err(ToolError::new(
            "cancelled",
            "The approval was withdrawn. Try again if it is still needed.",
        )),
    }
}

/// The audit line: who, to whom, where, the text's SHA-256 (never the text).
pub(crate) fn audit(
    state: &AppState,
    action: &str,
    me: &Me,
    to: Option<&str>,
    machine: Option<&str>,
    text: Option<&str>,
    extra: Value,
) {
    let mut detail = json!({
        "from": me.session,
        "to": to,
        "textSha256": text.map(|t| crate::claude::secrets::sha256_hex(t.as_bytes())),
        "textBytes": text.map(str::len),
    });
    if let (Some(d), Value::Object(more)) = (detail.as_object_mut(), extra) {
        d.extend(more);
    }
    state.devices.audit.record(AuditEntry {
        action: action.to_string(),
        principal: Some("mcp".into()),
        device_id: machine.map(str::to_string),
        device_name: machine.map(|m| crate::machines::peer_name(state, m)),
        detail: Some(strip_nulls(detail)),
        ..AuditEntry::now()
    });
}
