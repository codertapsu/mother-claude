//! The foreign-control hooks plugin: assets, install, uninstall, status
//! (design §5.2, client side, with the §10 amendments).
//!
//! Owner: **B4 (foreign control)**.
//!
//! Foreign sessions (terminal, VS Code, desktop, JetBrains) are reached
//! through a user-scope Claude Code **plugin** — never by editing the `hooks`
//! of `~/.claude/settings.json`. The app writes a local-directory marketplace
//! into its data dir and asks the installed CLI to add and install it; the
//! CLI edits only `enabledPlugins` / `extraKnownMarketplaces`. Relative-path
//! plugins of a local marketplace load in place, so later versions of the
//! scripts reach every new session without reinstalling.
//!
//! ```text
//! <app data>/claude-plugin/.claude-plugin/marketplace.json   mother-claude-local
//! <app data>/claude-plugin/mother-claude/.claude-plugin/plugin.json
//! <app data>/claude-plugin/mother-claude/hooks/hooks.json    exec-form hooks
//! <app data>/claude-plugin/mother-claude/scripts/mc-hook     0755, POSIX sh
//! <app data>/claude-plugin/mother-claude/scripts/mc-wake     0755, POSIX sh
//! <app data>/hook.curlrc    0600: `header = "Authorization: Bearer mch_…"` only
//! <app data>/hook.base      `http://127.0.0.1:<port>` (not secret)
//! <app data>/hook.disabled  present after uninstall: the scripts exit 0
//! ```
//!
//! The scripts live in `scripts/`, not `bin/` (a plugin's `bin/` lands on the
//! model's Bash PATH), carry the absolute data dir (it contains a space on
//! macOS; exec-form hooks need no quoting), read `hook.curlrc` / `hook.base`
//! on every call (so a port or token change heals at once) and always exit 0
//! — except `mc-wake`'s exit 2, which is the wake. With
//! `MOTHER_CLAUDE_OWNED=1` (set on every child Mother Claude spawns) they exit
//! at once, so owned sessions never double-report.
//!
//! Install / uninstall run only on an explicit desktop-local action
//! (`routes/foreign.rs`), through the `claude plugin` CLI with a scrubbed
//! environment (never exporting `CLAUDE_CONFIG_DIR`), stdin closed and a 60 s
//! timeout. Uninstall keeps the files and writes `hook.disabled`, so hooks
//! still loaded in running sessions go quiet instead of failing every turn.
//!
//! AppState field: `state.plugin`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The marketplace name the plugin is installed from.
pub const MARKETPLACE: &str = "mother-claude-local";
/// The plugin name.
pub const PLUGIN: &str = "mother-claude";
/// `plugin@marketplace`, the install/enable key.
pub const PLUGIN_ID: &str = "mother-claude@mother-claude-local";

/// Timeout of one `claude plugin …` run.
const CLI_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `claude plugin list` answers are reused.
const STATUS_TTL_MS: i64 = 30_000;

/// Inherited variables that make a `claude` child think it runs inside
/// another session (or would leak our token). `CLAUDE_CONFIG_DIR` is
/// deliberately not here: a user-set value is inherited untouched, and we
/// never set one.
const ENV_DENYLIST: [&str; 8] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SSE_PORT",
    "MOTHER_CLAUDE_TOKEN",
];

// ─────────────────────────────── assets ───────────────────────────────

const MARKETPLACE_JSON: &str =
    include_str!("../../assets/claude-plugin/.claude-plugin/marketplace.json");
const PLUGIN_JSON: &str =
    include_str!("../../assets/claude-plugin/mother-claude/.claude-plugin/plugin.json");
const HOOKS_JSON: &str = include_str!("../../assets/claude-plugin/mother-claude/hooks/hooks.json");
const MC_HOOK: &str = include_str!("../../assets/claude-plugin/mother-claude/scripts/mc-hook");
const MC_WAKE: &str = include_str!("../../assets/claude-plugin/mother-claude/scripts/mc-wake");

