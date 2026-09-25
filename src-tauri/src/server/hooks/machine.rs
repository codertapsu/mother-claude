//! The hook dispatcher's state machine: pure, synchronous, unit-tested.
//!
//! Per session it tracks the process the hooks come from, the turn state, the
//! recent PreToolUse calls (to give a PermissionRequest — which carries no
//! `tool_use_id` — its tool call), the relay queue, the single wake waiter and
//! the subagents. Across sessions it tracks the held PermissionRequests.
//!
//! The async glue in `server/hooks.rs` feeds events in and carries out the
//! [`Outcome`]s (withdrawals in the pending store, timeline entries, push).
//!
//! Rules (design §5.2 with the §10 review amendments):
//!
//! * **Turn state.** A main-thread UserPromptSubmit with a new `prompt_id`
//!   starts a turn; a main-thread Stop/StopFailure ends it; as a fallback, the
//!   pidfile turning `idle` ends it. A main-thread PreToolUse/PostToolUse is
//!   also evidence of a running turn. pidfile `busy` alone is not (background
//!   tasks keep it busy).
//! * **Relay queue, one atomic claim.** A main-thread PostToolUse or Stop with
//!   a non-empty queue claims every message into its `additionalContext` (the
//!   turn stays active). The wake waiter is released only when the session is
//!   idle, with every queued message in one text. So a message is handed over
//!   exactly once.
//! * **Correlation.** A PermissionRequest takes the oldest unmatched PreToolUse
//!   with the same (agent, tool, canonical input); failing that, the oldest
//!   unmatched one with the same (agent, tool) from the last 5 s.
//! * **Withdrawal of held prompts.** The matching PostToolUse /
//!   PostToolUseFailure / PermissionDenied; for main-thread items Stop,
//!   StopFailure or a new-prompt UserPromptSubmit; for an agent's items its
//!   SubagentStop; SessionEnd for all; and a pidfile transition
//!   waiting → busy/idle (confirmed 300 ms later) whose `statusUpdatedAt` is
//!   later than the item.
//! * **Interrupt.** A flag answered with `continue:false` by the next
//!   main-thread PreToolUse/PostToolUse; a held main-thread prompt is denied
//!   with `interrupt:true` at once. Cleared by Stop, StopFailure, a new prompt
//!   or the pidfile going idle, so it can never kill the next turn.
//! * **Relays and interrupts attach only to main-thread events** (an
//!   `agent_id` event's output goes to the subagent).

use std::collections::{HashMap, VecDeque};

use serde_json::Value;
use tokio::sync::oneshot;

use super::relay;
use crate::claude::model::{QueueStatus, QueueVia, QueuedMessage};

/// Recent PreToolUse records kept per session.
const PRE_TOOL_CAP: usize = 256;
/// How long a PreToolUse stays matchable.
const PRE_TOOL_TTL_MS: i64 = 10 * 60 * 1000;
/// The loose (agent, tool) correlation window.
pub const FALLBACK_WINDOW_MS: i64 = 5_000;
/// `wake` stays true this long after a waiter's poll ended (it re-polls at
/// once), so the capability does not flap every 50 s.
const WAKE_GRACE_MS: i64 = 5_000;
/// A pidfile transition must hold this long before it withdraws prompts.
const LEAVE_CONFIRM_MS: i64 = 300;
/// The pidfile must have been idle this long before it ends a turn.
const IDLE_CONFIRM_MS: i64 = 1_000;
/// Delivered/cancelled ids remembered (for `409` on a late cancel).
const HISTORY_CAP: usize = 64;
/// Queued messages per session.
pub const QUEUE_CAP: usize = 20;

/// What a parked `/hooks/wait` waiter is told.
#[derive(Debug, PartialEq, Eq)]
pub enum WaitSignal {
    /// Print this on stderr and exit 2 (wakes the session).
    Release(String),
    /// A newer waiter took over (410).
    Superseded,
    /// The session ended or foreign control was turned off (410).
    Gone,
}

#[derive(Debug)]
struct Waiter {
    id: u64,
    tx: oneshot::Sender<WaitSignal>,
}

#[derive(Debug, Clone)]
struct PreTool {
    tool_use_id: String,
    agent_id: Option<String>,
    tool_name: String,
    input_key: u64,
    at_ms: i64,
    matched: bool,
}

/// A tool call between its PreToolUse and PostToolUse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlight {
    pub tool_name: String,
    pub agent_id: Option<String>,
    pub since_ms: i64,
}

/// A subagent seen through SubagentStart/SubagentStop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subagent {
    pub agent_type: String,
    pub started_ms: i64,
    pub running: bool,
}

#[derive(Debug, Clone)]
struct Queued {
    msg: QueuedMessage,
}

/// One session's hook state.
#[derive(Debug, Default)]
pub struct SessionHooks {
    /// The claude pid the hooks come from (`X-MC-Pid`, the wake `pid`).
    pub pid: Option<u32>,
    /// Any hook event or wake poll arrived from the current process.
    pub seen: bool,
    /// SessionEnd arrived.
    pub ended: bool,
    pub last_event_ms: i64,
    /// Latest `permission_mode` a hook reported.
    pub permission_mode: Option<String>,
    pub turn_active: bool,
    pub turn_started_ms: i64,
    last_prompt_id: Option<String>,
    pub interrupt: bool,
    /// The per-session relay marker.
    pub marker: String,
    /// The marker explanation reached the session (SessionStart context or a
    /// first relay).
    pub marker_announced: bool,
    pre_tools: VecDeque<PreTool>,
    pub in_flight: HashMap<String, InFlight>,
    queue: Vec<Queued>,
    history: VecDeque<(String, QueueStatus)>,
    waiter: Option<Waiter>,
    last_waiter_end_ms: i64,
    pub subagents: HashMap<String, Subagent>,
    pid_status: Option<String>,
    left_waiting_at: Option<i64>,
    /// Relays whose delivery a later UserPromptSubmit confirmed.
    pub confirmed_relays: u64,
}

impl SessionHooks {
    fn new(marker: String) -> Self {
        Self {
            marker,
            ..Default::default()
        }
    }

    /// A new claude process now speaks for this session id (a resume, a new
    /// window): forget everything that belonged to the old process. The queue
    /// stays (it is addressed to the session, not the process).
    fn reset_process(&mut self) {
        if let Some(w) = self.waiter.take() {
            let _ = w.tx.send(WaitSignal::Gone);
        }
        self.turn_active = false;
        self.interrupt = false;
        self.marker_announced = false;
        self.last_prompt_id = None;
        self.pre_tools.clear();
        self.in_flight.clear();
        self.subagents.clear();
        self.pid_status = None;
        self.left_waiting_at = None;
        self.ended = false;
    }

