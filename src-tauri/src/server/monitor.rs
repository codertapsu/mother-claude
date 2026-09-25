//! Background monitor: keeps the live roster, the transcript index, the agent
//! view and the session snapshot fresh, and feeds the WebSocket hub.
//!
//! * **Inputs**: a debounced filesystem watcher over `projects/`,
//!   `sessions/` and `jobs/` (batches are coalesced; only the files named in a
//!   batch are touched), a 2 s tick (pidfiles + the recently written files of
//!   live sessions), a 60 s sweep (discovery of new files, every file of live
//!   sessions) and the index worker's "file indexed" notifications.
//! * **Index worker**: one background task runs the full-pass / history jobs
//!   of the index (concurrency 1), most recent files first.
//! * **Outputs**: `ws::publish_sessions` (a content-hash diff: only changed
//!   sessions are sent), `transcript{session, agent?}` appends for the
//!   subscriptions the hub reports (bounded; `transcriptReset` when a burst is
//!   too large), `agents{…}` for subscribers of sessions whose agents changed,
//!   and `fleet{…}` when the fleet changed and someone watches it.
//! * No `claude agents --json` spawn: the roster comes from pidfiles.
//! * `index-cache.json` is written every 30 s when dirty and when the history
//!   bootstrap finishes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::claude::agents::{fleet_entry, AgentCtx};
use crate::claude::index::{classify_path, Advance};
use crate::claude::model::{FleetSummary, SessionStatus, SessionSummary};
use crate::claude::summaries::{self, BuildCtx};
use crate::claude::FsWatcher;
use crate::server::ws;
use crate::state::AppState;

const TICK: Duration = Duration::from_secs(2);
const SWEEP: Duration = Duration::from_secs(60);
const AGENTS_LIVE_EVERY: Duration = Duration::from_secs(10);
/// Watcher-triggered refreshes are at least this far apart.
const MIN_GAP: Duration = Duration::from_millis(250);
/// Subagent files written this recently are stat'ed on every tick.
const RECENT_MS: i64 = 15 * 60 * 1000;

/// What one refresh cycle should do.
#[derive(Debug, Default)]
struct Trigger {
    paths: Vec<PathBuf>,
    tick: bool,
    sweep: bool,
    indexed: HashSet<String>,
}

/// State kept between cycles.
#[derive(Debug, Default)]
struct Cycle {
    started: bool,
    last_agents_live: Option<Instant>,
    previous_status: HashMap<String, SessionStatus>,
    live_prev: HashSet<String>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The session a path under `projects/` belongs to.
fn session_of_path(projects: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(projects).ok()?;
    let mut it = rel.iter().filter_map(|c| c.to_str());
    let _enc = it.next()?;
    let second = it.next()?;
    let sid = second.strip_suffix(".jsonl").unwrap_or(second);
    crate::claude::ids::is_session_id(sid).then(|| sid.to_string())
}

pub async fn run(state: AppState) {
    let (idx_tx, mut idx_rx) = mpsc::unbounded_channel::<String>();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let base = state.home.base().to_path_buf();
    let paths = vec![
        state.home.projects_dir(),
        base.join("sessions"),
        state.home.jobs_dir(),
    ];
    let watcher = match FsWatcher::spawn(&paths, tx) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!(error = %e, "filesystem watcher unavailable; polling only");
            None
        }
    };
    let _watcher = watcher;

    let mut cycle = Cycle::default();
    run_cycle(
        &state,
        &mut cycle,
        Trigger {
            sweep: true,
            tick: true,
            ..Default::default()
        },
    )
    .await;
    tokio::spawn(index_worker(state.clone(), idx_tx));

    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut sweep = tokio::time::interval(SWEEP);
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    sweep.tick().await;
    let mut last_run = Instant::now();
    let mut worker_alive = true;
    loop {
        let mut trigger = Trigger::default();
        tokio::select! {
            _ = tick.tick() => trigger.tick = true,
            _ = sweep.tick() => trigger.sweep = true,
            batch = rx.recv() => match batch {
                Some(paths) => trigger.paths = paths,
                None => {
                    // Watcher gone: keep polling.
                    tick.tick().await;
                    trigger.tick = true;
                }
            },
            sid = idx_rx.recv(), if worker_alive => match sid {
                Some(sid) => {
                    trigger.indexed.insert(sid);
                }
                None => {
                    tracing::warn!("index worker stopped");
                    worker_alive = false;
                }
            },
        }
        // Coalesce: wait out the minimum gap, then drain everything queued.
        let since = last_run.elapsed();
        if since < MIN_GAP {
            tokio::time::sleep(MIN_GAP - since).await;
        }
        while let Ok(more) = rx.try_recv() {
            trigger.paths.extend(more);
        }
        while let Ok(sid) = idx_rx.try_recv() {
            trigger.indexed.insert(sid);
        }
        run_cycle(&state, &mut cycle, trigger).await;
        last_run = Instant::now();
        let st = state.clone();
        let _ = tokio::task::spawn_blocking(move || st.index.persist(false)).await;
    }
}

