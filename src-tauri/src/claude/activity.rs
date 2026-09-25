//! "What is each session doing right now" (design §3.5).
//!
//! Owner: **B2 (data layer)** — STUB. What B2 implements: the activity
//! priority (pending item ⇒ waiting; pidfile waitingFor; owned stream state;
//! latest hook event; pidfile busy + transcript tail; idle) and the tool
//! summaries (see `uievents::tool_summary`, which B2 may move here).
//!
//! Other agents feed it: B3 reports owned stream state, B4 reports hook events.
//! The feeding functions below are the agreed entry points; B2 fills them in.
//! AppState field: `state.activity`.

use super::model::Activity;

/// Live activity hints per session. STUB: remembers nothing.
#[derive(Debug, Default)]
pub struct ActivityTracker {}

impl ActivityTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// An owned session's stream state changed (B3 calls this). STUB.
    pub fn note_owned(&self, _session_id: &str, _activity: Activity) {}

    /// A hook event arrived for a session (B4 calls this with the event name,
    /// tool name and agent id). STUB.
    pub fn note_hook(
        &self,
        _session_id: &str,
        _event: &str,
        _tool_name: Option<&str>,
        _agent_id: Option<&str>,
    ) {
    }

    /// The best current activity hint for a session, if any. STUB.
    pub fn current(&self, _session_id: &str) -> Option<Activity> {
        None
    }
}
