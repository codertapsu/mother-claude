//! The claude.ai plan's usage limits — 5-hour session and weekly windows —
//! for the dashboard (design2 §3, verified against CLI 2.1.284).
//!
//! **Sources**, in priority order:
//! 1. *Passive*: an owned session's `rate_limit_event`
//!    (`rate_limit_info.unifiedWindows.{five_hour,seven_day}` — utilization a
//!    0–1 fraction, `resetsAt` epoch seconds), emitted for free when a
//!    window's rounded percentage or reset time moves. It covers those two
//!    windows only: merged into the last full read, which keeps its
//!    `fetchedAt` and source (`streamAt` says when the two were streamed);
//!    source `stream` only while there is no full read at all.
//! 2. *Active*: the experimental `{"subtype":"get_usage","skip_behaviors":true}`
//!    control request (one `GET /api/oauth/usage` by the CLI, no tokens,
//!    ~0.7 s) through a live, initialised Path B owned session, else through
//!    a short-lived helper: the installed `claude -p` in stream-json mode with
//!    no tools, no MCP, project settings only and no session persistence,
//!    run in `<app data dir>/usage-helper` (0700; the live roster ignores
//!    that directory), with the scrubbed child environment. It gets
//!    `initialize`, then `get_usage`, and is terminated right after the answer
//!    (SIGTERM, so it removes its own pidfile; it never gets a user message).
//!    Running it makes Claude Code itself write under `~/.claude` — its
//!    pidfile in `sessions/` (removed on exit; [`shutdown`] waits for that
//!    when the app quits mid-read) and its usage and feature caches in
//!    `~/.claude.json` — which this code never does. With reads set to
//!    manual only, the helper starts only for a Refresh the user asks for.
//!    An owned session counts only when its sign-in is the machine's plan
//!    (`initialize.account`: no third-party provider, no API key, the
//!    machine's email — a project's `env` or `apiKeyHelper` can change it),
//!    for reads and stream updates alike; one that answers without plan
//!    usage is not believed alone: the helper, on the machine's own sign-in,
//!    decides.
//! 3. *Fallback*: the CLI's own snapshot in `~/.claude.json`
//!    (`cachedUsageUtilization`) when at most an hour old and written for
//!    the signed-in account (`oauthAccount.accountUuid`, compared in-process
//!    and dropped: never stored, served or logged). Read at startup, after a
//!    failed read, and whenever it is newer than the last full read. Source
//!    `cache`.
//!
//! Everything is whitelisted into [`PlanUsage`]: window percentages and
//! reset times, severities, and extra usage. Codename windows, `spend`, the
//! breakdown and account data never leave this module.
//!
//! **Scheduler** ([`decide`], a pure function with an injected clock): no
//! scheduled reads while no WebSocket client is connected (a manual refresh
//! reads anyway); a read on the first connect when the full set is stale;
//! every interval (default 5 min, 3/5/10 min or manual only,
//! `<app data dir>/usage.json`) while some session was active since the last
//! full read and no fresher stream update covers it; every 30 min when idle;
//! once on a plan-limit hit (a transcript `quotaLimits` record, a
//! `StopFailure` hook with `rate_limit`, a rejected `rate_limit_event`) and
//! on a manual refresh. Never two reads within 60 s; failures back off
//! 5 → 10 → 20 → 40 → 60 min (a limit hit and a manual refresh still read).
//! An account without plan usage (`rate_limits_available: false`) is
//! re-checked at most hourly or when the sign-in changes — limit hits
//! included.
//!
//! AppState field: `state.usage`; the scheduler is started by [`start`].

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Notify;

use super::control::{protocol, AccountFacts};
use super::index::loose::{LStr, Obj, LF64};
use super::model::{
    ExtraUsage, PlanSeverity, PlanUsage, PlanUsageSource, PlanWindow, SessionStatus,
};
use crate::state::AppState;

/// Refresh intervals the settings accept (seconds; 0 = manual only).
pub const ALLOWED_INTERVALS: [u32; 4] = [0, 180, 300, 600];
/// The default refresh interval (seconds).
pub const DEFAULT_INTERVAL: u32 = 300;
/// Never two active reads closer than this (the CLI answers repeat reads
/// within 60 s from its own snapshot anyway).
pub const MIN_GAP_MS: i64 = 60_000;
/// While everything is idle, other surfaces (claude.ai, other devices) can
/// still move the usage: read at least this often.
pub const IDLE_READ_MS: i64 = 30 * 60_000;
/// How often an account without plan usage is re-checked.
pub const UNAVAILABLE_RECHECK_MS: i64 = 60 * 60_000;
/// The CLI's own snapshot is trusted this long (its `o0n`).
pub const CACHE_MAX_AGE_MS: i64 = 60 * 60_000;
/// Failure back-off steps.
pub const BACKOFF_MS: [i64; 5] = [
    5 * 60_000,
    10 * 60_000,
    20 * 60_000,
    40 * 60_000,
    60 * 60_000,
];
/// A limit-hit signal nobody could act on (no client connected) expires.
const LIMIT_HIT_TTL_MS: i64 = 10 * 60_000;
/// How long `get_usage` may take (VS Code's value).
pub const USAGE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the helper may take to boot and answer `initialize`.
const HELPER_INIT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a whole helper read may take.
const HELPER_TIMEOUT: Duration = Duration::from_secs(50);
/// Scheduler tick.
const TICK: Duration = Duration::from_secs(30);

/// The helper's argv (without the binary): print mode speaking stream-json,
/// no tools, no MCP, project settings only (an empty directory), nothing
/// persisted. It never gets a user message, so no model call is made.
pub const HELPER_ARGS: [&str; 16] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--model",
    "haiku",
    "--setting-sources",
    "project",
    "--no-session-persistence",
    "--strict-mcp-config",
    "--tools",
    "",
    "--permission-mode",
    "default",
];

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// `<app data dir>/usage-helper`: the helper's working directory.
pub fn helper_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("usage-helper")
}

// ─────────────────────────────── parsing ───────────────────────────────

/// What a `get_usage` answer says.
#[derive(Debug, Clone, PartialEq)]
pub enum UsageAnswer {
    /// No claude.ai plan usage for this sign-in (API key, third-party
    /// provider, a token without the profile scope).
    Unavailable,
    Data {
        windows: Vec<PlanWindow>,
        extra_usage: Option<ExtraUsage>,
    },
}

