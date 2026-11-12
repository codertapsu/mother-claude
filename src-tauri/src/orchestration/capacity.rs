//! Admission control (docs/PLAN-BROWSER-AND-TEAMS.md §9, docs/ROUTING.md):
//! whether a role session may start now, and why it waits otherwise.
//!
//! **Contract** (the teams side calls [`admit`] before it launches a role
//! session, with the role sessions it runs now). The types are as fixed
//! before the work began; the decision is:
//!
//! `admit = min(hardware cap, plan-window cap, repository cap, user cap)`,
//! with every reason that holds collected in the user's words:
//!
//! * **Rate limits**: a request Claude rejected on a limit that has not
//!   reset (5 minutes when no reset time was given) stops everything for
//!   that account; a warning about the 5-hour limit in the last 5 minutes
//!   lets only the lead start.
//! * **The 5-hour window** of the account the session runs under: at 85 %
//!   or more only the lead starts, at 95 % or more nothing (both settings).
//!   The weekly per-family windows feed the router instead.
//! * **Your cap** on role sessions running at once (default 3; the lead is
//!   not counted) and **the project's cap** (default 4).
//! * **Memory**: `floor((free − reserve) / per-session estimate)` more fit,
//!   the estimate measured from live sessions (600 MB until then); a lone
//!   session always starts, never more than 16 at once.
//! * **Budgets**: a project whose daily budget is spent starts no more
//!   workers (the lead still starts: a soft alert for it).
//! * **Priorities**: the lead always ahead of workers; a worker yields a
//!   free slot to a higher-priority request that waits for the same slot.
//!
//! The last decision per (project, role) is kept with its reasons
//! ([`Queue`]) for the Capacity page, which so shows who waits and why
//! without knowing the teams code, and so do the role sessions teams said
//! were running.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::host::{self, HostSample};
use super::limits::Limits;
use super::scheduler::{lock, now_ms, RateLimitNote};
use crate::claude::paths;
use crate::state::AppState;

/// The role sessions running at once when nothing else says otherwise: the
/// user's cap's default (§9).
pub const DEFAULT_USER_CAP: usize = 3;

/// A role session the teams side wants to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    /// The project folder.
    pub project: String,
    /// The role id.
    pub role: String,
    /// The model it would run on (after [`super::router::choose`]).
    pub model: String,
    /// 0 (highest) … 3.
    #[serde(default)]
    pub priority: u8,
    /// The account it runs under (`None`: the project's default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The lead (CEO) of a team: admitted ahead of workers.
    #[serde(default)]
    pub lead: bool,
}

/// A role session that runs now (the teams side knows them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningWorker {
    pub session_id: String,
    pub project: String,
    pub role: String,
    pub model: String,
    #[serde(default)]
    pub lead: bool,
    /// A role this computer runs for a team on another computer: that
    /// computer's name. It counts here, where it runs (never at the
    /// lead's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub for_machine: Option<String>,
}

/// May it start?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Admission {
    Admit,
    /// Not now: each reason in the user's words ("3 role sessions run
    /// already (your limit)", "Opus weekly usage at 91%").
    Wait {
        reasons: Vec<String>,
    },
}

impl Admission {
    pub fn is_admit(&self) -> bool {
        matches!(self, Self::Admit)
    }
}

/// A waiting entry counts against a free slot only this long after its
/// last ask (a cancelled task must not hold a slot).
pub const YIELD_TTL_MS: i64 = 90_000;
/// A waiting entry not asked again for this long leaves the queue.
pub const WAIT_TTL_MS: i64 = 10 * 60_000;
/// An admitted entry shows as "started" this long.
pub const STARTED_TTL_MS: i64 = 2 * 60_000;
/// A rejection without a reset time holds this long.
pub const REJECTED_HOLD_MS: i64 = 5 * 60_000;
/// A warning about the 5-hour limit holds workers this long.
pub const WARNING_HOLD_MS: i64 = 5 * 60_000;

/// What the memory rule knows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryFacts {
    pub available: u64,
    pub reserve: u64,
    /// Per role session.
    pub estimate: u64,
    pub measured: bool,
}

