//! A task handed to another computer (docs/PLAN-MACHINES.md Phase 4,
//! decision 6), git's half.
//!
//! The task session works in a worktree of the target's checkout made at
//! the handed-over snapshot (`work::reset_work_tree`), so the session's code
//! — its uncommitted changes included — is the worktree's first commit.
//! When the session's turn ends, what it did since — its commits and what
//! it left in the working tree — is a snapshot of its own ([`task_result`]),
//! kept under `refs/mother-claude/moves/<task>-result/…` and chained on the
//! last result (an extra parent), so each result descends from every
//! earlier one and the branch it becomes on the asking computer only ever
//! moves forward; its bundle carries only what that computer lacks.
//!
//! On the asking computer the result becomes `refs/heads/mother-claude/
//! <machine>/<task>` ([`land_task_branch`]): fetched from the bundle and
//! checked like every snapshot (it comes from another machine), never
//! checked out, never merged, moved only forward and never while a worktree
//! has it checked out. [`remove_task_tree`] takes the worktree and its refs
//! away again.
//!
//! Like the rest of `code`: argv only, a private empty hooks folder per
//! operation, and every id checked before git sees it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::check::{is_oid, safe_name, valid_branch, valid_session};
use super::git::{GitOp, GitRunner};
use super::repo::{busy, checked_trees, native, repo_info, unmerged};
use super::snapshot::snapshot;
use super::types::{io_err, CodeError, SnapshotMeta, SnapshotRequest};
use super::work::{fetch_snapshot, is_worktree_of};

/// What the branch names start with.
pub const TASK_BRANCH_PREFIX: &str = "refs/heads/mother-claude/";

/// The "session" a task's results are kept under, on both computers:
/// `refs/mother-claude/moves/<task>-result/<epoch>/{snap,index}`.
pub fn result_session(task: &str) -> String {
    format!("{task}-result")
}

/// The branch a task's result becomes on the asking computer:
/// `refs/heads/mother-claude/<machine>/<task>`, the machine's name made
/// safe for a ref. `None` when the task's id could not be part of one.
pub fn task_branch(machine_name: &str, task: &str) -> Option<String> {
    if !valid_session(task) || task.starts_with('.') {
        return None;
    }
    let branch = format!("{TASK_BRANCH_PREFIX}{}/{task}", safe_name(machine_name));
    valid_branch(&branch).then_some(branch)
}

/// The branch a team role on another computer's work becomes in the lead's
/// project (docs/PLAN-MACHINES.md Phase 4 decision 9):
/// `refs/heads/mother-claude/<machine>/<team>/<role>`, the computer's and
/// the team's names made safe for a ref. `None` when the role's id could
/// not be part of one.
pub fn role_branch(machine_name: &str, team: &str, role: &str) -> Option<String> {
    if !valid_session(role) || role.starts_with('.') {
        return None;
    }
    let branch = format!(
        "{TASK_BRANCH_PREFIX}{}/{}/{role}",
        safe_name(machine_name),
        safe_name(team)
    );
    valid_branch(&branch).then_some(branch)
}

/// Whether a task's worktree holds nothing since its last result (or,
/// before the first one, since the code it got): what [`task_result`]
/// would answer [`TaskResult::Unchanged`] for, without taking anything. A
/// merge or rebase under way, or unmerged files, count as changes.
pub fn task_unchanged(
    git: &GitRunner,
    worktree: &Path,
    base: &str,
    base_tree: &str,
    last: Option<&TaskTip>,
) -> Result<bool, CodeError> {
    for oid in [base, base_tree] {
        if !is_oid(oid) {
            return Err(CodeError::Invalid {
                what: "commit",
                value: oid.to_string(),
            });
        }
    }
    if let Some(l) = last {
        l.validate()?;
    }
    let op = git.begin()?;
    let info = repo_info(&op, worktree)?;
    if busy(&info).is_some() || unmerged(&op, &info.top)? {
        return Ok(false);
    }
    let (_, wt_tree, _) = checked_trees(&op, &info)?;
    Ok(match last {
        Some(l) => l.head == info.head && l.wt_tree == wt_tree,
        None => info.head.as_deref() == Some(base) && wt_tree == base_tree,
    })
}

