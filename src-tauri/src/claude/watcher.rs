//! Debounced recursive filesystem watching over the `~/.claude` subtrees we care
//! about (`projects/`, `jobs/`, `daemon/`).
//!
//! Changed paths are coalesced by [`notify_debouncer_full`] and forwarded on a
//! tokio channel. The registry/tailer layer reacts to these to re-read state and
//! pull new transcript lines, instead of polling on a tight loop.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use tokio::sync::mpsc::UnboundedSender;

/// Channel of debounced batches of changed paths.
pub type WatchSender = UnboundedSender<Vec<PathBuf>>;

/// Owns the live debouncer; dropping it stops watching.
pub struct FsWatcher {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

impl FsWatcher {
    /// Watch each existing path recursively, forwarding debounced batches of
    /// changed paths on `tx`. Missing paths are skipped (logged), not fatal.
    pub fn spawn(paths: &[PathBuf], tx: WatchSender) -> Result<Self> {
        let mut debouncer = new_debouncer(
            Duration::from_millis(250),
            None,
            move |res: DebounceEventResult| match res {
                Ok(events) => {
                    let mut changed: Vec<PathBuf> = Vec::new();
                    for ev in events {
                        for path in &ev.paths {
                            changed.push(path.clone());
                        }
                    }
                    if !changed.is_empty() {
                        // Receiver gone => app shutting down; ignore.
                        let _ = tx.send(changed);
                    }
                }
                Err(errors) => {
                    for e in errors {
                        tracing::warn!(error = %e, "filesystem watch error");
                    }
                }
            },
        )?;

        for path in paths {
            if path.exists() {
                // notify-debouncer-full 0.5: Debouncer implements Watcher itself
                // and manages its file-id cache internally.
                debouncer.watch(path, RecursiveMode::Recursive)?;
                tracing::debug!(path = %path.display(), "watching");
            } else {
                tracing::debug!(path = %path.display(), "skip watching (missing)");
            }
        }

        Ok(Self {
            _debouncer: debouncer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn forwards_changes_for_watched_dir() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let _watcher = FsWatcher::spawn(&[dir.path().to_path_buf()], tx).unwrap();

        // Give the watcher a moment to register before mutating.
        std::thread::sleep(Duration::from_millis(300));
        let file = dir.path().join("s.jsonl");
        {
            let mut f = std::fs::File::create(&file).unwrap();
            f.write_all(b"{\"type\":\"user\"}\n").unwrap();
            f.flush().unwrap();
        }

        // Poll for a debounced batch (FSEvents has latency); generous timeout.
        let mut got = false;
        for _ in 0..100 {
            if let Ok(batch) = rx.try_recv() {
                if !batch.is_empty() {
                    got = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(got, "expected a filesystem change notification");
    }

    #[test]
    fn missing_paths_are_skipped_not_fatal() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let res = FsWatcher::spawn(&[PathBuf::from("/nonexistent-xyz-abc-123")], tx);
        assert!(res.is_ok());
    }
}
