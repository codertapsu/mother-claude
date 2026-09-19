//! Discovery: health, capabilities, metadata, models, and the OpenAPI document.
//!
//! `GET /capabilities` is the contract a client should read instead of
//! hardcoding limits — it reports every cap, which backends are usable right
//! now, and whether this listener wants a token.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde_json::{json, Value};

use super::ok;
use crate::bridge::error::BridgeResult;
use crate::bridge::host::{host_entry, RPC_TIMEOUT};
use crate::bridge::{events, inputs, messages, ops, MountPolicy};
use crate::state::AppState;

/// `GET /health` — lifecycle only. It does not call the model, refresh a login
/// or acquire any lock, so it stays fast and side-effect free.
pub async fn health(State(state): State<AppState>) -> Response {
    let host_built = host_entry().is_some();
    let body = json!({
        "status": if host_built { "ready" } else { "unavailable" },
        "backend": "claude-agent-sdk",
        "app": "mother-claude",
        "version": env!("CARGO_PKG_VERSION"),
        "host_running": state.bridge.host.is_running().await,
        "host_built": host_built,
        "messages_api": state.bridge.messages.is_configured(),
        "active_operations": state.bridge.ops.active_count(),
        // Whether Claude itself can reach Anthropic. Identifying details live
        // on GET /auth; a liveness probe does not need the account's email.
        "claude_authenticated": state.claude_auth.read().await.logged_in,
    });
    if host_built {
        ok(body)
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, axum::Json(body)).into_response()
    }
}

/// `GET /capabilities` — the machine-readable contract.
pub async fn capabilities(
    State(state): State<AppState>,
    Extension(policy): Extension<MountPolicy>,
) -> Response {
    ok(json!({
        "app": "mother-claude",
        "version": env!("CARGO_PKG_VERSION"),
        "backends": {
            "agent_sdk": {
                "available": host_entry().is_some(),
                "features": [
                    "threads", "turns", "streaming", "steering", "interrupt",
                    "tool_approval", "questions", "fork", "structured_output",
                    "images", "documents", "mcp", "skills", "subagents",
                    "model_switching", "permission_modes", "context_usage",
                ],
            },
            "messages_api": {
                "available": state.bridge.messages.is_configured(),
                "features": [
                    "vision", "vision_coordinates", "files", "tool_use",
                    "programmatic_tool_calling", "server_tools",
                    "structured_output", "streaming", "token_counting",
                ],
                "requires": "ANTHROPIC_API_KEY",
            },
        },
        "limits": {
            "max_active_operations": ops::MAX_ACTIVE,
            "max_retained_operations": ops::MAX_RETAINED,
            "operation_retention_seconds": ops::RETENTION.as_secs(),
            "event_replay_limit": events::MAX_EVENTS,
            "event_replay_bytes": events::MAX_BYTES,
            "max_request_bytes": crate::bridge::MAX_BODY_BYTES,
            "max_image_bytes": inputs::MAX_IMAGE_BYTES,
            "max_image_edge": inputs::MAX_IMAGE_EDGE,
            "min_image_edge": inputs::MIN_IMAGE_EDGE,
            "max_images_per_message": inputs::MAX_IMAGES_PER_MESSAGE,
            "max_text_chars": inputs::MAX_TEXT_CHARS,
            "run_timeout_seconds": crate::bridge::RUN_TIMEOUT_SECS,
            "request_timeout_seconds": crate::bridge::host::REQUEST_TIMEOUT.as_secs(),
            "image_media_types": inputs::IMAGE_MEDIA_TYPES,
        },
        "defaults": {
            "cwd": super::threads::default_cwd(),
            "permission_mode": "default",
            "efforts": super::threads::EFFORT_LEVELS,
            "permission_modes": super::threads::PERMISSION_MODES,
            "messages_model": messages::DEFAULT_MODEL,
        },
        "auth": {
            // This mount's policy, not the other listener's: the loopback port
            // may be tokenless while the LAN mount never is.
            "require_token": policy.require_token,
            "token_sources": ["Authorization: Bearer", "?token=", "mc_token cookie"],
        },
        "notes": [
            "Conversations are process-local: a conversation created here is lost \
             on restart until its first turn has written a transcript.",
            "Operations and event logs are in memory only and do not survive a restart.",
            "Requests are never retried or replayed; a disconnected client does not \
             interrupt a running turn.",
        ],
    }))
}

