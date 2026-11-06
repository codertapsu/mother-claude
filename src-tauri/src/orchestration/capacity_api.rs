//! The scheduler side's HTTP routes (docs/ROUTING.md), merged into the
//! protected `/api` router (every caller already passed
//! `auth::require_api_principal`):
//!
//! * `GET /api/host` (Authed): the last host sample — memory, load, each
//!   live session's resident memory — with the per-session estimate.
//! * `GET /api/capacity` (Authed): everything the Capacity page shows: caps
//!   and signals, the role sessions running, who waits and why, budgets,
//!   recent routing decisions, the settings.
//! * `PUT /api/capacity/settings` (ManageData: the desktop or a paired
//!   device; the Haiku classifier and billing overrides desktop only): a
//!   partial update of the settings → the Capacity view; `422` with `field`.
//! * `GET|PUT|DELETE /api/routing/policy` (read: Authed; edit and reset:
//!   DesktopOnly): the routing policy, validated.
//! * `GET /api/routing/report?days=&project=` (Authed) and
//!   `GET /api/routing/report.csv?days=&project=&rows=groups|outcomes`.
//! * `POST /api/routing/classify {title, goal, haiku?}` (Authed; `haiku`
//!   spends tokens, so the desktop or a paired device only).

use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::budget::{self, BillingFacts, BudgetState};
use super::capacity::{self, QueueEntry, RoleSession};
use super::classify::{self, Classification, ClassifierStats};
use super::host::{self, HostSample};
use super::limits::{Invalid, Limits};
use super::outcomes::{ClassView, EscalationNote};
use super::policy::{self, Policy};
use super::reports::{self, Report};
use super::router::RouteNote;
use super::scheduler::{lock, now_ms, RateLimitNote};
use crate::claude::model::PlanWindow;
use crate::server::auth::{note_device_action, Authed, DesktopOnly, ManageData};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// The routes, for `server::routes::api_router`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/host", get(get_host))
        .route("/capacity", get(get_capacity))
        .route("/capacity/settings", put(put_settings))
        .route(
            "/routing/policy",
            get(get_policy).put(put_policy).delete(reset_policy),
        )
        .route("/routing/report", get(get_report))
        .route("/routing/report.csv", get(get_report_csv))
        .route("/routing/classify", post(post_classify))
}

fn invalid(e: Invalid, code: &str) -> ApiError {
    let mut err = ApiError::unprocessable(code.to_string(), e.message);
    if !e.field.is_empty() {
        err = err.with("field", e.field);
    }
    err
}

// ─────────────────────────────── host ─────────────────────────────────────────

/// `GET /api/host`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    #[serde(flatten)]
    pub sample: HostSample,
    /// What one role session is expected to need.
    pub estimate_bytes: u64,
    /// Whether that was measured from live sessions (or is your setting /
    /// the 600 MB default).
    pub measured: bool,
    /// What always stays free.
    pub reserve_bytes: u64,
    /// How many more role sessions fit in memory now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headroom: Option<usize>,
}

pub fn host_view(state: &AppState, limits: &Limits) -> HostView {
    let sample = state.scheduler.host.last();
    let facts = capacity::memory_facts(&sample, limits);
    let (estimate_bytes, measured) = match facts {
        Some(f) => (f.estimate, f.measured),
        None => (
            limits
                .session_mb
                .map(|mb| mb * host::MIB)
                .unwrap_or(host::DEFAULT_SESSION),
            false,
        ),
    };
    HostView {
        headroom: facts.map(|f| host::headroom(f.available, f.reserve, f.estimate)),
        sample,
        estimate_bytes,
        measured,
        reserve_bytes: limits.reserve_bytes(),
    }
}

async fn get_host(Authed(_ctx): Authed, State(state): State<AppState>) -> Json<HostView> {
    Json(host_view(&state, &state.scheduler.limits()))
}

// ─────────────────────────────── capacity ─────────────────────────────────────

/// The caps in force now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsView {
    pub user_cap: usize,
    pub project_cap: usize,
    pub workers_running: usize,
    pub leads_running: usize,
    /// Role sessions memory allows at once (absent before the first sample).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_cap: Option<usize>,
}