    fn start_turn(&mut self, now: i64) {
        if !self.turn_active {
            self.turn_active = true;
            self.turn_started_ms = now;
        }
    }

    fn prune_pre(&mut self, now: i64) {
        while self
            .pre_tools
            .front()
            .is_some_and(|p| now - p.at_ms > PRE_TOOL_TTL_MS)
            || self.pre_tools.len() > PRE_TOOL_CAP
        {
            self.pre_tools.pop_front();
        }
    }

    fn remember(&mut self, id: &str, status: QueueStatus) {
        self.history.push_back((id.to_string(), status));
        while self.history.len() > HISTORY_CAP {
            self.history.pop_front();
        }
    }

    /// Hand every queued message over: marks them delivered and returns the
    /// relay text plus the messages.
    fn claim(&mut self) -> Option<(String, Vec<QueuedMessage>)> {
        if self.queue.is_empty() {
            return None;
        }
        let mut msgs: Vec<QueuedMessage> = self.queue.drain(..).map(|q| q.msg).collect();
        for m in &mut msgs {
            m.status = QueueStatus::Delivered;
            m.cancellable = false;
        }
        for m in &msgs {
            self.remember(&m.id, QueueStatus::Delivered);
        }
        let text = relay::build(&self.marker, &msgs, !self.marker_announced);
        self.marker_announced = true;
        Some((text, msgs))
    }

    /// Undo a claim whose delivery failed (the waiter vanished).
    fn unclaim(&mut self, msgs: Vec<QueuedMessage>, announced_before: bool) {
        let ids: Vec<String> = msgs.iter().map(|m| m.id.clone()).collect();
        self.history.retain(|(id, _)| !ids.contains(id));
        let mut back: Vec<Queued> = msgs
            .into_iter()
            .map(|mut m| {
                m.status = QueueStatus::Queued;
                m.cancellable = true;
                Queued { msg: m }
            })
            .collect();
        back.append(&mut self.queue);
        self.queue = back;
        self.marker_announced = announced_before;
    }

    /// Release the waiter when the session is idle with queued messages.
    fn try_release(&mut self, now: i64) -> Vec<QueuedMessage> {
        if self.turn_active || self.queue.is_empty() || self.waiter.is_none() {
            return Vec::new();
        }
        let announced_before = self.marker_announced;
        let Some((text, msgs)) = self.claim() else {
            return Vec::new();
        };
        let Some(w) = self.waiter.take() else {
            self.unclaim(msgs, announced_before);
            return Vec::new();
        };
        match w.tx.send(WaitSignal::Release(text)) {
            Ok(()) => {
                self.last_waiter_end_ms = now;
                // The wake starts a turn; nothing else may be handed over
                // until it ends.
                self.turn_active = true;
                self.turn_started_ms = now;
                msgs
            }
            Err(_) => {
                self.unclaim(msgs, announced_before);
                Vec::new()
            }
        }
    }

    /// A queued (undelivered) message by id.
    pub fn queued(&self) -> Vec<QueuedMessage> {
        self.queue.iter().map(|q| q.msg.clone()).collect()
    }

    /// Whether a wake listener is connected (or just re-polling).
    pub fn wake(&self, now: i64) -> bool {
        self.waiter.is_some()
            || (self.last_waiter_end_ms > 0 && now - self.last_waiter_end_ms < WAKE_GRACE_MS)
    }

    pub fn has_waiter(&self) -> bool {
        self.waiter.is_some()
    }
}

/// A held PermissionRequest.
#[derive(Debug)]
pub struct Held {
    pub session: String,
    pub agent_id: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_name: String,
    pub input_key: u64,
    pub created_ms: i64,
    /// Fired to answer the held request with deny + interrupt.
    pub interrupt: Option<oneshot::Sender<()>>,
}

/// One hook event, as the state machine sees it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Event<'a> {
    pub name: &'a str,
    pub session: &'a str,
    pub agent_id: Option<&'a str>,
    pub agent_type: Option<&'a str>,
    pub tool_use_id: Option<&'a str>,
    pub tool_name: Option<&'a str>,
    pub input_key: Option<u64>,
    pub prompt_id: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub permission_mode: Option<&'a str>,
    pub pid: Option<u32>,
}

/// What processing an event asks the glue to do.
#[derive(Debug)]
pub struct Outcome {
    /// The hook's stdout JSON.
    pub response: Value,
    /// Pending ids to withdraw, with the reason.
    pub withdraw: Vec<(String, &'static str)>,
    /// Relay messages handed over by this event (timeline).
    pub delivered: Vec<QueuedMessage>,
    /// A turn ended after this long (push "finished").
    pub turn_finished_ms: Option<u64>,
    /// The turn ended with an error (push "failed").
    pub turn_failed: bool,
    /// hooksActive / wake may have changed (refresh the caps).
    pub caps_changed: bool,
}

impl Default for Outcome {
    fn default() -> Self {
        Self {
            response: relay::empty(),
            withdraw: Vec::new(),
            delivered: Vec::new(),
            turn_finished_ms: None,
            turn_failed: false,
            caps_changed: false,
        }
    }
}

/// Result of arming a wake waiter.
#[derive(Debug)]
pub enum Arm {
    /// Deliver at once (the session is idle with queued messages).
    Immediate(String, Vec<QueuedMessage>),
    /// Park until released, superseded or timed out.
    Parked(u64, oneshot::Receiver<WaitSignal>),
}

/// Why a queued message could not be cancelled.
#[derive(Debug, PartialEq, Eq)]
pub enum CancelError {
    NotFound,
    AlreadyDelivered,
}

/// Every session's hook state plus the held prompts.
#[derive(Debug, Default)]
pub struct Hub {
    sessions: HashMap<String, SessionHooks>,
    held: HashMap<String, Held>,
    next_waiter: u64,
}

fn session_mut<'a>(
    sessions: &'a mut HashMap<String, SessionHooks>,
    id: &str,
) -> &'a mut SessionHooks {
    sessions
        .entry(id.to_string())
        .or_insert_with(|| SessionHooks::new(relay::new_marker()))
}

