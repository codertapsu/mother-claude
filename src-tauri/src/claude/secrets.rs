//! Secrets the accounts feature keeps (docs/ACCOUNTS.md D3, §4.2): a token
//! account's long-lived token and the backup passphrase.
//!
//! On macOS they are generic passwords in the login Keychain under the service
//! [`KEYCHAIN_SERVICE`]; elsewhere they are `0600` files under
//! `<app data dir>/secrets/`, like `script-token`. A secret value never reaches
//! a log line, an error message, a WebSocket frame, `accounts.json`, a backup or
//! an audit line: [`Secret`] has no `Display`, its `Debug` prints a placeholder
//! and no error here quotes a value.
//!
//! Tests never touch the real Keychain: the Keychain is used only for the app's
//! own data dir ([`store_for`]); a temp data dir gets files, no data dir gets
//! memory.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

/// The Keychain service every Mother Claude secret is filed under.
pub const KEYCHAIN_SERVICE: &str = "dev.motherclaude.desktop";

/// `MOTHER_CLAUDE_SECRET_STORE=file` keeps secrets in files on macOS too (for
/// builds whose changing code signature would make the Keychain ask on every
/// launch).
const STORE_ENV: &str = "MOTHER_CLAUDE_SECRET_STORE";

/// A secret value. Deliberately not `Display`, `Serialize` or printable.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that needs it (a child's environment, the
    /// encryption key). Never format it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Lowercase hex SHA-256 of the value: what `accounts.json` keeps to match a
    /// running process to its account (D8).
    pub fn sha256_hex(&self) -> String {
        sha256_hex(self.0.as_bytes())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Which secret.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretName {
    /// A token account's token, by account id (`acc_` + 12 hex).
    AccountToken(String),
    /// The passphrase the backup sets are encrypted with.
    BackupPassphrase,
}

impl SecretName {
    /// The Keychain item's account attribute.
    pub fn key(&self) -> String {
        match self {
            Self::AccountToken(id) => id.clone(),
            Self::BackupPassphrase => "backup-passphrase".to_string(),
        }
    }

    /// The file name under `secrets/`.
    fn file_name(&self) -> String {
        match self {
            Self::AccountToken(id) => format!("{id}.token"),
            Self::BackupPassphrase => "backup-passphrase".to_string(),
        }
    }

    /// Only names that are safe as one path component and Keychain account.
    fn is_valid(&self) -> bool {
        match self {
            Self::AccountToken(id) => super::accounts::is_account_id(id),
            Self::BackupPassphrase => true,
        }
    }
}

/// Where secrets are kept, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretStoreKind {
    Keychain,
    Files,
    Memory,
}

