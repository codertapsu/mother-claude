//! The push decision engine — pure, clock-injected, unit-tested:
//!
//! * [`PushEvent`]: one thing worth telling a phone about.
//! * [`Suppressor`]: drops duplicates arriving through two paths (a direct
//!   trigger call AND the same fact observed on the event bus), and "waiting
//!   at the Mac" noise right after a real approval card.
//! * [`Coalescer`]: per-session throttle. The first event of a session goes out
//!   at once; anything else for that session within the window (10 s) is held
//!   and sent as ONE follow-up notification when the window closes.
//! * [`wants`]: whether a device's prefs / follow / mute / one-shot select an
//!   event.
//! * [`SessionTracker`]: turns `sessions` / `transcript` bus frames into
//!   derived events (a turn finished after ≥ 30 s, a session failed, a session
//!   started waiting at the Mac, Claude called PushNotification).
//!
//! Times are milliseconds on a caller-supplied clock (monotonic in the driver,
//! synthetic in tests).

use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;

use super::store::{DevicePush, SessionMode};
use crate::claude::model::{PendingItem, PendingKind, SessionStatus, SessionSummary, UiEvent};
use crate::state::ServerEvent;

/// A turn must have worked this long before "finished" is pushed (unless the
/// device armed a one-shot for the session).
pub const FINISHED_MIN_MS: u64 = 30_000;
/// Per-session coalescing window.
pub const WINDOW_MS: i64 = 10_000;
/// After an answerable card was pushed for a session, a "waiting at the Mac"
/// derived from the session status is redundant for this long.
const WAITING_AFTER_PENDING_MS: i64 = 60_000;
/// Two "finished" pushes for one session within this long are one event seen
/// through two paths.
const FINISHED_DEDUPE_MS: i64 = 30_000;
const FAILED_DEDUPE_MS: i64 = 60_000;
const IDLE_DEDUPE_MS: i64 = 60_000;
/// Bounded memory of keys already pushed (pending ids, tool-use ids).
const RECENT_KEYS: usize = 512;
/// Transcript events older than this are history, not news.
const TRANSCRIPT_FRESH_MS: i64 = 10 * 60 * 1000;

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PushKind {
    Permission,
    Question,
    Plan,
    /// The session waits at the Mac for something we cannot answer here.
    Waiting,
    Finished,
    Failed,
    /// Claude decided to notify (PushNotification tool, Notification hook).
    Claude,
    /// "Claude is waiting for your input" (idle_prompt).
    Idle,
    Test,
}

impl PushKind {
    pub fn is_attention(self) -> bool {
        matches!(
            self,
            PushKind::Permission | PushKind::Question | PushKind::Plan | PushKind::Waiting
        )
    }
}

/// One notifiable fact about a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushEvent {
    pub session_id: String,
    pub kind: PushKind,
    /// Dedupe key (pending id, tool-use id, or the kind for per-session facts).
    pub key: String,
    /// Tool name (allowed in the notification text).
    pub tool: Option<String>,
    pub pending_id: Option<String>,
    /// A short non-sensitive qualifier (pidfile `waitingFor`, error kind).
    pub label: Option<String>,
    /// Sensitive text (command, question, plan line, message) — only shown to
    /// devices that opted into details.
    pub detail: Option<String>,
    pub worked_ms: Option<u64>,
    pub dangerous: bool,
    /// Observed on the bus rather than reported directly.
    pub derived: bool,
}

impl PushEvent {
    pub fn new(session_id: &str, kind: PushKind, key: impl Into<String>) -> Self {
        Self {
            session_id: session_id.to_string(),
            kind,
            key: key.into(),
            tool: None,
            pending_id: None,
            label: None,
            detail: None,
            worked_ms: None,
            dangerous: false,
            derived: false,
        }
    }

