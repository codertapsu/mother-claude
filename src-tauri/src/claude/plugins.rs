//! Claude Code's plugins, managed from Mother Claude (docs/PLUGINS.md): what
//! is installed and where it is turned on, the marketplaces plugins come
//! from, what they offer, and the desktop's explicit changes. Each change is
//! one `claude plugin …` run — Claude Code itself writes its settings, never
//! our code — with the account's environment (a directory account manages
//! its own directory) and, for a project's scopes, in that project's folder.
//!
//! Reading answers what the CLI says, field by field and tolerant of what it
//! adds; a plugin's MCP servers are named with their kind and host only (a
//! header or an environment value can hold a secret). Mother Claude's own
//! plugin is shown but changed only from *Sessions started elsewhere*.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::claude::accounts::AccountEnv;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Where a plugin is turned on: for this person everywhere (`user`), for a
/// project and everyone who works on it (`project`, its
/// `.claude/settings.json`), or for a project and only this person
/// (`local`, its `.claude/settings.local.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    Project,
    Local,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
            Scope::Local => "local",
        }
    }

    /// Its settings live in a project's folder.
    pub fn needs_project(self) -> bool {
        !matches!(self, Scope::User)
    }
}

/// Reading: listing what is installed, the marketplaces, the catalog.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// A change may clone a repository (an install, an update, a marketplace).
const CHANGE_TIMEOUT: Duration = Duration::from_secs(300);
/// How long the catalog of available plugins is reused.
const CATALOG_TTL_MS: i64 = 10 * 60_000;
/// The most plugins one catalog search answers.
pub const SEARCH_LIMIT: usize = 60;

/// What one `claude plugin …` run said.
#[derive(Debug, Clone)]
pub struct CliRun {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl CliRun {
    /// The last line of its output that is a JSON object (`--json` prints
    /// one result line, after anything else it says).
    pub fn result(&self) -> Option<Value> {
        self.stdout
            .lines()
            .rev()
            .map(str::trim)
            .filter(|l| l.starts_with('{'))
            .find_map(|l| serde_json::from_str::<Value>(l).ok())
    }

