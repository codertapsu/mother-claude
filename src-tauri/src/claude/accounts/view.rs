//! The accounts as clients see them: `GET /api/accounts` and the `accounts`
//! WebSocket frame (docs/ACCOUNTS.md §4.4). Never a secret: a token account
//! says whether its token is stored here, not what it is; the sign-in URL of
//! a running `setup-token` is only in the desktop's own read.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

use serde::Serialize;

use super::capture::CaptureView;
use super::{AccountDefaults, AccountKind, CopyConsent, DEFAULT_ID};
use crate::claude::model::AccountRef;
use crate::claude::secrets::SecretStoreKind;
use crate::state::AppState;

/// What needs the user's attention on an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Attention {
    /// A token account whose token is not stored on this machine.
    TokenMissing,
    /// A session of it failed to authenticate.
    AuthFailed,
    /// `claude auth status` says it is signed out.
    SignedOut,
    /// The sign-in could not be checked.
    CheckFailed,
}

/// One account.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub name: String,
    pub kind: AccountKind,
    pub is_default: bool,
    /// Its config directory (the Default's as resolved; none for a token
    /// account, which uses the Default's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    pub color: String,
    pub created_at: i64,
    pub signed_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attention: Option<Attention>,
    pub checking: bool,
    pub signing_in: bool,
    /// A token account's token is in this machine's secret store.
    pub token_stored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks_plugin: Option<String>,
    /// Live sessions (only in `GET /accounts`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<u32>,
    /// A `setup-token` capture, running or last finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureView>,
}

/// The registry as clients see it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsView {
    pub accounts: Vec<AccountView>,
    pub defaults: AccountDefaults,
    pub copy_consent: Vec<CopyConsent>,
    /// Where secrets are kept on this machine.
    pub secret_store: SecretStoreKind,
    /// This build can run `claude setup-token` itself.
    pub token_capture: bool,
}

/// The Default's hooks plugin, as foreign control last saw it.
fn default_plugin_state(state: &AppState) -> Option<String> {
    state.plugin.summary().map(str::to_string)
}

/// Live sessions per account in the current snapshot.
pub async fn live_counts(state: &AppState) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    for s in state
        .snapshot
        .read()
        .await
        .sessions
        .iter()
        .filter(|s| s.live)
    {
        let id = s
            .account
            .as_ref()
            .map_or(DEFAULT_ID, |a| a.id.as_str())
            .to_string();
        *out.entry(id).or_insert(0) += 1;
    }
    out
}

/// Build the view. `desktop`: include a running capture's sign-in URL.
pub fn build(state: &AppState, desktop: bool, live: Option<&HashMap<String, u32>>) -> AccountsView {
    let reg = state.accounts.snapshot();
    let runtime = &state.accounts.runtime;
    let accounts = reg
        .accounts
        .iter()
        .map(|a| {
            let status = runtime.status(&a.id);
            let auth = status.auth.as_ref();
            let token_stored = a.kind == AccountKind::Token && state.accounts.has_token(&a.id);
            let dir = match (&a.kind, &a.dir) {
                (AccountKind::Directory, Some(d)) => Some(d.to_string_lossy().into_owned()),
                _ if a.is_default() => Some(
                    state
                        .accounts
                        .default_home()
                        .base()
                        .to_string_lossy()
                        .into_owned(),
                ),
                _ => None,
            };
            let attention = if a.kind == AccountKind::Token && !token_stored {
                Some(Attention::TokenMissing)
            } else if status.auth_failed {
                Some(Attention::AuthFailed)
            } else {
                match auth {
                    Some(x) if x.error.is_some() => Some(Attention::CheckFailed),
                    Some(x) if !x.logged_in => Some(Attention::SignedOut),
                    _ => None,
                }
            };
            let capture = runtime.capture(&a.id).map(|j| {
                let v = j.view();
                if desktop {
                    v
                } else {
                    v.public()
                }
            });
            AccountView {
                id: a.id.clone(),
                name: a.name.clone(),
                kind: a.kind,
                is_default: a.is_default(),
                dir,
                color: a.color_or_default().to_string(),
                created_at: a.created_at,
                signed_in: auth.is_some_and(|x| x.logged_in) && !status.auth_failed,
                email: auth.and_then(|x| x.email.clone()),
                subscription: auth.and_then(|x| x.subscription_type.clone()),
                auth_method: auth.and_then(|x| x.auth_method.clone()),
                auth_error: auth.and_then(|x| x.error.clone()),
                attention,
                checking: status.checking,
                signing_in: status.signing_in,
                token_stored,
                hooks_plugin: if a.is_default() {
                    default_plugin_state(state)
                } else {
                    a.hooks_plugin.clone()
                },
                live: live.map(|l| l.get(&a.id).copied().unwrap_or(0)),
                capture,
            }
        })
        .collect();
    AccountsView {
        accounts,
        defaults: reg.defaults.clone(),
        copy_consent: reg.copy_consent.clone(),
        secret_store: state.accounts.secret_store().kind(),
        token_capture: super::capture::AVAILABLE,
    }
}

