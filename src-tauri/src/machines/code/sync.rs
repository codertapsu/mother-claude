//! Code sync's half of git (docs/PLAN-MACHINES.md §5.15): what a checkout
//! holds ([`read_state`]), whether two states differ only by what a
//! computer cannot hold ([`limits_only`]), what changed since the last
//! synced state ([`changes`]), and the losing side of a conflict
//! ([`keep_and_put_back`]): its state kept as commits under
//! `refs/mother-claude/sync/<id>/kept-<time>`, its checkout put back to
//! the last synced state — refused before anything changes when something
//! could not be brought back.
//!
//! Like the rest of `code`: argv only, a private empty hooks folder per
//! operation, the real index read through copies (F1, F4), and every id
//! from another machine checked first ([`TreeState::validate`]).

use std::path::{Path, PathBuf};

use super::apply::{other_worktree_on, overwrites, refresh};
use super::check::{is_oid, stamp, valid_branch, valid_session};
use super::git::{GitOp, GitRunner};
use super::repo::{
    busy, checked_trees, empty_tree, identity_env, parse_status, repo_info, session_moves,
    status_z, tree_diff, unmerged, RepoInfo,
};
use super::snapshot::filter_haves;
use super::types::{CodeError, TreeState};

/// The kept refs of sync `id`: `refs/mother-claude/sync/<id>/kept-<time>`.
pub fn kept_prefix(id: &str) -> String {
    format!("refs/mother-claude/sync/{id}/")
}

impl TreeState {
    /// Checks every id and name another machine sent.
    pub fn validate(&self) -> Result<(), CodeError> {
        let bad = |what: &'static str, v: &str| {
            Err(CodeError::Invalid {
                what,
                value: v.to_string(),
            })
        };
        if let Some(b) = self.branch.as_deref().filter(|b| !valid_branch(b)) {
            return bad("branch", b);
        }
        if let Some(h) = self.head.as_deref().filter(|h| !is_oid(h)) {
            return bad("commit", h);
        }
        for t in [&self.idx_tree, &self.wt_tree] {
            if !is_oid(t) {
                return bad("tree", t);
            }
        }
        if self.branch.is_none() && self.head.is_none() {
            return Err(CodeError::BadMove(
                "a detached state without a commit".into(),
            ));
        }
        Ok(())
    }
}

/// A checkout as code sync reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkout {
    /// The working tree's top folder.
    pub top: PathBuf,
    pub state: TreeState,
    /// Paths `git status` shows (staged, unstaged, untracked).
    pub changed: usize,
}

/// The checkout `repo` is in, read without touching it: a merge, rebase,
/// cherry-pick, revert or bisect under way is [`CodeError::InProgress`], a
/// half-done merge [`CodeError::Unmerged`].
pub fn read_state(git: &GitRunner, repo: &Path) -> Result<Checkout, CodeError> {
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    state_of(&op, &info)
}

fn state_of(op: &GitOp, info: &RepoInfo) -> Result<Checkout, CodeError> {
    if let Some(what) = busy(info) {
        return Err(CodeError::InProgress(what));
    }
    if unmerged(op, &info.top)? {
        return Err(CodeError::Unmerged);
    }
    let (idx_tree, wt_tree, _) = checked_trees(op, info)?;
    let changed = parse_status(&status_z(op, &info.top)?).len();
    Ok(Checkout {
        top: info.top.clone(),
        state: TreeState {
            branch: info.sym.clone(),
            head: info.head.clone(),
            idx_tree,
            wt_tree,
        },
        changed,
    })
}

/// The tree of HEAD (the empty tree on an unborn branch).
fn head_tree(op: &GitOp, info: &RepoInfo) -> Result<String, CodeError> {
    match &info.head {
        Some(h) => op.git(&info.top, &["rev-parse", &format!("{h}^{{tree}}")]),
        None => empty_tree(op, &info.top),
    }
}

/// Differences git's settings or a file system explain, never content.
fn limit_kind(kind: &str) -> bool {
    matches!(kind, "submodule" | "filemode" | "symlink-as-file" | "case")
}

