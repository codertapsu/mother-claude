//! Customizations (docs/ACCOUNTS.md §13.1, §13.3 rule 7): what makes an
//! account one's own — `CLAUDE.md`, `settings.json`, skills, agents, commands,
//! output styles, keybindings, the user-scope MCP servers and the prompt
//! history — copied between accounts in different directories, never moved.
//!
//! Each category adds what the target lacks; with `replaceExisting` what it
//! has is replaced, the old copy renamed into the undo directory first. MCP
//! servers merge by name (only their names are ever served), the prompt
//! history by `(timestamp, display)`. A symbolic link (a skill linked from a
//! repository) is made again as a link to the same absolute target, never
//! followed. Plugins are never copied: they are installed per account.
//!
//! A new file is placed with a hard link from a temp file beside it, so it
//! never lands over one that appeared meanwhile; adding only new files runs
//! while the target has live sessions, while a replacement, the MCP servers
//! and the prompt history wait until nothing uses the target (D4).

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    history_key, io, is_plain_dir, missing_dirs, remove_empty_dirs, sha256_file, stash, Created,
    Manifest, Progress, Skipped, TransferError,
};
use crate::claude::accounts::ACCOUNT_OVERRIDE_ENV;
use crate::claude::home::ClaudeHome;

/// A category of customizations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Custom {
    /// `CLAUDE.md`.
    Memory,
    /// `settings.json`.
    Settings,
    /// `skills/` (each skill).
    Skills,
    /// `agents/`.
    Agents,
    /// `commands/`.
    Commands,
    /// `output-styles/`.
    OutputStyles,
    /// `keybindings.json`.
    Keybindings,
    /// The user-scope `mcpServers` of `.claude.json`.
    McpServers,
    /// `history.jsonl`.
    History,
}

impl Custom {
    /// The wire name (`outputStyles`, `mcpServers`, …).
    pub fn key(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Settings => "settings",
            Self::Skills => "skills",
            Self::Agents => "agents",
            Self::Commands => "commands",
            Self::OutputStyles => "outputStyles",
            Self::Keybindings => "keybindings",
            Self::McpServers => "mcpServers",
            Self::History => "history",
        }
    }

    /// Copying it needs the dangerous right (§13.9): settings choose how
    /// Claude signs in and what runs; MCP servers start programs.
    pub fn dangerous(self) -> bool {
        matches!(self, Self::Settings | Self::McpServers)
    }

    /// One file at the top of the config directory.
    fn file(self) -> Option<&'static str> {
        match self {
            Self::Memory => Some("CLAUDE.md"),
            Self::Settings => Some("settings.json"),
            Self::Keybindings => Some("keybindings.json"),
            _ => None,
        }
    }

    /// A folder whose entries are copied one by one.
    fn folder(self) -> Option<&'static str> {
        match self {
            Self::Skills => Some("skills"),
            Self::Agents => Some("agents"),
            Self::Commands => Some("commands"),
            Self::OutputStyles => Some("output-styles"),
            _ => None,
        }
    }
}

/// What a plan does with one customization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CustomAction {
    Add,
    Replace,
    Skip,
    Merge,
}

/// One customization in a plan (and in a job's result): named only by its
/// file, folder entry or MCP server name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomItem {
    pub item: Custom,
    pub name: String,
    pub action: CustomAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A file, folder or link to place in the target.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Place {
    from: PathBuf,
    to: PathBuf,
    /// What stands there now goes to the undo directory first.
    replace: bool,
}

/// An MCP server to write into the target's `.claude.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpChange {
    pub name: String,
    /// What it is set to.
    pub value: Value,
    /// What the target had (`None`: it had none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Value>,
}

/// The customizations of a plan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomPlan {
    pub items: Vec<CustomItem>,
    pub warnings: Vec<String>,
    /// A category the source does not have.
    pub skipped: Vec<Skipped>,
    /// Prompt-history lines the target lacks (the caller appends them).
    pub history: Vec<String>,
    pub files: usize,
    pub bytes: u64,
    places: Vec<Place>,
    mcp: Vec<McpChange>,
}

impl CustomPlan {
    /// Whether it writes what Claude Code itself uses and rewrites: a
    /// replaced file, the MCP servers, the prompt history (D4: only while
    /// nothing uses the target).
    pub fn writes_shared(&self) -> bool {
        self.places.iter().any(|p| p.replace) || !self.mcp.is_empty() || !self.history.is_empty()
    }

    /// Whether running it would change anything.
    pub fn is_empty(&self) -> bool {
        self.places.is_empty() && self.mcp.is_empty() && self.history.is_empty()
    }
}

