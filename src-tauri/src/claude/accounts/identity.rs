//! Who an account signed in as (docs/ACCOUNTS.md §15.3): every token stays
//! with its account, and a sign-in that landed in the wrong one says so.
//!
//! Where the CLI can say: `claude auth status --json` in a Default or
//! directory account's environment gives the email (a token account's says
//! only `oauth_token`, so a token account has no identity here). The registry
//! keeps the email's SHA-256 (`identitySha256`), never the email: learned
//! from the account's first check that shows one, unless another account
//! signed in as the same, and replaced only when the user keeps a new one
//! (`POST /api/accounts/{id}/identity/accept`).
//!
//! The warnings are derived, never stored: an account whose sign-in now shows
//! a different email than the one kept is `changed`; one signed in as the same
//! Claude account as another account here (kept or shown) is `same-as` that
//! one, unless the user kept it so. Nothing is learned or warned while only the
//! Default exists (goal 7: that setup never gets an `accounts.json`).

use std::collections::BTreeMap;

use serde::Serialize;

use super::{AccountRegistry, Registry};
use crate::claude::secrets::sha256_hex;

/// The SHA-256 an email is kept as (trimmed, lowercased).
pub fn email_sha256(email: &str) -> String {
    sha256_hex(email.trim().to_lowercase().as_bytes())
}

/// What is wrong with who an account signed in as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum WarningKind {
    /// Not who it signed in as before.
    #[serde(rename = "changed")]
    Changed,
    /// The same Claude account as another account here.
    #[serde(rename = "same-as")]
    SameAs,
}

/// `identityWarning` on the account view.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct IdentityWarning {
    pub kind: WarningKind,
    /// `same-as`: the other account's id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// The other account (in registry order) signed in, or kept, as `sha`.
fn same_as(
    reg: &Registry,
    current: &BTreeMap<String, String>,
    id: &str,
    sha: &str,
) -> Option<String> {
    reg.accounts
        .iter()
        .filter(|a| a.id != id)
        .find(|a| {
            a.identity_sha256.as_deref() == Some(sha)
                || current.get(&a.id).map(String::as_str) == Some(sha)
        })
        .map(|a| a.id.clone())
}

/// Every account's warning (`current`: each account's shown identity).
pub fn warnings(
    reg: &Registry,
    current: &BTreeMap<String, String>,
) -> BTreeMap<String, IdentityWarning> {
    if reg.is_default_only() {
        return BTreeMap::new();
    }
    let mut out = BTreeMap::new();
    for a in &reg.accounts {
        let Some(sha) = current.get(&a.id) else {
            continue;
        };
        if a.identity_sha256.as_deref() == Some(sha.as_str()) {
            continue;
        }
        let other = same_as(reg, current, &a.id, sha);
        let warning = match (&a.identity_sha256, other) {
            (_, Some(other)) => IdentityWarning {
                kind: WarningKind::SameAs,
                account: Some(other),
            },
            (Some(_), None) => IdentityWarning {
                kind: WarningKind::Changed,
                account: None,
            },
            (None, None) => continue,
        };
        out.insert(a.id.clone(), warning);
    }
    out
}

/// The identities to keep without asking: accounts that keep none yet,
/// signed in as nobody else here.
pub fn to_learn(reg: &Registry, current: &BTreeMap<String, String>) -> Vec<(String, String)> {
    if reg.is_default_only() {
        return Vec::new();
    }
    reg.accounts
        .iter()
        .filter(|a| a.identity_sha256.is_none())
        .filter_map(|a| {
            let sha = current.get(&a.id)?;
            same_as(reg, current, &a.id, sha)
                .is_none()
                .then(|| (a.id.clone(), sha.clone()))
        })
        .collect()
}

impl AccountRegistry {
    /// Each account's shown identity: its last `claude auth status`, when it
    /// is signed in and names an email.
    pub(super) fn current_identities(&self, reg: &Registry) -> BTreeMap<String, String> {
        reg.accounts
            .iter()
            .filter_map(|a| {
                let auth = self.runtime.status(&a.id).auth?;
                let email = auth.email.filter(|_| auth.logged_in)?;
                Some((a.id.clone(), email_sha256(&email)))
            })
            .collect()
    }

    /// Every account's identity warning now.
    pub fn identity_warnings(&self) -> BTreeMap<String, IdentityWarning> {
        let reg = self.snapshot();
        warnings(&reg, &self.current_identities(&reg))
    }

    /// Keep the identities there is nothing to ask about (after a sign-in
    /// check). Saved only when something was learned; returns how many.
    pub fn learn_identities(&self) -> usize {
        let mut guard = self.write();
        let current = self.current_identities(&guard);
        let learned = to_learn(&guard, &current);
        if learned.is_empty() {
            return 0;
        }
        let mut next = guard.clone();
        for (id, sha) in &learned {
            if let Some(a) = next.get_mut(id) {
                a.identity_sha256 = Some(sha.clone());
            }
        }
        match self.commit(&mut guard, next) {
            Ok(()) => learned.len(),
            Err(e) => {
                tracing::debug!(error = %e, "could not record who an account signed in as");
                0
            }
        }
    }