/// Whether commit `a` is in commit `b`'s history in the repository at
/// `repo` (a lead's newer code holds a role's work once it merged it).
pub fn contains(git: &GitRunner, repo: &Path, a: &str, b: &str) -> Result<bool, CodeError> {
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    is_ancestor(&op, &info.top, a, b)
}

/// A result of a task: what the worktree held when it was taken, and the
/// tip the branch gets for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskTip {
    /// The worktree's HEAD (none on an unborn branch).
    #[serde(default)]
    pub head: Option<String>,
    /// Its working tree, untracked files included (ignored ones not).
    pub wt_tree: String,
    /// The result's snapshot commit: the working tree, HEAD its first
    /// parent, the last result's snapshot its second.
    pub snap: String,
    /// The branch's tip: HEAD itself when the working tree is HEAD's and
    /// the last tip is in HEAD's history (a session that committed
    /// everything), else `snap`.
    pub tip: String,
}

impl TaskTip {
    /// Every id checked (one comes from another machine).
    pub fn validate(&self) -> Result<(), CodeError> {
        let ids = [&self.wt_tree, &self.snap, &self.tip];
        if ids.iter().any(|i| !is_oid(i)) || self.head.as_deref().is_some_and(|h| !is_oid(h)) {
            return Err(CodeError::Invalid {
                what: "task result",
                value: self.snap.clone(),
            });
        }
        Ok(())
    }
}

/// What [`task_result`] is asked.
#[derive(Clone, Debug)]
pub struct TaskResultRequest {
    /// The task's id.
    pub task: String,
    pub epoch: u64,
    /// This machine's name (the commits say where they were made).
    pub machine: String,
    /// Where the snapshot's record and bundle go (`meta.json`, the bundle).
    pub out_dir: PathBuf,
    /// The handed-over snapshot (the worktree's first commit) and its tree.
    pub base: String,
    pub base_tree: String,
    /// The last result made (the next one descends from it).
    pub last: Option<TaskTip>,
    /// Commits the asking computer has: the bundle leaves them out.
    pub haves: Vec<String>,
}

/// What [`task_result`] found.
#[derive(Clone, Debug)]
pub enum TaskResult {
    /// Nothing changed since the last result (or, before the first one,
    /// since the handed-over code): no snapshot was made.
    Unchanged,
    Made {
        meta: Box<SnapshotMeta>,
        tip: TaskTip,
    },
}

/// `git merge-base --is-ancestor a b`.
fn is_ancestor(op: &GitOp, top: &Path, a: &str, b: &str) -> Result<bool, CodeError> {
    if !is_oid(a) || !is_oid(b) {
        return Ok(false);
    }
    let out = op
        .cmd(top)
        .args(["merge-base", "--is-ancestor", a, b])
        .output()?;
    match out.code {
        0 => Ok(true),
        1 => Ok(false),
        code => Err(CodeError::Git {
            command: "merge-base --is-ancestor".into(),
            code,
            stderr: out.stderr.trim().chars().take(500).collect(),
        }),
    }
}

/// The tree of a commit.
fn tree_of(op: &GitOp, top: &Path, commit: &str) -> Result<String, CodeError> {
    op.git(top, &["rev-parse", &format!("{commit}^{{tree}}")])
}