/// The index's background worker: full passes of live transcripts first,
/// then history, one file at a time.
async fn index_worker(state: AppState, done: mpsc::UnboundedSender<String>) {
    let mut was_busy = false;
    loop {
        let Some(job) = state.index.next_job() else {
            if was_busy {
                was_busy = false;
                let st = state.clone();
                let _ = tokio::task::spawn_blocking(move || st.index.persist(true)).await;
                tracing::info!("transcript index bootstrap complete");
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), state.index.job_notified()).await;
            continue;
        };
        was_busy = true;
        let sid = job.session_id.clone();
        let st = state.clone();
        let res = tokio::task::spawn_blocking(move || {
            let fi = crate::claude::index::run_job(&job);
            st.index.install(fi);
        })
        .await;
        if let Err(e) = res {
            tracing::warn!(error = %e, "index job failed");
        }
        let _ = done.send(sid);
    }
}

/// Run a single full refresh cycle synchronously: every discovered file is
/// indexed inline (no background worker). Exposed for integration tests.
pub async fn refresh_once(state: &AppState) {
    let st = state.clone();
    let _ = tokio::task::spawn_blocking(move || {
        st.index.load_cache();
        st.index.discover(&st.home);
        st.index.queue_all();
        st.index.drain_jobs_blocking();
    })
    .await;
    let mut cycle = Cycle::default();
    run_cycle(
        state,
        &mut cycle,
        Trigger {
            sweep: true,
            tick: true,
            ..Default::default()
        },
    )
    .await;
}

/// One steady-state refresh (what the 2 s tick does once the index is
/// warm): pidfiles, recent live files, summaries and the snapshot diff — no
/// discovery. Exposed for measurements and tests.
pub async fn tick_once(state: &AppState) {
    let mut cycle = Cycle {
        started: true,
        last_agents_live: Some(Instant::now()),
        ..Default::default()
    };
    run_cycle(
        state,
        &mut cycle,
        Trigger {
            tick: true,
            ..Default::default()
        },
    )
    .await;
}

async fn gather_ctx(state: &AppState) -> BuildCtx {
    let owned_ids = state.owned.read().await.clone();
    let mut owned_meta = HashMap::new();
    let mut bridge_ids = HashSet::new();
    for m in state.control.live() {
        owned_meta.insert(m.id, (m.cwd, m.started_at));
    }
    for m in state.bridge.live_sessions().await {
        bridge_ids.insert(m.id.clone());
        owned_meta.insert(m.id, (m.cwd, m.started_at));
    }
    let previous: HashMap<String, SessionSummary> = state
        .sessions()
        .await
        .into_iter()
        .filter(|s| s.owned)
        .map(|s| (s.id.clone(), s))
        .collect();
    BuildCtx {
        now: now_ms(),
        owned_ids,
        bridge_ids,
        owned_meta,
        previous,
    }
}

