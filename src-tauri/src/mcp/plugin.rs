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
//! **On Windows** that shell is `cmd.exe` (verified on Claude Code 2.1.317),
//! which cannot run an sh script: the helper is its PowerShell twin,
//! `scripts/mc-mcp-headers.ps1`, and the entry starts Windows PowerShell on
//! it, both by full paths ([`windows_headers_helper`]). Without it every
//! session's MCP connection went out with no token (401), then OAuth
//! discovery answered 503: "HTTP 503 trying to load OAuth metadata". As
//! cmd.exe stays between the CLI and the helper there, the helper names the
//! first ancestor that is not a shell. Owned sessions' copy carries no hooks
//! on Windows: each would cost a PowerShell start and only exit at once.
//!
//! With the setting off, both entries, their helpers and the token file are
//! removed and owned sessions get no `--plugin-dir`.

use std::path::Path;
use std::sync::RwLock;

#[cfg(any(not(windows), test))]
use crate::claude::hooks::sh_single_quote;
use crate::claude::hooks::{remove_if_present, PluginDir, PluginPaths, PLUGIN};
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
#[cfg(any(not(windows), test))]
const MC_MCP_HEADERS: &str =
    include_str!("../../assets/claude-plugin/mother-claude/scripts/mc-mcp-headers");
#[cfg(any(windows, test))]
const MC_MCP_HEADERS_PS1: &str =
    include_str!("../../assets/claude-plugin/mother-claude/scripts/mc-mcp-headers.ps1");

/// The headers helper's file name on this platform (`scripts/<name>`).
#[cfg(windows)]
const HELPER: &str = "mc-mcp-headers.ps1";
#[cfg(not(windows))]
const HELPER: &str = "mc-mcp-headers";
/// Both platforms' names: what removing the entry removes.
const ALL_HELPERS: [&str; 2] = ["mc-mcp-headers", "mc-mcp-headers.ps1"];

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

/// `.mcp.json` for the server listening on `port`, for a plugin folder at
/// `root`: the asset as it is, or on Windows with the PowerShell helper
/// ([`windows_headers_helper`]).
pub fn mcp_json(port: u16, root: &Path) -> String {
    let json = MCP_JSON.replace("@PORT@", &port.to_string());
    #[cfg(windows)]
    {
        windows_entry(
            &json,
            &crate::claude::hooks::windows_powershell(),
            &root.join("scripts").join(HELPER),
        )
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        json
    }
}

/// The `headersHelper` command line on Windows, which cmd.exe runs: Windows
/// PowerShell on the helper, both quoted (the data dir has a space in it).
#[cfg(any(windows, test))]
pub fn windows_headers_helper(powershell: &Path, script: &Path) -> String {
    format!(
        "\"{}\" {} \"{}\"",
        powershell.display(),
        crate::claude::hooks::POWERSHELL_ARGS.join(" "),
        script.display()
    )
}

/// `json` (an `.mcp.json`) with its server's helper set to the Windows one.
#[cfg(any(windows, test))]
fn windows_entry(json: &str, powershell: &Path, script: &Path) -> String {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(json) else {
        return json.to_string();
    };
    v["mcpServers"][SERVER_NAME]["headersHelper"] =
        windows_headers_helper(powershell, script).into();
    serde_json::to_string_pretty(&v).map_or_else(|_| json.to_string(), |s| s + "\n")
}

/// The headers helper with the data dir baked in (like `mc-hook`).
pub fn helper_script(data_dir: &Path) -> String {
    #[cfg(windows)]
    {
        crate::claude::hooks::render_ps_script(MC_MCP_HEADERS_PS1, data_dir)
    }
    #[cfg(not(windows))]
    {
        sh_helper_script(data_dir)
    }
}

/// The sh helper with the data dir baked in.
#[cfg(any(not(windows), test))]
fn sh_helper_script(data_dir: &Path) -> String {
    MC_MCP_HEADERS.replace("'@MC_DATA@'", &sh_single_quote(&data_dir.to_string_lossy()))
}

/// Write the MCP entry and its helper into the plugin folder.
pub fn write_entry(paths: &PluginPaths, port: u16) -> std::io::Result<()> {
    write_file(
        &paths.plugin.root.join(".mcp.json"),
        &mcp_json(port, &paths.plugin.root),
        0o644,
    )?;
    write_file(
        &paths.plugin.root.join("scripts").join(HELPER),
        &helper_script(&paths.data_dir),
        0o755,
    )?;
    // An earlier version's sh helper, which nothing starts any more here.
    #[cfg(windows)]
    remove_if_present(&paths.plugin.root.join("scripts").join("mc-mcp-headers"))?;
    Ok(())
}