/// One account's signals.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSignals {
    pub id: String,
    pub name: String,
    pub billing: BillingFacts,
    /// Whether the plan usage is known for it.
    pub plan_usage: bool,
    /// The 5-hour and weekly windows (Opus and Sonnet weekly when the plan
    /// has them).
    pub windows: Vec<PlanWindow>,
    /// At the 5-hour lead-only threshold.
    pub lead_only: bool,
    /// At the 5-hour stop threshold.
    pub stopped: bool,
    /// Opus work runs on Sonnet (the Opus weekly gate).
    pub opus_gated: bool,
    /// The newest rate-limit report, while it holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitNote>,
}

/// A role session with what it spent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningView {
    #[serde(flatten)]
    pub session: RoleSession,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<bool>,
    /// Estimated USD since teams first reported it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spent_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
}

/// The optional classifier.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifierView {
    pub enabled: bool,
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_not: Option<String>,
    pub model: &'static str,
    pub stats: ClassifierStats,
}

/// `GET /api/capacity`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityView {
    pub now: i64,
    pub host: HostView,
    pub caps: CapsView,
    pub accounts: Vec<AccountSignals>,
    pub rate_limits: Vec<RateLimitNote>,
    pub running: Vec<RunningView>,
    pub queue: Vec<QueueEntry>,
    pub budgets: Vec<BudgetState>,
    pub routes: Vec<RouteNote>,
    pub settings: Limits,
    pub classifier: ClassifierView,
    /// The caller may change settings (desktop or paired device).
    pub can_manage: bool,
    /// The caller may edit the routing policy (desktop).
    pub can_edit_policy: bool,
}

const WINDOW_KEYS: [&str; 4] = [
    "five_hour",
    "seven_day",
    "seven_day_opus",
    "seven_day_sonnet",
];

pub fn capacity_view(state: &AppState, ctx: &ClientContext) -> CapacityView {
    let now = now_ms();
    let limits = state.scheduler.limits();
    let host = host_view(state, &limits);
    let spend = state.scheduler.spend();
    let (roles, queue) = {
        let q = lock(&state.scheduler.queue);
        (q.roles(), q.entries())
    };
    let live_roles: Vec<&RoleSession> = roles
        .iter()
        .filter(|r| spend.is_live(&r.session_id) != Some(false))
        .collect();
    let workers = live_roles.iter().filter(|r| !r.lead).count();
    let leads = live_roles.len() - workers;
    let hardware_cap = capacity::memory_facts(&host.sample, &limits).map(|m| {
        host::hardware_cap(
            live_roles.len(),
            host::headroom(m.available, m.reserve, m.estimate),
        )
    });
    let accounts = state
        .accounts
        .accounts()
        .into_iter()
        .map(|a| {
            let usage = crate::claude::usage::current_for(state, &a.id);
            let windows: Vec<PlanWindow> = usage
                .windows
                .iter()
                .filter(|w| WINDOW_KEYS.contains(&w.key.as_str()))
                .cloned()
                .collect();
            let five = capacity::five_hour_of(state, &a.id).map(|(p, _)| p);
            let opus = super::router::opus_weekly_of(state, &a.id).map(|(p, _)| p);
            let rate = state
                .scheduler
                .newest_rate(&a.id)
                .filter(|n| capacity::rejection_holds(n, now) || capacity::warning_holds(n, now));
            AccountSignals {
                billing: budget::billing(state, &a.id),
                plan_usage: usage.available && !windows.is_empty(),
                windows,
                lead_only: five.is_some_and(|p| p >= limits.lead_only_pct),
                stopped: five.is_some_and(|p| p >= limits.stop_pct),
                opus_gated: opus.is_some_and(|p| p >= limits.opus_gate_pct),
                rate_limit: rate,
                id: a.id,
                name: a.name,
            }
        })
        .collect();
    let rss = |id: &str| {
        host.sample
            .sessions
            .iter()
            .find(|s| s.session_id == id)
            .map(|s| s.rss_bytes)
    };
    let running = roles
        .iter()
        .map(|r| RunningView {
            live: spend.is_live(&r.session_id),
            spent_usd: spend
                .session_cost(&r.session_id)
                .map(|c| ((c - r.baseline_usd.unwrap_or(0.0)).max(0.0) * 100.0).round() / 100.0),
            rss_bytes: rss(&r.session_id),
            session: r.clone(),
        })
        .collect();
    let ready = classify::haiku_ready(state);
    CapacityView {
        now,
        caps: CapsView {
            user_cap: limits.user_cap,
            project_cap: limits.project_cap,
            workers_running: workers,
            leads_running: leads,
            hardware_cap,
        },
        host,
        accounts,
        rate_limits: lock(&state.scheduler.rate).all(),
        running,
        queue,
        budgets: budget::states(state),
        routes: lock(&state.scheduler.routes).iter().cloned().collect(),
        classifier: ClassifierView {
            enabled: limits.haiku_classifier,
            ready: ready.is_ok(),
            why_not: ready.err().filter(|_| limits.haiku_classifier),
            model: classify::HAIKU_MODEL,
            stats: classify::stats(state),
        },
        settings: limits,
        can_manage: ctx.is_desktop() || ctx.device_id().is_some(),
        can_edit_policy: ctx.is_desktop(),
    }
}