/// The signals one decision reads, gathered by [`admit`] (synthetic in
/// tests and the simulation).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signals {
    pub now: i64,
    /// `None` until the host was sampled (the memory rule then waits).
    pub memory: Option<MemoryFacts>,
    /// The account's 5-hour window: percent used and when it resets.
    pub five_hour: Option<(f64, Option<i64>)>,
    /// The account's newest rate-limit report.
    pub rate: Option<RateLimitNote>,
    /// The project's daily budget: spent and limit (estimated USD).
    pub budget: Option<(f64, f64)>,
}

/// One decision as kept and shown.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub project: String,
    pub role: String,
    pub model: String,
    pub priority: u8,
    pub lead: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub admitted: bool,
    /// Why it waits (empty once admitted).
    pub reasons: Vec<String>,
    /// Since when it waits (epoch ms; absent once admitted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_since: Option<i64>,
    /// The last decision (epoch ms).
    pub at: i64,
    /// How often teams asked.
    pub asks: u32,
    /// It waits only for a slot to free up (your cap, memory): a worker
    /// with a lower priority leaves a free slot to it.
    #[serde(skip)]
    pub slot_bound: bool,
}

/// A role session teams said is running.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleSession {
    pub session_id: String,
    pub project: String,
    pub role: String,
    pub model: String,
    pub lead: bool,
    pub first_seen: i64,
    pub last_seen: i64,
    /// It runs here for a team on another computer (that computer's name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub for_machine: Option<String>,
    /// Its cost when first seen (what it spent since is this role's).
    #[serde(skip)]
    pub baseline_usd: Option<f64>,
}

/// The last decision per (project, role) and the role sessions running.
#[derive(Debug, Default)]
pub struct Queue {
    entries: HashMap<(String, String), QueueEntry>,
    pub(crate) roles: HashMap<String, RoleSession>,
}

fn key(project: &str, role: &str) -> (String, String) {
    (paths::path_key(project), role.to_string())
}

impl Queue {
    /// Every entry, waiting first (longest first), then started.
    pub fn entries(&self) -> Vec<QueueEntry> {
        let mut out: Vec<QueueEntry> = self.entries.values().cloned().collect();
        out.sort_by(|a, b| {
            a.admitted
                .cmp(&b.admitted)
                .then(a.waiting_since.cmp(&b.waiting_since))
                .then(b.at.cmp(&a.at))
        });
        out
    }

    /// The role sessions running, by project then role.
    pub fn roles(&self) -> Vec<RoleSession> {
        let mut out: Vec<RoleSession> = self.roles.values().cloned().collect();
        out.sort_by(|a, b| (&a.project, &a.role).cmp(&(&b.project, &b.role)));
        out
    }

    fn remember(&mut self, req: &StartRequest, account: &str, verdict: &Verdict, now: i64) {
        let entry = self
            .entries
            .entry(key(&req.project, &req.role))
            .or_insert_with(|| QueueEntry {
                project: req.project.clone(),
                role: req.role.clone(),
                model: req.model.clone(),
                priority: req.priority,
                lead: req.lead,
                account: None,
                admitted: false,
                reasons: Vec::new(),
                waiting_since: None,
                at: now,
                asks: 0,
                slot_bound: false,
            });
        entry.model = req.model.clone();
        entry.priority = req.priority;
        entry.lead = req.lead;
        entry.account = Some(account.to_string());
        entry.at = now;
        entry.asks = entry.asks.saturating_add(1);
        match &verdict.admission {
            Admission::Admit => {
                entry.admitted = true;
                entry.reasons.clear();
                entry.waiting_since = None;
                entry.slot_bound = false;
            }
            Admission::Wait { reasons } => {
                if entry.admitted || entry.waiting_since.is_none() {
                    entry.waiting_since = Some(now);
                }
                entry.admitted = false;
                entry.reasons = reasons.clone();
                entry.slot_bound = verdict.slot_bound;
            }
        }
    }

