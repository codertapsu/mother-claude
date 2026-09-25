//! Small helpers for Mother Claude's own persisted state in the app data dir
//! (`~/Library/Application Support/dev.motherclaude.Mother-Claude` or the
//! platform equivalent). **Never** used for anything under `~/.claude`.
//!
//! Every subsystem that keeps a file (`devices.json`, `remote.json`,
//! `push.json`, `index-cache.json`, the VAPID key, `hook.curlrc`, tokens) goes
//! through these so the permissions (0600) and the atomic write (tmp + rename)
//! are handled once.

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
        f.sync_all()?;
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
}