async fn get_capacity(Authed(ctx): Authed, State(state): State<AppState>) -> Json<CapacityView> {
    Json(capacity_view(&state, &ctx))
}

/// Settings a paired device may not change.
const DESKTOP_KEYS: [&str; 2] = ["haikuClassifier", "billing"];

async fn put_settings(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(patch): Body<Value>,
) -> ApiResult<Json<CapacityView>> {
    if !ctx.is_desktop() {
        if let Some(k) = patch
            .as_object()
            .and_then(|o| o.keys().find(|k| DESKTOP_KEYS.contains(&k.as_str())))
        {
            return Err(ApiError::forbidden(
                "desktop_only",
                format!("{k} can be changed on the Mother Claude desktop app only."),
            )
            .with("field", k.clone()));
        }
    }
    let current = state.scheduler.limits();
    let next = current
        .patched(&patch)
        .map_err(|e| invalid(e, "invalid_settings"))?;
    if next != current {
        state.scheduler.set_limits(next.clone());
        note_device_action(&state, &ctx, &settings_change(&current, &next));
    }
    Ok(Json(capacity_view(&state, &ctx)))
}

/// What changed, for the desktop's notice about a device.
fn settings_change(before: &Limits, after: &Limits) -> String {
    let mut parts = Vec::new();
    if before.user_cap != after.user_cap {
        parts.push(format!("the role session limit to {}", after.user_cap));
    }
    if before.budgets != after.budgets {
        parts.push("the budgets".to_string());
    }
    if before.project_cap != after.project_cap || before.project_caps != after.project_caps {
        parts.push("the project limits".to_string());
    }
    if parts.is_empty() {
        "changed the capacity settings".into()
    } else {
        format!("changed {}", parts.join(", "))
    }
}

// ─────────────────────────────── policy ───────────────────────────────────────

/// A class and its name in words.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassInfo {
    pub class: &'static str,
    pub label: &'static str,
}

/// `GET /api/routing/policy`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyView {
    pub policy: Policy,
    pub defaults: Policy,
    pub is_default: bool,
    pub classes: Vec<ClassInfo>,
    pub models: [&'static str; 4],
    pub efforts: [&'static str; 5],
    /// Task classes carrying an escalation now.
    pub learned: Vec<ClassView>,
    /// Recent escalations and steps down.
    pub escalations: Vec<EscalationNote>,
    pub can_edit: bool,
}

fn policy_view(state: &AppState, ctx: &ClientContext) -> PolicyView {
    let learned = lock(&state.scheduler.learned);
    PolicyView {
        policy: state.scheduler.policy(),
        defaults: Policy::default(),
        is_default: state.scheduler.policy_is_default(),
        classes: policy::CLASSES
            .iter()
            .map(|c| ClassInfo {
                class: policy::class_name(*c),
                label: policy::class_label(*c),
            })
            .collect(),
        models: policy::ALIASES,
        efforts: policy::EFFORTS,
        learned: learned.classes(),
        escalations: learned.notes(),
        can_edit: ctx.is_desktop(),
    }
}

async fn get_policy(Authed(ctx): Authed, State(state): State<AppState>) -> Json<PolicyView> {
    Json(policy_view(&state, &ctx))
}

async fn put_policy(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<Value>,
) -> ApiResult<Json<PolicyView>> {
    let policy = Policy::parse(&body).map_err(|e| invalid(e, "invalid_policy"))?;
    state.scheduler.set_policy(Some(policy));
    Ok(Json(policy_view(&state, &ctx)))
}

async fn reset_policy(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
) -> Json<PolicyView> {
    state.scheduler.set_policy(None);
    Json(policy_view(&state, &ctx))
}

// ─────────────────────────────── reports ──────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReportQuery {
    pub days: Option<u32>,
    pub project: Option<String>,
    /// CSV: `groups` (default) or `outcomes`.
    pub rows: Option<String>,
}