    fn note_running(
        &mut self,
        running: &[RunningWorker],
        now: i64,
        cost_of: impl Fn(&str) -> Option<f64>,
    ) {
        for w in running {
            if !crate::claude::ids::is_session_id(&w.session_id) {
                continue;
            }
            let r = self
                .roles
                .entry(w.session_id.clone())
                .or_insert_with(|| RoleSession {
                    session_id: w.session_id.clone(),
                    project: w.project.clone(),
                    role: w.role.clone(),
                    model: w.model.clone(),
                    lead: w.lead,
                    first_seen: now,
                    last_seen: now,
                    for_machine: w.for_machine.clone(),
                    baseline_usd: cost_of(&w.session_id),
                });
            r.project = w.project.clone();
            r.role = w.role.clone();
            r.model = w.model.clone();
            r.lead = w.lead;
            r.for_machine = w.for_machine.clone();
            r.last_seen = now;
        }
    }

    /// Waiting entries older than [`WAIT_TTL_MS`], started ones older than
    /// [`STARTED_TTL_MS`], and role sessions that ended (or were not heard
    /// of for a day) go.
    pub fn forget_stale(&mut self, now: i64, live: impl Fn(&str) -> Option<bool>) {
        self.entries.retain(|_, e| {
            let ttl = if e.admitted {
                STARTED_TTL_MS
            } else {
                WAIT_TTL_MS
            };
            now - e.at <= ttl
        });
        self.roles.retain(|id, r| {
            let ended = live(id) == Some(false) && now - r.last_seen > 60_000;
            !ended && now - r.last_seen <= 24 * 60 * 60_000
        });
    }
}

/// A decision and whether it waits only for a free slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub admission: Admission,
    pub slot_bound: bool,
}

/// "1.1 GB", "600 MB".
pub fn bytes_label(b: u64) -> String {
    if b >= host::GIB {
        format!("{:.1} GB", b as f64 / host::GIB as f64)
    } else {
        format!("{} MB", b / host::MIB)
    }
}

/// "just now", "4 min", "1 h 5 min".
pub fn span_label(ms: i64) -> String {
    let mins = (ms.max(0) + 30_000) / 60_000;
    match mins {
        0 => "less than a minute".into(),
        m if m < 60 => format!("{m} min"),
        m if m % 60 == 0 => format!("{} h", m / 60),
        m => format!("{} h {} min", m / 60, m % 60),
    }
}

/// A limit's name in words.
pub fn limit_label(limit: Option<&str>) -> &'static str {
    match limit {
        Some("five_hour") => "the 5-hour limit",
        Some("seven_day") => "the weekly limit",
        Some("seven_day_opus") => "the Opus weekly limit",
        Some("seven_day_sonnet") => "the Sonnet weekly limit",
        _ => "a usage limit",
    }
}

fn pct_label(p: f64) -> String {
    if p.fract() == 0.0 {
        format!("{p:.0}")
    } else {
        format!("{p:.1}")
    }
}

/// The last folder of a project path ("mother-claude").
pub fn project_name(project: &str) -> String {
    paths::last_component(project)
        .filter(|n| !n.is_empty())
        .unwrap_or(project)
        .to_string()
}

/// Whether a rejection still holds at `now`.
pub fn rejection_holds(note: &RateLimitNote, now: i64) -> bool {
    note.is_rejected()
        && match note.resets_at {
            Some(r) if r > note.at => now < r,
            _ => now - note.at < REJECTED_HOLD_MS,
        }
}

/// Whether a 5-hour warning still holds workers at `now`.
pub fn warning_holds(note: &RateLimitNote, now: i64) -> bool {
    note.is_warning()
        && now - note.at < WARNING_HOLD_MS
        && matches!(note.limit.as_deref(), None | Some("five_hour"))
}

