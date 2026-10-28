//! Switching a session's Claude account (docs/ACCOUNTS.md §15.1),
//! `POST /api/sessions/{id}/account {account, when?}`.
//!
//! * **A session Mother Claude runs** goes on under the other account in
//!   place: same id, same transcript, never a fork. Its process is closed to
//!   new messages and stopped first (never two writers on one transcript);
//!   into another directory the conversation then moves there (the transfer
//!   engine's move, an undo job, behind the account pair's copy consent as
//!   "continue on"); and it is resumed (`--resume <id>`) with the other
//!   account's environment and the settings it ran with — model, effort,
//!   thinking, ultracode, permission mode: always explicit, and only a mode
//!   the caller may launch. A move that fails resumes it where it was, under
//!   its own account, and the answer says so. Busy with a turn, `when: idle`
//!   waits for the turn to end (a pending switch, `switchingTo` on the
//!   summary, cancellable with `DELETE`); `when: now` takes back what is
//!   queued behind the turn, interrupts it, and hands the queued messages to
//!   the resumed process.
//! * **A session nobody runs**: its next continue runs under the other
//!   account — recorded as that account's within the shared folder (an
//!   assign), moved into another directory.
//! * **A session another process runs** cannot change accounts:
//!   `409 session_not_owned` (the UI offers *Continue on*).
//!
//! Background tasks a stopped process started stop with it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use super::accounts::DEFAULT_ID;
use super::control::{CloseRefused, Closing, Driver, OwnedLive, OwnedThinking, QueuedContent};
use super::owned::{self, Caller};
use super::transfer::ops::{self, ContinueCopyError};
use super::ClaudeHome;
use crate::claude::model::NoticeLevel;
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::server::ws;
use crate::state::AppState;

/// How long a stopped process may take to end (SIGTERM, a grace, SIGKILL).
const STOP_WAIT: Duration = Duration::from_secs(20);
/// How long an interrupted turn may take to end before the process stops
/// anyway.
const INTERRUPT_WAIT: Duration = Duration::from_secs(10);
/// How often a pending switch looks at its session's turn (it is also woken
/// when it is cancelled or replaced).
const POLL: Duration = Duration::from_millis(200);

/// When a busy session switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum When {
    /// As soon as its turn ends (the default).
    #[default]
    Idle,
    /// Now: the turn is interrupted first.
    Now,
}

impl When {
    /// `idle` (or nothing) | `now`; anything else is `422 invalid_when`.
    pub fn parse(value: Option<&str>) -> ApiResult<Self> {
        match value.map(str::trim) {
            None | Some("") | Some("idle") => Ok(Self::Idle),
            Some("now") => Ok(Self::Now),
            Some(other) => Err(ApiError::unprocessable(
                "invalid_when",
                format!("Unknown `when` {other:?} (expected \"idle\" or \"now\")."),
            )),
        }
    }
}

/// `POST /api/sessions/{id}/account` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SwitchBody {
    /// The account to go on under.
    pub account: String,
    /// `idle` (default) | `now`.
    pub when: Option<String>,
}

/// What a switch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It runs — or, ended, will next run — under `account`; `moved` into
    /// its directory by the data job `job` (which can be undone).
    Switched {
        account: String,
        moved: bool,
        job: Option<String>,
    },
    /// Busy: it switches to `account` when its turn ends.
    Pending { account: String },
}

// ───────────────────────────── the registry ─────────────────────────────

/// Where a switch stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the session's turn to end; cancellable.
    Waiting,
    /// Stopping, moving, resuming: nothing reaches the session meanwhile.
    Underway,
}

#[derive(Debug, Default)]
struct Signal {
    /// Cancelled, or replaced by another request.
    cancelled: AtomicBool,
    /// Wakes a waiting switch at once.
    wake: Notify,
    /// The bridge host ended a conversation closed for the switch.
    host_closed: AtomicBool,
    closed: Notify,
}

#[derive(Debug, Clone)]
struct Entry {
    seq: u64,
    from: String,
    to: String,
    phase: Phase,
    signal: Arc<Signal>,
}

/// The account switches waiting for a turn to end or under way, by session
/// id (`state.control.switches`). The summary's `switchingTo` and the gates
/// that keep a second writer away while a session is between its stop and
/// its resume read it.
#[derive(Debug, Default)]
pub struct Switches {
    map: Mutex<HashMap<String, Entry>>,
    seq: AtomicU64,
}