    /// What went wrong, in the CLI's words (bounded).
    pub fn message(&self) -> String {
        let from_json = self.result().and_then(|r| {
            ["error", "message"]
                .iter()
                .find_map(|k| r.get(*k).and_then(Value::as_str).map(str::to_string))
        });
        let text = from_json.unwrap_or_else(|| {
            let err = self.stderr.trim();
            if err.is_empty() {
                self.stdout.trim().to_string()
            } else {
                err.to_string()
            }
        });
        let s: String = text.chars().take(2000).collect();
        if s.is_empty() {
            "Claude Code reported a failure without a message.".into()
        } else {
            s
        }
    }
}

/// Runs `claude <args>` with the account's environment (and none of the
/// variables that would make it think it runs inside a session), stdin
/// closed, in `cwd` when given, within `timeout`.
async fn run(
    args: &[String],
    env: &AccountEnv,
    cwd: Option<&Path>,
    timeout: Duration,
) -> ApiResult<CliRun> {
    let mut cmd = crate::proc::command(crate::claude::claude_bin());
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for k in crate::claude::hooks::ENV_DENYLIST {
        cmd.env_remove(k);
    }
    env.apply(&mut cmd);
    let child = cmd.spawn().map_err(|e| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "claude_unavailable",
            format!("Claude Code could not be started: {e}"),
        )
    })?;
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "plugin_cli_timeout",
                format!("Claude Code did not finish within {} s.", timeout.as_secs()),
            )
        })?
        .map_err(|e| ApiError::internal(format!("`claude plugin` failed: {e}")))?;
    Ok(CliRun {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

fn failed(run: &CliRun) -> ApiError {
    ApiError::unprocessable("plugin_cli_failed", run.message())
}

// ─────────────────────────────── checks ───────────────────────────────

fn name_like(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && !s.starts_with(['-', '.'])
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A GitHub owner's or repository's name.
fn repo_part(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && !s.starts_with('-')
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `name` or `name@marketplace`.
pub fn valid_plugin_id(s: &str) -> bool {
    match s.split_once('@') {
        Some((name, market)) => name_like(name, 120) && name_like(market, 120),
        None => name_like(s, 120),
    }
}

pub fn valid_marketplace_name(s: &str) -> bool {
    name_like(s, 120)
}

/// Where a marketplace comes from: a GitHub `owner/repo` (optionally
/// `#ref`), an `https://`, `ssh://` or `git@host:path` git URL, or a folder
/// (or `marketplace.json`) on this computer. Never anything that reads as
/// an option.
pub fn check_source(raw: &str) -> ApiResult<String> {
    let s = raw.trim();
    let bad = |why: &str| ApiError::unprocessable("invalid_source", why.to_string());
    if s.is_empty() {
        return Err(bad(
            "Say where the marketplace is: a GitHub owner/repo, a git URL or a folder.",
        ));
    }
    if s.len() > 1000 || s.chars().any(char::is_control) || s.starts_with('-') {
        return Err(bad("That is not a marketplace source."));
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("ssh://") || lower.starts_with("git@") {
        return Ok(s.to_string());
    }
    if lower.starts_with("http://") {
        return Err(bad(
            "Use an https:// address: plugins run code on this computer.",
        ));
    }
    let path = Path::new(s);
    if path.is_absolute() {
        return if path.exists() {
            Ok(s.to_string())
        } else {
            Err(bad("There is no such folder or file on this computer."))
        };
    }
    let github = |t: &str| {
        let (repo, reference) = t.split_once('#').unwrap_or((t, ""));
        let mut parts = repo.split('/');
        let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
            return false;
        };
        repo_part(owner)
            && repo_part(name)
            && reference.len() <= 200
            && reference
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    };
    if github(s) {
        return Ok(s.to_string());
    }
    Err(bad(
        "Use a GitHub owner/repo (e.g. anthropics/claude-plugins-official), an https:// git URL, \
         or the full path of a folder on this computer.",
    ))
}

/// The project folder a project or local scope writes into: an existing
/// folder on this computer, never inside a Claude configuration directory.
pub fn check_project(state: &AppState, raw: &str) -> ApiResult<PathBuf> {
    let bad = |why: &str| ApiError::unprocessable("invalid_project", why.to_string());
    let p = Path::new(raw.trim());
    if raw.trim().is_empty() || !p.is_absolute() {
        return Err(bad("Choose the project's folder."));
    }
    let dir =
        dunce::canonicalize(p).map_err(|_| bad("There is no such folder on this computer."))?;
    if !dir.is_dir() {
        return Err(bad("That is not a folder."));
    }
    let config =
        dunce::canonicalize(state.home.base()).unwrap_or_else(|_| state.home.base().to_path_buf());
    if dir.starts_with(&config) {
        return Err(bad(
            "That folder is Claude Code's own configuration, not a project.",
        ));
    }
    Ok(dir)
}

/// Two paths name the same folder (as written, or once resolved).
fn same_folder(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (dunce::canonicalize(a), dunce::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

/// The account's environment (`None`: the Default, ours unchanged).
pub fn account_env(state: &AppState, account: Option<&str>) -> ApiResult<AccountEnv> {
    match account.map(str::trim).filter(|a| !a.is_empty()) {
        None => Ok(AccountEnv::Inherit),
        Some(id) => state
            .accounts
            .env(id)
            .map_err(crate::claude::owned::account_error),
    }
}

// ─────────────────────────────── reading ───────────────────────────────

/// An MCP server a plugin brings, without anything that could be a secret.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    pub name: String,
    /// `stdio`, `http`, `sse`, …
    pub kind: String,
    /// An http server's host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// A stdio server's program, its name only (never its arguments).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
}

fn servers_of(v: Option<&Value>) -> Vec<ServerView> {
    let Some(map) = v.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<ServerView> = map
        .iter()
        .map(|(name, cfg)| {
            let url = cfg.get("url").and_then(Value::as_str);
            let kind = cfg
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| if url.is_some() { "http" } else { "stdio" }.to_string());
            let host = url.and_then(|u| {
                let rest = u.split_once("://").map(|(_, r)| r).unwrap_or(u);
                let host = rest.split(['/', '?', '#']).next().unwrap_or("");
                // Never credentials in the authority.
                let host = host.rsplit('@').next().unwrap_or(host);
                (!host.is_empty()).then(|| host.chars().take(200).collect())
            });
            let program = cfg.get("command").and_then(Value::as_str).and_then(|c| {
                Path::new(c)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.chars().take(100).collect())
            });
            ServerView {
                name: name.chars().take(200).collect(),
                kind: kind.chars().take(20).collect(),
                host,
                program,
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// One installed plugin, as this page shows it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    /// `name@marketplace`.
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `user`, `project`, `local`, `managed`, `synced`, … as Claude Code
    /// says.
    pub scope: String,
    pub enabled: bool,
    /// `enabled` holds: Claude Code says whether a plugin is on for the
    /// folder it runs in, so a project's own plugin is known only when the
    /// list was read in that project.
    pub enabled_known: bool,
    /// On in the project the list was read in (when one was).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_enabled: Option<bool>,
    /// The project folder of a `project` or `local` plugin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    pub mcp_servers: Vec<ServerView>,
    /// Mother Claude's own (changed from Sessions started elsewhere).
    pub ours: bool,
    /// Its scope is one this page changes (user, project, local).
    pub changeable: bool,
    /// What Claude Code notes about it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(|s| s.chars().take(300).collect())
}

pub fn parse_installed(v: &Value) -> Vec<Installed> {
    let list = v
        .as_array()
        .or_else(|| v.get("installed").and_then(Value::as_array));
    let Some(list) = list else {
        return Vec::new();
    };
    let mut out: Vec<Installed> = list
        .iter()
        .filter_map(|p| {
            let id = text(p, "id")?;
            let (name, marketplace) = match id.split_once('@') {
                Some((n, m)) => (n.to_string(), Some(m.to_string())),
                None => (id.clone(), None),
            };
            let scope = text(p, "scope").unwrap_or_else(|| "user".into());
            let notes = p
                .get("notes")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .take(5)
                        .map(|s| s.chars().take(300).collect())
                        .collect()
                })
                .unwrap_or_default();
            Some(Installed {
                ours: id == crate::claude::hooks::PLUGIN_ID,
                changeable: matches!(scope.as_str(), "user" | "project" | "local"),
                version: text(p, "folderVersion").or_else(|| text(p, "version")),
                enabled: p.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                enabled_known: true,
                project_enabled: p.get("projectEnabled").and_then(Value::as_bool),
                project: p
                    .get("projectPath")
                    .and_then(Value::as_str)
                    .filter(|_| matches!(scope.as_str(), "project" | "local"))
                    .map(|s| s.chars().take(1000).collect()),
                installed_at: text(p, "installedAt"),
                updated_at: text(p, "lastUpdated"),
                mcp_servers: servers_of(p.get("mcpServers")),
                id,
                name,
                marketplace,
                scope,
                notes,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.scope.cmp(&b.scope)));
    out
}

/// A marketplace plugins come from.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Marketplace {
    pub name: String,
    /// `github`, `git`, `url`, `directory`, `file`, … as Claude Code says.
    pub source: String,
    /// Its repository, address or folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub ours: bool,
}

pub fn parse_marketplaces(v: &Value) -> Vec<Marketplace> {
    let Some(list) = v.as_array() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|m| {
            let name = text(m, "name")?;
            Some(Marketplace {
                ours: name == crate::claude::hooks::MARKETPLACE,
                source: text(m, "source").unwrap_or_default(),
                location: text(m, "repo")
                    .or_else(|| text(m, "url"))
                    .or_else(|| text(m, "path")),
                name,
            })
        })
        .collect()
}

/// A plugin a marketplace offers.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub id: String,
    pub name: String,
    pub description: String,
    pub marketplace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_count: Option<u64>,
}

pub fn parse_available(v: &Value) -> Vec<Available> {
    let Some(list) = v.get("available").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|p| {
            let id = text(p, "pluginId")?;
            let name = text(p, "name").unwrap_or_else(|| id.split('@').next().unwrap_or("").into());
            Some(Available {
                description: p
                    .get("description")
                    .and_then(Value::as_str)
                    .map(|d| d.chars().take(400).collect())
                    .unwrap_or_default(),
                marketplace: text(p, "marketplaceName")
                    .or_else(|| id.split_once('@').map(|(_, m)| m.to_string()))
                    .unwrap_or_default(),
                install_count: p.get("installCount").and_then(Value::as_u64),
                id,
                name,
            })
        })
        .collect()
}

/// The available plugins matching `q` (name, id, marketplace or
/// description), most installed first, at most `limit`.
pub fn search(all: &[Available], q: &str, limit: usize) -> (Vec<Available>, usize) {
    let words: Vec<String> = q
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    let mut hits: Vec<(u8, &Available)> = all
        .iter()
        .filter_map(|p| {
            if words.is_empty() {
                return Some((1, p));
            }
            let name = p.name.to_lowercase();
            let rest = format!("{} {} {}", p.id, p.marketplace, p.description).to_lowercase();
            let all_match = words.iter().all(|w| name.contains(w) || rest.contains(w));
            all_match.then(|| (u8::from(words.iter().all(|w| name.contains(w))) + 1, p))
        })
        .collect();
    let total = hits.len();
    hits.sort_by(|(ra, a), (rb, b)| {
        rb.cmp(ra)
            .then(
                b.install_count
                    .unwrap_or(0)
                    .cmp(&a.install_count.unwrap_or(0)),
            )
            .then(a.name.cmp(&b.name))
    });
    (
        hits.into_iter()
            .take(limit)
            .map(|(_, p)| p.clone())
            .collect(),
        total,
    )
}

/// One account's catalog and when it was read (ms).
type Catalog = (i64, std::sync::Arc<Vec<Available>>);

/// The catalogs read lately, per account (`None`: the Default).
#[derive(Default)]
pub struct Catalogs {
    by_account: Mutex<HashMap<String, Catalog>>,
}

impl Catalogs {
    fn get(&self, key: &str, now: i64) -> Option<std::sync::Arc<Vec<Available>>> {
        let map = self.by_account.lock().unwrap_or_else(|e| e.into_inner());
        map.get(key)
            .filter(|(at, _)| now - *at < CATALOG_TTL_MS)
            .map(|(_, list)| list.clone())
    }

    fn put(&self, key: &str, now: i64, list: Vec<Available>) -> std::sync::Arc<Vec<Available>> {
        let list = std::sync::Arc::new(list);
        self.by_account
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), (now, list.clone()));
        list
    }

    /// A change of plugins or marketplaces: read the catalog again.
    pub fn forget(&self) {
        self.by_account
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

/// What is installed (seen from `project` when given) and the marketplaces.
pub async fn overview(
    state: &AppState,
    account: Option<&str>,
    project: Option<&Path>,
) -> ApiResult<Value> {
    let env = account_env(state, account)?;
    let list = run(
        &args(&["plugin", "list", "--json"]),
        &env,
        project,
        READ_TIMEOUT,
    )
    .await?;
    if !list.ok {
        return Err(failed(&list));
    }
    let mut installed = serde_json::from_str::<Value>(list.stdout.trim())
        .map(|v| parse_installed(&v))
        .map_err(|_| {
            ApiError::unprocessable(
                "plugin_cli_unreadable",
                "Claude Code's list of plugins could not be read.",
            )
        })?;
    // A project's own plugin is on or off for that project: known only
    // when the list was read there.
    for p in &mut installed {
        if let Some(theirs) = &p.project {
            p.enabled_known = project.is_some_and(|here| same_folder(here, Path::new(theirs)));
        }
    }
    let markets = run(
        &args(&["plugin", "marketplace", "list", "--json"]),
        &env,
        project,
        READ_TIMEOUT,
    )
    .await?;
    let marketplaces = if markets.ok {
        serde_json::from_str::<Value>(markets.stdout.trim())
            .map(|v| parse_marketplaces(&v))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(json!({
        "installed": installed,
        "marketplaces": marketplaces,
        "project": project.map(|p| p.to_string_lossy().into_owned()),
    }))
}

/// The catalog's plugins matching `q`, each marked when installed.
pub async fn available(state: &AppState, account: Option<&str>, q: &str) -> ApiResult<Value> {
    let key = account.unwrap_or("").to_string();
    let now = crate::claude::registry::now_ms();
    let catalog = match state.plugin_catalogs.get(&key, now) {
        Some(c) => c,
        None => {
            let env = account_env(state, account)?;
            let out = run(
                &args(&["plugin", "list", "--json", "--available"]),
                &env,
                None,
                CHANGE_TIMEOUT,
            )
            .await?;
            if !out.ok {
                return Err(failed(&out));
            }
            let v = serde_json::from_str::<Value>(out.stdout.trim()).map_err(|_| {
                ApiError::unprocessable(
                    "plugin_cli_unreadable",
                    "Claude Code's catalog of plugins could not be read.",
                )
            })?;
            state.plugin_catalogs.put(&key, now, parse_available(&v))
        }
    };
    let (hits, total) = search(&catalog, q, SEARCH_LIMIT);
    Ok(json!({ "plugins": hits, "total": total, "catalog": catalog.len() }))
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

// ─────────────────────────────── changes ───────────────────────────────

/// A change to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Install,
    Uninstall,
    Enable,
    Disable,
    Update,
}

impl Change {
    fn verb(&self) -> &'static str {
        match self {
            Change::Install => "install",
            Change::Uninstall => "uninstall",
            Change::Enable => "enable",
            Change::Disable => "disable",
            Change::Update => "update",
        }
    }

    /// It may run a command the marketplace declares (an install from a
    /// command, or a fetch through a helper): shown, then accepted.
    fn may_declare_command(&self) -> bool {
        matches!(self, Change::Install | Change::Update)
    }
}

/// The argv of a change (verified with `claude plugin <verb> --help` on CLI
/// 2.1.325).
pub fn change_argv(
    change: &Change,
    plugin: &str,
    scope: Scope,
    accept: Option<&str>,
) -> Vec<String> {
    let mut a = args(&[
        "plugin",
        change.verb(),
        plugin,
        "--scope",
        scope.as_str(),
        "--json",
    ]);
    if let Some(sha) = accept.filter(|_| change.may_declare_command()) {
        a.push("--accept-command".into());
        a.push(sha.to_string());
    }
    a
}

/// The command a marketplace declares, when Claude Code wants a person to
/// see it before it runs (`shownCommand` of a `--json` result).
pub fn shown_command(result: &Value) -> Option<(String, String)> {
    let shown = result
        .get("shownCommand")
        .or_else(|| result.get("shown_command"))?;
    let sha = shown.get("sha256").and_then(Value::as_str)?;
    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let command = ["command", "text", "display"]
        .iter()
        .find_map(|k| shown.get(*k).and_then(Value::as_str))
        .unwrap_or("")
        .chars()
        .take(4000)
        .collect();
    Some((command, sha.to_string()))
}

/// Installs, removes, turns on or off, or updates a plugin where `scope`
/// says (`project` for the project scopes). `accept`: the SHA-256 of the
/// marketplace-declared command the person saw and accepted.
pub async fn change(
    state: &AppState,
    change: Change,
    plugin: &str,
    scope: Scope,
    project: Option<&Path>,
    account: Option<&str>,
    accept: Option<&str>,
) -> ApiResult<Value> {
    if !valid_plugin_id(plugin) {
        return Err(ApiError::unprocessable(
            "invalid_plugin",
            "That is not a plugin's name.",
        ));
    }
    if plugin == crate::claude::hooks::PLUGIN_ID
        || plugin == crate::claude::hooks::PLUGIN
        || plugin.ends_with(&format!("@{}", crate::claude::hooks::MARKETPLACE))
    {
        return Err(ApiError::conflict(
            "plugin_is_ours",
            "Mother Claude's own plugin is turned on and off in Sessions started elsewhere.",
        ));
    }
    if scope.needs_project() != project.is_some() {
        return Err(ApiError::unprocessable(
            "invalid_project",
            if scope.needs_project() {
                "Choose the project this is for."
            } else {
                "A plugin for everywhere takes no project."
            },
        ));
    }
    if let Some(sha) = accept {
        if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ApiError::unprocessable(
                "invalid_command",
                "That is not the command that was shown.",
            ));
        }
    }
    let env = account_env(state, account)?;
    let out = run(
        &change_argv(&change, plugin, scope, accept),
        &env,
        project,
        CHANGE_TIMEOUT,
    )
    .await?;
    state.plugin_catalogs.forget();
    let result = out.result().unwrap_or(Value::Null);
    if !out.ok {
        if let Some((command, sha)) = shown_command(&result) {
            return Err(ApiError::conflict(
                "confirm_command",
                "The marketplace runs a command to install this plugin. Read it, and run it only \
                 if you trust it.",
            )
            .with("command", json!(command))
            .with("sha256", json!(sha)));
        }
        return Err(failed(&out));
    }
    Ok(json!({ "ok": true, "result": sanitize_result(&result) }))
}