/// Plan usage as one source produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageData {
    pub windows: Vec<PlanWindow>,
    pub extra_usage: Option<ExtraUsage>,
    /// Epoch ms the data was produced.
    pub fetched_at: i64,
    pub source: PlanUsageSource,
}

/// The windows a `rate_limit_event` reports: `(key, percent, resets at ms)`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamUpdate {
    pub windows: Vec<(String, f64, Option<i64>)>,
    /// The request was rejected by a limit.
    pub rejected: bool,
}

/// An ISO-8601 time (`2026-09-29T14:00:00.123456+00:00`) as epoch ms.
fn iso_ms(v: Option<&Value>) -> Option<i64> {
    v.and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s.trim()).ok())
        .map(|d| d.timestamp_millis())
}

/// Epoch seconds (or ms, when already that large) as epoch ms.
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

/// A sane percentage (0–1000, one decimal).
fn pct_of(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .map(|n| (n.clamp(0.0, 1000.0) * 10.0).round() / 10.0)
}

fn severity_of(v: Option<&Value>) -> Option<PlanSeverity> {
    match v.and_then(Value::as_str) {
        Some("normal") => Some(PlanSeverity::Normal),
        Some("warning") => Some(PlanSeverity::Warning),
        Some("critical") => Some(PlanSeverity::Critical),
        _ => None,
    }
}

/// A short display name from the CLI (a model or surface name).
fn display_name(v: Option<&Value>) -> Option<String> {
    let s = v.and_then(Value::as_str)?.trim();
    let ok = !s.is_empty()
        && s.chars().count() <= 40
        && s.chars()
            .all(|c| c.is_alphanumeric() || " .-_()".contains(c));
    ok.then(|| s.to_string())
}

/// `Weekly · Fable` → `fable` (keys are `[a-z0-9_]`).
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').chars().take(40).collect()
}

/// `weekly_all` → `Weekly all`.
fn humanize(kind: &str) -> String {
    let words = kind.replace(['_', '-'], " ");
    let mut c = words.trim().chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// The label of a window key (stream updates only carry the key).
fn label_for(key: &str) -> String {
    match key {
        "five_hour" => "Current session".to_string(),
        "seven_day" => "Weekly · all models".to_string(),
        "seven_day_opus" => "Weekly · Opus".to_string(),
        "seven_day_sonnet" => "Weekly · Sonnet".to_string(),
        "seven_day_overage_included" => "Weekly · with extra usage".to_string(),
        "overage" => "Extra usage".to_string(),
        other => humanize(other),
    }
}

/// Display order: the session window, the weekly one, then the rest.
fn rank(key: &str) -> u8 {
    match key {
        "five_hour" => 0,
        "seven_day" => 1,
        _ => 2,
    }
}

fn sort_windows(windows: &mut [PlanWindow]) {
    windows.sort_by_key(|w| rank(&w.key));
}

/// One `limits[]` row (`{kind, group, percent, severity, resets_at, scope,
/// is_active}`) as a window. Rows the CLI marks inactive at 0 % are skipped.
fn window_from_row(row: &Map<String, Value>) -> Option<PlanWindow> {
    let kind = row.get("kind").and_then(Value::as_str)?.trim();
    if kind.is_empty() || kind.len() > 40 {
        return None;
    }
    let pct = pct_of(row.get("percent"))?;
    let inactive = row.get("is_active").and_then(Value::as_bool) == Some(false);
    if inactive && pct == 0.0 {
        return None;
    }
    let scope = row.get("scope").and_then(Value::as_object);
    let scoped_name = scope.and_then(|s| {
        display_name(s.get("model").and_then(|m| m.get("display_name")))
            .or_else(|| display_name(s.get("surface").and_then(|m| m.get("display_name"))))
    });
    let (key, label) = match (kind, scoped_name) {
        ("session", _) => ("five_hour".to_string(), "Current session".to_string()),
        ("weekly_all", _) => ("seven_day".to_string(), "Weekly · all models".to_string()),
        ("weekly_scoped", Some(name)) => (
            format!("seven_day_{}", slug(&name)),
            format!("Weekly · {name}"),
        ),
        ("weekly_scoped", None) => (
            "seven_day_scoped".to_string(),
            "Weekly · scoped".to_string(),
        ),
        (other, name) => {
            let base = slug(other);
            match name {
                Some(n) => (
                    format!("{base}_{}", slug(&n)),
                    format!("{} · {n}", humanize(other)),
                ),
                None => (base, humanize(other)),
            }
        }
    };
    if key.is_empty() {
        return None;
    }
    Some(PlanWindow {
        key,
        label,
        pct,
        resets_at: iso_ms(row.get("resets_at")),
        severity: severity_of(row.get("severity")),
    })
}

/// The windows of a `rate_limits` object (a `get_usage` answer or the CLI's
/// cache): its `limits[]` rows when present, else the documented windows.
/// Codename keys, `spend` and the breakdown are never looked at.
pub fn windows_from_rate_limits(rl: &Map<String, Value>) -> Vec<PlanWindow> {
    let mut out: Vec<PlanWindow> = Vec::new();
    let rows = rl.get("limits").and_then(Value::as_array);
    if let Some(rows) = rows.filter(|r| !r.is_empty()) {
        for w in rows
            .iter()
            .filter_map(Value::as_object)
            .filter_map(window_from_row)
        {
            if !out.iter().any(|o| o.key == w.key) {
                out.push(w);
            }
        }
    } else {
        for key in [
            "five_hour",
            "seven_day",
            "seven_day_opus",
            "seven_day_sonnet",
            "seven_day_overage_included",
            "overage",
        ] {
            let Some(w) = rl.get(key).and_then(Value::as_object) else {
                continue;
            };
            let Some(pct) = pct_of(w.get("utilization")) else {
                continue;
            };
            out.push(PlanWindow {
                key: key.to_string(),
                label: label_for(key),
                pct,
                resets_at: iso_ms(w.get("resets_at")),
                severity: None,
            });
        }
    }
    sort_windows(&mut out);
    out
}

/// `rate_limits.extra_usage` (`{is_enabled, monthly_limit, used_credits,
/// utilization, currency, decimal_places, …}`, amounts in minor units).
pub fn extra_usage_from(v: Option<&Value>) -> Option<ExtraUsage> {
    let e = v?.as_object()?;
    let enabled = e.get("is_enabled").and_then(Value::as_bool)?;
    let minor = |k: &str| {
        e.get(k)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n >= 0.0 && *n < 1e15)
            .map(|n| n.round() as i64)
    };
    // An ISO 4217 code or nothing: amounts are never labelled with a
    // currency the CLI did not send.
    let currency = e
        .get("currency")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| c.len() == 3 && c.chars().all(|x| x.is_ascii_alphabetic()))
        .map(str::to_ascii_uppercase);
    let decimals = e
        .get("decimal_places")
        .and_then(Value::as_u64)
        .filter(|d| *d <= 6)
        .unwrap_or(2) as u32;
    Some(ExtraUsage {
        enabled,
        used_minor: minor("used_credits").unwrap_or(0),
        limit_minor: minor("monthly_limit"),
        currency,
        decimals,
        utilization: pct_of(e.get("utilization")),
    })
}

