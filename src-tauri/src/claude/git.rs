//! Per-session Git state via libgit2 (`git2`), with no network features.
//!
//! A session's working directory (its `cwd`) is the repo we diff. Background
//! sessions with worktree isolation live under `<project>/.claude/worktrees/<name>`;
//! those are enumerated too. The diff is HEAD → working tree (staged + unstaged +
//! untracked), which is the practical "what has this session changed" view.
//!
//! NOTE: the brief suggests correlating with `file-history-snapshot` transcript
//! records for a precise per-session change set; the working-tree diff is the
//! pragmatic baseline and is what the dashboard shows today.

use std::path::{Path, PathBuf};

use git2::{Delta, DiffOptions, Repository, StatusOptions};
use serde::{Deserialize, Serialize};

/// A changed file in the working tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
}

/// One recent commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub id: String,
    pub summary: String,
    pub author: String,
    /// Commit time, epoch seconds.
    pub time: i64,
}

/// A linked worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub name: String,
    pub path: String,
    pub locked: bool,
}

/// Combined per-session Git overview.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOverview {
    pub is_repo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    pub files: Vec<FileChange>,
    pub additions: usize,
    pub deletions: usize,
    pub commits: Vec<CommitInfo>,
    pub worktrees: Vec<WorktreeInfo>,
}

fn delta_status(delta: Delta) -> &'static str {
    match delta {
        Delta::Added => "added",
        Delta::Deleted => "deleted",
        Delta::Modified => "modified",
        Delta::Renamed => "renamed",
        Delta::Copied => "copied",
        Delta::Untracked => "untracked",
        Delta::Typechange => "typechange",
        Delta::Conflicted => "conflicted",
        _ => "unknown",
    }
}

/// Open the repository that contains `cwd` (searching upward). Returns `None` if
/// `cwd` is not inside a Git repository.
fn open(cwd: &Path) -> Option<Repository> {
    Repository::discover(cwd).ok()
}

fn current_branch(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    if head.is_branch() {
        head.shorthand().ok().map(String::from)
    } else {
        // Detached HEAD — report the short commit id.
        head.target().map(|oid| {
            let s = oid.to_string();
            s.chars().take(8).collect()
        })
    }
}

fn head_commit_id(repo: &Repository) -> Option<String> {
    let oid = repo.head().ok()?.target()?;
    Some(oid.to_string().chars().take(8).collect())
}

fn collect_changes(repo: &Repository) -> (Vec<FileChange>, usize, usize) {
    let mut opts = DiffOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);

    // Prefer HEAD tree → workdir; if there is no HEAD yet (no commits), diff the
    // empty tree → workdir so newly added files still show up.
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let diff = match head_tree {
        Some(tree) => repo.diff_tree_to_workdir_with_index(Some(&tree), Some(&mut opts)),
        None => repo.diff_tree_to_workdir_with_index(None, Some(&mut opts)),
    };
    let Ok(diff) = diff else {
        return (Vec::new(), 0, 0);
    };

    let mut files = Vec::new();
    let mut total_add = 0usize;
    let mut total_del = 0usize;

    let n = diff.deltas().len();
    for i in 0..n {
        let Ok(Some(patch)) = git2::Patch::from_diff(&diff, i) else {
            continue;
        };
        let (_ctx, additions, deletions) = patch.line_stats().unwrap_or((0, 0, 0));
        let delta = patch.delta();
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        total_add += additions;
        total_del += deletions;
        files.push(FileChange {
            path,
            status: delta_status(delta.status()).to_string(),
            additions,
            deletions,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    (files, total_add, total_del)
}

fn recent_log(repo: &Repository, limit: usize) -> Vec<CommitInfo> {
    let mut commits = Vec::new();
    let Ok(mut walk) = repo.revwalk() else {
        return commits;
    };
    if walk.push_head().is_err() {
        return commits;
    }
    for oid in walk.flatten().take(limit) {
        if let Ok(commit) = repo.find_commit(oid) {
            commits.push(CommitInfo {
                id: oid.to_string().chars().take(8).collect(),
                summary: commit.summary().ok().flatten().unwrap_or("").to_string(),
                author: commit.author().name().unwrap_or("").to_string(),
                time: commit.time().seconds(),
            });
        }
    }
    commits
}

fn list_worktrees(repo: &Repository) -> Vec<WorktreeInfo> {
    let mut out = Vec::new();
    let Ok(names) = repo.worktrees() else {
        return out;
    };
    for name in names.iter().filter_map(|r| r.ok().flatten()) {
        if let Ok(wt) = repo.find_worktree(name) {
            out.push(WorktreeInfo {
                name: name.to_string(),
                path: wt.path().to_string_lossy().to_string(),
                locked: matches!(wt.is_locked(), Ok(git2::WorktreeLockStatus::Locked(_))),
            });
        }
    }
    out
}

/// Build the full Git overview for a session's working directory.
pub fn overview(cwd: &Path, log_limit: usize) -> GitOverview {
    let Some(repo) = open(cwd) else {
        return GitOverview::default();
    };
    let (files, additions, deletions) = collect_changes(&repo);
    GitOverview {
        is_repo: true,
        repo_root: repo
            .workdir()
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| Some(repo.path().to_string_lossy().to_string())),
        branch: current_branch(&repo),
        head: head_commit_id(&repo),
        files,
        additions,
        deletions,
        commits: recent_log(&repo, log_limit),
        worktrees: list_worktrees(&repo),
    }
}

/// Return the unified diff (patch) text for a single file in the working tree,
/// truncated to `max_bytes`. `None` if not a repo or the file has no diff.
pub fn file_patch(cwd: &Path, rel_path: &str, max_bytes: usize) -> Option<String> {
    let repo = open(cwd)?;
    let mut opts = DiffOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .pathspec(rel_path);
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let diff = repo
        .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
        .ok()?;

    let mut buf = String::new();
    let _ = diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        match line.origin() {
            '+' | '-' | ' ' => buf.push(line.origin()),
            _ => {}
        }
        buf.push_str(&String::from_utf8_lossy(line.content()));
        buf.len() < max_bytes
    });
    if buf.is_empty() {
        None
    } else {
        if buf.len() >= max_bytes {
            buf.push_str("\n… (truncated)\n");
        }
        Some(buf)
    }
}

