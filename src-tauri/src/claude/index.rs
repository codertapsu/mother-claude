//! Incremental transcript index (design §3.2).
//!
//! Owner: **B2 (data layer)** — STUB. What B2 implements: one `FileIndex` per
//! transcript (`{path, inode, size, offset, partial_line_buf, aggregates}`),
//! locating transcripts by session id (`projects/*/<sid>.jsonl`, cwd drift),
//! lazy background bootstrap (bounded concurrency 1, streaming, lines > 8 MB
//! skipped after extracting type/message.id/usage), tail-window bootstrap for
//! live files, persisted checkpoints in `<app data dir>/index-cache.json`, and
//! the aggregates listed in the design (usage de-dup by message.id keeping the
//! LAST record, model, effort, permission mode, titles, last prompt/message,
//! branch, PR link, cost state, context fill, todos, queue, pending tool uses,
//! background tasks, last error, compact boundaries, remote control).
//!
//! AppState field: `state.index`.

use std::path::{Path, PathBuf};

/// The index. STUB.
#[derive(Debug, Default)]
pub struct TranscriptIndex {
    /// `<app data dir>/index-cache.json`, or `None` (tests: in-memory).
    pub cache_path: Option<PathBuf>,
}

impl TranscriptIndex {
    /// Tolerant constructor; `None` ⇒ no persisted checkpoints.
    pub fn new(data_dir: Option<&Path>) -> Self {
        Self {
            cache_path: data_dir.map(|d| d.join("index-cache.json")),
        }
    }

    /// The transcript file of a session, located by id (independent of cwd).
    /// STUB: unknown — callers fall back to `ClaudeHome::transcript_path`.
    pub fn transcript_path(&self, _session_id: &str) -> Option<PathBuf> {
        None
    }
}