/// Parse a `get_usage` answer (`{session, subscription_type,
/// rate_limits_available, rate_limits, behaviors}`). `Err` when the CLI
/// says limits exist but sent none (a failed fetch).
pub fn parse_get_usage(resp: &Value) -> Result<UsageAnswer, String> {
    if resp.get("rate_limits_available").and_then(Value::as_bool) == Some(false) {
        return Ok(UsageAnswer::Unavailable);
    }
    let Some(rl) = resp.get("rate_limits").and_then(Value::as_object) else {
        return Err("Claude Code could not read the plan usage right now.".to_string());
    };
    Ok(UsageAnswer::Data {
        windows: windows_from_rate_limits(rl),
        extra_usage: extra_usage_from(rl.get("extra_usage")),
    })
}

/// Parse an owned session's `rate_limit_event` line.
pub fn parse_rate_limit_event(v: &Value) -> Option<StreamUpdate> {
    if v.get("type").and_then(Value::as_str) != Some("rate_limit_event") {
        return None;
    }
    let info = v.get("rate_limit_info")?.as_object()?;
    let mut update = StreamUpdate {
        rejected: info.get("status").and_then(Value::as_str) == Some("rejected"),
        ..Default::default()
    };
    if let Some(uw) = info.get("unifiedWindows").and_then(Value::as_object) {
        for key in ["five_hour", "seven_day"] {
            let Some(w) = uw.get(key).and_then(Value::as_object) else {
                continue;
            };
            let Some(fraction) = w
                .get("utilization")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= 0.0)
            else {
                continue;
            };
            let pct = (fraction.min(10.0) * 1000.0).round() / 10.0;
            update
                .windows
                .push((key.to_string(), pct, epoch_ms(w.get("resetsAt"))));
        }
    }
    (!update.windows.is_empty() || update.rejected).then_some(update)
}

// ──────────────────────────── the CLI's cache ────────────────────────────

