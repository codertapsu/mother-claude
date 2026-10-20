//! Which token account a session ran under, kept across restarts
//! (docs/ACCOUNTS.md §5.5).
//!
//! A directory account's sessions are found by where their transcript is; a
//! token account's live in the Default's directory, and after a restart
//! nothing else says whose they were — Continue, or a bridge `/chat` naming
//! the conversation, would run them under the Default. So the sessions seen
//! running under a token account are written to
//! `<app data dir>/account-sessions.json` (0600): only token accounts' (with
//! only the Default the file is never written), at most [`MAX`] of them, and
//! pruned of sessions whose transcript is gone.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

/// The file in the app data dir.
pub const FILE_NAME: &str = "account-sessions.json";
/// Most sessions kept (the most recently seen).
pub const MAX: usize = 2000;
/// An entry this young is kept even while the index does not know its
/// session (a conversation's transcript is written with its first answer).
const GRACE_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    account: String,
    /// When it was last recorded (ms).
    at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    sessions: BTreeMap<String, Entry>,
}

/// The persisted map.
#[derive(Debug, Default)]
pub struct SessionAccounts {
    path: Option<PathBuf>,
    map: Mutex<BTreeMap<String, Entry>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl SessionAccounts {
    /// Load `<data dir>/account-sessions.json`, keeping the entries of
    /// `is_token_account` accounts with a valid session id.
    pub fn load(
        data_dir: Option<&std::path::Path>,
        is_token_account: impl Fn(&str) -> bool,
    ) -> Self {
        let path = data_dir.map(|d| d.join(FILE_NAME));
        let mut map = path
            .as_deref()
            .and_then(crate::persist::read_json::<File>)
            .map(|f| f.sessions)
            .unwrap_or_default();
        map.retain(|sid, e| crate::claude::ids::is_session_id(sid) && is_token_account(&e.account));
        Self {
            path,
            map: Mutex::new(map),
        }
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Entry>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, map: &BTreeMap<String, Entry>) {
        let Some(path) = &self.path else { return };
        let file = File {
            version: 1,
            sessions: map.clone(),
        };
        if let Err(e) = crate::persist::write_json_private(path, &file) {
            tracing::debug!(error = %e, "could not save the session accounts");
        }
    }

    /// The token account `session_id` last ran under, if it was one.
    pub fn get(&self, session_id: &str) -> Option<String> {
        self.lock().get(session_id).map(|e| e.account.clone())
    }

    /// `session_id` runs under `account`; `token` says whether that is a
    /// token account. Saved only when something changed: a token account's
    /// session is recorded, a session seen under another account forgotten.
    pub fn note(&self, session_id: &str, account: &str, token: bool) {
        if !crate::claude::ids::is_session_id(session_id) {
            return;
        }
        let mut map = self.lock();
        let changed = if token {
            if map.get(session_id).is_some_and(|e| e.account == account) {
                false
            } else {
                map.insert(
                    session_id.to_string(),
                    Entry {
                        account: account.to_string(),
                        at: now_ms(),
                    },
                );
                if map.len() > MAX {
                    let mut by_age: Vec<(i64, String)> =
                        map.iter().map(|(k, e)| (e.at, k.clone())).collect();
                    by_age.sort();
                    for (_, k) in by_age.into_iter().take(map.len() - MAX) {
                        map.remove(&k);
                    }
                }
                true
            }
        } else {
            map.remove(session_id).is_some()
        };
        if changed {
            self.save(&map);
        }
    }

    /// Drop the sessions of `account` (removed).
    pub fn forget_account(&self, account: &str) {
        let mut map = self.lock();
        let before = map.len();
        map.retain(|_, e| e.account != account);
        if map.len() != before {
            self.save(&map);
        }
    }

    /// Drop the sessions `exists` no longer knows (their transcript is
    /// gone), except the ones recorded in the last hour.
    pub fn prune(&self, exists: impl Fn(&str) -> bool) {
        let mut map = self.lock();
        if map.is_empty() {
            return;
        }
        let cutoff = now_ms() - GRACE_MS;
        let before = map.len();
        map.retain(|sid, e| e.at >= cutoff || exists(sid));
        if map.len() != before {
            self.save(&map);
        }
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Backdate every entry (tests of the grace period).
    #[cfg(test)]
    fn age_all(&self, by_ms: i64) {
        for e in self.lock().values_mut() {
            e.at -= by_ms;
        }
    }

    /// Record `n` sessions without saving each (tests of the bound).
    #[cfg(test)]
    fn fill(&self, n: usize, account: &str) {
        let mut map = self.lock();
        for i in 0..n {
            map.insert(
                format!("0a1b2c3d-0000-4000-8000-{i:012}"),
                Entry {
                    account: account.to_string(),
                    at: now_ms() - (n - i) as i64,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0a1b2c3d-0000-4000-8000-0000000000a1";
    const B: &str = "0a1b2c3d-0000-4000-8000-0000000000b2";

    #[test]
    fn only_token_accounts_sessions_are_written_and_they_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let tokens = |a: &str| a == "acc_000000000001";
        let s = SessionAccounts::load(Some(dir.path()), tokens);
        s.note(A, "default", false);
        s.note(B, "acc_0000000000d1", false);
        assert!(!path.exists(), "nothing of a token account: no file");
        s.note(A, "acc_000000000001", true);
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        s.note("not-a-session", "acc_000000000001", true);
        let again = SessionAccounts::load(Some(dir.path()), tokens);
        assert_eq!(again.get(A).as_deref(), Some("acc_000000000001"));
        assert_eq!(again.len(), 1);
        // Seen under another account since: forgotten.
        again.note(A, "default", false);
        assert!(SessionAccounts::load(Some(dir.path()), tokens).is_empty());
        // An account that is no longer a token account is dropped at load.
        again.note(B, "acc_000000000001", true);
        assert!(SessionAccounts::load(Some(dir.path()), |_| false).is_empty());
        again.forget_account("acc_000000000001");
        assert!(SessionAccounts::load(Some(dir.path()), tokens).is_empty());
    }

    #[test]
    fn it_is_bounded_and_pruned_of_sessions_that_are_gone() {
        let dir = tempfile::tempdir().unwrap();
        let s = SessionAccounts::load(Some(dir.path()), |_| true);
        s.fill(MAX, "acc_000000000001");
        s.note(A, "acc_000000000001", true);
        assert_eq!(s.len(), MAX, "the oldest one gave way");
        assert!(s.get("0a1b2c3d-0000-4000-8000-000000000000").is_none());
        assert!(s.get(A).is_some());
        // Young entries are kept even when the index does not know them.
        s.prune(|_| false);
        assert_eq!(s.len(), MAX);
        s.age_all(GRACE_MS + 1000);
        let keep = "0a1b2c3d-0000-4000-8000-000000000010";
        s.prune(|sid| sid == keep);
        assert_eq!(s.len(), 1);
        assert!(s.get(keep).is_some());
    }

    #[test]
    fn without_a_data_dir_nothing_is_written() {
        let s = SessionAccounts::load(None, |_| true);
        s.note(A, "acc_000000000001", true);
        assert_eq!(s.get(A).as_deref(), Some("acc_000000000001"));
    }
}
