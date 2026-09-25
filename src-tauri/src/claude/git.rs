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
    /// `?base=branch`: the default branch the diff is taken against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// `?base=branch`: the merge base commit (short id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_base: Option<String>,
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
    // Prefer HEAD tree → workdir; if there is no HEAD yet (no commits), diff the
    // empty tree → workdir so newly added files still show up.
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    collect_changes_from(repo, head_tree.as_ref())
}

/// Changes from `base` (a tree; `None` = empty tree) to the working tree.
fn collect_changes_from(
    repo: &Repository,
    base: Option<&git2::Tree>,
) -> (Vec<FileChange>, usize, usize) {
    let mut opts = DiffOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let diff = repo.diff_tree_to_workdir_with_index(base, Some(&mut opts));
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
        ..Default::default()
    }
}

/// Return the unified diff (patch) text for a single file in the working tree,
/// truncated to `max_bytes`. `None` if not a repo or the file has no diff.
pub fn file_patch(cwd: &Path, rel_path: &str, max_bytes: usize) -> Option<String> {
    file_patch_against(cwd, rel_path, max_bytes, false)
}

/// [`file_patch`] against HEAD (`branch == false`) or against the merge base
/// with the default branch (`branch == true`). The path is matched
/// literally (no pathspec globbing).
pub fn file_patch_against(
    cwd: &Path,
    rel_path: &str,
    max_bytes: usize,
    branch: bool,
) -> Option<String> {
    let repo = open(cwd)?;
    let mut opts = DiffOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .disable_pathspec_match(true)
        .pathspec(rel_path);
    let head_tree = || repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let base_tree = match branch.then(|| merge_base_with_default(&repo)).flatten() {
        Some((_, oid)) => repo.find_commit(oid).ok().and_then(|c| c.tree().ok()),
        // No default branch to compare with: HEAD, as `branch_overview`
        // falls back to the working tree changes (never the empty tree).
        None => head_tree(),
    };
    let diff = repo
        .diff_tree_to_workdir_with_index(base_tree.as_ref(), Some(&mut opts))
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

/// The default branch (`origin/HEAD`'s target, else `main`/`master`, local or
/// on origin) and its commit.
fn default_branch(repo: &Repository) -> Option<(String, git2::Oid)> {
    if let Ok(r) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Ok(resolved) = r.resolve() {
            if let (Some(oid), Ok(name)) = (resolved.target(), resolved.shorthand()) {
                return Some((name.to_string(), oid));
            }
        }
    }
    for name in [
        "main",
        "master",
        "origin/main",
        "origin/master",
        "trunk",
        "develop",
    ] {
        let refname = if name.starts_with("origin/") {
            format!("refs/remotes/{name}")
        } else {
            format!("refs/heads/{name}")
        };
        if let Some(oid) = repo.find_reference(&refname).ok().and_then(|r| r.target()) {
            return Some((name.to_string(), oid));
        }
    }
    None
}

/// `(default branch name, merge-base commit)` of HEAD and the default branch.
fn merge_base_with_default(repo: &Repository) -> Option<(String, git2::Oid)> {
    let head = repo.head().ok()?.target()?;
    let (name, def) = default_branch(repo)?;
    let base = repo.merge_base(head, def).ok()?;
    Some((name, base))
}

/// The branch-vs-default overview: everything on this branch since it left
/// the default branch (commits + committed and uncommitted changes).
pub fn branch_overview(cwd: &Path, log_limit: usize) -> GitOverview {
    let Some(repo) = open(cwd) else {
        return GitOverview::default();
    };
    let mut ov = GitOverview {
        is_repo: true,
        repo_root: repo
            .workdir()
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| Some(repo.path().to_string_lossy().to_string())),
        branch: current_branch(&repo),
        head: head_commit_id(&repo),
        worktrees: list_worktrees(&repo),
        ..Default::default()
    };
    let Some((name, base)) = merge_base_with_default(&repo) else {
        // No default branch to compare with: fall back to the working tree.
        let (files, a, d) = collect_changes(&repo);
        ov.files = files;
        ov.additions = a;
        ov.deletions = d;
        ov.commits = recent_log(&repo, log_limit);
        return ov;
    };
    let tree = repo.find_commit(base).ok().and_then(|c| c.tree().ok());
    let (files, a, d) = collect_changes_from(&repo, tree.as_ref());
    ov.files = files;
    ov.additions = a;
    ov.deletions = d;
    ov.base = Some(name);
    ov.merge_base = Some(base.to_string().chars().take(8).collect());
    if let Ok(mut walk) = repo.revwalk() {
        if walk.push_head().is_ok() && walk.hide(base).is_ok() {
            for oid in walk.flatten().take(log_limit) {
                if let Ok(commit) = repo.find_commit(oid) {
                    ov.commits.push(CommitInfo {
                        id: oid.to_string().chars().take(8).collect(),
                        summary: commit.summary().ok().flatten().unwrap_or("").to_string(),
                        author: commit.author().name().unwrap_or("").to_string(),
                        time: commit.time().seconds(),
                    });
                }
            }
        }
    }
    ov
}