/// The only parts of `~/.claude.json` read here. No `Debug`: the account id
/// must never reach a log.
#[derive(Default, Deserialize)]
#[serde(default)]
struct CacheRoot {
    #[serde(rename = "cachedUsageUtilization")]
    cached: Obj<CachedUsage>,
    #[serde(rename = "oauthAccount")]
    account: Obj<AccountRef>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CachedUsage {
    #[serde(rename = "fetchedAtMs")]
    fetched_at_ms: LF64,
    #[serde(rename = "accountUuid")]
    account_uuid: LStr,
    utilization: Option<Value>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct AccountRef {
    #[serde(rename = "accountUuid")]
    account_uuid: LStr,
}

/// The CLI's own usage snapshot from `~/.claude.json` text: at most an hour
/// old at `now`, written for the account signed in now. The account ids are
/// compared here and dropped.
pub fn parse_cache(text: &str, now: i64) -> Option<UsageData> {
    let root: CacheRoot = serde_json::from_str(text).ok()?;
    let cached = root.cached.0?;
    let theirs = cached.account_uuid.get()?;
    let ours = root.account.get().and_then(|a| a.account_uuid.get())?;
    if theirs.is_empty() || theirs != ours {
        return None;
    }
    let fetched_at = cached
        .fetched_at_ms
        .0
        .filter(|t| t.is_finite() && *t > 0.0)? as i64;
    if now - fetched_at > CACHE_MAX_AGE_MS || fetched_at - now > MIN_GAP_MS {
        return None;
    }
    let rl = cached.utilization.as_ref()?.as_object()?;
    let windows = windows_from_rate_limits(rl);
    if windows.is_empty() {
        return None;
    }
    Some(UsageData {
        windows,
        extra_usage: extra_usage_from(rl.get("extra_usage")),
        fetched_at,
        source: PlanUsageSource::Cache,
    })
}

// ─────────────────────────────── scheduling ───────────────────────────────

/// Why an active read happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Manual,
    LimitHit,
    FirstConnect,
    Periodic,
    Idle,
    Recheck,
}

/// What [`decide`] looks at (all times epoch ms).
#[derive(Debug, Clone, Copy, Default)]
pub struct Inputs {
    pub now: i64,
    /// The refresh interval (0 = manual only).
    pub interval_ms: i64,
    /// Connected WebSocket clients.
    pub clients: usize,
    /// A client connected while none was.
    pub first_connect: bool,
    /// A manual refresh was asked for.
    pub manual: bool,
    /// A plan limit was hit since the last read.
    pub limit_hit: bool,
    /// Start of the last active read.
    pub last_read: Option<i64>,
    /// When the last full data (a live read or the CLI's snapshot) was made.
    /// A stream update covers only two windows, so it never counts here.
    pub full_at: Option<i64>,
    /// The last stream update.
    pub stream_at: Option<i64>,
    /// Some session worked, waited or wrote since `full_at`.
    pub activity: bool,
    /// Failure back-off.
    pub backoff_until: Option<i64>,
    /// The account has no plan usage (as of the last read or sign-in check).
    pub unavailable: bool,
    /// When that was established.
    pub unavailable_since: Option<i64>,
    /// The sign-in changed since the last check.
    pub auth_changed: bool,
}

/// Whether to read now, and why. Pure; the table is in the module docs.
pub fn decide(i: &Inputs) -> Option<Reason> {
    let since_read = i.last_read.map(|t| i.now - t);
    if since_read.is_some_and(|d| d < MIN_GAP_MS) {
        return None;
    }
    if i.manual {
        return Some(Reason::Manual);
    }
    if i.clients == 0 {
        return None;
    }
    // A limit hit skips the failure back-off — for a plan's limits only: an
    // account without plan usage (throttled API-key or third-party requests
    // raise the same signals) waits for its hourly re-check.
    if i.limit_hit && !i.unavailable {
        return Some(Reason::LimitHit);
    }
    if i.backoff_until.is_some_and(|t| i.now < t) {
        return None;
    }
    if i.unavailable {
        let due = i.auth_changed
            || i.unavailable_since
                .is_none_or(|t| i.now - t >= UNAVAILABLE_RECHECK_MS);
        return due.then_some(Reason::Recheck);
    }
    let stale_after = if i.interval_ms > 0 {
        i.interval_ms
    } else {
        i64::from(DEFAULT_INTERVAL) * 1000
    };
    if i.first_connect && i.full_at.is_none_or(|t| i.now - t >= stale_after) {
        return Some(Reason::FirstConnect);
    }
    if i.interval_ms <= 0 {
        return None;
    }
    let full_age = i.full_at.map(|t| i.now - t);
    if full_age.is_none_or(|a| a >= IDLE_READ_MS) {
        return Some(Reason::Idle);
    }
    let stream_fresh = i.stream_at.is_some_and(|t| i.now - t < i.interval_ms);
    let due = full_age.is_some_and(|a| a >= i.interval_ms);
    (due && i.activity && !stream_fresh).then_some(Reason::Periodic)
}

/// When the next scheduled read is due as things stand, if one is: the
/// 30-minute read while nothing is active, the interval's (pushed out by a
/// fresher stream update, never past the 30-minute one) while something is.
/// `None` when reads are manual only, nobody is connected, or the account
/// has no plan usage.
pub fn next_due(i: &Inputs) -> Option<i64> {
    if i.interval_ms <= 0 || i.clients == 0 || i.unavailable {
        return None;
    }
    let floor = [
        i.last_read.map(|t| t + MIN_GAP_MS),
        i.backoff_until,
        Some(i.now),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(i.now);
    let Some(full) = i.full_at else {
        return Some(floor);
    };
    let idle = full + IDLE_READ_MS;
    if !i.activity {
        return Some(idle.max(floor));
    }
    let periodic = (full + i.interval_ms).max(i.stream_at.map_or(0, |s| s + i.interval_ms));
    Some(periodic.min(idle).max(floor))
}

// ──────────────────────────────── the store ────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct UsageSettings {
    interval_secs: u32,
}

#[derive(Debug, Default)]
struct Inner {
    interval_secs: u32,
    data: Option<UsageData>,
    full_at: Option<i64>,
    stream_at: Option<i64>,
    /// The windows the stream last reported, re-applied over an older
    /// snapshot adopted later.
    stream_windows: Vec<PlanWindow>,
    /// [`email_key`] of the machine's sign-in (`claude auth status`), to
    /// tell an owned session on other credentials apart.
    account_key: Option<u64>,
    unavailable: bool,
    unavailable_since: Option<i64>,
    refreshing: bool,
    last_read: Option<i64>,
    failures: u32,
    backoff_until: Option<i64>,
    error: Option<String>,
    manual: bool,
    /// When the last manual refresh was asked for.
    manual_at: Option<i64>,
    limit_hit: Option<i64>,
    first_connect: bool,
    next_refresh_at: Option<i64>,
    /// Fingerprint of the sign-in at the last read (hash, never the values).
    auth_key: Option<u64>,
    /// `~/.claude.json` mtime at the last snapshot read.
    cache_mtime: Option<i64>,
    /// Newest transcript `quotaLimits` already acted on.
    quota_seen: i64,
    last_published: Option<PlanUsage>,
}

/// Plan-usage state and the scheduler's bookkeeping. AppState: `state.usage`.
#[derive(Debug)]
pub struct UsageStore {
    inner: Mutex<Inner>,
    wake: Notify,
    settings_path: Option<PathBuf>,
    helper_dir: Option<PathBuf>,
}

impl UsageStore {
    /// Load the interval from `<data_dir>/usage.json` (`None`: in memory).
    pub fn new(data_dir: Option<&Path>) -> Self {
        let settings_path = data_dir.map(|d| d.join("usage.json"));
        let interval_secs = settings_path
            .as_deref()
            .and_then(crate::persist::read_json::<UsageSettings>)
            .map(|s| s.interval_secs)
            .filter(|s| ALLOWED_INTERVALS.contains(s))
            .unwrap_or(DEFAULT_INTERVAL);
        Self {
            inner: Mutex::new(Inner {
                interval_secs,
                quota_seen: now_ms(),
                ..Default::default()
            }),
            wake: Notify::new(),
            settings_path,
            helper_dir: data_dir.map(helper_dir),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The configured interval (seconds; 0 = manual only).
    pub fn interval_secs(&self) -> u32 {
        self.lock().interval_secs
    }

    /// What clients see now. A window whose reset time has passed shows 0 %.
    pub fn snapshot(&self, now: i64) -> PlanUsage {
        snapshot_of(&self.lock(), now)
    }

    /// A plan-limit hit waits for the next read.
    pub fn limit_hit_pending(&self) -> bool {
        self.lock().limit_hit.is_some()
    }

    /// Seed the store (tests and tools).
    pub fn set_data_for_test(&self, data: UsageData) {
        let mut g = self.lock();
        g.full_at = Some(data.fetched_at);
        g.data = Some(data);
    }
}

fn snapshot_of(g: &Inner, now: i64) -> PlanUsage {
    let data = g.data.as_ref().filter(|_| !g.unavailable);
    PlanUsage {
        available: !g.unavailable,
        windows: data
            .map(|d| {
                d.windows
                    .iter()
                    .cloned()
                    .map(|mut w| {
                        if w.resets_at.is_some_and(|r| r <= now) {
                            w.pct = 0.0;
                            w.severity = None;
                        }
                        w
                    })
                    .collect()
            })
            .unwrap_or_default(),
        extra_usage: data.and_then(|d| d.extra_usage.clone()),
        fetched_at: data.map(|d| d.fetched_at),
        source: data.map(|d| d.source),
        // Only when the stream is newer than the rest of the data.
        stream_at: data.and_then(|d| g.stream_at.filter(|s| *s > d.fetched_at)),
        refreshing: g.refreshing,
        next_refresh_at: g.next_refresh_at,
        interval_secs: g.interval_secs,
        error: g.error.clone(),
    }
}

/// Publish a `planUsage` frame when what clients see changed. Several tasks
/// publish (the scheduler, owned stdout readers, HTTP handlers), so the
/// frame goes out under the same lock that records it: frames leave in the
/// order `last_published` changes, and the last one clients get is the one
/// recorded (the send never blocks and never takes this lock).
fn publish(state: &AppState) {
    let mut g = state.usage.lock();
    let usage = snapshot_of(&g, now_ms());
    if g.last_published.as_ref() == Some(&usage) {
        return;
    }
    g.last_published = Some(usage.clone());
    crate::server::ws::publish_plan_usage(state, usage);
}

/// The current plan usage (`GET /api/usage`, the WS connect frame).
pub fn current(state: &AppState) -> PlanUsage {
    state.usage.snapshot(now_ms())
}

/// A WebSocket client connected (`clients` now connected): the first one
/// may trigger a read of stale data.
pub fn on_client_connected(state: &AppState, clients: usize) {
    if clients == 1 {
        state.usage.lock().first_connect = true;
        state.usage.wake.notify_one();
    }
}

/// A plan limit was hit somewhere (a `StopFailure` hook with `rate_limit`, a
/// rejected owned request): read once, at most one read per minute.
pub fn note_limit_hit(state: &AppState) {
    state.usage.lock().limit_hit = Some(now_ms());
    state.usage.wake.notify_one();
}

/// `POST /api/usage/refresh`: `Err(seconds to wait)` within a minute of the
/// last read (or of the last refresh asked for), else the usage with
/// `refreshing: true` (the result arrives as a `planUsage` frame).
pub fn request_refresh(state: &AppState) -> Result<PlanUsage, u64> {
    let now = now_ms();
    let usage = {
        let mut g = state.usage.lock();
        let since = [g.last_read, g.manual_at].into_iter().flatten().max();
        if let Some(wait) = since.map(|t| MIN_GAP_MS - (now - t)).filter(|w| *w > 0) {
            return Err(u64::try_from(wait).unwrap_or(0).div_ceil(1000).max(1));
        }
        g.manual_at = Some(now);
        if !g.refreshing {
            g.manual = true;
            g.refreshing = true;
        }
        snapshot_of(&g, now)
    };
    state.usage.wake.notify_one();
    publish(state);
    Ok(usage)
}

impl UsageStore {
    /// Set the refresh interval: `Err` for a value outside
    /// [`ALLOWED_INTERVALS`]. Persisted to `<app data dir>/usage.json`.
    pub fn set_interval(&self, secs: u32) -> Result<(), String> {
        if !ALLOWED_INTERVALS.contains(&secs) {
            return Err(format!(
                "`intervalSecs` must be one of {ALLOWED_INTERVALS:?} (0 = manual only)."
            ));
        }
        if let Some(path) = &self.settings_path {
            if let Err(e) = crate::persist::write_json_private(
                path,
                &UsageSettings {
                    interval_secs: secs,
                },
            ) {
                tracing::warn!(error = %e, "could not save the plan-usage interval");
            }
        }
        self.lock().interval_secs = secs;
        self.wake.notify_one();
        Ok(())
    }
}

/// `PUT /api/usage/settings`: the new interval, published to everyone.
pub fn set_interval(state: &AppState, secs: u32) -> Result<PlanUsage, String> {
    state.usage.set_interval(secs)?;
    publish(state);
    Ok(current(state))
}

/// Put streamed `(key, percent, resets at)` windows into `windows`.
fn apply_stream_windows(windows: &mut Vec<PlanWindow>, streamed: &[(String, f64, Option<i64>)]) {
    for (key, pct, resets_at) in streamed {
        match windows.iter_mut().find(|w| &w.key == key) {
            Some(w) => {
                if w.pct != *pct {
                    // The CLI's severity went with the old percentage.
                    w.severity = None;
                }
                w.pct = *pct;
                w.resets_at = resets_at.or(w.resets_at);
            }
            None => windows.push(PlanWindow {
                label: label_for(key),
                key: key.clone(),
                pct: *pct,
                resets_at: *resets_at,
                severity: None,
            }),
        }
    }
    sort_windows(windows);
}

/// Merge an owned session's `rate_limit_event` (free, real time). It only
/// carries the 5-hour and weekly windows: the rest of a full read keeps its
/// `fetchedAt` and source, and `streamAt` tells the two apart.
pub fn merge_stream(state: &AppState, update: StreamUpdate) {
    let now = now_ms();
    {
        let mut g = state.usage.lock();
        if update.rejected {
            g.limit_hit = Some(now);
        }
        if !update.windows.is_empty() {
            // Only accounts with plan limits send these.
            g.unavailable = false;
            apply_stream_windows(&mut g.stream_windows, &update.windows);
            let data = g.data.get_or_insert_with(|| UsageData {
                windows: Vec::new(),
                extra_usage: None,
                fetched_at: now,
                source: PlanUsageSource::Stream,
            });
            apply_stream_windows(&mut data.windows, &update.windows);
            let covered = data
                .windows
                .iter()
                .all(|w| update.windows.iter().any(|(k, ..)| k == &w.key));
            if data.source == PlanUsageSource::Stream && covered {
                // Nothing but streamed windows, all in this update: all of
                // it is this fresh.
                data.fetched_at = now;
            }
            g.stream_at = Some(now);
        }
    }
    if update.rejected {
        state.usage.wake.notify_one();
    }
    publish(state);
}

/// A fingerprint of Claude's sign-in (never the values themselves).
fn auth_key(a: &crate::claude::AuthStatus) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        a.logged_in,
        &a.auth_method,
        &a.api_provider,
        &a.subscription_type,
        &a.org_name,
        &a.email,
    )
        .hash(&mut h);
    h.finish()
}

/// The sign-in has no claude.ai plan for sure: a third-party provider
/// (Bedrock, Vertex, …). Anything less certain is left to `get_usage`.
fn auth_rules_out(a: &crate::claude::AuthStatus) -> bool {
    a.error.is_none()
        && a.logged_in
        && a.api_provider
            .as_deref()
            .is_some_and(|p| !p.eq_ignore_ascii_case("firstParty"))
}

/// Start the scheduler (next to the monitor, hooks and push).
pub fn start(state: &AppState) {
    let st = state.clone();
    tokio::spawn(async move { run(st).await });
}

async fn run(state: AppState) {
    // The CLI's last-known snapshot until the first read.
    adopt_cache(&state).await;
    publish(&state);
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = state.usage.wake.notified() => {}
        }
        step(&state).await;
    }
}

