//! Budgets (docs/ROUTING.md): per project per day, per task, and per role
//! session — in estimated US dollars — with alerts at the near threshold
//! (80 %) and at 100 % as a desktop notice and a push notification.
//!
//! * **A project's day**: what every session working in the project folder
//!   (worktrees under it included) spent since local midnight — the
//!   sessions' `costToday`, priced from their transcripts. At 100 % the
//!   project starts no more workers (the lead still does); from the near
//!   threshold the router sends Opus work to Sonnet.
//! * **A role session**: [`session_cap_usd`] is the `--max-budget-usd` the
//!   launcher passes on accounts that pay per token ([`pays_per_token`]);
//!   Claude Code enforces it (its own client-side estimate). Here it is
//!   watched from the session's cost since teams first reported it running.
//! * **A task**: watched while it runs when the task was reported
//!   ([`super::metrics::task_started`]), and checked at its end otherwise.
//!
//! Plan accounts pay in usage windows, not dollars: their figures are what
//! the work would have cost through the API, and a task's share of the
//! 5-hour window is estimated from a per-account series ([`window_share`]).

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

use super::capacity::project_name;
use super::limits::Billing;
use super::outcomes::OutcomeRecord;
use super::scheduler::lock;
use crate::claude::accounts::{AccountKind, DEFAULT_ID};
use crate::claude::model::{NoticeLevel, SessionSummary};
use crate::claude::paths;
use crate::state::AppState;

/// Series points kept per account (a day and some).
const SERIES_KEEP_MS: i64 = 26 * 60 * 60_000;
/// Alerts remembered this long (a project's alert is per day).
const ALERT_KEEP_MS: i64 = 3 * 24 * 60 * 60_000;

// ─────────────────────────────── what was spent ───────────────────────────────

/// What one session spent, as of the last tick.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionSpend {
    pub cwd: String,
    pub account: String,
    pub cost_today: f64,
    pub cost_usd: f64,
    pub live: bool,
}

/// What sessions spent, as of the last tick (live sessions and those
/// active today).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spend {
    pub at: i64,
    sessions: HashMap<String, SessionSpend>,
}

impl Spend {
    /// From the session snapshot.
    pub fn from_sessions<'a>(sessions: impl Iterator<Item = &'a SessionSummary>, now: i64) -> Self {
        let day_ago = now - 24 * 60 * 60_000;
        let sessions = sessions
            .filter(|s| {
                s.live
                    || s.cost_today.is_some_and(|c| c > 0.0)
                    || s.last_activity_at.is_some_and(|t| t >= day_ago)
            })
            .map(|s| {
                (
                    s.id.clone(),
                    SessionSpend {
                        cwd: s.cwd.clone(),
                        account: s
                            .account
                            .as_ref()
                            .map_or(DEFAULT_ID, |a| a.id.as_str())
                            .to_string(),
                        cost_today: s.cost_today.unwrap_or(0.0),
                        cost_usd: s.cost_usd.unwrap_or(0.0),
                        live: s.live,
                    },
                )
            })
            .collect();
        Self { at: now, sessions }
    }

    /// Add or replace one session (tests and the simulation).
    pub fn insert(&mut self, id: &str, spend: SessionSpend) {
        self.sessions.insert(id.to_string(), spend);
    }

    /// What sessions in `project` (or under it) spent today.
    pub fn project_today(&self, project: &str) -> f64 {
        self.sessions
            .values()
            .filter(|s| !s.cwd.is_empty() && paths::is_within_recorded(&s.cwd, project))
            .map(|s| s.cost_today)
            .sum()
    }

    /// What `account`'s sessions spent today.
    pub fn account_today(&self, account: &str) -> f64 {
        self.sessions
            .values()
            .filter(|s| s.account == account)
            .map(|s| s.cost_today)
            .sum()
    }

    /// A session's lifetime cost.
    pub fn session_cost(&self, id: &str) -> Option<f64> {
        self.sessions.get(id).map(|s| s.cost_usd)
    }

    /// Whether a session runs (`None`: not seen).
    pub fn is_live(&self, id: &str) -> Option<bool> {
        self.sessions.get(id).map(|s| s.live)
    }
}

