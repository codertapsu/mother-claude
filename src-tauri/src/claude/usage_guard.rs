//! Pausing work near a plan limit (Settings › Usage limits).
//!
//! When one of an account's plan windows (`claude::usage`) reaches the
//! threshold — 90 % by default, set in Settings — Mother Claude pauses that
//! account's work before the limit cuts it off mid-step:
//!
//! * a session working on a turn is interrupted: Mother Claude's own through
//!   its control channel, one started elsewhere through the plugin's hooks
//!   (it stops at its next tool call) or its tmux pane, as the Interrupt
//!   button does; an owned session's background agents and workflows are
//!   stopped too (background shells keep running: they spend no usage);
//! * what each was doing — the agents and workflows that stopped, its todos,
//!   how to continue it — is saved with the pause in
//!   `<app data>/usage-guard.json` (0600, atomic), so a restart of Mother
//!   Claude loses nothing;
//! * a timer in Mother Claude, not in Claude, fires the buffer (one minute
//!   by default) after the window resets, checks the window again, and sends
//!   every session it stopped a message to go on exactly where it left off;
//!   an owned session whose process ended meanwhile is continued with it;
//! * while paused, a session waiting on a scheduled run (a `/loop` wakeup,
//!   a cron job) is *held*: when the run starts a turn, the turn is stopped
//!   at once (the guard looks at every change to the session list) — in a
//!   session started elsewhere too, through its hooks or tmux; an owned
//!   session that starts a turn by itself (also a finished background task)
//!   is stopped again; and teams start no role session of that account
//!   (`capacity::admit`). A session someone starts or continues from Mother
//!   Claude meanwhile is left alone, as is a turn typed in a held session
//!   elsewhere well before its run.
//!
//! A message someone sends a paused session goes through, and that session
//! is left alone until the resume: they chose to go on. *Resume now* resumes
//! by hand and keeps that window from pausing again until it resets; *Don't
//! resume* keeps the pause until someone resumes it. A window pauses once per
//! reset. The 5-hour (`five_hour`) and weekly (`seven_day`) windows pause the
//! whole account; a per-model weekly window (`seven_day_<model>`) only the
//! sessions on that model. An account without a window (no plan usage, a
//! plan without a 5-hour limit) is never paused.
//!
//! Usage comes from `claude::usage`: while the guard is on and a session
//! works it reads even with no dashboard open, and every minute once a
//! window is within [`NEAR_POINTS`] of the threshold.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::Notify;

use super::accounts::DEFAULT_ID;
use super::model::{
    ActivityKind, AgentStatus, ControlChannel, ModelInfo, NoticeLevel, PlanWindow, SessionStatus,
    SessionSummary, WorkflowStatus,
};
use crate::server::ws::{self, ServerEvent};
use crate::state::AppState;

/// The threshold until someone sets another (percent of a window).
pub const DEFAULT_THRESHOLD: u8 = 90;
/// The lowest and highest threshold the settings take.
pub const MIN_THRESHOLD: u8 = 50;
pub const MAX_THRESHOLD: u8 = 99;
/// How long after a window's reset the resume waits (seconds).
pub const DEFAULT_BUFFER_SECS: u32 = 60;
pub const MAX_BUFFER_SECS: u32 = 3_600;
/// Within this many points of the threshold, usage is read every minute.
pub const NEAR_POINTS: f64 = 10.0;
/// How often the guard looks.
const TICK: Duration = Duration::from_secs(15);
/// The shortest wait between two looks; also how long a burst of session
/// changes settles before the guard looks at it.
const MIN_WAIT: Duration = Duration::from_secs(1);
/// A scheduled run can start a little before its time (the CLI's own
/// slack): a held session that starts working from this long before its
/// run on is taken for that run.
pub const RUN_EARLY_MS: i64 = 3 * 60_000;
/// A session asked to stop is not asked again for this long (through the
/// hooks the interrupt lands at its next tool call).
const ASK_AGAIN_MS: i64 = 60_000;
/// Sessions someone started or continued are remembered this long.
const STARTED_TTL_MS: i64 = 8 * 24 * 3_600_000;
/// A resume that found the window still full looks again this much later
/// when the window names no later reset.
const RETRY_MS: i64 = 5 * 60_000;
/// Ended pauses kept for the page.
const HISTORY: usize = 10;
const FILE: &str = "usage-guard.json";

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ───────────────────────────────── settings ─────────────────────────────────

/// What the Settings page sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GuardSettings {
    /// Pause sessions near a limit.
    pub enabled: bool,
    /// The percentage of a window that pauses.
    pub threshold_pct: u8,
    /// Resume by itself after the reset (else a pause waits for someone).
    pub auto_resume: bool,
    /// Seconds after the reset before resuming.
    pub buffer_secs: u32,
    /// Also sessions started elsewhere (VS Code, a terminal), through the
    /// plugin's hooks or their tmux pane.
    pub include_elsewhere: bool,
}

impl Default for GuardSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_pct: DEFAULT_THRESHOLD,
            auto_resume: true,
            buffer_secs: DEFAULT_BUFFER_SECS,
            include_elsewhere: true,
        }
    }
}

/// A change to the settings (`PUT /api/usage-guard/settings`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub enabled: Option<bool>,
    pub threshold_pct: Option<u8>,
    pub auto_resume: Option<bool>,
    pub buffer_secs: Option<u32>,
    pub include_elsewhere: Option<bool>,
}

impl GuardSettings {
    /// The settings with `patch` applied, or why not.
    pub fn with(&self, patch: &SettingsPatch) -> Result<Self, String> {
        let mut next = self.clone();
        if let Some(v) = patch.enabled {
            next.enabled = v;
        }
        if let Some(v) = patch.threshold_pct {
            if !(MIN_THRESHOLD..=MAX_THRESHOLD).contains(&v) {
                return Err(format!(
                    "The threshold is a percentage from {MIN_THRESHOLD} to {MAX_THRESHOLD}."
                ));
            }
            next.threshold_pct = v;
        }
        if let Some(v) = patch.auto_resume {
            next.auto_resume = v;
        }
        if let Some(v) = patch.buffer_secs {
            if v > MAX_BUFFER_SECS {
                return Err(format!(
                    "The wait after the reset is at most {} minutes.",
                    MAX_BUFFER_SECS / 60
                ));
            }
            next.buffer_secs = v;
        }
        if let Some(v) = patch.include_elsewhere {
            next.include_elsewhere = v;
        }
        Ok(next)
    }
}