/// Read the CLI's snapshot when `~/.claude.json` changed and it is newer
/// than the last full read.
async fn adopt_cache(state: &AppState) {
    let path = state.home.claude_json();
    let mtime = std::fs::metadata(&path)
        .ok()
        .map(|m| super::index::stamp_of(&m).mtime);
    let Some(mtime) = mtime else {
        return;
    };
    if state.usage.lock().cache_mtime == Some(mtime) {
        return;
    }
    let read = tokio::task::spawn_blocking(move || {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| parse_cache(&t, now_ms()))
    })
    .await
    .ok()
    .flatten();
    let mut g = state.usage.lock();
    g.cache_mtime = Some(mtime);
    if let Some(data) = read {
        adopt_snapshot(&mut g, data);
    }
}

/// Take the CLI's snapshot when it is newer than the last full read (stream
/// updates do not count: they cover two windows). Windows streamed after it
/// was taken stay as streamed.
fn adopt_snapshot(g: &mut Inner, mut data: UsageData) {
    let newer = g.full_at.is_none_or(|t| t < data.fetched_at);
    if !newer || g.unavailable {
        return;
    }
    if g.stream_at.is_some_and(|s| s > data.fetched_at) {
        let streamed: Vec<(String, f64, Option<i64>)> = g
            .stream_windows
            .iter()
            .map(|w| (w.key.clone(), w.pct, w.resets_at))
            .collect();
        apply_stream_windows(&mut data.windows, &streamed);
    }
    g.full_at = Some(data.fetched_at);
    g.data = Some(data);
}

