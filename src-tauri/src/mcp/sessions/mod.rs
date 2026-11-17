//! Sessions working together (docs/MCP.md "Sessions working together"): the
//! `session_*` tools let a Claude Code session find the user's other
//! sessions — on this computer and on the paired computers that show theirs
//! here — read what one said, message it, ask it something and wait for its
//! answer, or start a new one in a project.
//!
//! * `session_list {computer?, query?, live_only?, limit?, projects?}` —
//!   who else there is: id, name, project, folder, computer, live, status,
//!   last activity, how a message reaches it, its permission mode (and, with
//!   `projects`, the folders a session can start in). No approval.
//! * `session_read {session, computer?, last?, include_tools?}` — its last
//!   messages, read only.
//! * `session_send {session, computer?, message, start_if_ended?}` — a
//!   message, attributed ([`format::message`]); an ended session is
//!   continued as an owned one unless `start_if_ended` is false.
//! * `session_ask {session, computer?, question, timeout_s?, start_if_ended?}`
//!   — the same, then waits for that session's turn to end and answers what
//!   it said (on a timeout, what is known so far).
//! * `session_start {folder? | project?, computer?, prompt, model?, wait?,
//!   timeout_s?}` — a new owned session here or on a paired computer.
//!
//! **Approvals.** Reading, messaging, asking and starting each raise a card
//! (`PendingSource::Session`, on the calling session) unless a permission
//! covers it (`RuleKind::Sessions`, "talking to other sessions": from this
//! session, its project or any session here; `pending::rules`). A target
//! that runs without permission prompts (`bypassPermissions`, `dontAsk`)
//! makes the card dangerous: only someone with the dangerous right may allow
//! it, and it is never remembered.
//!
//! **Delivery.** Here, through what the dashboard's composer uses
//! (`routes::control::deliver_message`: an owned session, an HTTP API
//! conversation, tmux, the hooks relay); an ended one through
//! `owned::continue_as` (a fork when another process holds it). On another
//! computer, through its `/api/sessions/*` routes over the Machines link,
//! with the `view` or `control` grant it gave this one and `X-MC-Acting:
//! machine` — dangerous only after a dangerous approval here, intersected
//! there with its own `dangerous` grant.
//!
//! **Never itself.** A session never reads, messages or asks itself (nor a
//! copy of its own conversation on another computer).
//!
//! **Records.** Each read, send, ask and start is audited (`session.read`,
//! `session.send`, `session.ask`, `session.start`) with the ids, the
//! computer and the text's SHA-256 — never the text.

mod approve;
mod format;
mod talk;
mod target;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use super::{Registry, ToolAnnotations, ToolCtx, ToolDef, ToolError, ToolOutput};
use crate::state::AppState;

/// How long an approval waits for someone to answer.
pub(crate) const APPROVAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// The longest message, question or prompt (bytes).
pub(crate) const MAX_MESSAGE: usize = 100_000;
const READ_DEFAULT: usize = 10;
const READ_MAX: usize = 50;
const ASK_DEFAULT_SECS: u64 = 300;
const ASK_MAX_SECS: u64 = 1800;
const LIST_DEFAULT: usize = 30;
const LIST_MAX: usize = 100;
/// A call that waits for an approval, then delivers.
const TALK_LIMIT: Duration = Duration::from_secs(12 * 60);
/// A call that waits for an approval, then for an answer.
const ASK_LIMIT: Duration = Duration::from_secs(42 * 60);
/// Turn ends kept (the newest).
const TURNS_KEPT: usize = 256;

// ─── the turns owned sessions end ─────────────────────────────────────────────

/// How an owned session's last turn ended (its stream's `result`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TurnEnd {
    /// Turns ended since Mother Claude started.
    pub count: u64,
    /// The `result` text: the turn's last message.
    pub text: Option<String>,
    pub is_error: bool,
    /// Messages still queued started the next turn at once.
    pub more: bool,
    /// When (ms).
    pub at: i64,
}

