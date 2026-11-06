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
    /// A new backup passphrase while the sets are re-keyed with it: every set
    /// opens with one of the two until the change is through.
    BackupPassphrasePending,
}

impl SecretName {
    /// The Keychain item's account attribute.
    pub fn key(&self) -> String {
        match self {
            Self::AccountToken(id) => id.clone(),
            Self::BackupPassphrase => "backup-passphrase".to_string(),
            Self::BackupPassphrasePending => "backup-passphrase-pending".to_string(),
        }
    }

    /// The file name under `secrets/`.
    fn file_name(&self) -> String {
        match self {
            Self::AccountToken(id) => format!("{id}.token"),
            Self::BackupPassphrase => "backup-passphrase".to_string(),
            Self::BackupPassphrasePending => "backup-passphrase-pending".to_string(),
        }
    }

    /// Only names that are safe as one path component and Keychain account.
    fn is_valid(&self) -> bool {
        match self {
            Self::AccountToken(id) => super::accounts::is_account_id(id),
            Self::BackupPassphrase | Self::BackupPassphrasePending => true,
        }
    }
}

/// Where secrets are kept, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretStoreKind {
    Keychain,
    /// Windows Credential Manager.
    CredentialManager,
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
    #[error("Windows Credential Manager refused to {op} the secret: {detail}")]
    CredentialManager { op: &'static str, detail: String },
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

/// The store for a data dir: for the app's own data dir the Keychain on macOS
/// and Windows Credential Manager on Windows (which takes over the `0600` files
/// an older build kept there, one secret at a time as each is read), unless
/// `MOTHER_CLAUDE_SECRET_STORE=file`; `0600` files in `<data dir>/secrets/` for
/// any other data dir and elsewhere; memory without one.
pub fn store_for(data_dir: Option<&Path>) -> Arc<dyn SecretStore> {
    let Some(dir) = data_dir else {
        return Arc::new(MemoryStore::default());
    };
    #[cfg(any(target_os = "macos", windows))]
    {
        let files_requested = std::env::var(STORE_ENV)
            .map(|v| v.trim().eq_ignore_ascii_case("file"))
            .unwrap_or(false);
        let app_dir = crate::persist::app_data_dir();
        if !files_requested && app_dir.as_deref() == Some(dir) {
            #[cfg(target_os = "macos")]
            return Arc::new(KeychainStore);
            #[cfg(windows)]
            return Arc::new(MigratingStore::new(
                CredentialStore,
                FileStore::new(dir.join("secrets")),
            ));
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = STORE_ENV;
    Arc::new(FileStore::new(dir.join("secrets")))
}

// ─────────────────────────── Credential Manager ───────────────────────────

/// Generic credentials in Windows Credential Manager, named
/// `dev.motherclaude.desktop/<key>`: this user's, on this machine only (not
/// roaming), protected by Windows for the user as the Keychain is on a Mac.
#[cfg(windows)]
#[derive(Debug)]
pub struct CredentialStore;

/// The credential's target name.
#[cfg(windows)]
fn credential_target(name: &SecretName) -> Vec<u16> {
    format!("{KEYCHAIN_SERVICE}/{}", name.key())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn credential_error(op: &'static str) -> SecretError {
    // SAFETY: reads this thread's last error, set by the call that failed.
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    SecretError::CredentialManager {
        op,
        detail: format!("error {code}"),
    }
}

#[cfg(windows)]
impl SecretStore for CredentialStore {
    fn get(&self, name: &SecretName) -> Result<Option<Secret>, SecretError> {
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
        use windows_sys::Win32::Security::Credentials::{
            CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
        };
        if !name.is_valid() {
            return Err(SecretError::InvalidName);
        }
        let target = credential_target(name);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated and outlives the call; on success
        // `cred` points to a buffer the system allocated, freed below.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) } == 0 {
            // SAFETY: reads this thread's last error, set by CredReadW.
            if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                return Ok(None);
            }
            return Err(credential_error("read"));
        }
        // SAFETY: `cred` is the valid credential CredReadW returned; its blob
        // is `CredentialBlobSize` bytes long (or null when empty). Copied out
        // before the one CredFree.
        let bytes = unsafe {
            let c = &*cred;
            let bytes = if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
            };
            CredFree(cred as *const core::ffi::c_void);
            bytes
        };
        match String::from_utf8(bytes) {
            Ok(s) if s.is_empty() => Ok(None),
            Ok(s) => Ok(Some(Secret(s))),
            Err(_) => Err(SecretError::CredentialManager {
                op: "read",
                detail: "the stored value is not text".into(),
            }),
        }
    }

    fn set(&self, name: &SecretName, value: &Secret) -> Result<(), SecretError> {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE,
            CRED_TYPE_GENERIC,
        };
        if !name.is_valid() {
            return Err(SecretError::InvalidName);
        }
        let mut blob = value.expose().as_bytes().to_vec();
        if blob.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
            return Err(SecretError::CredentialManager {
                op: "store",
                detail: "the value is longer than Credential Manager keeps".into(),
            });
        }
        let mut target = credential_target(name);
        let mut user: Vec<u16> = "Mother Claude"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            Comment: std::ptr::null_mut(),
            LastWritten: FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            },
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: std::ptr::null_mut(),
            UserName: user.as_mut_ptr(),
        };
        // SAFETY: every pointer in `cred` points into a buffer that lives until
        // the call returns; CredWriteW copies what it keeps.
        let ok = unsafe { CredWriteW(&cred, 0) } != 0;
        let failure = (!ok).then(|| credential_error("store"));
        // Our copy of the value, cleared before it is freed.
        blob.iter_mut().for_each(|b| *b = 0);
        failure.map_or(Ok(()), Err)
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        if !name.is_valid() {
            return Err(SecretError::InvalidName);
        }
        let target = credential_target(name);
        // SAFETY: `target` is NUL-terminated and outlives the call.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
            // SAFETY: reads this thread's last error, set by CredDeleteW.
            if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                return Ok(());
            }
            return Err(credential_error("delete"));
        }
        Ok(())
    }

    fn kind(&self) -> SecretStoreKind {
        SecretStoreKind::CredentialManager
    }
}

