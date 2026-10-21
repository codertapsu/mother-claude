//! Storages (docs/ACCOUNTS.md §13.1, §13.3 rule 8): folders that hold
//! encrypted sets. The backup folder is the first (id `backup`); the user may
//! add more (`stor_` + 12 hex): another cloud's folder, a NAS share, an
//! external disk. Every set in every storage opens with the one backup
//! passphrase, and a passphrase change re-keys them all.
//!
//! A set named after an account is that account's backup only in the backup
//! folder: the schedule, "Back up now" and pull on launch use the backup
//! folder alone. Every other set — in the backup folder or anywhere else — is
//! read and written by Move data only. Removing a storage forgets it and never
//! deletes its files.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{BackupError, Set};
use crate::claude::accounts::{self, AccountError, StorageEntry, BACKUP_STORAGE_ID};
use crate::claude::home::user_home_dir;
use crate::state::AppState;

/// The backup folder's label as a storage.
pub const PRIMARY_LABEL: &str = "Backup folder";
/// Longest label, in characters.
pub const LABEL_MAX: usize = 60;

/// What kind of folder a storage is (the UI's icon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageKind {
    Icloud,
    Gdrive,
    Onedrive,
    Dropbox,
    Folder,
}

/// What kind of folder `path` is, for a user whose home is `home` (the
/// places the folder suggestions look at, plus the classic `~/Dropbox`,
/// `~/OneDrive*` and `~/Google Drive`).
pub fn kind_in(home: &Path, path: &Path) -> StorageKind {
    if path.starts_with(home.join("Library/Mobile Documents/com~apple~CloudDocs"))
        || path.starts_with(home.join("iCloudDrive"))
    {
        return StorageKind::Icloud;
    }
    let first = |base: &Path| -> Option<String> {
        path.strip_prefix(base)
            .ok()?
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
    };
    for name in [first(&home.join("Library/CloudStorage")), first(home)]
        .into_iter()
        .flatten()
    {
        if name.starts_with("GoogleDrive") || name == "Google Drive" || name == "My Drive" {
            return StorageKind::Gdrive;
        }
        if name.starts_with("OneDrive") {
            return StorageKind::Onedrive;
        }
        if name.starts_with("Dropbox") {
            return StorageKind::Dropbox;
        }
    }
    StorageKind::Folder
}

/// [`kind_in`] for this user.
pub fn kind_of(path: &Path) -> StorageKind {
    user_home_dir().map_or(StorageKind::Folder, |home| kind_in(&home, path))
}

/// A storage, resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Storage {
    pub id: String,
    pub label: String,
    pub path: PathBuf,
    /// The backup folder.
    pub primary: bool,
}

impl Storage {
    /// Its folder is there now (an external disk may be unplugged). The
    /// backup folder always counts: its first backup makes it, as before
    /// storages.
    pub fn available(&self) -> bool {
        self.primary || self.path.is_dir()
    }

    /// `Err(storage_unavailable)` unless [`Self::available`].
    pub fn require_available(&self) -> Result<(), BackupError> {
        if self.available() {
            Ok(())
        } else {
            Err(BackupError::StorageUnavailable(self.label.clone()))
        }
    }

    /// The set `name` in it (the name checked by [`super::clean_set_name`]).
    pub fn set(&self, name: &str) -> Set {
        Set::new(&self.path, name)
    }
}

/// Every storage in `settings`: the backup folder first (when one is set),
/// then the ones the user added, in order.
pub fn list_in(settings: &accounts::BackupSettings) -> Vec<Storage> {
    settings
        .folder
        .iter()
        .map(|f| Storage {
            id: BACKUP_STORAGE_ID.to_string(),
            label: PRIMARY_LABEL.to_string(),
            path: f.clone(),
            primary: true,
        })
        .chain(settings.storages.iter().map(|s| Storage {
            id: s.id.clone(),
            label: s.label.clone(),
            path: s.path.clone(),
            primary: false,
        }))
        .collect()
}

/// Every storage.
pub fn list(state: &AppState) -> Vec<Storage> {
    list_in(&state.accounts.backup_settings())
}

/// One storage by id (`backup`: the backup folder, `no_backup_folder` while
/// none is set).
pub fn get(state: &AppState, id: &str) -> Result<Storage, BackupError> {
    if id == BACKUP_STORAGE_ID {
        return list(state)
            .into_iter()
            .find(|s| s.primary)
            .ok_or(BackupError::NoFolder);
    }
    list(state)
        .into_iter()
        .find(|s| !s.primary && s.id == id)
        .ok_or_else(|| BackupError::StorageNotFound(id.to_string()))
}

/// One set as a storage lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSummary {
    pub name: String,
    /// The account (one with a set of its own) the set is named after.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub exists: bool,
    /// Something of a set is there but not its key (not downloaded yet).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub key_missing: bool,
    /// Sessions in the set, as this app last read or wrote its index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<usize>,
    /// Of those, the ones in storage only (§13.3 rule 4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<usize>,
}

