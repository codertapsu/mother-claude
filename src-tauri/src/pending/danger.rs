//! Server-side "is this approval dangerous?" classification (design §10, B3).
//!
//! One classifier for every channel — owned `can_use_tool`, hook
//! `PermissionRequest`, bridge host and sidecar items. A channel's own
//! `dangerous` flag can only *raise* the result, never lower it.
//!
//! An item is dangerous when approving it once has the same effect as a
//! standing grant or code execution outside the session:
//!
//! * an `Edit`/`Write`/`MultiEdit`/`NotebookEdit` whose target — resolved
//!   against the session's cwd, `~` expanded, `..` folded and symlinks followed
//!   — is Claude Code configuration (`**/.claude/**`, `.claude.json`,
//!   `.mcp.json`), Mother Claude's own data dir, `~/.ssh/**`, a shell startup
//!   file, a launch agent/daemon, or `.git/hooks/**` / `.git/config`;
//! * a `Bash` (or `PowerShell`) command that names any of those paths, runs
//!   `claude plugin|mcp|config`, or asks for permission bypass.
//!
//! Suggestions are classified separately ([`super::is_standing`]): choosing a
//! standing one is dangerous even on a harmless item.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::PendingItem;

/// Tools that write the file named in their input.
const WRITE_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// Tools whose `command` runs in a shell.
const SHELL_TOOLS: [&str; 2] = ["Bash", "PowerShell"];

/// Shell startup / profile files (matched by file name anywhere).
const SHELL_RC: [&str; 16] = [
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".profile",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".zlogout",
    ".kshrc",
    ".cshrc",
    ".tcshrc",
    ".login",
    ".inputrc",
    "config.fish",
];

/// Where the classifier looks things up. [`DangerEnv::current`] reads the
/// real home and app data dir; tests build their own.
#[derive(Debug, Clone, Default)]
pub struct DangerEnv {
    /// The session's working directory (relative paths resolve against it).
    pub cwd: Option<PathBuf>,
    /// The user's home directory (`~`).
    pub home: Option<PathBuf>,
    /// Mother Claude's own data directories.
    pub app_dirs: Vec<PathBuf>,
}

impl DangerEnv {
    /// The real environment for a session running in `cwd`.
    pub fn current(cwd: Option<&str>) -> Self {
        Self {
            cwd: cwd.filter(|c| !c.is_empty()).map(PathBuf::from),
            home: crate::claude::user_home_dir(),
            app_dirs: crate::persist::app_data_dir().into_iter().collect(),
        }
    }
}

/// Why approving `item` is dangerous, or `None`.
pub fn classify(item: &PendingItem, env: &DangerEnv) -> Option<String> {
    let tool = item.tool_name.as_deref().unwrap_or_default();
    let input = item.input.as_ref();
    let text = |key: &str| input.and_then(|i| i.get(key)).and_then(Value::as_str);
    if WRITE_TOOLS.contains(&tool) {
        for key in ["file_path", "notebook_path", "path"] {
            if let Some(why) = text(key).and_then(|p| sensitive_path(p, env)) {
                return Some(format!("{tool} changes {why}."));
            }
        }
    }
    if SHELL_TOOLS.contains(&tool) {
        if let Some(why) = text("command").and_then(|c| sensitive_command(c, env)) {
            return Some(format!("The command {why}."));
        }
    }
    None
}

/// Why writing `raw` is dangerous, or `None`. Both the lexical path and the
/// symlink-resolved one are checked.
pub fn sensitive_path(raw: &str, env: &DangerEnv) -> Option<String> {
    let lexical = lexical_path(raw, env);
    let resolved = resolve_existing(&lexical);
    sensitive_resolved(&lexical, env).or_else(|| sensitive_resolved(&resolved, env))
}

fn sensitive_resolved(path: &Path, env: &DangerEnv) -> Option<String> {
    let comps: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let name = comps.last().map(String::as_str).unwrap_or_default();
    let has_pair = |a: &str, b: &str| comps.windows(2).any(|w| w[0] == a && w[1] == b);

    for dir in &env.app_dirs {
        if path.starts_with(dir) || path.starts_with(resolve_existing(dir)) {
            return Some("Mother Claude's own data (devices, hook credentials)".into());
        }
    }
    if comps.iter().any(|c| c == ".claude") {
        return Some("Claude Code configuration (.claude/)".into());
    }
    if name == ".claude.json" {
        return Some("Claude Code's global config (~/.claude.json)".into());
    }
    if name == ".mcp.json" {
        return Some("an MCP server configuration (.mcp.json)".into());
    }
    if comps.iter().any(|c| c == ".ssh") {
        return Some("SSH configuration (~/.ssh)".into());
    }
    if SHELL_RC.contains(&name) {
        return Some(format!("a shell startup file ({name})"));
    }
    if has_pair("Library", "LaunchAgents") || comps.iter().any(|c| c == "LaunchDaemons") {
        return Some("a launch agent (runs at login)".into());
    }
    if has_pair(".git", "hooks") {
        return Some("a git hook (runs on every git command)".into());
    }
    if comps.len() >= 2 && comps[comps.len() - 2] == ".git" && name == "config" {
        return Some("the repository's git config".into());
    }
    None
}