/// Remove the MCP entry and its helper (missing files are fine).
pub fn remove_entry(paths: &PluginPaths) -> std::io::Result<()> {
    remove_if_present(&paths.plugin.root.join(".mcp.json"))?;
    for name in ALL_HELPERS {
        remove_if_present(&paths.plugin.root.join("scripts").join(name))?;
    }
    Ok(())
}

/// Write the token file (0600) for the helper.
pub fn write_token(data_dir: &Path, token: &str) -> std::io::Result<()> {
    crate::persist::write_private(&data_dir.join(TOKEN_FILE), format!("{token}\n").as_bytes())
}

/// Write a whole plugin folder with the MCP entry: what a session loads with
/// `--plugin-dir <paths.plugin.root>` (owned sessions, the live test). On
/// Windows without hooks: owned sessions' hooks exit at once, and there each
/// would cost a PowerShell start first.
pub fn write_all(paths: &PluginPaths, port: u16, token: &str) -> std::io::Result<()> {
    write_token(&paths.data_dir, token)?;
    crate::claude::hooks::write_manifests(paths)?;
    if cfg!(windows) {
        crate::claude::hooks::remove_hooks(paths)?;
    } else {
        crate::claude::hooks::write_hooks(paths)?;
    }
    write_entry(paths, port)
}

/// Where owned sessions' copy of the plugin lives: the same plugin in its
/// own folder (`<app data>/mcp-plugin/mother-claude`), its scripts reading
/// the app data dir's runtime files like the installed copy's do.
pub fn owned_paths(data_dir: &Path) -> PluginPaths {
    let mut paths = PluginPaths::new(data_dir);
    paths.marketplace = data_dir.join(OWNED_DIR);
    paths.plugin = PluginDir::at(paths.marketplace.join(PLUGIN));
    paths
}

