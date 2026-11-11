//! What a move's preflight reads of a checkout, changing nothing (plan §5.8
//! "Preflight", §5.10): the source's dirty state for the move sheet, and
//! whether the target's checkout can take a landing — clean, or parked for
//! this session (F7), no merge or rebase under way, the branch not checked
//! out in another worktree — with what it has (a bundle's haves) and where
//! its branch stands, which the source judges against its own history.
//!
//! A preflight's answer comes from another machine: every id and name in it
//! is checked here before git sees one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::apply::other_worktree_on;
use super::check::{is_oid, valid_branch, valid_session};
use super::git::{trim_text, GitOp, GitRunner};
use super::repo::{busy, parked_state, parse_status, repo_info, status_z, RepoInfo};
use super::types::CodeError;

/// A checkout as the move sheet shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutState {
    /// The working tree's top folder.
    pub top: PathBuf,
    /// The branch HEAD points at (`refs/heads/…`), none when detached.
    pub branch: Option<String>,
    /// HEAD's commit, none on an unborn branch.
    pub head: Option<String>,
    /// Paths `git status` shows: staged, unstaged and untracked ones
    /// (ignored ones never travel, so never count).
    pub changed: usize,
    /// Of those, the untracked ones.
    pub untracked: usize,
    /// A merge, rebase, cherry-pick, revert or bisect in progress.
    pub busy: Option<String>,
    /// `remote.origin.url`, when there is an `origin`.
    pub origin: Option<String>,
    /// The remotes' names.
    pub remotes: Vec<String>,
}

/// The checkout `repo` is in, read without touching it.
pub fn checkout_state(git: &GitRunner, repo: &Path) -> Result<CheckoutState, CodeError> {
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    state_of(&op, &info)
}

fn state_of(op: &GitOp, info: &RepoInfo) -> Result<CheckoutState, CodeError> {
    let status = parse_status(&status_z(op, &info.top)?);
    let untracked = status.values().filter(|xy| xy.as_str() == "??").count();
    let remotes: Vec<String> = op
        .git(&info.top, &["remote"])?
        .lines()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .collect();
    let origin = if remotes.iter().any(|r| r == "origin") {
        let out = op
            .cmd(&info.top)
            .args(["config", "--get", "remote.origin.url"])
            .output()?;
        Some(trim_text(&out.stdout)).filter(|u| out.success && !u.is_empty())
    } else {
        None
    };
    Ok(CheckoutState {
        top: info.top.clone(),
        branch: info.sym.clone(),
        head: info.head.clone(),
        changed: status.len(),
        untracked,
        busy: busy(info),
        origin,
        remotes,
    })
}

/// The target's checkout, as a landing there would find it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetCheck {
    #[serde(flatten)]
    pub state: CheckoutState,
    /// It holds exactly a state of this session it sent or received (F7).
    pub parked: bool,
    /// A checkout landing may change it: clean, or parked for this session,
    /// and no operation under way.
    pub clean: bool,
    /// The tip of the snapshot's branch here (`None`: no such branch).
    pub branch_tip: Option<String>,
    /// That branch is checked out in another worktree here.
    pub elsewhere: Option<PathBuf>,
    /// Commits here a bundle may leave out (`haves`).
    pub haves: Vec<String>,
    pub shallow: bool,
}

/// [`TargetCheck`] of the checkout `repo` is in, for a move of `session`
/// whose snapshot names `branch` (`refs/heads/…`).
pub fn target_check(
    git: &GitRunner,
    repo: &Path,
    session: &str,
    branch: Option<&str>,
) -> Result<TargetCheck, CodeError> {
    if !valid_session(session) {
        return Err(CodeError::Invalid {
            what: "session id",
            value: session.to_string(),
        });
    }
    if let Some(b) = branch.filter(|b| !valid_branch(b)) {
        return Err(CodeError::Invalid {
            what: "branch",
            value: b.to_string(),
        });
    }
    let (state, parked, branch_tip, elsewhere) = {
        let op = git.begin()?;
        let info = repo_info(&op, repo)?;
        let state = state_of(&op, &info)?;
        let parked = state.changed > 0 && parked_state(&op, &info, session)?.is_some();
        let (tip, elsewhere) = match branch {
            Some(b) => (
                op.rev(&info.top, &format!("{b}^{{commit}}"))?,
                other_worktree_on(&op, &info.top, b)?,
            ),
            None => (None, None),
        };
        (state, parked, tip, elsewhere)
    };
    let clean = state.busy.is_none() && (state.changed == 0 || parked);
    let haves = super::snapshot::haves(git, repo, Some(session))?;
    Ok(TargetCheck {
        state,
        parked,
        clean,
        branch_tip,
        elsewhere,
        haves: haves.haves,
        shallow: haves.shallow,
    })
}

