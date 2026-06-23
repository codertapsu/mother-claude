//! Adapter layer over `~/.claude`.
//!
//! This module is the *only* place that knows the on-disk layout and formats of
//! Claude Code's research-preview internals. Everything above it works with the
//! tolerant types defined here, so a single module absorbs version churn.

pub mod control;
#[cfg(feature = "experimental")]
pub mod experimental;
pub mod git;
pub mod home;
pub mod registry;
pub mod schema;
pub mod transcript;
pub mod watcher;

/// Resolve the Claude Code binary: `$MOTHER_CLAUDE_CLI` if set, else `claude`.
pub fn claude_bin() -> String {
    std::env::var("MOTHER_CLAUDE_CLI")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "claude".to_string())
}

pub use control::{foreign_injection_enabled, ControlRegistry, OwnedSessionMeta, SpawnOptions};
pub use git::{
    file_patch, overview as git_overview, CommitInfo, FileChange, GitOverview, WorktreeInfo,
};
pub use home::{best_effort_decode, encode_cwd, ClaudeHome};
pub use registry::{
    build_registry, query_agents, read_state_jsons, scan_transcripts, summarize_transcript,
    PendingInput, PendingKind, RegistryInputs, Session, SessionState, Surface, TranscriptSummary,
    UsageSummary,
};
pub use schema::{
    parse_transcript, parse_transcript_line, AgentEntry, ContentBlock, Message, MessageContent,
    Roster, StateJson, TranscriptEvent, Usage,
};
pub use transcript::{read_all as read_transcript, TranscriptTailer};
pub use watcher::{FsWatcher, WatchSender};