/// The decision itself: pure, every signal given.
pub fn decide(
    limits: &Limits,
    s: &Signals,
    req: &StartRequest,
    running: &[RunningWorker],
    waiting: &[QueueEntry],
) -> Verdict {
    let mut reasons: Vec<String> = Vec::new();
    let mut slot_reasons = 0usize;
    let now = s.now;

    // Rate limits: a rejection stops everyone; a warning, the workers.
    if let Some(note) = &s.rate {
        if rejection_holds(note, now) {
            let reset = match note.resets_at.filter(|r| *r > now) {
                Some(r) => format!(" (it resets in {})", span_label(r - now)),
                None => String::new(),
            };
            reasons.push(format!(
                "Claude refused a request {} ago on {}{reset}",
                span_label(now - note.at),
                limit_label(note.limit.as_deref())
            ));
        } else if !req.lead && warning_holds(note, now) {
            reasons.push(format!(
                "Claude warned {} ago that {} is near; only the lead starts for 5 minutes",
                span_label(now - note.at),
                limit_label(note.limit.as_deref())
            ));
        }
    }

    // The 5-hour window.
    if let Some((pct, resets_at)) = s.five_hour {
        let reset = match resets_at.filter(|r| *r > now) {
            Some(r) => format!(", it resets in {}", span_label(r - now)),
            None => String::new(),
        };
        if pct >= limits.stop_pct {
            reasons.push(format!(
                "the 5-hour usage window is at {} % (nothing starts at {} % or more{reset})",
                pct_label(pct),
                pct_label(limits.stop_pct)
            ));
        } else if !req.lead && pct >= limits.lead_only_pct {
            reasons.push(format!(
                "the 5-hour usage window is at {} % (at {} % or more only the lead starts{reset})",
                pct_label(pct),
                pct_label(limits.lead_only_pct)
            ));
        }
    }

    let workers = running.iter().filter(|w| !w.lead).count();
    let all_roles = running.len();

    // Your cap (workers only: the lead is not counted).
    let free_user = limits.user_cap.saturating_sub(workers);
    if !req.lead && free_user == 0 {
        reasons.push(format!(
            "{workers} role session{} run already (your limit is {})",
            if workers == 1 { "" } else { "s" },
            limits.user_cap
        ));
        slot_reasons += 1;
    }

    // The project's cap (workers in the same project).
    if !req.lead {
        let cap = limits.project_cap_for(&req.project);
        let here = running
            .iter()
            .filter(|w| !w.lead && paths::same_path(&w.project, &req.project))
            .count();
        if here >= cap {
            reasons.push(format!(
                "{here} role session{} run in {} already (its limit is {cap})",
                if here == 1 { "" } else { "s" },
                project_name(&req.project)
            ));
        }
    }

    // Memory headroom.
    let mut free_mem = usize::MAX;
    if let Some(m) = s.memory {
        let headroom = host::headroom(m.available, m.reserve, m.estimate);
        let cap = host::hardware_cap(all_roles, headroom);
        free_mem = cap.saturating_sub(all_roles);
        if free_mem == 0 {
            reasons.push(format!(
                "only {} of memory free (a role session needs about {}{}, and {} stay free)",
                bytes_label(m.available),
                bytes_label(m.estimate),
                if m.measured { " as measured" } else { "" },
                bytes_label(m.reserve)
            ));
            slot_reasons += 1;
        }
    }

    // The project's daily budget: workers only.
    if !req.lead {
        if let Some((spent, limit)) = s.budget.filter(|(_, l)| *l > 0.0) {
            if spent >= limit {
                reasons.push(format!(
                    "{}'s daily budget is spent (${spent:.2} of ${limit:.2}, estimated)",
                    project_name(&req.project)
                ));
            }
        }
    }

    // Priorities: a worker leaves a free slot to a waiting lead or a
    // higher-priority worker.
    if reasons.is_empty() && !req.lead {
        let me = key(&req.project, &req.role);
        let ahead: Vec<&QueueEntry> = waiting
            .iter()
            .filter(|w| {
                !w.admitted
                    && w.slot_bound
                    && now - w.at <= YIELD_TTL_MS
                    && key(&w.project, &w.role) != me
                    && (w.lead || w.priority < req.priority)
            })
            .collect();
        let slots = free_user.min(free_mem);
        if !ahead.is_empty() && ahead.len() >= slots {
            let first = ahead
                .iter()
                .min_by_key(|w| (!w.lead, w.priority, w.waiting_since))
                .copied();
            if let Some(first) = first {
                let who = if first.lead {
                    format!("the lead of {}", project_name(&first.project))
                } else {
                    format!(
                        "{} in {} (P{})",
                        first.role,
                        project_name(&first.project),
                        first.priority
                    )
                };
                reasons.push(if ahead.len() == 1 {
                    format!("{who} waits for the same slot and goes first")
                } else {
                    format!(
                        "{} role sessions with a higher priority wait for the same slots ({who} first)",
                        ahead.len()
                    )
                });
            }
        }
    }

    let slot_bound = !reasons.is_empty() && slot_reasons == reasons.len();
    Verdict {
        admission: if reasons.is_empty() {
            Admission::Admit
        } else {
            Admission::Wait { reasons }
        },
        slot_bound,
    }
}

