//! Owned-session control facade (design §5.1).
//!
//! Owner: **B3 (owned control)**. Every route that drives an owned session
//! goes through these functions, so B3 can replace the machinery in
//! `claude/control.rs` (Path B "full": `--permission-prompt-tool stdio`,
//! `initialize`, control requests, stream events, env scrubbing, reaper that
//! unmarks owned + withdraws pending) without touching the routes.
//!
//! The foundation implements what the existing `ControlRegistry` can already
//! do (spawn, continue in place, send a text message, kill) and returns
//! `501 not_implemented` for the rest. Pending items raised by owned sessions
//! go through `crate::pending::raise` with `source: Owned`; live deltas go out
//! with `crate::server::ws::publish_stream`; activity hints with
//! `state.activity.note_owned`; turn completion with
//! `crate::server::push::on_turn_finished`.

use serde::{Deserialize, Serialize};

use super::control::SpawnOptions;
use super::model::{CommandInfo, Delivery, ImageInput};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Effort levels the CLI accepts.
pub const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Permission modes a spawn/continue/mode change may request.
pub const PERMISSION_MODES: [&str; 6] = [
    "default",
    "acceptEdits",
    "plan",
    "auto",
    "bypassPermissions",
    "dontAsk",
];

/// `POST /api/sessions` body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpawnRequest {
    pub cwd: String,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    /// Legacy `"on"`/`"off"` thinking override.
    pub thinking: Option<String>,
    pub images: Vec<ImageInput>,
}

/// `POST /api/sessions/{id}/continue` body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContinueRequest {
    pub fork: Option<bool>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub thinking: Option<String>,
}

/// Whether a permission mode removes the human from the loop (bypass-class ⇒
/// the caller needs `ClientContext::may_dangerous`).
pub fn is_dangerous_mode(mode: Option<&str>) -> bool {
    mode.is_some_and(|m| crate::pending::UNGATED_MODES.contains(&m))
}

/// Reject unknown effort / thinking / permission-mode values.
pub fn validate_launch(
    effort: Option<&str>,
    thinking: Option<&str>,
    permission_mode: Option<&str>,
) -> ApiResult<()> {
    if let Some(e) = effort {
        if !EFFORT_LEVELS.contains(&e) {
            return Err(ApiError::unprocessable(
                "invalid_effort",
                format!("Unknown effort level {e:?} (expected one of {EFFORT_LEVELS:?})."),
            ));
        }
    }
    if let Some(t) = thinking {
        if t != "on" && t != "off" {
            return Err(ApiError::unprocessable(
                "invalid_thinking",
                format!("Unknown thinking value {t:?} (expected \"on\" or \"off\")."),
            ));
        }
    }
    if let Some(m) = permission_mode {
        if !PERMISSION_MODES.contains(&m) {
            return Err(ApiError::unprocessable(
                "invalid_permission_mode",
                format!("Unknown permission mode {m:?} (expected one of {PERMISSION_MODES:?})."),
            ));
        }
    }
    Ok(())
}

/// Whether Mother Claude holds a live control handle for `id`.
pub fn is_owned(state: &AppState, id: &str) -> bool {
    state.control.controls(id)
}

/// Spawn a new owned session; returns its id.
pub async fn spawn(state: &AppState, req: SpawnRequest) -> ApiResult<String> {
    validate_launch(
        req.effort.as_deref(),
        req.thinking.as_deref(),
        req.permission_mode.as_deref(),
    )?;
    if !req.images.is_empty() {
        return Err(ApiError::not_implemented("Images on a new session (B3)"));
    }
    let opts = SpawnOptions {
        cwd: req.cwd,
        prompt: req.prompt.unwrap_or_default(),
        model: req.model,
        effort: req.effort,
        thinking: req.thinking,
        permission_mode: req.permission_mode,
        resume: None,
    };
    state
        .control
        .spawn(state, opts)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))
}