/// The task's result in `worktree`, unless nothing changed since the last
/// one ([`TaskResult::Unchanged`]): a snapshot of the worktree (its HEAD,
/// what is staged, its working tree with untracked files) kept under
/// [`result_session`], its bundle without what the asking computer has,
/// descending from the last result. A merge or rebase under way there is
/// [`CodeError::InProgress`]; the worktree itself is never touched.
pub fn task_result(
    git: &GitRunner,
    worktree: &Path,
    req: &TaskResultRequest,
) -> Result<TaskResult, CodeError> {
    let session = result_session(&req.task);
    if !valid_session(&session) {
        return Err(CodeError::Invalid {
            what: "task id",
            value: req.task.clone(),
        });
    }
    for oid in [&req.base, &req.base_tree] {
        if !is_oid(oid) {
            return Err(CodeError::Invalid {
                what: "commit",
                value: oid.clone(),
            });
        }
    }
    if let Some(l) = &req.last {
        l.validate()?;
    }
    let op = git.begin()?;
    let info = repo_info(&op, worktree)?;
    if let Some(what) = busy(&info) {
        return Err(CodeError::InProgress(what));
    }
    if unmerged(&op, &info.top)? {
        return Err(CodeError::Unmerged);
    }
    let (_, wt_tree, _) = checked_trees(&op, &info)?;
    let unchanged = match &req.last {
        Some(l) => l.head == info.head && l.wt_tree == wt_tree,
        None => info.head.as_deref() == Some(req.base.as_str()) && wt_tree == req.base_tree,
    };
    if unchanged {
        return Ok(TaskResult::Unchanged);
    }
    let mut sreq = SnapshotRequest::new(&session, req.epoch, &req.machine, req.out_dir.clone());
    let mut haves = vec![req.base.clone()];
    haves.extend(req.haves.iter().filter(|h| is_oid(h)).cloned());
    sreq.haves = haves;
    sreq.chain = false;
    sreq.extra_parents = req.last.iter().map(|l| l.snap.clone()).collect();
    sreq.label = Some(format!("{} on {}", req.task, safe_name(&req.machine)));
    let meta = snapshot(git, worktree, &sreq)?;
    let clean = match &meta.base {
        Some(h) => tree_of(&op, &info.top, h)? == meta.wt_tree,
        None => false,
    };
    let follows = match (&req.last, &meta.base) {
        (Some(l), Some(h)) => is_ancestor(&op, &info.top, &l.tip, h)?,
        (None, Some(_)) => true,
        (_, None) => false,
    };
    let tip = match &meta.base {
        Some(h) if clean && follows => h.clone(),
        _ => meta.snap.clone(),
    };
    let tip = TaskTip {
        head: meta.base.clone(),
        wt_tree: meta.wt_tree.clone(),
        snap: meta.snap.clone(),
        tip,
    };
    Ok(TaskResult::Made {
        meta: Box::new(meta),
        tip,
    })
}

/// What landing a result did on the asking computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskLanded {
    /// The branch was made now.
    pub created: bool,
    /// The branch points somewhere new (false: it pointed at the tip).
    pub moved: bool,
    /// Commits the task session made since the handed-over code.
    pub commits: u32,
    /// Files that differ between the handed-over code and the tip.
    pub files: u32,
    /// The tip holds changes the session left uncommitted.
    pub uncommitted: bool,
}