// ─────────────────────────────── pauses ─────────────────────────────────────

/// A session a pause stopped, holds or left alone.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PausedSession {
    pub id: String,
    pub name: String,
    pub owned: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    /// A turn or background agents were stopped: it gets the resume message.
    pub stopped: bool,
    /// It waits on a scheduled run (a `/loop` wakeup, a cron job) that the
    /// pause holds: when that run starts a turn, the turn is stopped too.
    pub held: bool,
    /// When that run is due (epoch ms; absent: unknown, a recurring job).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_at: Option<i64>,
    /// What runs then: the wakeup's reason or the job's schedule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// What stopped (the turn in progress, agents, workflows), for the
    /// resume message.
    pub doing: Vec<String>,
    /// Its todos then: done of total.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todos: Option<(u32, u32)>,
    /// To continue an owned session whose process ended meanwhile as it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Someone sent it a message while paused: it is not stopped again, and
    /// gets no resume message.
    pub released: bool,
    /// When the resume message went out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumed_at: Option<i64>,
    /// Why it could not be resumed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_error: Option<String>,
}

/// How a pause ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Ended {
    /// The timer resumed it after the reset.
    Resumed,
    /// Someone pressed *Resume now*.
    ResumedByHand,
    /// Someone dismissed it without resuming.
    Dismissed,
}

/// One window of one account paused.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pause {
    /// `up_<12 hex>`.
    pub id: String,
    pub account: String,
    pub account_name: String,
    /// The window's key (`five_hour`, `seven_day`, `seven_day_<model>`).
    pub window: String,
    pub label: String,
    /// The window's use when it paused, and the threshold then.
    pub pct: f64,
    pub threshold_pct: u8,
    pub paused_at: i64,
    pub resets_at: i64,
    /// When the timer resumes it (`resets_at` + the buffer).
    pub resume_at: i64,
    /// A per-model window: only sessions on this model (a slug: `opus`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub sessions: Vec<PausedSession>,
    /// No resuming by itself.
    pub hold: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<Ended>,
}

impl Pause {
    pub fn active(&self) -> bool {
        self.ended.is_none()
    }

    /// Whether the pause covers `s` (its account; for a per-model window,
    /// its model).
    pub fn covers(&self, s: &SessionSummary) -> bool {
        account_of(s) == self.account && model_matches(self.model.as_deref(), s.model.as_ref())
    }
}

/// A window someone resumed by hand: not paused again until it resets.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct HeldOff {
    account: String,
    window: String,
    until: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    settings: GuardSettings,
    pauses: Vec<Pause>,
    held_off: Vec<HeldOff>,
}

/// What `GET /api/usage-guard` answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardView {
    pub settings: GuardSettings,
    /// Active pauses first, then the last few that ended.
    pub pauses: Vec<Pause>,
}

// ─────────────────────────────── the rules ──────────────────────────────────

/// Which sessions a window's pause covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The whole account (the 5-hour and weekly windows).
    Account,
    /// Sessions on one model (`seven_day_<model>`).
    Model(String),
}

/// The scope of a window the guard watches, `None` for one it leaves alone
/// (extra usage, a scope it cannot tie to sessions).
pub fn scope_of(key: &str) -> Option<Scope> {
    match key {
        "five_hour" | "seven_day" => Some(Scope::Account),
        "seven_day_overage_included" | "seven_day_scoped" => None,
        k => k
            .strip_prefix("seven_day_")
            .filter(|m| !m.is_empty() && m.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
            .map(|m| Scope::Model(m.to_string())),
    }
}

/// `Fable 5` → `fable_5`, `claude-opus-4-7` → `claude_opus_4_7`.
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

/// Whether a session on `model` is one a per-model window (`wanted`, a slug
/// such as `opus`) covers. No model asked: every session.
pub fn model_matches(wanted: Option<&str>, model: Option<&ModelInfo>) -> bool {
    let Some(w) = wanted else {
        return true;
    };
    let Some(m) = model else {
        return false;
    };
    let word = |hay: &str| {
        let hay = format!("_{}_", slug(hay));
        // The slug's first word is the family: `opus`, `fable` (a version
        // after it, `fable_5`, is the label's).
        let family = w.split('_').next().unwrap_or(w);
        hay.contains(&format!("_{w}_")) || hay.contains(&format!("_{family}_"))
    };
    word(&m.id) || word(&m.label)
}

fn account_of(s: &SessionSummary) -> &str {
    s.account.as_ref().map_or(DEFAULT_ID, |a| a.id.as_str())
}

/// A window's use as of `now` (nothing once its reset has passed).
pub fn pct_now(w: &PlanWindow, now: i64) -> f64 {
    match w.resets_at {
        Some(r) if r <= now => 0.0,
        _ => w.pct,
    }
}

/// Whether window `w` of `account` pauses now.
pub fn should_pause(
    settings: &GuardSettings,
    w: &PlanWindow,
    account: &str,
    pauses: &[Pause],
    held_off: &[(String, String, i64)],
    now: i64,
) -> bool {
    if !settings.enabled || scope_of(&w.key).is_none() {
        return false;
    }
    let Some(resets_at) = w.resets_at.filter(|r| *r > now) else {
        return false;
    };
    if pct_now(w, now) < f64::from(settings.threshold_pct) {
        return false;
    }
    // Once per reset: an active pause, or one of this very window that
    // ended (it was resumed already), keeps it from pausing again.
    let seen = pauses.iter().any(|p| {
        p.account == account && p.window == w.key && (p.active() || p.resets_at >= resets_at)
    });
    let held = held_off
        .iter()
        .any(|(a, k, until)| a == account && k == &w.key && *until >= resets_at);
    !seen && !held
}

/// Whether a window is within [`NEAR_POINTS`] of the threshold (usage is
/// then read every minute while a session works).
pub fn near(settings: &GuardSettings, windows: &[PlanWindow], now: i64) -> bool {
    settings.enabled
        && windows.iter().any(|w| {
            scope_of(&w.key).is_some()
                && w.resets_at.is_some_and(|r| r > now)
                && pct_now(w, now) >= f64::from(settings.threshold_pct) - NEAR_POINTS
        })
}

/// "in 1 h 16 min", "in 4 min", "in under a minute".
pub fn span_text(ms: i64) -> String {
    let mins = (ms.max(0) + 59_999) / 60_000;
    match mins {
        0 | 1 if ms < 60_000 => "in under a minute".to_string(),
        m if m < 60 => format!("in {m} min"),
        m => {
            let (h, rest) = (m / 60, m % 60);
            if h >= 48 {
                format!("in {} days", (h + 12) / 24)
            } else if rest == 0 {
                format!("in {h} h")
            } else {
                format!("in {h} h {rest} min")
            }
        }
    }
}

/// The window as people say it: "the 5-hour limit", "the weekly Opus limit".
pub fn window_words(key: &str, label: &str) -> String {
    match key {
        "five_hour" => "the 5-hour limit".to_string(),
        "seven_day" => "the weekly limit".to_string(),
        _ => match label.split_once('·') {
            Some((_, model)) if !model.trim().is_empty() => {
                format!("the weekly {} limit", model.trim())
            }
            _ => format!("the {} limit", label.trim().to_lowercase()),
        },
    }
}

/// What a resumed session is told (the first line names Mother Claude, as
/// every message from it does).
pub fn resume_message(p: &Pause, s: &PausedSession) -> String {
    let mut text = format!(
        "Mother Claude paused this session at {:.0} % of {} and resumes it now that the limit \
         has reset. Continue exactly where you left off.",
        p.pct,
        window_words(&p.window, &p.label)
    );
    if !s.doing.is_empty() {
        text.push_str("\n\nWhen it paused, this was stopped:");
        for d in &s.doing {
            text.push_str("\n- ");
            text.push_str(d);
        }
        text.push_str("\n\nStart again what did not finish, then go on with the task.");
    }
    if let Some((done, total)) = s.todos.filter(|(_, t)| *t > 0) {
        text.push_str(&format!(
            "\n\nYour todo list was at {done} of {total} done."
        ));
    }
    text
}

/// What the guard does with a session pause `p` covers while it holds work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watch {
    /// Nothing.
    Leave,
    /// Note that it waits on a scheduled run due `at` (`run`: what runs).
    Hold {
        at: Option<i64>,
        run: Option<String>,
    },
    /// It works again by itself: stop it (`fired`: the held run started it).
    Stop { fired: bool },
    /// A turn someone typed in a session that waited on a held run (well
    /// before the run): it goes on, and is left alone from then on.
    Release,
}