/// Why running `command` is dangerous, or `None`.
pub fn sensitive_command(command: &str, env: &DangerEnv) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    let mentions = |needle: &str| lower.contains(needle);

    if mentions("bypasspermissions") || mentions("dangerously-skip-permissions") {
        return Some("asks for permission bypass".into());
    }
    if runs_claude_admin(&lower) {
        return Some("changes Claude Code's plugins, MCP servers or config".into());
    }
    for dir in &env.app_dirs {
        let d = dir.to_string_lossy().to_ascii_lowercase();
        if !d.is_empty() && lower.contains(&d) {
            return Some("touches Mother Claude's own data".into());
        }
    }
    if mentions("dev.motherclaude") {
        return Some("touches Mother Claude's own data".into());
    }
    const PATTERNS: [(&str, &str); 12] = [
        (".claude/", "touches Claude Code configuration (.claude/)"),
        (".claude.json", "touches ~/.claude.json"),
        (".mcp.json", "touches an MCP server configuration"),
        (".ssh", "touches SSH configuration"),
        ("launchagents", "touches launch agents"),
        ("launchdaemons", "touches launch daemons"),
        (".git/hooks", "touches git hooks"),
        (".git/config", "touches the repository's git config"),
        ("core.hookspath", "changes where git hooks run from"),
        ("core.fsmonitor", "makes git run a program"),
        (".claude ", "touches Claude Code configuration (.claude/)"),
        ("/.claude\"", "touches Claude Code configuration (.claude/)"),
    ];
    for (needle, why) in PATTERNS {
        if mentions(needle) {
            return Some(why.into());
        }
    }
    if lower.ends_with("/.claude") || lower.ends_with("~/.claude") {
        return Some("touches Claude Code configuration (.claude/)".into());
    }
    for rc in SHELL_RC {
        if contains_word(&lower, rc) {
            return Some(format!("touches a shell startup file ({rc})"));
        }
    }
    None
}

/// `claude plugin …`, `claude mcp …`, `claude config …` anywhere in the line.
fn runs_claude_admin(lower: &str) -> bool {
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("claude") {
        let start = from + pos;
        let end = start + "claude".len();
        from = end;
        let boundary_before = start == 0
            || !(bytes[start - 1].is_ascii_alphanumeric()
                || bytes[start - 1] == b'-'
                || bytes[start - 1] == b'_'
                || bytes[start - 1] == b'.');
        if !boundary_before {
            continue;
        }
        let rest = lower[end..].trim_start_matches([' ', '\t']);
        if rest.len() == lower.len() - end {
            continue; // no whitespace after `claude`
        }
        for sub in ["plugin", "plugins", "mcp", "config"] {
            if let Some(after) = rest.strip_prefix(sub) {
                if after.is_empty() || after.starts_with(|c: char| !c.is_ascii_alphanumeric()) {
                    return true;
                }
            }
        }
    }
    false
}