fn period(q: &ReportQuery) -> ApiResult<(u32, i64, Option<String>)> {
    let days = q.days.unwrap_or(30);
    if !(1..=365).contains(&days) {
        return Err(
            ApiError::unprocessable("invalid_days", "`days` is 1 to 365.").with("field", "days"),
        );
    }
    let project =
        match q
            .project
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            Some(p) => Some(super::limits::clean_project(p).map_err(|m| {
                ApiError::unprocessable("invalid_project", m).with("field", "project")
            })?),
            None => None,
        };
    let since = now_ms() - i64::from(days) * 24 * 60 * 60_000;
    Ok((days, since, project))
}

pub fn report(state: &AppState, days: u32, since: i64, project: Option<String>) -> Report {
    let records = state.scheduler.outcomes.since(since);
    let (rows, classes, totals, pending) = reports::build(&records, since, project.as_deref());
    let learned = lock(&state.scheduler.learned);
    Report {
        generated_at: now_ms(),
        since,
        days,
        rows,
        classes,
        totals,
        escalations: learned
            .notes()
            .into_iter()
            .filter(|n| n.at >= since)
            .filter(|n| {
                project
                    .as_deref()
                    .is_none_or(|p| crate::claude::paths::is_within_recorded(&n.project, p))
            })
            .collect(),
        learned: learned.classes(),
        classifier: classify::stats(state),
        pending,
        estimate_note: reports::ESTIMATE_NOTE,
        project,
    }
}

async fn get_report(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Query(q): Query<ReportQuery>,
) -> ApiResult<Json<Report>> {
    let (days, since, project) = period(&q)?;
    Ok(Json(report(&state, days, since, project)))
}

async fn get_report_csv(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Query(q): Query<ReportQuery>,
) -> ApiResult<Response> {
    let (days, since, project) = period(&q)?;
    let outcomes = match q.rows.as_deref() {
        None | Some("groups") => false,
        Some("outcomes") => true,
        Some(_) => {
            return Err(ApiError::unprocessable(
                "invalid_rows",
                "`rows` is `groups` or `outcomes`.",
            )
            .with("field", "rows"))
        }
    };
    let records = state.scheduler.outcomes.since(since);
    let (csv, name) = if outcomes {
        (
            reports::outcomes_csv(&records, since, project.as_deref()),
            format!("mother-claude-task-outcomes-{days}d.csv"),
        )
    } else {
        let (rows, classes, totals, _) = reports::build(&records, since, project.as_deref());
        (
            reports::rows_csv(&rows, &classes, &totals),
            format!("mother-claude-routing-{days}d.csv"),
        )
    };
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        csv,
    )
        .into_response())
}

// ─────────────────────────────── classify ─────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClassifyBody {
    pub title: String,
    pub goal: String,
    /// Ask Haiku (when switched on) instead of the keyword rules.
    pub haiku: bool,
}

const TEXT_MAX: usize = 4_000;

async fn post_classify(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Body(body): Body<ClassifyBody>,
) -> ApiResult<Json<Classification>> {
    if body.title.trim().is_empty() && body.goal.trim().is_empty() {
        return Err(
            ApiError::unprocessable("invalid_task", "Give the task a title or a goal.")
                .with("field", "title"),
        );
    }
    if body.title.chars().count() > TEXT_MAX || body.goal.chars().count() > TEXT_MAX {
        return Err(ApiError::unprocessable(
            "invalid_task",
            "The title and the goal are up to 4,000 characters each.",
        ));
    }
    if !body.haiku {
        return Ok(Json(classify::classify(&body.title, &body.goal)));
    }
    if !(ctx.is_desktop() || ctx.device_id().is_some()) {
        return Err(ApiError::forbidden(
            "desktop_only",
            "Asking Haiku spends tokens: the desktop app or a paired device only.",
        ));
    }
    if let Err(why) = classify::haiku_ready(&state) {
        return Err(ApiError::conflict("classifier_unavailable", why));
    }
    Ok(Json(
        classify::classify_task(&state, &body.title, &body.goal).await,
    ))
}

#[cfg(test)]
mod tests;