// ───────────────────────────── moving stores ─────────────────────────────

/// A store that takes over the `0600` files an older build kept: `primary`
/// first; a secret found only in its file is written to `primary` when it is
/// read and its file removed once `primary` has it. Writing and deleting go
/// to `primary`, and remove the file too. If `primary` cannot be read at all,
/// the file still answers, so a broken store never loses a secret.
#[derive(Debug)]
pub struct MigratingStore<P> {
    primary: P,
    legacy: FileStore,
}

impl<P: SecretStore> MigratingStore<P> {
    pub fn new(primary: P, legacy: FileStore) -> Self {
        Self { primary, legacy }
    }
}

impl<P: SecretStore> SecretStore for MigratingStore<P> {
    fn get(&self, name: &SecretName) -> Result<Option<Secret>, SecretError> {
        match self.primary.get(name) {
            Ok(Some(secret)) => return Ok(Some(secret)),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "the secret store could not be read; trying the file");
                return self.legacy.get(name);
            }
        }
        let Some(secret) = self.legacy.get(name)? else {
            return Ok(None);
        };
        match self.primary.set(name, &secret) {
            Ok(()) => {
                if let Err(e) = self.legacy.delete(name) {
                    tracing::warn!(error = %e, "a moved secret's old file could not be removed");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "a secret stays in its file: the store refused it")
            }
        }
        Ok(Some(secret))
    }

    fn set(&self, name: &SecretName, value: &Secret) -> Result<(), SecretError> {
        self.primary.set(name, value)?;
        if let Err(e) = self.legacy.delete(name) {
            tracing::warn!(error = %e, "an old secret file could not be removed");
        }
        Ok(())
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        self.primary.delete(name)?;
        self.legacy.delete(name)
    }

    fn kind(&self) -> SecretStoreKind {
        self.primary.kind()
    }
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

    /// What a Windows build does with the files an older one kept: each moves
    /// into Credential Manager (here a memory store) on its first read, and
    /// its file goes once the store has it.
    #[test]
    fn a_secret_moves_out_of_its_old_file_when_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let files = FileStore::new(dir.path().join("secrets"));
        let token = SecretName::AccountToken(ID.into());
        files.set(&token, &Secret::new("tok-old")).unwrap();
        files
            .set(&SecretName::BackupPassphrase, &Secret::new("words"))
            .unwrap();
        let store = MigratingStore::new(
            MemoryStore::default(),
            FileStore::new(dir.path().join("secrets")),
        );
        assert_eq!(store.kind(), SecretStoreKind::Memory);
        assert_eq!(store.get(&token).unwrap().unwrap().expose(), "tok-old");
        let file = dir.path().join(format!("secrets/{ID}.token"));
        assert!(!file.exists(), "moved: its file is gone");
        assert_eq!(
            store.primary.get(&token).unwrap().unwrap().expose(),
            "tok-old"
        );
        // Read again: from the store, the same value.
        assert_eq!(store.get(&token).unwrap().unwrap().expose(), "tok-old");
        // A secret nobody read yet stays in its file until then.
        assert!(dir.path().join("secrets/backup-passphrase").exists());
        // Writing replaces both; deleting removes both.
        store
            .set(&SecretName::BackupPassphrase, &Secret::new("new words"))
            .unwrap();
        assert!(!dir.path().join("secrets/backup-passphrase").exists());
        assert_eq!(
            store
                .get(&SecretName::BackupPassphrase)
                .unwrap()
                .unwrap()
                .expose(),
            "new words"
        );
        store.delete(&token).unwrap();
        assert!(store.get(&token).unwrap().is_none());
        assert!(store
            .get(&SecretName::BackupPassphrasePending)
            .unwrap()
            .is_none());
    }

    /// A store that cannot be read: the file still answers.
    #[test]
    fn a_broken_store_never_hides_a_secret_its_file_has() {
        #[derive(Debug)]
        struct Broken;
        impl SecretStore for Broken {
            fn get(&self, _: &SecretName) -> Result<Option<Secret>, SecretError> {
                Err(SecretError::CredentialManager {
                    op: "read",
                    detail: "error 1312".into(),
                })
            }
            fn set(&self, _: &SecretName, _: &Secret) -> Result<(), SecretError> {
                Err(SecretError::CredentialManager {
                    op: "store",
                    detail: "error 1312".into(),
                })
            }
            fn delete(&self, _: &SecretName) -> Result<(), SecretError> {
                Ok(())
            }
            fn kind(&self) -> SecretStoreKind {
                SecretStoreKind::CredentialManager
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let files = FileStore::new(dir.path().join("secrets"));
        files
            .set(&SecretName::BackupPassphrase, &Secret::new("words"))
            .unwrap();
        let store = MigratingStore::new(Broken, FileStore::new(dir.path().join("secrets")));
        assert_eq!(
            store
                .get(&SecretName::BackupPassphrase)
                .unwrap()
                .unwrap()
                .expose(),
            "words"
        );
        assert!(dir.path().join("secrets/backup-passphrase").exists());
        // A write the store refuses is an error, and the old file stays.
        assert!(store
            .set(&SecretName::BackupPassphrase, &Secret::new("x"))
            .is_err());
        assert_eq!(
            files
                .get(&SecretName::BackupPassphrase)
                .unwrap()
                .unwrap()
                .expose(),
            "words"
        );
    }

    /// Run by hand on the Windows desktop:
    /// `cargo test --lib credential_manager_round_trip -- --ignored`. Files a
    /// dummy credential under our own name, reads it back, deletes it.
    #[cfg(windows)]
    #[test]
    #[ignore = "touches Windows Credential Manager"]
    fn credential_manager_round_trip() {
        let store = CredentialStore;
        let name = SecretName::AccountToken("acc_000000007e57".into());
        store.set(&name, &Secret::new("dummy-value")).unwrap();
        assert_eq!(store.get(&name).unwrap().unwrap().expose(), "dummy-value");
        store.set(&name, &Secret::new("dummy-value-2")).unwrap();
        assert_eq!(store.get(&name).unwrap().unwrap().expose(), "dummy-value-2");
        store.delete(&name).unwrap();
        store.delete(&name).unwrap();
        assert!(store.get(&name).unwrap().is_none());
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