// ─────────────────────────────── who pays how ────────────────────────────────

/// How an account pays and how that is known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingFacts {
    /// `None`: not known (treated as a plan: no `--max-budget-usd`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing: Option<Billing>,
    pub source: String,
}

/// How an account pays, from its sign-in (`claude auth status`): a cloud
/// provider, an API key or a Console sign-in pay per token; a claude.ai
/// plan (and a `setup-token` account) through usage windows.
pub fn detect_billing(
    auth: Option<&crate::claude::AuthStatus>,
    kind: Option<AccountKind>,
) -> BillingFacts {
    let facts = |billing, source: &str| BillingFacts {
        billing,
        source: source.into(),
    };
    if let Some(a) = auth.filter(|a| a.error.is_none() && a.logged_in) {
        if a.api_provider
            .as_deref()
            .is_some_and(|p| !p.trim().is_empty() && !p.eq_ignore_ascii_case("firstParty"))
        {
            return facts(Some(Billing::PerToken), "a cloud provider's sign-in");
        }
        let method = a
            .auth_method
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .replace(['_', ' ', '-'], "");
        if method.contains("apikey") {
            return facts(Some(Billing::PerToken), "an API key");
        }
        if method == "console" {
            return facts(Some(Billing::PerToken), "an Anthropic Console sign-in");
        }
        if a.subscription_type
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            || method == "claude.ai"
            || method == "claudeai"
        {
            return facts(Some(Billing::Plan), "a claude.ai plan");
        }
    }
    if kind == Some(AccountKind::Token) {
        return facts(
            Some(Billing::Plan),
            "a setup-token account (a claude.ai plan)",
        );
    }
    facts(None, "not known yet")
}

/// How `account` pays: your setting, else what its sign-in says.
pub fn billing(state: &AppState, account: &str) -> BillingFacts {
    if let Some(b) = state.scheduler.limits().billing.get(account) {
        return BillingFacts {
            billing: Some(*b),
            source: "your setting".into(),
        };
    }
    let kind = state.accounts.get(account).map(|a| a.kind);
    let auth = state.accounts.runtime.status(account).auth;
    detect_billing(auth.as_ref(), kind)
}

/// Whether `account` pays per token.
pub fn pays_per_token_account(state: &AppState, account: &str) -> bool {
    billing(state, account).billing == Some(Billing::PerToken)
}

/// Whether a role session in `project` under `account` (`None`: the
/// project's default account) pays per token — when the launcher should
/// pass [`session_cap_usd`] as `--max-budget-usd`.
pub fn pays_per_token(state: &AppState, account: Option<&str>, project: &str) -> bool {
    let account = super::capacity::account_for(state, account, project);
    pays_per_token_account(state, &account)
}

/// The `--max-budget-usd` for a role session: the role's own budget, else
/// every role session's (default $5; `None` when you turned it off).
pub fn session_cap_usd(state: &AppState, project: &str, role: &str) -> Option<f64> {
    let _ = project;
    let limits = state.scheduler.limits();
    limits
        .budgets
        .roles
        .get(role)
        .copied()
        .or(limits.budgets.session_usd)
}

/// A task's budget (`None`: none set).
pub fn task_cap_usd(state: &AppState, project: &str) -> Option<f64> {
    let _ = project;
    state.scheduler.limits().budgets.task_usd
}

// ─────────────────────────────── window shares ───────────────────────────────

/// One point of an account's series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub at: i64,
    /// The 5-hour window and its reset.
    pub five_hour: Option<(f64, Option<i64>)>,
    /// What the account's sessions spent today.
    pub cost_today: f64,
}

/// Per-account points, a minute apart, for a day.
#[derive(Debug, Default)]
pub struct Series {
    by_account: HashMap<String, VecDeque<Point>>,
}