/// The newest transcript `quotaLimits` rejection of these sessions.
fn newest_quota_hit(state: &AppState, ids: &[String]) -> Option<i64> {
    ids.iter()
        .filter_map(|id| {
            state
                .index
                .with_main(id, |f| f.agg.last_quota.as_ref().and_then(|q| q.ts))
                .flatten()
        })
        .max()
}

/// Whether some session worked, waited or wrote after `since`, and the
/// sessions active since `recent` (the only ones that can hold a new limit
/// hit).
async fn activity_since(state: &AppState, since: Option<i64>, recent: i64) -> (bool, Vec<String>) {
    let snap = state.snapshot.read().await;
    let active = snap.sessions.iter().any(|s| {
        (s.live && matches!(s.status, SessionStatus::Working | SessionStatus::Waiting))
            || since.is_some_and(|r| s.last_activity_at.is_some_and(|t| t > r))
    });
    let ids = snap
        .sessions
        .iter()
        .filter(|s| s.last_activity_at.is_some_and(|t| t >= recent))
        .map(|s| s.id.clone())
        .collect();
    (active, ids)
}

/// Note the newest transcript `quotaLimits` rejection: a new one within the
/// TTL asks for a read; a limit hit nobody acted on expires.
fn note_quota(g: &mut Inner, quota: Option<i64>, now: i64) {
    if let Some(q) = quota.filter(|q| *q > g.quota_seen) {
        g.quota_seen = q;
        if now - q < LIMIT_HIT_TTL_MS {
            g.limit_hit = Some(now);
        }
    }
    if g.limit_hit.is_some_and(|t| now - t > LIMIT_HIT_TTL_MS) {
        g.limit_hit = None;
    }
}

/// Note Claude's sign-in: a third-party provider has no plan usage for sure
/// (re-checked hourly: the sign-in is only checked at startup); any change
/// re-opens the question. Returns whether it changed since the last step.
fn note_auth(g: &mut Inner, auth: &crate::claude::AuthStatus, now: i64) -> bool {
    g.account_key = auth.email.as_deref().and_then(email_key);
    let key = auth_key(auth);
    if g.auth_key == Some(key) {
        return false;
    }
    let changed = g.auth_key.is_some();
    g.auth_key = Some(key);
    if auth_rules_out(auth) {
        g.unavailable = true;
        g.unavailable_since = Some(now);
        return false;
    }
    if changed {
        g.unavailable = false;
    }
    changed
}

/// One scheduler step: gather the inputs, decide, read.
async fn step(state: &AppState) {
    adopt_cache(state).await;
    let auth = state.claude_auth.read().await.clone();
    let full_at = state.usage.lock().full_at;
    let (activity, recent_ids) = activity_since(state, full_at, now_ms() - LIMIT_HIT_TTL_MS).await;
    let st = state.clone();
    let quota = tokio::task::spawn_blocking(move || newest_quota_hit(&st, &recent_ids))
        .await
        .ok()
        .flatten();
    let now = now_ms();
    let clients = state.hub.client_count();
    let inputs = {
        let mut g = state.usage.lock();
        note_quota(&mut g, quota, now);
        let auth_changed = note_auth(&mut g, &auth, now);
        let inputs = Inputs {
            now,
            interval_ms: i64::from(g.interval_secs) * 1000,
            clients,
            first_connect: std::mem::take(&mut g.first_connect),
            manual: g.manual,
            limit_hit: g.limit_hit.is_some(),
            last_read: g.last_read,
            full_at: g.full_at,
            stream_at: g.stream_at,
            activity,
            backoff_until: g.backoff_until,
            unavailable: g.unavailable,
            unavailable_since: g.unavailable_since,
            auth_changed,
        };
        g.next_refresh_at = next_due(&inputs);
        inputs
    };
    let decided = decide(&inputs);
    let helper = helper_allowed(decided, inputs.interval_ms);
    // Nothing to read through (manual only, no owned session): no read.
    let reason = decided.filter(|_| helper || owned_reader(state).is_some());
    match reason {
        Some(reason) => read(state, reason, helper).await,
        None => {
            if inputs.manual {
                // Asked within the floor (another read just ran): done.
                let mut g = state.usage.lock();
                g.manual = false;
                g.refreshing = false;
            }
            publish(state);
        }
    }
}