/// Tracked files under `cwd` (paths relative to `cwd`), like `git ls-files`
/// — cached per repo for 30 s.
pub fn list_files(cwd: &Path) -> Vec<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    type Cache = HashMap<PathBuf, (Instant, std::sync::Arc<Vec<String>>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    const TTL: Duration = Duration::from_secs(30);
    const MAX_FILES: usize = 200_000;

    let key = cwd.to_path_buf();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(g) = cache.lock() {
        if let Some((at, files)) = g.get(&key) {
            if at.elapsed() < TTL {
                return files.as_ref().clone();
            }
        }
    }
    let Some(repo) = open(cwd) else {
        return Vec::new();
    };
    let Some(root) = repo.workdir().and_then(|w| std::fs::canonicalize(w).ok()) else {
        return Vec::new();
    };
    let here = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let prefix = here
        .strip_prefix(&root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let prefix = if prefix.is_empty() {
        String::new()
    } else {
        format!("{}/", prefix.trim_end_matches('/'))
    };
    let mut files = Vec::new();
    if let Ok(index) = repo.index() {
        for entry in index.iter().take(MAX_FILES) {
            let path = String::from_utf8_lossy(&entry.path).to_string();
            if let Some(rel) = path.strip_prefix(&prefix) {
                files.push(rel.to_string());
            }
        }
    }
    files.sort();
    if let Ok(mut g) = cache.lock() {
        if g.len() > 32 {
            g.clear();
        }
        g.insert(key, (Instant::now(), std::sync::Arc::new(files.clone())));
    }
    files
}

/// Fuzzy file match for `@` completion: substring matches first (basename
/// matches best), then in-order subsequences; at most `limit`.
pub fn fuzzy_files(files: &[String], query: &str, limit: usize) -> Vec<String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return files.iter().take(limit).cloned().collect();
    }
    let mut scored: Vec<(i64, &String)> = files
        .iter()
        .filter_map(|f| {
            let lower = f.to_lowercase();
            let base = lower.rsplit('/').next().unwrap_or(&lower);
            if let Some(i) = base.find(&q) {
                return Some((3000 - i as i64 - f.len() as i64 / 4, f));
            }
            if let Some(i) = lower.find(&q) {
                return Some((2000 - i as i64 - f.len() as i64 / 4, f));
            }
            // Subsequence with a gap penalty.
            let mut gaps = 0i64;
            let mut last: Option<usize> = None;
            let mut it = lower.char_indices();
            for qc in q.chars() {
                let (pos, _) = it.by_ref().find(|(_, c)| *c == qc)?;
                if let Some(l) = last {
                    gaps += (pos - l - 1) as i64;
                }
                last = Some(pos);
            }
            Some((1000 - gaps - f.len() as i64 / 4, f))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, f)| f.clone())
        .collect()
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
        // Pathspecs are literal: a glob names nothing.
        assert!(file_patch(dir.path(), "*.txt", 64_000).is_none());
    }

    #[test]
    fn branch_overview_diffs_against_the_merge_base() {
        let dir = tempfile::tempdir().unwrap();
        let repo = init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "base\n").unwrap();
        let base = commit_all(&repo, "base");
        // Name the default branch `main` and branch off it.
        let base_commit = repo.find_commit(base).unwrap();
        repo.branch("main", &base_commit, true).unwrap();
        repo.branch("feature", &base_commit, true).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        fs::write(dir.path().join("b.txt"), "on branch\n").unwrap();
        commit_all(&repo, "feature work");
        fs::write(dir.path().join("c.txt"), "uncommitted\n").unwrap();

        let ov = branch_overview(dir.path(), 10);
        assert_eq!(ov.base.as_deref(), Some("main"));
        assert!(ov.merge_base.is_some());
        let paths: Vec<&str> = ov.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"b.txt") && paths.contains(&"c.txt"));
        assert!(!paths.contains(&"a.txt"));
        assert_eq!(ov.commits.len(), 1);
        assert_eq!(ov.commits[0].summary, "feature work");
        let patch = file_patch_against(dir.path(), "b.txt", 64_000, true).unwrap();
        assert!(patch.contains("+on branch"));
    }

    #[test]
    fn branch_patch_without_a_default_branch_diffs_against_head() {
        let dir = tempfile::tempdir().unwrap();
        let repo = init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        let first = commit_all(&repo, "init");
        // The only branch is `dev`: no origin/HEAD, main, master, … to find.
        let commit = repo.find_commit(first).unwrap();
        repo.branch("dev", &commit, true).unwrap();
        repo.set_head("refs/heads/dev").unwrap();
        for name in ["main", "master"] {
            if let Ok(mut b) = repo.find_branch(name, git2::BranchType::Local) {
                b.delete().unwrap();
            }
        }
        fs::write(dir.path().join("a.txt"), "one\nTWO\n").unwrap();

        let ov = branch_overview(dir.path(), 10);
        assert!(ov.merge_base.is_none());
        let patch = file_patch_against(dir.path(), "a.txt", 64_000, true).unwrap();
        assert!(patch.contains("-two") && patch.contains("+TWO"), "{patch}");
        assert!(
            !patch.contains("+one"),
            "not the whole file as added: {patch}"
        );
    }

    #[test]
    fn lists_and_fuzzy_matches_files() {
        let dir = tempfile::tempdir().unwrap();
        let repo = init_repo(dir.path());
        fs::create_dir_all(dir.path().join("src/app")).unwrap();
        fs::write(dir.path().join("src/app/main.rs"), "x").unwrap();
        fs::write(dir.path().join("src/lib.rs"), "x").unwrap();
        fs::write(dir.path().join("README.md"), "x").unwrap();
        commit_all(&repo, "init");
        let files = list_files(dir.path());
        assert_eq!(files.len(), 3);
        let sub = list_files(&dir.path().join("src"));
        assert!(sub.contains(&"lib.rs".to_string()));
        assert!(sub.contains(&"app/main.rs".to_string()));
        let m = fuzzy_files(&files, "main", 50);
        assert_eq!(m[0], "src/app/main.rs");
        let m = fuzzy_files(&files, "sam", 50);
        assert_eq!(m, vec!["src/app/main.rs".to_string()]);
        assert!(fuzzy_files(&files, "zzz", 50).is_empty());
        assert_eq!(fuzzy_files(&files, "", 2).len(), 2);
    }
}
