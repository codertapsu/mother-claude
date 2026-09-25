//! The foreign-control hooks plugin: assets, install, uninstall, status
//! (design §5.2, client side).
//!
//! Owner: **B4 (foreign control)** — STUB. What B4 implements here:
//!
//! * embedded plugin assets written to `<app data dir>/claude-plugin/`
//!   (`marketplace.json`, `plugin.json`, `hooks/hooks.json`, `bin/mc-hook`,
//!   `bin/mc-wake`) plus `<app data dir>/hook.curlrc` (0600, the
//!   `url = "http://127.0.0.1:<port>/hooks/event"` and
//!   `header = "Authorization: Bearer <state.auth.hook_token>"`), rewritten on
//!   port/token change;
//! * install/uninstall ONLY on an explicit desktop-local action via
//!   `claude plugin marketplace add|remove` and
//!   `claude plugin install|uninstall mother-claude@mother-claude-local --scope user`
//!   (never edit `~/.claude/settings.json` hooks; never export
//!   `CLAUDE_CONFIG_DIR` onto the child);
//! * status via `claude plugin list --json` (cached) and recent hook traffic.
//!
//! Routes (`/api/control/foreign*`) live in `server/routes/foreign.rs` (B4).
//! AppState field: `state.plugin`.

use serde::{Deserialize, Serialize};

/// `GET /api/control/foreign` payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ForeignStatus {
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_version: Option<String>,
    pub cli_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<i64>,
    pub sessions_with_hooks: u32,
    pub notes: Vec<String>,
}

/// Plugin install state. STUB: never installed.
#[derive(Debug, Default)]
pub struct PluginManager {}

impl PluginManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cheap cached answer to "is the plugin installed?" (used by the summary
    /// builder for `ControlCaps` notes; must not spawn a process). STUB.
    pub fn installed_cached(&self) -> bool {
        false
    }
}