    /// The user keeps who `id` is signed in as now (its warning goes).
    /// Returns false when its sign-in shows no email to keep.
    pub fn accept_identity(&self, id: &str) -> Result<bool, super::AccountError> {
        let mut guard = self.write();
        let Some(sha) = self.current_identities(&guard).remove(id) else {
            return Ok(false);
        };
        let mut next = guard.clone();
        let account = next
            .get_mut(id)
            .ok_or_else(|| super::AccountError::NotFound(id.to_string()))?;
        if account.identity_sha256.as_deref() == Some(sha.as_str()) {
            return Ok(true);
        }
        account.identity_sha256 = Some(sha);
        self.commit(&mut guard, next)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::accounts::{Account, AccountKind};

    fn account(id: &str, identity: Option<&str>) -> Account {
        Account {
            id: id.to_string(),
            name: id.to_string(),
            kind: AccountKind::Directory,
            dir: None,
            color: None,
            created_at: 0,
            hooks_plugin: None,
            token_sha256: None,
            account_uuid: None,
            identity_sha256: identity.map(email_sha256),
            extra: BTreeMap::new(),
        }
    }

    fn registry(accounts: Vec<Account>) -> Registry {
        Registry {
            accounts,
            ..Default::default()
        }
    }

    fn shown(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(id, email)| (id.to_string(), email_sha256(email)))
            .collect()
    }

    #[test]
    fn an_email_is_kept_as_its_hash_whatever_its_case() {
        assert_eq!(
            email_sha256(" Me@Example.TEST "),
            email_sha256("me@example.test")
        );
        assert_eq!(email_sha256("me@example.test").len(), 64);
        assert!(!email_sha256("me@example.test").contains("example"));
    }

    #[test]
    fn a_first_sign_in_is_learned_unless_another_account_is_the_same() {
        let reg = registry(vec![
            account("default", None),
            account("acc_aaaaaaaaaaaa", None),
            account("acc_bbbbbbbbbbbb", None),
        ]);
        // Each its own: learned, no warning.
        let current = shown(&[
            ("default", "me@example.test"),
            ("acc_aaaaaaaaaaaa", "work@example.test"),
        ]);
        assert!(warnings(&reg, &current).is_empty());
        assert_eq!(to_learn(&reg, &current).len(), 2);
        // Two showing the same: neither learned, both warned.
        let current = shown(&[
            ("default", "me@example.test"),
            ("acc_aaaaaaaaaaaa", "ME@example.test"),
        ]);
        assert!(to_learn(&reg, &current).is_empty());
        let w = warnings(&reg, &current);
        assert_eq!(
            w["acc_aaaaaaaaaaaa"],
            IdentityWarning {
                kind: WarningKind::SameAs,
                account: Some("default".into())
            }
        );
        assert_eq!(w["default"].account.as_deref(), Some("acc_aaaaaaaaaaaa"));
    }

    #[test]
    fn a_kept_identity_warns_when_the_sign_in_changes() {
        let reg = registry(vec![
            account("default", Some("me@example.test")),
            account("acc_aaaaaaaaaaaa", Some("work@example.test")),
        ]);
        // As kept: nothing.
        assert!(warnings(&reg, &shown(&[("acc_aaaaaaaaaaaa", "work@example.test")])).is_empty());
        // Someone else: changed.
        let w = warnings(&reg, &shown(&[("acc_aaaaaaaaaaaa", "other@example.test")]));
        assert_eq!(
            w["acc_aaaaaaaaaaaa"],
            IdentityWarning {
                kind: WarningKind::Changed,
                account: None
            }
        );
        // The Default's own Claude account (kept there, even signed out now):
        // same-as, which says more than changed.
        let w = warnings(&reg, &shown(&[("acc_aaaaaaaaaaaa", "me@example.test")]));
        assert_eq!(w["acc_aaaaaaaaaaaa"].kind, WarningKind::SameAs);
        assert_eq!(w["acc_aaaaaaaaaaaa"].account.as_deref(), Some("default"));
        assert!(!w.contains_key("default"), "the Default is as it was kept");
        // Nothing is learned over a kept identity.
        assert!(to_learn(&reg, &shown(&[("acc_aaaaaaaaaaaa", "x@example.test")])).is_empty());
    }

    #[test]
    fn a_kept_same_as_stays_quiet_and_signed_out_accounts_say_nothing() {
        let reg = registry(vec![
            account("default", Some("me@example.test")),
            account("acc_aaaaaaaaaaaa", Some("me@example.test")),
        ]);
        let current = shown(&[
            ("default", "me@example.test"),
            ("acc_aaaaaaaaaaaa", "me@example.test"),
        ]);
        assert!(warnings(&reg, &current).is_empty(), "the user kept it so");
        assert!(warnings(&reg, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn only_the_default_learns_and_warns_nothing() {
        let reg = registry(vec![account("default", Some("me@example.test"))]);
        let current = shown(&[("default", "other@example.test")]);
        assert!(warnings(&reg, &current).is_empty());
        let reg = registry(vec![account("default", None)]);
        assert!(to_learn(&reg, &current).is_empty());
    }

    #[test]
    fn a_warning_reads_as_the_ui_expects() {
        let w = IdentityWarning {
            kind: WarningKind::SameAs,
            account: Some("default".into()),
        };
        assert_eq!(
            serde_json::to_value(&w).unwrap(),
            serde_json::json!({"kind": "same-as", "account": "default"})
        );
        let w = IdentityWarning {
            kind: WarningKind::Changed,
            account: None,
        };
        assert_eq!(
            serde_json::to_value(&w).unwrap(),
            serde_json::json!({"kind": "changed"})
        );
    }
}
