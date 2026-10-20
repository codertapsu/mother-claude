//! The transfer engine wired to the app: which directories two accounts use,
//! whether a session runs anywhere, the undo directory, data jobs, the
//! audit-free core of `POST /api/data/move` and the copy behind "continue on"
//! a directory account (docs/ACCOUNTS.md P3.2, P3.3).

use std::path::PathBuf;
use std::sync::Arc;

use super::{Context, JobState, Mode, Plan, Request, TransferError, What};
use crate::claude::ClaudeHome;
use crate::state::AppState;

/// `<app data dir>/undo`, or `None` without a data dir.
pub fn undo_root(state: &AppState) -> Option<PathBuf> {
    state.auth.config_dir.as_ref().map(|d| d.join("undo"))
}

/// The config directory of an account.
fn home_of(state: &AppState, account: &str) -> Result<ClaudeHome, TransferError> {
    state
        .accounts
        .home(account)
        .ok_or_else(|| TransferError::NotFound(format!("no account {account:?}")))
}

/// Whether a session runs anywhere: a live pidfile in any directory, an
/// owned handle, a bridge conversation.
fn is_live(state: &AppState, id: &str) -> bool {
    state.live.is_live(id) || state.control.controls(id) || state.bridge.account_of(id).is_some()
}

/// Plan a transfer between two accounts (blocking file reads off the
/// runtime).
pub async fn plan(
    state: &AppState,
    from: &str,
    to: &str,
    req: Request,
) -> Result<Plan, TransferError> {
    let from_home = home_of(state, from)?;
    let to_home = home_of(state, to)?;
    let st = state.clone();
    tokio::task::spawn_blocking(move || {
        let live = |id: &str| is_live(&st, id);
        let ctx = Context {
            from: &from_home,
            to: &to_home,
            is_live: &live,
        };
        super::plan(&ctx, &req)
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))?
}

/// Run a plan as a data job, one data job at a time (it waits for a running
/// backup or transfer). Returns once it is done (it is quick for a session;
/// the routes run projects in the background).
pub async fn execute(state: &AppState, job_id: &str, plan: Plan) -> Result<(), TransferError> {
    let one = state.jobs.lock.lock().await;
    execute_locked(state, job_id, plan, one).await
}

/// [`execute`] with the jobs lock held: the plan's checks are made again
/// first, since it may have waited (a session resumed meanwhile, a directory
/// now in use).
async fn execute_locked(
    state: &AppState,
    job_id: &str,
    plan: Plan,
    _one: tokio::sync::MutexGuard<'_, ()>,
) -> Result<(), TransferError> {
    let root = undo_root(state)
        .ok_or_else(|| TransferError::Invalid("this build has no data directory".into()))?;
    let kind = match plan.mode {
        Mode::Move => "move",
        Mode::Copy => "copy",
    };
    let progress = state
        .jobs
        .start(job_id, kind, plan.files as u64, plan.bytes);
    let sessions: Vec<String> = plan.sessions.iter().map(|s| s.id.clone()).collect();
    state.jobs.update(job_id, |v| v.sessions = sessions.clone());
    let id = job_id.to_string();
    let progress2 = Arc::clone(&progress);
    let st = state.clone();
    let done = tokio::task::spawn_blocking(move || {
        super::prune_undo(&root);
        let live = |id: &str| is_live(&st, id);
        super::recheck(&plan, &live)?;
        super::run(&plan, &root, &id, &progress2, &live)
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))?;
    let finished = chrono::Utc::now().timestamp_millis();
    match &done {
        Ok(m) => state.jobs.update(job_id, |v| {
            v.state = JobState::Done;
            v.finished_at = Some(finished);
            v.files_done = m.created.len() as u64;
            v.bytes_done = m.created.iter().map(|c| c.size).sum();
            v.undo_until = Some(m.created_at + super::UNDO_KEEP.as_millis() as i64);
        }),
        Err(e) => state.jobs.update(job_id, |v| {
            v.state = JobState::Failed;
            v.finished_at = Some(finished);
            v.error = Some(e.to_string());
            v.error_code = Some(e.code().to_string());
            // Its undo directory stayed: Undo finishes putting things back.
            if matches!(e, TransferError::Incomplete(_)) {
                v.undo_until = Some(finished + super::UNDO_KEEP.as_millis() as i64);
            }
        }),
    }
    rediscover(state).await;
    done.map(|_| ())
}

