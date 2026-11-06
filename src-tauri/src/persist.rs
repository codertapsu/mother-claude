//! Small helpers for Mother Claude's own persisted state in the app data dir
//! (`~/Library/Application Support/dev.motherclaude.Mother-Claude` or the
//! platform equivalent). **Never** used for anything under `~/.claude`.
//!
//! Every subsystem that keeps a file (`devices.json`, `remote.json`,
//! `push.json`, `index-cache.json`, the VAPID key, `hook.curlrc`, tokens) goes
//! through these so the permissions (0600) and the atomic write (tmp + rename)
//! are handled once.
//!
//! [`sync_file`] is the flush of every atomic write in the crate — these, the
//! backup's and the copies between accounts and storages alike. It only
//! flushes a file its caller opened, so it never picks a path of its own.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// The app data dir, or `None` when the platform has no home directory.
pub fn app_data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "motherclaude", "Mother Claude")
        .map(|d| d.data_dir().to_path_buf())
}

/// chmod 0600 (no-op off unix).
#[cfg(unix)]
pub fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::debug!(path = %path.display(), error = %e, "could not chmod 0600");
    }
}

#[cfg(not(unix))]
pub fn restrict(_path: &Path) {}

/// `File::sync_all`, where the file system can do it. On macOS that asks for
/// `F_FULLFSYNC`, which flushes the drive's own cache too; an SMB share (a
/// NAS) answers ENOTSUP, "os error 45", and gets the plain `fsync` it can do
/// instead, as SQLite does. A real failure stays one.
pub fn sync_file(f: &std::fs::File) -> std::io::Result<()> {
    #[cfg(target_vendor = "apple")]
    {
        use std::os::fd::AsRawFd;
        sync_or_fall_back(f.sync_all(), || loop {
            // SAFETY: `f` keeps the descriptor open for the whole call.
            if unsafe { libc::fsync(f.as_raw_fd()) } == 0 {
                return Ok(());
            }
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::Interrupted {
                return Err(e);
            }
        })
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        f.sync_all()
    }
}

/// `full`'s outcome, or `plain`'s where the file system cannot do a full
/// sync. Never after an I/O error: a second flush could then report success
/// for data that is gone.
#[cfg(target_vendor = "apple")]
fn sync_or_fall_back(
    full: std::io::Result<()>,
    plain: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    match full {
        Err(e)
            if matches!(
                e.raw_os_error(),
                Some(libc::ENOTSUP | libc::EOPNOTSUPP | libc::EINVAL | libc::ENOTTY)
            ) =>
        {
            tracing::debug!(error = %e, "no full sync on this file system; a plain fsync");
            plain()
        }
        other => other,
    }
}

/// Write `bytes` to `path` atomically with mode 0600: write a sibling temp file,
/// fsync, chmod, rename over the target. Creates the parent directory.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    std::fs::create_dir_all(dir)?;
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("state");
    let tmp = dir.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        sync_file(&f)?;
    }
    restrict(&tmp);
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Serialize `value` as pretty JSON and [`write_private`] it.
pub fn write_json_private<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_private(path, &bytes)
}

/// Read and parse a JSON file. Missing → `None`; unparseable → `None` with a
/// warning (tolerant: a corrupt state file must never crash the app).
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "could not read state file");
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "ignoring unparseable state file");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_private_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/state.json");
        write_json_private(&path, &serde_json::json!({ "a": 1 })).unwrap();
        let v: serde_json::Value = read_json(&path).unwrap();
        assert_eq!(v["a"], 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // Overwrite in place; no temp files left behind.
        write_json_private(&path, &serde_json::json!({ "a": 2 })).unwrap();
        let entries: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);
        assert!(read_json::<serde_json::Value>(&dir.path().join("missing.json")).is_none());
        std::fs::write(dir.path().join("bad.json"), "{nope").unwrap();
        assert!(read_json::<serde_json::Value>(&dir.path().join("bad.json")).is_none());
    }

    #[test]
    fn a_file_is_synced_where_it_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let f = std::fs::File::create(dir.path().join("f")).unwrap();
        sync_file(&f).unwrap();
    }

    /// An SMB share answers ENOTSUP to `F_FULLFSYNC` (what `sync_all` asks
    /// for on macOS) and takes a plain `fsync`; an I/O error is never
    /// flushed over.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn a_file_system_without_a_full_sync_gets_a_plain_one() {
        use std::io::Error;
        let plain_ran = std::cell::Cell::new(false);
        let plain = || {
            plain_ran.set(true);
            Ok(())
        };
        for unsupported in [libc::ENOTSUP, libc::EOPNOTSUPP, libc::EINVAL, libc::ENOTTY] {
            plain_ran.set(false);
            sync_or_fall_back(Err(Error::from_raw_os_error(unsupported)), plain).unwrap();
            assert!(plain_ran.get(), "errno {unsupported}");
        }
        // The plain one's own failure is the outcome.
        let e = sync_or_fall_back(Err(Error::from_raw_os_error(libc::ENOTSUP)), || {
            Err(Error::from_raw_os_error(libc::EIO))
        })
        .unwrap_err();
        assert_eq!(e.raw_os_error(), Some(libc::EIO));
        // A full sync that worked, or a real failure: nothing more.
        plain_ran.set(false);
        sync_or_fall_back(Ok(()), plain).unwrap();
        let e = sync_or_fall_back(Err(Error::from_raw_os_error(libc::EIO)), plain).unwrap_err();
        assert_eq!(e.raw_os_error(), Some(libc::EIO));
        assert!(!plain_ran.get());
    }
}