impl Series {
    pub fn push(&mut self, account: &str, p: Point) {
        let points = self.by_account.entry(account.to_string()).or_default();
        points.push_back(p);
        while points.front().is_some_and(|f| p.at - f.at > SERIES_KEEP_MS) {
            points.pop_front();
        }
    }

    /// The last point at or before `at`.
    pub fn at_or_before(&self, account: &str, at: i64) -> Option<Point> {
        self.by_account
            .get(account)?
            .iter()
            .rev()
            .find(|p| p.at <= at)
            .copied()
    }
}

/// Take a point for every account (the loop, once a minute).
pub fn take_series_point(state: &AppState, now: i64) {
    let spend = state.scheduler.spend();
    let mut series = lock(&state.scheduler.series);
    for a in state.accounts.accounts() {
        series.push(
            &a.id,
            Point {
                at: now,
                five_hour: super::capacity::five_hour_of(state, &a.id),
                cost_today: spend.account_today(&a.id),
            },
        );
    }
}

/// A task's share of its account's 5-hour window: how far the window moved
/// while it ran, times the task's part of what the account spent then.
pub fn share_between(start: Point, end: Point, task_cost: f64) -> Option<f64> {
    let (p0, r0) = start.five_hour?;
    let (p1, r1) = end.five_hour?;
    if r0.is_some() && r1.is_some() && r0 != r1 {
        return None; // the window started over meanwhile
    }
    let moved = p1 - p0;
    let spent = end.cost_today - start.cost_today;
    if moved <= 0.0 || spent <= 0.0 || task_cost <= 0.0 {
        return None;
    }
    let share = moved * (task_cost / spent).min(1.0);
    Some((share * 10.0).round() / 10.0)
}

/// The share of the 5-hour window a finished task used (plan accounts).
pub fn window_share(state: &AppState, record: &OutcomeRecord) -> Option<f64> {
    if record.pays_per_token == Some(true) {
        return None;
    }
    let account = record.account.as_deref()?;
    let cost = record.cost_usd?;
    let start = lock(&state.scheduler.series).at_or_before(account, record.outcome.started_at)?;
    let end = Point {
        at: super::scheduler::now_ms(),
        five_hour: super::capacity::five_hour_of(state, account),
        cost_today: state.scheduler.spend().account_today(account),
    };
    share_between(start, end, cost)
}

// ─────────────────────────────── budget states ───────────────────────────────

/// One budget and how much of it is used.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetState {
    /// `project`, `session` or `task`.
    pub scope: &'static str,
    /// What it is about (a project folder, a session id, a task id).
    pub key: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub spent_usd: f64,
    pub limit_usd: f64,
    pub pct: f64,
    /// `ok`, `near` or `over`.
    pub level: &'static str,
    /// Whether it is enforced and alerts (a session budget only on accounts
    /// that pay per token).
    pub applies: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn state_of(
    scope: &'static str,
    key: String,
    label: String,
    spent: f64,
    limit: f64,
    near_pct: f64,
) -> BudgetState {
    let spent = (spent.max(0.0) * 100.0).round() / 100.0;
    let pct = if limit > 0.0 {
        ((spent / limit * 100.0) * 10.0).round() / 10.0
    } else {
        0.0
    };
    BudgetState {
        scope,
        key,
        label,
        project: None,
        role: None,
        session_id: None,
        spent_usd: spent,
        limit_usd: limit,
        pct,
        level: if pct >= 100.0 {
            "over"
        } else if pct >= near_pct {
            "near"
        } else {
            "ok"
        },
        applies: true,
        note: None,
    }
}