impl Switches {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.map.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The account a session is switching to, while a switch waits or is
    /// under way.
    pub fn target(&self, id: &str) -> Option<String> {
        self.lock().get(id).map(|e| e.to.clone())
    }

    /// Where a session's switch stands, if it has one.
    pub fn phase(&self, id: &str) -> Option<Phase> {
        self.lock().get(id).map(|e| e.phase)
    }

    /// A switch is stopping, moving or resuming the session right now.
    pub fn is_underway(&self, id: &str) -> bool {
        self.phase(id) == Some(Phase::Underway)
    }

    /// The bridge host ended the stream of a conversation closed for a
    /// switch: the switch may open it again. `false` when no switch is under
    /// way for it (the conversation is gone, as usual).
    pub fn note_host_closed(&self, id: &str) -> bool {
        let map = self.lock();
        match map.get(id).filter(|e| e.phase == Phase::Underway) {
            Some(e) => {
                e.signal.host_closed.store(true, Ordering::SeqCst);
                e.signal.closed.notify_one();
                true
            }
            None => false,
        }
    }

    /// Register a switch. One under way refuses (its phase is returned); one
    /// waiting is replaced (and its task told).
    fn begin(&self, id: &str, from: &str, to: &str, phase: Phase) -> Result<Entry, Phase> {
        let mut map = self.lock();
        if let Some(old) = map.get(id) {
            if old.phase == Phase::Underway {
                return Err(Phase::Underway);
            }
            old.signal.cancelled.store(true, Ordering::SeqCst);
            old.signal.wake.notify_one();
        }
        let entry = Entry {
            seq: self.seq.fetch_add(1, Ordering::Relaxed) + 1,
            from: from.to_string(),
            to: to.to_string(),
            phase,
            signal: Arc::new(Signal::default()),
        };
        map.insert(id.to_string(), entry.clone());
        Ok(entry)
    }

    /// Whether the switch `seq` is still the session's and not cancelled.
    fn is_current(&self, id: &str, seq: u64) -> bool {
        self.lock()
            .get(id)
            .is_some_and(|e| e.seq == seq && !e.signal.cancelled.load(Ordering::SeqCst))
    }

    /// Move the switch `seq` to `phase`; `false` when it is no longer the
    /// session's (cancelled or replaced).
    fn set_phase(&self, id: &str, seq: u64, phase: Phase) -> bool {
        match self.lock().get_mut(id) {
            Some(e) if e.seq == seq && !e.signal.cancelled.load(Ordering::SeqCst) => {
                e.phase = phase;
                true
            }
            _ => false,
        }
    }

    /// Forget the switch `seq` (done, failed or given up).
    fn finish(&self, id: &str, seq: u64) {
        let mut map = self.lock();
        if map.get(id).is_some_and(|e| e.seq == seq) {
            map.remove(id);
        }
    }

    /// Cancel a switch that waits for its turn. `Err(Some(Underway))` when
    /// it is under way already, `Err(None)` when there is none.
    fn cancel_waiting(&self, id: &str) -> Result<Entry, Option<Phase>> {
        let mut map = self.lock();
        match map.get(id).map(|e| e.phase) {
            Some(Phase::Waiting) => {
                let e = map.remove(id).ok_or(None)?;
                e.signal.cancelled.store(true, Ordering::SeqCst);
                e.signal.wake.notify_one();
                Ok(e)
            }
            Some(Phase::Underway) => Err(Some(Phase::Underway)),
            None => Err(None),
        }
    }
}

// ───────────────────────────── errors ─────────────────────────────

/// `409 session_not_owned`: another process runs it.
pub fn session_not_owned() -> ApiError {
    ApiError::conflict(
        "session_not_owned",
        "Another process runs this session (a terminal, an editor), and a running process \
         cannot change accounts. Continue it on the other account instead: a new session from \
         where it stands, while this one keeps running.",
    )
}

/// `409 switch_underway`: too late to cancel or change.
fn switch_underway() -> ApiError {
    ApiError::conflict(
        "switch_underway",
        "This session is switching accounts right now; it comes back under the other account \
         in a moment.",
    )
}

/// `409 copy_consent_required`, as "continue on" answers it.
fn consent_required(from: &str, to: &str) -> ApiError {
    ApiError::conflict(
        "copy_consent_required",
        "That account keeps its own directory: the conversation has to move there. Allow \
         copies between these two accounts (on the desktop), then try again.",
    )
    .with("from", from.to_string())
    .with("to", to.to_string())
}