/// Whether a read may start the helper: always for a Refresh the user asked
/// for; otherwise only while reads are automatic (with "manually only",
/// automatic reads — first connect, limit hits, re-checks — use an owned
/// session or nothing, so Claude Code never runs on its own).
fn helper_allowed(reason: Option<Reason>, interval_ms: i64) -> bool {
    reason == Some(Reason::Manual) || interval_ms > 0
}

/// The owned session a read goes through: live, initialised, on the
/// machine's own plan.
fn owned_reader(state: &AppState) -> Option<String> {
    let machine = state.usage.lock().account_key;
    state
        .control
        .usage_session(|account| account_ok(account, machine))
}

/// Book a read's result. `Ok(None)`: nothing conclusive (nothing changes).
/// Returns whether it failed (the caller then falls back to the CLI's
/// snapshot).
fn apply_result(g: &mut Inner, result: Result<Option<UsageAnswer>, String>, now: i64) -> bool {
    g.refreshing = false;
    match result {
        Ok(None) => false,
        Ok(Some(UsageAnswer::Unavailable)) => {
            g.unavailable = true;
            g.unavailable_since = Some(now);
            g.data = None;
            g.error = None;
            g.failures = 0;
            g.backoff_until = None;
            false
        }
        Ok(Some(UsageAnswer::Data {
            windows,
            extra_usage,
        })) => {
            g.unavailable = false;
            g.data = Some(UsageData {
                windows,
                extra_usage,
                fetched_at: now,
                source: PlanUsageSource::Live,
            });
            g.full_at = Some(now);
            g.error = None;
            g.failures = 0;
            g.backoff_until = None;
            false
        }
        Err(message) => {
            let step = (g.failures as usize).min(BACKOFF_MS.len() - 1);
            g.failures = g.failures.saturating_add(1);
            g.backoff_until = Some(now + BACKOFF_MS[step]);
            g.error = Some(message);
            true
        }
    }
}

/// One active read and its bookkeeping.
async fn read(state: &AppState, reason: Reason, allow_helper: bool) {
    let started = now_ms();
    {
        let mut g = state.usage.lock();
        g.refreshing = true;
        g.last_read = Some(started);
        g.manual = false;
        g.limit_hit = None;
        g.next_refresh_at = None;
    }
    publish(state);
    tracing::debug!(?reason, "reading plan usage");
    let result = read_once(state, allow_helper).await;
    if let Err(message) = &result {
        tracing::info!(error = %message, "plan usage read failed");
    }
    let failed = apply_result(&mut state.usage.lock(), result, now_ms());
    if failed {
        // Fall back to the CLI's own snapshot while reads fail.
        state.usage.lock().cache_mtime = None;
        adopt_cache(state).await;
    }
    // One more step right away: it schedules the next read and publishes
    // the result together with it.
    state.usage.wake.notify_one();
}

/// `get_usage` through a live owned session on the machine's plan, else
/// through the helper (when allowed). An owned session that answers without
/// plan usage may run on other credentials (a project's own `env` or
/// `apiKeyHelper`): it is not asked again, and only the helper, on the
/// machine's own sign-in, may say the plan has none. `Ok(None)`: nothing
/// conclusive.
async fn read_once(state: &AppState, allow_helper: bool) -> Result<Option<UsageAnswer>, String> {
    let request = json!({ "subtype": "get_usage", "skip_behaviors": true });
    if let Some(id) = owned_reader(state) {
        let resp = state
            .control
            .request(&id, request, USAGE_TIMEOUT)
            .await
            .map_err(|e| short_error(&e.to_string()))?;
        match parse_get_usage(&resp)? {
            UsageAnswer::Unavailable => {
                tracing::debug!(session = %id, "owned session reports no plan usage; asking the helper");
                state.control.update(&id, |l| l.no_plan_usage = true);
            }
            data => {
                tracing::debug!(session = %id, "plan usage read through an owned session");
                return Ok(Some(data));
            }
        }
    }
    if !allow_helper {
        return Ok(None);
    }
    let dir = state
        .usage
        .helper_dir
        .clone()
        .ok_or_else(|| "No app data directory for the usage helper.".to_string())?;
    prepare_dir(&dir).map_err(|e| format!("Could not prepare the usage helper directory: {e}"))?;
    // Never listed as a session (its canonical path, now it exists).
    state.live.ignore_cwd(&dir);
    let resp = helper_read(&crate::claude::claude_bin(), &dir).await?;
    tracing::debug!("plan usage read through the helper");
    parse_get_usage(&resp).map(Some)
}

/// A fingerprint of a sign-in email (trimmed, lower-cased), to compare an
/// owned session's account with the machine's without keeping either.
pub fn email_key(email: &str) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    email.hash(&mut h);
    Some(h.finish())
}

/// Whether an owned session's credentials are the machine's plan: its
/// `initialize.account` names no third-party provider, no API key and no
/// other email than `machine` (unknown on either side: yes).
fn account_ok(account: Option<AccountFacts>, machine: Option<u64>) -> bool {
    match account {
        None => true,
        Some(a) => {
            !a.not_plan
                && match (a.email_key, machine) {
                    (Some(theirs), Some(ours)) => theirs == ours,
                    _ => true,
                }
        }
    }
}

/// [`account_ok`] against the machine's sign-in: may this owned session's
/// `rate_limit_event`s feed the plan usage?
pub fn plan_session_ok(state: &AppState, account: Option<AccountFacts>) -> bool {
    account_ok(account, state.usage.lock().account_key)
}

fn short_error(s: &str) -> String {
    let one_line = s.lines().next().unwrap_or_default().trim();
    let mut out: String = one_line.chars().take(200).collect();
    if out.is_empty() {
        out = "Plan usage could not be read.".to_string();
    }
    out
}

// ─────────────────────────────── the helper ───────────────────────────────

