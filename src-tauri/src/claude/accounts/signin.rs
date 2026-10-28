//! Signing an account in (docs/ACCOUNTS.md §15.2–15.3): `claude auth login`
//! for the Default and directory accounts, `claude setup-token` for token
//! accounts, each run in a pseudo-terminal ([`super::capture`]) with
//! `BROWSER=true`, so the CLI opens nothing itself and Mother Claude opens the
//! link it shows ([`crate::claude::browser`]).
//!
//! * A login opens the default browser on its own as soon as its link is
//!   known; who signed in is checked afterwards (`claude auth status` in the
//!   account's environment). A re-sign-in of an account whose email is known
//!   passes `--email`, so the page asks for that account.
//! * A token sign-in opens nothing on its own: its account cannot be checked
//!   afterwards, so the desktop asks first (the private window recommended).
//! * One sign-in runs at a time, across all accounts. A run is bound to the
//!   account that started it from the first byte: its token goes to that
//!   account's secret only ([`super::AccountRegistry::set_token`], which also
//!   refuses a token another account holds).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::capture::{
    self, RunSpec, SignInError, SignInJob, SignInKind, SignInSink, SignInView, LINK_TIMEOUT,
    SIGN_IN_TIMEOUT,
};
use super::{AccountEnv, AccountError, AccountKind, EnvTarget, ACCOUNT_OVERRIDE_ENV};
use crate::claude::browser::{Browser, OpenError};
use crate::claude::secrets::Secret;
use crate::state::AppState;

/// The variable the CLI's opener runs (`$BROWSER <url>`; CLI 2.1.312) …
pub const BROWSER_ENV: &str = "BROWSER";
/// … and the no-op it runs instead, so it opens nothing.
pub const BROWSER_NOOP: &str = "true";

/// Longest email passed to `--email`.
const EMAIL_MAX: usize = 254;

/// Why a sign-in could not start, take input or open its link. Never
/// carries a secret.
#[derive(Debug, thiserror::Error)]
pub enum Failure {
    #[error(transparent)]
    Account(#[from] AccountError),
    #[error(transparent)]
    SignIn(#[from] SignInError),
    #[error(transparent)]
    Browser(#[from] OpenError),
    #[error("A token account signs in with `claude setup-token`: get or paste a token instead.")]
    TokenAccount,
}

/// What a login asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginOptions {
    /// An Anthropic Console account (API billing) instead of a Claude
    /// subscription (`--console`).
    pub console: bool,
    /// The email the sign-in page asks for (`--email`); `None`: the
    /// account's known one, if any.
    pub email: Option<String>,
}

/// The scrubbed environment every sign-in starts from: the parent session's
/// identity and our own token gone (`control::SCRUBBED_ENV`), never marked
/// owned, and `BROWSER=true` (set last, so nothing undoes it).
fn compose(spec: &mut RunSpec, account: impl FnOnce(&mut RunSpec)) {
    for key in crate::claude::control::SCRUBBED_ENV {
        spec.remove_var(key);
    }
    spec.remove_var(crate::claude::control::OWNED_ENV);
    account(spec);
    spec.set_var(BROWSER_ENV, OsStr::new(BROWSER_NOOP));
}

/// `claude auth login` for an account: `dir` is a directory account's
/// directory (`CLAUDE_CONFIG_DIR`, the overrides removed: D5); `None` is the
/// Default, which keeps the environment as the app found it — its
/// `CLAUDE_CONFIG_DIR` too, never set by us.
pub fn login_spec(
    bin: &str,
    dir: Option<&Path>,
    console: bool,
    email: Option<&str>,
    cwd: Option<PathBuf>,
) -> RunSpec {
    let mut spec = RunSpec {
        program: bin.to_string(),
        args: vec!["auth".into(), "login".into()],
        cwd,
        env: Vec::new(),
    };
    if console {
        spec.args.push("--console".into());
    }
    if let Some(email) = email {
        spec.args.push("--email".into());
        spec.args.push(email.into());
    }
    compose(&mut spec, |spec| {
        if let Some(dir) = dir {
            AccountEnv::Directory(dir.to_path_buf()).apply(spec);
        }
    });
    spec
}

/// `claude setup-token` for a token account: the Default's directory with the
/// inherited token, API key and provider switches removed — a clean
/// `setup-token`, whose token belongs to whoever signs in in the browser.
pub fn token_spec(bin: &str, cwd: Option<PathBuf>) -> RunSpec {
    let mut spec = RunSpec {
        program: bin.to_string(),
        args: vec!["setup-token".into()],
        cwd,
        env: Vec::new(),
    };
    compose(&mut spec, |spec| {
        for key in ACCOUNT_OVERRIDE_ENV {
            spec.remove_var(key);
        }
    });
    spec
}

/// An email for `--email`: one `local@domain` without spaces or control
/// characters, nothing the CLI could take for an option.
pub fn clean_email(raw: &str) -> Result<String, SignInError> {
    let email = raw.trim();
    let shaped = email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && !domain.contains('@')
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    });
    if !shaped
        || email.len() > EMAIL_MAX
        || email.starts_with('-')
        || email.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(SignInError::Invalid(
            "that does not look like an email address".to_string(),
        ));
    }
    Ok(email.to_string())
}

