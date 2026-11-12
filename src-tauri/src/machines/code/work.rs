//! A run's work folder, git's half (docs/PLAN-MACHINES.md Phase 4, decision
//! 1): a kept `git worktree` of this computer's own checkout of a project,
//! set to the snapshot another computer sent before each run — its tracked
//! and untracked files as the snapshot holds them, the files git ignores
//! (`target/`, `node_modules/`, caches) left where they are, so a build
//! there picks up where the last one stopped.
//!
//! The user's checkout is never touched: its working tree, index, HEAD and
//! branches stay as they are. Its `.git` gains what any worktree and fetch
//! add — the worktree's admin folder, the snapshot's objects and its refs
//! under `refs/mother-claude/moves/<work>/…` — and every git call runs the
//! one way this module allows ([`super::GitRunner`]: argv only, hooks never
//! run).

use std::path::{Path, PathBuf};

use super::apply::{fetch_into, install_lfs, load_meta, same_place};
use super::check::is_oid;
use super::git::{trim_text, GitOp, GitRunner};
use super::repo::{native, repo_info, RepoInfo};
use super::types::{io_err, CodeError, SnapshotMeta};

/// The top of the working tree `path` is in.
pub fn top_of(git: &GitRunner, path: &Path) -> Result<PathBuf, CodeError> {
    let op = git.begin()?;
    Ok(repo_info(&op, path)?.top)
}

/// What [`fetch_snapshot`] brought in.
#[derive(Clone, Debug)]
pub struct Fetched {
    pub meta: SnapshotMeta,
    /// Git LFS objects the snapshot carried, newly installed here.
    pub lfs_installed: usize,
}

/// The snapshot in `in_dir` (`code/meta.json`'s folder: the record and its
/// bundle, from another computer) fetched into the repository at `repo`
/// under its refs and checked against its record, and the Git LFS objects
/// it carried installed. `work` is the snapshot's "session": the work
/// folder's id, which the record must name.
pub fn fetch_snapshot(
    git: &GitRunner,
    repo: &Path,
    in_dir: &Path,
    work: &str,
) -> Result<Fetched, CodeError> {
    let op = git.begin()?;
    let meta = load_meta(in_dir)?;
    if meta.session != work {
        return Err(CodeError::WrongSession);
    }
    let info = repo_info(&op, repo)?;
    fetch_into(&op, &info.top, in_dir, &meta, None, false)?;
    let lfs_installed = install_lfs(&info, in_dir, &meta)?;
    Ok(Fetched {
        meta,
        lfs_installed,
    })
}

/// What [`reset_work_tree`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkTree {
    /// The worktree was made now (it was missing, or not one of `repo`).
    pub made: bool,
    /// The commit it holds.
    pub head: String,
}

/// The work folder `folder` set to the commit `snap`: made a detached
/// worktree of the repository at `repo` when it is none (missing, emptied,
/// or a folder that is not one of `repo`'s worktrees: ours to make again),
/// then reset to the commit — every file the snapshot holds as it holds it,
/// untracked files it does not hold removed — keeping the files git
/// ignores. Submodules are not followed.
pub fn reset_work_tree(
    git: &GitRunner,
    repo: &Path,
    folder: &Path,
    snap: &str,
) -> Result<WorkTree, CodeError> {
    if !is_oid(snap) {
        return Err(CodeError::Invalid {
            what: "snapshot",
            value: snap.to_string(),
        });
    }
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    let made = ensure_worktree(&op, &info, folder, snap)?;
    op.cmd(folder)
        .args(["reset", "-q", "--hard", "--no-recurse-submodules", snap])
        .raw()?;
    // Untracked files the snapshot does not hold go; ignored ones stay.
    op.cmd(folder).args(["clean", "-f", "-d", "-q"]).raw()?;
    Ok(WorkTree {
        made,
        head: snap.to_string(),
    })
}

/// Whether `folder` is a worktree of `info`'s repository (its own `.git`
/// file, naming that repository's common folder).
fn is_worktree_of(op: &GitOp, info: &RepoInfo, folder: &Path) -> Result<bool, CodeError> {
    if !folder.join(".git").is_file() {
        return Ok(false);
    }
    let out = op
        .cmd(folder)
        .args(["rev-parse", "--show-toplevel", "--git-common-dir"])
        .output()?;
    if !out.success {
        return Ok(false);
    }
    let text = trim_text(&out.stdout);
    let mut lines = text.lines();
    let (Some(top), Some(common)) = (lines.next(), lines.next()) else {
        return Ok(false);
    };
    let top = native(top);
    let common = folder.join(native(common));
    Ok(same_place(&top, folder) && same_place(&common, &info.common_dir))
}

/// `folder` made a worktree of the repository, detached at `snap`, unless
/// it is one already. Answers whether it was made now.
fn ensure_worktree(
    op: &GitOp,
    info: &RepoInfo,
    folder: &Path,
    snap: &str,
) -> Result<bool, CodeError> {
    if is_worktree_of(op, info, folder)? {
        return Ok(false);
    }
    if folder.exists() {
        std::fs::remove_dir_all(folder).map_err(io_err(format!("clear {}", folder.display())))?;
    }
    if let Some(parent) = folder.parent() {
        std::fs::create_dir_all(parent).map_err(io_err(format!("create {}", parent.display())))?;
    }
    // `-f`: a worktree registered at this path whose folder is gone is
    // ours; it is replaced, never another one.
    op.cmd(&info.top)
        .args(["worktree", "add", "-q", "-f", "--detach", "--no-checkout"])
        .arg(folder)
        .arg(snap)
        .raw()?;
    Ok(true)
}