/// Whether `a` and `b` are one state but for what a computer cannot hold
/// (an exec bit, a symbolic link held as a file, a case-only name, a
/// submodule's commit): the same HEAD and branch, and trees that differ in
/// no file's content.
pub fn limits_only(
    git: &GitRunner,
    repo: &Path,
    a: &TreeState,
    b: &TreeState,
) -> Result<bool, CodeError> {
    if a == b {
        return Ok(true);
    }
    if a.head != b.head || a.branch != b.branch {
        return Ok(false);
    }
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    for (x, y) in [(&a.idx_tree, &b.idx_tree), (&a.wt_tree, &b.wt_tree)] {
        if x != y
            && tree_diff(&op, &info.top, x, y)?
                .iter()
                .any(|c| !limit_kind(c.kind))
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// What changed in a checkout since a state, for the conflict's two sides.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    /// Files of the working tree that differ (untracked ones included).
    pub files: usize,
    /// HEAD is another commit (a commit, a reset, a pull).
    pub head_moved: bool,
    /// Another branch is checked out.
    pub branch_switched: bool,
    /// The branch now (`refs/heads/…`).
    pub branch: Option<String>,
    pub head: Option<String>,
}

/// What `now` changed since `since` (none: since its HEAD, clean).
pub fn changes(
    git: &GitRunner,
    repo: &Path,
    since: Option<&TreeState>,
    now: &TreeState,
) -> Result<Changes, CodeError> {
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    let base_tree = match since {
        Some(s) => s.wt_tree.clone(),
        None => head_tree(&op, &info)?,
    };
    let files = if base_tree == now.wt_tree {
        0
    } else {
        tree_diff(&op, &info.top, &base_tree, &now.wt_tree)?.len()
    };
    Ok(Changes {
        files,
        head_moved: since.is_some_and(|s| s.head != now.head),
        branch_switched: since.is_some_and(|s| s.branch != now.branch),
        branch: now.branch.clone(),
        head: now.head.clone(),
    })
}

/// The commits of `list` this repository has (a common commit tells two
/// checkouts of one project apart from strangers).
pub fn known_commits(
    git: &GitRunner,
    repo: &Path,
    list: &[String],
) -> Result<Vec<String>, CodeError> {
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    filter_haves(&op, &info.top, list)
}

/// The first commits of HEAD's history (at most 10): two clones of one
/// project share them.
pub fn root_commits(git: &GitRunner, repo: &Path) -> Result<Vec<String>, CodeError> {
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    if info.head.is_none() {
        return Ok(Vec::new());
    }
    let out = op.git(&info.top, &["rev-list", "--max-parents=0", "HEAD"])?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| is_oid(l))
        .take(10)
        .map(str::to_string)
        .collect())
}

/// The tip of `branch` (`refs/heads/…`) here, none when it does not exist.
pub fn tip_of(git: &GitRunner, repo: &Path, branch: &str) -> Result<Option<String>, CodeError> {
    if !valid_branch(branch) {
        return Err(CodeError::Invalid {
            what: "branch",
            value: branch.to_string(),
        });
    }
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    op.rev(&info.top, &format!("{branch}^{{commit}}"))
}

/// Keeps the newest `keep` snapshots of `session` (a sync's steps) and
/// deletes the refs of the older ones: the newest are what the next step
/// chains on (F6); the rest only grow the haves. Answers how many went.
pub fn prune_snapshots(
    git: &GitRunner,
    repo: &Path,
    session: &str,
    keep: usize,
) -> Result<usize, CodeError> {
    if !valid_session(session) {
        return Err(CodeError::Invalid {
            what: "session id",
            value: session.to_string(),
        });
    }
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    let old: Vec<_> = session_moves(&op, &info.top, session)?
        .into_iter()
        .skip(keep)
        .collect();
    if old.is_empty() {
        return Ok(0);
    }
    let mut input = String::new();
    for m in &old {
        let base = format!("refs/mother-claude/moves/{session}/{}", m.epoch);
        input.push_str(&format!("delete {base}/snap\ndelete {base}/index\n"));
    }
    op.cmd(&info.top)
        .args(["update-ref", "--stdin"])
        .input(input)
        .raw()?;
    Ok(old.len())
}

