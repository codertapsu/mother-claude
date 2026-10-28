//! What is known about each account right now, beyond the registry: its
//! sign-in (`claude auth status` with the account's environment), whether a
//! session of it just failed to authenticate, and its sign-in run — running
//! or last finished (docs/ACCOUNTS.md §5.8, §15.2, P1.3).
//!
//! The Default account's sign-in stays `state.claude_auth` (checked at
//! startup as before); the others are checked in parallel at startup, after a
//! sign-in, and on demand.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use super::capture::{SignInError, SignInJob};
use super::{AccountError, AccountKind};
use crate::claude::secrets::Secret;
use crate::claude::AuthStatus;
use crate::state::AppState;

/// One account's runtime facts.
#[derive(Debug, Clone, Default)]
pub struct AccountStatus {
    /// The last `claude auth status` (for the Default: `state.claude_auth`).
    pub auth: Option<AuthStatus>,
    pub checked_at: Option<i64>,
    pub checking: bool,
    /// A session of this account failed to authenticate since the last good
    /// check.
    pub auth_failed: bool,
}

/// Runtime state for every account. Lives in [`super::AccountRegistry`].
#[derive(Default)]
pub struct AccountRuntime {
    status: Mutex<HashMap<String, AccountStatus>>,
    /// Each account's sign-in run, running or last finished. At most one
    /// runs at a time, across all accounts ([`AccountRuntime::begin_sign_in`]).
    sign_ins: Mutex<HashMap<String, Arc<SignInJob>>>,
    /// Whether a token account's token is in the secret store (asked once).
    pub(super) tokens: Mutex<HashMap<String, bool>>,
    /// The account a session was last seen running under (live sources
    /// only), so an ended one keeps its chip until the app restarts.
    seen: Mutex<HashMap<String, String>>,
    /// Hash of the last `accounts` frame sent.
    pub(super) last_published: Mutex<Option<u64>>,
}

impl std::fmt::Debug for AccountRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountRuntime").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl AccountRuntime {
    pub fn status(&self, id: &str) -> AccountStatus {
        lock(&self.status).get(id).cloned().unwrap_or_default()
    }

    pub(super) fn update(&self, id: &str, f: impl FnOnce(&mut AccountStatus)) {
        f(lock(&self.status).entry(id.to_string()).or_default());
    }

    /// Forget a removed account (its sessions included: they are found
    /// where their transcript is again).
    pub fn forget(&self, id: &str) {
        lock(&self.status).remove(id);
        lock(&self.tokens).remove(id);
        lock(&self.seen).retain(|_, account| account != id);
        if let Some(job) = lock(&self.sign_ins).remove(id) {
            job.cancel();
        }
    }

    /// Remember the account a live session runs under.
    pub fn remember(&self, session_id: &str, account: &str) {
        let mut seen = lock(&self.seen);
        if seen.get(session_id).map(String::as_str) != Some(account) {
            seen.insert(session_id.to_string(), account.to_string());
        }
    }

    /// The account a session was last seen running under.
    pub fn last_seen(&self, session_id: &str) -> Option<String> {
        lock(&self.seen).get(session_id).cloned()
    }

    /// Record a sign-in check made elsewhere (the Default's at startup).
    pub fn set_auth(&self, id: &str, auth: AuthStatus) {
        self.update(id, |s| {
            s.auth = Some(auth);
            s.checked_at = Some(now_ms());
        });
    }

    /// The sign-in run of an account, running or last finished.
    pub fn sign_in(&self, id: &str) -> Option<Arc<SignInJob>> {
        lock(&self.sign_ins).get(id).cloned()
    }

    /// Start a sign-in for `id` unless one runs (docs/ACCOUNTS.md §15.3): the
    /// account's own running one is handed back; another account's refuses
    /// with [`SignInError::Running`] (its name left for the caller). Checked
    /// and started under one lock, so two never run at once.
    pub fn begin_sign_in(
        &self,
        id: &str,
        start: impl FnOnce() -> Result<Arc<SignInJob>, SignInError>,
    ) -> Result<Arc<SignInJob>, SignInError> {
        let mut jobs = lock(&self.sign_ins);
        if let Some(running) = jobs.values().find(|j| j.is_running()) {
            if running.account == id {
                return Ok(running.clone());
            }
            return Err(SignInError::Running {
                account: running.account.clone(),
                name: String::new(),
            });
        }
        let job = start()?;
        jobs.insert(id.to_string(), job.clone());
        Ok(job)
    }
}

