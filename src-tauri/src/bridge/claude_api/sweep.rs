//! Keeping the Claude API's conversations in bounds: a conversation idle
//! for [`IDLE_CLOSE`] has its Claude Code process closed (its next request
//! resumes it from its transcript), and tool calls a client has not answered
//! for [`TOOL_WAIT`] are given up (the turn ends; results sent later are read
//! as a message).

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::state::AppState;

/// An idle conversation's process is closed after this.
pub const IDLE_CLOSE: Duration = Duration::from_secs(10 * 60);
/// Tool calls nobody answers are given up after this.
pub const TOOL_WAIT: Duration = Duration::from_secs(30 * 60);
/// How often it looks.
const EVERY: Duration = Duration::from_secs(30);

/// Starts the sweeper, once (the first Claude API request does).
pub fn ensure(state: &AppState) {
    if state.bridge.api.sweeping.swap(true, Ordering::SeqCst) {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(EVERY).await;
            sweep(&state, Instant::now()).await;
        }
    });
}

/// One look: what is idle past its time is closed.
pub async fn sweep(state: &AppState, now: Instant) {
    let store = &state.bridge.api;
    let mut ids = store.idle_open(now.checked_sub(IDLE_CLOSE).unwrap_or(now));
    ids.extend(store.stale_waiting(now.checked_sub(TOOL_WAIT).unwrap_or(now)));
    ids.sort();
    ids.dedup();
    for id in ids {
        tracing::debug!(conversation = %id, "closing an idle Claude API conversation");
        super::routes::close_conversation(state, &id).await;
    }
}