fn move_error(e: ContinueCopyError, from: &str, to: &str) -> ApiError {
    match e {
        ContinueCopyError::ConsentRequired => consent_required(from, to),
        ContinueCopyError::Transfer(e) => crate::server::routes::transfers::transfer_error(e),
    }
}

fn unknown_account(id: &str) -> ApiError {
    ApiError::unprocessable("unknown_account", format!("No account {id:?}."))
}

// ───────────────────────────── switching ─────────────────────────────

/// Switch session `id` to the account `body` names on behalf of `ctx` (the
/// route audits it). Refusals change nothing.
pub async fn switch(
    state: &AppState,
    ctx: &ClientContext,
    id: &str,
    body: &SwitchBody,
) -> ApiResult<Outcome> {
    if !owned::is_uuid(id) {
        return Err(ApiError::unprocessable(
            "invalid_session_id",
            "Only Claude Code sessions (UUID ids) change accounts.",
        ));
    }
    let when = When::parse(body.when.as_deref())?;
    let to = body.account.trim();
    if to.is_empty() || !state.accounts.is_usable(to) {
        return Err(unknown_account(to));
    }
    // A token account whose token is not on this machine: refused now, never
    // a fallback to another sign-in later.
    state.accounts.env(to).map_err(owned::account_error)?;
    let caller = Caller::of(ctx);
    if state.bridge.thread(id).await.is_some() {
        return Err(ApiError::conflict(
            "not_supported",
            "Switching the account of a conversation of the API is not available yet.",
        ));
    }
    if owned::has_handle(state, id) {
        return switch_owned(state, ctx, caller, id, to, when).await;
    }
    if state.control.switches.is_underway(id) {
        return Err(owned::switching_account());
    }
    if owned::live_elsewhere(state, id).await {
        return Err(session_not_owned());
    }
    switch_ended(state, id, to).await
}

/// Cancel the switch session `id` waits to make; returns the account it
/// stays on.
pub async fn cancel(state: &AppState, id: &str) -> ApiResult<String> {
    match state.control.switches.cancel_waiting(id) {
        Ok(entry) => {
            publish_switching(state, id).await;
            Ok(entry.from)
        }
        Err(Some(_)) => Err(switch_underway()),
        Err(None) => Err(ApiError::conflict(
            "switch_not_pending",
            "This session is not waiting to switch accounts.",
        )),
    }
}

/// Forget a switch a session waits to make because the session is being
/// stopped or removed: nothing should start it again later. `Err` while one
/// is under way.
pub async fn drop_waiting(state: &AppState, id: &str) -> ApiResult<()> {
    match state.control.switches.cancel_waiting(id) {
        Ok(_) => {
            publish_switching(state, id).await;
            Ok(())
        }
        Err(Some(_)) => Err(owned::switching_account()),
        Err(None) => Ok(()),
    }
}

/// What a switch found it can do, checked before anything changes.
#[derive(Debug, Clone, Copy)]
struct Prepared {
    /// Both accounts keep their data in the same directory: nothing moves.
    same_dir: bool,
}

fn same_dir(a: &ClaudeHome, b: &ClaudeHome) -> bool {
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a.base()) == canon(b.base())
}

/// The refusals of resuming `id` (in `cwd`) under `to` instead of `from`
/// that need nothing to change first: the folder's trust in the target
/// account (headless runs show no trust dialog), and for another directory
/// the pair's copy consent, a target without this conversation, no other
/// data job running.
fn check_target(
    state: &AppState,
    caller: Caller,
    id: &str,
    from: &str,
    to: &str,
    cwd: &str,
) -> ApiResult<Prepared> {
    let to_home = state.accounts.home(to).ok_or_else(|| unknown_account(to))?;
    let from_home = state
        .accounts
        .home(from)
        .unwrap_or_else(|| state.home.clone());
    owned::launch_cwd(&to_home, cwd, caller)?;
    let same_dir = same_dir(&from_home, &to_home);
    if !same_dir {
        if !state.accounts.has_copy_consent(from, to) {
            return Err(consent_required(from, to));
        }
        if !super::transfer::transcripts_of(&to_home, id).is_empty() {
            return Err(ApiError::conflict(
                "already_exists",
                "That account has this conversation already (a copy): continue that one there \
                 instead.",
            ));
        }
        if state.jobs.lock.try_lock().is_err() {
            return Err(crate::server::routes::transfers::transfer_error(
                super::transfer::TransferError::Busy,
            ));
        }
    }
    Ok(Prepared { same_dir })
}

