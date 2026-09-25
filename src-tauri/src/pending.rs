//! Pending human decisions, v2 (design §4.2).
//!
//! Every permission prompt, question, plan approval or elicitation that a human
//! must answer — whichever channel raised it (owned Path B stream-json, the
//! hooks plugin, the HTTP bridge host, the Path A sidecar) — is one
//! [`PendingItem`] in the [`PendingStore`]:
//!
//! * multi-slot per session, keyed by request id (parallel subagents no longer
//!   overwrite each other);
//! * [`PendingStore::register`] returns a `oneshot::Receiver<Decision>` the
//!   raising channel awaits;
//! * [`PendingStore::resolve`] applies the caller's policy (dangerous answers
//!   need a local caller or an `allowDangerous` device), removes the item and
//!   delivers the decision;
//! * [`PendingStore::withdraw`] removes an item that no longer needs an answer
//!   (answered elsewhere, tool finished, process exited);
//! * a late answer to a decided/withdrawn item is `409 already_decided`.
//!
//! The store itself is pure and synchronous (unit-tested below). The async
//! free functions ([`raise`], [`withdraw`], [`respond`], …) wrap it and keep
//! the session snapshot / WebSocket clients / push in sync.
//!
//! Every item is classified server-side ([`classify_dangerous`]) whatever
//! channel raised it; a channel's own flag can only raise the result. Items
//! leave the store through [`respond`] / [`withdraw`] / the [`PendingGuard`],
//! which also publish a `pendingResolved` frame (who answered, or why it went
//! away) and remember the resolution for `409 already_decided`.
//!
//! Owner: foundation; classification, suggestion rewriting and resolution
//! tracking: B3. Channels (B3 owned, B4 hooks) only call the API.

mod danger;

pub use danger::{sensitive_command, sensitive_path, DangerEnv};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::oneshot;

pub use crate::claude::model::{
    PendingItem, PendingKind, PendingOutcome, PendingResolved, PendingSource, PermissionSuggestion,
    Question, QuestionOption, ResolvedBy, ResolverKind,
};
use crate::server::context::{ClientContext, Principal};
use crate::server::error::ApiError;
use crate::state::AppState;

/// How long a raising channel should wait by default before giving up.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// Permission modes that remove the human from the loop entirely.
pub const UNGATED_MODES: [&str; 2] = ["bypassPermissions", "dontAsk"];

/// Modes a plan approval / respond body may switch to.
pub const RESPOND_MODES: [&str; 6] = [
    "default",
    "acceptEdits",
    "plan",
    "auto",
    "bypassPermissions",
    "dontAsk",
];

/// How many decided/withdrawn ids to remember for `409 already_decided`.
const FINISHED_CAP: usize = 2048;

/// Allow or deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Allow,
    #[default]
    Deny,
}

/// A human's answer: the fields of `RespondBody` minus `requestId`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub decision: Verdict,
    /// Index into the item's `suggestions` ("always allow …").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<usize>,
    /// Deny feedback / plan feedback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// AskUserQuestion: question text → chosen label (multi: comma-joined) or
    /// free text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<BTreeMap<String, String>>,
    /// Plan approval → switch to this permission mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Deny and stop the turn.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interrupt: bool,
    /// Elicitation: the form content to accept with (allow = accept, deny =
    /// decline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
}

impl Decision {
    pub fn allow() -> Self {
        Self {
            decision: Verdict::Allow,
            ..Default::default()
        }
    }

    pub fn deny(message: Option<String>) -> Self {
        Self {
            decision: Verdict::Deny,
            message,
            ..Default::default()
        }
    }

    pub fn is_allow(&self) -> bool {
        self.decision == Verdict::Allow
    }

    /// For a single-answer consumer (the legacy sidecar `ask_user`, the bridge
    /// host's `question` kind): the answers joined, else the message.
    pub fn answer_text(&self) -> Option<String> {
        match &self.answers {
            Some(a) if !a.is_empty() => Some(a.values().cloned().collect::<Vec<_>>().join("\n")),
            _ => self.message.clone(),
        }
    }
}

/// `POST /api/sessions/{id}/respond` body. Parsed strictly: unknown fields
/// are rejected (a typo must not silently turn into a plain "allow").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "RespondWire")]
pub struct RespondBody {
    pub request_id: String,
    #[serde(flatten)]
    pub decision: Decision,
}

/// The wire shape of [`RespondBody`] (flatten cannot deny unknown fields).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RespondWire {
    request_id: String,
    decision: Verdict,
    #[serde(default)]
    suggestion: Option<usize>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    answers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    interrupt: bool,
    #[serde(default)]
    content: Option<Value>,
}

impl From<RespondWire> for RespondBody {
    fn from(w: RespondWire) -> Self {
        Self {
            request_id: w.request_id,
            decision: Decision {
                decision: w.decision,
                suggestion: w.suggestion,
                message: w.message,
                answers: w.answers,
                mode: w.mode,
                interrupt: w.interrupt,
                content: w.content,
            },
        }
    }
}

/// Why a response was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RespondError {
    /// No such request (never existed, or long forgotten).
    NotFound,
    /// Already answered, withdrawn, or its raiser stopped waiting.
    AlreadyDecided,
    /// The item was observed on disk; nothing here can answer it.
    NotAnswerable,
    /// A dangerous answer from a caller without the right.
    Forbidden(String),
    /// The body does not fit the item (bad suggestion index, unknown mode).
    Invalid(String),
}

impl RespondError {
    pub fn into_api(self, request_id: &str) -> ApiError {
        match self {
            RespondError::NotFound => ApiError::not_found(
                "request_not_found",
                format!("No pending request {request_id}."),
            ),
            RespondError::AlreadyDecided => ApiError::conflict(
                "already_decided",
                "This request was already answered or withdrawn.",
            ),
            RespondError::NotAnswerable => ApiError::conflict(
                "not_answerable",
                "This prompt was seen in the transcript but cannot be answered from here.",
            ),
            RespondError::Forbidden(msg) => ApiError::dangerous_blocked(msg),
            RespondError::Invalid(msg) => ApiError::unprocessable("invalid_response", msg),
        }
        .with("requestId", request_id)
    }
}

impl std::fmt::Display for RespondError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

struct Slot {
    item: PendingItem,
    tx: Option<oneshot::Sender<Decision>>,
}

#[derive(Default)]
struct Inner {
    slots: HashMap<String, Slot>,
    finished: VecDeque<String>,
    finished_set: HashSet<String>,
    /// How each recently finished item was resolved (same cap as `finished`).
    resolutions: HashMap<String, PendingResolved>,
}