/// Why a result did not become the branch (it is fetched and kept under
/// its refs all the same).
#[derive(Debug, thiserror::Error)]
pub enum TaskBranchError {
    #[error("{} is checked out in {}, so it is not moved under it.", short_branch(.branch), .worktree.display())]
    CheckedOut { branch: String, worktree: PathBuf },
    #[error("{} has commits the task's result does not have (it changed here), so it is not moved.", short_branch(.branch))]
    Diverged { branch: String, current: String },
    #[error(transparent)]
    Failed(#[from] CodeError),
}

/// `mother-claude/…` without `refs/heads/`.
pub fn short_branch(branch: &str) -> &str {
    branch.strip_prefix("refs/heads/").unwrap_or(branch)
}

/// A worktree (the main one too) that has `branch` checked out.
fn checked_out_in(op: &GitOp, top: &Path, branch: &str) -> Result<Option<PathBuf>, CodeError> {
    let out = op
        .cmd(top)
        .args(["worktree", "list", "--porcelain", "-z"])
        .raw()?;
    let mut wt: Option<PathBuf> = None;
    for field in out.split(|b| *b == 0) {
        let line = String::from_utf8_lossy(field);
        if let Some(p) = line.strip_prefix("worktree ") {
            wt = Some(native(p));
        } else if line.strip_prefix("branch ") == Some(branch) {
            return Ok(Some(wt.clone().unwrap_or_else(|| top.to_path_buf())));
        }
    }
    Ok(None)
}

/// The task's result in `in_dir` (its `meta.json` and bundle, from the
/// computer that ran the task) fetched into the repository at `repo` and
/// checked, then made the branch `branch` at `tip` (the result's snapshot,
/// or the HEAD it names): created, or moved forward — never when that
/// would drop a commit the branch has, never while a worktree has it
/// checked out. `base` is the handed-over code, for what it changed.
pub fn land_task_branch(
    git: &GitRunner,
    repo: &Path,
    in_dir: &Path,
    task: &str,
    branch: &str,
    tip: &str,
    base: &str,
) -> Result<(SnapshotMeta, TaskLanded), TaskBranchError> {
    if !valid_branch(branch) || !branch.starts_with(TASK_BRANCH_PREFIX) {
        return Err(CodeError::Invalid {
            what: "branch",
            value: branch.to_string(),
        }
        .into());
    }
    for oid in [tip, base] {
        if !is_oid(oid) {
            return Err(CodeError::Invalid {
                what: "commit",
                value: oid.to_string(),
            }
            .into());
        }
    }
    let fetched = fetch_snapshot(git, repo, in_dir, &result_session(task))?;
    let meta = fetched.meta;
    if tip != meta.snap && meta.base.as_deref() != Some(tip) {
        return Err(CodeError::FetchMismatch.into());
    }
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    let top = &info.top;
    let current = op.rev(top, &format!("{branch}^{{commit}}"))?;
    let (created, moved) = match current.as_deref() {
        Some(c) if c == tip => (false, false),
        Some(c) => {
            if let Some(worktree) = checked_out_in(&op, top, branch)? {
                return Err(TaskBranchError::CheckedOut {
                    branch: branch.to_string(),
                    worktree,
                });
            }
            if !is_ancestor(&op, top, c, tip)? {
                return Err(TaskBranchError::Diverged {
                    branch: branch.to_string(),
                    current: c.to_string(),
                });
            }
            op.cmd(top)
                .args(["update-ref", "-m", "mother-claude: a task's result"])
                .args([branch, tip, c])
                .raw()?;
            (false, true)
        }
        None => {
            op.cmd(top)
                .args(["update-ref", "-m", "mother-claude: a task's result"])
                .args([branch, tip])
                .arg("")
                .raw()?;
            (true, true)
        }
    };
    let commits = match &meta.base {
        Some(h) => op
            .git(top, &["rev-list", "--count", &format!("{base}..{h}")])?
            .trim()
            .parse::<u32>()
            .unwrap_or(0),
        None => 0,
    };
    let names = op
        .cmd(top)
        .args([
            "diff",
            "--name-only",
            "-z",
            "-M",
            "--no-ext-diff",
            "--no-textconv",
            base,
            tip,
        ])
        .raw()?;
    let files = names.split(|b| *b == 0).filter(|n| !n.is_empty()).count() as u32;
    let uncommitted = match &meta.base {
        Some(h) => tip == meta.snap && tree_of(&op, top, h)? != meta.wt_tree,
        None => true,
    };
    Ok((
        meta,
        TaskLanded {
            created,
            moved,
            commits,
            files,
            uncommitted,
        },
    ))
}

/// Deletes every ref under `refs/mother-claude/moves/<session>/`.
fn drop_refs(op: &GitOp, top: &Path, session: &str) -> Result<usize, CodeError> {
    if !valid_session(session) {
        return Err(CodeError::Invalid {
            what: "session id",
            value: session.to_string(),
        });
    }
    let prefix = format!("refs/mother-claude/moves/{session}/");
    let out = op
        .cmd(top)
        .args(["for-each-ref", "--format=%(refname)"])
        .arg(&prefix)
        .text()?;
    let refs: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|r| r.starts_with(&prefix) && !r.contains(char::is_whitespace))
        .collect();
    if refs.is_empty() {
        return Ok(0);
    }
    let input: String = refs.iter().map(|r| format!("delete {r}\n")).collect();
    op.cmd(top)
        .args(["update-ref", "--stdin"])
        .input(input)
        .raw()?;
    Ok(refs.len())
}