/// Best-effort check whether `cwd` has any uncommitted changes (cheap status).
pub fn is_dirty(cwd: &Path) -> bool {
    let Some(repo) = open(cwd) else {
        return false;
    };
    let mut opts = StatusOptions::new();
    opts.include_untracked(true);
    repo.statuses(Some(&mut opts))
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// Worktrees Mother Claude-style: any under `<project>/.claude/worktrees/`.
pub fn claude_worktrees_dir(project_root: &Path) -> PathBuf {
    project_root.join(".claude").join("worktrees")
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature};
    use std::fs;
    use std::path::Path;

    fn commit_all(repo: &Repository, msg: &str) -> git2::Oid {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();
        let parents = match repo.head().ok().and_then(|h| h.target()) {
            Some(oid) => vec![repo.find_commit(oid).unwrap()],
            None => vec![],
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
            .unwrap()
    }

    fn init_repo(path: &Path) -> Repository {
        let repo = Repository::init(path).unwrap();
        {
            let mut cfg = repo.config().unwrap();
            cfg.set_str("user.name", "Test").unwrap();
            cfg.set_str("user.email", "test@example.com").unwrap();
        }
        repo
    }

    #[test]
    fn non_repo_path_reports_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let ov = overview(dir.path(), 10);
        assert!(!ov.is_repo);
        assert!(ov.files.is_empty());
    }

    #[test]
    fn reports_branch_commit_and_modified_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        commit_all(&repo, "initial");

        // Modify the file (1 deletion + 1 addition for the changed line).
        fs::write(dir.path().join("a.txt"), "one\nTWO\nthree\n").unwrap();

        let ov = overview(dir.path(), 10);
        assert!(ov.is_repo);
        assert_eq!(ov.commits.len(), 1);
        assert_eq!(ov.commits[0].summary, "initial");
        assert_eq!(ov.files.len(), 1);
        assert_eq!(ov.files[0].path, "a.txt");
        assert_eq!(ov.files[0].status, "modified");
        assert_eq!(ov.files[0].additions, 1);
        assert_eq!(ov.files[0].deletions, 1);
        assert_eq!(ov.additions, 1);
        assert_eq!(ov.deletions, 1);
        assert!(ov.branch.is_some());
        assert!(ov.head.is_some());
    }

    #[test]
    fn reports_untracked_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "x\n").unwrap();
        commit_all(&repo, "init");
        fs::write(dir.path().join("new.txt"), "fresh\n").unwrap();

        let ov = overview(dir.path(), 10);
        let new = ov.files.iter().find(|f| f.path == "new.txt").unwrap();
        assert_eq!(new.status, "untracked");
        assert!(super::is_dirty(dir.path()));
    }

    #[test]
    fn file_patch_returns_diff_text() {
        let dir = tempfile::tempdir().unwrap();
        let repo = init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        commit_all(&repo, "init");
        fs::write(dir.path().join("a.txt"), "one\nTWO\n").unwrap();

        let patch = file_patch(dir.path(), "a.txt", 64_000).unwrap();
        assert!(patch.contains("-two"));
        assert!(patch.contains("+TWO"));
    }
}