/// A storage as `GET /api/backup` lists it (`storage`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageView {
    pub id: String,
    pub label: String,
    pub path: String,
    pub kind: StorageKind,
    pub primary: bool,
    pub available: bool,
    pub sets: Vec<SetSummary>,
}

fn summary(set: &Set, name: &str, account: Option<String>) -> SetSummary {
    let counts = super::cached_counts(set);
    SetSummary {
        name: name.to_string(),
        account,
        exists: set.exists(),
        key_missing: set.key_missing(),
        sessions: counts.map(|c| c.0),
        archived: counts.map(|c| c.1).filter(|n| *n > 0),
    }
}

/// The sets of a storage: in the backup folder every account's own set
/// (made or not yet), then whatever other set is there; elsewhere the sets
/// that are there. `owners`: `(id, name)` of the accounts with a set of their
/// own.
pub fn sets_of(storage: &Storage, owners: &[(String, String)]) -> Vec<SetSummary> {
    let named = |name: &str| {
        owners
            .iter()
            .find(|(_, n)| n.to_lowercase() == name.to_lowercase())
            .map(|(id, _)| id.clone())
    };
    let mut out: Vec<SetSummary> = Vec::new();
    if storage.primary {
        for (id, name) in owners {
            out.push(summary(&storage.set(name), name, Some(id.clone())));
        }
    }
    for (name, set) in super::ops::folder_sets(&storage.path) {
        if out
            .iter()
            .any(|s| s.name.to_lowercase() == name.to_lowercase())
        {
            continue;
        }
        let account = named(&name);
        out.push(summary(&set, &name, account));
    }
    out
}

/// A storage's view.
pub fn view_of(storage: &Storage, owners: &[(String, String)]) -> StorageView {
    StorageView {
        id: storage.id.clone(),
        label: storage.label.clone(),
        path: storage.path.to_string_lossy().into_owned(),
        kind: kind_of(&storage.path),
        primary: storage.primary,
        available: storage.available(),
        sets: sets_of(storage, owners),
    }
}

/// Every storage's view, the backup folder first.
pub fn views(state: &AppState) -> Vec<StorageView> {
    let owners = super::ops::set_owner_names(state);
    list(state).iter().map(|s| view_of(s, &owners)).collect()
}