/// Create the helper's directory, private to the user.
fn prepare_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Helpers running right now (pids), for [`shutdown`].
static HELPERS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

fn helpers() -> MutexGuard<'static, Vec<u32>> {
    HELPERS.lock().unwrap_or_else(|p| p.into_inner())
}

/// A running helper's pid in [`HELPERS`] until the process is reaped.
struct HelperPid(Option<u32>);

impl HelperPid {
    fn register(pid: Option<u32>) -> Self {
        if let Some(pid) = pid {
            helpers().push(pid);
        }
        Self(pid)
    }
}

impl Drop for HelperPid {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            helpers().retain(|p| *p != pid);
        }
    }
}

/// How long a helper may take to exit after SIGTERM (it removes its
/// pidfile, then flushes telemetry) before it is killed.
const HELPER_EXIT_GRACE: Duration = Duration::from_secs(10);
/// How long [`shutdown`] waits for a helper to remove its pidfile.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// The app is quitting: end a helper that is mid-read the way a finished
/// read does — SIGTERM, so Claude Code removes its pidfile under
/// `~/.claude/sessions` — and wait up to 2 s for that, rather than leave it
/// to the runtime's teardown, whose SIGKILL would leave the pidfile behind.
pub fn shutdown(state: &AppState) {
    let pids = helpers().clone();
    stop_helpers(&pids, &state.home.base().join("sessions"));
}

/// SIGTERM these helpers and wait (up to [`SHUTDOWN_GRACE`]) until none has
/// a pidfile in `sessions` any more. Only ever reads that directory.
fn stop_helpers(pids: &[u32], sessions: &Path) {
    if pids.is_empty() {
        return;
    }
    #[cfg(unix)]
    for pid in pids {
        if let Ok(pid) = i32::try_from(*pid) {
            // SAFETY: plain syscall on our own, not yet reaped, child's pid.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
    let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
    while std::time::Instant::now() < deadline
        && pids
            .iter()
            .any(|pid| sessions.join(format!("{pid}.json")).exists())
    {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Run the helper once: `initialize`, then `get_usage`. The answer (or an
/// error) returns as soon as it is in; the process is then terminated in
/// the background (SIGTERM, SIGKILL only after [`HELPER_EXIT_GRACE`]).
/// Returns the `get_usage` response body.
pub async fn helper_read(bin: &str, dir: &Path) -> Result<Value, String> {
    prepare_dir(dir).map_err(|e| format!("Could not prepare the usage helper directory: {e}"))?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(HELPER_ARGS)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // The scrubbed child environment, marked owned; CLAUDE_CONFIG_DIR only
    // when the user set it themselves.
    super::control::apply_child_env(&mut cmd, &super::accounts::AccountEnv::Inherit);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start `{bin}` for the plan usage: {e}"))?;
    let registered = HelperPid::register(child.id());
    let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
    if let Some(stderr) = child.stderr.take() {
        let tail = tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
                t.push_back(line.chars().take(200).collect());
                while t.len() > 5 {
                    t.pop_front();
                }
            }
        });
    }
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let outcome = match (stdin, stdout) {
        (Some(stdin), Some(stdout)) => {
            tokio::time::timeout(HELPER_TIMEOUT, converse(stdin, stdout))
                .await
                .unwrap_or_else(|_| Err("The usage helper did not answer in time.".to_string()))
        }
        _ => Err("The usage helper has no pipes.".to_string()),
    };
    // The answer is in: end it now rather than let it finish on its own.
    // SIGTERM, not SIGKILL: the CLI then removes its pidfile under
    // `~/.claude/sessions` and exits within a second (verified on 2.1.284);
    // SIGKILL follows only if it lingers well past that. Off the read's
    // path, so the answer is published at once.
    tokio::spawn(async move {
        let _ = super::control::terminate_within(&mut child, HELPER_EXIT_GRACE).await;
        drop(registered);
    });
    outcome.map_err(|e| {
        let last = tail
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .back()
            .cloned()
            .filter(|l| !l.trim().is_empty());
        match last {
            Some(l) if e.contains("exited") => format!("{e}: {l}"),
            _ => e,
        }
    })
}

async fn converse(
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
) -> Result<Value, String> {
    let mut lines = BufReader::new(stdout).lines();
    send(
        &mut stdin,
        "mc_usage_init",
        json!({ "subtype": "initialize" }),
    )
    .await?;
    tokio::time::timeout(
        HELPER_INIT_TIMEOUT,
        answer_to(&mut lines, &mut stdin, "mc_usage_init"),
    )
    .await
    .map_err(|_| "The usage helper did not start in time.".to_string())??;
    send(
        &mut stdin,
        "mc_usage_read",
        json!({ "subtype": "get_usage", "skip_behaviors": true }),
    )
    .await?;
    tokio::time::timeout(
        USAGE_TIMEOUT,
        answer_to(&mut lines, &mut stdin, "mc_usage_read"),
    )
    .await
    .map_err(|_| "Claude Code did not answer the plan usage request in time.".to_string())?
}

async fn send(
    stdin: &mut tokio::process::ChildStdin,
    rid: &str,
    request: Value,
) -> Result<(), String> {
    let line = protocol::control_request(rid, request).to_string() + "\n";
    stdin
        .write_all(line.as_bytes())
        .await
        .and(stdin.flush().await)
        .map_err(|e| format!("Could not write to the usage helper: {e}"))
}

/// Read lines until the reply to `rid`; CLI → host requests get
/// `unsupported` (the helper runs no turn, so none are expected).
async fn answer_to(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    stdin: &mut tokio::process::ChildStdin,
    rid: &str,
) -> Result<Value, String> {
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => return Err("The usage helper exited".to_string()),
            Err(e) => return Err(format!("Could not read the usage helper: {e}")),
        };
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("control_response") => {
                if let Some((id, reply)) = protocol::parse_control_response(&v) {
                    if id == rid {
                        return reply.map_err(|e| short_error(&e.message));
                    }
                }
            }
            Some("control_request") => {
                if let Some(id) = v.get("request_id").and_then(Value::as_str) {
                    let reply = protocol::control_error(id, "unsupported").to_string() + "\n";
                    let _ = stdin.write_all(reply.as_bytes()).await;
                    let _ = stdin.flush().await;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