/// An owned session: in place now, or when its turn ends.
async fn switch_owned(
    state: &AppState,
    ctx: &ClientContext,
    caller: Caller,
    id: &str,
    to: &str,
    when: When,
) -> ApiResult<Outcome> {
    let from = state
        .control
        .account_of(id)
        .unwrap_or_else(|| DEFAULT_ID.to_string());
    if from == to {
        // There already; a switch that waited is no longer wanted.
        if state.control.switches.cancel_waiting(id).is_ok() {
            publish_switching(state, id).await;
        }
        return Ok(Outcome::Switched {
            account: to.to_string(),
            moved: false,
            job: None,
        });
    }
    let live = owned::live_state(state, id).ok_or_else(|| ApiError::no_such_session(id))?;
    // It is resumed in the mode it runs in.
    let mode = live.permission_mode.as_deref().unwrap_or("default");
    if owned::is_dangerous_mode(Some(mode)) {
        crate::server::auth::require_dangerous(
            ctx,
            "Switching the account of a session that runs without permission prompts",
        )?;
    }
    let cwd = state.control.cwd_of(id).unwrap_or_default();
    let prepared = check_target(state, caller, id, &from, to, &cwd)?;
    let entry = state
        .control
        .switches
        .begin(id, &from, to, Phase::Underway)
        .map_err(|_| switch_underway())?;
    publish_switching(state, id).await;
    match state.control.close_for_switch(id, when == When::Idle) {
        Ok(closing) => {
            let result = run_owned(state, caller, id, &from, to, when, prepared, closing).await;
            state.control.switches.finish(id, entry.seq);
            publish_switching(state, id).await;
            announce(state, id, to, &result).await;
            result
        }
        Err(CloseRefused::Busy) => {
            state
                .control
                .switches
                .set_phase(id, entry.seq, Phase::Waiting);
            tokio::spawn(wait_then_switch(
                state.clone(),
                caller,
                id.to_string(),
                from,
                to.to_string(),
                entry,
            ));
            Ok(Outcome::Pending {
                account: to.to_string(),
            })
        }
        Err(CloseRefused::Gone) => {
            // Its process ended just now: a session nobody runs.
            state.control.switches.finish(id, entry.seq);
            publish_switching(state, id).await;
            switch_ended(state, id, to).await
        }
    }
}

/// A pending switch: wait for the turn to end (or the switch to be
/// cancelled or replaced), check again, then switch.
async fn wait_then_switch(
    state: AppState,
    caller: Caller,
    id: String,
    from: String,
    to: String,
    entry: Entry,
) {
    loop {
        if !state.control.switches.is_current(&id, entry.seq) {
            return;
        }
        match state.control.is_idle(&id) {
            None => {
                state.control.switches.finish(&id, entry.seq);
                publish_switching(&state, &id).await;
                let name = session_name(&state, &id).await;
                ws::publish_notice(
                    &state,
                    NoticeLevel::Warning,
                    format!(
                        "{name} ended before its turn did, so it was not switched to {}.",
                        account_name(&state, &to)
                    ),
                    Some(&id),
                );
                return;
            }
            Some(true) => {
                let cwd = state.control.cwd_of(&id).unwrap_or_default();
                let result = match check_target(&state, caller, &id, &from, &to, &cwd) {
                    Err(e) => Err(e),
                    Ok(prepared) => {
                        if !state
                            .control
                            .switches
                            .set_phase(&id, entry.seq, Phase::Underway)
                        {
                            return;
                        }
                        match state.control.close_for_switch(&id, true) {
                            Ok(closing) => {
                                run_owned(
                                    &state,
                                    caller,
                                    &id,
                                    &from,
                                    &to,
                                    When::Idle,
                                    prepared,
                                    closing,
                                )
                                .await
                            }
                            // A message got in first: wait for that turn too.
                            Err(CloseRefused::Busy) => {
                                state
                                    .control
                                    .switches
                                    .set_phase(&id, entry.seq, Phase::Waiting);
                                continue;
                            }
                            Err(CloseRefused::Gone) => continue,
                        }
                    }
                };
                state.control.switches.finish(&id, entry.seq);
                publish_switching(&state, &id).await;
                announce(&state, &id, &to, &result).await;
                return;
            }
            Some(false) => {}
        }
        let _ = tokio::time::timeout(POLL, entry.signal.wake.notified()).await;
    }
}