/// Every budget in force now.
pub fn states(state: &AppState) -> Vec<BudgetState> {
    let limits = state.scheduler.limits();
    let spend = state.scheduler.spend();
    let near = limits.budgets.near_pct;
    let mut out = Vec::new();

    // Projects: those with their own budget, and — when every project has
    // one — those with role sessions or queue entries.
    let mut projects: Vec<String> = limits.budgets.projects.keys().cloned().collect();
    let (roles, queued) = {
        let q = lock(&state.scheduler.queue);
        (q.roles(), q.entries())
    };
    if limits.budgets.project_daily_usd.is_some() {
        for p in roles
            .iter()
            .map(|r| &r.project)
            .chain(queued.iter().map(|e| &e.project))
        {
            if !projects.iter().any(|q| paths::same_path(q, p)) {
                projects.push(p.clone());
            }
        }
    }
    for project in projects {
        let Some(limit) = limits.project_budget(&project) else {
            continue;
        };
        let mut s = state_of(
            "project",
            project.clone(),
            format!("{} today", project_name(&project)),
            spend.project_today(&project),
            limit,
            near,
        );
        s.project = Some(project);
        out.push(s);
    }

    // Role sessions.
    for r in roles {
        let Some(cap) = session_cap_usd(state, &r.project, &r.role) else {
            continue;
        };
        let spent = spend
            .session_cost(&r.session_id)
            .map(|c| c - r.baseline_usd.unwrap_or(0.0))
            .unwrap_or(0.0);
        let account = crate::claude::summaries::acting_account(state, &r.session_id);
        let applies = pays_per_token_account(state, &account);
        let mut s = state_of(
            "session",
            r.session_id.clone(),
            format!("{} in {}", r.role, project_name(&r.project)),
            spent,
            cap,
            near,
        );
        s.project = Some(r.project);
        s.role = Some(r.role);
        s.session_id = Some(r.session_id);
        s.applies = applies;
        if !applies {
            s.note = Some("a plan account: not passed to Claude Code, shown only".into());
        }
        out.push(s);
    }

    // Tasks reported running.
    let tasks: Vec<super::metrics::LiveTask> =
        lock(&state.scheduler.tasks).values().cloned().collect();
    for t in tasks {
        let Some(cap) = task_cap_usd(state, &t.project) else {
            continue;
        };
        let spent =
            spend.session_cost(&t.session_id).unwrap_or(0.0) - t.baseline_usd.unwrap_or(0.0);
        let mut s = state_of(
            "task",
            t.task_id.clone(),
            format!("task {} in {}", t.task_id, project_name(&t.project)),
            spent,
            cap,
            near,
        );
        s.project = Some(t.project);
        s.role = t.role;
        s.session_id = Some(t.session_id);
        out.push(s);
    }
    out
}

// ─────────────────────────────── alerts ──────────────────────────────────────

/// Alerts already sent (each budget alerts once per threshold; a project's
/// once per threshold per day).
#[derive(Debug, Default)]
pub struct AlertMemory {
    sent: HashMap<String, i64>,
}

impl AlertMemory {
    /// Whether `key` is new (and remember it).
    pub fn first(&mut self, key: String, now: i64) -> bool {
        self.sent.retain(|_, at| now - *at <= ALERT_KEEP_MS);
        match self.sent.entry(key) {
            std::collections::hash_map::Entry::Occupied(_) => false,
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(now);
                true
            }
        }
    }
}