/// The email a re-sign-in asks for: the one the account is signed in as.
fn known_email(state: &AppState, id: &str) -> Option<String> {
    state
        .accounts
        .runtime
        .status(id)
        .auth
        .filter(|a| a.logged_in)
        .and_then(|a| a.email)
        .and_then(|e| clean_email(&e).ok())
}

/// Start a run of `kind` for `id` unless one runs (its own running one is
/// handed back).
fn begin(
    state: &AppState,
    id: &str,
    kind: SignInKind,
    spec: &RunSpec,
) -> Result<Arc<SignInJob>, Failure> {
    let sink = Arc::new(Sink {
        state: state.clone(),
        runtime: tokio::runtime::Handle::current(),
    });
    let auto_open = kind == SignInKind::Login;
    state
        .accounts
        .runtime
        .begin_sign_in(id, || {
            capture::start(
                id,
                kind,
                spec,
                auto_open,
                sink,
                SIGN_IN_TIMEOUT,
                LINK_TIMEOUT,
            )
        })
        .map_err(|e| match e {
            SignInError::Running { account, .. } => {
                let name = state
                    .accounts
                    .get(&account)
                    .map(|a| a.name)
                    .unwrap_or_default();
                Failure::SignIn(SignInError::Running { account, name })
            }
            other => Failure::SignIn(other),
        })
}

/// `claude auth login` for the Default or a directory account. The default
/// browser opens on its own once the link is known; the run ends when the
/// CLI does and `claude auth status` agrees.
pub async fn start_login(
    state: &AppState,
    id: &str,
    options: LoginOptions,
) -> Result<SignInView, Failure> {
    let account = state
        .accounts
        .get(id)
        .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
    if account.kind == AccountKind::Token {
        return Err(Failure::TokenAccount);
    }
    let dir = match state.accounts.env(id)? {
        AccountEnv::Directory(dir) => Some(dir),
        _ => None,
    };
    let email = match options.email.as_deref() {
        Some(e) => Some(clean_email(e)?),
        None => known_email(state, id),
    };
    let spec = login_spec(
        &crate::claude::claude_bin(),
        dir.as_deref(),
        options.console,
        email.as_deref(),
        crate::claude::home::user_home_dir(),
    );
    let job = begin(state, id, SignInKind::Login, &spec)?;
    super::view::publish(state).await;
    Ok(job.view())
}

/// `claude setup-token` for a token account; the token it shows is stored
/// into this account's secret. Nothing opens on its own.
pub async fn start_token(state: &AppState, id: &str) -> Result<SignInJobView, Failure> {
    let account = state
        .accounts
        .get(id)
        .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
    if account.kind != AccountKind::Token {
        return Err(AccountError::NotToken.into());
    }
    let spec = token_spec(
        &crate::claude::claude_bin(),
        crate::claude::home::user_home_dir(),
    );
    let job = begin(state, id, SignInKind::Token, &spec)?;
    super::view::publish(state).await;
    Ok(SignInJobView::of(&job))
}