// ─── the losing side of a conflict ─────────────────────────────────────────────

/// What [`keep_and_put_back`] is asked.
#[derive(Clone, Debug)]
pub struct PutBack<'a> {
    /// The sync's id (a ref's name part).
    pub id: &'a str,
    /// This machine's name, for the kept commit's message.
    pub machine: &'a str,
    /// The state to put the checkout back to (the last synced one); none:
    /// its HEAD, clean (a first sync that met changes on both sides).
    pub to: Option<&'a TreeState>,
}

/// What it kept, and the checkout as it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    /// `refs/mother-claude/sync/<id>/kept-<time>`; none when the checkout
    /// already held the state (nothing to keep).
    pub kept_ref: Option<String>,
    /// The kept commit: its tree is the working tree as it was (untracked
    /// files included), its first parent HEAD as it was, its second the
    /// staged state, a third the branch's own commits when HEAD was
    /// elsewhere.
    pub commit: Option<String>,
    /// Files of the working tree that were put back.
    pub files: usize,
    /// The branch had commits beyond the synced state: they are kept too.
    pub commits_kept: bool,
    /// The checkout now (the synced state).
    pub state: TreeState,
}

/// Why the losing side could not be put back: nothing changed then (the
/// kept ref, once written, stays).
#[derive(Debug, thiserror::Error)]
pub enum PutBackError {
    #[error("A git operation is in progress here ({0}): finish or abort it first.")]
    Busy(String),
    #[error("The index has unmerged files: finish the merge first.")]
    Unmerged,
    #[error(
        "Files git ignores are where the synced state has files ({}): putting it back would overwrite them.",
        .paths.join(", ")
    )]
    WouldOverwrite { paths: Vec<String> },
    #[error("The submodule {path} changed: put it back by hand first.")]
    Submodule { path: String },
    #[error("{branch} is checked out in another worktree ({}).", .worktree.display())]
    CheckedOutElsewhere { branch: String, worktree: PathBuf },
    #[error("The synced state is not in this repository ({0}).")]
    Missing(String),
    #[error(
        "After putting it back, the checkout is not the synced state ({0}); what it held is kept."
    )]
    Mismatch(String),
    #[error(transparent)]
    Failed(#[from] CodeError),
}