/// `GET /api/accounts`.
pub async fn view(state: &AppState, desktop: bool) -> AccountsView {
    let live = live_counts(state).await;
    build(state, desktop, Some(&live))
}

fn hash_of(v: &AccountsView) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(v).unwrap_or_default().hash(&mut h);
    h.finish()
}

/// Send the `accounts` frame when what every client sees changed. With only
/// the Default nothing is sent (goal 7: a v0.4 setup gets exactly the frames
/// it always got — its sign-in trouble shows the way it always did), except
/// the one frame that tells clients the other accounts are gone.
pub async fn publish(state: &AppState) {
    let default_only = state.accounts.is_default_only();
    let view = build(state, false, None);
    let hash = hash_of(&view);
    {
        let mut last = state
            .accounts
            .runtime
            .last_published
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if default_only {
            // `None`: no frame since the last one with other accounts.
            if last.take().is_none() {
                return;
            }
        } else {
            if *last == Some(hash) {
                return;
            }
            *last = Some(hash);
        }
    }
    crate::server::ws::publish_accounts(state, view);
}

/// The frame a connecting client gets (none while only the Default exists,
/// so a v0.4 setup sends exactly what it always sent).
pub fn connect_frame(state: &AppState) -> Option<AccountsView> {
    (!state.accounts.is_default_only()).then(|| build(state, false, None))
}

/// A session's account chip. `None` while only the Default exists (goal 7:
/// every response as before).
pub fn account_ref(state: &AppState, id: &str) -> Option<AccountRef> {
    let refs = account_refs(state);
    refs.get(id).or_else(|| refs.get(DEFAULT_ID)).cloned()
}

/// The chip of a session that runs with a token no account holds.
fn unregistered_ref() -> AccountRef {
    AccountRef {
        id: super::UNREGISTERED_TOKEN_ID.to_string(),
        name: "Token (unregistered)".to_string(),
        color: "#94a3b8".to_string(),
        kind: AccountKind::Token,
    }
}

/// Every account's chip by account id, for labelling many sessions at once.
/// Empty while only the Default exists.
pub fn account_refs(state: &AppState) -> BTreeMap<String, AccountRef> {
    let reg = state.accounts.snapshot();
    if reg.is_default_only() {
        return BTreeMap::new();
    }
    let mut refs: BTreeMap<String, AccountRef> = reg
        .accounts
        .iter()
        .map(|a| {
            (
                a.id.clone(),
                AccountRef {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    color: a.color_or_default().to_string(),
                    kind: a.kind,
                },
            )
        })
        .collect();
    refs.insert(super::UNREGISTERED_TOKEN_ID.to_string(), unregistered_ref());
    refs
}