fn local_day(now: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(now)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// The texts of an alert: the desktop's, the lock-screen-safe push line,
/// and the push detail (for phones that show details).
pub fn alert_texts(s: &BudgetState, threshold: f64) -> (String, String, String) {
    let over = threshold >= 100.0;
    let what = match s.scope {
        "project" => "Project budget",
        "session" => "Role session budget",
        _ => "Task budget",
    };
    let short = if over {
        format!("{what} reached")
    } else {
        format!("{what} {threshold:.0} % used")
    };
    let amounts = format!("${:.2} of ${:.2}, estimated", s.spent_usd, s.limit_usd);
    let desktop = match (s.scope, over) {
        ("project", true) => format!(
            "Budget: {} reached its daily budget ({amounts}). New role sessions there wait until \
             tomorrow; the lead still starts.",
            s.label.trim_end_matches(" today")
        ),
        ("project", false) => format!(
            "Budget: {} has used {:.0} % of its daily budget ({amounts}); Opus work there now \
             runs on Sonnet.",
            s.label.trim_end_matches(" today"),
            s.pct.floor()
        ),
        ("session", true) => format!(
            "Budget: the {} session reached its budget ({amounts}); Claude Code stops it at its \
             --max-budget-usd.",
            s.label
        ),
        ("session", false) => format!(
            "Budget: the {} session has used {:.0} % of its budget ({amounts}).",
            s.label,
            s.pct.floor()
        ),
        (_, true) => format!("Budget: {} reached its budget ({amounts}).", s.label),
        (_, false) => format!(
            "Budget: {} has used {:.0} % of its budget ({amounts}).",
            s.label,
            s.pct.floor()
        ),
    };
    let detail = format!("{} · {amounts}", s.label);
    (desktop, short, detail)
}

/// Send one alert where the settings say.
pub fn alert(state: &AppState, key: &str, desktop: &str, short: &str, detail: &str) {
    let limits = state.scheduler.limits();
    tracing::info!(%key, "{desktop}");
    if limits.alerts.desktop {
        crate::server::ws::publish_desktop_notice(state, NoticeLevel::Warning, desktop);
    }
    if limits.alerts.push {
        crate::server::push::on_budget_alert(state, key, short, detail);
    }
}

/// Check every budget and alert at the thresholds (the loop).
pub fn check(state: &AppState, now: i64) {
    let near = state.scheduler.limits().budgets.near_pct;
    let day = local_day(now);
    for s in states(state)
        .into_iter()
        .filter(|s| s.applies && s.limit_usd > 0.0)
    {
        for threshold in [near, 100.0] {
            if s.pct < threshold {
                continue;
            }
            let key = match s.scope {
                "project" => format!("project:{}:{threshold}:{day}", paths::path_key(&s.key)),
                scope => format!("{scope}:{}:{threshold}", s.key),
            };
            if lock(&state.scheduler.alerts).first(key.clone(), now) {
                let (desktop, short, detail) = alert_texts(&s, threshold);
                alert(state, &key, &desktop, &short, &detail);
            }
        }
    }
}

/// A finished task's cost against its budget (once).
pub fn check_task_end(state: &AppState, record: &OutcomeRecord) {
    let (Some(cap), Some(cost)) = (
        task_cap_usd(state, &record.outcome.project),
        record.cost_usd,
    ) else {
        return;
    };
    if cost < cap {
        return;
    }
    let key = format!("task:{}:100", record.outcome.task_id);
    if !lock(&state.scheduler.alerts).first(key.clone(), super::scheduler::now_ms()) {
        return;
    }
    let s = state_of(
        "task",
        record.outcome.task_id.clone(),
        format!(
            "task {} in {}",
            record.outcome.task_id,
            project_name(&record.outcome.project)
        ),
        cost,
        cap,
        100.0,
    );
    let (desktop, short, detail) = alert_texts(&s, 100.0);
    alert(state, &key, &desktop, &short, &detail);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::AuthStatus;

    fn auth(method: Option<&str>, provider: Option<&str>, plan: Option<&str>) -> AuthStatus {
        AuthStatus {
            logged_in: true,
            auth_method: method.map(str::to_string),
            api_provider: provider.map(str::to_string),
            subscription_type: plan.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn billing_follows_the_sign_in() {
        let b = |a: AuthStatus| detect_billing(Some(&a), Some(AccountKind::Directory)).billing;
        assert_eq!(
            b(auth(Some("apiKey"), Some("firstParty"), None)),
            Some(Billing::PerToken)
        );
        assert_eq!(
            b(auth(Some("api_key"), None, None)),
            Some(Billing::PerToken)
        );
        assert_eq!(
            b(auth(Some("console"), None, None)),
            Some(Billing::PerToken)
        );
        assert_eq!(
            b(auth(None, Some("bedrock"), None)),
            Some(Billing::PerToken)
        );
        assert_eq!(
            b(auth(Some("claude.ai"), Some("firstParty"), Some("max"))),
            Some(Billing::Plan)
        );
        assert_eq!(b(auth(Some("oauth"), None, None)), None);
        // A failed check says nothing; a token account is a plan.
        let failed = AuthStatus {
            error: Some("timeout".into()),
            ..Default::default()
        };
        assert_eq!(detect_billing(Some(&failed), None).billing, None);
        assert_eq!(
            detect_billing(None, Some(AccountKind::Token)).billing,
            Some(Billing::Plan)
        );
    }

    #[test]
    fn spend_is_summed_by_project_folder_and_account() {
        let mut s = Spend::default();
        let ss = |cwd: &str, account: &str, today: f64| SessionSpend {
            cwd: cwd.into(),
            account: account.into(),
            cost_today: today,
            cost_usd: today * 2.0,
            live: true,
        };
        s.insert("a", ss("/Users/me/app", "default", 1.5));
        s.insert(
            "b",
            ss("/Users/me/app/.claude/worktrees/backend", "default", 2.0),
        );
        s.insert("c", ss("/Users/me/app2", "acc_000000000001", 4.0));
        s.insert("d", ss("C:\\dev\\App", "default", 1.0));
        assert_eq!(s.project_today("/Users/me/app"), 3.5);
        assert_eq!(s.project_today("/Users/me/app/"), 3.5);
        assert_eq!(s.project_today("c:/dev/app"), 1.0);
        assert_eq!(s.account_today("default"), 4.5);
        assert_eq!(s.session_cost("c"), Some(8.0));
        assert_eq!(s.is_live("zzz"), None);
    }

    #[test]
    fn a_tasks_window_share_is_its_part_of_the_move() {
        let p = |at, pct, cost| Point {
            at,
            five_hour: Some((pct, Some(9_999))),
            cost_today: cost,
        };
        // The window moved 10 points while the account spent $4; the task
        // spent $1 of it: 2.5 points.
        assert_eq!(
            share_between(p(0, 40.0, 6.0), p(10, 50.0, 10.0), 1.0),
            Some(2.5)
        );
        // More than the account's spend (a race): capped at the whole move.
        assert_eq!(
            share_between(p(0, 40.0, 6.0), p(10, 50.0, 7.0), 3.0),
            Some(10.0)
        );
        // A reset in between, no move, midnight: unknown.
        let reset = Point {
            five_hour: Some((5.0, Some(1))),
            ..p(10, 0.0, 9.0)
        };
        assert_eq!(share_between(p(0, 40.0, 6.0), reset, 1.0), None);
        assert_eq!(share_between(p(0, 40.0, 6.0), p(10, 40.0, 9.0), 1.0), None);
        assert_eq!(share_between(p(0, 40.0, 6.0), p(10, 45.0, 1.0), 1.0), None);
        let mut series = Series::default();
        series.push("a", p(1_000, 1.0, 1.0));
        series.push("a", p(61_000, 2.0, 2.0));
        assert_eq!(series.at_or_before("a", 60_000).unwrap().at, 1_000);
        assert!(series.at_or_before("a", 999).is_none());
        assert!(series.at_or_before("b", 99_999).is_none());
    }

    #[test]
    fn alerts_say_what_and_how_much() {
        let s = state_of(
            "project",
            "/Users/me/app".into(),
            "app today".into(),
            16.2,
            20.0,
            80.0,
        );
        assert_eq!((s.pct, s.level), (81.0, "near"));
        let (desktop, short, detail) = alert_texts(&s, 80.0);
        assert_eq!(
            desktop,
            "Budget: app has used 81 % of its daily budget ($16.20 of $20.00, estimated); Opus \
             work there now runs on Sonnet."
        );
        assert_eq!(short, "Project budget 80 % used");
        assert_eq!(detail, "app today · $16.20 of $20.00, estimated");
        let over = state_of(
            "session",
            "s".into(),
            "backend in app".into(),
            5.01,
            5.0,
            80.0,
        );
        assert_eq!(over.level, "over");
        assert_eq!(alert_texts(&over, 100.0).1, "Role session budget reached");
        let mut mem = AlertMemory::default();
        assert!(mem.first("k".into(), 0));
        assert!(!mem.first("k".into(), 1));
        assert!(
            mem.first("k".into(), ALERT_KEEP_MS + 10),
            "forgotten after days"
        );
    }
}