/// Continue session `id` as an owned session; returns the (possibly new) id.
/// Foundation: resumes in place only (B3 adds `--fork-session` and the
/// live-elsewhere ⇒ fork rule).
pub async fn continue_session(
    state: &AppState,
    id: &str,
    cwd: &str,
    req: ContinueRequest,
) -> ApiResult<String> {
    validate_launch(
        req.effort.as_deref(),
        req.thinking.as_deref(),
        req.permission_mode.as_deref(),
    )?;
    if req.fork == Some(true) {
        return Err(ApiError::not_implemented("Forking a session (B3)"));
    }
    let opts = SpawnOptions {
        cwd: cwd.to_string(),
        prompt: req.prompt.unwrap_or_default(),
        model: req.model,
        effort: req.effort,
        thinking: req.thinking,
        permission_mode: req.permission_mode,
        resume: Some(id.to_string()),
    };
    state
        .control
        .spawn(state, opts)
        .await
        .map_err(|e| ApiError::conflict("continue_failed", e.to_string()))
}

/// Send a user message to an owned session.
pub async fn send_message(
    state: &AppState,
    id: &str,
    text: &str,
    images: &[ImageInput],
) -> ApiResult<Delivery> {
    if !images.is_empty() {
        return Err(ApiError::not_implemented("Images in messages (B3)"));
    }
    state
        .control
        .send_message(id, text)
        .await
        .map(|()| Delivery::Sent)
        .map_err(|e| ApiError::conflict("owned_session_unavailable", e.to_string()))
}

/// Interrupt the current turn. STUB.
pub async fn interrupt(_state: &AppState, _id: &str) -> ApiResult<()> {
    Err(ApiError::not_implemented(
        "Interrupting an owned session (B3)",
    ))
}

/// Switch model live. STUB.
pub async fn set_model(_state: &AppState, _id: &str, _model: &str) -> ApiResult<()> {
    Err(ApiError::not_implemented("Changing the model live (B3)"))
}

/// Switch permission mode live. STUB.
pub async fn set_mode(_state: &AppState, _id: &str, _mode: &str) -> ApiResult<()> {
    Err(ApiError::not_implemented(
        "Changing the permission mode live (B3)",
    ))
}

/// Switch effort live. STUB.
pub async fn set_effort(_state: &AppState, _id: &str, _effort: &str) -> ApiResult<()> {
    Err(ApiError::not_implemented("Changing the effort live (B3)"))
}

/// Rename the session. STUB.
pub async fn rename(_state: &AppState, _id: &str, _name: &str) -> ApiResult<()> {
    Err(ApiError::not_implemented("Renaming a session (B3)"))
}

/// Stop one background task of an owned session. STUB.
pub async fn stop_task(_state: &AppState, _id: &str, _task_id: &str) -> ApiResult<()> {
    Err(ApiError::not_implemented("Stopping a background task (B3)"))
}

/// Slash commands the owned session offers (from `initialize`). STUB: none
/// known, so the data route serves the static common list.
pub async fn commands(_state: &AppState, _id: &str) -> Option<Vec<CommandInfo>> {
    None
}

/// Kill an owned session's process (best effort).
pub async fn kill(state: &AppState, id: &str) -> ApiResult<()> {
    state
        .control
        .kill(id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_validation() {
        assert!(validate_launch(Some("high"), Some("on"), Some("plan")).is_ok());
        assert_eq!(
            validate_launch(Some("turbo"), None, None).unwrap_err().code,
            "invalid_effort"
        );
        assert_eq!(
            validate_launch(None, Some("maybe"), None).unwrap_err().code,
            "invalid_thinking"
        );
        assert_eq!(
            validate_launch(None, None, Some("yolo")).unwrap_err().code,
            "invalid_permission_mode"
        );
        assert!(is_dangerous_mode(Some("bypassPermissions")));
        assert!(!is_dangerous_mode(Some("plan")));
        assert!(!is_dangerous_mode(None));
    }
}