    /// The event for a freshly raised pending item.
    pub fn from_pending(item: &PendingItem) -> Self {
        let kind = match item.kind {
            PendingKind::Permission => PushKind::Permission,
            PendingKind::Question | PendingKind::Elicitation => PushKind::Question,
            PendingKind::Plan => PushKind::Plan,
        };
        let detail = match item.kind {
            PendingKind::Question => item
                .questions
                .as_ref()
                .and_then(|q| q.first())
                .map(|q| q.question.clone())
                .or_else(|| item.detail.clone()),
            PendingKind::Plan => item.plan.as_deref().and_then(|p| {
                p.lines()
                    .map(|l| l.trim().trim_start_matches('#').trim())
                    .find(|l| !l.is_empty())
                    .map(str::to_string)
            }),
            _ => item.detail.clone().or_else(|| Some(item.title.clone())),
        };
        let tool = match item.kind {
            PendingKind::Permission | PendingKind::Elicitation => item.tool_name.clone(),
            _ => None,
        };
        Self {
            tool,
            pending_id: Some(item.id.clone()),
            detail: detail.filter(|d| !d.trim().is_empty()),
            dangerous: item.dangerous,
            ..Self::new(&item.session_id, kind, format!("pending:{}", item.id))
        }
    }
}

// ─────────────────────────────── suppression ─────────────────────────────────

#[derive(Debug, Default, Clone, Copy)]
struct Marks {
    pending: Option<i64>,
    finished: Option<i64>,
    failed: Option<i64>,
    idle: Option<i64>,
}

fn within(mark: Option<i64>, now: i64, ms: i64) -> bool {
    mark.is_some_and(|t| now - t < ms)
}

fn admit_once(mark: &mut Option<i64>, now: i64, ms: i64) -> bool {
    if within(*mark, now, ms) {
        return false;
    }
    *mark = Some(now);
    true
}

/// Cross-path dedupe. Stateful but pure (explicit clock).
#[derive(Debug, Default)]
pub struct Suppressor {
    marks: HashMap<String, Marks>,
    recent: VecDeque<String>,
    recent_set: HashSet<String>,
}

impl Suppressor {
    /// Whether `ev` is news. `has_answerable` = the session currently has an
    /// answerable card in the pending store (its own push covered it).
    pub fn admit(&mut self, now: i64, ev: &PushEvent, has_answerable: bool) -> bool {
        if matches!(
            ev.kind,
            PushKind::Permission | PushKind::Question | PushKind::Plan | PushKind::Claude
        ) {
            if self.recent_set.contains(&ev.key) {
                return false;
            }
            self.remember(ev.key.clone());
        }
        let m = self.marks.entry(ev.session_id.clone()).or_default();
        match ev.kind {
            PushKind::Permission | PushKind::Question | PushKind::Plan => {
                m.pending = Some(now);
                true
            }
            PushKind::Waiting => {
                !(has_answerable || within(m.pending, now, WAITING_AFTER_PENDING_MS))
            }
            // The mark moves only when an event is admitted, so a duplicate
            // never extends the quiet period into the next real one.
            PushKind::Finished => admit_once(&mut m.finished, now, FINISHED_DEDUPE_MS),
            PushKind::Failed => admit_once(&mut m.failed, now, FAILED_DEDUPE_MS),
            PushKind::Idle => admit_once(&mut m.idle, now, IDLE_DEDUPE_MS),
            PushKind::Claude | PushKind::Test => true,
        }
    }

    /// Forget a session (it disappeared from the snapshot).
    pub fn forget(&mut self, session_id: &str) {
        self.marks.remove(session_id);
    }

    fn remember(&mut self, key: String) {
        if self.recent_set.insert(key.clone()) {
            self.recent.push_back(key);
        }
        while self.recent.len() > RECENT_KEYS {
            if let Some(old) = self.recent.pop_front() {
                self.recent_set.remove(&old);
            }
        }
    }
}

// ─────────────────────────────── coalescing ──────────────────────────────────

/// Events for one session sent as one notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub session_id: String,
    pub events: Vec<PushEvent>,
}

#[derive(Debug)]
struct Window {
    opened: i64,
    sent: HashSet<String>,
    held: Vec<PushEvent>,
}

/// Leading-edge + trailing per-session throttle.
#[derive(Debug)]
pub struct Coalescer {
    window: i64,
    windows: HashMap<String, Window>,
}

impl Coalescer {
    pub fn new(window_ms: i64) -> Self {
        Self {
            window: window_ms.max(1),
            windows: HashMap::new(),
        }
    }