/// The refs a task left in the repository at `repo` (the handed-over
/// snapshot's, the results'), not its branch. Answers how many went.
pub fn drop_task_refs(git: &GitRunner, repo: &Path, task: &str) -> Result<usize, CodeError> {
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    let mut n = drop_refs(&op, &info.top, task)?;
    n += drop_refs(&op, &info.top, &result_session(task))?;
    Ok(n)
}

/// A task's worktree `folder` removed from the checkout at `repo` (its
/// admin entry too, whatever it holds: its work went back with the
/// results), and the task's refs there. A checkout that is gone leaves
/// only the folder to delete.
pub fn remove_task_tree(
    git: &GitRunner,
    repo: &Path,
    folder: &Path,
    task: &str,
) -> Result<(), CodeError> {
    if !valid_session(task) {
        return Err(CodeError::Invalid {
            what: "task id",
            value: task.to_string(),
        });
    }
    let op = git.begin()?;
    let info = match repo_info(&op, repo) {
        Ok(i) => Some(i),
        Err(CodeError::NotARepository(_)) => None,
        Err(e) => return Err(e),
    };
    if let Some(info) = &info {
        if folder.exists() && is_worktree_of(&op, info, folder)? {
            op.cmd(&info.top)
                .args(["worktree", "remove", "--force", "--force"])
                .arg(folder)
                .raw()?;
        }
    }
    if folder.exists() {
        std::fs::remove_dir_all(folder).map_err(io_err(format!("remove {}", folder.display())))?;
    }
    if let Some(info) = &info {
        op.cmd(&info.top).args(["worktree", "prune"]).raw()?;
        drop_refs(&op, &info.top, task)?;
        drop_refs(&op, &info.top, &result_session(task))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_s_branch_is_named_for_the_computer_and_the_task() {
        assert_eq!(
            task_branch("DESKTOP-PC", "task_0123456789abcdef").as_deref(),
            Some("refs/heads/mother-claude/DESKTOP-PC/task_0123456789abcdef")
        );
        assert_eq!(
            task_branch("Khanh's MacBook Pro", "task_0123456789abcdef").as_deref(),
            Some("refs/heads/mother-claude/Khanh-s-MacBook-Pro/task_0123456789abcdef"),
            "a name made safe for a ref"
        );
        assert_eq!(
            task_branch("..", "task_0123456789abcdef").as_deref(),
            Some("refs/heads/mother-claude/machine/task_0123456789abcdef")
        );
        for bad in ["", "../x", "a b", "-x", "x.lock/..", ".x"] {
            assert!(task_branch("PC", bad).is_none(), "{bad:?}");
        }
        assert_eq!(
            short_branch("refs/heads/mother-claude/PC/task_1"),
            "mother-claude/PC/task_1"
        );
        assert_eq!(
            result_session("task_0123456789abcdef"),
            "task_0123456789abcdef-result"
        );
    }

    #[test]
    fn a_remote_role_s_branch_is_named_for_the_computer_the_team_and_the_role() {
        assert_eq!(
            role_branch("DESKTOP-PC", "shop", "windows").as_deref(),
            Some("refs/heads/mother-claude/DESKTOP-PC/shop/windows")
        );
        assert_eq!(
            role_branch("Khanh's PC", "my.app", "ios").as_deref(),
            Some("refs/heads/mother-claude/Khanh-s-PC/my-app/ios"),
            "names made safe for a ref"
        );
        for bad in ["", "../x", "a b", "-x", ".x"] {
            assert!(role_branch("PC", "shop", bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn a_tip_from_another_machine_is_checked() {
        let oid = "a".repeat(40);
        let good = TaskTip {
            head: Some(oid.clone()),
            wt_tree: oid.clone(),
            snap: oid.clone(),
            tip: oid.clone(),
        };
        assert!(good.validate().is_ok());
        let bad = TaskTip {
            tip: "--upload-pack=x".into(),
            ..good.clone()
        };
        assert!(bad.validate().is_err());
        let bad = TaskTip {
            head: Some("HEAD".into()),
            ..good
        };
        assert!(bad.validate().is_err());
    }
}
