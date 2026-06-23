//! Background monitor: keeps the session snapshot fresh and streams live
//! transcript deltas onto the broadcast bus.
//!
//! Refreshes on a fixed interval *and* on debounced filesystem changes, so live
//! edits surface in well under a second while a periodic sweep catches anything
//! the watcher misses.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::claude::registry::now_ms;
use crate::claude::{
    build_registry, query_agents, read_state_jsons, scan_transcripts, FsWatcher, RegistryInputs,
    TranscriptTailer,
};
use crate::state::{AppState, ServerEvent};

const REFRESH_INTERVAL: Duration = Duration::from_secs(3);

pub async fn run(state: AppState) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let paths = vec![
        state.home.projects_dir(),
        state.home.jobs_dir(),
        state.home.daemon_dir(),
    ];
    let watcher = match FsWatcher::spawn(&paths, tx) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!(error = %e, "filesystem watcher unavailable; polling only");
            None
        }
    };

    let mut tailers: HashMap<String, TranscriptTailer> = HashMap::new();
    refresh(&state, &mut tailers).await;

    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    if watcher.is_none() {
        // Poll-only fallback.
        loop {
            interval.tick().await;
            refresh(&state, &mut tailers).await;
        }
    }

    let _watcher = watcher; // keep alive for the lifetime of the loop
    loop {
        tokio::select! {
            _ = interval.tick() => refresh(&state, &mut tailers).await,
            batch = rx.recv() => match batch {
                Some(_paths) => refresh(&state, &mut tailers).await,
                None => break, // watcher gone
            },
        }
    }
}

/// Run a single refresh cycle with a throwaway tailer set. Exposed for
/// integration tests that drive the pipeline deterministically.
pub async fn refresh_once(state: &AppState) {
    let mut tailers = HashMap::new();
    refresh(state, &mut tailers).await;
}

/// One refresh cycle: rebuild the registry, broadcast it, then tail each live
/// session's transcript and broadcast any new lines.
async fn refresh(state: &AppState, tailers: &mut HashMap<String, TranscriptTailer>) {
    let agents = query_agents(true).await;

    let home = state.home.clone();
    let (transcripts, states) =
        tokio::task::spawn_blocking(move || (scan_transcripts(&home), read_state_jsons(&home)))
            .await
            .unwrap_or_default();

    let owned = state.owned.read().await.clone();
    let pending = state.pending.read().await.clone();
    let owned_live = state.control.live();

    let sessions = build_registry(RegistryInputs {
        agents: &agents,
        transcripts: &transcripts,
        owned: &owned,
        pending: &pending,
        states: &states,
        now_ms: now_ms(),
        foreign_injection: crate::claude::foreign_injection_enabled(),
        owned_live: &owned_live,
    });

    *state.sessions.write().await = sessions.clone();
    state.broadcast(ServerEvent::Sessions(sessions.clone()));

    for s in &sessions {
        if s.cwd.is_empty() {
            continue;
        }
        let path = state.home.transcript_path(&s.cwd, &s.id);
        let tailer = tailers
            .entry(s.id.clone())
            .or_insert_with(|| TranscriptTailer::at_end(path));
        match tailer.poll() {
            Ok(events) if !events.is_empty() => state.broadcast(ServerEvent::Transcript {
                session_id: s.id.clone(),
                events,
            }),
            Ok(_) => {}
            Err(e) => tracing::debug!(error = %e, id = %s.id, "transcript tail error"),
        }
    }

    // Drop tailers for sessions that no longer exist.
    let live: HashSet<&String> = sessions.iter().map(|s| &s.id).collect();
    tailers.retain(|id, _| live.contains(id));
}
