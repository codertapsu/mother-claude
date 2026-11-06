//! The scheduler side's state and loop (docs/ROUTING.md): everything the
//! router ([`super::router`]), admission control ([`super::capacity`]),
//! outcome records ([`super::metrics`]) and budgets ([`super::budget`])
//! share. AppState field: `state.scheduler`.
//!
//! The loop ([`start`], every 5 s) samples the host (memory, load, each live
//! session's resident memory), refreshes what sessions spent today from the
//! session snapshot, keeps a per-account series of the 5-hour window and
//! spend (once a minute, for a task's share of the window), checks budgets
//! and raises their alerts, and forgets stale queue entries. It never
//! launches, stops or messages a session.
//!
//! Rate-limit backpressure: owned sessions' `rate_limit_event`s
//! ([`note_rate_limit_event`], called by `claude::control`) and a hooked
//! session's `StopFailure` on a rate limit ([`note_rate_limited`]) are kept
//! here (the last 100, a day at most).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use super::budget::{AlertMemory, Series, Spend};
use super::capacity::Queue;
use super::classify::ClassifierStats;
use super::host::HostMonitor;
use super::limits::Limits;
use super::metrics::LiveTask;
use super::outcomes::{Learned, OutcomeStore};
use super::policy::Policy;
use super::router::RouteNote;
use crate::state::AppState;

/// How often the loop runs.
pub const TICK: Duration = Duration::from_secs(5);
/// How often the per-account window/spend series takes a point.
pub const SERIES_EVERY_MS: i64 = 60_000;
/// Rate-limit events kept.
const RATE_KEEP: usize = 100;
/// … and for how long.
const RATE_KEEP_MS: i64 = 24 * 60 * 60_000;
/// Routing decisions kept for the Capacity page.
pub const ROUTES_KEEP: usize = 50;

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A rate-limit report: an owned session's `rate_limit_event` or a hooked
/// session's turn that died on a rate limit.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitNote {
    pub at: i64,
    pub account: String,
    /// `allowed`, `allowed_warning` or `rejected`.
    pub status: String,
    /// `five_hour`, `seven_day`, … when the CLI said which.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    /// How much of the limit is used (0–100), when the CLI said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pct: Option<f64>,
    /// `stream` (an owned session) or `hook`.
    pub source: &'static str,
}

impl RateLimitNote {
    pub fn is_rejected(&self) -> bool {
        self.status == "rejected"
    }

    pub fn is_warning(&self) -> bool {
        self.status == "allowed_warning"
    }
}

/// The rate-limit reports kept.
#[derive(Debug, Default)]
pub struct RateLimits {
    notes: VecDeque<RateLimitNote>,
}

impl RateLimits {
    pub fn push(&mut self, note: RateLimitNote) {
        let now = note.at;
        self.notes.push_back(note);
        while self.notes.len() > RATE_KEEP {
            self.notes.pop_front();
        }
        self.notes.retain(|n| now - n.at <= RATE_KEEP_MS);
    }

    /// The newest report of `account`.
    pub fn newest(&self, account: &str) -> Option<&RateLimitNote> {
        self.notes.iter().rev().find(|n| n.account == account)
    }

    /// Every report, newest first.
    pub fn all(&self) -> Vec<RateLimitNote> {
        self.notes.iter().rev().cloned().collect()
    }
}