fn remove_token(data_dir: &Path) {
    if let Err(e) = remove_if_present(&data_dir.join(TOKEN_FILE)) {
        tracing::warn!(error = %e, "could not remove the MCP token file");
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
        let root = Path::new("/data dir/mcp-plugin/mother-claude");
        let v: Value = serde_json::from_str(&mcp_json(6725, root)).unwrap();
        let s = &v["mcpServers"][SERVER_NAME];
        assert_eq!(s["type"], "http");
        assert_eq!(s["url"], "http://127.0.0.1:6725/mcp");
        // Quoted: the CLI runs it through a shell, and the data dir has a
        // space on macOS (and on Windows).
        let helper = if cfg!(windows) {
            windows_headers_helper(
                &crate::claude::hooks::windows_powershell(),
                &root.join("scripts").join(HELPER),
            )
        } else {
            "\"${CLAUDE_PLUGIN_ROOT}/scripts/mc-mcp-headers\"".to_string()
        };
        assert_eq!(s["headersHelper"], helper);
        assert!(s.get("headers").is_none(), "no secret in the entry");
        assert!(!mcp_json(1, root).contains("@PORT@"));
        assert_eq!(
            TOOL_PREFIX,
            format!(
                "mcp__plugin_{}_{SERVER_NAME}__",
                crate::claude::hooks::PLUGIN
            )
        );
    }

    /// What cmd.exe gets on Windows: PowerShell and the helper, both quoted,
    /// the rest of the entry untouched.
    #[test]
    fn the_windows_entry_starts_powershell_on_the_helper() {
        let ps = Path::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        let script = Path::new(
            r"C:\Users\O'Brien\AppData\Roaming\motherclaude\Mother Claude\data\mcp-plugin\mother-claude\scripts\mc-mcp-headers.ps1",
        );
        let line = windows_headers_helper(ps, script);
        assert_eq!(
            line,
            "\"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile \
             -NonInteractive -ExecutionPolicy Bypass -File \"C:\\Users\\O'Brien\\AppData\\Roaming\\\
             motherclaude\\Mother Claude\\data\\mcp-plugin\\mother-claude\\scripts\\mc-mcp-headers.ps1\""
        );
        let json = MCP_JSON.replace("@PORT@", "6725");
        let v: Value = serde_json::from_str(&windows_entry(&json, ps, script)).unwrap();
        let s = &v["mcpServers"][SERVER_NAME];
        assert_eq!(s["headersHelper"], line);
        assert_eq!(s["url"], "http://127.0.0.1:6725/mcp");
        assert_eq!(s["type"], "http");
    }

    #[test]
    fn the_helper_carries_the_quoted_data_dir_and_no_secret() {
        let dir = Path::new("/Users/me/Library/Application Support/dev.x/it's");
        let s = sh_helper_script(dir);
        assert!(s.contains("MC_DATA='/Users/me/Library/Application Support/dev.x/it'\\''s'\n"));
        assert!(!s.contains("@MC_DATA@"));
        assert!(!s.contains('\r'), "LF line endings only");
        assert!(
            !s.contains("mcm_") || s.contains("mcm_*"),
            "no token baked in"
        );
        assert!(s.starts_with("#!/bin/sh\n"));

        // The Windows twin: a PowerShell string, behind a BOM.
        let dir = Path::new("C:\\Users\\O'Brien\u{2019}s\\Mother Claude\\data");
        let s = crate::claude::hooks::render_ps_script(MC_MCP_HEADERS_PS1, dir);
        assert!(s.starts_with('\u{FEFF}'));
        assert!(
            s.contains("$McData = 'C:\\Users\\O''Brien\u{2019}\u{2019}s\\Mother Claude\\data'\n")
        );
        assert!(!s.contains("@MC_DATA@"));
        assert!(MC_MCP_HEADERS_PS1.is_ascii(), "the template stays ASCII");
        assert!(!s.contains("mcm_A"), "no token baked in");
        assert_eq!(helper_script(dir).starts_with('\u{FEFF}'), cfg!(windows));
    }

    /// The helper as Claude Code runs it on Windows: the entry's command line
    /// through `cmd.exe /d /s /c "…"` (Node's shell form), from this process,
    /// in a data dir with a space, a quote and a non-ASCII letter in its name.
    #[cfg(windows)]
    #[test]
    fn the_helper_prints_the_headers_through_cmd() {
        use std::os::windows::process::CommandExt;
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Mother Claude").join("it's d\u{e1}ta");
        let paths = owned_paths(&data);
        write_all(&paths, 6725, "mcm_AbC-_9").unwrap();
        let entry: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.plugin.root.join(".mcp.json")).unwrap(),
        )
        .unwrap();
        let line = entry["mcpServers"][SERVER_NAME]["headersHelper"]
            .as_str()
            .unwrap()
            .to_string();
        let system_root = std::env::var("SystemRoot").unwrap();
        let run = |envs: &[(&str, &str)]| -> Value {
            let mut cmd = std::process::Command::new(format!(r"{system_root}\System32\cmd.exe"));
            cmd.raw_arg(format!("/d /s /c \"{line}\""))
                .env_clear()
                .env("SystemRoot", &system_root)
                .env("PATH", format!(r"{system_root}\System32"));
            for (k, v) in envs {
                cmd.env(k, v);
            }
            let out = cmd.output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                out.stderr.is_empty(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            serde_json::from_slice(&out.stdout)
                .unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&out.stdout)))
        };
        let v = run(&[]);
        assert_eq!(v["Authorization"], "Bearer mcm_AbC-_9");
        // Past cmd.exe: the process that ran the command, as the CLI is.
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
        // No token file: no Authorization, still the pid and exit 0.
        std::fs::remove_file(data.join(TOKEN_FILE)).unwrap();
        let v = run(&[]);
        assert!(v.get("Authorization").is_none());
        assert_eq!(v["X-MC-Pid"], std::process::id().to_string());
        // A malformed token is never sent.
        std::fs::write(data.join(TOKEN_FILE), "mcm_bad\"quote\n").unwrap();
        assert!(run(&[]).get("Authorization").is_none());
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
        // An earlier version's copy, hooks included.
        std::fs::create_dir_all(owned.plugin.root.join("hooks")).unwrap();
        std::fs::write(owned.plugin.root.join("hooks/hooks.json"), "{}").unwrap();

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
        assert!(owned.plugin.root.join("scripts").join(HELPER).is_file());
        // Hooks, except on Windows, where they would only cost a PowerShell
        // start each and exit.
        assert_eq!(owned.plugin.mc_hook.is_file(), !cfg!(windows));
        assert_eq!(
            owned.plugin.root.join("hooks/hooks.json").is_file(),
            !cfg!(windows)
        );
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
        assert!(!owned.plugin.root.join("scripts").join(HELPER).exists());
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