/// The account's sign-in now: asks the CLI with the account's environment.
/// A token account without its token is reported without asking.
pub async fn refresh(state: &AppState, id: &str) -> Result<AuthStatus, AccountError> {
    let account = state
        .accounts
        .get(id)
        .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
    let runtime = &state.accounts.runtime;
    runtime.update(id, |s| s.checking = true);
    super::view::publish(state).await;
    let status = if account.is_default() {
        state.refresh_claude_auth().await
    } else {
        match state.accounts.env(id) {
            Ok(env) => crate::claude::auth::status_for(&env).await,
            Err(AccountError::TokenMissing) => AuthStatus {
                error: Some(AccountError::TokenMissing.to_string()),
                ..Default::default()
            },
            Err(e) => {
                runtime.update(id, |s| s.checking = false);
                return Err(e);
            }
        }
    };
    // `auth status` does not ask Anthropic whether the credentials still
    // work, so a good answer never clears `auth_failed`: a new sign-in, a new
    // token or a turn that worked does.
    runtime.update(id, |s| {
        s.checking = false;
        s.checked_at = Some(now_ms());
        s.auth = Some(status.clone());
    });
    // Who it signed in as, kept the first time (docs/ACCOUNTS.md §15.3); a
    // different one is a warning on the account view, never kept unasked.
    state.accounts.learn_identities();
    super::view::publish(state).await;
    Ok(status)
}

/// Every account but the Default, in parallel (the Default is checked by the
/// server's startup as before; its answer is mirrored here).
pub async fn refresh_all(state: &AppState) {
    let ids: Vec<String> = state
        .accounts
        .accounts()
        .into_iter()
        .filter(|a| !a.is_default())
        .map(|a| a.id)
        .collect();
    let checks = ids.iter().map(|id| refresh(state, id));
    for (id, r) in ids.iter().zip(futures_util::future::join_all(checks).await) {
        if let Err(e) = r {
            tracing::debug!(account = %id, error = %e, "account sign-in check failed");
        }
    }
    // The hooks plugin in each directory account's own directory.
    let plugins = ids
        .iter()
        .map(|id| crate::claude::hooks::refresh_account(state, id));
    futures_util::future::join_all(plugins).await;
    super::view::publish(state).await;
}

/// A turn of `id` worked: whatever failed to authenticate before works now.
pub async fn note_auth_ok(state: &AppState, id: &str) {
    let mut changed = false;
    state.accounts.runtime.update(id, |s| {
        changed = s.auth_failed;
        s.auth_failed = false;
    });
    if changed {
        super::view::publish(state).await;
    }
}

/// A session of `id` failed to authenticate: show it on the account at once.
pub async fn note_auth_failed(state: &AppState, id: &str) {
    if state.accounts.get(id).is_none() {
        return;
    }
    let mut changed = false;
    state.accounts.runtime.update(id, |s| {
        changed = !s.auth_failed;
        s.auth_failed = true;
    });
    if changed {
        super::view::publish(state).await;
    }
}

/// Store a token the user pasted, after `claude auth status` with it accepts
/// it (it checks the shape and the sign-in it implies; whether the token
/// works shows on its first use).
pub async fn paste_token(state: &AppState, id: &str, token: Secret) -> Result<AuthStatus, String> {
    let account = state
        .accounts
        .get(id)
        .ok_or_else(|| AccountError::NotFound(id.to_string()).to_string())?;
    if account.kind != AccountKind::Token {
        return Err(AccountError::NotToken.to_string());
    }
    let status = crate::claude::auth::status_for(&super::AccountEnv::Token(token.clone())).await;
    if !status.logged_in {
        return Err(status
            .error
            .unwrap_or_else(|| "Claude Code does not accept this token.".to_string()));
    }
    state
        .accounts
        .set_token(id, &token)
        .map_err(|e| e.to_string())?;
    state.accounts.runtime.update(id, |s| {
        s.auth_failed = false;
        s.auth = Some(status.clone());
        s.checked_at = Some(now_ms());
    });
    super::view::publish(state).await;
    Ok(status)
}