/// Stop the closed session, move it when the directory changes, resume it
/// under `to` (under `from` when the move failed), and hand it what was
/// queued. Before the stop every check is made again, and a refusal opens
/// the session again as it was.
#[allow(clippy::too_many_arguments)]
async fn run_owned(
    state: &AppState,
    caller: Caller,
    id: &str,
    from: &str,
    to: &str,
    when: When,
    prepared: Prepared,
    closing: Closing,
) -> ApiResult<Outcome> {
    let live = &closing.live;
    let ready = owned::launch_mode(
        Some(live.permission_mode.as_deref().unwrap_or("default")),
        caller,
    )
    .and_then(|_| state.accounts.env(to).map_err(owned::account_error));
    if let Err(e) = ready {
        state.control.reopen(id);
        return Err(e);
    }

    // Now: what is queued comes back (it goes to the resumed process), then
    // the turn is interrupted so the transcript ends cleanly.
    let mut held: Vec<(QueuedContent, bool)> = Vec::new();
    if when == When::Now && closing.driver == Driver::Stream {
        for q in &live.queued {
            let Some(content) = closing.queued.get(&q.id) else {
                continue;
            };
            match owned::cancel_queued(state, id, &q.id).await {
                Ok(()) => held.push((content.clone(), live.restricted_queued.contains(&q.id))),
                // Answered meanwhile: the conversation has it.
                Err(e) => tracing::debug!(session = %id, error = %e.message, "queued message kept"),
            }
        }
        if state.control.is_idle(id) == Some(false) {
            if let Err(e) = owned::interrupt(state, id).await {
                tracing::debug!(session = %id, error = %e.message, "interrupt before a switch");
            }
            let deadline = Instant::now() + INTERRUPT_WAIT;
            while state.control.is_idle(id) == Some(false) && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    // The process stops (its pending approvals are withdrawn as it goes):
    // only then may the same conversation run again.
    if !state.control.stop_and_wait(id, STOP_WAIT).await {
        return Err(ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "switch_failed",
            "The session's process did not stop in time, so it was not switched: a second \
             process on the same conversation would write over it.",
        ));
    }
    crate::pending::withdraw_session(state, id, "switching account").await;
    // The roster without its process, so the move sees nobody running it.
    crate::server::monitor::tick_once(state).await;

    // Into another directory, the conversation moves there first.
    let from_home = state
        .accounts
        .home(from)
        .unwrap_or_else(|| state.home.clone());
    let resume = !super::transfer::transcripts_of(&from_home, id).is_empty();
    let mut job: Option<String> = None;
    let mut failure: Option<ApiError> = None;
    let mut run_as = to.to_string();
    if !prepared.same_dir && resume {
        match ops::move_for_switch(state, from, to, id).await {
            Ok(j) => job = Some(j),
            Err(e) => {
                let mut err = move_error(e, from, to).with("account", from.to_string());
                err.message = format!(
                    "{} It goes on under {}.",
                    err.message.trim_end(),
                    account_name(state, from)
                );
                failure = Some(err);
                run_as = from.to_string();
            }
        }
    }

    // Resumed in place.
    if let Err(e) = spawn_resumed(state, caller, id, &run_as, &closing, resume).await {
        if run_as == from {
            return Err(nothing_runs(e));
        }
        // It could not start under the other account: back where it was.
        let back = match &job {
            Some(j) => ops::undo(state, j).await.is_ok(),
            None => true,
        };
        if !back
            || spawn_resumed(state, caller, id, from, &closing, resume)
                .await
                .is_err()
        {
            return Err(nothing_runs(e));
        }
        after_resume(state, id, &closing, held).await;
        let mut err = e.with("account", from.to_string());
        err.message = format!(
            "{} It goes on under {}.",
            err.message.trim_end(),
            account_name(state, from)
        );
        return Err(err);
    }
    after_resume(state, id, &closing, held).await;
    match failure {
        Some(e) => Err(e),
        None => Ok(Outcome::Switched {
            account: to.to_string(),
            moved: job.is_some(),
            job,
        }),
    }
}

/// The error of a switch that left the session stopped.
fn nothing_runs(mut e: ApiError) -> ApiError {
    e.message = format!(
        "{} The session is stopped: continue it to start it again.",
        e.message.trim_end()
    );
    e
}

/// Launch `id` again under `account` with what it ran with: in place
/// (`--resume`), or with the same id when it has no transcript yet.
async fn spawn_resumed(
    state: &AppState,
    caller: Caller,
    id: &str,
    account: &str,
    closing: &Closing,
    resume: bool,
) -> ApiResult<()> {
    let live = &closing.live;
    let mode = owned::launch_mode(
        Some(live.permission_mode.as_deref().unwrap_or("default")),
        caller,
    )?;
    let account_env = state.accounts.env(account).map_err(owned::account_error)?;
    let opts = super::control::SpawnOptions {
        cwd: closing.cwd.clone(),
        model: carried_model(live),
        effort: carried_effort(live),
        thinking: live.thinking.map(|t| {
            match t {
                OwnedThinking::Off => "off",
                OwnedThinking::On(_) => "on",
            }
            .to_string()
        }),
        permission_mode: Some(mode),
        resume: resume.then(|| id.to_string()),
        session_id: (!resume).then(|| id.to_string()),
        // The bypass flag only as the session had it, and only for a caller
        // who may launch with it.
        allow_bypass: live.bypass_available && caller.may_dangerous,
        caller_may_dangerous: caller.may_dangerous,
        account: Some(account.to_string()),
        account_env,
        ..Default::default()
    };
    state
        .control
        .spawn(state, opts)
        .await
        .map(|_| ())
        .map_err(owned::spawn_error)
}

/// The model it ran: what `get_settings` resolved (unless a change is not
/// confirmed yet), else what the stream reported or we switched to.
fn carried_model(live: &OwnedLive) -> Option<String> {
    live.applied_model()
        .and_then(|a| a.model.clone())
        .or_else(|| live.model.clone())
        .or_else(|| live.applied.as_ref().and_then(|a| a.model.clone()))
        .filter(|m| !crate::claude::pricing::is_synthetic(m))
        .and_then(|m| owned::validate_model(&m).ok())
}

/// The effort it ran at: what was asked for, else what the CLI applied.
fn carried_effort(live: &OwnedLive) -> Option<String> {
    live.effort
        .clone()
        .or_else(|| live.applied_effort().and_then(|a| a.effort.clone()))
        .filter(|e| owned::EFFORT_LEVELS.contains(&e.as_str()))
}

/// What a launch flag cannot carry: ultracode (a session flag), and the
/// messages that were queued behind the interrupted turn.
async fn after_resume(
    state: &AppState,
    id: &str,
    closing: &Closing,
    held: Vec<(QueuedContent, bool)>,
) {
    if closing.live.ultracode_wanted() {
        if let Err(e) = owned::set_ultracode(state, id, true).await {
            tracing::debug!(session = %id, error = %e.message, "ultracode after a switch");
        }
    }
    for (message, restricted) in held {
        if let Err(e) = owned::send_as(state, !restricted, id, &message.text, &message.images).await
        {
            ws::publish_notice(
                state,
                NoticeLevel::Warning,
                format!(
                    "A message that was queued could not be handed to the resumed session: {}",
                    e.message
                ),
                Some(id),
            );
        }
    }
}

/// A session nobody runs: recorded as `to`'s within the shared folder,
/// moved into another directory; its next continue runs under `to`.
async fn switch_ended(state: &AppState, id: &str, to: &str) -> ApiResult<Outcome> {
    // A switch that waited for its turn is no longer wanted.
    if state.control.switches.cancel_waiting(id).is_ok() {
        publish_switching(state, id).await;
    }
    if state.index.transcript_path(id).is_none() && state.find_session(id).await.is_none() {
        return Err(ApiError::no_such_session(id));
    }
    if super::summaries::session_account(state, id) == to {
        return Ok(Outcome::Switched {
            account: to.to_string(),
            moved: false,
            job: None,
        });
    }
    let from = super::summaries::acting_account(state, id);
    let from_home = state
        .accounts
        .home(&from)
        .unwrap_or_else(|| state.home.clone());
    let to_home = state.accounts.home(to).ok_or_else(|| unknown_account(to))?;
    if same_dir(&from_home, &to_home) {
        if ops::is_live(state, id) {
            return Err(session_not_owned());
        }
        state.accounts.remember_session(id, to);
        let chip = crate::claude::accounts::view::account_ref(state, to);
        ws::patch_session(state, id, move |s| s.account = chip).await;
        return Ok(Outcome::Switched {
            account: to.to_string(),
            moved: false,
            job: None,
        });
    }
    let job = ops::move_for_switch(state, &from, to, id)
        .await
        .map_err(|e| move_error(e, &from, to))?;
    Ok(Outcome::Switched {
        account: to.to_string(),
        moved: true,
        job: Some(job),
    })
}

// ───────────────────────────── telling ─────────────────────────────

/// Put the session's `switchingTo` in step with the registry.
async fn publish_switching(state: &AppState, id: &str) {
    let to = state.control.switches.target(id);
    ws::patch_session(state, id, move |s| s.switching_to = to).await;
}

async fn session_name(state: &AppState, id: &str) -> String {
    state
        .find_session(id)
        .await
        .map(|s| s.name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("Session {}", id.chars().take(8).collect::<String>()))
}

/// An account's name, else its id.
pub fn account_name(state: &AppState, id: &str) -> String {
    state
        .accounts
        .get(id)
        .map(|a| a.name)
        .unwrap_or_else(|| id.to_string())
}

/// Tell every client how a switch of a running session ended.
async fn announce(state: &AppState, id: &str, to: &str, result: &ApiResult<Outcome>) {
    let name = session_name(state, id).await;
    match result {
        Ok(Outcome::Switched { .. }) => ws::publish_notice(
            state,
            NoticeLevel::Success,
            format!("{name} goes on under {}", account_name(state, to)),
            Some(id),
        ),
        Ok(Outcome::Pending { .. }) => {}
        Err(e) => ws::publish_notice(
            state,
            NoticeLevel::Error,
            format!(
                "{name} could not switch to {}: {}",
                account_name(state, to),
                e.message
            ),
            Some(id),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_is_idle_unless_now() {
        assert_eq!(When::parse(None).unwrap(), When::Idle);
        assert_eq!(When::parse(Some(" idle ")).unwrap(), When::Idle);
        assert_eq!(When::parse(Some("now")).unwrap(), When::Now);
        assert_eq!(When::parse(Some("later")).unwrap_err().code, "invalid_when");
    }

    #[test]
    fn a_waiting_switch_is_replaced_or_cancelled_one_under_way_is_not() {
        let s = Switches::default();
        assert_eq!(s.target("a"), None);
        let first = s.begin("a", "default", "acc_1", Phase::Waiting).unwrap();
        assert_eq!(s.target("a").as_deref(), Some("acc_1"));
        assert!(!s.is_underway("a"));
        assert!(
            !s.note_host_closed("a"),
            "only one under way waits for the host"
        );

        // Another request replaces a waiting one, and its task is told.
        let second = s.begin("a", "default", "acc_2", Phase::Waiting).unwrap();
        assert!(first.signal.cancelled.load(Ordering::SeqCst));
        assert!(!s.is_current("a", first.seq) && s.is_current("a", second.seq));
        assert!(!s.set_phase("a", first.seq, Phase::Underway));
        s.finish("a", first.seq);
        assert_eq!(s.target("a").as_deref(), Some("acc_2"), "not the old one's");

        // Under way: neither replaced nor cancelled.
        assert!(s.set_phase("a", second.seq, Phase::Underway));
        assert!(s.is_underway("a"));
        assert_eq!(
            s.begin("a", "default", "acc_3", Phase::Waiting)
                .unwrap_err(),
            Phase::Underway
        );
        assert_eq!(s.cancel_waiting("a").unwrap_err(), Some(Phase::Underway));
        assert!(s.note_host_closed("a"));
        assert!(second.signal.host_closed.load(Ordering::SeqCst));

        // Back to waiting (a message got in first), then cancelled.
        assert!(s.set_phase("a", second.seq, Phase::Waiting));
        let cancelled = s.cancel_waiting("a").unwrap();
        assert_eq!(
            (cancelled.from.as_str(), cancelled.to.as_str()),
            ("default", "acc_2")
        );
        assert!(second.signal.cancelled.load(Ordering::SeqCst));
        assert_eq!(s.target("a"), None);
        assert_eq!(s.cancel_waiting("a").unwrap_err(), None);
    }
}
