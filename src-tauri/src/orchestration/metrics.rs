//! What tasks cost and how they ended (docs/PLAN-BROWSER-AND-TEAMS.md §7.2,
//! docs/ROUTING.md): the records the router's policy is tuned on.
//!
//! **Contract** (the teams side calls [`record_outcome`] whenever a task ends
//! in any way). The types are as fixed before the work began. A record:
//!
//! 1. teaches the router (failed gates escalate a task, with the reason;
//!    a class steps up and back down — [`super::outcomes::Learned`]);
//! 2. is stored at once (`outcomes.jsonl`, 0600) with the account it ran
//!    under and whether that account pays per token;
//! 3. a few seconds later, off the runtime, is joined with the session's
//!    token usage within the task's time (its transcripts, main and
//!    subagents, through the index — `claude::index::window`), priced as an
//!    estimate, given its share of the 5-hour window on plan accounts, and
//!    stored again; the task's budget is checked then.
//!
//! [`task_started`] (optional) lets a running task's budget alert at 80 %
//! and 100 % while it runs; without it the task budget is checked at its end.

use serde::{Deserialize, Serialize};

use super::outcomes::OutcomeRecord;
use super::router::{ModelChoice, TaskClass};
use super::scheduler::{lock, now_ms};
use crate::state::AppState;

/// How a task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskResult {
    Done,
    Failed,
    /// Handed to a more capable model or effort.
    Escalated,
    Cancelled,
}

/// One task's end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOutcome {
    pub task_id: String,
    pub project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<TaskClass>,
    pub choice: ModelChoice,
    /// The session that worked on it (its token usage is in the index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Epoch milliseconds.
    pub started_at: i64,
    pub ended_at: i64,
    pub result: TaskResult,
    /// Whether the integration gate (tests) passed, when one ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_passed: Option<bool>,
    #[serde(default)]
    pub escalations: u8,
}

/// How long after a task's end its usage is read (the transcript's last
/// message is flushed when it completes).
const JOIN_DELAY: std::time::Duration = std::time::Duration::from_secs(3);
/// Messages up to this long after the end still count (clock skew).
const END_GRACE_MS: i64 = 5_000;
/// Longest task id, project path and role kept.
const ID_MAX: usize = 200;
const PATH_MAX: usize = 4096;

/// A task handed to a session ([`task_started`]), for its live budget.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTask {
    pub task_id: String,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub session_id: String,
    pub started_at: i64,
    /// The session's cost when the task started.
    #[serde(skip)]
    pub baseline_usd: f64,
}

fn clip(s: &str, max: usize) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    if clean.len() <= max {
        return clean;
    }
    let mut end = max;
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    clean[..end].to_string()
}

/// An outcome made safe to keep: ids and paths bounded, times sane.
pub fn sanitize(o: &TaskOutcome, now: i64) -> TaskOutcome {
    let mut out = o.clone();
    out.task_id = clip(&o.task_id, ID_MAX);
    out.project = clip(&o.project, PATH_MAX);
    out.role = o.role.as_deref().map(|r| clip(r, ID_MAX));
    out.choice.model = clip(&o.choice.model, 100);
    out.choice.effort = o.choice.effort.as_deref().map(|e| clip(e, 20));
    out.session_id = o
        .session_id
        .clone()
        .filter(|id| crate::claude::ids::is_session_id(id));
    if out.ended_at <= 0 {
        out.ended_at = now;
    }
    if out.started_at <= 0 || out.started_at > out.ended_at {
        out.started_at = out.ended_at;
    }
    out
}

/// The account a task's session ran under (else the project's default).
fn account_of(state: &AppState, o: &TaskOutcome) -> String {
    match &o.session_id {
        Some(id) => crate::claude::summaries::acting_account(state, id),
        None => state.accounts.account_for_cwd(&o.project),
    }
}

/// Record how a task ended.
pub fn record_outcome(state: &AppState, outcome: &TaskOutcome) {
    let now = now_ms();
    let outcome = sanitize(outcome, now);
    tracing::info!(
        task = %outcome.task_id,
        result = ?outcome.result,
        model = %outcome.choice.model,
        "task ended"
    );
    let policy = state.scheduler.policy();
    let escalation = lock(&state.scheduler.learned).note(&outcome, &policy, now);
    lock(&state.scheduler.tasks).remove(&outcome.task_id);
    let account = account_of(state, &outcome);
    let pays = super::budget::pays_per_token_account(state, &account);
    let record = OutcomeRecord {
        outcome,
        recorded_at: now,
        account: Some(account),
        pays_per_token: Some(pays),
        usage: None,
        cost_usd: None,
        window_pct: None,
        note: None,
        escalation,
        joined: false,
    };
    state.scheduler.outcomes.push(record.clone());
    join_later(state, record);
}