/// Where everything lives, relative to the app data dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPaths {
    pub data_dir: PathBuf,
    /// The marketplace root (`claude plugin marketplace add <this>`).
    pub marketplace: PathBuf,
    /// The plugin directory (`--plugin-dir <this>` for a one-off session).
    pub plugin: PluginDir,
    pub curlrc: PathBuf,
    pub base: PathBuf,
    pub disabled: PathBuf,
}

/// The plugin directory and its scripts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDir {
    pub root: PathBuf,
    pub mc_hook: PathBuf,
    pub mc_wake: PathBuf,
}

impl PluginPaths {
    pub fn new(data_dir: &Path) -> Self {
        let marketplace = data_dir.join("claude-plugin");
        let root = marketplace.join(PLUGIN);
        Self {
            data_dir: data_dir.to_path_buf(),
            plugin: PluginDir {
                mc_hook: root.join("scripts").join("mc-hook"),
                mc_wake: root.join("scripts").join("mc-wake"),
                root,
            },
            marketplace,
            curlrc: data_dir.join("hook.curlrc"),
            base: data_dir.join("hook.base"),
            disabled: data_dir.join("hook.disabled"),
        }
    }
}

/// Quote `s` for a POSIX shell single-quoted string.
pub fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A script with the data dir baked in.
fn render_script(template: &str, data_dir: &Path) -> String {
    let quoted = sh_single_quote(&data_dir.to_string_lossy());
    template.replace("'@MC_DATA@'", &quoted)
}

/// The secret curl config: the hook token header only (a `url` line would
/// make curl send a second request).
pub fn curlrc_contents(hook_token: &str) -> String {
    // The token is `mch_` + base64url: nothing to escape.
    format!("header = \"Authorization: Bearer {hook_token}\"\n")
}

/// `hook.base`: the loopback listener.
pub fn base_contents(port: u16) -> String {
    format!("http://127.0.0.1:{port}\n")
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

/// Write the marketplace, the plugin and its scripts (LF, 0755).
pub fn write_plugin_files(paths: &PluginPaths) -> std::io::Result<()> {
    write_file(
        &paths
            .marketplace
            .join(".claude-plugin")
            .join("marketplace.json"),
        MARKETPLACE_JSON,
        0o644,
    )?;
    write_file(
        &paths.plugin.root.join(".claude-plugin").join("plugin.json"),
        &PLUGIN_JSON.replace("@VERSION@", env!("CARGO_PKG_VERSION")),
        0o644,
    )?;
    write_file(
        &paths.plugin.root.join("hooks").join("hooks.json"),
        HOOKS_JSON,
        0o644,
    )?;
    write_file(
        &paths.plugin.mc_hook,
        &render_script(MC_HOOK, &paths.data_dir),
        0o755,
    )?;
    write_file(
        &paths.plugin.mc_wake,
        &render_script(MC_WAKE, &paths.data_dir),
        0o755,
    )?;
    Ok(())
}

/// Write `hook.curlrc` (0600) and `hook.base`.
pub fn write_runtime_files(
    paths: &PluginPaths,
    port: u16,
    hook_token: &str,
) -> std::io::Result<()> {
    crate::persist::write_private(&paths.curlrc, curlrc_contents(hook_token).as_bytes())?;
    crate::persist::write_private(&paths.base, base_contents(port).as_bytes())?;
    Ok(())
}

// ───────────────────────────── the manager ─────────────────────────────

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
    /// Installed but turned off (the scripts exit at once).
    pub enabled: bool,
    /// Live foreign sessions that started before the install (they need
    /// `/reload-plugins` or a restart).
    pub sessions_needing_reload: u32,
    /// The marketplace directory (`claude plugin marketplace add` source).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_dir: Option<String>,
    /// The last install/uninstall/list error, verbatim from the CLI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct Cache {
    installed: bool,
    enabled: bool,
    version: Option<String>,
    cli_available: Option<bool>,
    checked_at: i64,
    last_error: Option<String>,
}

