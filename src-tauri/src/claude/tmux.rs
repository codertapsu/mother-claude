//! Driving foreign sessions that run inside tmux (design §5.4).
//!
//! Owner: **B4 (foreign control)** — STUB. What B4 implements: pane lookup
//! from the pidfile `tmux` field (`session:@win.%pane`, see
//! `claude::live::LiveSession::tmux`), socket discovery (default server, then
//! `$TMUX_TMPDIR` / `/tmp/tmux-$UID/*` with `list-panes -a -F '#{pane_pid}
//! #{pane_id}'` matched against the claude pid or its parents), and:
//!
//! * [`send_text`]: `send-keys -l <text>`, sleep 400 ms, `send-keys Enter`
//!   (multi-line: `load-buffer` + `paste-buffer -p`); check waitingFor/idle
//!   first;
//! * [`send_key`]: allowlisted keys (`Escape`, `Enter`, `Up`, `Down`, `Tab`,
//!   `1`…`9`);
//! * [`capture`]: `capture-pane -p -S -<lines>` for the terminal view.
//!
//! The message/interrupt dispatchers in `server/routes/control.rs` call these
//! when `SessionSummary.control.channel == Tmux`. AppState field: `state.tmux`.

use super::model::SessionSummary;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Keys `POST /sessions/{id}/keys` accepts.
pub const ALLOWED_KEYS: [&str; 14] = [
    "Escape", "Enter", "Up", "Down", "Tab", "1", "2", "3", "4", "5", "6", "7", "8", "9",
];

/// Resolved tmux sockets / panes. STUB.
#[derive(Debug, Default)]
pub struct TmuxState {}

impl TmuxState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The pane (`%12`) a session runs in, if found. STUB.
pub async fn pane_for(_state: &AppState, _session: &SessionSummary) -> Option<String> {
    None
}

/// Type `text` into the session's pane and press Enter. STUB.
pub async fn send_text(_state: &AppState, _session: &SessionSummary, _text: &str) -> ApiResult<()> {
    Err(ApiError::not_implemented("Typing into tmux sessions (B4)"))
}

/// Press one allowlisted key in the session's pane. STUB.
pub async fn send_key(_state: &AppState, _session: &SessionSummary, _key: &str) -> ApiResult<()> {
    Err(ApiError::not_implemented(
        "Sending keys to tmux sessions (B4)",
    ))
}

/// The last `lines` lines of the pane. STUB.
pub async fn capture(
    _state: &AppState,
    _session: &SessionSummary,
    _lines: u32,
) -> ApiResult<String> {
    Err(ApiError::not_implemented("Capturing tmux panes (B4)"))
}