/// Every file and link under `path` (or `path` itself), never through a
/// link, with the folders on the way.
fn entries_under(path: &Path) -> std::io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    fn visit(dir: &Path, files: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) -> std::io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let ft = e.file_type()?;
            if ft.is_dir() {
                dirs.push(e.path());
                visit(&e.path(), files, dirs)?;
            } else if ft.is_file() || ft.is_symlink() {
                files.push(e.path());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    if is_plain_dir(path) {
        dirs.push(path.to_path_buf());
        visit(path, &mut files, &mut dirs)?;
    } else {
        files.push(path.to_path_buf());
    }
    Ok((files, dirs))
}

/// Size of a file, or of everything under a folder (links count nothing).
fn size_of(path: &Path) -> (usize, u64) {
    let Ok((files, _)) = entries_under(path) else {
        return (0, 0);
    };
    let bytes = files
        .iter()
        .filter_map(|f| std::fs::symlink_metadata(f).ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum();
    (files.len(), bytes)
}

/// Whether two files hold the same bytes.
fn same_file(a: &Path, b: &Path) -> bool {
    match (sha256_file(a), sha256_file(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

/// Plan copying `what` from one config directory to another (its `.claude.json`
/// through [`ClaudeHome::claude_json`]: the Default's is `~/.claude.json`).
/// Nothing is touched.
pub fn plan(
    from: &ClaudeHome,
    to: &ClaudeHome,
    what: &[Custom],
    replace: bool,
) -> Result<CustomPlan, TransferError> {
    let mut out = CustomPlan::default();
    let mut what: Vec<Custom> = what.to_vec();
    what.sort();
    what.dedup();
    for item in what {
        if let Some(name) = item.file() {
            plan_entry(
                &mut out,
                item,
                name,
                &from.base().join(name),
                &to.base().join(name),
                replace,
            );
            if item == Custom::Settings
                && out
                    .items
                    .last()
                    .is_some_and(|i| matches!(i.action, CustomAction::Add | CustomAction::Replace))
            {
                out.warnings
                    .extend(settings_warnings(&from.base().join(name)));
            }
        } else if let Some(folder) = item.folder() {
            let src = from.base().join(folder);
            let names: Vec<String> = std::fs::read_dir(&src)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                // Hidden entries (`.DS_Store`, a `.trash`) and synced skills
                // (runtime state, D6) are not the user's own.
                .filter(|n| !n.starts_with('.') && (item != Custom::Skills || n != "synced"))
                .collect();
            let mut names = names;
            names.sort();
            if names.is_empty() {
                out.skipped.push(Skipped::item(
                    item.key(),
                    "not_found",
                    format!("there is nothing in {folder}/ to copy"),
                ));
                continue;
            }
            for name in names {
                plan_entry(
                    &mut out,
                    item,
                    &name,
                    &src.join(&name),
                    &to.base().join(folder).join(&name),
                    replace,
                );
            }
        } else if item == Custom::McpServers {
            plan_mcp(&mut out, &from.claude_json(), &to.claude_json(), replace)?;
        } else {
            plan_history(&mut out, &from.history_file(), &to.history_file())?;
        }
    }
    Ok(out)
}

fn plan_entry(
    out: &mut CustomPlan,
    item: Custom,
    name: &str,
    from: &Path,
    to: &Path,
    replace: bool,
) {
    let push = |out: &mut CustomPlan, action, reason: Option<&str>| {
        out.items.push(CustomItem {
            item,
            name: name.to_string(),
            action,
            reason: reason.map(str::to_string),
        });
    };
    if !exists(from) {
        out.skipped.push(Skipped::item(
            item.key(),
            "not_found",
            format!("there is no {name} to copy"),
        ));
        return;
    }
    let replacing = exists(to);
    if replacing && !is_plain_dir(from) && !is_plain_dir(to) && same_file(from, to) {
        return push(out, CustomAction::Skip, Some("the same already"));
    }
    if replacing && !replace {
        return push(out, CustomAction::Skip, Some("the target has its own"));
    }
    let (files, bytes) = size_of(from);
    out.files += files;
    out.bytes += bytes;
    out.places.push(Place {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        replace: replacing,
    });
    push(
        out,
        if replacing {
            CustomAction::Replace
        } else {
            CustomAction::Add
        },
        None,
    );
}

/// The `mcpServers` object of a `.claude.json` (empty when there is none).
fn mcp_servers(claude_json: &Path) -> Result<serde_json::Map<String, Value>, TransferError> {
    let text = match std::fs::read_to_string(claude_json) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(io("reading", claude_json, e)),
    };
    let v: Value = serde_json::from_str(&text).map_err(|e| {
        TransferError::Io(format!(
            "{} is not readable JSON: {e}",
            claude_json.display()
        ))
    })?;
    Ok(v.get("mcpServers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default())
}

fn plan_mcp(
    out: &mut CustomPlan,
    from: &Path,
    to: &Path,
    replace: bool,
) -> Result<(), TransferError> {
    let theirs = mcp_servers(from)?;
    if theirs.is_empty() {
        out.skipped.push(Skipped::item(
            Custom::McpServers.key(),
            "not_found",
            "there are no MCP servers to copy",
        ));
        return Ok(());
    }
    let ours = mcp_servers(to)?;
    let mut copied = Vec::new();
    for (name, value) in theirs {
        let (action, reason) = match ours.get(&name) {
            Some(v) if *v == value => (CustomAction::Skip, Some("the same already")),
            Some(_) if !replace => (CustomAction::Skip, Some("the target has its own")),
            Some(v) => {
                out.mcp.push(McpChange {
                    name: name.clone(),
                    value: value.clone(),
                    previous: Some(v.clone()),
                });
                (CustomAction::Replace, None)
            }
            None => {
                out.mcp.push(McpChange {
                    name: name.clone(),
                    value: value.clone(),
                    previous: None,
                });
                (CustomAction::Add, None)
            }
        };
        if matches!(action, CustomAction::Add | CustomAction::Replace) {
            copied.push(name.clone());
        }
        out.items.push(CustomItem {
            item: Custom::McpServers,
            name,
            action,
            reason: reason.map(str::to_string),
        });
    }
    if !copied.is_empty() {
        out.warnings.push(format!(
            "The copied MCP servers ({}) start with every session of the target account.",
            copied.join(", ")
        ));
    }
    Ok(())
}

fn plan_history(out: &mut CustomPlan, from: &Path, to: &Path) -> Result<(), TransferError> {
    let read = |p: &Path| -> Result<Vec<String>, TransferError> {
        match std::fs::read(p) {
            Ok(bytes) => Ok(bytes
                .split(|b| *b == b'\n')
                .filter_map(|l| std::str::from_utf8(l).ok())
                .map(|l| l.trim_end_matches('\r').to_string())
                .filter(|l| !l.is_empty())
                .collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(io("reading", p, e)),
        }
    };
    let theirs = read(from)?;
    if theirs.is_empty() {
        out.skipped.push(Skipped::item(
            Custom::History.key(),
            "not_found",
            "there is no prompt history to copy",
        ));
        return Ok(());
    }
    let mut have: HashSet<(String, String)> =
        read(to)?.iter().filter_map(|l| history_key(l)).collect();
    let lines: Vec<String> = theirs
        .into_iter()
        .filter(|l| history_key(l).is_some_and(|k| have.insert(k)))
        .collect();
    out.items.push(CustomItem {
        item: Custom::History,
        name: "history.jsonl".into(),
        action: if lines.is_empty() {
            CustomAction::Skip
        } else {
            CustomAction::Merge
        },
        reason: if lines.is_empty() {
            Some("the target has every line".into())
        } else {
            Some(format!("{} line(s)", lines.len()))
        },
    });
    out.history = lines;
    Ok(())
}

/// What copied settings would change about the target account: how it signs
/// in, what it runs, the plugins it expects.
fn settings_warnings(path: &Path) -> Vec<String> {
    let Some(v) = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return vec!["The copied settings.json is not valid JSON.".into()];
    };
    let mut out = Vec::new();
    let mut signin: Vec<String> = v
        .get("env")
        .and_then(Value::as_object)
        .map(|env| {
            ACCOUNT_OVERRIDE_ENV
                .iter()
                .filter(|k| env.contains_key(**k))
                .map(|k| format!("env {k}"))
                .collect()
        })
        .unwrap_or_default();
    signin.extend(
        ["apiKeyHelper", "forceLoginMethod", "forceLoginOrgUUID"]
            .iter()
            .filter(|k| v.get(**k).is_some_and(|x| !x.is_null()))
            .map(|k| k.to_string()),
    );
    if !signin.is_empty() {
        out.push(format!(
            "The copied settings.json chooses how Claude signs in ({}): the target account \
             would sign in that way rather than with its own sign-in.",
            signin.join(", ")
        ));
    }
    let runs: Vec<&str> = ["hooks", "statusLine"]
        .into_iter()
        .filter(|k| v.get(*k).is_some_and(|x| !x.is_null()))
        .collect();
    if !runs.is_empty() {
        out.push(format!(
            "The copied settings.json runs commands ({}) in every session of the target account.",
            runs.join(", ")
        ));
    }
    if let Some(plugins) = v.get("enabledPlugins").and_then(Value::as_object) {
        if !plugins.is_empty() {
            let names: Vec<&str> = plugins.keys().map(String::as_str).collect();
            out.push(format!(
                "The copied settings.json enables plugins ({}); plugins are never copied: \
                 install them in the target account.",
                names.join(", ")
            ));
        }
    }
    out
}

/// A link's target made absolute against the folder the link is in,
/// lexically (nothing is followed).
fn absolute_target(link: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        return target.to_path_buf();
    }
    let joined = link.parent().unwrap_or(Path::new("/")).join(target);
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                if out.parent().is_some() {
                    out.pop();
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Place one file without ever landing over another: a temp file beside it,
/// fsynced, with the source's time, hard-linked into place (which fails when
/// something is there), verified.
fn place_file(from: &Path, to: &Path) -> Result<Created, TransferError> {
    let dir = to
        .parent()
        .ok_or_else(|| TransferError::Invalid(format!("{} has no parent", to.display())))?;
    let name = to.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.mc-{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<(), TransferError> {
        std::fs::copy(from, &tmp).map_err(|e| io("copying", from, e))?;
        let f = std::fs::File::options()
            .write(true)
            .open(&tmp)
            .map_err(|e| io("opening", &tmp, e))?;
        if let Ok(mtime) = std::fs::metadata(from).and_then(|m| m.modified()) {
            let _ = f.set_modified(mtime);
        }
        f.sync_all().map_err(|e| io("syncing", &tmp, e))?;
        std::fs::hard_link(&tmp, to).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                TransferError::AlreadyExists(to.display().to_string())
            } else {
                io("placing", to, e)
            }
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result?;
    let (size, sha) = sha256_file(to).map_err(|e| io("verifying", to, e))?;
    let (src_size, src_sha) = sha256_file(from).map_err(|e| io("verifying", from, e))?;
    if size != src_size || sha != src_sha {
        let _ = std::fs::remove_file(to);
        return Err(TransferError::VerifyFailed(to.display().to_string()));
    }
    Ok(Created {
        path: to.to_path_buf(),
        size,
        sha256: sha,
        link: None,
    })
}

/// Make the target's folders down to `dir`, recording each one made.
fn make_dirs(dir: &Path, base: &Path, m: &mut Manifest) -> Result<(), TransferError> {
    for d in missing_dirs(dir, base) {
        std::fs::create_dir(&d).map_err(|e| io("creating", &d, e))?;
        m.created_dirs.push(d);
    }
    Ok(())
}

/// Copy a planned entry (a file, a folder, a link) into place, recording
/// what it made.
fn place(
    p: &Place,
    base: &Path,
    m: &mut Manifest,
    progress: &Progress,
) -> Result<(), TransferError> {
    let (files, dirs) = entries_under(&p.from).map_err(|e| io("listing", &p.from, e))?;
    let map = |src: &Path| -> PathBuf {
        match src.strip_prefix(&p.from) {
            Ok(rel) if !rel.as_os_str().is_empty() => p.to.join(rel),
            _ => p.to.clone(),
        }
    };
    if let Some(parent) = p.to.parent() {
        make_dirs(parent, base, m)?;
    }
    for d in &dirs {
        make_dirs(&map(d), base, m)?;
    }
    for f in &files {
        let dest = map(f);
        if let Some(parent) = dest.parent() {
            make_dirs(parent, base, m)?;
        }
        let meta = std::fs::symlink_metadata(f).map_err(|e| io("reading", f, e))?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(f).map_err(|e| io("reading", f, e))?;
            let target = absolute_target(f, &target);
            super::make_symlink(&target, &dest).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    TransferError::AlreadyExists(dest.display().to_string())
                } else {
                    io("linking", &dest, e)
                }
            })?;
            m.created.push(Created {
                path: dest,
                size: 0,
                sha256: String::new(),
                link: Some(target),
            });
        } else {
            let created = place_file(f, &dest)?;
            progress.files_done.fetch_add(1, Ordering::Relaxed);
            progress
                .bytes_done
                .fetch_add(created.size, Ordering::Relaxed);
            m.created.push(created);
        }
    }
    Ok(())
}

/// Rename what stands at `path` (a file, a link, a folder and everything
/// under it) into the undo directory, recording it; the folders it emptied
/// are recorded too (undo makes them again).
fn stash_existing(
    path: &Path,
    base: &Path,
    undo_dir: &Path,
    m: &mut Manifest,
) -> Result<(), TransferError> {
    let (files, dirs) = entries_under(path).map_err(|e| io("listing", path, e))?;
    for f in &files {
        let rel = Path::new("target").join(f.strip_prefix(base).unwrap_or(f));
        m.moved.push(stash(f, undo_dir, &rel)?);
    }
    for d in dirs.iter().rev() {
        remove_empty_dirs(d);
        if !d.exists() {
            m.moved_dirs.push(d.clone());
        }
    }
    Ok(())
}

/// Run a customization plan into `to`, after the job's conversations:
/// replaced entries go to the undo directory first, then the new ones are
/// placed; then the MCP servers (only where the target still has what the
/// plan saw). `save` keeps the manifest current; the prompt history is the
/// caller's. Blocking.
pub fn run(
    plan: &CustomPlan,
    to: &ClaudeHome,
    to_config: &Path,
    undo_dir: &Path,
    m: &mut Manifest,
    progress: &Progress,
    save: &dyn Fn(&Manifest) -> Result<(), TransferError>,
) -> Result<(), TransferError> {
    let base = to.base();
    for p in &plan.places {
        if p.replace && exists(&p.to) {
            stash_existing(&p.to, base, undo_dir, m)?;
            save(m)?;
        }
        place(p, base, m, progress)?;
        save(m)?;
    }
    if !plan.mcp.is_empty() {
        super::keep_shared(to_config, undo_dir, "to-claude.json")?;
        let mut done = Vec::new();
        super::edit_claude_json(to_config, |root| {
            let servers = root
                .entry("mcpServers")
                .or_insert_with(|| Value::Object(Default::default()));
            let servers = servers.as_object_mut().ok_or_else(|| {
                TransferError::Io(format!("{} has no mcpServers object", to_config.display()))
            })?;
            for c in &plan.mcp {
                if servers.get(&c.name) == c.previous.as_ref() {
                    servers.insert(c.name.clone(), c.value.clone());
                    done.push(c.clone());
                }
            }
            Ok(())
        })?;
        m.mcp = done;
        save(m)?;
    }
    Ok(())
}

/// Undo the MCP servers a job wrote: each back to what the target had, only
/// while it is still what the job set.
pub fn undo_mcp(m: &Manifest) -> Result<(), TransferError> {
    if m.mcp.is_empty() {
        return Ok(());
    }
    let path = m.to_config();
    super::edit_claude_json(&path, |root| {
        if let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) {
            for c in &m.mcp {
                if servers.get(&c.name) == Some(&c.value) {
                    match &c.previous {
                        Some(v) => {
                            servers.insert(c.name.clone(), v.clone());
                        }
                        None => {
                            servers.remove(&c.name);
                        }
                    }
                }
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_link_target_is_made_absolute_lexically() {
        assert_eq!(
            absolute_target(
                Path::new("/Users/me/.claude/skills/find-skills"),
                Path::new("../../.agents/skills/find-skills")
            ),
            PathBuf::from("/Users/me/.agents/skills/find-skills")
        );
        assert_eq!(
            absolute_target(Path::new("/a/b/link"), Path::new("/abs/x")),
            PathBuf::from("/abs/x")
        );
        assert_eq!(
            absolute_target(Path::new("/a/b/link"), Path::new("./c/../d")),
            PathBuf::from("/a/b/d")
        );
    }

    #[test]
    fn settings_that_choose_the_sign_in_or_run_commands_are_named() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("settings.json");
        std::fs::write(
            &p,
            serde_json::json!({
                "env": {"ANTHROPIC_API_KEY": "secret-value", "FOO": "1"},
                "apiKeyHelper": "/bin/key",
                "hooks": {"Stop": []},
                "enabledPlugins": {"mother-claude@local": true}
            })
            .to_string(),
        )
        .unwrap();
        let w = settings_warnings(&p);
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w[0].contains("env ANTHROPIC_API_KEY") && w[0].contains("apiKeyHelper"));
        assert!(!w.concat().contains("secret-value"), "names, never values");
        assert!(w[1].contains("hooks"));
        assert!(w[2].contains("mother-claude@local"));
        std::fs::write(&p, "{\"model\": \"opus\"}").unwrap();
        assert!(settings_warnings(&p).is_empty());
    }
}