/// A result line without anything that could be a secret or a long dump.
fn sanitize_result(v: &Value) -> Value {
    let Some(obj) = v.as_object() else {
        return Value::Null;
    };
    let mut out = serde_json::Map::new();
    for k in [
        "message",
        "plugin",
        "pluginId",
        "scope",
        "version",
        "restartRequired",
    ] {
        if let Some(x) = obj.get(k) {
            if x.is_string() || x.is_boolean() || x.is_number() {
                out.insert(k.to_string(), x.clone());
            }
        }
    }
    Value::Object(out)
}

/// Adds a marketplace (for this person, everywhere).
pub async fn add_marketplace(
    state: &AppState,
    source: &str,
    account: Option<&str>,
) -> ApiResult<Value> {
    let source = check_source(source)?;
    let env = account_env(state, account)?;
    let out = run(
        &[
            "plugin".into(),
            "marketplace".into(),
            "add".into(),
            source,
            "--scope".into(),
            "user".into(),
            "--json".into(),
        ],
        &env,
        None,
        CHANGE_TIMEOUT,
    )
    .await?;
    state.plugin_catalogs.forget();
    if !out.ok {
        return Err(failed(&out));
    }
    Ok(json!({ "ok": true, "result": sanitize_result(&out.result().unwrap_or(Value::Null)) }))
}

/// Removes a marketplace from every scope that declares it.
pub async fn remove_marketplace(
    state: &AppState,
    name: &str,
    account: Option<&str>,
) -> ApiResult<Value> {
    if !valid_marketplace_name(name) {
        return Err(ApiError::unprocessable(
            "invalid_marketplace",
            "That is not a marketplace's name.",
        ));
    }
    if name == crate::claude::hooks::MARKETPLACE {
        return Err(ApiError::conflict(
            "plugin_is_ours",
            "Mother Claude's own marketplace goes with its plugin, in Sessions started elsewhere.",
        ));
    }
    let env = account_env(state, account)?;
    let out = run(
        &args(&["plugin", "marketplace", "remove", name, "--json"]),
        &env,
        None,
        CHANGE_TIMEOUT,
    )
    .await?;
    state.plugin_catalogs.forget();
    if !out.ok {
        return Err(failed(&out));
    }
    Ok(json!({ "ok": true }))
}

#[cfg(test)]
mod tests;