impl Inner {
    fn finish(&mut self, id: &str) {
        if self.finished_set.insert(id.to_string()) {
            self.finished.push_back(id.to_string());
            while self.finished.len() > FINISHED_CAP {
                if let Some(old) = self.finished.pop_front() {
                    self.finished_set.remove(&old);
                    self.resolutions.remove(&old);
                }
            }
        }
    }
}

/// The pending-decision store. Cheap `std::sync::Mutex`: held only for map
/// operations, never across an await.
#[derive(Default)]
pub struct PendingStore {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for PendingStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingStore")
            .field("items", &self.lock().slots.len())
            .finish()
    }
}

impl PendingStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Register an item and get the receiver its decision will arrive on.
    ///
    /// The returned id is the item's id — freshly generated when the item had
    /// none, **or when it collides** with a live or recently finished one (a
    /// caller must never be able to replace someone else's blocked request).
    /// `created_at` is filled when zero. Registered items are answerable unless
    /// their source is `Observed`.
    pub fn register(&self, mut item: PendingItem) -> (String, oneshot::Receiver<Decision>) {
        // Server-side classification (no cwd here: `raise` classifies with
        // the session's cwd first). A caller's flag can only raise it.
        if !item.dangerous || item.dangerous_reason.is_none() {
            if let Some(why) = classify_dangerous(&item, None) {
                item.dangerous = true;
                item.dangerous_reason.get_or_insert(why);
            }
        }
        let (tx, rx) = oneshot::channel();
        let mut g = self.lock();
        if item.id.is_empty() || g.slots.contains_key(&item.id) || g.finished_set.contains(&item.id)
        {
            item.id = uuid::Uuid::new_v4().to_string();
        }
        if item.created_at == 0 {
            item.created_at = crate::claude::registry::now_ms();
        }
        item.answerable = item.source != PendingSource::Observed;
        let id = item.id.clone();
        g.slots.insert(id.clone(), Slot { item, tx: Some(tx) });
        (id, rx)
    }

    /// Answer an item on behalf of `ctx`. On success the item is removed, the
    /// decision is delivered to its raiser, and the item is returned.
    pub fn resolve(
        &self,
        id: &str,
        decision: Decision,
        ctx: &ClientContext,
    ) -> Result<PendingItem, RespondError> {
        let mut g = self.lock();
        let Some(slot) = g.slots.get(id) else {
            return Err(if g.finished_set.contains(id) {
                RespondError::AlreadyDecided
            } else {
                RespondError::NotFound
            });
        };
        if !slot.item.answerable {
            return Err(RespondError::NotAnswerable);
        }
        validate(&slot.item, &decision)?;
        let dangerous = decision.is_allow() && is_dangerous(&slot.item, &decision);
        if dangerous && !ctx.may_dangerous() {
            return Err(RespondError::Forbidden(
                "Approving a dangerous action — bypassPermissions, or an \"always allow\" that \
                 is written to settings or widens the filesystem — is restricted to this \
                 computer or a device you granted dangerous actions."
                    .into(),
            ));
        }
        if dangerous {
            // Audited, and a device passing the gate raises a desktop notice.
            ctx.note_dangerous("approve a dangerous action");
        }
        let Some(mut slot) = g.slots.remove(id) else {
            return Err(RespondError::NotFound);
        };
        g.finish(id);
        let resolution = resolution_for_decision(&slot.item, &decision, ctx, None);
        let delivered = match slot.tx.take() {
            Some(tx) => tx.send(decision).is_ok(),
            None => false,
        };
        if !delivered {
            // The raiser stopped waiting in the meantime: it was withdrawn.
            let (outcome, kind) = outcome_for_reason("request ended");
            g.resolutions.insert(
                id.to_string(),
                resolved(&slot.item, outcome, Some(kind), None, None),
            );
            return Err(RespondError::AlreadyDecided);
        }
        g.resolutions.insert(id.to_string(), resolution);
        Ok(slot.item)
    }

    /// Remove an item that no longer needs an answer. Its raiser's receiver
    /// errors (the sender is dropped). Returns the item if it was pending.
    ///
    /// `reason` is free text; it also decides the published outcome (see
    /// [`outcome_for_reason`]: "timeout"/"expired" → expired, "cancel",
    /// "request ended", "exited", "stopped", "removed" → cancelled, anything
    /// else — e.g. "answered in VS Code", "tool finished" → withdrawn).
    pub fn withdraw(&self, id: &str, reason: &str) -> Option<PendingItem> {
        let mut g = self.lock();
        let slot = g.slots.remove(id)?;
        g.finish(id);
        let (outcome, kind) = outcome_for_reason(reason);
        g.resolutions.insert(
            id.to_string(),
            resolved(&slot.item, outcome, Some(kind), None, Some(reason)),
        );
        tracing::debug!(request = %id, session = %slot.item.session_id, %reason, "pending withdrawn");
        Some(slot.item)
    }

    /// How a finished item was resolved (for `409 already_decided` and the
    /// `pendingResolved` frame), while it is still remembered.
    pub fn resolution(&self, id: &str) -> Option<PendingResolved> {
        self.lock().resolutions.get(id).cloned()
    }

    /// Replace the remembered resolution of a finished item (the async layer
    /// adds the device name the sync store cannot look up).
    pub fn set_resolution(&self, resolution: PendingResolved) {
        let mut g = self.lock();
        if g.finished_set.contains(&resolution.id) {
            g.resolutions.insert(resolution.id.clone(), resolution);
        }
    }

    /// Withdraw every item of one session (process exited, session stopped).
    pub fn withdraw_session(&self, session_id: &str, reason: &str) -> Vec<PendingItem> {
        let ids: Vec<String> = self
            .lock()
            .slots
            .values()
            .filter(|s| s.item.session_id == session_id)
            .map(|s| s.item.id.clone())
            .collect();
        ids.iter()
            .filter_map(|id| self.withdraw(id, reason))
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<PendingItem> {
        self.lock().slots.get(id).map(|s| s.item.clone())
    }

    /// Whether the id was decided or withdrawn recently.
    pub fn is_finished(&self, id: &str) -> bool {
        self.lock().finished_set.contains(id)
    }

    /// One session's items, oldest first.
    pub fn list(&self, session_id: &str) -> Vec<PendingItem> {
        let mut items: Vec<PendingItem> = self
            .lock()
            .slots
            .values()
            .filter(|s| s.item.session_id == session_id)
            .map(|s| s.item.clone())
            .collect();
        sort(&mut items);
        items
    }

    /// Every item across sessions, oldest first (the inbox).
    pub fn all(&self) -> Vec<PendingItem> {
        let mut items: Vec<PendingItem> =
            self.lock().slots.values().map(|s| s.item.clone()).collect();
        sort(&mut items);
        items
    }

    /// The live item of `session_id` raised for tool call `tool_use_id` (hook
    /// and owned channels withdraw by tool call).
    pub fn find_by_tool_use(&self, session_id: &str, tool_use_id: &str) -> Option<PendingItem> {
        self.lock()
            .slots
            .values()
            .find(|s| {
                s.item.session_id == session_id
                    && s.item.tool_use_id.as_deref() == Some(tool_use_id)
            })
            .map(|s| s.item.clone())
    }

    /// The oldest answerable item of a session (legacy `/permission` and
    /// `/answer` routes that do not name a request).
    pub fn oldest_for(&self, session_id: &str) -> Option<PendingItem> {
        self.list(session_id).into_iter().find(|i| i.answerable)
    }

    pub fn len(&self) -> usize {
        self.lock().slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn sort(items: &mut [PendingItem]) {
    items.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn validate(item: &PendingItem, d: &Decision) -> Result<(), RespondError> {
    if let Some(i) = d.suggestion {
        if !item.suggestions.iter().any(|s| s.index == i) {
            return Err(RespondError::Invalid(format!(
                "Suggestion {i} does not exist on this request."
            )));
        }
    }
    if let Some(mode) = &d.mode {
        if !RESPOND_MODES.contains(&mode.as_str()) {
            return Err(RespondError::Invalid(format!(
                "Unknown mode {mode:?}; expected one of {}.",
                RESPOND_MODES.join(", ")
            )));
        }
        if item.kind != PendingKind::Plan {
            return Err(RespondError::Invalid(
                "`mode` only applies to plan approvals.".into(),
            ));
        }
    }
    if d.answers.as_ref().is_some_and(|a| !a.is_empty()) && item.kind != PendingKind::Question {
        return Err(RespondError::Invalid(
            "`answers` only apply to questions.".into(),
        ));
    }
    if d.content.as_ref().is_some_and(|c| !c.is_null()) {
        if item.kind != PendingKind::Elicitation {
            return Err(RespondError::Invalid(
                "`content` only applies to elicitation requests.".into(),
            ));
        }
        if !d.content.as_ref().is_some_and(Value::is_object) {
            return Err(RespondError::Invalid("`content` must be an object.".into()));
        }
    }
    Ok(())
}

/// Why approving `item` is dangerous regardless of the chosen suggestion
/// (design §10 B3): it writes Claude Code / Mother Claude configuration,
/// credentials, shell startup files, launch agents or git hooks, or its
/// command does. `cwd` is the session's working directory (relative paths
/// resolve against it). Returns the human reason.
pub fn classify_dangerous(item: &PendingItem, cwd: Option<&str>) -> Option<String> {
    danger::classify(item, &DangerEnv::current(cwd))
}

/// The outcome and resolver kind a free-text withdraw reason maps to.
pub fn outcome_for_reason(reason: &str) -> (PendingOutcome, ResolverKind) {
    let r = reason.to_ascii_lowercase();
    if r.contains("timeout") || r.contains("timed out") || r.contains("expired") {
        (PendingOutcome::Expired, ResolverKind::Timeout)
    } else if [
        "cancel",
        "request ended",
        "exited",
        "stopped",
        "removed",
        "no longer owned",
        "interrupt",
    ]
    .iter()
    .any(|k| r.contains(k))
    {
        (PendingOutcome::Cancelled, ResolverKind::Host)
    } else {
        (PendingOutcome::Withdrawn, ResolverKind::Host)
    }
}

fn resolved(
    item: &PendingItem,
    outcome: PendingOutcome,
    kind: Option<ResolverKind>,
    name: Option<String>,
    note: Option<&str>,
) -> PendingResolved {
    let by = kind.map(|kind| ResolvedBy { kind, name });
    let who = match &by {
        Some(ResolvedBy {
            kind: ResolverKind::Desktop | ResolverKind::Device,
            name: Some(n),
        }) => format!(" by {n}"),
        Some(ResolvedBy {
            kind: ResolverKind::Device,
            name: None,
        }) => " from a paired device".to_string(),
        _ => String::new(),
    };
    let text = match outcome {
        PendingOutcome::Allowed => format!("Allowed{who}"),
        PendingOutcome::Denied => format!("Denied{who}"),
        PendingOutcome::Answered => format!("Answered{who}"),
        PendingOutcome::Expired => "Nobody answered in time".to_string(),
        PendingOutcome::Cancelled => match note {
            Some(n) if !n.is_empty() => format!("Cancelled ({n})"),
            _ => "Cancelled".to_string(),
        },
        PendingOutcome::Withdrawn => match note {
            Some(n) if !n.is_empty() => format!("Answered elsewhere ({n})"),
            _ => "Answered elsewhere".to_string(),
        },
    };
    PendingResolved {
        id: item.id.clone(),
        session: item.session_id.clone(),
        outcome,
        by,
        at: crate::claude::registry::now_ms(),
        text: Some(text),
    }
}

/// The resolution record of `item` answered with `d` by `ctx` (`name`: the
/// device name when known).
fn resolution_for_decision(
    item: &PendingItem,
    d: &Decision,
    ctx: &ClientContext,
    name: Option<String>,
) -> PendingResolved {
    let outcome = if !d.is_allow() {
        PendingOutcome::Denied
    } else if matches!(item.kind, PendingKind::Question | PendingKind::Elicitation) {
        PendingOutcome::Answered
    } else {
        PendingOutcome::Allowed
    };
    let (kind, name) = match &ctx.principal {
        Principal::Device { .. } => (ResolverKind::Device, name),
        _ => (
            ResolverKind::Desktop,
            Some(name.unwrap_or_else(|| "Mac".into())),
        ),
    };
    let mut r = resolved(item, outcome, Some(kind), name, None);
    if d.is_allow() {
        if let Some(s) = d
            .suggestion
            .and_then(|i| item.suggestions.iter().find(|s| s.index == i))
        {
            if let Some(t) = r.text.as_mut() {
                t.push_str(" · ");
                t.push_str(&s.label);
            }
        }
        if let Some(mode) = &d.mode {
            if let Some(t) = r.text.as_mut() {
                t.push_str(&format!(" · switched to {mode}"));
            }
        }
    }
    r
}

/// Whether allowing `item` with `d` is dangerous: the item says so, the chosen
/// suggestion is standing, or the answer switches to an ungated mode.
pub fn is_dangerous(item: &PendingItem, d: &Decision) -> bool {
    if item.dangerous {
        return true;
    }
    if d.mode
        .as_deref()
        .is_some_and(|m| UNGATED_MODES.contains(&m))
    {
        return true;
    }
    d.suggestion
        .and_then(|i| item.suggestions.iter().find(|s| s.index == i))
        .is_some_and(|s| s.standing || is_standing(&s.raw))
}

/// Whether one `PermissionUpdate` outlives the request it rides in on:
/// written outside the session (userSettings/localSettings/projectSettings),
/// switches to bypassPermissions/dontAsk, or adds directories. Anything that is
/// not an object is treated as standing (we cannot reason about it).
pub fn is_standing(raw: &Value) -> bool {
    let Some(obj) = raw.as_object() else {
        return !raw.is_null();
    };
    let kind = obj.get("type").and_then(Value::as_str).unwrap_or_default();
    let destination = obj
        .get("destination")
        .and_then(Value::as_str)
        .unwrap_or("session");
    if destination != "session" && destination != "cliArg" {
        return true;
    }
    match kind {
        "setMode" => obj
            .get("mode")
            .and_then(Value::as_str)
            .is_some_and(|m| UNGATED_MODES.contains(&m)),
        "addDirectories" => true,
        "addRules" | "replaceRules" => {
            // A tool-wide or wildcard allow rule is a blank cheque even for
            // one session: `Bash`, `Bash(*)`, `Bash(:*)`, a whole
            // `mcp__server`.
            let allows = obj
                .get("behavior")
                .and_then(Value::as_str)
                .is_none_or(|b| b == "allow");
            allows
                && obj
                    .get("rules")
                    .and_then(Value::as_array)
                    .is_some_and(|rules| rules.iter().any(is_blanket_rule))
        }
        _ => false,
    }
}

/// `Bash`, `Bash(*)`, `Bash(:*)`, `mcp__server`, `mcp__server__*`: a rule that
/// covers every use of a tool (or a whole MCP server).
fn is_blanket_rule(rule: &Value) -> bool {
    let tool = rule
        .get("toolName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let content = rule
        .get("ruleContent")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if tool.is_empty() || tool == "*" {
        return true;
    }
    if let Some(rest) = tool.strip_prefix("mcp__") {
        // `mcp__server` or `mcp__server__*` covers the whole server; one named
        // MCP tool (which takes no rule content) is a specific grant.
        let parts: Vec<&str> = rest.split("__").collect();
        return parts.len() < 2 || parts[1].is_empty() || parts[1] == "*";
    }
    matches!(content, "" | "*" | ":*" | "**" | "* *")
}

/// Build the UI description of one CLI permission suggestion.
pub fn suggestion_from_raw(index: usize, raw: &Value) -> PermissionSuggestion {
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let destination = raw
        .get("destination")
        .and_then(Value::as_str)
        .map(str::to_string);
    let rules = || -> String {
        raw.get("rules")
            .and_then(Value::as_array)
            .map(|rs| {
                rs.iter()
                    .filter_map(|r| {
                        let tool = r.get("toolName").and_then(Value::as_str)?;
                        Some(match r.get("ruleContent").and_then(Value::as_str) {
                            Some(c) if !c.is_empty() => format!("{tool}({c})"),
                            _ => tool.to_string(),
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    };
    let base = match kind.as_str() {
        "addRules" | "replaceRules" => {
            let verb = match raw.get("behavior").and_then(Value::as_str) {
                Some("deny") => "Always deny",
                Some("ask") => "Always ask for",
                _ => "Always allow",
            };
            format!("{verb} {}", rules())
        }
        "removeRules" => format!("Stop allowing {}", rules()),
        "setMode" => format!(
            "Switch to {} mode",
            raw.get("mode").and_then(Value::as_str).unwrap_or("another")
        ),
        "addDirectories" | "removeDirectories" => {
            let dirs = raw
                .get("directories")
                .and_then(Value::as_array)
                .map(|d| {
                    d.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            if kind == "addDirectories" {
                format!("Allow access to {dirs}")
            } else {
                format!("Remove access to {dirs}")
            }
        }
        other => other.to_string(),
    };
    let scope = match destination.as_deref() {
        Some("session") => " in this session",
        Some("localSettings") => " in this project (just you)",
        Some("projectSettings") => " in this project (shared)",
        Some("userSettings") => " everywhere",
        _ => "",
    };
    PermissionSuggestion {
        index,
        label: format!("{}{scope}", base.trim()),
        kind,
        destination,
        standing: is_standing(raw),
        raw: raw.clone(),
    }
}

/// Suggestions from a CLI `permission_suggestions` / SDK `suggestions` array.
///
/// Every `addRules` suggestion that would be written to a settings file
/// (a standing grant, so dangerous) is preceded by a copy with its
/// destination rewritten to `session` — "Always allow … in this session" —
/// which any caller may choose (verified to be honoured by the CLI and the
/// hook path without writing anything to disk). Indices are positions in the
/// returned list, and the server always echoes its own stored `raw`.
pub fn suggestions_from_raw(raw: Option<&Value>) -> Vec<PermissionSuggestion> {
    let Some(list) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut raws: Vec<Value> = Vec::with_capacity(list.len() * 2);
    for r in list {
        if let Some(copy) = session_copy(r) {
            if !list.contains(&copy) && !raws.contains(&copy) {
                raws.push(copy);
            }
        }
        raws.push(r.clone());
    }
    raws.iter()
        .enumerate()
        .map(|(i, r)| suggestion_from_raw(i, r))
        .collect()
}

/// A session-scoped copy of an `addRules` suggestion aimed at a settings file.
fn session_copy(raw: &Value) -> Option<Value> {
    let obj = raw.as_object()?;
    if obj.get("type").and_then(Value::as_str) != Some("addRules") {
        return None;
    }
    let destination = obj.get("destination").and_then(Value::as_str)?;
    if !matches!(
        destination,
        "localSettings" | "userSettings" | "projectSettings"
    ) {
        return None;
    }
    let mut copy = obj.clone();
    copy.insert("destination".into(), Value::String("session".into()));
    Some(Value::Object(copy))
}

/// Parse AskUserQuestion-shaped input (`{questions:[{question, header,
/// multiSelect, options:[{label, description}]}]}`) into questions. Options may
/// be plain strings.
pub fn questions_from_input(input: &Value) -> Vec<Question> {
    let list: Vec<&Value> = match input.get("questions").and_then(Value::as_array) {
        Some(qs) => qs.iter().collect(),
        None if input.get("question").is_some() => vec![input],
        None => Vec::new(),
    };
    list.into_iter()
        .filter_map(|q| {
            let question = q.get("question").and_then(Value::as_str)?.to_string();
            let options = q
                .get("options")
                .and_then(Value::as_array)
                .map(|opts| {
                    opts.iter()
                        .filter_map(|o| match o {
                            Value::String(s) => Some(QuestionOption {
                                label: s.clone(),
                                description: None,
                            }),
                            _ => Some(QuestionOption {
                                label: o.get("label")?.as_str()?.to_string(),
                                description: o
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                            }),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Question {
                question,
                header: q.get("header").and_then(Value::as_str).map(str::to_string),
                options,
                multi_select: q
                    .get("multiSelect")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// The CLI/SDK permission result for a decision:
/// `{behavior:"allow", updatedInput?, updatedPermissions?}` or
/// `{behavior:"deny", message, interrupt?}` — the shape shared by the
/// stream-json `control_response` (Path B), the Agent SDK `canUseTool` result
/// (bridge host / sidecar) and the `PermissionRequest` hook decision.
///
/// Allow: `updatedInput` is the item's input with `answers` merged in when the
/// decision carries answers (AskUserQuestion); `updatedPermissions` holds the
/// chosen suggestion's raw update and, when `mode` is set, a session-scoped
/// `setMode`.
pub fn permission_result(item: &PendingItem, d: &Decision) -> Value {
    if item.kind == PendingKind::Elicitation {
        return elicitation_result(d);
    }
    match d.decision {
        Verdict::Allow => {
            let mut out = json!({ "behavior": "allow" });
            let mut input = item.input.clone();
            if let Some(answers) = d.answers.as_ref().filter(|a| !a.is_empty()) {
                let mut obj = match input.take() {
                    Some(Value::Object(m)) => m,
                    _ => serde_json::Map::new(),
                };
                obj.insert("answers".into(), json!(answers));
                input = Some(Value::Object(obj));
            }
            if let Some(input) = input {
                out["updatedInput"] = input;
            }
            let mut perms = Vec::new();
            if let Some(s) = d
                .suggestion
                .and_then(|i| item.suggestions.iter().find(|s| s.index == i))
            {
                perms.push(s.raw.clone());
            }
            if let Some(mode) = &d.mode {
                perms.push(json!({ "type": "setMode", "mode": mode, "destination": "session" }));
            }
            if !perms.is_empty() {
                out["updatedPermissions"] = Value::Array(perms);
            }
            out
        }
        Verdict::Deny => {
            let mut out = json!({
                "behavior": "deny",
                "message": d.message.clone().filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| "Denied from Mother Claude.".into()),
            });
            if d.interrupt {
                out["interrupt"] = json!(true);
            }
            out
        }
    }
}

/// The MCP elicitation result for a decision: allow = `{action:"accept",
/// content}`, deny = `{action:"decline"}`.
pub fn elicitation_result(d: &Decision) -> Value {
    if d.is_allow() {
        json!({
            "action": "accept",
            "content": d.content.clone().filter(Value::is_object).unwrap_or_else(|| json!({})),
        })
    } else {
        json!({ "action": "decline" })
    }
}

// ───────────────────────── state-level helpers (async) ─────────────────────────

/// Withdraws its item when dropped — hold it for as long as the raiser waits,
/// so a cancelled request (client gone, process exited, timeout) never leaves a
/// stale card behind.
pub struct PendingGuard {
    state: AppState,
    id: String,
    session_id: String,
}

impl PendingGuard {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if self
            .state
            .pending
            .withdraw(&self.id, "request ended")
            .is_none()
        {
            return; // already resolved (and published) or withdrawn
        }
        publish_resolution(&self.state, &self.id);
        let (state, session) = (self.state.clone(), self.session_id.clone());
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { changed(&state, &session).await });
        }
    }
}

/// Broadcast the remembered resolution of `id` (`pendingResolved`) and keep
/// a `decision` row for the session's timeline (merged into transcript pages).
fn publish_resolution(state: &AppState, id: &str) {
    if let Some(r) = state.pending.resolution(id) {
        if let Some(text) = r.text.clone().filter(|t| !t.trim().is_empty()) {
            state.timeline.push(
                &r.session,
                crate::claude::model::UiEvent {
                    id: format!("decision:{}", r.id),
                    ts: Some(r.at),
                    role: crate::claude::model::UiRole::System,
                    kind: crate::claude::model::UiKind::Decision,
                    text: Some(text),
                    ..Default::default()
                },
            );
        }
        crate::server::ws::publish_pending_resolved(state, r);
    }
}

/// Register `item`, publish it to every client, fire the push trigger, and
/// return the guard plus the decision receiver.
pub async fn raise(
    state: &AppState,
    mut item: PendingItem,
) -> (PendingGuard, oneshot::Receiver<Decision>) {
    let session_id = item.session_id.clone();
    // Classify against the session's cwd (relative paths, symlinks).
    let cwd = match state.find_session(&session_id).await {
        Some(s) if !s.cwd.is_empty() => Some(s.cwd),
        _ => state.control.cwd_of(&session_id),
    };
    if let Some(why) = classify_dangerous(&item, cwd.as_deref()) {
        item.dangerous = true;
        item.dangerous_reason.get_or_insert(why);
    }
    let (id, rx) = state.pending.register(item);
    changed(state, &session_id).await;
    if let Some(item) = state.pending.get(&id) {
        crate::server::push::on_pending_added(state, &item);
    }
    (
        PendingGuard {
            state: state.clone(),
            id,
            session_id,
        },
        rx,
    )
}

/// Await a decision for at most `timeout`. `None` = timed out or withdrawn.
pub async fn wait(rx: oneshot::Receiver<Decision>, timeout: Duration) -> Option<Decision> {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(d)) => Some(d),
        _ => None,
    }
}

/// Withdraw one item and publish the change.
pub async fn withdraw(state: &AppState, id: &str, reason: &str) -> Option<PendingItem> {
    let item = state.pending.withdraw(id, reason)?;
    publish_resolution(state, id);
    changed(state, &item.session_id).await;
    Some(item)
}

/// Withdraw all items of a session and publish the change.
pub async fn withdraw_session(
    state: &AppState,
    session_id: &str,
    reason: &str,
) -> Vec<PendingItem> {
    let items = state.pending.withdraw_session(session_id, reason);
    for item in &items {
        publish_resolution(state, &item.id);
    }
    if !items.is_empty() {
        changed(state, session_id).await;
    }
    items
}

/// Answer `request_id` of `session_id` on behalf of `ctx` and publish.
pub async fn respond(
    state: &AppState,
    session_id: &str,
    request_id: &str,
    decision: Decision,
    ctx: &ClientContext,
) -> Result<PendingItem, RespondError> {
    match state.pending.get(request_id) {
        Some(item) if item.session_id != session_id => return Err(RespondError::NotFound),
        _ => {}
    }
    let item = state.pending.resolve(request_id, decision.clone(), ctx)?;
    // The store cannot look up device names; fill it in, then publish.
    if let Some(device) = ctx.device_id() {
        let name = state
            .devices
            .get(device)
            .map(|d| d.name)
            .filter(|n| !n.trim().is_empty());
        if name.is_some() {
            state
                .pending
                .set_resolution(resolution_for_decision(&item, &decision, ctx, name));
        }
    }
    publish_resolution(state, request_id);
    changed(state, session_id).await;
    Ok(item)
}

/// Re-publish one session's pending list (snapshot + WebSocket diff).
pub async fn changed(state: &AppState, session_id: &str) {
    crate::server::ws::sync_pending(state, session_id).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::context::{ClientContext, Ingress, Principal};
    use axum::http::HeaderMap;

    fn item(session: &str) -> PendingItem {
        PendingItem {
            session_id: session.into(),
            kind: PendingKind::Permission,
            source: PendingSource::Owned,
            title: "Bash".into(),
            tool_name: Some("Bash".into()),
            input: Some(json!({ "command": "ls" })),
            ..Default::default()
        }
    }

    fn local() -> ClientContext {
        let mut h = HeaderMap::new();
        h.insert("host", "127.0.0.1:6725".parse().unwrap());
        let mut c =
            ClientContext::new(Ingress::Loopback, "127.0.0.1:9".parse().unwrap(), &h, false);
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

    #[tokio::test]
    async fn multi_slot_register_and_resolve() {
        let store = PendingStore::new();
        let (a, rx_a) = store.register(item("s1"));
        let (b, _rx_b) = store.register(item("s1"));
        assert_ne!(a, b);
        assert_eq!(store.list("s1").len(), 2);
        assert!(store.list("s1").iter().all(|i| i.answerable));
        assert!(store.list("s2").is_empty());

        let resolved = store.resolve(&a, Decision::allow(), &phone(false)).unwrap();
        assert_eq!(resolved.id, a);
        assert!(rx_a.await.unwrap().is_allow());
        assert_eq!(store.list("s1").len(), 1);

        // A late second answer is a conflict, an unknown id a 404.
        assert_eq!(
            store.resolve(&a, Decision::allow(), &local()).unwrap_err(),
            RespondError::AlreadyDecided
        );
        assert_eq!(
            store
                .resolve("nope", Decision::allow(), &local())
                .unwrap_err(),
            RespondError::NotFound
        );
    }

    #[test]
    fn colliding_ids_never_replace_a_live_request() {
        let store = PendingStore::new();
        let mut first = item("s1");
        first.id = "req-1".into();
        let (id1, _rx1) = store.register(first.clone());
        let (id2, _rx2) = store.register(first);
        assert_eq!(id1, "req-1");
        assert_ne!(id2, "req-1");
        assert_eq!(store.len(), 2);
    }

    #[tokio::test]
    async fn withdraw_drops_the_sender_and_late_answers_conflict() {
        let store = PendingStore::new();
        let (id, rx) = store.register(item("s1"));
        assert!(store.withdraw(&id, "tool finished").is_some());
        assert!(rx.await.is_err());
        assert_eq!(
            store.resolve(&id, Decision::allow(), &local()).unwrap_err(),
            RespondError::AlreadyDecided
        );
        assert!(store.withdraw(&id, "again").is_none());
    }

    #[tokio::test]
    async fn raiser_that_stopped_waiting_is_already_decided() {
        let store = PendingStore::new();
        let (id, rx) = store.register(item("s1"));
        drop(rx);
        assert_eq!(
            store.resolve(&id, Decision::allow(), &local()).unwrap_err(),
            RespondError::AlreadyDecided
        );
    }

    #[test]
    fn dangerous_policy() {
        let store = PendingStore::new();

        // Item flagged dangerous: phone blocked, granted phone and desktop allowed.
        let mut dangerous = item("s1");
        dangerous.dangerous = true;
        let (id, _rx) = store.register(dangerous.clone());
        assert!(matches!(
            store.resolve(&id, Decision::allow(), &phone(false)),
            Err(RespondError::Forbidden(_))
        ));
        // Denying is never dangerous.
        let (id2, _rx2) = store.register(dangerous.clone());
        assert!(store
            .resolve(&id2, Decision::deny(None), &phone(false))
            .is_ok());
        assert!(store.resolve(&id, Decision::allow(), &phone(true)).is_ok());

        // Standing suggestion.
        let mut it = item("s1");
        it.suggestions = suggestions_from_raw(Some(&json!([
            { "type": "addRules", "behavior": "allow", "destination": "session",
              "rules": [{ "toolName": "Bash", "ruleContent": "npm test:*" }] },
            { "type": "addRules", "behavior": "allow", "destination": "userSettings",
              "rules": [{ "toolName": "Bash" }] },
        ])));
        assert!(!it.suggestions[0].standing);
        assert!(it.suggestions[1].standing);
        let (id, _rx) = store.register(it.clone());
        let mut d = Decision::allow();
        d.suggestion = Some(1);
        assert!(matches!(
            store.resolve(&id, d.clone(), &phone(false)),
            Err(RespondError::Forbidden(_))
        ));
        d.suggestion = Some(0);
        assert!(store.resolve(&id, d, &phone(false)).is_ok());

        // Plan approval into bypassPermissions.
        let mut plan = item("s1");
        plan.kind = PendingKind::Plan;
        let (id, _rx) = store.register(plan);
        let mut d = Decision::allow();
        d.mode = Some("bypassPermissions".into());
        assert!(matches!(
            store.resolve(&id, d.clone(), &phone(false)),
            Err(RespondError::Forbidden(_))
        ));
        assert!(store.resolve(&id, d, &local()).is_ok());
    }

    #[test]
    fn invalid_bodies_and_observed_items() {
        let store = PendingStore::new();
        let (id, _rx) = store.register(item("s1"));
        let mut d = Decision::allow();
        d.suggestion = Some(7);
        assert!(matches!(
            store.resolve(&id, d, &local()),
            Err(RespondError::Invalid(_))
        ));
        let mut d = Decision::allow();
        d.mode = Some("yolo".into());
        assert!(matches!(
            store.resolve(&id, d, &local()),
            Err(RespondError::Invalid(_))
        ));

        let mut observed = item("s1");
        observed.source = PendingSource::Observed;
        let (oid, _rx) = store.register(observed);
        assert_eq!(
            store
                .resolve(&oid, Decision::allow(), &local())
                .unwrap_err(),
            RespondError::NotAnswerable
        );
    }

    #[test]
    fn withdraw_session_and_ordering() {
        let store = PendingStore::new();
        let mut a = item("s1");
        a.created_at = 20;
        let mut b = item("s1");
        b.created_at = 10;
        let (ida, _ra) = store.register(a);
        let (idb, _rb) = store.register(b);
        store.register(item("s2"));
        let list = store.list("s1");
        assert_eq!(list[0].id, idb);
        assert_eq!(list[1].id, ida);
        assert_eq!(store.oldest_for("s1").unwrap().id, idb);
        let mut tooled = item("s3");
        tooled.tool_use_id = Some("toolu_9".into());
        let (tid, _rt) = store.register(tooled);
        assert_eq!(store.find_by_tool_use("s3", "toolu_9").unwrap().id, tid);
        assert!(store.find_by_tool_use("s1", "toolu_9").is_none());
        store.withdraw(&tid, "done");
        assert_eq!(store.all().len(), 3);
        assert_eq!(store.withdraw_session("s1", "exited").len(), 2);
        assert_eq!(store.all().len(), 1);
    }

    #[test]
    fn permission_results() {
        let mut it = item("s1");
        it.suggestions = suggestions_from_raw(Some(&json!([
            { "type": "addRules", "behavior": "allow", "destination": "localSettings",
              "rules": [{ "toolName": "Bash", "ruleContent": "ls" }] }
        ])));
        // The session-scoped copy comes first, then the CLI's own.
        assert_eq!(it.suggestions.len(), 2);
        assert_eq!(
            it.suggestions[0].label,
            "Always allow Bash(ls) in this session"
        );
        assert!(!it.suggestions[0].standing);
        assert_eq!(
            it.suggestions[1].label,
            "Always allow Bash(ls) in this project (just you)"
        );
        assert!(it.suggestions[1].standing);
        let mut d = Decision::allow();
        d.suggestion = Some(1);
        let r = permission_result(&it, &d);
        assert_eq!(r["behavior"], "allow");
        assert_eq!(r["updatedInput"]["command"], "ls");
        assert_eq!(r["updatedPermissions"][0]["destination"], "localSettings");
        d.suggestion = Some(0);
        let r = permission_result(&it, &d);
        assert_eq!(r["updatedPermissions"][0]["destination"], "session");
        assert_eq!(r["updatedPermissions"][0]["rules"][0]["ruleContent"], "ls");

        // AskUserQuestion answers merge into updatedInput.
        let mut q = item("s1");
        q.kind = PendingKind::Question;
        q.input = Some(json!({ "questions": [{ "question": "Which?", "options": ["a", "b"] }] }));
        let mut d = Decision::allow();
        d.answers = Some(BTreeMap::from([("Which?".to_string(), "a".to_string())]));
        let r = permission_result(&q, &d);
        assert_eq!(r["updatedInput"]["answers"]["Which?"], "a");
        assert!(r["updatedInput"]["questions"].is_array());
        assert_eq!(d.answer_text().as_deref(), Some("a"));

        // Plan approval with a mode switch.
        let mut d = Decision::allow();
        d.mode = Some("acceptEdits".into());
        let mut plan = item("s1");
        plan.kind = PendingKind::Plan;
        plan.input = Some(json!({ "plan": "1. do it" }));
        let r = permission_result(&plan, &d);
        assert_eq!(r["updatedPermissions"][0]["type"], "setMode");
        assert_eq!(r["updatedPermissions"][0]["mode"], "acceptEdits");
        assert_eq!(r["updatedPermissions"][0]["destination"], "session");

        // Deny with interrupt.
        let mut d = Decision::deny(Some("no".into()));
        d.interrupt = true;
        let r = permission_result(&item("s1"), &d);
        assert_eq!(r["behavior"], "deny");
        assert_eq!(r["message"], "no");
        assert_eq!(r["interrupt"], true);
        let r = permission_result(&item("s1"), &Decision::deny(None));
        assert!(r["message"].as_str().unwrap().contains("Denied"));
        assert!(r.get("interrupt").is_none());
    }

    #[test]
    fn standing_classification() {
        assert!(!is_standing(
            &json!({ "type": "addRules", "destination": "session" })
        ));
        assert!(!is_standing(
            &json!({ "type": "addRules", "destination": "cliArg" })
        ));
        assert!(is_standing(
            &json!({ "type": "addRules", "destination": "projectSettings" })
        ));
        assert!(is_standing(
            &json!({ "type": "setMode", "mode": "bypassPermissions", "destination": "session" })
        ));
        assert!(!is_standing(
            &json!({ "type": "setMode", "mode": "acceptEdits", "destination": "session" })
        ));
        assert!(is_standing(
            &json!({ "type": "addDirectories", "directories": ["/"], "destination": "session" })
        ));
        assert!(is_standing(&json!("weird")));
        assert!(!is_standing(&Value::Null));
        // Tool-wide / wildcard rules are standing even for one session.
        for rules in [
            json!([{ "toolName": "Bash" }]),
            json!([{ "toolName": "Bash", "ruleContent": "*" }]),
            json!([{ "toolName": "Bash", "ruleContent": ":*" }]),
            json!([{ "toolName": "mcp__github" }]),
            json!([{ "toolName": "mcp__github__*" }]),
        ] {
            assert!(
                is_standing(&json!({ "type": "addRules", "behavior": "allow",
                    "destination": "session", "rules": rules })),
                "{rules}"
            );
        }
        assert!(!is_standing(
            &json!({ "type": "addRules", "behavior": "allow",
            "destination": "session",
            "rules": [{ "toolName": "Bash", "ruleContent": "npm test:*" }] })
        ));
        assert!(!is_standing(
            &json!({ "type": "addRules", "behavior": "allow",
            "destination": "session", "rules": [{ "toolName": "mcp__github__get_issue" }] })
        ));
        // A deny rule is never a blank cheque.
        assert!(!is_standing(
            &json!({ "type": "addRules", "behavior": "deny",
            "destination": "session", "rules": [{ "toolName": "Bash" }] })
        ));
    }

    #[test]
    fn session_copies_of_settings_suggestions() {
        let s = suggestions_from_raw(Some(&json!([
            { "type": "addRules", "behavior": "allow", "destination": "localSettings",
              "rules": [{ "toolName": "Bash", "ruleContent": "touch x1 *" }] },
            { "type": "addDirectories", "directories": ["/tmp/x"], "destination": "session" },
            { "type": "setMode", "mode": "acceptEdits", "destination": "session" },
            // Already offered for the session: no duplicate.
            { "type": "addRules", "behavior": "allow", "destination": "session",
              "rules": [{ "toolName": "Read" }] },
            { "type": "addRules", "behavior": "allow", "destination": "userSettings",
              "rules": [{ "toolName": "Read" }] },
        ])));
        let kinds: Vec<(&str, Option<&str>, bool)> = s
            .iter()
            .map(|x| (x.kind.as_str(), x.destination.as_deref(), x.standing))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("addRules", Some("session"), false),
                ("addRules", Some("localSettings"), true),
                ("addDirectories", Some("session"), true),
                ("setMode", Some("session"), false),
                ("addRules", Some("session"), true), // tool-wide Read
                ("addRules", Some("userSettings"), true),
            ]
        );
        assert!(s.iter().enumerate().all(|(i, x)| x.index == i));
    }

    #[test]
    fn respond_validation_is_strict_per_kind() {
        let store = PendingStore::new();
        let (id, _rx) = store.register(item("s1"));
        let mut d = Decision::allow();
        d.answers = Some(BTreeMap::from([("Q".to_string(), "A".to_string())]));
        assert!(matches!(
            store.resolve(&id, d, &local()),
            Err(RespondError::Invalid(_))
        ));
        let mut d = Decision::allow();
        d.mode = Some("acceptEdits".into());
        assert!(matches!(
            store.resolve(&id, d, &local()),
            Err(RespondError::Invalid(_))
        ));
        let mut d = Decision::allow();
        d.content = Some(json!({ "x": 1 }));
        assert!(matches!(
            store.resolve(&id, d, &local()),
            Err(RespondError::Invalid(_))
        ));
        // An empty answers map is harmless.
        let mut d = Decision::allow();
        d.answers = Some(BTreeMap::new());
        assert!(store.resolve(&id, d, &local()).is_ok());

        // Elicitation: accept with content / decline.
        let mut e = item("s1");
        e.kind = PendingKind::Elicitation;
        e.tool_name = None;
        let mut d = Decision::allow();
        d.content = Some(json!({ "name": "x" }));
        assert_eq!(
            permission_result(&e, &d),
            json!({ "action": "accept", "content": { "name": "x" } })
        );
        assert_eq!(
            permission_result(&e, &Decision::deny(None)),
            json!({ "action": "decline" })
        );
        let (eid, _erx) = store.register(e);
        assert!(store.resolve(&eid, d, &phone(false)).is_ok());
    }

    #[test]
    fn server_classification_raises_caller_flags() {
        let store = PendingStore::new();
        let mut it = item("s1");
        it.input = Some(json!({ "command": "echo x >> ~/.zshrc" }));
        it.dangerous = false; // the channel said harmless
        let (id, _rx) = store.register(it);
        let got = store.get(&id).unwrap();
        assert!(got.dangerous);
        assert!(got.dangerous_reason.unwrap().contains("shell startup"));
        assert!(matches!(
            store.resolve(&id, Decision::allow(), &phone(false)),
            Err(RespondError::Forbidden(_))
        ));
        // A caller flag still counts.
        let mut flagged = item("s1");
        flagged.dangerous = true;
        let (fid, _frx) = store.register(flagged);
        assert!(store.get(&fid).unwrap().dangerous);
    }

    #[tokio::test]
    async fn resolutions_are_remembered() {
        let store = PendingStore::new();
        let (a, _ra) = store.register(item("s1"));
        store.resolve(&a, Decision::allow(), &local()).unwrap();
        let r = store.resolution(&a).unwrap();
        assert_eq!(r.outcome, PendingOutcome::Allowed);
        assert_eq!(r.by.as_ref().unwrap().kind, ResolverKind::Desktop);
        assert_eq!(r.text.as_deref(), Some("Allowed by Mac"));

        let (b, _rb) = store.register(item("s1"));
        store
            .resolve(&b, Decision::deny(None), &phone(false))
            .unwrap();
        let r = store.resolution(&b).unwrap();
        assert_eq!(r.outcome, PendingOutcome::Denied);
        assert_eq!(r.by.unwrap().kind, ResolverKind::Device);

        let (c, _rc) = store.register(item("s1"));
        store.withdraw(&c, "answered in VS Code");
        let r = store.resolution(&c).unwrap();
        assert_eq!(r.outcome, PendingOutcome::Withdrawn);
        assert_eq!(r.by.unwrap().kind, ResolverKind::Host);

        let (d, _rd) = store.register(item("s1"));
        store.withdraw(&d, "timeout");
        assert_eq!(
            store.resolution(&d).unwrap().outcome,
            PendingOutcome::Expired
        );
        let (e, _re) = store.register(item("s1"));
        store.withdraw(&e, "control_cancel_request");
        assert_eq!(
            store.resolution(&e).unwrap().outcome,
            PendingOutcome::Cancelled
        );

        let mut q = item("s1");
        q.kind = PendingKind::Question;
        let (qid, _rq) = store.register(q);
        let mut dq = Decision::allow();
        dq.answers = Some(BTreeMap::from([("Q".to_string(), "A".to_string())]));
        store.resolve(&qid, dq, &local()).unwrap();
        assert_eq!(
            store.resolution(&qid).unwrap().outcome,
            PendingOutcome::Answered
        );
    }

    #[test]
    fn respond_body_shape() {
        let b: RespondBody = serde_json::from_value(json!({
            "requestId": "r1", "decision": "allow", "suggestion": 0,
            "answers": { "Q": "A" }, "mode": "plan", "interrupt": true
        }))
        .unwrap();
        assert_eq!(b.request_id, "r1");
        assert!(b.decision.is_allow());
        assert_eq!(b.decision.suggestion, Some(0));
        assert!(b.decision.interrupt);
        assert!(serde_json::from_value::<RespondBody>(json!({ "requestId": "r1" })).is_err());
        // Unknown fields are rejected.
        assert!(serde_json::from_value::<RespondBody>(
            json!({ "requestId": "r1", "decision": "allow", "always": true })
        )
        .is_err());
        let e: RespondBody = serde_json::from_value(
            json!({ "requestId": "r2", "decision": "allow", "content": { "a": 1 } }),
        )
        .unwrap();
        assert_eq!(e.decision.content, Some(json!({ "a": 1 })));
        // Serializing keeps the flat wire shape.
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["requestId"], "r2");
        assert_eq!(v["decision"], "allow");
    }

    #[test]
    fn questions_parse_both_shapes() {
        let qs = questions_from_input(&json!({ "questions": [
            { "question": "DB?", "header": "Storage", "multiSelect": true,
              "options": ["pg", { "label": "sqlite", "description": "file" }] }
        ]}));
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].options.len(), 2);
        assert!(qs[0].multi_select);
        assert_eq!(qs[0].options[1].description.as_deref(), Some("file"));
        let single = questions_from_input(&json!({ "question": "Name?" }));
        assert_eq!(single[0].question, "Name?");
        assert!(questions_from_input(&json!({})).is_empty());
    }
}