/// The turns owned sessions here ended, for `session_ask` and a waiting
/// `session_start` (`McpState::turns`): the newest [`TURNS_KEPT`].
#[derive(Debug, Default)]
pub struct Turns {
    ended: Mutex<HashMap<String, TurnEnd>>,
}

impl Turns {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, TurnEnd>> {
        self.ended.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn get(&self, id: &str) -> TurnEnd {
        self.lock().get(id).cloned().unwrap_or_default()
    }

    fn note(&self, id: &str, text: Option<&str>, is_error: bool, more: bool) {
        let mut m = self.lock();
        let at = crate::claude::registry::now_ms();
        let e = m.entry(id.to_string()).or_default();
        e.count += 1;
        e.text = text.map(str::to_string);
        e.is_error = is_error;
        e.more = more;
        e.at = at;
        if m.len() > TURNS_KEPT {
            if let Some(oldest) = m.iter().min_by_key(|(_, t)| t.at).map(|(k, _)| k.clone()) {
                m.remove(&oldest);
            }
        }
    }
}

/// An owned session's turn ended (`claude::control`, its stream's
/// `result`): `more` when queued messages start the next one at once.
pub fn turn_ended(state: &AppState, id: &str, text: Option<&str>, is_error: bool, more: bool) {
    state.mcp.turns().note(id, text, is_error, more);
}

// ─── the tools ─────────────────────────────────────────────────────────────────

fn def(name: &str, title: &str, description: &str, schema: Value, a: ToolAnnotations) -> ToolDef {
    ToolDef {
        name: name.into(),
        title: Some(title.into()),
        description: description.into(),
        input_schema: schema,
        annotations: Some(a),
    }
}

/// A string argument, trimmed, `None` when absent or empty.
fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A string argument kept as written (a message), `None` when absent or
/// blank.
fn body(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

fn flag(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

/// A whole number argument (also given as a string, as models sometimes do).
fn number(v: &Value, key: &str) -> Result<Option<u64>, ToolError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
            .map(Some)
            .ok_or_else(|| ToolError::invalid(format!("{key} must be a whole number."))),
        Some(Value::String(s)) => s
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| ToolError::invalid(format!("{key} must be a whole number."))),
        Some(_) => Err(ToolError::invalid(format!("{key} must be a whole number."))),
    }
}

/// `session`: required.
fn session_arg(args: &Value) -> Result<String, ToolError> {
    text(args, "session").ok_or_else(|| {
        ToolError::invalid(
            "Name the session: {\"session\": \"<its id>\"} (session_list names them; the first \
             8 characters of an id or a session's exact name do too).",
        )
    })
}

/// A message, question or prompt: required, at most [`MAX_MESSAGE`] bytes.
fn message_arg(args: &Value, key: &str) -> Result<String, ToolError> {
    let m = body(args, key)
        .ok_or_else(|| ToolError::invalid(format!("Write the {key}: {{\"{key}\": \"…\"}}.")))?;
    if m.len() > MAX_MESSAGE || m.contains('\0') {
        return Err(ToolError::invalid(format!(
            "The {key} must be text of at most {MAX_MESSAGE} bytes."
        )));
    }
    Ok(m)
}

/// `timeout_s`: 1 to [`ASK_MAX_SECS`], else the default.
fn timeout_arg(args: &Value) -> Result<Duration, ToolError> {
    let secs = number(args, "timeout_s")?.unwrap_or(ASK_DEFAULT_SECS);
    if secs == 0 || secs > ASK_MAX_SECS {
        return Err(ToolError::invalid(format!(
            "timeout_s is 1 to {ASK_MAX_SECS} seconds (30 minutes)."
        )));
    }
    Ok(Duration::from_secs(secs))
}

const COMPUTER: &str = "Which computer: this one (\"here\") or a paired computer's name. Without \
                        it, the session is looked for here and on every paired computer that \
                        shows its sessions here.";