/// Plugin install state.
#[derive(Debug, Default)]
pub struct PluginManager {
    cache: Mutex<Cache>,
    installed: AtomicBool,
    disabled: AtomicBool,
    /// Serializes install/uninstall/list.
    busy: tokio::sync::Mutex<()>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cheap cached answer to "is the plugin installed (and enabled)?" (used
    /// by the summary builder for `ControlCaps`; never spawns a process).
    pub fn installed_cached(&self) -> bool {
        self.installed.load(Ordering::Relaxed)
    }

    /// Whether foreign control was turned off (the `hook.disabled` marker).
    pub fn disabled_cached(&self) -> bool {
        self.disabled.load(Ordering::Relaxed)
    }

    fn set(&self, f: impl FnOnce(&mut Cache)) {
        let mut c = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut c);
        self.installed
            .store(c.installed && c.enabled, Ordering::Relaxed);
    }

    fn snapshot(&self) -> Cache {
        self.cache.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The Default directory's plugin as last checked: `installed`, `off`,
    /// `missing`, or `None` before the CLI was asked.
    pub fn summary(&self) -> Option<&'static str> {
        let c = self.snapshot();
        c.cli_available?;
        Some(match (c.installed, c.enabled && !self.disabled_cached()) {
            (true, true) => "installed",
            (true, false) => "off",
            (false, _) => "missing",
        })
    }

    /// Record the installed state directly (tests; a caller that knows).
    pub fn set_installed(&self, installed: bool) {
        self.set(|c| {
            c.installed = installed;
            c.enabled = installed;
            c.checked_at = crate::claude::registry::now_ms();
        });
    }

    fn set_disabled(&self, disabled: bool) {
        self.disabled.store(disabled, Ordering::Relaxed);
    }
}

fn paths_of(state: &AppState) -> Option<PluginPaths> {
    state.auth.config_dir.as_deref().map(PluginPaths::new)
}

/// At startup: if the plugin was ever installed from this data dir, rewrite
/// its files (this app version's scripts) and the runtime files (the current
/// port and hook token), then refresh the installed state in the background.
/// Writes nothing when the plugin was never installed. Tests (no data dir)
/// skip it all.
pub fn on_start(state: &AppState) {
    let Some(paths) = paths_of(state) else {
        return;
    };
    state.plugin.set_disabled(paths.disabled.exists());
    if !paths.plugin.root.exists() {
        return;
    }
    if let Err(e) = write_plugin_files(&paths)
        .and_then(|()| write_runtime_files(&paths, state.config.port, &state.auth.hook_token))
    {
        tracing::warn!(error = %e, "could not refresh the foreign-control plugin files");
    }
    let st = state.clone();
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move {
            refresh_status(&st, true).await;
        });
    }
}

/// The CLI's own answer to "is it installed", from `claude plugin list
/// --json` (`[{id, version, scope, enabled, …}]`).
pub fn parse_list(json: &str) -> Option<(bool, bool, Option<String>)> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let entries = v.as_array()?;
    let hit = entries.iter().find(|e| {
        e.get("id").and_then(Value::as_str) == Some(PLUGIN_ID)
            && e.get("scope").and_then(Value::as_str).unwrap_or("user") == "user"
    });
    Some(match hit {
        Some(e) => (
            true,
            e.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            e.get("version").and_then(Value::as_str).map(str::to_string),
        ),
        None => (false, false, None),
    })
}

/// Whether `claude plugin marketplace list --json` names our marketplace.
pub fn marketplace_listed(json: &str) -> bool {
    serde_json::from_str::<Value>(json.trim())
        .ok()
        .and_then(|v| v.as_array().cloned())
        .is_some_and(|a| {
            a.iter()
                .any(|m| m.get("name").and_then(Value::as_str) == Some(MARKETPLACE))
        })
}