/// [`Watch`] for session `s` as of `now`. `started`: when someone started or
/// continued it from Mother Claude, if they did.
///
/// * Someone sent it a message, or started it since the pause began: left
///   alone.
/// * It waits on a scheduled run due before the resume (any, while the pause
///   waits for someone): held, so the run is known when it fires.
/// * It works: Mother Claude's own session is stopped again (only a wakeup,
///   a cron job or a finished background task starts one by itself — every
///   person's message goes through Mother Claude). One started elsewhere is
///   stopped only when the run the pause holds has come; a turn typed there
///   before that is someone's, and goes on.
pub fn watch_one(p: &Pause, s: &SessionSummary, now: i64, started: Option<i64>) -> Watch {
    let entry = p.sessions.iter().find(|ps| ps.id == s.id);
    if entry.is_some_and(|e| e.released) || started.is_some_and(|t| t >= p.paused_at) {
        return Watch::Leave;
    }
    // Nothing reaches it (no hooks, no tmux pane): it cannot be stopped, so
    // it is not shown as held either.
    if !s.owned
        && !matches!(
            s.control.channel,
            ControlChannel::Hooks | ControlChannel::Tmux
        )
    {
        return Watch::Leave;
    }
    if s.activity.kind == ActivityKind::Scheduled {
        let at = s.activity.resumes_at;
        if !p.hold && at.is_some_and(|at| at > p.resume_at) {
            return Watch::Leave;
        }
        let run = s
            .activity
            .detail
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(|d| d.chars().take(160).collect::<String>());
        if entry.is_some_and(|e| e.held && e.run_at == at && e.run == run) {
            return Watch::Leave;
        }
        return Watch::Hold { at, run };
    }
    if !matches!(s.status, SessionStatus::Working | SessionStatus::Starting) {
        return Watch::Leave;
    }
    let held = entry.filter(|e| e.held);
    let fired = held.is_some_and(|e| e.run_at.is_none_or(|at| now >= at - RUN_EARLY_MS));
    if s.owned || fired {
        Watch::Stop { fired }
    } else if held.is_some() {
        Watch::Release
    } else {
        Watch::Leave
    }
}

/// What a stopped scheduled run is called in the resume message.
fn run_words(run: Option<&str>) -> String {
    match run {
        Some(r) => format!("the turn its scheduled run “{r}” started while it was paused"),
        None => "the turn a scheduled run (a wakeup or a cron job) started while it was paused"
            .to_string(),
    }
}

// ───────────────────────────────── the store ────────────────────────────────

/// The guard's settings, pauses and timer. AppState: `state.usage_guard`.
#[derive(Debug)]
pub struct UsageGuard {
    path: Option<PathBuf>,
    saved: Mutex<Saved>,
    /// Something changed: look now (and re-arm the timer).
    pub wake: Notify,
    /// Sessions asked to stop, and when (not asked again for a minute).
    asked: Mutex<HashMap<String, i64>>,
    /// Sessions someone started or continued from Mother Claude, and when.
    started: Mutex<HashMap<String, i64>>,
}