/// The losing side of a sync conflict (plan §5.15): keeps everything the
/// checkout holds as one commit under `refs/mother-claude/sync/<id>/kept-
/// <time>` — the working tree with untracked files (its tree), what was
/// staged (its second parent) and the commits made since the synced state
/// (its first parent, or a third when HEAD was on another branch) — then
/// puts the checkout back to the synced state: its files, index, branch
/// and HEAD. Refused, with nothing changed, while an operation is under
/// way, when files git ignores (never kept) would be overwritten, when a
/// submodule changed (its inside is not kept), or when the branch is
/// checked out in another worktree. Files git ignores stay as they are.
pub fn keep_and_put_back(
    git: &GitRunner,
    repo: &Path,
    req: &PutBack,
) -> Result<Kept, PutBackError> {
    if !valid_session(req.id) {
        return Err(CodeError::Invalid {
            what: "sync id",
            value: req.id.to_string(),
        }
        .into());
    }
    if let Some(t) = req.to {
        t.validate()?;
    }
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    let top = info.top.clone();
    if let Some(what) = busy(&info) {
        return Err(PutBackError::Busy(what));
    }
    if unmerged(&op, &top)? {
        return Err(PutBackError::Unmerged);
    }
    let (idx_tree, wt_tree, _) = checked_trees(&op, &info)?;
    let cur = TreeState {
        branch: info.sym.clone(),
        head: info.head.clone(),
        idx_tree,
        wt_tree,
    };
    let to = match req.to {
        Some(t) => t.clone(),
        None => {
            let tree = head_tree(&op, &info)?;
            TreeState {
                branch: info.sym.clone(),
                head: info.head.clone(),
                idx_tree: tree.clone(),
                wt_tree: tree,
            }
        }
    };
    // Everything the synced state names is here.
    let mut need: Vec<String> = vec![
        format!("{}^{{tree}}", to.idx_tree),
        format!("{}^{{tree}}", to.wt_tree),
    ];
    if let Some(h) = &to.head {
        need.push(format!("{h}^{{commit}}"));
    }
    for spec in &need {
        if op.rev(&top, spec)?.is_none() {
            return Err(PutBackError::Missing(
                spec.split('^').next().unwrap_or_default().to_string(),
            ));
        }
    }
    if cur == to {
        return Ok(Kept {
            kept_ref: None,
            commit: None,
            files: 0,
            commits_kept: false,
            state: cur,
        });
    }
    // The branch's own commits since the synced state.
    let tip = match &to.branch {
        Some(b) => op.rev(&top, &format!("{b}^{{commit}}"))?,
        None => None,
    };
    let branch_moved = to.branch.is_some() && tip != to.head;
    if let Some(b) = &to.branch {
        if let Some(worktree) = other_worktree_on(&op, &top, b)? {
            return Err(PutBackError::CheckedOutElsewhere {
                branch: b.strip_prefix("refs/heads/").unwrap_or(b).to_string(),
                worktree,
            });
        }
    }
    let diff = tree_diff(&op, &top, &to.wt_tree, &cur.wt_tree)?;
    if let Some(c) = diff.iter().find(|c| c.kind == "submodule") {
        return Err(PutBackError::Submodule {
            path: c.path.clone(),
        });
    }
    // F10: a file git ignores where the synced state has one would be
    // overwritten, and nothing keeps it.
    let blocked = overwrites(&op, &top, &cur.wt_tree, &[&to.wt_tree])?;
    if !blocked.is_empty() {
        return Err(PutBackError::WouldOverwrite { paths: blocked });
    }

    // Kept first: one commit holds all of it.
    let id_env = identity_env(&op, &top)?;
    let commit = |parents: &[&String], message: String, tree: &str| {
        let mut cmd = op.cmd(&top).args(["commit-tree", "--no-gpg-sign"]);
        for p in parents {
            cmd = cmd.arg("-p").arg(p);
        }
        for (k, v) in &id_env {
            cmd = cmd.env(k, v);
        }
        cmd.arg("-m").arg(message).arg(tree).text()
    };
    let head_parent: Vec<&String> = cur.head.iter().collect();
    let index_commit = commit(
        &head_parent,
        format!(
            "mother-claude: staged state kept by code sync {} on {}",
            req.id, req.machine
        ),
        &cur.idx_tree,
    )?;
    let mut parents = head_parent.clone();
    parents.push(&index_commit);
    let branch_tip = tip
        .clone()
        .filter(|t| branch_moved && Some(t) != cur.head.as_ref());
    if let Some(t) = &branch_tip {
        parents.push(t);
    }
    let message = format!(
        "mother-claude: kept by code sync {} on {}\n\n\
         This commit's files are the working tree as it was (untracked files\n\
         included, ignored ones not); its first parent is HEAD as it was, its\n\
         second what was staged{}.",
        req.id,
        req.machine,
        if branch_tip.is_some() {
            ", its third the branch's own commits"
        } else {
            ""
        }
    );
    let kept = commit(&parents, message, &cur.wt_tree)?;
    let kept_ref = format!("{}kept-{}", kept_prefix(req.id), stamp());
    op.cmd(&top)
        .args(["update-ref", "-m", "mother-claude code sync: kept"])
        .args([&kept_ref, &kept, ""])
        .raw()?;
    let trees = op.git(
        &top,
        &[
            "rev-parse",
            &format!("{kept}^{{tree}}"),
            &format!("{index_commit}^{{tree}}"),
        ],
    )?;
    if trees.lines().collect::<Vec<_>>() != [cur.wt_tree.as_str(), cur.idx_tree.as_str()] {
        return Err(PutBackError::Mismatch("the kept commit".into()));
    }

    // Then the checkout goes back; a failure on the way puts it as it was.
    if let Err(e) = put_back(&op, &info, &cur, &to, tip.as_deref()) {
        restore(&op, &info, &cur, &to, tip.as_deref());
        return Err(e.into());
    }
    let after = repo_info(&op, repo)?;
    let now = state_of(&op, &after).map(|c| c.state)?;
    if now != to {
        let what = if now.head != to.head || now.branch != to.branch {
            "HEAD"
        } else if now.idx_tree != to.idx_tree {
            "what is staged"
        } else {
            "the files"
        };
        return Err(PutBackError::Mismatch(what.into()));
    }
    Ok(Kept {
        kept_ref: Some(kept_ref),
        commit: Some(kept),
        files: diff.len(),
        commits_kept: branch_moved && tip.is_some(),
        state: now,
    })
}

