//! Resolution of the `~/.claude` (or `$CLAUDE_CONFIG_DIR`) layout and the
//! project-path ↔ transcript-dir encoding.
//!
//! All knowledge of where Claude Code keeps its files lives here so version
//! drift is contained to one module.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

/// Handle to a resolved Claude config directory.
#[derive(Debug, Clone)]
pub struct ClaudeHome {
    base: PathBuf,
}

/// The current user's home directory: `$HOME` (Unix) or, when that is unset,
/// `%USERPROFILE%` (Windows). Returns `None` if neither is set.
///
/// Claude Code keeps its config under the home dir on every platform, but
/// Windows does not normally export `HOME`, so the `USERPROFILE` fallback is
/// what makes the packaged Windows build find `~/.claude` at runtime.
pub fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))
        .map(PathBuf::from)
}

impl ClaudeHome {
    /// Resolve the base dir from `CLAUDE_CONFIG_DIR`, falling back to
    /// `<home>/.claude`. Does not require the directory to exist.
    pub fn resolve() -> Result<Self> {
        if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            if !dir.trim().is_empty() {
                return Ok(Self {
                    base: PathBuf::from(dir),
                });
            }
        }
        let home = user_home_dir()
            .ok_or_else(|| anyhow!("none of CLAUDE_CONFIG_DIR, HOME, or USERPROFILE is set"))?;
        Ok(Self {
            base: home.join(".claude"),
        })
    }

    /// Construct a handle for an explicit base dir (used in tests).
    pub fn with_base(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.base.join("projects")
    }

    pub fn jobs_dir(&self) -> PathBuf {
        self.base.join("jobs")
    }

    pub fn daemon_dir(&self) -> PathBuf {
        self.base.join("daemon")
    }

    pub fn roster_path(&self) -> PathBuf {
        self.daemon_dir().join("roster.json")
    }

    pub fn daemon_log(&self) -> PathBuf {
        self.base.join("daemon.log")
    }

    pub fn history_file(&self) -> PathBuf {
        self.base.join("history.jsonl")
    }

    pub fn user_settings(&self) -> PathBuf {
        self.base.join("settings.json")
    }

    /// `jobs/<id>/state.json` (may not exist).
    pub fn job_state_path(&self, session_id: &str) -> PathBuf {
        self.jobs_dir().join(session_id).join("state.json")
    }

    /// Directory holding a project's transcripts: `projects/<encoded-cwd>/`.
    pub fn transcript_dir(&self, cwd: &str) -> PathBuf {
        self.projects_dir().join(encode_cwd(cwd))
    }

    /// Full transcript path for a session: `projects/<encoded-cwd>/<id>.jsonl`.
    pub fn transcript_path(&self, cwd: &str, session_id: &str) -> PathBuf {
        self.transcript_dir(cwd).join(format!("{session_id}.jsonl"))
    }

    /// True if `projects/` is readable. On packaged macOS builds a `false` here
    /// usually means Full Disk Access has not been granted.
    pub fn has_full_disk_access(&self) -> bool {
        std::fs::read_dir(self.projects_dir()).is_ok()
    }

    /// List the encoded project directory names under `projects/`.
    pub fn list_project_dirs(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.projects_dir()) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect()
    }
}

/// Encode an absolute project path into its `projects/` directory name:
/// every non-alphanumeric character becomes `-`.
///
/// e.g. `/Users/me/dev/app` → `-Users-me-dev-app`.
pub fn encode_cwd(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Best-effort reverse of [`encode_cwd`]. This is **lossy** — both `/` and `-`
/// (and `.`, `_`, …) encode to `-`, so the original cannot be recovered exactly.
/// Prefer reading the authoritative `cwd` from a transcript event or
/// `claude agents --json`. This is only a display fallback.
pub fn best_effort_decode(encoded: &str) -> String {
    // The leading `-` corresponds to the root `/`; interior `-` are assumed to
    // be path separators. Real hyphens in directory names will be wrong.
    let s = encoded.strip_prefix('-').unwrap_or(encoded);
    format!("/{}", s.replace('-', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_paths_like_the_cli() {
        assert_eq!(
            encode_cwd("/Users/marcus/development/projects/mother-claude"),
            "-Users-marcus-development-projects-mother-claude"
        );
        assert_eq!(
            encode_cwd("/Users/marcus/development/projects/zira/zira-client"),
            "-Users-marcus-development-projects-zira-zira-client"
        );
    }

    #[test]
    fn encodes_non_alnum_including_dots_and_underscores() {
        assert_eq!(encode_cwd("/a/.config/my_app"), "-a--config-my-app");
        assert_eq!(encode_cwd("/x/y.z"), "-x-y-z");
    }

    #[test]
    fn transcript_path_is_built_correctly() {
        let home = ClaudeHome::with_base("/tmp/cfg");
        let p = home.transcript_path("/Users/me/app", "abc-123");
        assert_eq!(
            p,
            PathBuf::from("/tmp/cfg/projects/-Users-me-app/abc-123.jsonl")
        );
    }

    #[test]
    fn resolve_honors_explicit_base() {
        let home = ClaudeHome::with_base("/tmp/cfg");
        assert_eq!(
            home.roster_path(),
            PathBuf::from("/tmp/cfg/daemon/roster.json")
        );
        assert_eq!(home.jobs_dir(), PathBuf::from("/tmp/cfg/jobs"));
        assert_eq!(home.history_file(), PathBuf::from("/tmp/cfg/history.jsonl"));
    }

    #[test]
    fn best_effort_decode_roundtrips_simple_paths() {
        // No hyphens in the original → exact round-trip.
        assert_eq!(best_effort_decode("-Users-me-app"), "/Users/me/app");
    }

    #[test]
    fn missing_projects_dir_reports_no_access() {
        let home = ClaudeHome::with_base("/nonexistent-xyz-123");
        assert!(!home.has_full_disk_access());
        assert!(home.list_project_dirs().is_empty());
    }
}