fn held_where(
    held: &HashMap<String, Held>,
    session: &str,
    f: impl Fn(&Held) -> bool,
) -> Vec<String> {
    let mut v: Vec<(&String, i64)> = held
        .iter()
        .filter(|(_, h)| h.session == session && f(h))
        .map(|(id, h)| (id, h.created_ms))
        .collect();
    v.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
    v.into_iter().map(|(id, _)| id.clone()).collect()
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn session(&self, id: &str) -> Option<&SessionHooks> {
        self.sessions.get(id)
    }

    /// Ids of every session with state.
    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// Sessions whose pidfile the watch loop should look at: something is
    /// held, a turn is running, or messages are waiting.
    pub fn watched(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .sessions
            .iter()
            .filter(|(id, s)| {
                s.turn_active
                    || !s.queue.is_empty()
                    || s.left_waiting_at.is_some()
                    || self.held.values().any(|h| &h.session == *id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        v.sort();
        v
    }

    fn caps_view(&self, id: &str, now: i64) -> (bool, bool, bool) {
        self.sessions
            .get(id)
            .map(|s| (s.seen && !s.ended, s.wake(now), s.pid.is_some()))
            .unwrap_or_default()
    }

    /// Note that `session` sent something (an event or a wake poll) from
    /// `pid`. Returns the session.
    fn touch(&mut self, session: &str, pid: Option<u32>, now: i64) -> &mut SessionHooks {
        let s = session_mut(&mut self.sessions, session);
        if let Some(pid) = pid {
            if s.pid.is_some_and(|old| old != pid) {
                s.reset_process();
            }
            s.pid = Some(pid);
        }
        s.seen = true;
        s.ended = false;
        s.last_event_ms = now;
        s
    }

    /// Record traffic without an event (legacy `HookHub::touch`).
    pub fn mark_seen(&mut self, session: &str, now: i64) {
        self.touch(session, None, now);
    }

    /// Process one event.
    pub fn on_event(&mut self, ev: &Event<'_>, now: i64) -> Outcome {
        let before = self.caps_view(ev.session, now);
        let mut out = Outcome::default();
        let main = ev.agent_id.is_none();
        self.touch(ev.session, ev.pid, now);
        let Hub { sessions, held, .. } = self;
        let s = session_mut(sessions, ev.session);
        if let Some(m) = ev.permission_mode.filter(|m| !m.is_empty()) {
            s.permission_mode = Some(m.to_string());
        }

        match ev.name {
            "PreToolUse" => {
                if let (Some(id), Some(name)) = (ev.tool_use_id, ev.tool_name) {
                    s.prune_pre(now);
                    s.pre_tools.push_back(PreTool {
                        tool_use_id: id.to_string(),
                        agent_id: ev.agent_id.map(str::to_string),
                        tool_name: name.to_string(),
                        input_key: ev.input_key.unwrap_or_default(),
                        at_ms: now,
                        matched: false,
                    });
                    s.in_flight.insert(
                        id.to_string(),
                        InFlight {
                            tool_name: name.to_string(),
                            agent_id: ev.agent_id.map(str::to_string),
                            since_ms: now,
                        },
                    );
                }
                if main {
                    s.start_turn(now);
                    if s.interrupt {
                        out.response = relay::interrupt_output();
                    }
                }
            }
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
                if let Some(id) = ev.tool_use_id {
                    s.in_flight.remove(id);
                    s.pre_tools.retain(|p| p.tool_use_id != id);
                    let mut ids =
                        held_where(held, ev.session, |h| h.tool_use_id.as_deref() == Some(id));
                    if ids.is_empty() {
                        // An uncorrelated prompt for the same call.
                        if let (Some(name), Some(key)) = (ev.tool_name, ev.input_key) {
                            ids = held_where(held, ev.session, |h| {
                                h.tool_use_id.is_none()
                                    && h.agent_id.as_deref() == ev.agent_id
                                    && h.tool_name == name
                                    && h.input_key == key
                            });
                            ids.truncate(1);
                        }
                    }
                    let reason = match ev.name {
                        "PostToolUse" => "the tool ran (answered elsewhere)",
                        "PostToolUseFailure" => "the tool failed (answered elsewhere)",
                        _ => "denied elsewhere",
                    };
                    out.withdraw.extend(ids.into_iter().map(|id| (id, reason)));
                }
                if main && ev.name == "PostToolUse" {
                    s.start_turn(now);
                    if s.interrupt {
                        out.response = relay::interrupt_output();
                    } else if let Some((text, msgs)) = s.claim() {
                        out.response = relay::additional_context("PostToolUse", &text);
                        out.delivered = msgs;
                    }
                }
            }
            "Stop" | "StopFailure" if main => {
                s.interrupt = false;
                out.withdraw.extend(
                    held_where(held, ev.session, |h| h.agent_id.is_none())
                        .into_iter()
                        .map(|id| (id, "the turn ended")),
                );
                if ev.name == "Stop" {
                    if let Some((text, msgs)) = s.claim() {
                        // The turn goes on with the relayed message.
                        out.response = relay::additional_context("Stop", &text);
                        out.delivered = msgs;
                        s.start_turn(now);
                        s.turn_started_ms = now;
                    } else {
                        if s.turn_active {
                            out.turn_finished_ms = Some((now - s.turn_started_ms).max(0) as u64);
                        }
                        s.turn_active = false;
                    }
                } else {
                    s.turn_active = false;
                    out.turn_failed = true;
                    out.delivered = s.try_release(now);
                }
            }
            "UserPromptSubmit" if main => {
                let new_prompt = match ev.prompt_id {
                    Some(p) => s.last_prompt_id.as_deref() != Some(p),
                    None => true,
                };
                if new_prompt {
                    out.withdraw.extend(
                        held_where(held, ev.session, |h| {
                            h.agent_id.is_none() && h.created_ms <= now
                        })
                        .into_iter()
                        .map(|id| (id, "a new prompt started")),
                    );
                    s.interrupt = false;
                    s.last_prompt_id = ev.prompt_id.map(str::to_string);
                    s.turn_active = true;
                    s.turn_started_ms = now;
                }
                if ev
                    .prompt
                    .is_some_and(|p| relay::contains_marker(p, &s.marker))
                {
                    s.confirmed_relays += 1;
                }
            }
            "SessionStart" => {
                out.response = relay::additional_context("SessionStart", &relay::notice(&s.marker));
                s.marker_announced = true;
                s.turn_active = false;
                s.interrupt = false;
            }
            "SessionEnd" => {
                s.ended = true;
                s.turn_active = false;
                s.interrupt = false;
                s.in_flight.clear();
                if let Some(w) = s.waiter.take() {
                    let _ = w.tx.send(WaitSignal::Gone);
                }
                out.withdraw.extend(
                    held_where(held, ev.session, |_| true)
                        .into_iter()
                        .map(|id| (id, "the session ended")),
                );
            }
            "SubagentStart" => {
                if let Some(a) = ev.agent_id {
                    s.subagents.insert(
                        a.to_string(),
                        Subagent {
                            agent_type: ev.agent_type.unwrap_or_default().to_string(),
                            started_ms: now,
                            running: true,
                        },
                    );
                }
            }
            "SubagentStop" => {
                if let Some(a) = ev.agent_id {
                    if let Some(sub) = s.subagents.get_mut(a) {
                        sub.running = false;
                    }
                    out.withdraw.extend(
                        held_where(held, ev.session, |h| h.agent_id.as_deref() == Some(a))
                            .into_iter()
                            .map(|id| (id, "the subagent finished")),
                    );
                }
            }
            _ => {}
        }
        out.caps_changed = before != self.caps_view(ev.session, now);
        out
    }

    /// Give a PermissionRequest its tool call (see the module docs).
    pub fn correlate(
        &mut self,
        session: &str,
        agent_id: Option<&str>,
        tool_name: &str,
        input_key: u64,
        now: i64,
    ) -> Option<String> {
        let s = self.sessions.get_mut(session)?;
        s.prune_pre(now);
        let exact = s.pre_tools.iter_mut().find(|p| {
            !p.matched
                && p.agent_id.as_deref() == agent_id
                && p.tool_name == tool_name
                && p.input_key == input_key
        });
        let hit = match exact {
            Some(p) => Some(p),
            None => s.pre_tools.iter_mut().find(|p| {
                !p.matched
                    && p.agent_id.as_deref() == agent_id
                    && p.tool_name == tool_name
                    && now - p.at_ms <= FALLBACK_WINDOW_MS
            }),
        }?;
        hit.matched = true;
        Some(hit.tool_use_id.clone())
    }

    /// Whether a main-thread prompt is (or may be about to be) on the
    /// session's screen: a held main-thread PermissionRequest, or — with
    /// `recent_tool_ms` — an unfinished main-thread tool call that started
    /// that recently (its permission dialog may be opening right now).
    pub fn main_prompt_open(&self, session: &str, now: i64, recent_tool_ms: Option<i64>) -> bool {
        if self
            .held
            .values()
            .any(|h| h.session == session && h.agent_id.is_none())
        {
            return true;
        }
        let Some(window) = recent_tool_ms else {
            return false;
        };
        self.sessions.get(session).is_some_and(|s| {
            s.in_flight
                .values()
                .any(|f| f.agent_id.is_none() && now - f.since_ms <= window)
        })
    }

    /// Whether an interrupt is pending for the session's main thread.
    pub fn interrupt_pending(&self, session: &str) -> bool {
        self.sessions.get(session).is_some_and(|s| s.interrupt)
    }

    /// Register a held PermissionRequest. If an interrupt for the session's
    /// main thread was requested meanwhile, the interrupt sender comes back so
    /// the caller answers at once.
    pub fn hold(&mut self, pending_id: String, mut h: Held) -> Option<oneshot::Sender<()>> {
        let fire = (h.agent_id.is_none() && self.interrupt_pending(&h.session))
            .then(|| h.interrupt.take())
            .flatten();
        self.held.insert(pending_id, h);
        fire
    }

    pub fn unhold(&mut self, pending_id: &str) {
        self.held.remove(pending_id);
    }

    pub fn held_count(&self, session: &str) -> usize {
        self.held.values().filter(|h| h.session == session).count()
    }

    /// Flag an interrupt; returns the senders of the held main-thread prompts
    /// to answer with deny + interrupt.
    pub fn interrupt(&mut self, session: &str, now: i64) -> Vec<oneshot::Sender<()>> {
        let s = session_mut(&mut self.sessions, session);
        s.interrupt = true;
        if s.last_event_ms == 0 {
            s.last_event_ms = now;
        }
        self.held
            .values_mut()
            .filter(|h| h.session == session && h.agent_id.is_none())
            .filter_map(|h| h.interrupt.take())
            .collect()
    }

    /// Queue a relay message; delivered at once when the session is idle and
    /// a waiter is parked. Returns the message (status `delivered` when it
    /// went out) and everything delivered with it.
    pub fn enqueue(
        &mut self,
        session: &str,
        text: String,
        now: i64,
    ) -> Result<(QueuedMessage, Vec<QueuedMessage>), QueuedMessage> {
        let s = session_mut(&mut self.sessions, session);
        let msg = QueuedMessage {
            id: uuid::Uuid::new_v4().to_string(),
            text,
            created_at: now,
            via: if s.turn_active {
                QueueVia::HookBusy
            } else {
                QueueVia::HookIdle
            },
            status: QueueStatus::Queued,
            // Hook-queued messages can be taken back until a hook claims them.
            cancellable: true,
        };
        if s.queue.len() >= QUEUE_CAP {
            return Err(msg);
        }
        s.queue.push(Queued { msg: msg.clone() });
        let delivered = s.try_release(now);
        let msg = delivered
            .iter()
            .find(|m| m.id == msg.id)
            .cloned()
            .unwrap_or(msg);
        Ok((msg, delivered))
    }

    /// Take a still-queued message back.
    pub fn cancel(&mut self, session: &str, id: &str) -> Result<QueuedMessage, CancelError> {
        let s = self
            .sessions
            .get_mut(session)
            .ok_or(CancelError::NotFound)?;
        if let Some(pos) = s.queue.iter().position(|q| q.msg.id == id) {
            let mut m = s.queue.remove(pos).msg;
            m.status = QueueStatus::Cancelled;
            m.cancellable = false;
            s.remember(id, QueueStatus::Cancelled);
            return Ok(m);
        }
        match s.history.iter().find(|(h, _)| h == id) {
            Some((_, QueueStatus::Delivered)) => Err(CancelError::AlreadyDelivered),
            _ => Err(CancelError::NotFound),
        }
    }

    /// Whether the message is still queued (for the no-waiter notice).
    pub fn still_queued(&self, session: &str, id: &str) -> bool {
        self.sessions
            .get(session)
            .is_some_and(|s| s.queue.iter().any(|q| q.msg.id == id))
    }

    /// A wake listener polled: supersede the previous one and either deliver
    /// at once or park.
    pub fn arm_waiter(&mut self, session: &str, pid: Option<u32>, now: i64) -> Arm {
        self.next_waiter += 1;
        let id = self.next_waiter;
        let s = self.touch(session, pid, now);
        if let Some(old) = s.waiter.take() {
            let _ = old.tx.send(WaitSignal::Superseded);
        }
        if !s.turn_active {
            if let Some((text, msgs)) = s.claim() {
                s.last_waiter_end_ms = now;
                s.turn_active = true;
                s.turn_started_ms = now;
                return Arm::Immediate(text, msgs);
            }
        }
        let (tx, rx) = oneshot::channel();
        s.waiter = Some(Waiter { id, tx });
        Arm::Parked(id, rx)
    }

    /// A parked waiter's request ended (timeout, disconnect, answered).
    pub fn disarm(&mut self, session: &str, waiter_id: u64, now: i64) {
        if let Some(s) = self.sessions.get_mut(session) {
            if s.waiter.as_ref().is_some_and(|w| w.id == waiter_id) {
                s.waiter = None;
            }
            s.last_waiter_end_ms = now;
        }
    }

    /// Tell every parked waiter to stop (foreign control turned off).
    pub fn release_all_waiters(&mut self) -> usize {
        let mut n = 0;
        for s in self.sessions.values_mut() {
            if let Some(w) = s.waiter.take() {
                let _ = w.tx.send(WaitSignal::Gone);
                n += 1;
            }
        }
        n
    }

    /// The pidfile of `session` says `status` since `updated_at`.
    pub fn observe_pidfile(
        &mut self,
        session: &str,
        status: &str,
        updated_at: i64,
        now: i64,
    ) -> Outcome {
        let before = self.caps_view(session, now);
        let mut out = Outcome::default();
        let Hub { sessions, held, .. } = self;
        let Some(s) = sessions.get_mut(session) else {
            return out;
        };
        let prev = s.pid_status.replace(status.to_string());
        if status == "waiting" {
            s.left_waiting_at = None;
        } else if prev.as_deref() == Some("waiting") && matches!(status, "busy" | "idle") {
            s.left_waiting_at = Some(updated_at);
        }
        if let Some(left) = s.left_waiting_at {
            if status != "waiting" && now - left >= LEAVE_CONFIRM_MS {
                s.left_waiting_at = None;
                out.withdraw.extend(
                    held_where(held, session, |h| h.created_ms < left)
                        .into_iter()
                        .map(|id| (id, "answered at the session")),
                );
            }
        }
        if status == "idle"
            && s.turn_active
            && updated_at > s.turn_started_ms
            && now - updated_at >= IDLE_CONFIRM_MS
        {
            // An Escape interrupt fires no Stop: the pidfile is the only sign.
            s.turn_active = false;
            s.interrupt = false;
            out.delivered = s.try_release(now);
        }
        out.caps_changed = before != self.caps_view(session, now);
        out
    }

    /// Forget sessions that ended an hour ago or went silent for a day with
    /// nothing queued or held.
    pub fn gc(&mut self, now: i64) -> usize {
        const HOUR: i64 = 60 * 60 * 1000;
        let held = &self.held;
        let before = self.sessions.len();
        self.sessions.retain(|id, s| {
            let idle_for = now - s.last_event_ms;
            let busy = !s.queue.is_empty()
                || s.waiter.is_some()
                || held.values().any(|h| &h.session == id);
            busy || !((s.ended && idle_for > HOUR) || idle_for > 24 * HOUR)
        });
        before - self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const S: &str = "0a1b2c3d-0000-4000-8000-000000000000";

    fn ev<'a>(name: &'a str) -> Event<'a> {
        Event {
            name,
            session: S,
            ..Default::default()
        }
    }

    fn held(hub: &mut Hub, id: &str, agent: Option<&str>, tool_use: Option<&str>, at: i64) {
        hub.hold(
            id.into(),
            Held {
                session: S.into(),
                agent_id: agent.map(str::to_string),
                tool_use_id: tool_use.map(str::to_string),
                tool_name: "Bash".into(),
                input_key: 7,
                created_ms: at,
                interrupt: None,
            },
        );
    }

    fn ids(out: &Outcome) -> Vec<&str> {
        out.withdraw.iter().map(|(i, _)| i.as_str()).collect()
    }

    /// Like the glue: a withdrawn prompt's handler ends and unholds it.
    fn taken(hub: &mut Hub, out: &Outcome) -> Vec<String> {
        let v: Vec<String> = out.withdraw.iter().map(|(i, _)| i.clone()).collect();
        for id in &v {
            hub.unhold(id);
        }
        v
    }

    fn pre(hub: &mut Hub, id: &str, agent: Option<&str>, key: u64, now: i64) {
        hub.on_event(
            &Event {
                tool_use_id: Some(id),
                tool_name: Some("Bash"),
                agent_id: agent,
                input_key: Some(key),
                ..ev("PreToolUse")
            },
            now,
        );
    }

    #[test]
    fn correlation_prefers_exact_input_then_the_recent_same_tool() {
        let mut hub = Hub::new();
        pre(&mut hub, "t1", None, 1, 100);
        pre(&mut hub, "t2", None, 2, 110);
        pre(&mut hub, "t3", Some("agent1"), 2, 120);
        // Exact key match, in the right agent scope.
        assert_eq!(
            hub.correlate(S, None, "Bash", 2, 130).as_deref(),
            Some("t2")
        );
        assert_eq!(
            hub.correlate(S, Some("agent1"), "Bash", 2, 130).as_deref(),
            Some("t3")
        );
        // Each PreToolUse matches once; the rewrite fallback takes the oldest
        // unmatched one of the same tool within 5 s.
        assert_eq!(
            hub.correlate(S, None, "Bash", 99, 140).as_deref(),
            Some("t1")
        );
        assert_eq!(hub.correlate(S, None, "Bash", 99, 150), None);
        // Identical parallel calls pair up in arrival order.
        pre(&mut hub, "d1", None, 5, 200);
        pre(&mut hub, "d2", None, 5, 201);
        assert_eq!(
            hub.correlate(S, None, "Bash", 5, 210).as_deref(),
            Some("d1")
        );
        assert_eq!(
            hub.correlate(S, None, "Bash", 5, 211).as_deref(),
            Some("d2")
        );
        // Outside the fallback window only an exact match counts.
        pre(&mut hub, "old", None, 8, 1_000);
        assert_eq!(
            hub.correlate(S, None, "Bash", 9, 1_000 + FALLBACK_WINDOW_MS + 1),
            None
        );
        assert_eq!(
            hub.correlate(S, None, "Bash", 8, 1_000 + FALLBACK_WINDOW_MS + 1)
                .as_deref(),
            Some("old")
        );
        // A tool that already ran cannot be matched any more.
        pre(&mut hub, "ran", None, 11, 2_000);
        hub.on_event(
            &Event {
                tool_use_id: Some("ran"),
                tool_name: Some("Bash"),
                input_key: Some(11),
                ..ev("PostToolUse")
            },
            2_010,
        );
        assert_eq!(hub.correlate(S, None, "Bash", 11, 2_020), None);
    }

    #[test]
    fn post_tool_use_withdraws_its_prompt() {
        let mut hub = Hub::new();
        hub.mark_seen(S, 1);
        held(&mut hub, "p1", None, Some("t1"), 10);
        held(&mut hub, "p2", None, Some("t2"), 11);
        let out = hub.on_event(
            &Event {
                tool_use_id: Some("t1"),
                tool_name: Some("Bash"),
                ..ev("PostToolUse")
            },
            20,
        );
        assert_eq!(ids(&out), vec!["p1"]);
        for name in ["PostToolUseFailure", "PermissionDenied"] {
            let out = hub.on_event(
                &Event {
                    tool_use_id: Some("t2"),
                    ..ev(name)
                },
                30,
            );
            assert_eq!(ids(&out), vec!["p2"], "{name}");
        }
        // An uncorrelated prompt is withdrawn by the same (agent, tool, input).
        held(&mut hub, "p3", None, None, 40);
        let out = hub.on_event(
            &Event {
                tool_use_id: Some("t9"),
                tool_name: Some("Bash"),
                input_key: Some(7),
                ..ev("PostToolUse")
            },
            50,
        );
        assert_eq!(ids(&out), vec!["p3"]);
    }

    #[test]
    fn stop_and_new_prompts_withdraw_main_thread_prompts_only() {
        let mut hub = Hub::new();
        hub.on_event(
            &Event {
                prompt_id: Some("pr1"),
                ..ev("UserPromptSubmit")
            },
            1,
        );
        held(&mut hub, "main", None, Some("t1"), 10);
        held(&mut hub, "sub", Some("agent1"), Some("t2"), 11);
        let out = hub.on_event(&ev("Stop"), 20);
        assert_eq!(taken(&mut hub, &out), vec!["main"]);
        // Same prompt id (a relayed message merged into the turn) → no
        // withdrawal; a new prompt id → main-thread items go.
        held(&mut hub, "main2", None, None, 30);
        let out = hub.on_event(
            &Event {
                prompt_id: Some("pr1"),
                ..ev("UserPromptSubmit")
            },
            31,
        );
        assert!(ids(&out).is_empty());
        let out = hub.on_event(
            &Event {
                prompt_id: Some("pr2"),
                ..ev("UserPromptSubmit")
            },
            32,
        );
        assert_eq!(taken(&mut hub, &out), vec!["main2"]);
        // The subagent's prompt survived all of that and goes with its
        // SubagentStop.
        let out = hub.on_event(
            &Event {
                agent_id: Some("agent1"),
                ..ev("SubagentStop")
            },
            40,
        );
        assert_eq!(taken(&mut hub, &out), vec!["sub"]);
        // SessionEnd takes everything.
        held(&mut hub, "a", None, None, 50);
        held(&mut hub, "b", Some("agent2"), None, 51);
        let out = hub.on_event(&ev("SessionEnd"), 60);
        assert_eq!(taken(&mut hub, &out), vec!["a", "b"]);
        assert!(hub.session(S).unwrap().ended);
        assert_eq!(hub.held_count(S), 0);
    }

    #[test]
    fn pidfile_withdrawal_is_edge_triggered_and_confirmed() {
        let mut hub = Hub::new();
        hub.mark_seen(S, 1);
        // The hook arrives before the pidfile turns waiting: a busy status
        // older than the item must not withdraw it.
        hub.observe_pidfile(S, "busy", 5, 5);
        held(&mut hub, "p1", None, None, 10);
        assert!(ids(&hub.observe_pidfile(S, "busy", 5, 400)).is_empty());
        hub.observe_pidfile(S, "waiting", 12, 412);
        // Answered at the terminal: waiting → busy, confirmed 300 ms later.
        assert!(ids(&hub.observe_pidfile(S, "busy", 500, 500)).is_empty());
        let out = hub.observe_pidfile(S, "busy", 500, 850);
        assert_eq!(taken(&mut hub, &out), vec!["p1"]);
        // A flicker back to waiting cancels the pending withdrawal.
        held(&mut hub, "p2", None, None, 900);
        hub.observe_pidfile(S, "waiting", 901, 901);
        hub.observe_pidfile(S, "busy", 950, 950);
        assert!(ids(&hub.observe_pidfile(S, "waiting", 960, 960)).is_empty());
        assert!(ids(&hub.observe_pidfile(S, "waiting", 960, 1_500)).is_empty());
        // An item created after the transition stays.
        hub.observe_pidfile(S, "idle", 2_000, 2_000);
        held(&mut hub, "p3", None, None, 2_100);
        assert_eq!(
            ids(&hub.observe_pidfile(S, "idle", 2_000, 2_400)),
            vec!["p2"]
        );
    }

    #[test]
    fn relay_goes_out_exactly_once() {
        let mut hub = Hub::new();
        hub.on_event(
            &Event {
                prompt_id: Some("p1"),
                ..ev("UserPromptSubmit")
            },
            1,
        );
        let (m, delivered) = hub.enqueue(S, "run the tests".into(), 2).unwrap();
        assert_eq!(m.status, QueueStatus::Queued);
        assert_eq!(m.via, QueueVia::HookBusy);
        assert!(delivered.is_empty());
        // A subagent's PostToolUse must not carry it.
        let out = hub.on_event(
            &Event {
                agent_id: Some("a1"),
                tool_use_id: Some("x"),
                ..ev("PostToolUse")
            },
            3,
        );
        assert_eq!(out.response, json!({}));
        // The next main-thread PostToolUse claims it.
        let out = hub.on_event(
            &Event {
                tool_use_id: Some("t1"),
                ..ev("PostToolUse")
            },
            4,
        );
        let ctx = out.response["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert_eq!(
            out.response["hookSpecificOutput"]["hookEventName"],
            "PostToolUse"
        );
        assert!(ctx.contains("run the tests"));
        let marker = hub.session(S).unwrap().marker.clone();
        assert!(ctx.contains(&format!("[[MC-RELAY {marker}]]")));
        // Never announced by a SessionStart → the first relay explains it.
        assert!(ctx.starts_with("Messages relayed from the user's phone"));
        assert_eq!(out.delivered.len(), 1);
        assert_eq!(out.delivered[0].status, QueueStatus::Delivered);
        // Nothing left for Stop; the turn ends and reports its duration.
        let out = hub.on_event(&ev("Stop"), 10);
        assert_eq!(out.response, json!({}));
        assert_eq!(out.turn_finished_ms, Some(9));
        assert!(!hub.session(S).unwrap().turn_active);
        assert_eq!(
            hub.cancel(S, &m.id).unwrap_err(),
            CancelError::AlreadyDelivered
        );
    }

    #[test]
    fn stop_claims_the_queue_and_keeps_the_turn_going() {
        let mut hub = Hub::new();
        hub.on_event(&ev("SessionStart"), 0);
        hub.on_event(
            &Event {
                prompt_id: Some("p1"),
                ..ev("UserPromptSubmit")
            },
            1,
        );
        hub.enqueue(S, "one".into(), 2).unwrap();
        hub.enqueue(S, "two".into(), 3).unwrap();
        let out = hub.on_event(&ev("Stop"), 5);
        let ctx = out.response["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert_eq!(out.response["hookSpecificOutput"]["hookEventName"], "Stop");
        assert!(ctx.contains("one") && ctx.contains("two"));
        assert!(
            !ctx.starts_with("Messages relayed"),
            "SessionStart already announced the marker"
        );
        assert_eq!(out.delivered.len(), 2);
        assert!(out.turn_finished_ms.is_none());
        assert!(hub.session(S).unwrap().turn_active);
    }

    #[test]
    fn the_waiter_is_released_only_when_idle_and_newest_wins() {
        let mut hub = Hub::new();
        hub.on_event(
            &Event {
                prompt_id: Some("p1"),
                ..ev("UserPromptSubmit")
            },
            1,
        );
        // Stop and the wake listener arrive together; the listener parks.
        let Arm::Parked(w1, mut rx1) = hub.arm_waiter(S, Some(42), 2) else {
            panic!("should park");
        };
        hub.on_event(&ev("Stop"), 3);
        assert!(hub.session(S).unwrap().wake(3));
        // A newer listener supersedes the old one.
        let Arm::Parked(_w2, mut rx2) = hub.arm_waiter(S, Some(42), 4) else {
            panic!("should park");
        };
        assert_eq!(rx1.try_recv().unwrap(), WaitSignal::Superseded);
        hub.disarm(S, w1, 5); // the old one's cleanup must not touch the new
        assert!(hub.session(S).unwrap().has_waiter());
        // Idle + queued → released with the relay; the message is delivered.
        let (m, delivered) = hub.enqueue(S, "hello".into(), 6).unwrap();
        assert_eq!(m.status, QueueStatus::Delivered);
        assert_eq!(m.via, QueueVia::HookIdle);
        assert_eq!(delivered.len(), 1);
        match rx2.try_recv().unwrap() {
            WaitSignal::Release(t) => assert!(t.contains("hello")),
            other => panic!("unexpected {other:?}"),
        }
        // The wake is a turn: a second message waits for PostToolUse/Stop.
        let (m2, delivered) = hub.enqueue(S, "again".into(), 7).unwrap();
        assert_eq!(m2.status, QueueStatus::Queued);
        assert!(delivered.is_empty());
        let out = hub.on_event(&ev("Stop"), 9);
        assert!(out.response["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("again"));
    }

    #[test]
    fn a_waiter_arriving_to_a_queue_takes_it_at_once() {
        let mut hub = Hub::new();
        hub.mark_seen(S, 1);
        let (m, _) = hub
            .enqueue(S, "queued while nobody listened".into(), 2)
            .unwrap();
        assert_eq!(m.status, QueueStatus::Queued);
        match hub.arm_waiter(S, Some(42), 3) {
            Arm::Immediate(text, msgs) => {
                assert!(text.contains("queued while nobody listened"));
                assert_eq!(msgs[0].id, m.id);
            }
            Arm::Parked(..) => panic!("should deliver at once"),
        }
        assert!(hub.session(S).unwrap().turn_active);
    }

    #[test]
    fn a_vanished_waiter_leaves_the_message_queued() {
        let mut hub = Hub::new();
        hub.mark_seen(S, 1);
        let Arm::Parked(_, rx) = hub.arm_waiter(S, Some(42), 2) else {
            panic!("should park");
        };
        drop(rx); // client went away before cleanup ran
        let (m, delivered) = hub.enqueue(S, "keep me".into(), 3).unwrap();
        assert_eq!(m.status, QueueStatus::Queued);
        assert!(delivered.is_empty());
        assert_eq!(hub.session(S).unwrap().queued().len(), 1);
        assert!(!hub.session(S).unwrap().marker_announced);
        assert!(hub.still_queued(S, &m.id));
        assert_eq!(hub.cancel(S, &m.id).unwrap().status, QueueStatus::Cancelled);
        assert_eq!(hub.cancel(S, &m.id).unwrap_err(), CancelError::NotFound);
    }

    #[test]
    fn interrupts_apply_once_and_only_to_the_main_thread() {
        let mut hub = Hub::new();
        hub.on_event(
            &Event {
                prompt_id: Some("p1"),
                ..ev("UserPromptSubmit")
            },
            1,
        );
        let (tx_main, mut rx_main) = oneshot::channel();
        let (tx_sub, mut rx_sub) = oneshot::channel();
        hub.hold(
            "main".into(),
            Held {
                session: S.into(),
                agent_id: None,
                tool_use_id: None,
                tool_name: "Bash".into(),
                input_key: 0,
                created_ms: 2,
                interrupt: Some(tx_main),
            },
        );
        hub.hold(
            "sub".into(),
            Held {
                session: S.into(),
                agent_id: Some("a1".into()),
                tool_use_id: None,
                tool_name: "Bash".into(),
                input_key: 0,
                created_ms: 2,
                interrupt: Some(tx_sub),
            },
        );
        for tx in hub.interrupt(S, 3) {
            let _ = tx.send(());
        }
        assert!(rx_main.try_recv().is_ok());
        assert!(rx_sub.try_recv().is_err());
        // Subagent tool events are not stopped; main-thread ones are.
        let sub = hub.on_event(
            &Event {
                agent_id: Some("a1"),
                tool_use_id: Some("s"),
                ..ev("PreToolUse")
            },
            4,
        );
        assert_eq!(sub.response, json!({}));
        let pre = hub.on_event(
            &Event {
                tool_use_id: Some("m"),
                tool_name: Some("Bash"),
                ..ev("PreToolUse")
            },
            5,
        );
        assert_eq!(pre.response, relay::interrupt_output());
        let post = hub.on_event(
            &Event {
                tool_use_id: Some("m"),
                ..ev("PostToolUse")
            },
            6,
        );
        assert_eq!(post.response, relay::interrupt_output());
        // A prompt held after the flag was set is answered at once.
        let (tx, _rx) = oneshot::channel();
        let fire = hub.hold(
            "late".into(),
            Held {
                session: S.into(),
                agent_id: None,
                tool_use_id: None,
                tool_name: "Bash".into(),
                input_key: 0,
                created_ms: 7,
                interrupt: Some(tx),
            },
        );
        assert!(fire.is_some());
        // Stop clears it: the next turn is not killed.
        hub.on_event(&ev("Stop"), 8);
        assert!(!hub.interrupt_pending(S));
        let pre = hub.on_event(
            &Event {
                tool_use_id: Some("n"),
                ..ev("PreToolUse")
            },
            9,
        );
        assert_eq!(pre.response, json!({}));
        // A new prompt clears it too.
        hub.interrupt(S, 10);
        hub.on_event(
            &Event {
                prompt_id: Some("p2"),
                ..ev("UserPromptSubmit")
            },
            11,
        );
        assert!(!hub.interrupt_pending(S));
    }

    #[test]
    fn the_pidfile_going_idle_ends_a_turn_without_a_stop() {
        let mut hub = Hub::new();
        hub.on_event(
            &Event {
                prompt_id: Some("p1"),
                ..ev("UserPromptSubmit")
            },
            100,
        );
        hub.interrupt(S, 101);
        let Arm::Parked(_, mut rx) = hub.arm_waiter(S, Some(42), 102) else {
            panic!("should park");
        };
        hub.enqueue(S, "after the escape".into(), 103).unwrap();
        // Idle, but not for long enough yet.
        hub.observe_pidfile(S, "busy", 100, 104);
        let out = hub.observe_pidfile(S, "idle", 200, 500);
        assert!(out.delivered.is_empty());
        let out = hub.observe_pidfile(S, "idle", 200, 1_300);
        assert_eq!(out.delivered.len(), 1);
        assert!(matches!(rx.try_recv(), Ok(WaitSignal::Release(_))));
        assert!(!hub.interrupt_pending(S));
    }

    #[test]
    fn a_new_process_resets_the_session() {
        let mut hub = Hub::new();
        let Arm::Parked(_, mut rx) = hub.arm_waiter(S, Some(1000), 1) else {
            panic!("should park");
        };
        hub.on_event(&ev("SessionStart"), 2);
        assert!(hub.session(S).unwrap().marker_announced);
        hub.on_event(
            &Event {
                pid: Some(2000),
                ..ev("PreToolUse")
            },
            3,
        );
        assert_eq!(rx.try_recv().unwrap(), WaitSignal::Gone);
        let s = hub.session(S).unwrap();
        assert_eq!(s.pid, Some(2000));
        assert!(!s.marker_announced);
    }

    #[test]
    fn session_start_announces_the_marker() {
        let mut hub = Hub::new();
        let out = hub.on_event(&ev("SessionStart"), 1);
        let marker = hub.session(S).unwrap().marker.clone();
        assert_eq!(
            out.response,
            json!({ "hookSpecificOutput": { "hookEventName": "SessionStart",
                "additionalContext": relay::notice(&marker) } })
        );
        assert!(out.caps_changed, "the first event makes hooks active");
        // UserPromptSubmit carrying the marker confirms a delivery.
        hub.on_event(
            &Event {
                prompt: Some(&format!("<x>[[MC-RELAY {marker}]] hi</x>")),
                prompt_id: Some("w"),
                ..ev("UserPromptSubmit")
            },
            2,
        );
        assert_eq!(hub.session(S).unwrap().confirmed_relays, 1);
    }

    #[test]
    fn the_queue_is_bounded_and_gc_keeps_busy_sessions() {
        let mut hub = Hub::new();
        for i in 0..QUEUE_CAP {
            hub.enqueue(S, format!("m{i}"), 1).unwrap();
        }
        assert!(hub.enqueue(S, "one too many".into(), 1).is_err());
        assert_eq!(
            hub.gc(100 * 24 * 60 * 60 * 1000),
            0,
            "queued messages keep it"
        );
        let other = "1a1b2c3d-0000-4000-8000-000000000000";
        hub.on_event(
            &Event {
                session: other,
                ..ev("SessionEnd")
            },
            1,
        );
        assert_eq!(hub.gc(2 * 60 * 60 * 1000), 1);
        assert!(hub.session(other).is_none());
    }

    #[test]
    fn open_main_thread_prompts_are_reported() {
        let mut hub = Hub::new();
        assert!(!hub.main_prompt_open(S, 0, Some(5_000)));
        pre(&mut hub, "t1", None, 1, 100);
        pre(&mut hub, "s1", Some("agent"), 1, 100);
        // A main-thread tool that just started may be opening a dialog.
        assert!(hub.main_prompt_open(S, 1_000, Some(5_000)));
        assert!(!hub.main_prompt_open(S, 1_000, None));
        assert!(
            !hub.main_prompt_open(S, 10_000, Some(5_000)),
            "long-running"
        );
        held(&mut hub, "sub", Some("agent"), None, 200);
        assert!(
            !hub.main_prompt_open(S, 10_000, None),
            "a subagent's prompt"
        );
        held(&mut hub, "main", None, None, 300);
        assert!(hub.main_prompt_open(S, 10_000, None));
    }

    #[test]
    fn subagents_are_tracked() {
        let mut hub = Hub::new();
        hub.on_event(
            &Event {
                agent_id: Some("a1"),
                agent_type: Some("Explore"),
                ..ev("SubagentStart")
            },
            1,
        );
        let s = hub.session(S).unwrap();
        assert_eq!(s.subagents["a1"].agent_type, "Explore");
        assert!(s.subagents["a1"].running);
        hub.on_event(
            &Event {
                agent_id: Some("a1"),
                ..ev("SubagentStop")
            },
            2,
        );
        assert!(!hub.session(S).unwrap().subagents["a1"].running);
    }
}
