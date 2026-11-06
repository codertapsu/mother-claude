//! Paired browsers and their `mcx_` tokens.
//!
//! `<app data dir>/browsers.json` (0600, atomic write through
//! `crate::persist`): `{browsers:[{id, name, browser:{name, version},
//! extensionVersion, origin, tokenSha256, createdAt, lastSeenAt, revoked,
//! revokedAt}]}`. Only the SHA-256 of a token is stored and compared (in
//! constant time); the token itself is shown once, in the `paired` frame.
//! `origin` pins the extension (`chrome-extension://<id>`): the token works
//! only from that extension. Revoked records are kept a while so their
//! extension is told `revoked` (and forgets its token) rather than
//! `unauthenticated`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::server::auth;

/// Prefix of a paired browser's token.
pub const TOKEN_PREFIX: &str = "mcx_";
/// Prefix of a paired browser's id.
pub const ID_PREFIX: &str = "brw_";
/// How long a revoked browser is remembered.
const REVOKED_KEEP_MS: i64 = 30 * 86_400_000;
/// `lastSeenAt` reaches the disk at most this often.
const FLUSH_EVERY: Duration = Duration::from_secs(60);

/// The browser an extension runs in, as it reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowserInfo {
    pub name: String,
    pub version: String,
}

impl BrowserInfo {
    /// From a frame's `browser` object (tolerant, sanitised).
    pub fn from_frame(v: Option<&Value>) -> Self {
        let field = |k: &str| {
            v.and_then(|b| b.get(k))
                .and_then(Value::as_str)
                .map(|s| crate::server::devices::sanitize_label(s, 40))
                .unwrap_or_default()
        };
        Self {
            name: field("name"),
            version: field("version"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Record {
    id: String,
    name: String,
    browser: BrowserInfo,
    extension_version: String,
    origin: String,
    token_sha256: String,
    created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seen_at: Option<i64>,
    revoked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_at: Option<i64>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    browsers: Vec<Record>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

/// A paired browser as the page shows it (never the token or its hash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserView {
    pub id: String,
    pub name: String,
    pub browser: BrowserInfo,
    pub extension_version: String,
    /// The pinned extension (`chrome-extension://<id>`).
    pub origin: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
    pub revoked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<i64>,
}

/// Why a token was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    /// Not a token we issued (or another extension's).
    Unknown,
    /// The user removed this browser.
    Revoked,
}

struct Inner {
    file: File,
    dirty: bool,
    last_flush: Instant,
}

/// The paired browsers.
pub struct BrowserStore {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for BrowserStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserStore")
            .field("browsers", &self.lock().file.browsers.len())
            .finish()
    }
}

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

impl BrowserStore {
    /// Load `browsers.json` (missing or corrupt → none); `None` keeps them in
    /// memory (tests).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join("browsers.json"));
        let mut file: File = path
            .as_deref()
            .and_then(crate::persist::read_json)
            .unwrap_or_default();
        file.browsers
            .retain(|b| b.id.starts_with(ID_PREFIX) && !b.token_sha256.is_empty());
        Self {
            path,
            inner: Mutex::new(Inner {
                file,
                dirty: false,
                last_flush: Instant::now(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn persist(&self, g: &mut Inner) {
        g.dirty = false;
        g.last_flush = Instant::now();
        let Some(path) = &self.path else { return };
        if let Err(e) = crate::persist::write_json_private(path, &g.file) {
            tracing::warn!(error = %e, "could not save browsers.json");
        }
    }

    fn view(r: &Record) -> BrowserView {
        BrowserView {
            id: r.id.clone(),
            name: r.name.clone(),
            browser: r.browser.clone(),
            extension_version: r.extension_version.clone(),
            origin: r.origin.clone(),
            created_at: r.created_at,
            last_seen_at: r.last_seen_at,
            revoked: r.revoked,
            revoked_at: r.revoked_at,
        }
    }

    /// Pair a browser: its record and its token (shown exactly once).
    pub fn issue(
        &self,
        name: &str,
        browser: BrowserInfo,
        extension_version: &str,
        origin: &str,
    ) -> (BrowserView, String) {
        let token = auth::random_token(TOKEN_PREFIX);
        let now = now_ms();
        let mut name = crate::server::devices::sanitize_label(name, 64);
        if name.is_empty() {
            name = if browser.name.is_empty() {
                "Browser".to_string()
            } else {
                browser.name.clone()
            };
        }
        let record = Record {
            id: format!("{ID_PREFIX}{}", uuid::Uuid::new_v4().simple()),
            name,
            browser,
            extension_version: crate::server::devices::sanitize_label(extension_version, 24),
            origin: origin.to_string(),
            token_sha256: auth::sha256_hex(&token),
            created_at: now,
            last_seen_at: Some(now),
            ..Record::default()
        };
        let view = Self::view(&record);
        let mut g = self.lock();
        g.file.browsers.push(record);
        self.persist(&mut g);
        (view, token)
    }

    /// The browser a token belongs to, presented by the extension `origin`.
    /// A token from any other extension is unknown (the id is pinned).
    pub fn authenticate(&self, token: &str, origin: &str) -> Result<BrowserView, AuthFailure> {
        if !token.starts_with(TOKEN_PREFIX) || token.len() > 128 {
            return Err(AuthFailure::Unknown);
        }
        let hash = auth::sha256_hex(token);
        let g = self.lock();
        let mut found = None;
        for (i, r) in g.file.browsers.iter().enumerate() {
            // Constant time per comparison; every record is checked.
            if auth::token_matches(&r.token_sha256, &hash) && found.is_none() {
                found = Some(i);
            }
        }
        let r = &g.file.browsers[found.ok_or(AuthFailure::Unknown)?];
        if r.origin != origin {
            return Err(AuthFailure::Unknown);
        }
        if r.revoked {
            return Err(AuthFailure::Revoked);
        }
        Ok(Self::view(r))
    }

    /// Note a connection: last seen now, and the versions it reported.
    pub fn touch(&self, id: &str, browser: Option<&BrowserInfo>, extension_version: Option<&str>) {
        let mut g = self.lock();
        let Some(r) = g.file.browsers.iter_mut().find(|r| r.id == id) else {
            return;
        };
        r.last_seen_at = Some(now_ms());
        let mut changed = false;
        if let Some(b) = browser.filter(|b| !b.name.is_empty() && **b != r.browser) {
            r.browser = b.clone();
            changed = true;
        }
        if let Some(v) = extension_version
            .map(|v| crate::server::devices::sanitize_label(v, 24))
            .filter(|v| !v.is_empty() && *v != r.extension_version)
        {
            r.extension_version = v;
            changed = true;
        }
        g.dirty = true;
        if changed || g.last_flush.elapsed() >= FLUSH_EVERY {
            self.persist(&mut g);
        }
    }

    /// Revoke a browser. `false` when there is no such (unrevoked) browser.
    pub fn revoke(&self, id: &str) -> Option<BrowserView> {
        let mut g = self.lock();
        let r = g
            .file
            .browsers
            .iter_mut()
            .find(|r| r.id == id && !r.revoked)?;
        r.revoked = true;
        r.revoked_at = Some(now_ms());
        let view = Self::view(r);
        self.persist(&mut g);
        Some(view)
    }

    pub fn get(&self, id: &str) -> Option<BrowserView> {
        self.lock()
            .file
            .browsers
            .iter()
            .find(|r| r.id == id)
            .map(Self::view)
    }

    /// Every browser, unrevoked first, newest first.
    pub fn list(&self) -> Vec<BrowserView> {
        let mut out: Vec<BrowserView> = self.lock().file.browsers.iter().map(Self::view).collect();
        out.sort_by(|a, b| {
            a.revoked
                .cmp(&b.revoked)
                .then(b.created_at.cmp(&a.created_at))
        });
        out
    }

    /// Unrevoked browsers.
    pub fn active_count(&self) -> usize {
        self.lock()
            .file
            .browsers
            .iter()
            .filter(|r| !r.revoked)
            .count()
    }

    /// Forget revoked browsers after a month; write pending `lastSeenAt`.
    pub fn sweep(&self) {
        let now = now_ms();
        let mut g = self.lock();
        let before = g.file.browsers.len();
        g.file
            .browsers
            .retain(|r| !r.revoked || now - r.revoked_at.unwrap_or(now) < REVOKED_KEEP_MS);
        if g.file.browsers.len() != before || (g.dirty && g.last_flush.elapsed() >= FLUSH_EVERY) {
            self.persist(&mut g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXT: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";

    fn chrome() -> BrowserInfo {
        BrowserInfo {
            name: "Chrome".into(),
            version: "156.0".into(),
        }
    }

    #[test]
    fn tokens_are_stored_hashed_pinned_and_revocable() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserStore::load(Some(dir.path()));
        let (view, token) = store.issue("Chrome on MacBook\u{7}", chrome(), "0.1.0", EXT);
        assert!(token.starts_with(TOKEN_PREFIX));
        assert!(view.id.starts_with(ID_PREFIX));
        assert_eq!(view.name, "Chrome on MacBook");
        let text = std::fs::read_to_string(dir.path().join("browsers.json")).unwrap();
        assert!(!text.contains(&token), "only the hash is stored");
        assert!(text.contains(&auth::sha256_hex(&token)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("browsers.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        assert_eq!(store.authenticate(&token, EXT).unwrap().id, view.id);
        // Another extension id, a wrong token, a non-token: unknown.
        let other = "chrome-extension://ppppppppppppppppppppppppppppppppp";
        assert_eq!(
            store.authenticate(&token, other).unwrap_err(),
            AuthFailure::Unknown
        );
        assert_eq!(
            store.authenticate("mcx_nope", EXT).unwrap_err(),
            AuthFailure::Unknown
        );
        assert_eq!(
            store.authenticate("mcd_x", EXT).unwrap_err(),
            AuthFailure::Unknown
        );

        // It survives a restart.
        let again = BrowserStore::load(Some(dir.path()));
        assert_eq!(again.authenticate(&token, EXT).unwrap().name, view.name);

        // Revoked: told so, and listed last.
        let (second, _) = store.issue("", chrome(), "0.1.0", EXT);
        assert_eq!(second.name, "Chrome");
        assert!(store.revoke(&view.id).is_some());
        assert!(store.revoke(&view.id).is_none(), "once");
        assert_eq!(
            store.authenticate(&token, EXT).unwrap_err(),
            AuthFailure::Revoked
        );
        let list = store.list();
        assert_eq!(list[0].id, second.id);
        assert!(list[1].revoked);
        assert_eq!(store.active_count(), 1);
    }

    #[test]
    fn touch_records_versions() {
        let store = BrowserStore::load(None);
        let (view, _) = store.issue("Edge", chrome(), "0.1.0", EXT);
        store.touch(
            &view.id,
            Some(&BrowserInfo {
                name: "Edge".into(),
                version: "157.0".into(),
            }),
            Some("0.2.0"),
        );
        let v = store.get(&view.id).unwrap();
        assert_eq!(v.browser.version, "157.0");
        assert_eq!(v.extension_version, "0.2.0");
        assert!(v.last_seen_at.is_some());
    }
}