/// Undo a finished transfer.
pub async fn undo(state: &AppState, job_id: &str) -> Result<(), TransferError> {
    let root = undo_root(state)
        .ok_or_else(|| TransferError::Invalid("this build has no data directory".into()))?;
    let _one = state.jobs.lock.lock().await;
    let id = job_id.to_string();
    let st = state.clone();
    let m = tokio::task::spawn_blocking(move || {
        let live = |id: &str| is_live(&st, id);
        super::undo_with(&root, &id, &live)
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))??;
    let view = super::view_from_manifest(&m);
    state.jobs.update(job_id, |v| {
        v.state = JobState::Undone;
        v.undo_until = None;
    });
    if state.jobs.get(job_id).is_none() {
        // Started before a restart: it is known only from its manifest.
        let p = state.jobs.start(job_id, &view.kind, 0, 0);
        drop(p);
        state.jobs.update(job_id, |v| *v = view.clone());
    }
    rediscover(state).await;
    Ok(())
}

/// A job's view: the running or recent one, else a finished transfer's
/// manifest from disk.
pub fn job(state: &AppState, job_id: &str) -> Option<super::JobView> {
    state.jobs.get(job_id).or_else(|| {
        let root = undo_root(state)?;
        super::manifest(&root, job_id).map(|m| super::view_from_manifest(&m))
    })
}

/// Re-list every directory now, so moved sessions show where they are.
async fn rediscover(state: &AppState) {
    let st = state.clone();
    let _ = tokio::task::spawn_blocking(move || {
        st.index.discover_homes(&st.accounts.homes());
    })
    .await;
    crate::server::monitor::tick_once(state).await;
}

/// "Continue on" a directory account (P3.3): copy the conversation into its
/// directory unless it is there already. Needs the account pair's copy
/// consent, which only the desktop grants (§11.2). Returns the job id of the
/// copy, if one ran.
pub async fn copy_for_continue(
    state: &AppState,
    from: &str,
    to: &str,
    session: &str,
) -> Result<Option<String>, ContinueCopyError> {
    if !state.accounts.has_copy_consent(from, to) {
        return Err(ContinueCopyError::ConsentRequired);
    }
    // Never queued behind a backup: the request would hang for minutes.
    let Ok(one) = state.jobs.lock.try_lock() else {
        return Err(ContinueCopyError::Transfer(TransferError::Busy));
    };
    // A snapshot: the conversation usually still has a process (it stopped
    // at its limit), and the fork continues from what is there now.
    let req = Request {
        what: What::Session {
            id: session.to_string(),
        },
        mode: Mode::Copy,
        trust: false,
        snapshot: true,
    };
    match plan(state, from, to, req).await {
        Ok(p) => {
            let job_id = super::new_job_id();
            let sessions = p.files;
            execute_locked(state, &job_id, p, one)
                .await
                .map_err(ContinueCopyError::Transfer)?;
            tracing::info!(
                session,
                from,
                to,
                files = sessions,
                "copied for continue-on"
            );
            Ok(Some(job_id))
        }
        // An earlier continue-on copied it already: fork that copy.
        Err(TransferError::AlreadyExists(_)) => Ok(None),
        Err(e) => Err(ContinueCopyError::Transfer(e)),
    }
}

/// Why a continue-on copy did not happen.
#[derive(Debug)]
pub enum ContinueCopyError {
    /// The desktop has not allowed copying between these two accounts.
    ConsentRequired,
    Transfer(TransferError),
}
