//! Code that travels with a session (docs/PLAN-MACHINES.md §5.10, Phase 3a).
//!
//! The source snapshots its checkout's dirty state with git plumbing — what
//! is staged, and the working tree with untracked files and without ignored
//! ones — as two commits under `refs/mother-claude/moves/<session>/<epoch>`,
//! carried as a bundle in a move folder (without what the target already
//! has) or through hidden refs on a remote. The target lands it in its
//! checkout when that is clean or parked for this session, or in a new
//! worktree, keeps what undo needs, and proves the result file by file.
//!
//! A run on another computer (Phase 4, `work`) uses the same snapshot and
//! fetch for its work folder: a kept worktree of the target's checkout,
//! reset to each snapshot, the files git ignores kept. A task handed to
//! another computer (`task`) gets a worktree of its own at the snapshot;
//! what its session did comes back as a snapshot of that worktree and
//! becomes a branch on the asking computer, moved only forward.
//!
//! Ported from spike S2 (`spikes/machines/code/roundtrip.mjs`; findings
//! F1–F9 in `RESULTS-S2.md`). Invariants:
//! - git runs with argv only, its environment scrubbed, the repository's
//!   hooks replaced by a fresh private folder per operation (F3, F9),
//!   `GIT_OPTIONAL_LOCKS=0`, commits never signed; on Windows
//!   `core.longpaths=true`, and a path git could not read fails a snapshot
//!   (F8) — [`GitRunner`];
//! - the source's real index, HEAD, refs and files are never touched (F4):
//!   both trees come from copies of the index that keep its mtime (F1),
//!   cross-checked with `git status`;
//! - the target's checkout changes only when it is clean or parked for this
//!   session (F7), and only as far as [`undo`] can put back;
//! - a move's record comes from another machine: every id, ref, name and
//!   path in it is checked before git or the file system sees one.
//!
//! The calls block (git runs as a child process): run them off the async
//! runtime (`spawn_blocking`).

mod apply;
mod check;
mod git;
mod preflight;
mod repo;
mod snapshot;
mod sync;
mod task;
mod types;
mod verify;
mod work;

#[cfg(test)]
mod tests;

pub use apply::{apply, remove_move_worktree, undo};
pub use check::{
    is_lfs_oid, is_oid, safe_name, safe_rel_path, valid_branch, valid_remote_name, valid_session,
};
pub use git::{Git, GitOp, GitOutput, GitRunner};
pub use preflight::{
    branch_relation, checkout_state, target_check, BranchRelation, CheckoutState, TargetCheck,
};
pub use snapshot::{drop_snapshot, haves, snapshot};
pub use sync::{
    changes, keep_and_put_back, kept_prefix, known_commits, limits_only, prune_snapshots,
    read_state, root_commits, tip_of, Changes, Checkout, Kept, PutBack, PutBackError,
};
pub use task::{
    contains, drop_task_refs, land_task_branch, remove_task_tree, result_session, role_branch,
    short_branch, task_branch, task_result, task_unchanged, TaskBranchError, TaskLanded,
    TaskResult, TaskResultRequest, TaskTip, TASK_BRANCH_PREFIX,
};
pub use types::*;
pub use verify::verify;
pub use work::{fetch_snapshot, reset_work_tree, top_of, Fetched, WorkTree};
