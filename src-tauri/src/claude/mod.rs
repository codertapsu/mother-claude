//! Adapter layer over `~/.claude`.
//!
//! This module is the *only* place that knows the on-disk layout and formats of
//! Claude Code's research-preview internals. Everything above it works with the
//! tolerant types defined here, so a single module absorbs version churn.

pub mod home;
pub mod schema;
pub mod transcript;
pub mod watcher;

pub use home::{best_effort_decode, encode_cwd, ClaudeHome};
pub use schema::{
    parse_transcript, parse_transcript_line, AgentEntry, ContentBlock, Message, MessageContent,
    Roster, StateJson, TranscriptEvent, Usage,
};
pub use transcript::{read_all as read_transcript, TranscriptTailer};
pub use watcher::{FsWatcher, WatchSender};