/// The scheduler side's state.
pub struct Scheduler {
    pub host: HostMonitor,
    data_dir: Option<PathBuf>,
    limits: RwLock<Limits>,
    policy: RwLock<Policy>,
    /// Last admission decision per (project, role) and the role sessions
    /// teams reported running.
    pub(crate) queue: Mutex<Queue>,
    pub(crate) rate: Mutex<RateLimits>,
    spend: RwLock<Spend>,
    pub(crate) learned: Mutex<Learned>,
    pub(crate) outcomes: OutcomeStore,
    pub(crate) alerts: Mutex<AlertMemory>,
    pub(crate) series: Mutex<Series>,
    pub(crate) routes: Mutex<VecDeque<RouteNote>>,
    pub(crate) classifier: Mutex<ClassifierStats>,
    pub(crate) tasks: Mutex<HashMap<String, LiveTask>>,
    /// Serializes saves of the settings and the policy.
    save: Mutex<()>,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl Scheduler {
    /// Load the settings, the policy and the outcome log from `data_dir`
    /// (`None`: in memory only, as in tests), and rebuild the router's
    /// escalation state from the outcomes.
    pub fn load(data_dir: Option<&Path>) -> Self {
        let limits = Limits::load(data_dir.map(Limits::path).as_deref());
        let policy = Policy::load(data_dir.map(Policy::path).as_deref());
        let outcomes = OutcomeStore::open(data_dir);
        let mut learned = Learned::default();
        for r in outcomes.all() {
            learned.note(&r.outcome, &policy, r.recorded_at);
        }
        Self {
            host: HostMonitor::new(),
            data_dir: data_dir.map(Path::to_path_buf),
            limits: RwLock::new(limits),
            policy: RwLock::new(policy),
            queue: Mutex::new(Queue::default()),
            rate: Mutex::new(RateLimits::default()),
            spend: RwLock::new(Spend::default()),
            learned: Mutex::new(learned),
            outcomes,
            alerts: Mutex::new(AlertMemory::default()),
            series: Mutex::new(Series::default()),
            routes: Mutex::new(VecDeque::new()),
            classifier: Mutex::new(ClassifierStats::default()),
            tasks: Mutex::new(HashMap::new()),
            save: Mutex::new(()),
        }
    }

    /// The current settings.
    pub fn limits(&self) -> Limits {
        self.limits
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Replace the settings and save them (in memory when there is no data
    /// dir). The save failing is logged; the new settings apply anyway.
    pub fn set_limits(&self, next: Limits) {
        let _g = lock(&self.save);
        if let Some(dir) = &self.data_dir {
            if let Err(e) = next.save(&Limits::path(dir)) {
                tracing::warn!(error = %e, "could not save the scheduler settings");
            }
        }
        *self.limits.write().unwrap_or_else(|p| p.into_inner()) = next;
    }

    /// The current routing policy.
    pub fn policy(&self) -> Policy {
        self.policy
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Replace the policy (`None`: back to the defaults, the file removed).
    pub fn set_policy(&self, next: Option<Policy>) -> Policy {
        let _g = lock(&self.save);
        let policy = next.clone().unwrap_or_default();
        if let Some(dir) = &self.data_dir {
            let path = Policy::path(dir);
            let result = match &next {
                Some(p) => p.save(&path),
                None => match std::fs::remove_file(&path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                    _ => Ok(()),
                },
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "could not save the routing policy");
            }
        }
        *self.policy.write().unwrap_or_else(|p| p.into_inner()) = policy.clone();
        policy
    }

    /// Whether the policy is the built-in one (no saved file in effect).
    pub fn policy_is_default(&self) -> bool {
        self.policy() == Policy::default()
    }

    /// What sessions spent, as of the last tick.
    pub fn spend(&self) -> Spend {
        self.spend.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Replace the spend snapshot (the loop, and tests).
    pub fn set_spend(&self, spend: Spend) {
        *self.spend.write().unwrap_or_else(|p| p.into_inner()) = spend;
    }

    /// Record a rate-limit report.
    pub fn note_rate(&self, note: RateLimitNote) {
        lock(&self.rate).push(note);
    }

    /// The newest rate-limit report of `account`.
    pub fn newest_rate(&self, account: &str) -> Option<RateLimitNote> {
        lock(&self.rate).newest(account).cloned()
    }
}

/// Epoch seconds (or ms, when already that large) as ms.
fn epoch_ms(v: Option<&Value>) -> Option<i64> {
    let n = v
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n > 0.0)?;
    Some(if n < 1e11 {
        (n * 1000.0) as i64
    } else {
        n as i64
    })
}

/// A `rate_limit_event` line as a note (`None` for any other line).
pub fn parse_rate_limit_event(account: &str, v: &Value, now: i64) -> Option<RateLimitNote> {
    if v.get("type").and_then(Value::as_str) != Some("rate_limit_event") {
        return None;
    }
    let info = v.get("rate_limit_info")?.as_object()?;
    let status = info.get("status").and_then(Value::as_str)?.trim();
    if status.is_empty() || status.len() > 40 {
        return None;
    }
    let limit = info
        .get("rateLimitType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| {
            !t.is_empty()
                && t.len() <= 40
                && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .map(str::to_string);
    let pct = info
        .get("utilization")
        .and_then(Value::as_f64)
        .filter(|u| u.is_finite() && *u >= 0.0)
        .map(|u| ((u.min(10.0) * 1000.0).round()) / 10.0);
    Some(RateLimitNote {
        at: now,
        account: account.to_string(),
        status: status.to_string(),
        limit,
        resets_at: epoch_ms(info.get("resetsAt")),
        pct,
        source: "stream",
    })
}

/// An owned session's `rate_limit_event` (every one, whatever account it
/// runs under: a rejection is backpressure either way).
pub fn note_rate_limit_event(state: &AppState, account: &str, v: &Value) {
    if let Some(note) = parse_rate_limit_event(account, v, now_ms()) {
        state.scheduler.note_rate(note);
    }
}

/// A hooked session's turn died on a rate limit (`StopFailure`).
pub fn note_rate_limited(state: &AppState, account: &str) {
    state.scheduler.note_rate(RateLimitNote {
        at: now_ms(),
        account: account.to_string(),
        status: "rejected".into(),
        limit: None,
        resets_at: None,
        pct: None,
        source: "hook",
    });
}

/// Start the loop (once, next to the other subsystems' `start`).
pub fn start(state: &AppState) {
    if tokio::runtime::Handle::try_current().is_err() {
        tracing::warn!(
            "scheduler::start called outside a Tokio runtime; the scheduler loop stays off"
        );
        return;
    }
    let weak = std::sync::Arc::downgrade(state);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_series = 0;
        loop {
            tick.tick().await;
            let Some(state) = weak.upgrade() else {
                break;
            };
            let now = now_ms();
            run_once(&state, now, now - last_series >= SERIES_EVERY_MS).await;
            if now - last_series >= SERIES_EVERY_MS {
                last_series = now;
            }
        }
    });
}

/// One pass of the loop.
pub async fn run_once(state: &AppState, now: i64, take_series: bool) {
    // The host: RAM, load, and the processes the roster knows.
    let procs: Vec<(String, u32)> = state
        .live
        .all()
        .into_iter()
        .filter_map(|l| Some((l.session_id?, l.pid?)))
        .collect();
    let st = state.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || st.scheduler.host.sample(&procs, now)).await
    {
        tracing::debug!(error = %e, "host sample failed");
    }
    // What sessions spent (only what the budgets and the queue need).
    let spend = {
        let snap = state.snapshot.read().await;
        Spend::from_sessions(snap.sessions.iter(), now)
    };
    state.scheduler.set_spend(spend);
    if take_series {
        super::budget::take_series_point(state, now);
    }
    super::capacity::forget_stale(state, now);
    super::budget::check(state, now);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rate_limit_events_become_notes() {
        let n = parse_rate_limit_event(
            "default",
            &json!({ "type": "rate_limit_event", "rate_limit_info": {
                "status": "rejected", "rateLimitType": "five_hour",
                "resetsAt": 1_790_010_000, "utilization": 1.02 } }),
            7,
        )
        .unwrap();
        assert!(n.is_rejected());
        assert_eq!(n.limit.as_deref(), Some("five_hour"));
        assert_eq!(n.resets_at, Some(1_790_010_000_000));
        assert_eq!(n.pct, Some(102.0));
        assert_eq!(n.at, 7);
        let w = parse_rate_limit_event(
            "acc_0123456789ab",
            &json!({ "type": "rate_limit_event", "rate_limit_info": {
                "status": "allowed_warning", "rateLimitType": "x y" } }),
            7,
        )
        .unwrap();
        assert!(w.is_warning());
        assert_eq!(w.limit, None, "an odd limit name is not kept");
        assert!(parse_rate_limit_event("d", &json!({ "type": "result" }), 0).is_none());
        assert!(parse_rate_limit_event(
            "d",
            &json!({ "type": "rate_limit_event", "rate_limit_info": {} }),
            0
        )
        .is_none());
    }

    #[test]
    fn rate_notes_are_bounded_and_found_per_account() {
        let mut r = RateLimits::default();
        for i in 0..150 {
            r.push(RateLimitNote {
                at: 1_000 + i,
                account: if i % 2 == 0 { "a" } else { "b" }.into(),
                status: "allowed".into(),
                limit: None,
                resets_at: None,
                pct: None,
                source: "stream",
            });
        }
        assert_eq!(r.all().len(), RATE_KEEP);
        assert_eq!(r.newest("a").unwrap().at, 1_148);
        assert_eq!(r.newest("b").unwrap().at, 1_149);
        assert!(r.newest("c").is_none());
        // A day later the old ones go.
        r.push(RateLimitNote {
            at: 1_000 + RATE_KEEP_MS + 500,
            account: "c".into(),
            status: "rejected".into(),
            limit: None,
            resets_at: None,
            pct: None,
            source: "hook",
        });
        assert_eq!(r.all().len(), 1);
    }
}
