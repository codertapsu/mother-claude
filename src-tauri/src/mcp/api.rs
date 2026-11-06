//! The dashboard's view of the MCP server (Services > Claude Code tools):
//! `GET /api/mcp` and `PUT /api/mcp/settings`.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::server::CallRecord;
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// How long an MCP session counts as connected after its last request.
const CLIENT_WINDOW_MS: i64 = 15 * 60 * 1000;

/// One tool, as the panel lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// What the desktop knows about the installed plugin (desktop only).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    /// Installed at user scope (the Default's directory).
    pub installed: bool,
    /// Installed and turned on.
    pub enabled: bool,
    /// The version sessions load, as `claude plugin list` says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The version this app writes (`claude::hooks::PLUGIN_VERSION`).
    pub expected_version: String,
    /// Installed, but sessions load another version than this app writes:
    /// they get no tools until the plugin is updated.
    pub outdated: bool,
    pub cli_available: bool,
}

/// `GET /api/mcp`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub enabled: bool,
    /// Where sessions reach the server (this machine only).
    pub url: String,
    /// What the model sees before a tool's name.
    pub tool_prefix: String,
    pub tools: Vec<ToolInfo>,
    pub tool_count: usize,
    /// The plugin folder carries the MCP entry.
    pub entry_written: bool,
    /// Sessions Mother Claude launches load the plugin with the entry.
    pub owned_sessions: bool,
    /// MCP sessions heard from in the last 15 minutes.
    pub clients: usize,
    pub calls_total: u64,
    /// Newest first: time, session, tool, ok — never arguments.
    pub recent_calls: Vec<CallRecord>,
    /// The desktop only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginInfo>,
}

/// The status a caller may see (`with_plugin`, the desktop's: with the
/// plugin's state, which may ask the CLI — at most every 30 s).
pub async fn status(state: &AppState, with_plugin: bool) -> McpStatus {
    let mcp = &state.mcp;
    let registry = mcp.registry();
    let tools: Vec<ToolInfo> = registry
        .list()
        .map(|d| ToolInfo {
            name: d.name.clone(),
            title: d.title.clone(),
        })
        .collect();
    let plugin = if with_plugin {
        let f = crate::claude::hooks::status(state).await;
        let expected = crate::claude::hooks::PLUGIN_VERSION.to_string();
        let outdated = f.installed && f.plugin_version.as_deref() != Some(expected.as_str());
        Some(PluginInfo {
            installed: f.installed,
            enabled: f.enabled,
            version: f.plugin_version,
            expected_version: expected,
            outdated,
            cli_available: f.cli_available,
        })
    } else {
        None
    };
    McpStatus {
        enabled: mcp.enabled(),
        url: format!("http://127.0.0.1:{}/mcp", state.config.port),
        tool_prefix: super::plugin::TOOL_PREFIX.to_string(),
        tool_count: tools.len(),
        tools,
        entry_written: super::plugin::entry_written(state),
        owned_sessions: mcp.enabled() && super::plugin::owned_plugin_dir().is_some(),
        clients: mcp.clients_seen(CLIENT_WINDOW_MS),
        calls_total: mcp.calls_total(),
        recent_calls: mcp.recent_calls(),
        plugin,
    }
}

/// `GET /api/mcp` query.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StatusQuery {
    /// `false`: leave the plugin out (a poll), so the CLI is not asked.
    pub plugin: Option<bool>,
}

/// `GET /api/mcp`: the desktop, a paired device or a local script. The
/// plugin's state is the desktop's, and only when asked for (the default):
/// it may run `claude plugin list` (at most every 30 s), which a page that
/// polls must not do.
pub async fn get_status(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Query(q): Query<StatusQuery>,
) -> Json<McpStatus> {
    let with_plugin = ctx.is_desktop() && q.plugin.unwrap_or(true);
    Json(status(&state, with_plugin).await)
}

/// `PUT /api/mcp/settings` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsBody {
    pub enabled: bool,
}

/// `PUT /api/mcp/settings` (the desktop): turn the server on or off. The
/// plugin entry follows at once; sessions already running keep what they
/// loaded (they get `503` while it is off).
pub async fn put_settings(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<SettingsBody>,
) -> ApiResult<Json<McpStatus>> {
    state
        .mcp
        .set_enabled(body.enabled)
        .map_err(|e| ApiError::internal(format!("Could not save the setting: {e}")))?;
    super::plugin::apply(&state);
    tracing::info!(enabled = body.enabled, "Claude Code tools (MCP) switched");
    Ok(Json(status(&state, true).await))
}
