//! What is known about each account right now, beyond the registry: its
//! sign-in (`claude auth status` with the account's environment), whether a
//! session of it just failed to authenticate, a `claude auth login` being
//! watched, and a `setup-token` capture (docs/ACCOUNTS.md §5.8, P1.3).
//!
//! The Default account's sign-in stays `state.claude_auth` (checked at
//! startup as before); the others are checked in parallel at startup, after a
//! sign-in, and on demand.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::capture::{self, CaptureError, CaptureJob, CaptureSink, CaptureView};
use super::{AccountError, AccountKind, ACCOUNT_OVERRIDE_ENV};
use crate::claude::secrets::Secret;
use crate::claude::AuthStatus;
use crate::state::AppState;

/// How often a running `claude auth login` is re-checked, and for how long.
const LOGIN_POLL: Duration = Duration::from_secs(3);
const LOGIN_WATCH: Duration = Duration::from_secs(5 * 60);

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
    /// A `claude auth login` is being watched.
    pub signing_in: bool,
}

/// Runtime state for every account. Lives in [`super::AccountRegistry`].
#[derive(Default)]
pub struct AccountRuntime {
    status: Mutex<HashMap<String, AccountStatus>>,
    captures: Mutex<HashMap<String, Arc<CaptureJob>>>,
    watching: Mutex<HashSet<String>>,
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

    fn update(&self, id: &str, f: impl FnOnce(&mut AccountStatus)) {
        f(lock(&self.status).entry(id.to_string()).or_default());
    }

    /// Forget a removed account.
    pub fn forget(&self, id: &str) {
        lock(&self.status).remove(id);
        lock(&self.tokens).remove(id);
        if let Some(job) = lock(&self.captures).remove(id) {
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

    /// The capture of an account, running or last finished.
    pub fn capture(&self, id: &str) -> Option<Arc<CaptureJob>> {
        lock(&self.captures).get(id).cloned()
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

/// `claude auth login` with the account's environment, then watch until it
/// is signed in (every 3 s, for up to five minutes).
pub async fn start_login(state: &AppState, id: &str, console: bool) -> Result<(), String> {
    let account = state
        .accounts
        .get(id)
        .ok_or_else(|| AccountError::NotFound(id.to_string()).to_string())?;
    if account.kind == AccountKind::Token {
        return Err(
            "A token account signs in with `claude setup-token`: get or paste a token instead."
                .to_string(),
        );
    }
    let env = state.accounts.env(id).map_err(|e| e.to_string())?;
    crate::claude::auth::begin_login_for(console, &env)
        .await
        .map_err(|e| e.to_string())?;
    if !lock(&state.accounts.runtime.watching).insert(id.to_string()) {
        return Ok(());
    }
    state.accounts.runtime.update(id, |s| s.signing_in = true);
    super::view::publish(state).await;
    let st = state.clone();
    let id = id.to_string();
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + LOGIN_WATCH;
        let before = st.accounts.runtime.status(&id).auth.and_then(|a| a.email);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(LOGIN_POLL).await;
            match refresh(&st, &id).await {
                Ok(a) if a.logged_in && (a.email != before || before.is_none()) => {
                    st.accounts.runtime.update(&id, |s| s.auth_failed = false);
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        lock(&st.accounts.runtime.watching).remove(&id);
        st.accounts.runtime.update(&id, |s| s.signing_in = false);
        super::view::publish(&st).await;
    });
    Ok(())
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

/// Where a capture reports to: the secret store and the `accounts` frame.
struct Sink {
    state: AppState,
    account: String,
    runtime: tokio::runtime::Handle,
}

impl CaptureSink for Sink {
    fn changed(&self, _view: &CaptureView) {
        let st = self.state.clone();
        self.runtime
            .spawn(async move { super::view::publish(&st).await });
    }

    fn captured(&self, token: Secret) -> Result<(), String> {
        self.state
            .accounts
            .set_token(&self.account, &token)
            .map_err(|e| e.to_string())?;
        // What was known about the old (or missing) token no longer holds.
        self.state.accounts.runtime.update(&self.account, |s| {
            s.auth_failed = false;
            s.auth = None;
        });
        tracing::info!(account = %self.account, "token account got a new token from setup-token");
        let st = self.state.clone();
        let id = self.account.clone();
        self.runtime.spawn(async move {
            let _ = refresh(&st, &id).await;
        });
        Ok(())
    }
}

/// Run `claude setup-token` for a token account and store the token it shows.
pub fn start_capture(state: &AppState, id: &str) -> Result<CaptureView, String> {
    let account = state
        .accounts
        .get(id)
        .ok_or_else(|| AccountError::NotFound(id.to_string()).to_string())?;
    if account.kind != AccountKind::Token {
        return Err(AccountError::NotToken.to_string());
    }
    let runtime = &state.accounts.runtime;
    if let Some(job) = runtime.capture(id).filter(|j| j.is_running()) {
        return Ok(job.view());
    }
    let sink = Arc::new(Sink {
        state: state.clone(),
        account: id.to_string(),
        runtime: tokio::runtime::Handle::current(),
    });
    // The Default's directory, the inherited sign-in overrides removed: a
    // clean `claude setup-token`.
    let env = |target: &mut dyn super::EnvTarget| {
        for key in crate::claude::control::SCRUBBED_ENV {
            target.remove_var(key);
        }
        target.remove_var(crate::claude::control::OWNED_ENV);
        for key in ACCOUNT_OVERRIDE_ENV {
            target.remove_var(key);
        }
    };
    let job = capture::start(
        &crate::claude::claude_bin(),
        &env,
        sink,
        capture::CAPTURE_TIMEOUT,
    )
    .map_err(|e: CaptureError| e.to_string())?;
    let view = job.view();
    lock(&runtime.captures).insert(id.to_string(), job);
    Ok(view)
}

/// Type the code the browser shows into a running capture.
pub fn submit_code(state: &AppState, id: &str, code: &str) -> Result<CaptureView, String> {
    let job = state
        .accounts
        .runtime
        .capture(id)
        .ok_or_else(|| CaptureError::NotRunning.to_string())?;
    job.submit_code(code).map_err(|e| e.to_string())?;
    Ok(job.view())
}

/// Stop a running capture.
pub fn cancel_capture(state: &AppState, id: &str) -> Option<CaptureView> {
    let job = state.accounts.runtime.capture(id)?;
    job.cancel();
    Some(job.view())
}