/// A run's two views: the sign-in and, for a token run, the capture the
/// token routes have always answered with.
#[derive(Debug, Clone)]
pub struct SignInJobView {
    pub sign_in: SignInView,
    pub capture: Option<capture::CaptureView>,
}

impl SignInJobView {
    fn of(job: &SignInJob) -> Self {
        Self {
            sign_in: job.view(),
            capture: (job.kind == SignInKind::Token).then(|| job.capture_view()),
        }
    }
}

/// The account's running sign-in.
fn running(state: &AppState, id: &str) -> Result<Arc<SignInJob>, SignInError> {
    state
        .accounts
        .runtime
        .sign_in(id)
        .filter(|j| j.is_running())
        .ok_or(SignInError::NotRunning)
}

/// Type the code the browser shows into the account's running sign-in.
pub fn submit_code(state: &AppState, id: &str, code: &str) -> Result<SignInJobView, Failure> {
    let job = running(state, id)?;
    job.submit_code(code)?;
    Ok(SignInJobView::of(&job))
}

/// Stop the account's sign-in, if one runs; its last run's view either way
/// (the token routes' answer).
pub fn cancel(state: &AppState, id: &str) -> Option<SignInJobView> {
    let job = state.accounts.runtime.sign_in(id)?;
    job.cancel();
    Some(SignInJobView::of(&job))
}

/// Stop the account's running sign-in ([`SignInError::NotRunning`] when none
/// runs).
pub fn cancel_running(state: &AppState, id: &str) -> Result<SignInView, Failure> {
    let job = running(state, id)?;
    job.cancel();
    Ok(job.view())
}

/// Open the running sign-in's link in `browser`: only a link that passed the
/// allowlist when it was read ever gets here (the opener takes nothing else).
pub fn open(state: &AppState, id: &str, browser: Browser) -> Result<SignInView, Failure> {
    let job = running(state, id)?;
    let link = job.open_link()?;
    state.opener.open(browser, &link)?;
    job.note_opened(browser);
    Ok(job.view())
}

/// The browsers a link can be opened in on this machine.
pub fn browsers(state: &AppState) -> Vec<Browser> {
    state.opener.installed().list()
}

/// Where a run reports to: the secret store, the opener and the `accounts`
/// frame.
struct Sink {
    state: AppState,
    runtime: tokio::runtime::Handle,
}

impl Sink {
    fn publish(&self) {
        let st = self.state.clone();
        self.runtime
            .spawn(async move { super::view::publish(&st).await });
    }
}

impl SignInSink for Sink {
    fn changed(&self, job: &Arc<SignInJob>) {
        // A login opens the default browser by itself, once, as soon as its
        // link is known (what the user asked for); a failure leaves the
        // buttons and *Copy link* to the desktop.
        if let Some(link) = job.take_auto_open() {
            match self.state.opener.open(Browser::Default, &link) {
                Ok(()) => job.note_opened(Browser::Default),
                Err(e) => {
                    tracing::warn!(account = %job.account, error = %e, "could not open the sign-in page")
                }
            }
        }
        self.publish();
    }

    fn token(&self, account: &str, token: Secret) -> Result<(), String> {
        // `account` is the run's own: bound when it started, never another.
        self.state
            .accounts
            .set_token(account, &token)
            .map_err(|e| e.to_string())?;
        // What was known about the old (or missing) token no longer holds.
        self.state.accounts.runtime.update(account, |s| {
            s.auth_failed = false;
            s.auth = None;
        });
        tracing::info!(account = %account, "token account got a new token from setup-token");
        let st = self.state.clone();
        let id = account.to_string();
        self.runtime.spawn(async move {
            let _ = super::status::refresh(&st, &id).await;
        });
        Ok(())
    }