/// The output of one CLI run.
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The argv of every CLI call (verified with `claude plugin help …` on
/// 2.1.282).
pub mod argv {
    use super::{MARKETPLACE, PLUGIN_ID};

    pub fn list() -> Vec<String> {
        ["plugin", "list", "--json"].map(String::from).to_vec()
    }
    pub fn marketplace_list() -> Vec<String> {
        ["plugin", "marketplace", "list", "--json"]
            .map(String::from)
            .to_vec()
    }
    pub fn marketplace_add(dir: &str) -> Vec<String> {
        vec![
            "plugin".into(),
            "marketplace".into(),
            "add".into(),
            dir.into(),
            "--scope".into(),
            "user".into(),
        ]
    }
    pub fn install() -> Vec<String> {
        ["plugin", "install", PLUGIN_ID, "--scope", "user", "--json"]
            .map(String::from)
            .to_vec()
    }
    pub fn uninstall() -> Vec<String> {
        [
            "plugin",
            "uninstall",
            PLUGIN_ID,
            "--scope",
            "user",
            "--json",
        ]
        .map(String::from)
        .to_vec()
    }
    pub fn marketplace_remove() -> Vec<String> {
        [
            "plugin",
            "marketplace",
            "remove",
            MARKETPLACE,
            "--scope",
            "user",
        ]
        .map(String::from)
        .to_vec()
    }
    pub fn validate(dir: &str) -> Vec<String> {
        vec![
            "plugin".into(),
            "validate".into(),
            "--json".into(),
            dir.into(),
        ]
    }
}

/// Run `claude <args>` with the scrubbed environment, stdin closed and a
/// timeout. `Err` only when the CLI could not be run at all.
pub async fn run_cli(args: &[String]) -> Result<CliOutput, String> {
    run_cli_with(args, &crate::claude::accounts::AccountEnv::Inherit).await
}

/// [`run_cli`] for an account: a directory account's `claude plugin …`
/// manages the plugin in its own directory (docs/ACCOUNTS.md §5.7).
pub async fn run_cli_with(
    args: &[String],
    account: &crate::claude::accounts::AccountEnv,
) -> Result<CliOutput, String> {
    let mut cmd = tokio::process::Command::new(crate::claude::claude_bin());
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for k in ENV_DENYLIST {
        cmd.env_remove(k);
    }
    account.apply(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not run `claude`: {e}"))?;
    let out = tokio::time::timeout(CLI_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "`claude plugin` did not finish within 60 s".to_string())?
        .map_err(|e| format!("`claude plugin` failed: {e}"))?;
    Ok(CliOutput {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

fn cli_message(out: &CliOutput) -> String {
    let err = out.stderr.trim();
    let text = if err.is_empty() {
        out.stdout.trim()
    } else {
        err
    };
    let mut s: String = text.chars().take(2000).collect();
    if s.is_empty() {
        s = "the CLI reported a failure without a message".into();
    }
    s
}

/// Ask the CLI whether the plugin is installed (cached for 30 s unless
/// `force`).
pub async fn refresh_status(state: &AppState, force: bool) {
    let now = crate::claude::registry::now_ms();
    let cached = state.plugin.snapshot();
    if !force && cached.cli_available.is_some() && now - cached.checked_at < STATUS_TTL_MS {
        return;
    }
    let _busy = state.plugin.busy.lock().await;
    match run_cli(&argv::list()).await {
        Ok(out) if out.ok => match parse_list(&out.stdout) {
            Some((installed, enabled, version)) => state.plugin.set(|c| {
                c.installed = installed;
                c.enabled = enabled;
                c.version = version;
                c.cli_available = Some(true);
                c.checked_at = now;
            }),
            None => state.plugin.set(|c| {
                c.cli_available = Some(true);
                c.checked_at = now;
                c.last_error = Some("Could not read `claude plugin list --json`.".into());
            }),
        },
        Ok(out) => state.plugin.set(|c| {
            c.cli_available = Some(true);
            c.checked_at = now;
            c.last_error = Some(cli_message(&out));
        }),
        Err(e) => state.plugin.set(|c| {
            c.cli_available = Some(false);
            c.checked_at = now;
            c.last_error = Some(e);
        }),
    }
}

/// The status payload (refreshes the CLI view when stale).
pub async fn status(state: &AppState) -> ForeignStatus {
    if paths_of(state).is_some() {
        refresh_status(state, false).await;
    }
    let c = state.plugin.snapshot();
    let sessions = state.sessions().await;
    let mut with_hooks = 0u32;
    let mut needing_reload = 0u32;
    for s in sessions.iter().filter(|s| s.live && !s.owned) {
        if crate::server::hooks::hooks_active(state, &s.id, s.pid) {
            with_hooks += 1;
        } else {
            needing_reload += 1;
        }
    }
    let installed = c.installed;
    let enabled = installed && c.enabled && !state.plugin.disabled_cached();
    let mut notes = Vec::new();
    if paths_of(state).is_none() {
        notes.push(
            "This build has no data directory, so foreign control cannot be installed.".into(),
        );
    } else if c.cli_available == Some(false) {
        notes.push(
            "The `claude` command was not found, so the plugin cannot be managed from here.".into(),
        );
    } else if !installed {
        notes.push(
            "Not installed. Installing adds the Mother Claude plugin at user scope: sessions \
             started afterwards (terminal, VS Code, desktop) can be approved and messaged from \
             here. Your own hooks and settings are not touched."
                .into(),
        );
    } else {
        if !enabled {
            notes.push("Installed but turned off.".into());
        }
        if needing_reload > 0 {
            notes.push(format!(
                "{needing_reload} running session(s) started before foreign control was \
                 installed: run /reload-plugins in them, or restart them."
            ));
        }
    }
    ForeignStatus {
        installed,
        plugin_version: c.version.clone(),
        cli_available: c.cli_available.unwrap_or(false),
        last_event_at: state.hooks.last_any_event_at(),
        sessions_with_hooks: with_hooks,
        notes,
        enabled,
        sessions_needing_reload: if installed { needing_reload } else { 0 },
        plugin_dir: paths_of(state).map(|p| p.marketplace.to_string_lossy().into_owned()),
        last_error: c.last_error.clone(),
    }
}

fn failed(what: &str, msg: String, state: &AppState) -> ApiError {
    state.plugin.set(|c| c.last_error = Some(msg.clone()));
    ApiError::unprocessable("plugin_cli_failed", format!("{what} failed: {msg}"))
}

/// Install (desktop-local only; the route checks). Writes the files, then
/// `claude plugin marketplace add <dir> --scope user` (unless already known)
/// and `claude plugin install mother-claude@mother-claude-local --scope user`.
pub async fn install(state: &AppState) -> ApiResult<ForeignStatus> {
    let paths = paths_of(state).ok_or_else(|| {
        ApiError::conflict(
            "no_data_dir",
            "This build has no data directory to install the plugin from.",
        )
    })?;
    {
        let _busy = state.plugin.busy.lock().await;
        write_plugin_files(&paths)
            .and_then(|()| write_runtime_files(&paths, state.config.port, &state.auth.hook_token))
            .map_err(|e| ApiError::internal(format!("Could not write the plugin files: {e}")))?;
        let dir = paths.marketplace.to_string_lossy().into_owned();
        let known = match run_cli(&argv::marketplace_list()).await {
            Ok(out) if out.ok => marketplace_listed(&out.stdout),
            Ok(_) => false,
            Err(e) => return Err(failed("Running `claude`", e, state)),
        };
        if !known {
            let out = run_cli(&argv::marketplace_add(&dir))
                .await
                .map_err(|e| failed("Adding the marketplace", e, state))?;
            if !out.ok {
                return Err(failed("Adding the marketplace", cli_message(&out), state));
            }
        }
        let out = run_cli(&argv::install())
            .await
            .map_err(|e| failed("Installing the plugin", e, state))?;
        if !out.ok {
            return Err(failed("Installing the plugin", cli_message(&out), state));
        }
        if paths.disabled.exists() {
            std::fs::remove_file(&paths.disabled).map_err(|e| {
                ApiError::internal(format!("Could not turn foreign control back on: {e}"))
            })?;
        }
        state.plugin.set_disabled(false);
        state.plugin.set(|c| c.last_error = None);
        tracing::info!("foreign-control plugin installed");
    }
    refresh_status(state, true).await;
    Ok(status(state).await)
}

/// Uninstall (desktop-local only). Turns the scripts off first (running
/// sessions keep the hooks they loaded), releases every wake listener, then
/// `claude plugin uninstall …` and `claude plugin marketplace remove …`. The
/// files stay.
pub async fn uninstall(state: &AppState) -> ApiResult<ForeignStatus> {
    let paths = paths_of(state)
        .ok_or_else(|| ApiError::conflict("no_data_dir", "This build has no data directory."))?;
    {
        let _busy = state.plugin.busy.lock().await;
        crate::persist::write_private(&paths.disabled, b"foreign control turned off\n")
            .map_err(|e| ApiError::internal(format!("Could not turn the hooks off: {e}")))?;
        state.plugin.set_disabled(true);
        let released = state.hooks.release_all_waiters();
        tracing::info!(released, "foreign control turned off");
        let out = run_cli(&argv::uninstall())
            .await
            .map_err(|e| failed("Uninstalling the plugin", e, state))?;
        let not_installed = out.stderr.contains("not installed")
            || out.stdout.contains("not installed")
            || out.stderr.contains("not found");
        if !out.ok && !not_installed {
            return Err(failed("Uninstalling the plugin", cli_message(&out), state));
        }
        match run_cli(&argv::marketplace_remove()).await {
            Ok(out) if !out.ok => {
                tracing::warn!(error = %cli_message(&out), "could not remove the marketplace")
            }
            Err(e) => tracing::warn!(error = %e, "could not remove the marketplace"),
            Ok(_) => {}
        }
        state.plugin.set(|c| c.last_error = None);
    }
    refresh_status(state, true).await;
    Ok(status(state).await)
}

// ──────────────────────────── per account ────────────────────────────

/// The plugin's state in a directory account's own directory:
/// `installed`, `off` (installed, disabled), `missing`, or `unknown` when
/// the CLI could not say. Recorded in the registry (`hooksPlugin`).
pub async fn refresh_account(state: &AppState, account: &str) -> Option<String> {
    let a = state.accounts.get(account)?;
    if a.is_default() || a.kind != crate::claude::accounts::AccountKind::Directory {
        return None;
    }
    let env = state.accounts.env(account).ok()?;
    let answer = match run_cli_with(&argv::list(), &env).await {
        Ok(out) if out.ok => match parse_list(&out.stdout) {
            Some((true, true, _)) => "installed",
            Some((true, false, _)) => "off",
            Some((false, _, _)) => "missing",
            None => "unknown",
        },
        _ => "unknown",
    };
    state.accounts.note_hooks_plugin(account, answer);
    Some(answer.to_string())
}

/// Install the plugin in a directory account's directory: the same plugin
/// files (they are shared — the hook token authenticates the plugin, not an
/// account), registered with `claude plugin marketplace add` and `claude
/// plugin install` run with the account's environment. The Default uses
/// [`install`].
pub async fn install_for(state: &AppState, account: &str) -> ApiResult<String> {
    let env = directory_env(state, account)?;
    let paths = paths_of(state).ok_or_else(|| {
        ApiError::conflict(
            "no_data_dir",
            "This build has no data directory to install the plugin from.",
        )
    })?;
    {
        let _busy = state.plugin.busy.lock().await;
        write_plugin_files(&paths)
            .and_then(|()| write_runtime_files(&paths, state.config.port, &state.auth.hook_token))
            .map_err(|e| ApiError::internal(format!("Could not write the plugin files: {e}")))?;
        let dir = paths.marketplace.to_string_lossy().into_owned();
        let known = match run_cli_with(&argv::marketplace_list(), &env).await {
            Ok(out) if out.ok => marketplace_listed(&out.stdout),
            Ok(_) => false,
            Err(e) => return Err(failed("Running `claude`", e, state)),
        };
        if !known {
            let out = run_cli_with(&argv::marketplace_add(&dir), &env)
                .await
                .map_err(|e| failed("Adding the marketplace", e, state))?;
            if !out.ok {
                return Err(failed("Adding the marketplace", cli_message(&out), state));
            }
        }
        let out = run_cli_with(&argv::install(), &env)
            .await
            .map_err(|e| failed("Installing the plugin", e, state))?;
        if !out.ok {
            return Err(failed("Installing the plugin", cli_message(&out), state));
        }
        tracing::info!(account, "foreign-control plugin installed for an account");
    }
    Ok(refresh_account(state, account)
        .await
        .unwrap_or_else(|| "unknown".into()))
}

/// Uninstall the plugin from a directory account's directory. The shared
/// scripts stay on for the other accounts.
pub async fn uninstall_for(state: &AppState, account: &str) -> ApiResult<String> {
    let env = directory_env(state, account)?;
    {
        let _busy = state.plugin.busy.lock().await;
        let out = run_cli_with(&argv::uninstall(), &env)
            .await
            .map_err(|e| failed("Uninstalling the plugin", e, state))?;
        let not_installed = out.stderr.contains("not installed")
            || out.stdout.contains("not installed")
            || out.stderr.contains("not found");
        if !out.ok && !not_installed {
            return Err(failed("Uninstalling the plugin", cli_message(&out), state));
        }
        if let Ok(out) = run_cli_with(&argv::marketplace_remove(), &env).await {
            if !out.ok {
                tracing::warn!(error = %cli_message(&out), "could not remove the marketplace");
            }
        }
    }
    Ok(refresh_account(state, account)
        .await
        .unwrap_or_else(|| "unknown".into()))
}

/// A directory account's environment, or why the plugin is not its own.
fn directory_env(
    state: &AppState,
    account: &str,
) -> ApiResult<crate::claude::accounts::AccountEnv> {
    let a = state.accounts.get(account).ok_or_else(|| {
        ApiError::not_found("account_not_found", format!("No account {account:?}."))
    })?;
    if a.is_default() {
        return Err(ApiError::unprocessable(
            "use_default_install",
            "The Default account's plugin is managed with foreign control's own install.",
        ));
    }
    if a.kind != crate::claude::accounts::AccountKind::Directory {
        return Err(ApiError::unprocessable(
            "not_a_directory_account",
            "A token account shares the Default account's directory and its plugin.",
        ));
    }
    state
        .accounts
        .env(account)
        .map_err(|e| ApiError::unprocessable(e.code(), e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_get_the_quoted_data_dir() {
        let dir = Path::new("/Users/me/Library/Application Support/dev.x/it's");
        let s = render_script(MC_HOOK, dir);
        assert!(s.contains("MC_DATA='/Users/me/Library/Application Support/dev.x/it'\\''s'\n"));
        assert!(!s.contains("@MC_DATA@"));
        assert!(!s.contains('\r'), "LF line endings only");
        assert!(render_script(MC_WAKE, dir).contains("MC_DATA='/Users/me/Library/"));
        assert_eq!(sh_single_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn runtime_files_hold_only_what_they_must() {
        let rc = curlrc_contents("mch_abc-_1");
        assert_eq!(rc, "header = \"Authorization: Bearer mch_abc-_1\"\n");
        assert!(!rc.contains("url"));
        assert_eq!(base_contents(6725), "http://127.0.0.1:6725\n");
    }

    #[test]
    fn hooks_json_is_exec_form_and_complete() {
        let v: Value = serde_json::from_str(HOOKS_JSON).unwrap();
        let hooks = v["hooks"].as_object().unwrap();
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PermissionRequest",
            "PostToolUse",
            "PostToolUseFailure",
            "PermissionDenied",
            "Notification",
            "SubagentStart",
            "SubagentStop",
            "PreCompact",
            "PostCompact",
            "Stop",
            "StopFailure",
            "SessionEnd",
        ] {
            let entries = hooks[event][0]["hooks"].as_array().unwrap();
            for h in entries {
                assert_eq!(h["type"], "command");
                assert!(h["args"].is_array(), "{event}: exec form needs args");
                let cmd = h["command"].as_str().unwrap();
                assert!(cmd.starts_with("${CLAUDE_PLUGIN_ROOT}/scripts/"), "{cmd}");
                assert!(h["timeout"].as_u64().unwrap() > 0);
            }
            let first = &entries[0];
            if first["command"].as_str().unwrap().ends_with("mc-hook") {
                assert_eq!(first["args"][0], event);
            }
        }
        let decide = |e: &str| {
            hooks[e][0]["hooks"][0]["args"][2]
                .as_str()
                .unwrap()
                .to_string()
        };
        for e in [
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "SessionStart",
            "PermissionRequest",
        ] {
            assert_eq!(decide(e), "decide", "{e}");
        }
        for e in [
            "UserPromptSubmit",
            "Notification",
            "SessionEnd",
            "SubagentStop",
        ] {
            assert_eq!(decide(e), "observe", "{e}");
        }
        let pr = &hooks["PermissionRequest"][0]["hooks"][0];
        assert_eq!(pr["args"][1], "3590");
        assert_eq!(pr["timeout"], 3600);
        let wake = &hooks["Stop"][0]["hooks"][1];
        assert_eq!(wake["command"], "${CLAUDE_PLUGIN_ROOT}/scripts/mc-wake");
        assert_eq!(wake["asyncRewake"], true);
        assert_eq!(wake["timeout"], 86400);
        // No `bin/`: it would put the helpers on the model's PATH.
        assert!(!HOOKS_JSON.contains("/bin/"));
    }

    #[test]
    fn manifests_are_valid_json() {
        let m: Value = serde_json::from_str(MARKETPLACE_JSON).unwrap();
        assert_eq!(m["name"], MARKETPLACE);
        assert_eq!(m["plugins"][0]["name"], PLUGIN);
        assert_eq!(m["plugins"][0]["source"], "./mother-claude");
        let p: Value = serde_json::from_str(&PLUGIN_JSON.replace("@VERSION@", "1.2.3")).unwrap();
        assert_eq!(p["name"], PLUGIN);
        assert_eq!(p["version"], "1.2.3");
    }

    #[test]
    fn plugin_list_is_parsed() {
        let out = r#"[
          {"id":"figma@claude-plugins-official","version":"2.2.120","scope":"user","enabled":true},
          {"id":"mother-claude@mother-claude-local","version":"0.3.0","scope":"user","enabled":false}
        ]"#;
        assert_eq!(parse_list(out), Some((true, false, Some("0.3.0".into()))));
        assert_eq!(parse_list("[]"), Some((false, false, None)));
        assert_eq!(parse_list("not json"), None);
        assert!(marketplace_listed(
            r#"[{"name":"mother-claude-local","source":"directory"}]"#
        ));
        assert!(!marketplace_listed(
            r#"[{"name":"claude-plugins-official"}]"#
        ));
        assert!(!marketplace_listed("oops"));
    }

    #[test]
    fn cli_argv_matches_the_verified_syntax() {
        assert_eq!(argv::list(), ["plugin", "list", "--json"]);
        assert_eq!(
            argv::marketplace_add("/a b"),
            ["plugin", "marketplace", "add", "/a b", "--scope", "user"]
        );
        assert_eq!(
            argv::install(),
            [
                "plugin",
                "install",
                "mother-claude@mother-claude-local",
                "--scope",
                "user",
                "--json"
            ]
        );
        assert_eq!(
            argv::uninstall(),
            [
                "plugin",
                "uninstall",
                "mother-claude@mother-claude-local",
                "--scope",
                "user",
                "--json"
            ]
        );
        assert_eq!(
            argv::marketplace_remove(),
            [
                "plugin",
                "marketplace",
                "remove",
                "mother-claude-local",
                "--scope",
                "user"
            ]
        );
        assert!(!ENV_DENYLIST.contains(&"CLAUDE_CONFIG_DIR"));
        assert!(ENV_DENYLIST.contains(&"MOTHER_CLAUDE_TOKEN"));
    }

    #[cfg(unix)]
    #[test]
    fn files_are_written_with_the_right_modes() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Application Support");
        let paths = PluginPaths::new(&data);
        write_plugin_files(&paths).unwrap();
        write_runtime_files(&paths, 6725, "mch_x").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&paths.plugin.mc_hook), 0o755);
        assert_eq!(mode(&paths.plugin.mc_wake), 0o755);
        assert_eq!(mode(&paths.curlrc), 0o600);
        assert!(paths
            .marketplace
            .join(".claude-plugin/marketplace.json")
            .is_file());
        assert!(paths.plugin.root.join("hooks/hooks.json").is_file());
        let script = std::fs::read_to_string(&paths.plugin.mc_hook).unwrap();
        assert!(script.contains(&sh_single_quote(&data.to_string_lossy())));
        assert!(!paths.plugin.root.join("bin").exists());
        let plugin: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.plugin.root.join(".claude-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(plugin["version"], env!("CARGO_PKG_VERSION"));
    }
}