/// What memory looks like for admission, from a sample and the settings.
pub fn memory_facts(sample: &HostSample, limits: &Limits) -> Option<MemoryFacts> {
    if !sample.sampled() {
        return None;
    }
    let (measured_estimate, measured) = host::session_estimate(&sample.sessions);
    let (estimate, measured) = match limits.session_mb {
        Some(mb) => (mb.saturating_mul(host::MIB), false),
        None => (measured_estimate, measured),
    };
    Some(MemoryFacts {
        available: sample.available_memory,
        reserve: limits.reserve_bytes(),
        estimate,
        measured,
    })
}

/// The account a request runs under: the one it names (when it is one),
/// else the project's default.
pub fn account_for(state: &AppState, req_account: Option<&str>, project: &str) -> String {
    req_account
        .map(str::trim)
        .filter(|a| state.accounts.get(a).is_some())
        .map(str::to_string)
        .unwrap_or_else(|| state.accounts.account_for_cwd(project))
}

/// The account's 5-hour window: percent used and its reset.
pub fn five_hour_of(state: &AppState, account: &str) -> Option<(f64, Option<i64>)> {
    let usage = crate::claude::usage::current_for(state, account);
    if !usage.available {
        return None;
    }
    usage
        .windows
        .iter()
        .find(|w| w.key == "five_hour")
        .map(|w| (w.pct, w.resets_at))
}

/// Gather what a decision about `req` reads now.
pub fn signals_for(state: &AppState, req: &StartRequest, account: &str, now: i64) -> Signals {
    let limits = state.scheduler.limits();
    Signals {
        now,
        memory: memory_facts(&state.scheduler.host.last(), &limits),
        five_hour: five_hour_of(state, account),
        rate: state.scheduler.newest_rate(account),
        budget: limits
            .project_budget(&req.project)
            .map(|limit| (state.scheduler.spend().project_today(&req.project), limit)),
    }
}

/// Decide whether `req` may start while `running` run.
pub fn admit(state: &AppState, req: &StartRequest, running: &[RunningWorker]) -> Admission {
    let now = now_ms();
    let account = account_for(state, req.account.as_deref(), &req.project);
    let signals = signals_for(state, req, &account, now);
    let limits = state.scheduler.limits();
    let spend = state.scheduler.spend();
    let mut queue = lock(&state.scheduler.queue);
    queue.note_running(running, now, |id| spend.session_cost(id));
    let waiting = queue.entries();
    let verdict = decide(&limits, &signals, req, running, &waiting);
    queue.remember(req, &account, &verdict, now);
    drop(queue);
    if let Admission::Wait { reasons } = &verdict.admission {
        tracing::debug!(project = %req.project, role = %req.role, ?reasons, "role session waits");
    }
    verdict.admission
}

/// Drop stale queue entries and ended role sessions, and give a role
/// session (or a task) reported before its session's cost was known its
/// baseline now (the loop).
pub fn forget_stale(state: &AppState, now: i64) {
    let spend = state.scheduler.spend();
    {
        let mut queue = lock(&state.scheduler.queue);
        queue.forget_stale(now, |id| spend.is_live(id));
        for r in queue
            .roles
            .values_mut()
            .filter(|r| r.baseline_usd.is_none())
        {
            r.baseline_usd = spend.session_cost(&r.session_id);
        }
    }
    for t in lock(&state.scheduler.tasks)
        .values_mut()
        .filter(|t| t.baseline_usd.is_none())
    {
        t.baseline_usd = spend.session_cost(&t.session_id);
    }
}

#[cfg(test)]
mod tests;