    fn exited(&self, job: &Arc<SignInJob>) {
        let st = self.state.clone();
        let job = job.clone();
        self.runtime.spawn(async move {
            let id = job.account.clone();
            // A login that finished well is a new sign-in even as the same
            // email (which is all `auth status` would show), so it clears an
            // "authentication failed" mark.
            let verdict = match super::status::refresh(&st, &id).await {
                Ok(a) if a.logged_in => {
                    st.accounts.runtime.update(&id, |s| s.auth_failed = false);
                    Ok(())
                }
                Ok(a) => Err(match a.error {
                    Some(e) => format!(
                        "`claude auth login` finished, but the sign-in could not be checked: {e}"
                    ),
                    None => "`claude auth login` finished, but the account is not signed in."
                        .to_string(),
                }),
                Err(e) => Err(e.to_string()),
            };
            job.complete(verdict);
            super::view::publish(&st).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::accounts::{CONFIG_DIR_ENV, OAUTH_TOKEN_ENV};

    fn set(spec: &RunSpec, key: &str) -> Option<String> {
        spec.env_change(key)
            .flatten()
            .map(|v| v.to_string_lossy().into_owned())
    }

    fn removed(spec: &RunSpec, key: &str) -> bool {
        spec.env_change(key) == Some(None)
    }

    #[test]
    fn a_directory_accounts_login_signs_its_own_directory_in() {
        let dir = PathBuf::from("/Users/x/.claude-work");
        let spec = login_spec("claude", Some(&dir), false, None, None);
        assert_eq!(spec.args, ["auth", "login"]);
        assert_eq!(
            set(&spec, CONFIG_DIR_ENV).as_deref(),
            Some("/Users/x/.claude-work")
        );
        assert_eq!(set(&spec, BROWSER_ENV).as_deref(), Some("true"));
        for key in ACCOUNT_OVERRIDE_ENV {
            assert!(removed(&spec, key), "{key} removed");
        }
        for key in crate::claude::control::SCRUBBED_ENV {
            assert!(removed(&spec, key), "{key} scrubbed");
        }
        assert!(removed(&spec, crate::claude::control::OWNED_ENV));
    }

    #[test]
    fn the_defaults_login_never_sets_its_directory() {
        let spec = login_spec(
            "claude",
            None,
            true,
            Some("me@example.test"),
            Some(PathBuf::from("/Users/x")),
        );
        assert_eq!(
            spec.args,
            ["auth", "login", "--console", "--email", "me@example.test"]
        );
        assert_eq!(
            spec.env_change(CONFIG_DIR_ENV),
            None,
            "inherited as the app found it: never set, never removed"
        );
        for key in ACCOUNT_OVERRIDE_ENV {
            assert_eq!(spec.env_change(key), None, "{key} left as found (D5)");
        }
        assert_eq!(set(&spec, BROWSER_ENV).as_deref(), Some("true"));
        assert!(removed(&spec, "MOTHER_CLAUDE_TOKEN"));
        assert_eq!(spec.cwd.as_deref(), Some(Path::new("/Users/x")));
    }

    #[test]
    fn a_token_sign_in_is_clean_and_carries_no_token() {
        let spec = token_spec("claude", None);
        assert_eq!(spec.args, ["setup-token"]);
        assert_eq!(
            spec.env_change(CONFIG_DIR_ENV),
            None,
            "the Default's directory"
        );
        for key in ACCOUNT_OVERRIDE_ENV {
            assert!(removed(&spec, key), "{key} removed");
        }
        assert!(removed(&spec, OAUTH_TOKEN_ENV));
        assert_eq!(set(&spec, BROWSER_ENV).as_deref(), Some("true"));
        assert!(spec
            .env
            .iter()
            .all(|(_, v)| v.as_deref().is_none_or(|v| v == OsStr::new(BROWSER_NOOP))));
    }

    #[test]
    fn emails_are_checked_before_they_reach_the_cli() {
        assert_eq!(
            clean_email("  me@example.test ").unwrap(),
            "me@example.test"
        );
        for bad in [
            "",
            "me",
            "@example.test",
            "me@",
            "me@example",
            "me@.example",
            "me@example.",
            "me@@example.test",
            "a@b@example.test",
            "me @example.test",
            "me@exa\nmple.test",
            "--sso@example.test",
        ] {
            assert!(clean_email(bad).is_err(), "{bad:?}");
        }
        assert!(clean_email(&format!("{}@example.test", "a".repeat(250))).is_err());
    }
}