async fn run_cycle(state: &AppState, cycle: &mut Cycle, trigger: Trigger) {
    let ctx = gather_ctx(state).await;
    let subs = state.hub.subscriptions();
    let st = state.clone();
    let first = !cycle.started;
    cycle.started = true;
    let live_prev = cycle.live_prev.clone();
    let agents_live_due = cycle
        .last_agents_live
        .is_none_or(|t| t.elapsed() >= AGENTS_LIVE_EVERY);
    if agents_live_due {
        cycle.last_agents_live = Some(Instant::now());
    }
    let owned_ids: HashSet<String> = ctx
        .owned_ids
        .iter()
        .chain(ctx.owned_meta.keys())
        .cloned()
        .collect();

    // Blocking part: roster, discovery, index updates, agents, summaries.
    let work = tokio::task::spawn_blocking(move || {
        let state = st;
        let projects = state.home.projects_dir();
        let change = state.live.refresh(&state.home);
        for gone in &change.gone {
            state.activity.forget(gone);
        }
        // Discovery: on start, on sweeps, and when a watcher path names a
        // file the index does not know.
        let mut discover = first || trigger.sweep;
        let mut touched_sessions: HashSet<String> = trigger.indexed.clone();
        let mut rediscover: HashSet<String> = HashSet::new();
        for p in &trigger.paths {
            let Some(sid) = session_of_path(&projects, p) else {
                continue;
            };
            touched_sessions.insert(sid.clone());
            if classify_path(&projects, p).is_some() && state.index.with_file(p, |_| ()).is_none() {
                match state.index.session_files(&sid) {
                    Some(_) => {
                        rediscover.insert(sid);
                    }
                    None => discover = true,
                }
            }
        }
        if discover {
            let changed = state.index.discover(&state.home);
            touched_sessions.extend(changed);
            if first {
                state.index.queue_all();
            }
        }
        for sid in &rediscover {
            state.index.rediscover_session(sid);
        }
        // Live sessions (and watched ones) are indexed now.
        let mut live: HashSet<String> = state
            .live
            .all()
            .into_iter()
            .filter_map(|s| s.session_id)
            .collect();
        live.extend(owned_ids.iter().cloned());
        let watched: HashSet<String> = subs.iter().map(|(s, _)| s.clone()).collect();
        for sid in live.iter().chain(watched.iter()) {
            state.index.ensure(sid, true);
        }
        for sid in &touched_sessions {
            if !live.contains(sid) {
                state.index.ensure(sid, false);
            }
        }
        // Which files to stat: watcher paths, plus (tick) the recent files of
        // live sessions, plus (sweep) all files of live sessions.
        let mut paths: Vec<PathBuf> = trigger
            .paths
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .cloned()
            .collect();
        if trigger.tick || trigger.sweep || first {
            let targets: Vec<String> = live.iter().chain(watched.iter()).cloned().collect();
            let window = (!trigger.sweep).then_some(RECENT_MS);
            paths.extend(state.index.tick_paths(&targets, now_ms(), window));
        }
        let advances = state.index.update(&paths);
        for a in &advances {
            touched_sessions.insert(a.session_id.clone());
        }
        // Journals / metas / run files change agents without a jsonl advance.
        let liveness_changed: HashSet<String> =
            live.symmetric_difference(&live_prev).cloned().collect();
        let mut agents_due: HashSet<String> = touched_sessions.clone();
        agents_due.extend(liveness_changed);
        if agents_live_due || first {
            agents_due.extend(live.iter().cloned());
        }
        agents_due.extend(watched.iter().cloned());
        let now = now_ms();
        let mut agents_changed: Vec<String> = Vec::new();
        for sid in &agents_due {
            if state.index.session_files(sid).is_none() && !live.contains(sid) {
                continue;
            }
            let pid = state.live.get(sid);
            let actx = AgentCtx {
                live: live.contains(sid),
                process_started: pid.as_ref().and_then(|p| p.started_at),
                owned: owned_ids.contains(sid),
                now,
            };
            if state.agents.recompute(&state.index, sid, &actx) {
                agents_changed.push(sid.clone());
            }
        }
        // Transcript appends for subscriptions.
        let mut appends: Vec<(Advance, crate::claude::uievents::RangeRead)> = Vec::new();
        for a in advances {
            if !subs
                .iter()
                .any(|(s, ag)| *s == a.session_id && *ag == a.agent_id)
            {
                continue;
            }
            match crate::claude::uievents::read_range(
                &a.session_id,
                &a.path,
                a.from,
                Some(a.to),
                a.agent_id.as_deref(),
            ) {
                Ok(r) => appends.push((a, r)),
                Err(e) => tracing::debug!(error = %e, "transcript append read failed"),
            }
        }
        let summaries = summaries::build_all(&state, &ctx);
        (summaries, appends, agents_changed, live)
    })
    .await;
    let (summaries, appends, agents_changed, live) = match work {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "refresh cycle failed");
            return;
        }
    };
    cycle.live_prev = live;

    // Failed transitions → push.
    for s in &summaries {
        let prev = cycle.previous_status.insert(s.id.clone(), s.status);
        if s.status == SessionStatus::Failed && prev.is_some_and(|p| p != SessionStatus::Failed) {
            let text = s
                .last_error
                .as_ref()
                .map(|e| e.text.clone())
                .unwrap_or_else(|| "The session failed.".into());
            crate::server::push::on_session_failed(state, &s.id, &text);
        }
    }

    // Fleet.
    let names: HashMap<&str, &str> = summaries
        .iter()
        .map(|s| (s.id.as_str(), s.name.as_str()))
        .collect();
    let mut fleet = Vec::new();
    let mut summary = FleetSummary::default();
    let midnight = local_midnight_ms();
    for s in &summaries {
        if s.live && s.status == SessionStatus::Working {
            summary.working += 1;
        }
        if s.pending.iter().any(|p| p.answerable) {
            summary.need_you += 1;
        }
        if s.last_activity_at.is_some_and(|t| t >= midnight) {
            summary.cost_today += s.cost_usd.unwrap_or(0.0);
        }
        summary.agents_running += s.agents.running;
        summary.workflows_running += s.agents.workflows_running;
        summary.background_running += s.agents.background_running;
        if s.live {
            let (agents, workflows) = state.agents.for_session(&s.id);
            if let Some(f) = fleet_entry(
                &s.id,
                names.get(s.id.as_str()).copied().unwrap_or(""),
                &agents,
                &workflows,
            ) {
                fleet.push(f);
            }
        }
    }
    summary.cost_today = (summary.cost_today * 100.0).round() / 100.0;
    let fleet_changed = state.agents.set_fleet(fleet.clone(), summary);

    ws::publish_sessions(state, summaries).await;

    for (a, r) in appends {
        if r.reset {
            ws::publish_transcript_reset(state, &a.session_id, a.agent_id.as_deref());
        } else {
            ws::publish_agent_transcript(
                state,
                &a.session_id,
                a.agent_id.as_deref(),
                r.events,
                r.tail.to_string(),
            );
        }
    }
    for sid in agents_changed {
        if state.hub.subscriptions().iter().any(|(s, _)| *s == sid) {
            let (agents, workflows) = state.agents.for_session(&sid);
            ws::publish_agents(state, &sid, agents, workflows);
        }
    }
    if fleet_changed && state.hub.fleet_subscribed() {
        ws::publish_fleet(state, fleet);
    }
}

/// Epoch ms of today's local midnight.
fn local_midnight_ms() -> i64 {
    let now = chrono::Local::now();
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|d| d.and_local_timezone(chrono::Local).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or_else(|| now.timestamp_millis() - 24 * 3600 * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_map_to_sessions() {
        let projects = Path::new("/h/projects");
        let sid = "e2b6972f-7126-4f6e-a490-5ce75cb6a9f9";
        assert_eq!(
            session_of_path(projects, &projects.join(format!("-p/{sid}.jsonl"))).as_deref(),
            Some(sid)
        );
        assert_eq!(
            session_of_path(
                projects,
                &projects.join(format!("-p/{sid}/subagents/workflows/wf_x/journal.jsonl"))
            )
            .as_deref(),
            Some(sid)
        );
        assert_eq!(
            session_of_path(projects, &projects.join("-p/memory/x.md")),
            None
        );
        assert_eq!(
            session_of_path(projects, Path::new("/h/sessions/1.json")),
            None
        );
    }

    #[test]
    fn midnight_is_today() {
        let m = local_midnight_ms();
        let now = now_ms();
        assert!(m <= now && now - m <= 25 * 3600 * 1000);
    }
}
