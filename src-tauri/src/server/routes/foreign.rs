//! Foreign-session control routes (design §4.5 "Foreign control", §5.2–5.4,
//! G2).
//!
//! Owner: **B4 (foreign control)**. The foundation implements the plugin
//! status (from what it can see cheaply), the G2-sanitized experimental PTY
//! paths, and `501` stubs for install/uninstall, keys and terminal.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::Body;
use crate::claude::hooks::ForeignStatus;
use crate::claude::model::{ControlChannel, SessionSummary};
use crate::claude::tmux;
use crate::server::auth::{require_dangerous, Authed, DangerousOk, DesktopOnly};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// G2: make text safe to type into a TUI. Strips C0 controls and ESC (so no
/// key sequences can be smuggled in), turns newlines into spaces, and refuses a
/// leading `!` (bash mode).
pub fn sanitize_pty_text(text: &str) -> ApiResult<String> {
    let cleaned: String = text
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if (c as u32) < 0x20 || c == '\u{7f}' => None,
            // C1 controls (incl. CSI 0x9b).
            c if ('\u{80}'..='\u{9f}').contains(&c) => None,
            c => Some(c),
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.starts_with('!') {
        return Err(ApiError::unprocessable(
            "refused_shell_escape",
            "Text starting with `!` would run a shell command in the session; refused.",
        ));
    }
    if cleaned.is_empty() {
        return Err(ApiError::unprocessable("empty_message", "Nothing to type."));
    }
    Ok(cleaned)
}

/// Type sanitized text into a background session over experimental PTY
/// attach (G2: needs `ctx.local` or an `allowDangerous` device).
#[cfg(feature = "experimental")]
pub async fn pty_inject_text(
    state: &AppState,
    ctx: &ClientContext,
    session: &SessionSummary,
    text: &str,
) -> ApiResult<()> {
    require_dangerous(ctx, "Typing into a foreign session through PTY attach")?;
    if !crate::claude::foreign_injection_enabled() {
        return Err(ApiError::conflict(
            "pty_disabled",
            "Foreign-session PTY injection is disabled (MOTHER_CLAUDE_FOREIGN_INJECTION=0).",
        ));
    }
    if session.owned {
        return Err(ApiError::conflict(
            "owned_session",
            "Owned sessions are driven directly, not through PTY.",
        ));
    }
    if !session.live {
        return Err(ApiError::conflict(
            "session_not_running",
            "The session is not running; cannot attach.",
        ));
    }
    let text = sanitize_pty_text(text)?;
    let target = session.job_id.clone().unwrap_or_else(|| session.id.clone());
    state
        .pty
        .attach(state, &target, &session.cwd)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    state
        .pty
        .inject(&target, &text)
        .map_err(|e| ApiError::unprocessable("pty_inject_failed", e.to_string()))
}

#[cfg(not(feature = "experimental"))]
pub async fn pty_inject_text(
    _state: &AppState,
    ctx: &ClientContext,
    _session: &SessionSummary,
    text: &str,
) -> ApiResult<()> {
    require_dangerous(ctx, "Typing into a foreign session through PTY attach")?;
    sanitize_pty_text(text)?;
    Err(ApiError::conflict(
        "pty_unavailable",
        "Foreign-session PTY injection is not compiled into this build.",
    ))
}