/// Where the target's branch stands against the snapshot's base.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BranchRelation {
    /// The target has no such branch: the landing makes it.
    Create,
    /// At the base already.
    Same,
    /// Behind the base: the landing fast-forwards it.
    FastForward,
    /// It has commits the base lacks: a landing in the checkout stops
    /// (a worktree, or replacing it, kept for undo).
    Diverged,
}

/// [`BranchRelation`] of the target's tip `tip` to the snapshot's `base`,
/// judged in the source's checkout `repo` (which has every commit of its
/// own history: a tip it does not know has commits it lacks).
pub fn branch_relation(
    git: &GitRunner,
    repo: &Path,
    base: Option<&str>,
    tip: Option<&str>,
) -> Result<BranchRelation, CodeError> {
    for oid in [base, tip].into_iter().flatten() {
        if !is_oid(oid) {
            return Err(CodeError::Invalid {
                what: "commit",
                value: oid.to_string(),
            });
        }
    }
    let Some(tip) = tip else {
        return Ok(BranchRelation::Create);
    };
    let Some(base) = base else {
        // An unborn branch here, one with commits there.
        return Ok(BranchRelation::Diverged);
    };
    if tip == base {
        return Ok(BranchRelation::Same);
    }
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    let known = op.rev(&info.top, &format!("{tip}^{{commit}}"))?.is_some();
    let behind = known
        && op
            .cmd(&info.top)
            .args(["merge-base", "--is-ancestor", tip, base])
            .ok()?;
    Ok(if behind {
        BranchRelation::FastForward
    } else {
        BranchRelation::Diverged
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{dirty_mix, put, runner, World, BASE_FILES};
    use super::*;

    /// The source's counts, the target's verdict (dirty, then a worktree
    /// elsewhere on its branch) and the branch relation, all read-only.
    #[test]
    fn a_preflight_reads_both_checkouts_without_touching_them() {
        let w = World::new();
        w.seed_with(&[BASE_FILES]);
        w.clones();
        let git = runner();
        let s = checkout_state(&git, &w.src).unwrap();
        assert_eq!((s.changed, s.untracked), (0, 0));
        assert!(s.origin.is_some() && s.remotes == ["origin"]);
        dirty_mix(&w.fx, &w.src);
        let s = checkout_state(&git, &w.src).unwrap();
        assert!(s.changed >= 8 && s.untracked >= 2, "{s:?}");
        assert_eq!(s.branch.as_deref(), Some("refs/heads/main"));

        let session = "0b5c3a1e-9a7f-4c55-8d1e-1c2b3a4d5e6f";
        let main = Some("refs/heads/main");
        let t = target_check(&git, &w.dst, session, main).unwrap();
        assert!(t.clean && !t.parked, "{t:?}");
        assert_eq!(t.branch_tip, s.head);
        assert!(!t.haves.is_empty());
        put(&w.dst, "own.txt", "the target's own\n");
        let t = target_check(&git, &w.dst, session, main).unwrap();
        assert!(!t.clean && t.state.untracked == 1, "{t:?}");
        assert!(target_check(&git, &w.dst, "../x", main).is_err());
        assert!(target_check(&git, &w.dst, session, Some("--upload-pack=x")).is_err());

        let base = s.head.as_deref();
        let rel = |tip: Option<&str>| branch_relation(&git, &w.src, base, tip).unwrap();
        assert_eq!(rel(None), BranchRelation::Create);
        assert_eq!(rel(base), BranchRelation::Same);
        assert_eq!(rel(Some(&"0".repeat(40))), BranchRelation::Diverged);
        assert!(branch_relation(&git, &w.src, base, Some("HEAD")).is_err());
    }
}