/// `needle` as a path component: preceded by `/`, `~`, whitespace, a quote or
/// the start, and followed by a non-name character or the end.
fn contains_word(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        from = start + 1;
        let before_ok = start == 0
            || matches!(
                hay.as_bytes()[start - 1],
                b'/' | b'~' | b' ' | b'\t' | b'"' | b'\'' | b'>' | b'='
            );
        let after_ok = end == hay.len()
            || !matches!(hay.as_bytes()[end], b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.');
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// `~` expanded, made absolute against cwd when possible, `.`/`..` folded.
fn lexical_path(raw: &str, env: &DangerEnv) -> PathBuf {
    let raw = raw.trim();
    let expanded = match (raw.strip_prefix("~/"), &env.home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if raw == "~" => env.home.clone().unwrap_or_else(|| PathBuf::from(raw)),
        _ => PathBuf::from(raw),
    };
    let joined = if expanded.is_absolute() {
        expanded
    } else {
        match &env.cwd {
            Some(cwd) => cwd.join(expanded),
            None => expanded,
        }
    };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalize the longest existing ancestor (following symlinks) and
/// re-append the rest, so a file that does not exist yet still resolves
/// through a symlinked directory.
fn resolve_existing(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = std::fs::canonicalize(&existing) {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (
            existing.file_name().map(|n| n.to_os_string()),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) if !parent.as_os_str().is_empty() => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::{PendingKind, PendingSource};
    use serde_json::json;

    fn env(tmp: &Path) -> DangerEnv {
        DangerEnv {
            cwd: Some(tmp.join("repo")),
            home: Some(tmp.join("home")),
            app_dirs: vec![
                tmp.join("home/Library/Application Support/dev.motherclaude.Mother-Claude")
            ],
        }
    }

    fn item(tool: &str, input: Value) -> PendingItem {
        PendingItem {
            session_id: "s".into(),
            kind: PendingKind::Permission,
            source: PendingSource::Owned,
            tool_name: Some(tool.into()),
            input: Some(input),
            ..Default::default()
        }
    }

    #[test]
    fn classifier_table() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(tmp.join("repo/src")).unwrap();
        std::fs::create_dir_all(tmp.join("home/.claude")).unwrap();
        std::fs::create_dir_all(tmp.join("elsewhere")).unwrap();
        // A symlink inside the repo pointing at ~/.claude.
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.join("home/.claude"), tmp.join("repo/cfg")).unwrap();
        let e = env(&tmp);

        let cases: Vec<(&str, Value, bool)> = vec![
            // Ordinary edits are fine.
            ("Edit", json!({ "file_path": "src/main.rs" }), false),
            ("Write", json!({ "file_path": "README.md" }), false),
            (
                "Write",
                json!({ "file_path": "../elsewhere/notes.txt" }),
                false,
            ),
            ("Read", json!({ "file_path": "~/.ssh/id_ed25519" }), false),
            // Claude Code configuration.
            (
                "Write",
                json!({ "file_path": ".claude/settings.local.json" }),
                true,
            ),
            (
                "Edit",
                json!({ "file_path": "~/.claude/settings.json" }),
                true,
            ),
            ("Write", json!({ "file_path": "~/.claude.json" }), true),
            ("Write", json!({ "file_path": ".mcp.json" }), true),
            ("Write", json!({ "file_path": "src/../.mcp.json" }), true),
            (
                "MultiEdit",
                json!({ "file_path": "sub/.claude/agents/x.md" }),
                true,
            ),
            // Through a symlinked directory, file not created yet.
            ("Write", json!({ "file_path": "cfg/settings.json" }), true),
            // Mother Claude's data dir.
            (
                "Write",
                json!({ "file_path": "~/Library/Application Support/dev.motherclaude.Mother-Claude/devices.json" }),
                true,
            ),
            // SSH, shell rc, launch agents, git hooks/config.
            (
                "Write",
                json!({ "file_path": "~/.ssh/authorized_keys" }),
                true,
            ),
            ("Edit", json!({ "file_path": "~/.zshrc" }), true),
            (
                "Write",
                json!({ "file_path": "~/Library/LaunchAgents/x.plist" }),
                true,
            ),
            (
                "Write",
                json!({ "file_path": ".git/hooks/pre-commit" }),
                true,
            ),
            ("Edit", json!({ "file_path": ".git/config" }), true),
            (
                "NotebookEdit",
                json!({ "notebook_path": "~/.claude/x.ipynb" }),
                true,
            ),
            // Bash.
            ("Bash", json!({ "command": "npm test" }), false),
            ("Bash", json!({ "command": "git status && ls -la" }), false),
            (
                "Bash",
                json!({ "command": "echo 'alias x=y' >> ~/.zshrc" }),
                true,
            ),
            (
                "Bash",
                json!({ "command": "cat ~/.claude/settings.json" }),
                true,
            ),
            ("Bash", json!({ "command": "cp evil.json .mcp.json" }), true),
            (
                "Bash",
                json!({ "command": "claude plugin install x@y" }),
                true,
            ),
            (
                "Bash",
                json!({ "command": "claude  mcp add foo -- npx foo" }),
                true,
            ),
            (
                "Bash",
                json!({ "command": "npx claude config set x" }),
                true,
            ),
            (
                "Bash",
                json!({ "command": "claude --dangerously-skip-permissions" }),
                true,
            ),
            (
                "Bash",
                json!({ "command": "chmod +x .git/hooks/post-checkout" }),
                true,
            ),
            (
                "Bash",
                json!({ "command": "git config core.hooksPath /tmp/h" }),
                true,
            ),
            (
                "Bash",
                json!({ "command": "ls ~/Library/LaunchAgents" }),
                true,
            ),
            ("Bash", json!({ "command": "echo claude-mcp-docs" }), false),
            ("Bash", json!({ "command": "grep profile src/*.rs" }), false),
        ];
        for (tool, input, want) in cases {
            let got = classify(&item(tool, input.clone()), &e);
            assert_eq!(got.is_some(), want, "{tool} {input} → {got:?}");
        }
    }

    #[test]
    fn words_and_admin_commands() {
        assert!(runs_claude_admin("claude plugin list"));
        assert!(runs_claude_admin("cd x && claude mcp"));
        assert!(!runs_claude_admin("claude -p hello"));
        assert!(!runs_claude_admin("my-claude plugin"));
        assert!(!runs_claude_admin("claudeplugin"));
        assert!(contains_word("cat ~/.zshrc", ".zshrc"));
        assert!(!contains_word("cat .zshrc.bak", ".zshrc"));
    }
}
