//! The transfer engine wired to the app: which directories two accounts use,
//! whether a session runs anywhere, the undo directory, data jobs, the
//! audit-free core of `POST /api/data/move` and the copy behind "continue on"
//! a directory account (docs/ACCOUNTS.md P3.2, P3.3).

use std::path::{Path, PathBuf};
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
pub(crate) fn is_live(state: &AppState, id: &str) -> bool {
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
    .map(|mut p| {
        p.from_account = Some(from.to_string());
        p.to_account = Some(to.to_string());
        p
    })
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
    state.jobs.update(job_id, |v| {
        v.sessions = sessions.clone();
        v.from = plan.from_account.clone();
        v.to = plan.to_account.clone();
    });
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
    // The lists show where things are before the job says it is done: a
    // client that waits for "done" never reads the old places.
    rediscover(state).await;
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
    done.map(|_| ())
}

/// Undo a finished transfer: files back as the engine does; an assign's
/// accounts back; a move into a set's marks cleared once its files are back
/// (a warning when the set cannot take that, the files are back all the
/// same); a move out of a set's entries back into it first, and nothing
/// undone when the set cannot take them (docs/ACCOUNTS.md §13.3 rule 6).
pub async fn undo(state: &AppState, job_id: &str) -> Result<(), TransferError> {
    let root = undo_root(state)
        .ok_or_else(|| TransferError::Invalid("this build has no data directory".into()))?;
    let _one = state.jobs.lock.lock().await;
    let found = super::manifest(&root, job_id)
        .ok_or_else(|| TransferError::NotFound("nothing to undo for that transfer".into()))?;
    let id = job_id.to_string();
    let st = state.clone();
    let pass = found
        .storage
        .as_ref()
        .and(crate::claude::backup::ops::passphrases(state));
    let (m, warning) = tokio::task::spawn_blocking(move || {
        let live = |id: &str| is_live(&st, id);
        if !found.assigned.is_empty() {
            return super::moves::undo_assign(&st, &root, &id).map(|m| (m, None));
        }
        if found.storage_to.is_some() {
            return super::moves::undo_between_sets(&root, &id, pass.as_deref()).map(|m| (m, None));
        }
        match &found.storage {
            // Out of a set: its entries back there first.
            Some(s) if !s.into => {
                let set = crate::claude::backup::Set { dir: s.dir.clone() };
                let m = super::undo_with_before(&root, &id, &live, &mut |m| {
                    if m.set_entries.is_empty() {
                        return Ok(());
                    }
                    let pass = pass.as_deref().ok_or_else(|| {
                        TransferError::from(crate::claude::backup::BackupError::NoPassphrase)
                    })?;
                    crate::claude::backup::put_back_entries(&set, pass, &m.set_entries)
                        .map(|_| ())
                        .map_err(TransferError::from)
                })?;
                Ok((m, None))
            }
            // Into a set: the files back, then the marks.
            Some(s) => {
                let m = super::undo_with(&root, &id, &live)?;
                let set = crate::claude::backup::Set { dir: s.dir.clone() };
                let cleared = match &pass {
                    Some(p) => crate::claude::backup::clear_marks(&set, p, &m.archived_marks)
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                    None => Err("no backup passphrase is set".to_string()),
                };
                let warning = cleared
                    .err()
                    .filter(|_| !m.archived_marks.is_empty())
                    .map(|e| {
                        format!(
                            "The conversations are back, but the set still says they are in \
                         storage only: {e}"
                        )
                    });
                Ok((m, warning))
            }
            None => super::undo_with(&root, &id, &live).map(|m| (m, None)),
        }
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))??;
    // A deleted conversation of a token account is that account's again.
    for (id, account) in &m.token_accounts {
        state.accounts.remember_session(id, account);
    }
    // Back in the lists before the job says it is undone.
    rediscover(state).await;
    let view = view_of(state, &m);
    state.jobs.update(job_id, |v| {
        v.state = JobState::Undone;
        v.undo_until = None;
        if let Some(w) = &warning {
            let mut r = v.result.take().unwrap_or_else(|| serde_json::json!({}));
            r["warnings"] = serde_json::json!([w]);
            v.result = Some(r);
        }
    });
    if state.jobs.get(job_id).is_none() {
        // Started before a restart: it is known only from its manifest.
        let p = state.jobs.start(job_id, &view.kind, 0, 0);
        drop(p);
        state.jobs.update(job_id, |v| *v = view.clone());
    }
    Ok(())
}

/// A job's view: the running or recent one, else a finished transfer's
/// manifest from disk.
pub fn job(state: &AppState, job_id: &str) -> Option<super::JobView> {
    state.jobs.get(job_id).or_else(|| {
        let root = undo_root(state)?;
        super::manifest(&root, job_id).map(|m| view_of(state, &m))
    })
}

/// The jobs of this run, then those of earlier runs that can still be
/// undone (from their manifests), newest first, at most 50.
pub fn list(state: &AppState) -> Vec<super::JobView> {
    let mut out = state.jobs.list();
    if let Some(root) = undo_root(state) {
        let known: std::collections::HashSet<String> = out.iter().map(|v| v.id.clone()).collect();
        let mut ids: Vec<String> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|id| super::is_job_id(id) && !known.contains(id))
            .collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        out.extend(
            ids.iter()
                .take(50)
                .filter_map(|id| super::manifest(&root, id))
                .map(|m| view_of(state, &m)),
        );
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out.truncate(50);
    out
}

/// A manifest's view, with the accounts its directories belong to now.
fn view_of(state: &AppState, m: &super::Manifest) -> super::JobView {
    let mut v = super::view_from_manifest(m);
    if m.kind.as_deref() == Some(super::delete::KIND) {
        // It names its account, and has no target.
        return v;
    }
    // The accounts it named; older manifests: whose directories they were.
    v.from = v.from.or_else(|| account_of_dir(state, &m.from_dir));
    v.to = v.to.or_else(|| account_of_dir(state, &m.to_dir));
    v
}

/// The account whose directory `dir` is (the Default for its own, since
/// token accounts share it).
fn account_of_dir(state: &AppState, dir: &Path) -> Option<String> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let dir = canon(dir);
    state
        .accounts
        .homes()
        .into_iter()
        .find(|(_, h)| canon(h.base()) == dir)
        .map(|(id, _)| id)
}

/// Re-list every directory now, so moved sessions show where they are.
pub(crate) async fn rediscover(state: &AppState) {
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
        // A copy an earlier continue-on made is brought up to date first, so
        // the fork has every turn the original had since.
        refresh: true,
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
        // An earlier continue-on copied it already and it is current, or it
        // was continued there since: fork that copy as it stands.
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