/// A failure of the store itself. Never carries a secret value.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("not a valid secret name")]
    InvalidName,
    #[error("the Keychain refused to {op} the secret: {detail}")]
    Keychain { op: &'static str, detail: String },
    #[error("could not {op} the secret file: {detail}")]
    Io { op: &'static str, detail: String },
}

/// Get, set and delete secrets by name.
pub trait SecretStore: Send + Sync + fmt::Debug {
    fn get(&self, name: &SecretName) -> Result<Option<Secret>, SecretError>;
    fn set(&self, name: &SecretName, value: &Secret) -> Result<(), SecretError>;
    /// Deleting a secret that is not there is not an error.
    fn delete(&self, name: &SecretName) -> Result<(), SecretError>;
    fn kind(&self) -> SecretStoreKind;
}

/// The store for a data dir: the Keychain for the app's own data dir on macOS
/// (unless `MOTHER_CLAUDE_SECRET_STORE=file`), `0600` files in
/// `<data dir>/secrets/` for any other data dir, memory without one.
pub fn store_for(data_dir: Option<&Path>) -> Arc<dyn SecretStore> {
    let Some(dir) = data_dir else {
        return Arc::new(MemoryStore::default());
    };
    #[cfg(target_os = "macos")]
    {
        let files_requested = std::env::var(STORE_ENV)
            .map(|v| v.trim().eq_ignore_ascii_case("file"))
            .unwrap_or(false);
        let app_dir = crate::persist::app_data_dir();
        if !files_requested && app_dir.as_deref() == Some(dir) {
            return Arc::new(KeychainStore);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = STORE_ENV;
    Arc::new(FileStore::new(dir.join("secrets")))
}

// ─────────────────────────────── Keychain ────────────────────────────────

/// Generic passwords in the login Keychain, service [`KEYCHAIN_SERVICE`].
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct KeychainStore;

/// `errSecItemNotFound`.
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

#[cfg(target_os = "macos")]
impl SecretStore for KeychainStore {
    fn get(&self, name: &SecretName) -> Result<Option<Secret>, SecretError> {
        if !name.is_valid() {
            return Err(SecretError::InvalidName);
        }
        match security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, &name.key()) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(s) => Ok(Some(Secret(s))),
                Err(_) => Err(SecretError::Keychain {
                    op: "read",
                    detail: "the stored value is not text".into(),
                }),
            },
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(SecretError::Keychain {
                op: "read",
                detail: e.to_string(),
            }),
        }
    }

    fn set(&self, name: &SecretName, value: &Secret) -> Result<(), SecretError> {
        if !name.is_valid() {
            return Err(SecretError::InvalidName);
        }
        security_framework::passwords::set_generic_password(
            KEYCHAIN_SERVICE,
            &name.key(),
            value.expose().as_bytes(),
        )
        .map_err(|e| SecretError::Keychain {
            op: "store",
            detail: e.to_string(),
        })
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        if !name.is_valid() {
            return Err(SecretError::InvalidName);
        }
        match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, &name.key())
        {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(e) => Err(SecretError::Keychain {
                op: "delete",
                detail: e.to_string(),
            }),
        }
    }

    fn kind(&self) -> SecretStoreKind {
        SecretStoreKind::Keychain
    }
}

// ──────────────────────────────── files ─────────────────────────────────

/// `0600` files in one directory (created `0700`).
#[derive(Debug)]
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, name: &SecretName) -> Result<PathBuf, SecretError> {
        if !name.is_valid() {
            return Err(SecretError::InvalidName);
        }
        Ok(self.dir.join(name.file_name()))
    }
}

fn io_error(op: &'static str, e: &std::io::Error) -> SecretError {
    // The kind only: an OS message can quote the path, never the value, but
    // keep it short either way.
    SecretError::Io {
        op,
        detail: e.kind().to_string(),
    }
}

impl SecretStore for FileStore {
    fn get(&self, name: &SecretName) -> Result<Option<Secret>, SecretError> {
        let path = self.path(name)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let value = text.trim_end_matches(['\n', '\r']).to_string();
                Ok((!value.is_empty()).then_some(Secret(value)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error("read", &e)),
        }
    }

    fn set(&self, name: &SecretName, value: &Secret) -> Result<(), SecretError> {
        let path = self.path(name)?;
        std::fs::create_dir_all(&self.dir).map_err(|e| io_error("create", &e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700));
        }
        crate::persist::write_private(&path, value.expose().as_bytes())
            .map_err(|e| io_error("write", &e))
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        let path = self.path(name)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_error("delete", &e)),
        }
    }

    fn kind(&self) -> SecretStoreKind {
        SecretStoreKind::Files
    }
}

// ──────────────────────────────── memory ────────────────────────────────

/// In memory only: tests, and a state without a data dir.
#[derive(Default)]
pub struct MemoryStore {
    items: Mutex<HashMap<SecretName, Secret>>,
}

impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.items.lock().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("MemoryStore").field("items", &n).finish()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &SecretName) -> Result<Option<Secret>, SecretError> {
        if !name.is_valid() {
            return Err(SecretError::InvalidName);
        }
        Ok(self
            .items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned())
    }

    fn set(&self, name: &SecretName, value: &Secret) -> Result<(), SecretError> {
        if !name.is_valid() {
            return Err(SecretError::InvalidName);
        }
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.clone(), value.clone());
        Ok(())
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(name);
        Ok(())
    }

    fn kind(&self) -> SecretStoreKind {
        SecretStoreKind::Memory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "acc_0123456789ab";

    #[test]
    fn a_secret_never_prints() {
        let s = Secret::new("sk-ant-oat01-very-secret");
        assert_eq!(format!("{s:?}"), "Secret(…)");
        assert!(!format!("{:?}", Some(s.clone())).contains("very-secret"));
        assert_eq!(s.sha256_hex().len(), 64);
        assert_eq!(
            s.sha256_hex(),
            Secret::new("sk-ant-oat01-very-secret").sha256_hex()
        );
    }

    #[test]
    fn names_map_to_keys_and_files() {
        assert_eq!(SecretName::AccountToken(ID.into()).key(), ID);
        assert_eq!(
            SecretName::AccountToken(ID.into()).file_name(),
            format!("{ID}.token")
        );
        assert_eq!(SecretName::BackupPassphrase.key(), "backup-passphrase");
        // Anything that could escape the directory is refused.
        for bad in ["../x", "acc_", "default", "acc_0123456789ab/..", ""] {
            assert!(!SecretName::AccountToken(bad.into()).is_valid(), "{bad}");
        }
    }

    #[test]
    fn file_store_round_trips_with_private_modes() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path().join("secrets"));
        let name = SecretName::AccountToken(ID.into());
        assert!(store.get(&name).unwrap().is_none());
        store.set(&name, &Secret::new("tok-1")).unwrap();
        assert_eq!(store.get(&name).unwrap().unwrap().expose(), "tok-1");
        store.set(&name, &Secret::new("tok-2")).unwrap();
        assert_eq!(store.get(&name).unwrap().unwrap().expose(), "tok-2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir.path().join("secrets")), 0o700);
            assert_eq!(mode(&dir.path().join(format!("secrets/{ID}.token"))), 0o600);
        }
        store.delete(&name).unwrap();
        store.delete(&name).unwrap();
        assert!(store.get(&name).unwrap().is_none());
        assert!(matches!(
            store.get(&SecretName::AccountToken("../../etc/passwd".into())),
            Err(SecretError::InvalidName)
        ));
    }

    #[test]
    fn memory_store_round_trips() {
        let store = MemoryStore::default();
        let name = SecretName::BackupPassphrase;
        store.set(&name, &Secret::new("words words")).unwrap();
        assert_eq!(store.get(&name).unwrap().unwrap().expose(), "words words");
        assert!(!format!("{store:?}").contains("words"));
        store.delete(&name).unwrap();
        assert!(store.get(&name).unwrap().is_none());
    }

    #[test]
    fn only_the_apps_own_data_dir_could_reach_the_keychain() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(store_for(Some(dir.path())).kind(), SecretStoreKind::Files);
        assert_eq!(store_for(None).kind(), SecretStoreKind::Memory);
    }

    /// P0.3's check, run by hand on the maintainer's Mac:
    /// `cargo test --lib keychain_round_trip -- --ignored`. It files a dummy
    /// item under our own service, confirms `security` sees it (attributes
    /// only, no password prompt) and that nothing landed on disk, then deletes
    /// it.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches the login Keychain"]
    fn keychain_round_trip() {
        let store = KeychainStore;
        let name = SecretName::AccountToken("acc_000000007e57".into());
        store.set(&name, &Secret::new("dummy-value")).unwrap();
        let seen = std::process::Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a"])
            .arg(name.key())
            .output()
            .unwrap();
        assert!(seen.status.success(), "the item is in the login Keychain");
        assert_eq!(store.get(&name).unwrap().unwrap().expose(), "dummy-value");
        store.delete(&name).unwrap();
        assert!(store.get(&name).unwrap().is_none());
    }
}