/// Join the usage a few seconds from now, off the runtime (inline where
/// there is no runtime).
fn join_later(state: &AppState, record: OutcomeRecord) {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            let st = state.clone();
            handle.spawn(async move {
                tokio::time::sleep(JOIN_DELAY).await;
                let files = transcript_files(&st, &record).await;
                let st2 = st.clone();
                if let Err(e) =
                    tokio::task::spawn_blocking(move || join(&st2, record, &files)).await
                {
                    tracing::warn!(error = %e, "could not join a task's usage");
                }
            });
        }
        Err(_) => {
            let files = record
                .outcome
                .session_id
                .as_deref()
                .map(|id| index_files(state, id))
                .unwrap_or_default();
            join(state, record, &files);
        }
    }
}

/// A session's transcripts as the index found them: the main one, then
/// every subagent's.
pub fn index_files(state: &AppState, session_id: &str) -> Vec<std::path::PathBuf> {
    let Some(files) = state.index.session_files(session_id) else {
        return Vec::new();
    };
    files
        .main
        .iter()
        .cloned()
        .chain(files.subagents.values().map(|a| a.path.clone()))
        .collect()
}

async fn transcript_files(state: &AppState, record: &OutcomeRecord) -> Vec<std::path::PathBuf> {
    let Some(id) = record.outcome.session_id.as_deref() else {
        return Vec::new();
    };
    let files = index_files(state, id);
    if !files.is_empty() {
        return files;
    }
    // Not indexed yet: where the session's transcript must be.
    state.transcript_path(id).await.into_iter().collect()
}

/// Read the task's usage from `files`, price it, place it in the window,
/// store the record again and check the task's budget. Blocking.
pub fn join(state: &AppState, mut record: OutcomeRecord, files: &[std::path::PathBuf]) {
    let o = &record.outcome;
    if o.session_id.is_none() {
        record.note = Some("no session named, so no usage".into());
    } else if files.is_empty() {
        record.note = Some("no transcript found for the session".into());
    } else {
        let usage = crate::claude::index::window::usage_between(
            files,
            o.started_at,
            o.ended_at + END_GRACE_MS,
        );
        if usage.partial {
            record.note = Some("part of the transcript could not be read".into());
        } else if usage.unpriced > 0 {
            record.note = Some(format!(
                "{} message{} on a model without a known price",
                usage.unpriced,
                if usage.unpriced == 1 { "" } else { "s" }
            ));
        }
        record.cost_usd = Some(usage.cost_usd);
        record.usage = Some(usage);
    }
    record.window_pct = super::budget::window_share(state, &record);
    record.joined = true;
    super::budget::check_task_end(state, &record);
    state.scheduler.outcomes.update(record);
}

/// A task was handed to a session (optional): its budget then alerts while
/// it runs. `task_id` and `session_id` as in the task's outcome later.
pub fn task_started(
    state: &AppState,
    task_id: &str,
    project: &str,
    role: Option<&str>,
    session_id: &str,
) {
    if !crate::claude::ids::is_session_id(session_id) || task_id.trim().is_empty() {
        return;
    }
    let baseline = state
        .scheduler
        .spend()
        .session_cost(session_id)
        .unwrap_or(0.0);
    let mut tasks = lock(&state.scheduler.tasks);
    tasks.insert(
        clip(task_id, ID_MAX),
        LiveTask {
            task_id: clip(task_id, ID_MAX),
            project: clip(project, PATH_MAX),
            role: role.map(|r| clip(r, ID_MAX)),
            session_id: session_id.to_string(),
            started_at: now_ms(),
            baseline_usd: baseline,
        },
    );
    // A task never reported as ended is forgotten after a day.
    let cutoff = now_ms() - 24 * 60 * 60_000;
    tasks.retain(|_, t| t.started_at >= cutoff);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outcome_is_made_safe_to_keep() {
        let o = TaskOutcome {
            task_id: format!("t\u{7}{}", "x".repeat(500)),
            project: "/p".into(),
            role: Some("r".into()),
            class: None,
            choice: ModelChoice::new("sonnet", None),
            session_id: Some("not-a-session".into()),
            started_at: 9_000,
            ended_at: 0,
            result: TaskResult::Done,
            gate_passed: None,
            escalations: 0,
        };
        let s = sanitize(&o, 5_000);
        assert_eq!(s.task_id.len(), ID_MAX);
        assert!(!s.task_id.contains('\u{7}'));
        assert_eq!(s.session_id, None);
        assert_eq!(s.ended_at, 5_000, "no end: now");
        assert_eq!(s.started_at, 5_000, "never after the end");
    }
}
