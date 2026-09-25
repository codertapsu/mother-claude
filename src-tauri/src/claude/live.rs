//! Live session roster from `~/.claude/sessions/<pid>.json` (design §3.1).
//!
//! Owner: **B2 (data layer)** — STUB. What B2 implements: reading
//! `~/.claude/sessions/*.json` (NEVER `*.key`), watching the dir, liveness via
//! `libc::kill(pid, 0)` (the `libc` dependency is already present) plus the
//! `procStart` check (`ps -o lstart= -p <pid>` with `TZ=UTC`, cached), a 50 ms
//! retry on a JSON parse error, background job short ids from
//! `~/.claude/jobs/<short>/state.json`, and dropping the per-refresh
//! `claude agents --json --all` spawn.
//!
//! The record type is defined here now so B3 (short job ids) and B4 (tmux
//! pane, pidfile `status`/`waitingFor`) can code against it.
//! AppState field: `state.live`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One `~/.claude/sessions/<pid>.json` record. Every field optional; unknown
/// keys land in `extra`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LiveSession {
    pub pid: Option<u32>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<i64>,
    pub proc_start: Option<String>,
    pub version: Option<String>,
    /// `interactive` | `background` | …
    pub kind: Option<String>,
    /// `claude-vscode` | `cli` | `sdk-cli` | …
    pub entrypoint: Option<String>,
    pub name: Option<String>,
    pub name_source: Option<String>,
    /// `idle` | `busy` | `waiting` | …
    pub status: Option<String>,
    pub status_updated_at: Option<i64>,
    /// e.g. `permission prompt`, `input needed`.
    pub waiting_for: Option<String>,
    /// Short background job id.
    pub job_id: Option<String>,
    /// tmux target `session:@win.%pane`.
    pub tmux: Option<String>,
    pub bridge_session_id: Option<String>,
    pub state: Option<String>,
    pub detail: Option<String>,
    pub tempo: Option<Value>,
    pub needs: Option<Value>,
    pub updated_at: Option<i64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The live roster. STUB: empty.
#[derive(Debug, Default)]
pub struct LiveRoster {}

impl LiveRoster {
    pub fn new() -> Self {
        Self::default()
    }

    /// The live record of a session, if its process is alive. STUB.
    pub fn get(&self, _session_id: &str) -> Option<LiveSession> {
        None
    }

    /// Every live session. STUB.
    pub fn all(&self) -> Vec<LiveSession> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pidfile_parses_tolerantly() {
        let s: LiveSession = serde_json::from_str(
            r#"{"pid":42,"sessionId":"abc","kind":"interactive","status":"busy",
                "waitingFor":"permission prompt","tmux":"main:@1.%3","novel":true}"#,
        )
        .unwrap();
        assert_eq!(s.pid, Some(42));
        assert_eq!(s.tmux.as_deref(), Some("main:@1.%3"));
        assert_eq!(s.extra["novel"], true);
        let empty: LiveSession = serde_json::from_str("{}").unwrap();
        assert!(empty.pid.is_none());
    }
}