/// The files, the branch, HEAD and the index, from `cur` to `to`.
fn put_back(
    op: &GitOp,
    info: &RepoInfo,
    cur: &TreeState,
    to: &TreeState,
    tip: Option<&str>,
) -> Result<(), CodeError> {
    let top = &info.top;
    // 1. The files: the index set to the working tree as it is (stat data
    //    refreshed), then a two-tree switch to the synced one.
    op.cmd(top).args(["read-tree", &cur.wt_tree]).raw()?;
    refresh(op, top)?;
    if cur.wt_tree != to.wt_tree {
        op.cmd(top)
            .args(["read-tree", "-m", "-u", &cur.wt_tree, &to.wt_tree])
            .raw()?;
    }
    // 2. The branch at the synced head, HEAD on it.
    match (&to.branch, &to.head) {
        (Some(b), Some(h)) => {
            if tip != Some(h.as_str()) {
                op.cmd(top)
                    .args(["update-ref", b, h, tip.unwrap_or("")])
                    .raw()?;
            }
            if info.sym.as_ref() != Some(b) {
                op.cmd(top).args(["symbolic-ref", "HEAD", b]).raw()?;
            }
        }
        (Some(b), None) => {
            // Unborn when synced: its first commits go (they are kept).
            if let Some(t) = tip {
                op.cmd(top).args(["update-ref", "-d", b, t]).raw()?;
            }
            if info.sym.as_ref() != Some(b) {
                op.cmd(top).args(["symbolic-ref", "HEAD", b]).raw()?;
            }
        }
        (None, Some(h)) => {
            op.cmd(top)
                .args(["update-ref", "--no-deref", "HEAD", h])
                .raw()?;
        }
        (None, None) => {
            return Err(CodeError::BadMove(
                "a detached state without a commit".into(),
            ))
        }
    }
    // 3. The index: what was staged.
    op.cmd(top).args(["read-tree", &to.idx_tree]).raw()?;
    refresh(op, top)?;
    Ok(())
}

/// Best effort after a failed [`put_back`]: the files, the branch, HEAD and
/// the index as they were (the kept commit holds them anyway).
fn restore(op: &GitOp, info: &RepoInfo, cur: &TreeState, to: &TreeState, tip: Option<&str>) {
    let top = &info.top;
    let _ = op
        .cmd(top)
        .args(["read-tree", "--reset", "-u", &cur.wt_tree])
        .raw();
    if let Some(b) = &to.branch {
        match tip {
            Some(t) => {
                let _ = op.cmd(top).args(["update-ref", b, t]).raw();
            }
            None => {
                let _ = op.cmd(top).args(["update-ref", "-d", b]).raw();
            }
        }
    }
    match (&info.sym, &info.head) {
        (Some(sym), _) => {
            let _ = op.cmd(top).args(["symbolic-ref", "HEAD", sym]).raw();
        }
        (None, Some(h)) => {
            let _ = op
                .cmd(top)
                .args(["update-ref", "--no-deref", "HEAD", h])
                .raw();
        }
        (None, None) => {}
    }
    let _ = op.cmd(top).args(["read-tree", &cur.idx_tree]).raw();
    let _ = refresh(op, top);
}