    /// Offer an event. `Some(batch)` = send now (first event of a window);
    /// `None` = held for the window's trailing notification (or a duplicate).
    pub fn offer(&mut self, now: i64, ev: PushEvent) -> Option<Batch> {
        let window = self.window;
        if let Some(w) = self.windows.get_mut(&ev.session_id) {
            if now - w.opened < window {
                if !w.sent.contains(&ev.key) && !w.held.iter().any(|h| h.key == ev.key) {
                    w.held.push(ev);
                }
                return None;
            }
        }
        // No open window (or an expired one whose trailing batch, if any, is
        // flushed by `due` first — callers call `due` before `offer`).
        let session_id = ev.session_id.clone();
        self.windows.insert(
            session_id.clone(),
            Window {
                opened: now,
                sent: HashSet::from([ev.key.clone()]),
                held: Vec::new(),
            },
        );
        Some(Batch {
            session_id,
            events: vec![ev],
        })
    }

    /// Close every window that has expired: its held events come back as one
    /// trailing batch (and open a fresh window, so a steady stream is still
    /// throttled to one notification per window); empty windows are dropped.
    pub fn due(&mut self, now: i64) -> Vec<Batch> {
        let window = self.window;
        let mut out = Vec::new();
        self.windows.retain(|session_id, w| {
            if now - w.opened < window {
                return true;
            }
            if w.held.is_empty() {
                return false;
            }
            let events = std::mem::take(&mut w.held);
            w.opened = now;
            w.sent = events.iter().map(|e| e.key.clone()).collect();
            out.push(Batch {
                session_id: session_id.clone(),
                events,
            });
            true
        });
        out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        out
    }

    /// When the earliest open window closes (the driver sleeps until then).
    pub fn next_deadline(&self) -> Option<i64> {
        self.windows.values().map(|w| w.opened + self.window).min()
    }
}

// ─────────────────────────────── selection ───────────────────────────────────

/// Whether a device's settings select `ev` (presence is checked separately at
/// send time). A one-shot "notify when this turn finishes" beats mute, the
/// per-event prefs and the 30 s threshold.
pub fn wants(dev: &DevicePush, ev: &PushEvent, now: i64) -> bool {
    if ev.kind == PushKind::Test {
        return true;
    }
    if ev.kind == PushKind::Finished && dev.turn_once_active(&ev.session_id, now) {
        return true;
    }
    let mode = dev.mode(&ev.session_id);
    if mode == Some(SessionMode::Mute) {
        return false;
    }
    let follow = mode == Some(SessionMode::Follow);
    let p = &dev.prefs;
    match ev.kind {
        PushKind::Permission => follow || p.permission,
        PushKind::Question => follow || p.question,
        PushKind::Plan => follow || p.plan,
        PushKind::Waiting => {
            let asks_input = ev.label.as_deref().is_some_and(|l| {
                let l = l.to_ascii_lowercase();
                l.contains("input") || l.contains("question")
            });
            follow || if asks_input { p.question } else { p.permission }
        }
        PushKind::Finished => {
            (follow || p.finished) && ev.worked_ms.unwrap_or(0) >= FINISHED_MIN_MS
        }
        PushKind::Failed => follow || p.failed,
        PushKind::Claude => follow || p.claude,
        PushKind::Idle => p.idle,
        PushKind::Test => true,
    }
}

// ─────────────────────────────── bus observation ─────────────────────────────

#[derive(Debug, Clone)]
struct Seen {
    status: SessionStatus,
    turn_started: Option<i64>,
    last_error_at: Option<i64>,
}

fn busy(s: SessionStatus) -> bool {
    matches!(
        s,
        SessionStatus::Working | SessionStatus::Waiting | SessionStatus::Starting
    )
}

/// Derives events from `sessions` diffs and `transcript` appends.
#[derive(Debug, Default)]
pub struct SessionTracker {
    seen: HashMap<String, Seen>,
}

impl SessionTracker {
    /// Observe one bus frame at wall-clock `now_ms` (epoch ms: transcript
    /// timestamps are compared against it). Returns derived events and the
    /// sessions that disappeared.
    pub fn observe(&mut self, ev: &ServerEvent, now_ms: i64) -> (Vec<PushEvent>, Vec<String>) {
        match ev {
            ServerEvent::Sessions {
                full,
                sessions,
                upsert,
                remove,
                ..
            } => self.observe_sessions(
                *full,
                sessions.as_deref(),
                upsert.as_deref(),
                remove.as_deref(),
                now_ms,
            ),
            ServerEvent::Transcript {
                session, events, ..
            } => (self.observe_transcript(session, events, now_ms), Vec::new()),
            _ => (Vec::new(), Vec::new()),
        }
    }

