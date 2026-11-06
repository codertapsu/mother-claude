//! How Claude Code sessions reach the MCP server: the Mother Claude plugin's
//! `.mcp.json` (docs/MCP.md, plan §5.4 as corrected).
//!
//! The plugin is the local-directory marketplace in our own data dir that
//! `claude::hooks` writes and the CLI loads in place, so the MCP entry is
//! written there with the other plugin files — never with `claude plugin
//! configure`, never under `~/.claude`:
//!
//! ```text
//! <app data>/claude-plugin/mother-claude/.mcp.json   the installed plugin: server "mc",
//!                                                    type http, this start's port
//! <app data>/claude-plugin/mother-claude/scripts/mc-mcp-headers   0755, POSIX sh
//! <app data>/mcp-plugin/mother-claude/…              owned sessions' inline copy (the
//!                                                    whole plugin, never registered)
//! <app data>/mcp.token                               0600: `mcm_…`, new every launch
//! ```
//!
//! Tools show up as `mcp__plugin_mother-claude_mc__<tool>` in every session.
//!
//! * **Foreign sessions** (terminal, VS Code, Desktop) get it from the
//!   installed plugin, which the CLI reads from our folder in place. The
//!   entry goes into that folder at every start and right after an install
//!   ([`refresh_installed_entry`]), but only once foreign control created
//!   the folder: a folder there makes `claude::hooks` ask the CLI about the
//!   plugin at startup (`claude plugin list`, which writes the CLI's own
//!   `.claude.json`), which it must not start doing for someone who never
//!   installed it.
//! * **Owned sessions** load their own copy inline (`--plugin-dir`, added
//!   to Path B's launch by `claude::control`), whether or not the plugin is
//!   installed for their account. Verified on Claude Code 2.1.319: an inline
//!   plugin shadows an installed one of the same name (one `mother-claude`,
//!   `source: mother-claude@inline`), so there is exactly one `mc` server and
//!   one copy of the hooks. Their headers helper adds `X-MC-Session` from
//!   `MOTHER_CLAUDE_SESSION`, which Mother Claude sets on those children.
//!   `--mcp-config` was not chosen: it would name the tools
//!   `mcp__mc__<tool>` there, and duplicate the plugin's server wherever the
//!   plugin is installed too.
//!
//! The helper is quoted in `.mcp.json` (`"\"${CLAUDE_PLUGIN_ROOT}/…\""`):
//! Claude Code runs a `headersHelper` through a shell after substituting the
//! root, and the data dir has a space in it on macOS. Unquoted, the helper
//! never ran and the CLI fell back to OAuth discovery against our server.
//!
//! With the setting off, both entries, their helpers and the token file are
//! removed and owned sessions get no `--plugin-dir`.

use std::path::Path;
use std::sync::RwLock;

use crate::claude::hooks::{sh_single_quote, PluginDir, PluginPaths, PLUGIN};
use crate::state::AppState;

/// The server's name in the plugin (tools: `mcp__plugin_mother-claude_mc__…`).
pub const SERVER_NAME: &str = "mc";
/// The token file in the app data dir (0600), read by the headers helper.
pub const TOKEN_FILE: &str = "mcp.token";
/// The variable naming an owned session to its headers helper.
pub const SESSION_ENV: &str = "MOTHER_CLAUDE_SESSION";
/// The prefix the model sees before a tool's name.
pub const TOOL_PREFIX: &str = "mcp__plugin_mother-claude_mc__";
/// Owned sessions' copy of the plugin, in the app data dir.
pub const OWNED_DIR: &str = "mcp-plugin";

const MCP_JSON: &str = include_str!("../../assets/claude-plugin/mother-claude/.mcp.json");
const MC_MCP_HEADERS: &str =
    include_str!("../../assets/claude-plugin/mother-claude/scripts/mc-mcp-headers");