pub(crate) fn register(r: &mut Registry) {
    r.add(
        def(
            "session_list",
            "List other Claude Code sessions",
            "List the user's other Claude Code sessions that Mother Claude knows: on this \
             computer and on the paired computers that show theirs here. Each has an id, a \
             name, its project and folder, the computer, whether it runs now and its status, \
             when it was last active, how a message reaches it, and its permission mode. \
             Without a query: the ones running, waiting or active in the last 24 hours; with \
             one: every session whose name, project, folder, last prompt or id matches. \
             projects: true also lists the project folders a new session can start in \
             (session_start). Read-only, no approval.",
            json!({
                "type": "object",
                "properties": {
                    "computer": { "type": "string", "description": COMPUTER },
                    "query": { "type": "string", "description": "Words to look for in a session's name, project, folder, last prompt or id." },
                    "live_only": { "type": "boolean", "description": "Only sessions running now." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": LIST_MAX, "description": "At most this many (default 30)." },
                    "projects": { "type": "boolean", "description": "Also list the project folders on each computer." }
                },
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(true),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
                ..Default::default()
            },
        ),
        |ctx: ToolCtx, args: Value| async move {
            let limit = number(&args, "limit")?
                .map(|n| (n as usize).clamp(1, LIST_MAX))
                .unwrap_or(LIST_DEFAULT);
            let me = target::me_if_known(&ctx.state, &ctx.caller).await;
            let v = target::list(
                &ctx.state,
                me.as_ref(),
                target::ListArgs {
                    computer: text(&args, "computer"),
                    query: text(&args, "query"),
                    live_only: flag(&args, "live_only").unwrap_or(false),
                    limit,
                    projects: flag(&args, "projects").unwrap_or(false),
                },
            )
            .await?;
            Ok(ToolOutput::json(v))
        },
    );

    r.add_with_limit(
        def(
            "session_read",
            "Read another session",
            "Read the last messages of another Claude Code session (here or on a paired \
             computer): what its user asked and what it answered, newest last; with \
             include_tools, the tools it ran too. Asks the user's approval in Mother Claude \
             unless a permission covers it.",
            json!({
                "type": "object",
                "properties": {
                    "session": { "type": "string", "description": "Its id (session_list), the first 8 characters of it, or its exact name." },
                    "computer": { "type": "string", "description": COMPUTER },
                    "last": { "type": "integer", "minimum": 1, "maximum": READ_MAX, "description": "How many messages (default 10, at most 50)." },
                    "include_tools": { "type": "boolean", "description": "Also show the tools it ran and their results (shortened)." }
                },
                "required": ["session"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(true),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
                ..Default::default()
            },
        ),
        Some(TALK_LIMIT),
        |ctx: ToolCtx, args: Value| async move {
            let session = session_arg(&args)?;
            let last = number(&args, "last")?
                .map(|n| (n as usize).clamp(1, READ_MAX))
                .unwrap_or(READ_DEFAULT);
            let me = target::me(&ctx.state, &ctx.caller).await?;
            talk::read(
                &ctx.state,
                &me,
                &session,
                text(&args, "computer").as_deref(),
                last,
                flag(&args, "include_tools").unwrap_or(false),
            )
            .await
            .map(ToolOutput::text)
        },
    );

    r.add_with_limit(
        def(
            "session_send",
            "Message another session",
            "Send a message to another Claude Code session (here or on a paired computer) and \
             return at once. The message starts with a line saying it comes from this session \
             and how to answer (session_send back to this session's id). A session that ended \
             is continued with the message as a session Mother Claude runs, unless \
             start_if_ended is false. Asks the user's approval in Mother Claude unless a \
             permission covers it. To wait for the answer, use session_ask.",
            json!({
                "type": "object",
                "properties": {
                    "session": { "type": "string", "description": "Its id (session_list), the first 8 characters of it, or its exact name." },
                    "computer": { "type": "string", "description": COMPUTER },
                    "message": { "type": "string", "description": "What to tell it." },
                    "start_if_ended": { "type": "boolean", "description": "Continue it when it has ended (default true)." }
                },
                "required": ["session", "message"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                open_world_hint: Some(false),
                ..Default::default()
            },
        ),
        Some(TALK_LIMIT),
        |ctx: ToolCtx, args: Value| async move {
            let session = session_arg(&args)?;
            let message = message_arg(&args, "message")?;
            let me = target::me(&ctx.state, &ctx.caller).await?;
            talk::send(
                &ctx.state,
                &me,
                &session,
                text(&args, "computer").as_deref(),
                &message,
                flag(&args, "start_if_ended").unwrap_or(true),
            )
            .await
            .map(ToolOutput::json)
        },
    );

    r.add_with_limit(
        def(
            "session_ask",
            "Ask another session",
            "Ask another Claude Code session (here or on a paired computer) something and wait \
             for its answer: the question goes to it as a message saying it comes from this \
             session, and when that session's turn ends its reply is returned. Waits up to \
             timeout_s (default 300, at most 1800); on a timeout it says what it knows and \
             session_read shows the answer later. A session that ended is continued with the \
             question unless start_if_ended is false. Asks the user's approval in Mother Claude \
             unless a permission covers it.",
            json!({
                "type": "object",
                "properties": {
                    "session": { "type": "string", "description": "Its id (session_list), the first 8 characters of it, or its exact name." },
                    "computer": { "type": "string", "description": COMPUTER },
                    "question": { "type": "string", "description": "What to ask it." },
                    "timeout_s": { "type": "integer", "minimum": 1, "maximum": ASK_MAX_SECS, "description": "How long to wait for its answer (default 300)." },
                    "start_if_ended": { "type": "boolean", "description": "Continue it when it has ended (default true)." }
                },
                "required": ["session", "question"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                open_world_hint: Some(false),
                ..Default::default()
            },
        ),
        Some(ASK_LIMIT),
        |ctx: ToolCtx, args: Value| async move {
            let session = session_arg(&args)?;
            let question = message_arg(&args, "question")?;
            let timeout = timeout_arg(&args)?;
            let me = target::me(&ctx.state, &ctx.caller).await?;
            talk::ask(
                &ctx.state,
                &me,
                &session,
                text(&args, "computer").as_deref(),
                &question,
                timeout,
                flag(&args, "start_if_ended").unwrap_or(true),
            )
            .await
            .map(ToolOutput::json)
        },
    );

    r.add_with_limit(
        def(
            "session_start",
            "Start a session in a project",
            "Start a new Claude Code session that Mother Claude runs, in a project folder here \
             or on a paired computer, with a first message (saying it comes from this session). \
             Name the folder (an absolute path on that computer) or the project (its folder's \
             name; session_list with projects: true lists them). The folder must be one Claude \
             Code was used in there. wait: true waits for its first turn to end and returns its \
             answer (up to timeout_s). Asks the user's approval in Mother Claude unless a \
             permission covers it.",
            json!({
                "type": "object",
                "properties": {
                    "folder": { "type": "string", "description": "The project folder, an absolute path on that computer." },
                    "project": { "type": "string", "description": "Or the project's name (its folder's name)." },
                    "computer": { "type": "string", "description": "Where: this one (\"here\", the default) or a paired computer's name." },
                    "prompt": { "type": "string", "description": "Its first message." },
                    "model": { "type": "string", "description": "A model id or alias (default: Claude Code's)." },
                    "wait": { "type": "boolean", "description": "Wait for its first answer (default false)." },
                    "timeout_s": { "type": "integer", "minimum": 1, "maximum": ASK_MAX_SECS, "description": "With wait: how long (default 300)." }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                open_world_hint: Some(false),
                ..Default::default()
            },
        ),
        Some(ASK_LIMIT),
        |ctx: ToolCtx, args: Value| async move {
            let prompt = message_arg(&args, "prompt")?;
            let wait = flag(&args, "wait").unwrap_or(false);
            let timeout = timeout_arg(&args)?;
            let me = target::me(&ctx.state, &ctx.caller).await?;
            talk::start(
                &ctx.state,
                &me,
                talk::StartArgs {
                    folder: text(&args, "folder"),
                    project: text(&args, "project"),
                    computer: text(&args, "computer"),
                    prompt,
                    model: text(&args, "model"),
                    wait,
                    timeout,
                },
            )
            .await
            .map(ToolOutput::json)
        },
    );
}