async fn session(state: &AppState, id: &str) -> ApiResult<SessionSummary> {
    state
        .find_session(id)
        .await
        .ok_or_else(|| ApiError::no_such_session(id))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct KeyBody {
    pub key: String,
}

/// `POST /api/sessions/{id}/keys {key}` (tmux only, L).
pub async fn keys(
    DangerousOk(_ctx): DangerousOk,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<KeyBody>,
) -> ApiResult<Json<Value>> {
    if !tmux::ALLOWED_KEYS.contains(&body.key.as_str()) {
        return Err(ApiError::unprocessable(
            "invalid_key",
            format!("Key must be one of {:?}.", tmux::ALLOWED_KEYS),
        ));
    }
    let s = session(&state, &id).await?;
    if s.control.channel != ControlChannel::Tmux {
        return Err(super::control::no_channel(&s));
    }
    tmux::send_key(&state, &s, &body.key).await?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/sessions/{id}/terminal` → `{text}` (tmux capture-pane; B4 adds
/// `claude logs` for background sessions).
pub async fn terminal(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let s = session(&state, &id).await?;
    if s.control.channel == ControlChannel::Tmux {
        let text = tmux::capture(&state, &s, 200).await?;
        return Ok(Json(json!({ "text": text })));
    }
    Err(ApiError::not_implemented(
        "Terminal view for this session (B4)",
    ))
}

/// `GET /api/control/foreign` (desktop).
pub async fn status(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
) -> Json<ForeignStatus> {
    let sessions = state.sessions().await;
    let with_hooks = sessions
        .iter()
        .filter(|s| crate::server::hooks::is_active(&state, &s.id))
        .count() as u32;
    Json(ForeignStatus {
        installed: state.plugin.installed_cached(),
        plugin_version: None,
        cli_available: false,
        last_event_at: state.hooks.last_any_event_at(),
        sessions_with_hooks: with_hooks,
        notes: vec!["Foreign-session control is not installed from this build yet.".into()],
    })
}

/// `POST /api/control/foreign/install` (desktop, local). STUB.
pub async fn install(DesktopOnly(ctx): DesktopOnly) -> ApiResult<Json<Value>> {
    if !ctx.local {
        return Err(ApiError::dangerous_blocked(
            "Installing the plugin is only possible on this computer.",
        ));
    }
    Err(ApiError::not_implemented(
        "Installing foreign-session control (B4)",
    ))
}

/// `POST /api/control/foreign/uninstall` (desktop, local). STUB.
pub async fn uninstall(DesktopOnly(ctx): DesktopOnly) -> ApiResult<Json<Value>> {
    if !ctx.local {
        return Err(ApiError::dangerous_blocked(
            "Uninstalling the plugin is only possible on this computer.",
        ));
    }
    Err(ApiError::not_implemented(
        "Uninstalling foreign-session control (B4)",
    ))
}

/// `POST /api/hooks/install` — retired: it edited `~/.claude/settings.json`
/// hooks, which Mother Claude must never do. Foreign control now ships as a
/// plugin (`/api/control/foreign/install`).
pub async fn legacy_install_hooks(Authed(_ctx): Authed) -> Response {
    ApiError::new(
        StatusCode::GONE,
        "gone",
        "Hook installation into settings.json was retired. Use POST \
         /api/control/foreign/install (the Mother Claude plugin) instead.",
    )
    .into_response()
}

/// `POST /api/sessions/{id}/pty-attach` (experimental, L).
pub async fn pty_attach(
    DangerousOk(_ctx): DangerousOk,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    #[cfg(feature = "experimental")]
    {
        let s = session(&state, &id).await?;
        let target = s.job_id.clone().unwrap_or_else(|| s.id.clone());
        state
            .pty
            .attach(&state, &target, &s.cwd)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(StatusCode::NO_CONTENT.into_response())
    }
    #[cfg(not(feature = "experimental"))]
    {
        let _ = (&state, &id);
        Err(ApiError::not_found(
            "not_found",
            "PTY attach is not compiled into this build.",
        ))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TextBody {
    pub text: String,
}

/// `POST /api/sessions/{id}/pty-inject {text}` (experimental, L, sanitized).
pub async fn pty_inject(
    DangerousOk(ctx): DangerousOk,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<TextBody>,
) -> ApiResult<Response> {
    let s = session(&state, &id).await?;
    pty_inject_text(&state, &ctx, &s, &body.text).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g2_sanitizing() {
        assert_eq!(sanitize_pty_text("hello\nworld").unwrap(), "hello world");
        assert_eq!(sanitize_pty_text("a\x1b[2Jb\x07c").unwrap(), "a[2Jbc");
        assert_eq!(sanitize_pty_text("x\u{9b}31my").unwrap(), "x31my");
        assert_eq!(sanitize_pty_text("  1 ").unwrap(), "1");
        assert_eq!(
            sanitize_pty_text("!rm -rf ~").unwrap_err().code,
            "refused_shell_escape"
        );
        assert_eq!(
            sanitize_pty_text("\x1b!ls").unwrap_err().code,
            "refused_shell_escape"
        );
        assert_eq!(
            sanitize_pty_text("\x1b\x1b").unwrap_err().code,
            "empty_message"
        );
    }
}