/// Whether `GET /api/backup` lists the storages: not while only the Default
/// is registered, no storage besides the backup folder exists and that folder
/// holds no set but the Default's (goal 7: every response as before §13).
pub fn shown(state: &AppState, views: &[StorageView]) -> bool {
    !state.accounts.is_default_only()
        || views
            .iter()
            .any(|v| !v.primary || v.sets.iter().any(|s| s.account.is_none()))
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

fn normalized(p: &Path) -> PathBuf {
    accounts::normalize_dir(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Every account directory, `(account name, directory)`: what a storage may
/// be neither inside nor around.
fn account_dirs(state: &AppState) -> Vec<(String, PathBuf)> {
    state
        .accounts
        .homes()
        .into_iter()
        .map(|(id, home)| {
            let name = state.accounts.get(&id).map(|a| a.name).unwrap_or(id);
            (name, normalized(home.base()))
        })
        .collect()
}

/// Whether `path` may be a storage (§13.3 rule 8): an absolute folder, not
/// the home folder or the root, not inside or around an account's directory
/// (`dirs`), the backup folder (`folder`) or another storage (`except`: the
/// one being changed).
pub fn check_free(
    dirs: &[(String, PathBuf)],
    path: &Path,
    folder: Option<&Path>,
    storages: &[StorageEntry],
    except: Option<&str>,
) -> Result<(), BackupError> {
    let invalid = |why: String| Err(BackupError::InvalidStorage(why));
    if path.parent().is_none()
        || user_home_dir()
            .map(|h| normalized(&h))
            .is_some_and(|h| h == path)
    {
        return invalid("the home folder or the root cannot be a storage".into());
    }
    if let Some((name, _)) = dirs.iter().find(|(_, dir)| overlaps(path, dir)) {
        return invalid(format!(
            "{} is inside (or around) the directory of account {name:?}",
            path.display()
        ));
    }
    if folder.is_some_and(|f| overlaps(path, &normalized(f))) {
        return invalid(format!(
            "{} is inside (or around) the backup folder",
            path.display()
        ));
    }
    if let Some(other) = storages
        .iter()
        .filter(|s| Some(s.id.as_str()) != except)
        .find(|s| overlaps(path, &normalized(&s.path)))
    {
        return invalid(format!(
            "{} is inside (or around) the storage {:?}",
            path.display(),
            other.label
        ));
    }
    if path.exists() && !path.is_dir() {
        return invalid(format!("{} is not a folder", path.display()));
    }
    Ok(())
}

/// A storage's label: trimmed, 1–60 characters, no control characters.
pub fn clean_label(label: &str) -> Result<String, BackupError> {
    let label = label.trim();
    let n = label.chars().count();
    if n == 0 || n > LABEL_MAX || label.chars().any(char::is_control) {
        return Err(BackupError::InvalidStorage(format!(
            "a storage's label has 1 to {LABEL_MAX} characters"
        )));
    }
    Ok(label.to_string())
}

fn saved(e: AccountError) -> BackupError {
    match e {
        AccountError::Invalid(why) => BackupError::InvalidStorage(why),
        other => BackupError::Io(other.to_string()),
    }
}

/// Add a storage. Its folder is made when only it is missing (its parent is
/// there); a folder whose parent is missing (an unplugged disk) is refused.
pub fn add(state: &AppState, path: &Path, label: Option<&str>) -> Result<Storage, BackupError> {
    let path =
        accounts::normalize_dir(path).map_err(|e| BackupError::InvalidStorage(e.to_string()))?;
    let label = match label.filter(|l| !l.trim().is_empty()) {
        Some(l) => clean_label(l)?,
        None => clean_label(
            &path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        )
        .unwrap_or_else(|_| "Storage".to_string()),
    };
    let settings = state.accounts.backup_settings();
    // Read before the registry's lock is taken below (never inside it).
    let dirs = account_dirs(state);
    check_free(
        &dirs,
        &path,
        settings.folder.as_deref(),
        &settings.storages,
        None,
    )?;
    if !path.is_dir() {
        if !path.parent().is_some_and(Path::is_dir) {
            return Err(BackupError::InvalidStorage(format!(
                "the folder that would hold {} is not there (an unplugged disk?)",
                path.display()
            )));
        }
        std::fs::create_dir(&path)
            .map_err(|e| BackupError::Io(format!("creating {}: {e}", path.display())))?;
    }
    let entry = StorageEntry {
        id: accounts::new_storage_id(),
        label,
        path,
        created_at: chrono::Utc::now().timestamp_millis(),
        extra: Default::default(),
    };
    state
        .accounts
        .update_backup(|s| {
            // Once more under the registry's lock: another one may have been
            // added meanwhile.
            check_free(&dirs, &entry.path, s.folder.as_deref(), &s.storages, None)
                .map_err(|e| AccountError::Invalid(e.to_string()))?;
            s.storages.push(entry.clone());
            Ok(())
        })
        .map_err(saved)?;
    Ok(Storage {
        id: entry.id,
        label: entry.label,
        path: entry.path,
        primary: false,
    })
}

/// Give a storage another label (the backup folder's is fixed).
pub fn rename(state: &AppState, id: &str, label: &str) -> Result<Storage, BackupError> {
    if id == BACKUP_STORAGE_ID {
        return Err(BackupError::InvalidStorage(
            "the backup folder's label is fixed".into(),
        ));
    }
    let label = clean_label(label)?;
    state
        .accounts
        .update_backup(|s| {
            let entry = s
                .storages
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
            entry.label = label.clone();
            Ok(())
        })
        .map_err(|e| match e {
            AccountError::NotFound(id) => BackupError::StorageNotFound(id),
            other => saved(other),
        })?;
    get(state, id)
}

/// Forget a storage; its files stay where they are. The backup folder is
/// changed in the backup settings, not here.
pub fn remove(state: &AppState, id: &str) -> Result<StorageEntry, BackupError> {
    if id == BACKUP_STORAGE_ID {
        return Err(BackupError::InvalidStorage(
            "the backup folder is changed in the backup settings, not removed here".into(),
        ));
    }
    state
        .accounts
        .update_backup(|s| {
            let at = s
                .storages
                .iter()
                .position(|e| e.id == id)
                .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
            Ok(s.storages.remove(at))
        })
        .map_err(|e| match e {
            AccountError::NotFound(id) => BackupError::StorageNotFound(id),
            other => saved(other),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_the_cloud_folders() {
        let home = Path::new("/Users/me");
        for (path, kind) in [
            (
                "/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Mother Claude",
                StorageKind::Icloud,
            ),
            (
                "/Users/me/Library/CloudStorage/GoogleDrive-me@example.com/My Drive/Archive",
                StorageKind::Gdrive,
            ),
            (
                "/Users/me/Library/CloudStorage/OneDrive-Personal/Claude",
                StorageKind::Onedrive,
            ),
            (
                "/Users/me/Library/CloudStorage/Dropbox/x",
                StorageKind::Dropbox,
            ),
            ("/Users/me/Dropbox (Work)/x", StorageKind::Dropbox),
            ("/Users/me/Google Drive/x", StorageKind::Gdrive),
            ("/Volumes/Backup/Claude", StorageKind::Folder),
            ("/Users/me/Documents/Claude", StorageKind::Folder),
        ] {
            assert_eq!(kind_in(home, Path::new(path)), kind, "{path}");
        }
    }

    #[test]
    fn labels_are_short_lines() {
        assert_eq!(clean_label("  Archive disk ").unwrap(), "Archive disk");
        assert!(clean_label("").is_err());
        assert!(clean_label("a\nb").is_err());
        assert!(clean_label(&"x".repeat(LABEL_MAX + 1)).is_err());
    }
}