/// `GET /metadata` — versions of everything in the chain.
pub async fn metadata(State(state): State<AppState>) -> Response {
    let cli = tokio::process::Command::new(crate::claude::claude_bin())
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    ok(json!({
        "app": "mother-claude",
        "version": env!("CARGO_PKG_VERSION"),
        "backend": "claude-agent-sdk",
        "claude_cli": cli,
        "claude_home": state.home.base().to_string_lossy(),
        "host_entry": host_entry().map(|p| p.to_string_lossy().into_owned()),
        "anthropic_version": messages::ANTHROPIC_VERSION,
    }))
}

/// `GET /models` — what this account can select.
///
/// A live conversation can report the rich `ModelInfo` list (effort support and
/// all); with none running, the user's own saved Claude settings are the next
/// best answer, and cost nothing to read.
pub async fn models(State(state): State<AppState>) -> BridgeResult<Response> {
    if let Some(thread) = state.bridge.threads().await.first() {
        if let Ok(result) = state
            .bridge
            .host
            .call(
                &state,
                "session.models",
                json!({ "sessionId": thread.thread_id }),
                RPC_TIMEOUT,
            )
            .await
        {
            if let Some(models) = result.get("models") {
                return Ok(ok(json!({ "data": models, "source": "session" })));
            }
        }
    }

    let defaults = crate::claude::read_launch_defaults(&state.home);
    let data: Vec<Value> = defaults
        .models
        .iter()
        .map(|m| {
            json!({
                "value": m.value,
                "displayName": m.label,
                "description": m.description,
            })
        })
        .collect();
    Ok(ok(json!({
        "data": data,
        "source": "settings",
        "selected": defaults.model,
        "effort": defaults.effort,
        "efforts": super::threads::EFFORT_LEVELS,
    })))
}

/// `GET /auth` — which Anthropic account Claude is signed in as.
///
/// Cached from startup; `?refresh=true` re-runs the check, which spawns the
/// CLI and therefore is not what a polling client should do.
pub async fn auth(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let status = if params.get("refresh").map(String::as_str) == Some("true") {
        state.refresh_claude_auth().await
    } else {
        state.claude_auth.read().await.clone()
    };
    ok(json!({
        "authenticated": status.logged_in,
        "summary": status.summary(),
        "method": status.auth_method,
        "api_provider": status.api_provider,
        "email": status.email,
        "organization": status.org_name,
        "subscription": status.subscription_type,
        "config_directory": status.config_directory,
        "error": status.error,
        "login_hint": "POST /auth/login, or run `claude auth login` in a terminal.",
    }))
}

/// `POST /auth/login` — start the interactive sign-in flow.
///
/// Opens a browser on *this* machine, so it is restricted to local clients for
/// the same reason every other irreversible action is. It returns as soon as the
/// flow is launched: waiting for a human to finish an OAuth round trip inside an
/// HTTP handler would only ever produce a timeout. Poll `GET /auth?refresh=true`.
pub async fn auth_login(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    super::Body(body): super::Body<LoginBody>,
) -> BridgeResult<Response> {
    if crate::server::auth::dangerous_blocked(
        true,
        crate::server::auth::is_loopback(&peer),
        state.auth.allow_remote_dangerous,
    ) {
        return Err(crate::bridge::error::BridgeError::forbidden(
            "dangerous_blocked",
            "Signing in opens a browser on the host machine, so it is restricted \
             to local clients.",
        ));
    }

    crate::claude::auth::begin_login(body.console)
        .await
        .map_err(|e| crate::bridge::error::BridgeError::claude_error(e.to_string()))?;

    Ok(ok(json!({
        "status": "started",
        "message": "A browser sign-in flow was launched on the host machine.                     Poll GET /auth?refresh=true until `authenticated` is true.",
    })))
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginBody {
    /// Sign in with an Anthropic Console account (API billing) instead of a
    /// Claude subscription.
    #[serde(default)]
    pub console: bool,
}

/// `GET /openapi.json` — generated per request so it always advertises this
/// listener's actual authentication policy.
pub async fn openapi(
    State(state): State<AppState>,
    Extension(policy): Extension<MountPolicy>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
) -> Response {
    // Strip our own filename to learn the prefix we are mounted at, so the
    // document's `servers` entry points back at this mount rather than at a
    // guessed root.
    let mount = uri
        .path()
        .strip_suffix("/openapi.json")
        .unwrap_or_default()
        .to_string();
    ok(crate::bridge::openapi::document(
        policy.require_token,
        state.bridge.messages.is_configured(),
        &mount,
    ))
}