    /// A `sessions` frame: a full list (`full`) or `upsert` / `remove` diffs.
    pub fn observe_sessions(
        &mut self,
        full: bool,
        sessions: Option<&[SessionSummary]>,
        upsert: Option<&[SessionSummary]>,
        remove: Option<&[String]>,
        now_ms: i64,
    ) -> (Vec<PushEvent>, Vec<String>) {
        let mut out = Vec::new();
        let mut gone = Vec::new();
        if let Some(list) = sessions {
            if full {
                let ids: HashSet<&str> = list.iter().map(|s| s.id.as_str()).collect();
                let stale: Vec<String> = self
                    .seen
                    .keys()
                    .filter(|k| !ids.contains(k.as_str()))
                    .cloned()
                    .collect();
                for id in stale {
                    self.seen.remove(&id);
                    gone.push(id);
                }
            }
            for s in list {
                out.extend(self.observe_summary(s, now_ms));
            }
        }
        for s in upsert.unwrap_or_default() {
            out.extend(self.observe_summary(s, now_ms));
        }
        for id in remove.unwrap_or_default() {
            if self.seen.remove(id).is_some() {
                gone.push(id.clone());
            }
        }
        (out, gone)
    }

    /// Live transcript appends of one session.
    pub fn observe_transcript(
        &mut self,
        session: &str,
        events: &[UiEvent],
        now_ms: i64,
    ) -> Vec<PushEvent> {
        events
            .iter()
            .filter_map(|e| push_notification_tool(session, e, now_ms))
            .collect()
    }