impl UsageGuard {
    /// Load from the app data dir (`None`: memory only, for tests).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join(FILE));
        let saved = path
            .as_deref()
            .and_then(crate::persist::read_json::<Saved>)
            .map(|mut s| {
                // A hand-edited file never takes a threshold outside the range.
                s.settings.threshold_pct =
                    s.settings.threshold_pct.clamp(MIN_THRESHOLD, MAX_THRESHOLD);
                s.settings.buffer_secs = s.settings.buffer_secs.min(MAX_BUFFER_SECS);
                s
            })
            .unwrap_or_default();
        Self {
            path,
            saved: Mutex::new(saved),
            wake: Notify::new(),
            asked: Mutex::new(HashMap::new()),
            started: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Saved> {
        self.saved.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Whether session `id` may be asked to stop now: not when it was asked
    /// within [`ASK_AGAIN_MS`] (its interrupt may still be landing); else it
    /// is noted as asked now.
    fn ask_once(&self, id: &str, now: i64) -> bool {
        let mut asked = self.asked.lock().unwrap_or_else(|p| p.into_inner());
        asked.retain(|_, at| now - *at < ASK_AGAIN_MS);
        if asked.contains_key(id) {
            return false;
        }
        asked.insert(id.to_string(), now);
        true
    }

    /// When someone started or continued session `id`, if they did.
    pub fn started_at(&self, id: &str) -> Option<i64> {
        self.started
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .copied()
    }

    fn save(&self, s: &Saved) {
        if let Some(path) = &self.path {
            if let Err(e) = crate::persist::write_json_private(path, s) {
                tracing::warn!(error = %e, "could not save the usage guard's state");
            }
        }
    }

    /// Change the saved state, write it, and say so to everyone.
    fn change<R>(&self, state: &AppState, f: impl FnOnce(&mut Saved) -> R) -> R {
        let r = {
            let mut s = self.lock();
            let r = f(&mut s);
            prune(&mut s, now_ms());
            self.save(&s);
            r
        };
        state.broadcast(ServerEvent::UsageGuard);
        self.wake.notify_one();
        r
    }

    pub fn settings(&self) -> GuardSettings {
        self.lock().settings.clone()
    }

    pub fn view(&self) -> GuardView {
        let s = self.lock();
        let mut pauses: Vec<Pause> = s.pauses.iter().filter(|p| p.active()).cloned().collect();
        pauses.extend(
            s.pauses
                .iter()
                .filter(|p| !p.active())
                .rev()
                .take(HISTORY)
                .cloned(),
        );
        GuardView {
            settings: s.settings.clone(),
            pauses,
        }
    }

    /// The active pauses.
    pub fn active(&self) -> Vec<Pause> {
        self.lock()
            .pauses
            .iter()
            .filter(|p| p.active())
            .cloned()
            .collect()
    }

    /// The active pause that covers `s`, if any.
    pub fn pause_covering(&self, s: &SessionSummary) -> Option<Pause> {
        self.lock()
            .pauses
            .iter()
            .find(|p| p.active() && p.covers(s))
            .cloned()
    }

    /// An active pause of a whole account (for teams' admission).
    pub fn account_pause(&self, account: &str) -> Option<Pause> {
        self.lock()
            .pauses
            .iter()
            .find(|p| p.active() && p.account == account && p.model.is_none())
            .cloned()
    }

    /// An active pause of `account`'s weekly window for `model` (an alias
    /// such as `opus` or a full id).
    pub fn model_pause(&self, account: &str, model: &str) -> Option<Pause> {
        if model.trim().is_empty() {
            return None;
        }
        let info = ModelInfo {
            id: model.to_string(),
            label: String::new(),
            context_window: None,
        };
        self.lock()
            .pauses
            .iter()
            .find(|p| {
                p.active()
                    && p.account == account
                    && p.model.is_some()
                    && model_matches(p.model.as_deref(), Some(&info))
            })
            .cloned()
    }
}

/// Keep the active pauses, the last [`HISTORY`] ended ones, and hold-offs
/// whose window has not reset yet.
fn prune(s: &mut Saved, now: i64) {
    let ended = s.pauses.iter().filter(|p| !p.active()).count();
    if ended > HISTORY {
        let mut drop = ended - HISTORY;
        s.pauses.retain(|p| {
            if drop > 0 && !p.active() {
                drop -= 1;
                false
            } else {
                true
            }
        });
    }
    s.held_off.retain(|h| h.until > now);
}

// ──────────────────────────────── the API ───────────────────────────────────

pub fn view(state: &AppState) -> GuardView {
    state.usage_guard.view()
}

/// How many pauses are active.
pub fn active_count(state: &AppState) -> usize {
    state.usage_guard.active().len()
}

/// Apply a settings change.
pub fn set_settings(state: &AppState, patch: &SettingsPatch) -> Result<GuardView, String> {
    let next = state.usage_guard.settings().with(patch)?;
    state.usage_guard.change(state, |s| s.settings = next);
    Ok(view(state))
}

/// `Don't resume` (`hold`), or let the timer resume it again.
pub fn set_hold(state: &AppState, id: &str, hold: bool) -> Result<GuardView, String> {
    let found = state.usage_guard.change(state, |s| {
        match s.pauses.iter_mut().find(|p| p.id == id && p.active()) {
            Some(p) => {
                p.hold = hold;
                true
            }
            None => false,
        }
    });
    if !found {
        return Err("No such pause.".into());
    }
    Ok(view(state))
}

/// Forget a pause without resuming its sessions.
pub fn dismiss(state: &AppState, id: &str) -> Result<GuardView, String> {
    let found = state.usage_guard.change(state, |s| {
        match s.pauses.iter_mut().find(|p| p.id == id && p.active()) {
            Some(p) => {
                p.ended = Some(Ended::Dismissed);
                p.ended_at = Some(now_ms());
                // Not paused again before its reset: someone saw it.
                s.held_off.push(HeldOff {
                    account: p.account.clone(),
                    window: p.window.clone(),
                    until: p.resets_at,
                });
                true
            }
            None => false,
        }
    });
    if !found {
        return Err("No such pause.".into());
    }
    Ok(view(state))
}

/// Someone sent session `id` a message: a pause leaves it alone from now on
/// (also one that was idle when it paused, so not on its list yet).
pub async fn note_message(state: &AppState, id: &str) {
    if state.usage_guard.active().is_empty() {
        return;
    }
    let summary = state.find_session(id).await;
    let touched = {
        let mut s = state.usage_guard.lock();
        let touched = release(&mut s.pauses, id, summary.as_ref());
        if touched {
            state.usage_guard.save(&s);
        }
        touched
    };
    if touched {
        state.broadcast(ServerEvent::UsageGuard);
    }
}

/// Someone started or continued session `id` from Mother Claude (New
/// session, Continue, a session tool's start): a pause that began before
/// leaves it alone ([`watch_one`]).
pub fn note_started(state: &AppState, id: &str) {
    let now = now_ms();
    let mut started = state
        .usage_guard
        .started
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    started.retain(|_, at| now - *at < STARTED_TTL_MS);
    started.insert(id.to_string(), now);
}

/// Mark session `id` (`summary`: what the session list knows of it) left
/// alone in every active pause that has it or covers it. Whether anything
/// changed.
pub fn release(pauses: &mut [Pause], id: &str, summary: Option<&SessionSummary>) -> bool {
    let mut touched = false;
    for p in pauses.iter_mut().filter(|p| p.active()) {
        match p.sessions.iter_mut().find(|ps| ps.id == id) {
            Some(ps) => {
                if !ps.released {
                    ps.released = true;
                    touched = true;
                }
            }
            None => {
                if let Some(sum) = summary.filter(|sum| p.covers(sum)) {
                    p.sessions.push(PausedSession {
                        id: id.to_string(),
                        name: session_name(sum),
                        owned: sum.owned,
                        cwd: sum.cwd.clone(),
                        released: true,
                        ..Default::default()
                    });
                    touched = true;
                }
            }
        }
    }
    touched
}

/// What the plan-usage reader needs: whether the guard watches `account`'s
/// windows, and whether one is near the threshold.
pub fn read_wants(state: &AppState, windows: &[PlanWindow]) -> (bool, bool) {
    let settings = state.usage_guard.settings();
    (settings.enabled, near(&settings, windows, now_ms()))
}

/// Why teams start nothing on `account` with `model` now: a pause of the
/// whole account, or of that model's weekly window.
pub fn admission_reason(state: &AppState, account: &str, model: &str, now: i64) -> Option<String> {
    let p = state
        .usage_guard
        .account_pause(account)
        .or_else(|| state.usage_guard.model_pause(account, model))?;
    Some(pause_reason(&p, now))
}

/// [`admission_reason`] in words.
pub fn pause_reason(p: &Pause, now: i64) -> String {
    let when = if p.hold {
        "until someone resumes it".to_string()
    } else {
        format!("it resumes {}", span_text(p.resume_at - now))
    };
    let whose = if p.model.is_some() {
        "this account's sessions on that model"
    } else {
        "this account's sessions"
    };
    format!(
        "Mother Claude paused {whose} at {:.0} % of {} ({when})",
        p.pct,
        window_words(&p.window, &p.label)
    )
}

// ─────────────────────────────── the loop ───────────────────────────────────

pub fn start(state: &AppState) {
    let st = state.clone();
    tokio::spawn(async move { run(st).await });
}

async fn run(state: AppState) {
    // While a pause holds work, every change to the session list is looked
    // at within a second: a scheduled run that starts a turn is stopped at
    // once, not at the next tick.
    let mut bus = Some(state.bus.subscribe());
    loop {
        step(&state).await;
        let until = tokio::time::Instant::now() + next_wait(&state, now_ms());
        loop {
            let changed = async {
                match bus.as_mut() {
                    Some(rx) => rx.recv().await.map(Some),
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = tokio::time::sleep_until(until) => break,
                _ = state.usage_guard.wake.notified() => break,
                ev = changed => match ev {
                    Ok(Some(ev))
                        if matches!(*ev, ServerEvent::Sessions { .. })
                            && active_count(&state) > 0 =>
                    {
                        tokio::time::sleep(MIN_WAIT).await;
                        break;
                    }
                    Err(RecvError::Closed) => bus = None,
                    // Other events, or a lag (the next change catches up).
                    _ => {}
                },
            }
        }
    }
}

/// Until the next look: the tick, or a pause's resume time if sooner (the
/// timer); never less than [`MIN_WAIT`], so a pause that stays due (a resume
/// that could not run) never spins the loop.
fn next_wait(state: &AppState, now: i64) -> Duration {
    wait_until_next(
        &state.usage_guard.settings(),
        &state.usage_guard.active(),
        now,
    )
}

/// [`next_wait`] over the settings and the active pauses.
pub fn wait_until_next(settings: &GuardSettings, active: &[Pause], now: i64) -> Duration {
    if !settings.auto_resume {
        return TICK;
    }
    let next_resume = active
        .iter()
        .filter(|p| p.active() && !p.hold)
        .map(|p| p.resume_at)
        .min();
    match next_resume {
        Some(at) if at - now < TICK.as_millis() as i64 => {
            Duration::from_millis((at - now).max(MIN_WAIT.as_millis() as i64) as u64)
        }
        _ => TICK,
    }
}

/// One look, as the loop takes it (tests drive it).
pub async fn tick(state: &AppState) {
    step(state).await;
}

/// One look: pause what reached the threshold, hold scheduled runs and stop
/// again what started by itself, resume what is due.
async fn step(state: &AppState) {
    let settings = state.usage_guard.settings();
    let now = now_ms();
    if settings.enabled {
        for (account, windows) in accounts_windows(state) {
            for w in windows {
                let (pauses, held) = {
                    let s = state.usage_guard.lock();
                    let held: Vec<(String, String, i64)> = s
                        .held_off
                        .iter()
                        .map(|h| (h.account.clone(), h.window.clone(), h.until))
                        .collect();
                    (s.pauses.clone(), held)
                };
                if should_pause(&settings, &w, &account, &pauses, &held, now) {
                    pause(state, &settings, &account, &w).await;
                }
            }
        }
        watch_sessions(state, &settings).await;
    }
    for p in state.usage_guard.active() {
        if p.hold || p.resume_at > now_ms() {
            continue;
        }
        if !settings.auto_resume {
            continue;
        }
        due(state, &settings, p).await;
    }
}

/// Every account's windows (those with plan usage).
fn accounts_windows(state: &AppState) -> Vec<(String, Vec<PlanWindow>)> {
    let mut out = Vec::new();
    let default = crate::claude::usage::current(state);
    if default.available {
        out.push((DEFAULT_ID.to_string(), default.windows));
    }
    for (id, u) in crate::claude::usage::others(state) {
        if u.available {
            out.push((id, u.windows));
        }
    }
    out
}

fn new_id() -> String {
    format!("up_{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

fn session_name(s: &SessionSummary) -> String {
    s.title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| Some(s.name.clone()).filter(|n| !n.trim().is_empty()))
        .unwrap_or_else(|| s.project_name.clone())
}

/// Pause window `w` of `account`.
async fn pause(state: &AppState, settings: &GuardSettings, account: &str, w: &PlanWindow) {
    let now = now_ms();
    let Some(resets_at) = w.resets_at else {
        return;
    };
    let model = match scope_of(&w.key) {
        Some(Scope::Model(m)) => Some(m),
        _ => None,
    };
    let mut pause = Pause {
        id: new_id(),
        account: account.to_string(),
        account_name: crate::claude::switch::account_name(state, account),
        window: w.key.clone(),
        label: w.label.clone(),
        pct: w.pct,
        threshold_pct: settings.threshold_pct,
        paused_at: now,
        resets_at,
        resume_at: resets_at + i64::from(settings.buffer_secs) * 1000,
        model,
        ..Default::default()
    };
    let sessions: Vec<SessionSummary> = {
        let snap = state.snapshot.read().await;
        snap.sessions
            .iter()
            .filter(|s| s.live && s.lives_on.is_none() && pause.covers(s))
            .filter(|s| s.owned || settings.include_elsewhere)
            .cloned()
            .collect()
    };
    // Recorded first, so a turn that ends meanwhile is not taken for one
    // that started by itself. A session waiting on a scheduled run is held:
    // when the run starts a turn, it is stopped too.
    for s in &sessions {
        if is_api_conversation(state, &s.id).await {
            continue;
        }
        if let Some(ps) = stop_session(state, s, None).await {
            pause.sessions.push(ps);
        } else if let Watch::Hold { at, run } =
            watch_one(&pause, s, now, state.usage_guard.started_at(&s.id))
        {
            pause.sessions.push(held_entry(s, at, run));
        }
    }
    let stopped = pause.sessions.iter().filter(|s| s.stopped).count();
    let held = pause.sessions.iter().filter(|s| s.held).count();
    let words = window_words(&pause.window, &pause.label);
    let when = span_text(pause.resume_at - now);
    let mut text = match stopped {
        0 => format!(
            "{} reached {:.0} % of {words}: Mother Claude holds its work until the limit resets \
             ({when}).",
            pause.account_name, pause.pct
        ),
        n => format!(
            "{} reached {:.0} % of {words}: Mother Claude paused {n} session{} and resumes {} \
             after the reset ({when}).",
            pause.account_name,
            pause.pct,
            if n == 1 { "" } else { "s" },
            if n == 1 { "it" } else { "them" },
        ),
    };
    if held > 0 {
        text.push_str(&format!(
            " The scheduled runs of {held} more session{} wait until then.",
            if held == 1 { "" } else { "s" }
        ));
    }
    tracing::info!(account, window = %pause.window, pct = pause.pct, stopped, "usage guard paused");
    ws::publish_notice(state, NoticeLevel::Warning, text.clone(), None);
    crate::server::push::on_budget_alert(
        state,
        &format!("usage:{}", pause.id),
        &format!("Paused at {:.0} % of {words}", pause.pct),
        &text,
    );
    state.usage_guard.change(state, |s| s.pauses.push(pause));
}

/// A conversation of the HTTP API (`crate::bridge`): its client sends each
/// turn and reads the answer, so the guard never cuts one off; the client
/// sees the limit's own error, as it would without Mother Claude.
async fn is_api_conversation(state: &AppState, id: &str) -> bool {
    state.bridge.thread(id).await.is_some()
}

/// A held entry for `s`, which waits on a scheduled run due `at`.
fn held_entry(s: &SessionSummary, at: Option<i64>, run: Option<String>) -> PausedSession {
    PausedSession {
        id: s.id.clone(),
        name: session_name(s),
        owned: s.owned,
        cwd: s.cwd.clone(),
        held: true,
        run_at: at,
        run,
        ..Default::default()
    }
}

/// Stop what `s` is doing; what to remember of it (`None`: nothing ran).
/// `turn`: what its turn is called (default: the turn it was working on).
async fn stop_session(
    state: &AppState,
    s: &SessionSummary,
    turn: Option<String>,
) -> Option<PausedSession> {
    let working = matches!(s.status, SessionStatus::Working | SessionStatus::Starting);
    let mut doing: Vec<String> = Vec::new();
    let mut stopped = false;
    if working {
        match crate::server::routes::control::interrupt_session(state, &s.id).await {
            Ok(()) => {
                stopped = true;
                doing.push(turn.unwrap_or_else(|| "the turn it was working on".to_string()));
            }
            Err(e) => {
                tracing::debug!(session = %s.id, error = %e.message, "usage guard could not interrupt");
            }
        }
    }
    // An owned session's background agents and workflows spend usage too.
    if s.owned {
        let tasks = crate::claude::owned::live_state(state, &s.id)
            .map(|l| l.tasks.clone())
            .unwrap_or_default();
        for t in tasks.iter().filter(|t| {
            matches!(t.status.as_str(), "running" | "pending")
                && !t.ambient
                && t.task_type
                    .as_deref()
                    .is_some_and(|k| k.contains("agent") || k.contains("workflow"))
        }) {
            if crate::claude::owned::stop_task(state, &s.id, &t.id)
                .await
                .is_ok()
            {
                stopped = true;
                let what = if t
                    .task_type
                    .as_deref()
                    .is_some_and(|k| k.contains("workflow"))
                {
                    "Workflow"
                } else {
                    "Agent"
                };
                doing.push(format!("{what} “{}”", t.description.trim()));
            }
        }
    } else {
        // Started elsewhere: what was running is said, even if only the
        // main turn could be stopped.
        let (agents, workflows) = state.agents.for_session(&s.id);
        if stopped {
            for a in agents
                .iter()
                .filter(|a| a.status == AgentStatus::Running)
                .take(8)
            {
                doing.push(format!("Agent “{}”", a.label.trim()));
            }
            for w in workflows
                .iter()
                .filter(|w| w.status == WorkflowStatus::Running)
                .take(4)
            {
                doing.push(format!("Workflow “{}”", w.name.trim()));
            }
        }
    }
    if !stopped {
        return None;
    }
    let live = crate::claude::owned::live_state(state, &s.id);
    Some(PausedSession {
        id: s.id.clone(),
        name: session_name(s),
        owned: s.owned,
        cwd: s.cwd.clone(),
        stopped,
        doing,
        todos: s.todos.as_ref().map(|t| (t.done, t.total)),
        permission_mode: s.permission_mode.clone(),
        model: live
            .as_ref()
            .and_then(crate::claude::switch::carried_model)
            .or_else(|| s.model.as_ref().map(|m| m.id.clone())),
        effort: live
            .as_ref()
            .and_then(crate::claude::switch::carried_effort)
            .or_else(|| s.effort.clone()),
        ..Default::default()
    })
}

/// While pauses hold work, every session they cover ([`watch_one`]): one
/// waiting on a scheduled run is held; one that works again by itself is
/// stopped again — Mother Claude's own on any turn nobody sent it, one
/// started elsewhere when the run its pause holds starts a turn; a turn
/// someone typed in a held session elsewhere releases it.
async fn watch_sessions(state: &AppState, settings: &GuardSettings) {
    let active = state.usage_guard.active();
    if active.is_empty() {
        return;
    }
    let now = now_ms();
    let sessions: Vec<SessionSummary> = {
        let snap = state.snapshot.read().await;
        snap.sessions
            .iter()
            .filter(|s| s.live && s.lives_on.is_none())
            .filter(|s| s.owned || settings.include_elsewhere)
            .filter(|s| active.iter().any(|p| p.covers(s)))
            .cloned()
            .collect()
    };
    for s in sessions {
        if is_api_conversation(state, &s.id).await {
            continue;
        }
        let started = state.usage_guard.started_at(&s.id);
        for p in active.iter().filter(|p| p.covers(&s)) {
            let pid = p.id.clone();
            match watch_one(p, &s, now, started) {
                Watch::Leave => {}
                Watch::Hold { at, run } => {
                    let entry = held_entry(&s, at, run);
                    state.usage_guard.change(state, |saved| {
                        if let Some(p) = active_mut(saved, &pid) {
                            match p.sessions.iter_mut().find(|x| x.id == entry.id) {
                                Some(old) => {
                                    old.held = true;
                                    old.run_at = entry.run_at;
                                    old.run = entry.run;
                                }
                                None => p.sessions.push(entry),
                            }
                        }
                    });
                }
                Watch::Release => {
                    state.usage_guard.change(state, |saved| {
                        if let Some(e) = active_mut(saved, &pid)
                            .and_then(|p| p.sessions.iter_mut().find(|x| x.id == s.id))
                        {
                            e.released = true;
                        }
                    });
                }
                Watch::Stop { fired } => {
                    if !state.usage_guard.ask_once(&s.id, now) {
                        continue;
                    }
                    let run = p
                        .sessions
                        .iter()
                        .find(|x| x.id == s.id)
                        .and_then(|x| x.run.clone());
                    let turn = fired.then(|| run_words(run.as_deref()));
                    if let Some(ps) = stop_session(state, &s, turn).await {
                        tracing::info!(session = %s.id, fired, "usage guard stopped a turn while paused");
                        state.usage_guard.change(state, |saved| {
                            if let Some(p) = active_mut(saved, &pid) {
                                merge_stopped(p, ps);
                            }
                        });
                    }
                }
            }
        }
    }
}

/// The active pause `id` in the saved state.
fn active_mut<'a>(saved: &'a mut Saved, id: &str) -> Option<&'a mut Pause> {
    saved.pauses.iter_mut().find(|p| p.id == id && p.active())
}

/// Record `ps`, just stopped, in pause `p`: stopped, everything that stopped
/// named once, no longer held (the run fired; held again when it waits on
/// another).
pub fn merge_stopped(p: &mut Pause, ps: PausedSession) {
    match p.sessions.iter_mut().find(|x| x.id == ps.id) {
        Some(old) => {
            old.stopped = true;
            old.held = false;
            for d in ps.doing {
                if !old.doing.contains(&d) {
                    old.doing.push(d);
                }
            }
            old.todos = old.todos.or(ps.todos);
            old.permission_mode = old.permission_mode.take().or(ps.permission_mode);
            old.model = old.model.take().or(ps.model);
            old.effort = old.effort.take().or(ps.effort);
            if old.cwd.is_empty() {
                old.cwd = ps.cwd;
            }
        }
        None => p.sessions.push(ps),
    }
}

/// A pause's resume time came: resume it if its window has room again,
/// else wait for the window's next reset.
async fn due(state: &AppState, settings: &GuardSettings, p: Pause) {
    let now = now_ms();
    let window = accounts_windows(state)
        .into_iter()
        .find(|(a, _)| *a == p.account)
        .and_then(|(_, ws)| ws.into_iter().find(|w| w.key == p.window));
    if let Some(w) = window {
        if pct_now(&w, now) >= f64::from(settings.threshold_pct) {
            let later = w
                .resets_at
                .filter(|r| *r > now)
                .map(|r| r + i64::from(settings.buffer_secs) * 1000)
                .unwrap_or(now + RETRY_MS);
            let id = p.id.clone();
            state.usage_guard.change(state, |s| {
                if let Some(p) = s.pauses.iter_mut().find(|x| x.id == id) {
                    p.resume_at = later;
                    if let Some(r) = w.resets_at.filter(|r| *r > now) {
                        p.resets_at = r;
                    }
                }
            });
            // A read for the next look.
            let _ = crate::claude::usage::request_refresh_for(state, &p.account);
            return;
        }
    }
    resume(state, &p.id, Ended::Resumed).await;
}

/// *Resume now* (one pause, or every active one): resumes by hand.
pub async fn resume_now(state: &AppState, id: Option<&str>) -> Result<GuardView, String> {
    let ids: Vec<String> = state
        .usage_guard
        .active()
        .into_iter()
        .map(|p| p.id)
        .filter(|p| id.is_none_or(|want| want == p))
        .collect();
    if ids.is_empty() {
        return Err("No such pause.".into());
    }
    for pid in ids {
        resume(state, &pid, Ended::ResumedByHand).await;
    }
    Ok(view(state))
}

/// Resume pause `id` now (`by` says who). `false`: no such active pause
/// (it ended meanwhile).
pub async fn resume(state: &AppState, id: &str, by: Ended) -> bool {
    // Ended first, in one step with the check, so a look meanwhile neither
    // stops its sessions again nor resumes it twice.
    let claimed = state.usage_guard.change(state, |s| {
        let x = s.pauses.iter_mut().find(|x| x.id == id && x.active())?;
        x.ended = Some(by);
        x.ended_at = Some(now_ms());
        let p = x.clone();
        if by == Ended::ResumedByHand {
            s.held_off.push(HeldOff {
                account: p.account.clone(),
                window: p.window.clone(),
                until: p.resets_at,
            });
        }
        Some(p)
    });
    let Some(p) = claimed else {
        return false;
    };
    // A session another pause still holds (the weekly limit, a model's) is
    // handed over to it: that one resumes it after its own reset.
    let others = state.usage_guard.active();
    let mut handed: Vec<(String, PausedSession)> = Vec::new();
    let mut to_send: Vec<&PausedSession> = Vec::new();
    for ps in p.sessions.iter().filter(|s| s.stopped && !s.released) {
        let holder = match state.find_session(&ps.id).await {
            Some(sum) => others.iter().find(|q| q.covers(&sum)).map(|q| q.id.clone()),
            None => None,
        };
        match holder {
            Some(q) => handed.push((q, ps.clone())),
            None => to_send.push(ps),
        }
    }
    if !handed.is_empty() {
        state.usage_guard.change(state, |s| {
            for (q, ps) in &handed {
                hand_over(&mut s.pauses, q, ps);
            }
        });
    }
    let mut results: Vec<(String, Result<(), String>)> = Vec::new();
    for ps in to_send {
        // Working again already (someone went on with it): nothing to say.
        let going = state.find_session(&ps.id).await.is_some_and(|s| {
            s.live && matches!(s.status, SessionStatus::Working | SessionStatus::Starting)
        });
        if going {
            results.push((ps.id.clone(), Ok(())));
            continue;
        }
        let text = resume_message(&p, ps);
        results.push((ps.id.clone(), send_resume(state, ps, &text).await));
    }
    let resumed = results.iter().filter(|(_, r)| r.is_ok()).count();
    let failed = results.len() - resumed;
    let kept = handed.len();
    let at = now_ms();
    state.usage_guard.change(state, |s| {
        if let Some(x) = s.pauses.iter_mut().find(|x| x.id == id) {
            for (sid, r) in &results {
                if let Some(ps) = x.sessions.iter_mut().find(|ps| &ps.id == sid) {
                    match r {
                        Ok(()) => ps.resumed_at = Some(at),
                        Err(e) => ps.resume_error = Some(e.clone()),
                    }
                }
            }
        }
    });
    let words = window_words(&p.window, &p.label);
    let mut text = match (resumed, failed) {
        (0, 0) if kept > 0 => format!("{} has room again on {words}.", p.account_name),
        (0, 0) => format!(
            "{} has room again on {words}: its work goes on.",
            p.account_name
        ),
        (n, 0) => format!(
            "{} has room again on {words}: Mother Claude resumed {n} session{}.",
            p.account_name,
            if n == 1 { "" } else { "s" }
        ),
        (n, f) => format!(
            "{} has room again on {words}: Mother Claude resumed {n} session{} and could not \
             resume {f}; see Settings › Usage limits.",
            p.account_name,
            if n == 1 { "" } else { "s" }
        ),
    };
    if kept > 0 {
        text.push_str(&format!(
            " {kept} session{} stay{} paused for another limit.",
            if kept == 1 { "" } else { "s" },
            if kept == 1 { "s" } else { "" },
        ));
    }
    tracing::info!(account = %p.account, window = %p.window, resumed, failed, kept, "usage guard resumed");
    ws::publish_notice(
        state,
        if failed > 0 {
            NoticeLevel::Warning
        } else {
            NoticeLevel::Info
        },
        text.clone(),
        None,
    );
    crate::server::push::on_budget_alert(
        state,
        &format!("usage:{}:resumed", p.id),
        &format!("Resumed after {words} reset"),
        &text,
    );
    true
}

/// Give `ps`, stopped by a pause that resumes now, to pause `to`, which
/// still holds it: it resumes with that one, told everything that stopped.
pub fn hand_over(pauses: &mut [Pause], to: &str, ps: &PausedSession) {
    let Some(q) = pauses.iter_mut().find(|q| q.id == to && q.active()) else {
        return;
    };
    match q.sessions.iter_mut().find(|x| x.id == ps.id) {
        Some(old) => {
            old.stopped = true;
            for d in &ps.doing {
                if !old.doing.contains(d) {
                    old.doing.push(d.clone());
                }
            }
            old.todos = old.todos.or(ps.todos);
            old.permission_mode = old.permission_mode.take().or(ps.permission_mode.clone());
            old.model = old.model.take().or(ps.model.clone());
            old.effort = old.effort.take().or(ps.effort.clone());
            if old.cwd.is_empty() {
                old.cwd = ps.cwd.clone();
            }
        }
        None => q.sessions.push(PausedSession {
            resumed_at: None,
            resume_error: None,
            ..ps.clone()
        }),
    }
}

/// Send the resume message by whatever reaches the session; an owned one
/// whose process ended is continued with it.
async fn send_resume(state: &AppState, ps: &PausedSession, text: &str) -> Result<(), String> {
    // Mother Claude itself: it only goes on with what the session was doing.
    let mut ctx = crate::server::context::ClientContext::unknown();
    ctx.principal = crate::server::context::Principal::Desktop;
    match crate::server::routes::control::deliver_message(state, &ctx, &ps.id, text).await {
        Ok(_) => Ok(()),
        Err(e) if e.code == "no_channel" && ps.owned => {
            let live = state.find_session(&ps.id).await.is_some_and(|s| s.live);
            if live {
                return Err(e.message);
            }
            crate::claude::owned::continue_as(
                state,
                crate::claude::owned::Caller {
                    may_dangerous: true,
                    desktop: true,
                },
                &ps.id,
                &ps.cwd,
                crate::claude::owned::ContinueRequest {
                    prompt: Some(text.to_string()),
                    model: ps.model.clone(),
                    effort: ps.effort.clone(),
                    permission_mode: ps.permission_mode.clone(),
                    ..Default::default()
                },
            )
            .await
            .map(|_| ())
            .map_err(|e| e.message)
        }
        Err(e) => Err(e.message),
    }
}

#[cfg(test)]
mod tests;