/// The plugin directory owned sessions load inline, while the server is on
/// and its files were written at this start.
static OWNED_PLUGIN_DIR: RwLock<Option<String>> = RwLock::new(None);

/// The `--plugin-dir` for an owned session's launch, if any.
pub fn owned_plugin_dir() -> Option<String> {
    OWNED_PLUGIN_DIR
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

fn set_owned_plugin_dir(dir: Option<String>) {
    *OWNED_PLUGIN_DIR.write().unwrap_or_else(|p| p.into_inner()) = dir;
}

/// `.mcp.json` for the server listening on `port`.
pub fn mcp_json(port: u16) -> String {
    MCP_JSON.replace("@PORT@", &port.to_string())
}

/// The headers helper with the data dir baked in (like `mc-hook`).
pub fn helper_script(data_dir: &Path) -> String {
    MC_MCP_HEADERS.replace("'@MC_DATA@'", &sh_single_quote(&data_dir.to_string_lossy()))
}

/// Write the MCP entry and its helper into the plugin folder.
pub fn write_entry(paths: &PluginPaths, port: u16) -> std::io::Result<()> {
    write_file(&paths.plugin.root.join(".mcp.json"), &mcp_json(port), 0o644)?;
    write_file(
        &paths.plugin.root.join("scripts").join("mc-mcp-headers"),
        &helper_script(&paths.data_dir),
        0o755,
    )
}

/// Remove the MCP entry and its helper (missing files are fine).
pub fn remove_entry(paths: &PluginPaths) -> std::io::Result<()> {
    for path in [
        paths.plugin.root.join(".mcp.json"),
        paths.plugin.root.join("scripts").join("mc-mcp-headers"),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Write the token file (0600) for the helper.
pub fn write_token(data_dir: &Path, token: &str) -> std::io::Result<()> {
    crate::persist::write_private(&data_dir.join(TOKEN_FILE), format!("{token}\n").as_bytes())
}

/// Write a whole plugin folder with the MCP entry: what a session loads with
/// `--plugin-dir <paths.plugin.root>` (owned sessions, the live test).
pub fn write_all(paths: &PluginPaths, port: u16, token: &str) -> std::io::Result<()> {
    write_token(&paths.data_dir, token)?;
    crate::claude::hooks::write_plugin_files(paths)?;
    write_entry(paths, port)
}

/// Where owned sessions' copy of the plugin lives: the same plugin in its
/// own folder (`<app data>/mcp-plugin/mother-claude`), its scripts reading
/// the app data dir's runtime files like the installed copy's do.
pub fn owned_paths(data_dir: &Path) -> PluginPaths {
    let mut paths = PluginPaths::new(data_dir);
    paths.marketplace = data_dir.join(OWNED_DIR);
    let root = paths.marketplace.join(PLUGIN);
    paths.plugin = PluginDir {
        mc_hook: root.join("scripts").join("mc-hook"),
        mc_wake: root.join("scripts").join("mc-wake"),
        root,
    };
    paths
}

fn remove_token(data_dir: &Path) {
    match std::fs::remove_file(data_dir.join(TOKEN_FILE)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(error = %e, "could not remove the MCP token file"),
    }
}

/// Bring the files in the app data dir in line with the setting (at start
/// and when it changes): the token file, owned sessions' copy of the plugin
/// and the installed plugin's entry. No data dir (tests): nothing is
/// written and owned sessions get no plugin.
pub fn apply(state: &AppState) {
    let Some(dir) = state.auth.config_dir.as_deref() else {
        set_owned_plugin_dir(None);
        return;
    };
    let owned = owned_paths(dir);
    if state.mcp.enabled() {
        match write_all(&owned, state.config.port, state.mcp.token()) {
            Ok(()) => set_owned_plugin_dir(Some(owned.plugin.root.to_string_lossy().into_owned())),
            Err(e) => {
                set_owned_plugin_dir(None);
                tracing::warn!(error = %e, "could not write owned sessions' MCP plugin");
            }
        }
    } else {
        set_owned_plugin_dir(None);
        if let Err(e) = remove_entry(&owned) {
            tracing::warn!(error = %e, "could not remove owned sessions' MCP entry");
        }
        remove_token(dir);
    }
    refresh_installed_entry(state);
}

/// The installed plugin's entry, as the setting says: written (this start's
/// port) or removed, in a plugin folder foreign control created — never
/// creating one. Called at start and right after an install.
pub fn refresh_installed_entry(state: &AppState) {
    let Some(dir) = state.auth.config_dir.as_deref() else {
        return;
    };
    let installed = PluginPaths::new(dir);
    if !installed.plugin.root.is_dir() {
        return;
    }
    let result = if state.mcp.enabled() {
        write_entry(&installed, state.config.port)
    } else {
        remove_entry(&installed)
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "could not update the plugin's MCP entry");
    }
}

/// Whether the installed plugin's folder carries the MCP entry now.
pub fn entry_written(state: &AppState) -> bool {
    state
        .auth
        .config_dir
        .as_deref()
        .is_some_and(|d| PluginPaths::new(d).plugin.root.join(".mcp.json").is_file())
}

fn write_file(path: &Path, contents: &str, mode: u32) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&tmp, contents.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn the_entry_is_an_http_server_with_a_quoted_helper() {
        let v: Value = serde_json::from_str(&mcp_json(6725)).unwrap();
        let s = &v["mcpServers"][SERVER_NAME];
        assert_eq!(s["type"], "http");
        assert_eq!(s["url"], "http://127.0.0.1:6725/mcp");
        // Quoted: the CLI runs it through a shell, and the data dir has a
        // space on macOS.
        assert_eq!(
            s["headersHelper"],
            "\"${CLAUDE_PLUGIN_ROOT}/scripts/mc-mcp-headers\""
        );
        assert!(s.get("headers").is_none(), "no secret in the entry");
        assert!(!mcp_json(1).contains("@PORT@"));
        assert_eq!(
            TOOL_PREFIX,
            format!(
                "mcp__plugin_{}_{SERVER_NAME}__",
                crate::claude::hooks::PLUGIN
            )
        );
    }

    #[test]
    fn the_helper_carries_the_quoted_data_dir_and_no_secret() {
        let dir = Path::new("/Users/me/Library/Application Support/dev.x/it's");
        let s = helper_script(dir);
        assert!(s.contains("MC_DATA='/Users/me/Library/Application Support/dev.x/it'\\''s'\n"));
        assert!(!s.contains("@MC_DATA@"));
        assert!(!s.contains('\r'), "LF line endings only");
        assert!(
            !s.contains("mcm_") || s.contains("mcm_*"),
            "no token baked in"
        );
        assert!(s.starts_with("#!/bin/sh\n"));
    }

    #[cfg(unix)]
    #[test]
    fn the_helper_prints_the_headers() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Application Support");
        let paths = PluginPaths::new(&data);
        write_all(&paths, 6725, "mcm_AbC-_9").unwrap();
        let helper = paths.plugin.root.join("scripts/mc-mcp-headers");
        let run = |envs: &[(&str, &str)]| -> Value {
            let mut cmd = std::process::Command::new(&helper);
            cmd.env_clear().env("PATH", "/usr/bin:/bin");
            for (k, v) in envs {
                cmd.env(k, v);
            }
            let out = cmd.output().unwrap();
            assert!(out.status.success());
            assert!(out.stderr.is_empty());
            serde_json::from_slice(&out.stdout).unwrap()
        };
        let v = run(&[]);
        assert_eq!(v["Authorization"], "Bearer mcm_AbC-_9");
        assert_eq!(v["X-MC-Pid"], std::process::id().to_string());
        assert!(v.get("X-MC-Session").is_none());
        let sid = "0f1e2d3c-4b5a-4968-8776-655443322110";
        let v = run(&[("MOTHER_CLAUDE_OWNED", "1"), (SESSION_ENV, sid)]);
        assert_eq!(v["X-MC-Session"], sid);
        // Not owned, or not an id: no session header.
        assert!(run(&[(SESSION_ENV, sid)]).get("X-MC-Session").is_none());
        assert!(run(&[("MOTHER_CLAUDE_OWNED", "1"), (SESSION_ENV, "x y")])
            .get("X-MC-Session")
            .is_none());
        // No token file: `{}` plus the pid, still exit 0.
        std::fs::remove_file(data.join(TOKEN_FILE)).unwrap();
        let v = run(&[]);
        assert!(v.get("Authorization").is_none());
        // A malformed token is never sent.
        std::fs::write(data.join(TOKEN_FILE), "mcm_bad\"quote\n").unwrap();
        assert!(run(&[]).get("Authorization").is_none());
    }

    /// One test, as it sets the process-wide `--plugin-dir` (no other test
    /// here launches an owned session).
    #[tokio::test]
    async fn apply_follows_the_setting_and_never_creates_the_installed_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("app data");
        let mut auth = crate::server::auth::Auth::ephemeral();
        auth.config_dir = Some(data.clone());
        let st = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(tmp.path().join("home")),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 7123,
            },
            auth,
        );
        let owned = owned_paths(&data);
        let installed = PluginPaths::new(&data);

        apply(&st);
        assert_eq!(
            owned_plugin_dir().as_deref(),
            Some(owned.plugin.root.to_string_lossy().as_ref())
        );
        let entry: Value = serde_json::from_str(
            &std::fs::read_to_string(owned.plugin.root.join(".mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            entry["mcpServers"]["mc"]["url"],
            "http://127.0.0.1:7123/mcp"
        );
        assert!(owned
            .plugin
            .root
            .join(".claude-plugin/plugin.json")
            .is_file());
        assert!(owned.plugin.mc_hook.is_file());
        assert_eq!(
            std::fs::read_to_string(data.join(TOKEN_FILE))
                .unwrap()
                .trim(),
            st.mcp.token()
        );
        assert!(
            !installed.marketplace.exists(),
            "never created for foreign control"
        );
        assert!(!entry_written(&st));

        // Once foreign control wrote its folder, the entry goes there too.
        crate::claude::hooks::write_plugin_files(&installed).unwrap();
        apply(&st);
        assert!(entry_written(&st));

        // Off: no entries, no token, no inline plugin; the hooks stay.
        st.mcp.set_enabled(false).unwrap();
        apply(&st);
        assert_eq!(owned_plugin_dir(), None);
        assert!(!entry_written(&st));
        assert!(!owned.plugin.root.join(".mcp.json").exists());
        assert!(!data.join(TOKEN_FILE).exists());
        assert!(installed.plugin.mc_hook.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn files_are_written_and_removed_with_the_right_modes() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let paths = PluginPaths::new(tmp.path());
        write_all(&paths, 7001, "mcm_x").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&tmp.path().join(TOKEN_FILE)), 0o600);
        assert_eq!(
            mode(&paths.plugin.root.join("scripts/mc-mcp-headers")),
            0o755
        );
        assert_eq!(mode(&paths.plugin.root.join(".mcp.json")), 0o644);
        // The whole plugin is there for `--plugin-dir`.
        assert!(paths
            .plugin
            .root
            .join(".claude-plugin/plugin.json")
            .is_file());
        assert!(paths.plugin.root.join("hooks/hooks.json").is_file());
        remove_entry(&paths).unwrap();
        assert!(!paths.plugin.root.join(".mcp.json").exists());
        assert!(!paths.plugin.root.join("scripts/mc-mcp-headers").exists());
        assert!(paths.plugin.mc_hook.is_file(), "the hooks stay");
        remove_entry(&paths).unwrap();
    }
}