    fn observe_summary(&mut self, s: &SessionSummary, now: i64) -> Vec<PushEvent> {
        let error_at = s.last_error.as_ref().map(|e| e.at);
        let Some(prev) = self.seen.get(&s.id).cloned() else {
            // First sight (startup, or a new session): record, never notify.
            self.seen.insert(
                s.id.clone(),
                Seen {
                    status: s.status,
                    turn_started: busy(s.status).then_some(now),
                    last_error_at: error_at,
                },
            );
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut turn_started = prev.turn_started;
        if s.status != prev.status {
            if busy(s.status) && turn_started.is_none() {
                turn_started = Some(now);
            }
            match s.status {
                SessionStatus::Waiting => {
                    let mut e = PushEvent::new(&s.id, PushKind::Waiting, "waiting");
                    e.label = s.waiting_for.clone();
                    e.derived = true;
                    out.push(e);
                }
                SessionStatus::Idle if busy(prev.status) => {
                    if let Some(t) = turn_started.take() {
                        let mut e = PushEvent::new(&s.id, PushKind::Finished, "finished");
                        e.worked_ms = Some((now - t).max(0) as u64);
                        e.detail = s.last_message.clone();
                        e.derived = true;
                        out.push(e);
                    }
                }
                SessionStatus::Failed => {
                    turn_started = None;
                    let mut e = PushEvent::new(&s.id, PushKind::Failed, "failed");
                    e.label = s.last_error.as_ref().map(|e| e.kind.clone());
                    e.detail = s.last_error.as_ref().map(|e| e.text.clone());
                    e.derived = true;
                    out.push(e);
                }
                SessionStatus::Idle | SessionStatus::Ended | SessionStatus::Stopped => {
                    turn_started = None
                }
                _ => {}
            }
        }
        // A new error that left the session stopped (not one being retried
        // mid-turn) is a failure too.
        let new_error = match (error_at, prev.last_error_at) {
            (Some(a), Some(b)) => a > b,
            (Some(_), None) => true,
            _ => false,
        };
        if new_error && !busy(s.status) && !out.iter().any(|e| e.kind == PushKind::Failed) {
            let mut e = PushEvent::new(&s.id, PushKind::Failed, "failed");
            e.label = s.last_error.as_ref().map(|e| e.kind.clone());
            e.detail = s.last_error.as_ref().map(|e| e.text.clone());
            e.derived = true;
            out.push(e);
        }
        self.seen.insert(
            s.id.clone(),
            Seen {
                status: s.status,
                turn_started,
                last_error_at: error_at.or(prev.last_error_at),
            },
        );
        out
    }
}

/// A fresh `PushNotification` tool call in a transcript append.
fn push_notification_tool(session: &str, e: &UiEvent, now_ms: i64) -> Option<PushEvent> {
    let tool = e.tool.as_ref()?;
    if tool.name != "PushNotification" {
        return None;
    }
    // Only live news: an event without a timestamp or older than 10 min is a
    // replay.
    let ts = e.ts?;
    if (now_ms - ts).abs() > TRANSCRIPT_FRESH_MS {
        return None;
    }
    let message = tool
        .input
        .as_ref()
        .and_then(|i| i.get("message"))
        .and_then(|m| m.as_str())
        .map(str::to_string);
    let key = if tool.id.is_empty() {
        format!("tool:{}", e.id)
    } else {
        format!("tool:{}", tool.id)
    };
    let mut ev = PushEvent::new(session, PushKind::Claude, key);
    ev.detail = message;
    ev.derived = true;
    Some(ev)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::model::{LastError, UiKind, UiRole, UiTool};
    use crate::server::push::store::Prefs;

    fn ev(session: &str, kind: PushKind, key: &str) -> PushEvent {
        PushEvent::new(session, kind, key)
    }

    #[test]
    fn coalescing_is_leading_edge_plus_one_trailing_batch() {
        let mut c = Coalescer::new(WINDOW_MS);
        // First event goes out at once.
        let b = c.offer(0, ev("s1", PushKind::Permission, "p1")).unwrap();
        assert_eq!(b.events.len(), 1);
        // Within the window: held, duplicates dropped, other sessions free.
        assert!(c.offer(1_000, ev("s1", PushKind::Question, "p2")).is_none());
        assert!(c.offer(2_000, ev("s1", PushKind::Question, "p2")).is_none());
        assert!(c
            .offer(2_500, ev("s1", PushKind::Permission, "p1"))
            .is_none());
        assert!(c.offer(3_000, ev("s1", PushKind::Plan, "p3")).is_none());
        assert!(c
            .offer(3_000, ev("s2", PushKind::Finished, "finished"))
            .is_some());
        assert_eq!(c.next_deadline(), Some(WINDOW_MS));
        assert!(c.due(WINDOW_MS - 1).is_empty());
        // Window closes: ONE trailing batch with the two held events.
        let due = c.due(WINDOW_MS);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].session_id, "s1");
        let keys: Vec<_> = due[0].events.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["p2", "p3"]);
        // A steady stream stays throttled: the trailing send opened a window.
        assert!(c
            .offer(WINDOW_MS + 100, ev("s1", PushKind::Question, "p4"))
            .is_none());
        let due = c.due(2 * WINDOW_MS);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].events[0].key, "p4");
        // Quiet windows are dropped; the next event leads again.
        assert!(c.due(4 * WINDOW_MS).is_empty());
        assert_eq!(c.next_deadline(), None);
        assert!(c
            .offer(5 * WINDOW_MS, ev("s1", PushKind::Question, "p5"))
            .is_some());
    }

    #[test]
    fn suppression_across_paths() {
        let mut s = Suppressor::default();
        let pending = ev("s1", PushKind::Permission, "pending:a");
        assert!(s.admit(0, &pending, true));
        assert!(!s.admit(1, &pending, true), "same pending id twice");
        // The status flip to waiting right after a card is redundant…
        let waiting = ev("s1", PushKind::Waiting, "waiting");
        assert!(!s.admit(2_000, &waiting, false));
        // …and so is any waiting while a card is answerable.
        assert!(!s.admit(120_000, &waiting, true));
        // An observed-only wait later is news.
        assert!(s.admit(120_000, &waiting, false));
        // finished from the owned loop and from the bus: one.
        let fin = ev("s1", PushKind::Finished, "finished");
        assert!(s.admit(200_000, &fin, false));
        assert!(!s.admit(203_000, &fin, false));
        assert!(!s.admit(200_000 + FINISHED_DEDUPE_MS - 1, &fin, false));
        assert!(s.admit(200_000 + FINISHED_DEDUPE_MS, &fin, false));
        let failed = ev("s2", PushKind::Failed, "failed");
        assert!(s.admit(0, &failed, false));
        assert!(!s.admit(59_000, &failed, false));
        let claude = ev("s2", PushKind::Claude, "tool:t1");
        assert!(s.admit(0, &claude, false));
        assert!(!s.admit(5, &claude, false));
        s.forget("s1");
        assert!(s.admit(203_500, &ev("s1", PushKind::Waiting, "waiting"), false));
    }

    #[test]
    fn device_selection() {
        let now = 1_000;
        let mut dev = DevicePush::default();
        let mut fin = ev("s1", PushKind::Finished, "finished");
        fin.worked_ms = Some(10_000);
        assert!(!wants(&dev, &fin, now), "short turn");
        fin.worked_ms = Some(FINISHED_MIN_MS);
        assert!(wants(&dev, &fin, now));
        assert!(
            !wants(&dev, &ev("s1", PushKind::Idle, "idle"), now),
            "idle off by default"
        );
        dev.prefs = Prefs {
            permission: false,
            ..Prefs::default()
        };
        assert!(!wants(&dev, &ev("s1", PushKind::Permission, "p"), now));
        let mut waiting = ev("s1", PushKind::Waiting, "waiting");
        waiting.label = Some("permission prompt".into());
        assert!(!wants(&dev, &waiting, now));
        waiting.label = Some("input needed".into());
        assert!(
            wants(&dev, &waiting, now),
            "input waits follow the question pref"
        );
        // Follow overrides per-event prefs, mute silences.
        dev.sessions.insert("s1".into(), "follow".into());
        assert!(wants(&dev, &ev("s1", PushKind::Permission, "p"), now));
        dev.sessions.insert("s1".into(), "mute".into());
        assert!(!wants(&dev, &ev("s1", PushKind::Question, "q"), now));
        assert!(!wants(&dev, &fin, now));
        // A one-shot beats mute and the threshold.
        fin.worked_ms = Some(1_000);
        dev.turn_finished.insert("s1".into(), now + 10);
        assert!(wants(&dev, &fin, now));
        assert!(!wants(&dev, &fin, now + 20), "expired");
        assert!(wants(&dev, &ev("s9", PushKind::Test, "t"), now));
    }

    fn summary(id: &str, status: SessionStatus) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            name: "proj".into(),
            status,
            ..Default::default()
        }
    }

    #[test]
    fn tracker_derives_turns_waits_and_failures() {
        let mut t = SessionTracker::default();
        // First sight never notifies.
        let (e, _) = t.observe_sessions(
            false,
            None,
            Some(&[summary("s1", SessionStatus::Idle)]),
            None,
            0,
        );
        assert!(e.is_empty());
        let (e, _) = t.observe_sessions(
            false,
            None,
            Some(&[summary("s1", SessionStatus::Working)]),
            None,
            1_000,
        );
        assert!(e.is_empty());
        let mut w = summary("s1", SessionStatus::Waiting);
        w.waiting_for = Some("permission prompt".into());
        let (e, _) = t.observe_sessions(false, None, Some(&[w]), None, 5_000);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].kind, PushKind::Waiting);
        assert_eq!(e[0].label.as_deref(), Some("permission prompt"));
        let (e, _) = t.observe_sessions(
            false,
            None,
            Some(&[summary("s1", SessionStatus::Working)]),
            None,
            6_000,
        );
        assert!(e.is_empty());
        let mut done = summary("s1", SessionStatus::Idle);
        done.last_message = Some("All tests pass".into());
        let (e, _) = t.observe_sessions(false, None, Some(&[done]), None, 41_000);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].kind, PushKind::Finished);
        assert_eq!(e[0].worked_ms, Some(40_000));
        assert_eq!(e[0].detail.as_deref(), Some("All tests pass"));
        assert!(e[0].derived);
        // Idle again (no change): nothing.
        let (e, _) = t.observe_sessions(
            false,
            None,
            Some(&[summary("s1", SessionStatus::Idle)]),
            None,
            42_000,
        );
        assert!(e.is_empty());
        // A failure.
        let (e, _) = t.observe_sessions(
            false,
            None,
            Some(&[summary("s1", SessionStatus::Working)]),
            None,
            50_000,
        );
        assert!(e.is_empty());
        let mut f = summary("s1", SessionStatus::Failed);
        f.last_error = Some(LastError {
            kind: "api_error".into(),
            text: "overloaded".into(),
            at: 55_000,
        });
        let (e, _) = t.observe_sessions(false, None, Some(&[f.clone()]), None, 55_000);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].kind, PushKind::Failed);
        assert_eq!(e[0].label.as_deref(), Some("api_error"));
        // An error retried mid-turn is not a failure; one that stops it is.
        let mut retry = summary("s1", SessionStatus::Working);
        retry.last_error = Some(LastError {
            kind: "api_error".into(),
            text: "529".into(),
            at: 60_000,
        });
        let (e, _) = t.observe_sessions(false, None, Some(&[retry]), None, 60_000);
        assert!(e.is_empty());
        let mut stopped = summary("s1", SessionStatus::Idle);
        stopped.last_error = Some(LastError {
            kind: "rate_limit".into(),
            text: "429".into(),
            at: 70_000,
        });
        let (e, _) = t.observe_sessions(false, None, Some(&[stopped]), None, 70_000);
        assert!(e.iter().any(|e| e.kind == PushKind::Failed));
        // Removal is reported.
        let (_, gone) = t.observe_sessions(false, None, None, Some(&["s1".to_string()]), 80_000);
        assert_eq!(gone, vec!["s1".to_string()]);

        // A full list replaces what we knew: missing sessions are gone, new
        // ones are first sightings.
        t.observe_sessions(
            false,
            None,
            Some(&[summary("a", SessionStatus::Working)]),
            None,
            0,
        );
        let (e, gone) = t.observe_sessions(
            true,
            Some(&[summary("b", SessionStatus::Waiting)]),
            None,
            None,
            1_000,
        );
        assert!(e.is_empty(), "first sight of b");
        assert_eq!(gone, vec!["a".to_string()]);
    }

    #[test]
    fn tracker_sees_push_notification_tool_calls() {
        let mut t = SessionTracker::default();
        let now = 1_700_000_000_000;
        let tool = |ts: Option<i64>, id: &str| UiEvent {
            id: format!("rec:{id}"),
            ts,
            role: UiRole::Assistant,
            kind: UiKind::Tool,
            tool: Some(UiTool {
                id: id.into(),
                name: "PushNotification".into(),
                summary: String::new(),
                input: Some(
                    serde_json::json!({"message": "Tests finished", "status": "proactive"}),
                ),
            }),
            ..Default::default()
        };
        let events = [
            tool(Some(now - 1_000), "t1"),
            tool(Some(now - 3_600_000), "t0"),
            tool(None, "t2"),
        ];
        let e = t.observe_transcript("s1", &events, now);
        assert_eq!(e.len(), 1, "only the fresh one");
        assert_eq!(e[0].kind, PushKind::Claude);
        assert_eq!(e[0].key, "tool:t1");
        assert_eq!(e[0].detail.as_deref(), Some("Tests finished"));
    }

    #[test]
    fn pending_events_carry_only_safe_fields_outside_detail() {
        let item = PendingItem {
            id: "abc".into(),
            session_id: "s1".into(),
            kind: PendingKind::Permission,
            tool_name: Some("Bash".into()),
            title: "Bash: rm -rf build".into(),
            detail: Some("rm -rf build".into()),
            ..Default::default()
        };
        let e = PushEvent::from_pending(&item);
        assert_eq!(e.kind, PushKind::Permission);
        assert_eq!(e.tool.as_deref(), Some("Bash"));
        assert_eq!(e.pending_id.as_deref(), Some("abc"));
        assert_eq!(e.detail.as_deref(), Some("rm -rf build"));
        let plan = PendingItem {
            kind: PendingKind::Plan,
            plan: Some("\n# Refactor the parser\n\n1. …".into()),
            ..item.clone()
        };
        let e = PushEvent::from_pending(&plan);
        assert_eq!(e.kind, PushKind::Plan);
        assert_eq!(e.tool, None);
        assert_eq!(e.detail.as_deref(), Some("Refactor the parser"));
    }
}
